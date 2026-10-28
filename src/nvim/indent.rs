//! Indentation and tabstop functions (`src/nvim/indent.c`).

use crate::api::private::helpers::{cstr_as_obj, integer_obj};
use crate::buffer::{buf_T, buf_get_changedtick, modifiable};
use crate::change::{
    SIN_INSERT, SIN_NOMARK, TAB, changed_bytes, changed_lines, get_leader_len, ins_bytes, ins_str,
    replace_normal,
};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, ascii_iswhite_or_nul, char2cells, getdigits,
    getwhitecols_curline, kOptDyFlagUhex, skipwhite, vim_strsize,
};
use crate::cursor::{
    check_cursor, coladvance, colnr_T, get_cursor_line_len, get_cursor_line_ptr, linenr_T, lt,
    pos_T,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later};
use crate::errors::*;
use crate::eval::eval_expr_option_number;
use crate::eval::typval::{EvalFuncData, kCallbackNone, tv_get_lnum, typval_T, varnumber_T};
use crate::eval::vars::{VV_LNUM, set_vim_var_nr};
use crate::ex_docmd::{CMOD_LOCKMARKS, exarg_T, handle_did_throw};
use crate::extmark::{extmark_splice_cols, kExtmarkUndo};
use crate::globals::*;
use crate::indent_c::{cindent_on, do_c_expr_indent, in_cinkeys};
use crate::insert::{
    backspace_until_column, beginline, get_nolist_virtcol, replace_join, replace_push_nul,
};
use crate::mbyte::{utf_ClusterInfo, utf_ptr2CharInfo, utf_ptr2StrCharInfo, utfc_ptr2len};
use crate::memline::{ml_get, ml_get_buf, ml_get_len, ml_get_pos, ml_replace};
use crate::message::{emsg, msg_progress};
use crate::r#move::{changed_cline_bef_curs, win_col_off, win_col_off2};
use crate::ops::shift_line;
use crate::option::{
    OPT_LOCAL, OptInt, copy_option_part, get_flp_value, get_showbreak_value, opt_error,
    set_option_value, was_set_insecurely,
};
use crate::option_vars::*;
use crate::optionstr::{CharBuf, OptIndex, OptKeyDict_briopt, opt_keyset};
use crate::os::input::line_breakcheck;
use crate::plines::{
    CSType, CharInfo, CharsizeArg, ClusterInfo, StrCharInfo, getvcol, getvcol_nolist,
    init_charsize_arg, win_charsize, win_chartabsize,
};
use crate::regexp::{
    MAXCOL, RE_AUTO, RE_MAGIC, RE_STRICT, RE_STRING, regmatch_T, vim_regcomp, vim_regexec,
    vim_regfree,
};
use crate::search::{findmatch, linewhite};
use crate::strings::{gettext, ngettext, vim_strchr};
use crate::textformat::{has_format_option, oparg_T};
use crate::undo::{u_clearline, u_save, u_savecommon, u_savesub};
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong};
use std::ptr;

use crate::msgpack_rpc::packer::handle_T;
pub type IndentGetter = fn() -> c_int;
pub type Indenter = fn() -> c_int;

use crate::ex_cmds::{BL_FIX, BL_SOL, BL_WHITE};
use crate::globals::IOSIZE;
use crate::state::{MODE_INSERT, VREPLACE_FLAG};

pub(crate) const TABSTOP_MAX: c_int = 9999;

pub(crate) const SIN_CHANGED: c_int = 1;
const SIN_UNDO: c_int = 4;

pub(crate) const INDENT_SET: c_int = 1;
pub(crate) const INDENT_DEC: c_int = 3;

pub(crate) const kFoQComs: c_int = b'q' as c_int;

pub(crate) const kOptBreakindentopt: OptIndex = 25;
pub(crate) const kOptIndentexpr: OptIndex = 149;
pub(crate) const kOptTabstop: OptIndex = 308;
pub(crate) const kOptVartabstop: OptIndex = 342;
pub(crate) const kBufOptIndentexpr: usize = 48;

const KEYSET_OPTIDX_briopt__min: u64 = 1;
const KEYSET_OPTIDX_briopt__sbr: u64 = 2;
const KEYSET_OPTIDX_briopt__list: u64 = 3;
const KEYSET_OPTIDX_briopt__shift: u64 = 4;
const KEYSET_OPTIDX_briopt__column: u64 = 5;

struct IndentModuleState {
    PREV_INDENT: c_int,
    PREV_TS: OptInt,
    PREV_VTS: Vec<colnr_T>,
    PREV_FNUM: c_int,
    PREV_LINE: Option<std::ffi::CString>,
    PREV_TICK: varnumber_T,
    PREV_LIST: c_int,
    PREV_LISTOPT: c_int,
    PREV_NO_TS: bool,
    PREV_DY_UHEX: c_uint,
    PREV_FLP: Option<std::ffi::CString>,
}

impl IndentModuleState {
    const fn new() -> Self {
        Self {
            PREV_INDENT: 0,
            PREV_TS: 0,
            PREV_VTS: Vec::new(),
            PREV_FNUM: 0,
            PREV_LINE: None,
            PREV_TICK: 0,
            PREV_LIST: 0,
            PREV_LISTOPT: 0,
            PREV_NO_TS: false,
            PREV_DY_UHEX: 0,
            PREV_FLP: None,
        }
    }
}

static INDENT_STATE: GlobalState<IndentModuleState> = GlobalState::new(IndentModuleState::new());

#[inline]
fn ind_st() -> &'static mut IndentModuleState {
    unsafe { &mut *INDENT_STATE.as_ptr() }
}

#[inline]
const fn has_briopt_key(v: &OptKeyDict_briopt, bit: u64) -> bool {
    (v.is_set__briopt_ & (1u64 << bit)) != 0
}

/// Set the integer values corresponding to the string setting of `'vartabstop'` (`tabstop_set`).
pub fn tabstop_set(
    var: impl crate::strings::IntoCCharPtr,
    array: &mut Vec<colnr_T>,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    let mut var = var.into_c_char_ptr().cast_mut();
    unsafe {
        let mut valcount: usize = 1;

        if *var == NUL || (*var == b'0' as c_char && *var.add(1) == NUL) {
            array.clear();
            return ptr::null();
        }

        let mut cp = var;
        while *cp != NUL {
            if cp == var || *cp.sub(1) == b',' as c_char {
                let mut end = cp;
                if getdigits(&raw mut end, false, 1) <= 0 {
                    if cp != end {
                        return e_positive.as_ptr();
                    }
                    var = cp;
                    return opt_error(errbuf, e_invarg2, var);
                }
            }

            if ascii_isdigit(*cp) {
                cp = cp.add(1);
                continue;
            }
            if *cp == b',' as c_char
                && cp > var
                && *cp.sub(1) != b',' as c_char
                && *cp.add(1) != NUL
            {
                valcount += 1;
                cp = cp.add(1);
                continue;
            }
            return opt_error(errbuf, e_invarg2, var);
        }

        let mut out = Vec::with_capacity(valcount);
        cp = var;
        while *cp != NUL {
            let n = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(cp)) as c_int;
            if n <= 0 || n > TABSTOP_MAX {
                array.clear();
                var = cp;
                return opt_error(errbuf, e_invarg2, var);
            }
            out.push(n);
            while *cp != NUL && *cp != b',' as c_char {
                cp = cp.add(1);
            }
            if *cp != NUL {
                cp = cp.add(1);
            }
        }

        *array = out;
        ptr::null()
    }
}

/// Calculate the number of screen spaces a tab will occupy (`tabstop_padding`).
pub fn tabstop_padding(col: colnr_T, ts_arg: OptInt, vts: &[colnr_T]) -> c_int {
    let ts = if ts_arg == 0 { 8 } else { ts_arg };
    let mut tabcol: colnr_T = 0;

    if vts.is_empty() {
        return (ts - (i64::from(col) % ts)) as c_int;
    }

    for &width in vts {
        tabcol += width;
        if tabcol > col {
            return tabcol - col;
        }
    }
    let last = *vts.last().unwrap();
    last - ((col - tabcol) % last)
}

/// Find the size of the tab that covers a particular column (`tabstop_at`).
pub fn tabstop_at(col: colnr_T, ts: OptInt, vts: &[colnr_T], left: bool) -> c_int {
    if vts.is_empty() {
        return ts as c_int;
    }

    let mut tabcol: colnr_T = 0;
    for (idx, &width) in vts.iter().enumerate() {
        tabcol += width;
        if tabcol > col {
            return if left && idx == 0 {
                col
            } else {
                vts[idx - usize::from(left)]
            };
        }
    }
    *vts.last().unwrap()
}

/// Find the column on which a tab starts (`tabstop_start`).
pub fn tabstop_start(col: colnr_T, ts: c_int, vts: &[colnr_T]) -> colnr_T {
    let mut tabcol: colnr_T = 0;

    if vts.is_empty() {
        return col - col % ts;
    }

    for &cur in vts {
        tabcol += cur;
        if tabcol > col {
            return tabcol - cur;
        }
    }

    let last = *vts.last().unwrap();
    let excess = tabcol % last;
    col - (col - excess) % last
}

/// Find the number of tabs and spaces necessary to get from one column to another (`tabstop_fromto`).
pub fn tabstop_fromto(
    start_col: colnr_T,
    end_col: colnr_T,
    ts_arg: c_int,
    vts: &[colnr_T],
    ntabs: &mut c_int,
    nspcs: &mut c_int,
) {
     {
        let mut spaces: c_int = end_col - start_col;
        let mut tabcol: colnr_T = 0;
        let mut padding: c_int = 0;
        let ts = if ts_arg == 0 {
            curbuf_mut().b_p_ts as c_int
        } else {
            ts_arg
        };
        debug_assert!(ts != 0);

        if vts.is_empty() {
            let mut tabs: c_int = 0;
            let initspc = ts - (start_col % ts);
            if spaces >= initspc {
                spaces -= initspc;
                tabs += 1;
            }
            tabs += spaces / ts;
            spaces -= (spaces / ts) * ts;

            *ntabs = tabs;
            *nspcs = spaces;
            return;
        }

        let tabcount = vts.len();
        let mut t: usize = 0;
        while t < tabcount {
            tabcol += vts[t];
            if tabcol > start_col {
                padding = tabcol - start_col;
                break;
            }
            t += 1;
        }
        if t >= tabcount {
            let last = *vts.last().unwrap();
            padding = last - ((start_col - tabcol) % last);
        }

        if spaces < padding {
            *ntabs = 0;
            *nspcs = spaces;
            return;
        }

        *ntabs = 1;
        spaces -= padding;

        while spaces != 0 && {
            t += 1;
            t < tabcount
        } {
            padding = vts[t];
            if spaces < padding {
                *nspcs = spaces;
                return;
            }
            *ntabs += 1;
            spaces -= padding;
        }

        let last = *vts.last().unwrap();
        *ntabs += spaces / last;
        *nspcs = spaces % last;
    }
}

/// Return a count of the number of tabstops (`tabstop_count`).
#[inline]
pub fn tabstop_count(ts: &[colnr_T]) -> c_int {
    ts.len() as c_int
}

/// Return the first tabstop, or 8 if there are no tabstops defined (`tabstop_first`).
#[inline]
pub fn tabstop_first(ts: &[colnr_T]) -> c_int {
    ts.first().copied().unwrap_or(8)
}

/// Return the effective shiftwidth value for current buffer (`get_sw_value`).
pub fn get_sw_value(buf: &mut buf_T) -> c_int {
    get_sw_value_col( buf, 0, false)
}

fn get_sw_value_pos(buf: &mut buf_T, pos: &pos_T, left: bool) -> c_int {
     {
        let save_cursor = curwin_mut().w_cursor;
        curwin_mut().w_cursor = *pos;
        let sw_value = get_sw_value_col( buf, get_nolist_virtcol(), left);
        curwin_mut().w_cursor = save_cursor;
        sw_value
    }
}

/// Return the effective shiftwidth value using the first non-blank in the current line (`get_sw_value_indent`).
pub fn get_sw_value_indent(buf: *mut buf_T, left: bool) -> c_int {
     {
        let mut pos = curwin_mut().w_cursor;
        pos.col = getwhitecols_curline() as colnr_T;
        get_sw_value_pos(unsafe { &mut *buf }, &pos, left)
    }
}

/// Return the effective shiftwidth value using virtual column `col` (`get_sw_value_col`).
pub fn get_sw_value_col(buf: &mut buf_T, col: colnr_T, left: bool) -> c_int {
     {
        if buf.b_p_sw != 0 {
            buf.b_p_sw as c_int
        } else {
            tabstop_at(col, buf.b_p_ts, &buf.b_p_vts_array, left)
        }
    }
}

/// Return the effective softtabstop value for the current buffer (`get_sts_value`).
pub fn get_sts_value() -> c_int {
     {
        if curbuf_mut().b_p_sts < 0 {
            get_sw_value(curbuf_mut())
        } else {
            curbuf_mut().b_p_sts as c_int
        }
    }
}

/// Count the size (in window cells) of the indent in the current line (`get_indent`).
pub fn get_indent() -> c_int {
     {
        indent_size_ts(
            get_cursor_line_ptr(),
            curbuf_mut().b_p_ts,
            &curbuf_mut().b_p_vts_array,
        )
    }
}

/// Count the size (in window cells) of the indent in line `lnum` (`get_indent_lnum`).
pub fn get_indent_lnum(lnum: linenr_T) -> c_int {
     indent_size_ts(ml_get(lnum), curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array)
}

/// Count the size (in window cells) of the indent in line `lnum` of buffer `buf` (`get_indent_buf`).
pub fn get_indent_buf(buf: &mut buf_T, lnum: linenr_T) -> c_int {
    indent_size_ts(ml_get_buf(&raw mut *buf, lnum), buf.b_p_ts, &buf.b_p_vts_array)
}

/// Compute the size of the indent (in window cells) in line `ptr`, without tabstops (`indent_size_no_ts`).
pub fn indent_size_no_ts(mut ptr: *const c_char) -> c_int {
    unsafe {
        let tab_size = opt_vars().dy_escape_width;
        let mut vcol: c_int = 0;
        loop {
            let c = *ptr;
            ptr = ptr.add(1);
            if c == b' ' as c_char {
                vcol += 1;
            } else if c == TAB {
                vcol += tab_size;
            } else {
                return vcol;
            }
        }
    }
}

/// Compute the size of the indent (in window cells) in line `ptr`, using tabstops (`indent_size_ts`).
pub fn indent_size_ts(ptr: impl crate::strings::IntoCCharPtr, ts: OptInt, vts: &[colnr_T]) -> c_int {
    let mut ptr = ptr.into_c_char_ptr();
    unsafe {
        debug_assert!(char2cells(b' ' as c_int) == 1);

        let mut vcol: c_int = 0;
        let tabstop_width: c_int;
        let mut next_tab_vcol: c_int;

        if let Some((&last_tabstop, init_tabstops)) = vts.split_last() {
            for &cur_tabstop in init_tabstops {
                let mut cur_vcol = vcol;
                vcol += cur_tabstop;
                debug_assert!(cur_vcol < vcol);

                loop {
                    let c = *ptr;
                    ptr = ptr.add(1);
                    if c == b' ' as c_char {
                        cur_vcol += 1;
                    } else if c == TAB {
                        break;
                    } else {
                        return cur_vcol;
                    }
                    if cur_vcol == vcol {
                        break;
                    }
                }
            }

            tabstop_width = last_tabstop;
            next_tab_vcol = vcol + tabstop_width;
        } else {
            tabstop_width = (if ts == 0 { 8 } else { ts }) as c_int;
            next_tab_vcol = tabstop_width;
        }

        debug_assert!(tabstop_width != 0);
        loop {
            let c = *ptr;
            ptr = ptr.add(1);
            if c == b' ' as c_char {
                vcol += 1;
                if vcol == next_tab_vcol {
                    next_tab_vcol += tabstop_width;
                }
            } else if c == TAB {
                vcol = next_tab_vcol;
                next_tab_vcol += tabstop_width;
            } else {
                return vcol;
            }
        }
    }
}

/// Set the indent of the current line (`set_indent`).
pub fn set_indent(size: c_int, flags: c_int) -> bool {
    unsafe {
        let mut doit = false;
        let mut ind_done: c_int = 0;
        let mut tab_pad: c_int;
        let mut retval = false;
        let mut orig_char_len: c_int = -1;

        let mut todo = size;
        let mut ind_len: c_int = 0;
        let oldline = get_cursor_line_ptr();
        let mut p = oldline;
        let mut line_len = get_cursor_line_len() + 1;

        if curbuf_mut().b_p_et == 0 || ((flags & SIN_INSERT) == 0 && curbuf_mut().b_p_pi != 0) {
            let mut ind_col: c_int = 0;
            if (flags & SIN_INSERT) == 0 && curbuf_mut().b_p_pi != 0 {
                ind_done = 0;

                while todo > 0 && ascii_iswhite(*p) {
                    if *p == TAB {
                        tab_pad =
                            tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                        if todo < tab_pad {
                            break;
                        }
                        todo -= tab_pad;
                        ind_len += 1;
                        ind_done += tab_pad;
                    } else {
                        todo -= 1;
                        ind_len += 1;
                        ind_done += 1;
                    }
                    p = p.add(1);
                }

                ind_col = ind_done;
                if curbuf_mut().b_p_et != 0 {
                    orig_char_len = ind_len;
                }
                tab_pad = tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                if todo >= tab_pad && orig_char_len == -1 {
                    doit = true;
                    todo -= tab_pad;
                    ind_len += 1;
                    ind_col += tab_pad;
                }
            }

            loop {
                tab_pad = tabstop_padding(ind_col, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                if todo < tab_pad {
                    break;
                }
                if *p != TAB {
                    doit = true;
                } else {
                    p = p.add(1);
                }
                todo -= tab_pad;
                ind_len += 1;
                ind_col += tab_pad;
            }
        }

        while todo > 0 {
            if *p != b' ' as c_char {
                doit = true;
            } else {
                p = p.add(1);
            }
            todo -= 1;
            ind_len += 1;
        }

        if !doit && !ascii_iswhite(*p) && (flags & SIN_INSERT) == 0 {
            return false;
        }

        if (flags & SIN_INSERT) != 0 {
            p = oldline;
        } else {
            p = skipwhite(p);
            line_len -= p.offset_from(oldline) as c_int;
        }

        let mut skipcols: c_int = 0;
        let mut newline_vec: Vec<c_char>;
        let newline: *mut c_char;
        let mut s: *mut c_char;
        if orig_char_len != -1 {
            let newline_size = orig_char_len
                .checked_add(size)
                .and_then(|v| v.checked_sub(ind_done))
                .and_then(|v| v.checked_add(line_len))
                .expect("newline_size overflow");
            debug_assert!(newline_size >= 0);
            newline_vec = vec![0 as c_char; newline_size as usize];
            newline = newline_vec.as_mut_ptr();
            todo = size - ind_done;

            ind_len = orig_char_len + todo;
            p = oldline;
            s = newline;
            skipcols = orig_char_len;

            while orig_char_len > 0 {
                *s = *p;
                s = s.add(1);
                p = p.add(1);
                orig_char_len -= 1;
            }

            while ascii_iswhite(*p) {
                p = p.add(1);
            }
        } else {
            todo = size;
            debug_assert!(ind_len + line_len >= 0);
            let newline_size = (ind_len as usize)
                .checked_add(line_len as usize)
                .expect("newline_size overflow");
            newline_vec = vec![0 as c_char; newline_size];
            newline = newline_vec.as_mut_ptr();
            s = newline;
        }

        if curbuf_mut().b_p_et == 0 {
            if (flags & SIN_INSERT) == 0 && curbuf_mut().b_p_pi != 0 {
                p = oldline;
                ind_done = 0;

                while todo > 0 && ascii_iswhite(*p) {
                    if *p == TAB {
                        tab_pad =
                            tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                        if todo < tab_pad {
                            break;
                        }
                        todo -= tab_pad;
                        ind_done += tab_pad;
                    } else {
                        todo -= 1;
                        ind_done += 1;
                    }
                    *s = *p;
                    s = s.add(1);
                    p = p.add(1);
                    skipcols += 1;
                }

                tab_pad = tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                if todo >= tab_pad {
                    *s = TAB;
                    s = s.add(1);
                    todo -= tab_pad;
                    ind_done += tab_pad;
                }
                p = skipwhite(p);
            }

            loop {
                tab_pad = tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                if todo < tab_pad {
                    break;
                }
                *s = TAB;
                s = s.add(1);
                todo -= tab_pad;
                ind_done += tab_pad;
            }
        }

        while todo > 0 {
            *s = b' ' as c_char;
            s = s.add(1);
            todo -= 1;
        }
        ptr::copy(p, s, line_len as usize);

        if (flags & SIN_UNDO) == 0 || u_savesub(curwin_mut().w_cursor.lnum) {
            let old_offset = p.offset_from(oldline) as colnr_T;
            let new_offset = s.offset_from(newline) as colnr_T;

            ml_replace(curwin_mut().w_cursor.lnum, newline, true);
            if (flags & SIN_NOMARK) == 0 {
                extmark_splice_cols(
                    curbuf_mut(),
                    curwin_mut().w_cursor.lnum - 1,
                    skipcols,
                    old_offset - skipcols,
                    new_offset - skipcols,
                    kExtmarkUndo,
                );
            }

            if (flags & SIN_CHANGED) != 0 {
                changed_bytes(curwin_mut().w_cursor.lnum, 0);
            }

            if g_st().saved_cursor.lnum == curwin_mut().w_cursor.lnum {
                if g_st().saved_cursor.col >= old_offset {
                    g_st().saved_cursor.col += ind_len - old_offset;
                } else if g_st().saved_cursor.col >= new_offset {
                    g_st().saved_cursor.col = new_offset;
                }
            }
            retval = true;
        }
        drop(newline_vec);
        curwin_mut().w_cursor.col = ind_len;
        retval
    }
}

/// Return the indent of the current line after a number (`get_number_indent`).
pub fn get_number_indent(lnum: linenr_T) -> c_int {
    unsafe {
        if lnum > curbuf_mut().b_ml.ml_line_count {
            return -1;
        }

        let mut pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut lead_len: c_int = 0;

        if (g_st().State & MODE_INSERT) != 0 || has_format_option(kFoQComs) {
            lead_len = get_leader_len(ml_get(lnum), ptr::null_mut(), false, true);
        }

        let mut regmatch = regmatch_T {
            regprog: vim_regcomp(curbuf_mut().b_p_flp, RE_MAGIC),
            startp: [ptr::null_mut(); 10],
            endp: [ptr::null_mut(); 10],
            rm_matchcol: 0,
            rm_ic: false,
        };

        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = false;
            if vim_regexec(&mut regmatch, ml_get(lnum).add(lead_len as usize), 0) {
                pos.lnum = lnum;
                pos.col = regmatch.endp[0].offset_from(ml_get(lnum)) as colnr_T;
                pos.coladd = 0;
            }
            vim_regfree(regmatch.regprog);
        }

        if pos.lnum == 0 || *ml_get_pos(&pos) == NUL {
            return -1;
        }
        let mut col: colnr_T = 0;
        getvcol(
            curwin_mut(),
            &mut pos,
            Some(&mut col),
            None,
            None,
            0,
        );
        col as c_int
    }
}

/// Check `briopt` as `'breakindentopt'` and update the members of `wp` (`briopt_check`).
pub fn briopt_check(wp: *mut win_T) {
    unsafe {
        if wp.is_null() {
            return;
        }
        let v = opt_keyset(
            (*wp).w_onebuf_opt.wo_briopt,
            kOptBreakindentopt,
            ptr::null_mut(),
        )
        .cast::<OptKeyDict_briopt>();
        let vr = &*v;
        (*wp).w_briopt_shift = if has_briopt_key(vr, KEYSET_OPTIDX_briopt__shift) {
            vr.shift as c_int
        } else {
            0
        };
        (*wp).w_briopt_min = if has_briopt_key(vr, KEYSET_OPTIDX_briopt__min) {
            vr.min as c_int
        } else {
            20
        };
        (*wp).w_briopt_sbr = has_briopt_key(vr, KEYSET_OPTIDX_briopt__sbr);
        (*wp).w_briopt_list = if has_briopt_key(vr, KEYSET_OPTIDX_briopt__list) {
            vr.list as c_int
        } else {
            0
        };
        (*wp).w_briopt_vcol = if has_briopt_key(vr, KEYSET_OPTIDX_briopt__column) {
            vr.column as c_int
        } else {
            0
        };
    }
}

/// Return appropriate space number for breakindent (`get_breakindent_win`).
pub fn get_breakindent_win(wp: &mut win_T, line: *mut c_char) -> c_int {
    unsafe {
        let eff_wwidth = wp.w_view_width - win_col_off(&raw mut *wp) + win_col_off2(wp);
        let no_ts = wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.tab1 == 0;

        let buf = wp.w_buffer;
        let prev_vts = &raw mut ind_st().PREV_VTS;
        let flp_val = CStr::from_ptr(get_flp_value(&mut *buf));
        let line_cstr = CStr::from_ptr(line);
        if ind_st().PREV_FNUM != (*buf).b_fnum
            || ind_st().PREV_TS != (*buf).b_p_ts
            || (*prev_vts).as_slice() != (*buf).b_p_vts_array.as_slice()
            || ind_st().PREV_TICK != buf_get_changedtick(&*buf)
            || ind_st().PREV_LISTOPT != wp.w_briopt_list
            || ind_st().PREV_NO_TS != no_ts
            || ind_st().PREV_DY_UHEX != (opt_vars().dy_flags & kOptDyFlagUhex)
            || ind_st().PREV_FLP.as_deref() != Some(flp_val)
            || ind_st().PREV_LINE.as_deref() != Some(line_cstr)
        {
            ind_st().PREV_FNUM = (*buf).b_fnum;
            ind_st().PREV_LINE = Some(line_cstr.to_owned());
            ind_st().PREV_TS = (*buf).b_p_ts;
            (*prev_vts).clone_from(&(*buf).b_p_vts_array);
            if wp.w_briopt_vcol == 0 {
                if no_ts {
                    ind_st().PREV_INDENT = indent_size_no_ts(line);
                } else {
                    ind_st().PREV_INDENT = indent_size_ts(line, (*buf).b_p_ts, &(*buf).b_p_vts_array);
                }
            }
            ind_st().PREV_TICK = buf_get_changedtick(&*buf);
            ind_st().PREV_LISTOPT = wp.w_briopt_list;
            ind_st().PREV_LIST = 0;
            ind_st().PREV_NO_TS = no_ts;
            ind_st().PREV_DY_UHEX = opt_vars().dy_flags & kOptDyFlagUhex;
            ind_st().PREV_FLP = Some(flp_val.to_owned());

            if wp.w_briopt_list != 0 && wp.w_briopt_vcol == 0 {
                let flp_ptr = (&raw const ind_st().PREV_FLP)
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .as_ptr()
                    .cast_mut();
                let mut regmatch = regmatch_T {
                    regprog: vim_regcomp(flp_ptr, RE_MAGIC + RE_STRING + RE_AUTO + RE_STRICT),
                    startp: [ptr::null_mut(); 10],
                    endp: [ptr::null_mut(); 10],
                    rm_matchcol: 0,
                    rm_ic: false,
                };
                if !regmatch.regprog.is_null() {
                    regmatch.rm_ic = false;
                    if vim_regexec(&mut regmatch, line, 0) {
                        if wp.w_briopt_list > 0 {
                            ind_st().PREV_LIST += wp.w_briopt_list;
                        } else {
                            let mut ptr_cur = regmatch.startp[0];
                            let end_ptr = regmatch.endp[0];
                            let mut indent: c_int = 0;
                            while ptr_cur < end_ptr {
                                indent += win_chartabsize(wp, ptr_cur, indent);
                                ptr_cur = ptr_cur.add(utfc_ptr2len(ptr_cur) as usize);
                            }
                            ind_st().PREV_INDENT = indent;
                        }
                    }
                    vim_regfree(regmatch.regprog);
                }
            }
        }

        let mut bri = if wp.w_briopt_vcol != 0 {
            ind_st().PREV_LIST = 0;
            wp.w_briopt_vcol
        } else {
            ind_st().PREV_INDENT + wp.w_briopt_shift
        };

        bri += win_col_off2(wp);

        if wp.w_briopt_list > 0 {
            bri += ind_st().PREV_LIST;
        }

        if wp.w_briopt_sbr {
            bri -= vim_strsize(get_showbreak_value(&mut *wp));
        }

        if bri < 0 {
            bri = 0;
        } else if bri > eff_wwidth - wp.w_briopt_min {
            bri = (eff_wwidth - wp.w_briopt_min).max(0);
        }

        bri
    }
}

/// Return true if the cursor is before (or on, when `extra == 0`) the first non-blank in the line (`inindent`).
pub fn inindent(extra: c_int) -> bool {
    unsafe {
        let mut ptr = get_cursor_line_ptr();
        let mut col: colnr_T = 0;
        while ascii_iswhite(*ptr) {
            col += 1;
            ptr = ptr.add(1);
        }
        col >= curwin_mut().w_cursor.col + extra
    }
}

/// Handle reindenting a block of lines (`op_reindent`).
pub fn op_reindent(oap: &mut oparg_T, how: Indenter) {
    unsafe {
        let mut i: c_int = 0;
        let mut first_changed: linenr_T = 0;
        let mut last_changed: linenr_T = 0;
        let start_lnum = curwin_mut().w_cursor.lnum;

        if !modifiable(curbuf_mut()) {
            emsg(gettext(e_modifiable));
            return;
        }

        if u_savecommon(
            curbuf_mut(),
            start_lnum - 1,
            start_lnum + oap.line_count,
            start_lnum + oap.line_count,
            false,
        )
        {
            i = oap.line_count - 1;
            while i >= 0 && !g_st().got_int {
                if i > 1
                    && (i % 50 == 0 || i == oap.line_count - 1)
                    && i64::from(oap.line_count) > opt_vars().p_report
                {
                    let fmt_ptr = gettext(c"%ld lines to indent... ");
                    let fmt_str = crate::memory::cstr_lossy(fmt_ptr);
                    let msg_str = fmt_str.replacen("%ld", &i.to_string(), 1);
                    let bytes = msg_str.as_bytes();
                    let copy_len = bytes.len().min(IOSIZE - 1);
                    ptr::copy_nonoverlapping(
                        bytes.as_ptr().cast::<c_char>(),
                        &raw mut g_st().IObuff[0],
                        copy_len,
                    );
                    g_st().IObuff[copy_len] = NUL;

                    let save_lnum = curwin_mut().w_cursor.lnum;
                    curwin_mut().w_cursor.lnum = start_lnum;
                    msg_progress(
                        &raw mut g_st().IObuff[0],
                        c"nvim.indent",
                        c"running",
                        -1,
                        0,
                        true,
                        false,
                        false,
                    );
                    curwin_mut().w_cursor.lnum = save_lnum;
                }

                if i != oap.line_count - 1
                    || oap.line_count == 1
                    || !std::ptr::fn_addr_eq(how, get_lisp_indent as Indenter)
                {
                    let l = skipwhite(get_cursor_line_ptr());
                    let amount = if *l == NUL { 0 } else { how() };
                    if amount >= 0 && set_indent(amount, 0) {
                        if first_changed == 0 {
                            first_changed = curwin_mut().w_cursor.lnum;
                        }
                        last_changed = curwin_mut().w_cursor.lnum;
                    }
                }
                curwin_mut().w_cursor.lnum += 1;
                curwin_mut().w_cursor.col = 0;
                i -= 1;
            }
        }

        curwin_mut().w_cursor.lnum = start_lnum;
        beginline(BL_SOL | BL_FIX);

        if last_changed != 0 {
            changed_lines(
                curbuf_mut(),
                first_changed,
                0,
                if oap.is_VIsual {
                    start_lnum + oap.line_count
                } else {
                    last_changed + 1
                },
                0,
                true,
            );
        } else if oap.is_VIsual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if i64::from(oap.line_count) > opt_vars().p_report {
            i = oap.line_count - (i + 1);
            let fmt_ptr = ngettext(
                c"%ld line indented ",
                c"%ld lines indented ",
                i as c_ulong,
            );
            let fmt_str = crate::memory::cstr_lossy(fmt_ptr);
            let msg_str = fmt_str.replacen("%ld", &i.to_string(), 1);
            let bytes = msg_str.as_bytes();
            let copy_len = bytes.len().min(IOSIZE - 1);
            ptr::copy_nonoverlapping(
                bytes.as_ptr().cast::<c_char>(),
                &raw mut g_st().IObuff[0],
                copy_len,
            );
            g_st().IObuff[copy_len] = NUL;
            msg_progress(
                &raw mut g_st().IObuff[0],
                c"nvim.indent",
                c"success",
                -1,
                0,
                true,
                false,
                false,
            );
        }
        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end = oap.end;
        }
    }
}

/// Return true if lines starting with `#` should be left aligned (`preprocs_left`).
pub fn preprocs_left() -> bool {
     {
        (curbuf_mut().b_p_si != 0 && curbuf_mut().b_p_cin == 0)
            || (curbuf_mut().b_p_cin != 0
                && in_cinkeys(b'#' as c_int, b' ' as c_int, true)
                && curbuf_mut().b_ind_hash_comment == 0)
    }
}

/// Return true if the conditions are OK for smart indenting (`may_do_si`).
pub fn may_do_si() -> bool {
     {
        curbuf_mut().b_p_si != 0
            && curbuf_mut().b_p_cin == 0
            && curbuf_mut().b_p_inde.cb_type() == kCallbackNone
            && opt_vars().p_paste == 0
    }
}

/// Try to do some very smart auto-indenting (`ins_try_si`).
pub fn ins_try_si(c: c_int) {
    unsafe {
        if ((g_st().Ins.did_si || g_st().Ins.can_si_back) && c == b'{' as c_int)
            || (g_st().Ins.can_si && c == b'}' as c_int && inindent(0))
        {
            let pos = if c == b'}' as c_int {
                findmatch(ptr::null_mut(), b'{' as c_int)
            } else {
                ptr::null_mut()
            };
            if !pos.is_null() {
                let old_pos = curwin_mut().w_cursor;
                let ptr = ml_get((*pos).lnum);
                let mut i = (*pos).col;
                if i > 0 {
                    while {
                        i -= 1;
                        i > 0 && ascii_iswhite(*ptr.add(i as usize))
                    } {}
                }
                curwin_mut().w_cursor.lnum = (*pos).lnum;
                curwin_mut().w_cursor.col = i;
                if *ptr.add(i as usize) == b')' as c_char {
                    let p2 = findmatch(ptr::null_mut(), b'(' as c_int);
                    if !p2.is_null() {
                        curwin_mut().w_cursor = *p2;
                    }
                }
                let ind = get_indent();
                curwin_mut().w_cursor = old_pos;
                if (g_st().State & VREPLACE_FLAG) != 0 {
                    change_indent(INDENT_SET, ind, 0, true);
                } else {
                    set_indent(ind, SIN_CHANGED);
                }
            } else if curwin_mut().w_cursor.col > 0 {
                let mut temp = true;
                if c == b'{' as c_int && g_st().Ins.can_si_back && curwin_mut().w_cursor.lnum > 1 {
                    let old_pos = curwin_mut().w_cursor;
                    let i = get_indent();
                    while curwin_mut().w_cursor.lnum > 1 {
                        curwin_mut().w_cursor.lnum -= 1;
                        let ptr = skipwhite(ml_get(curwin_mut().w_cursor.lnum));
                        if *ptr != b'#' as c_char && *ptr != NUL {
                            break;
                        }
                    }
                    if get_indent() >= i {
                        temp = false;
                    }
                    curwin_mut().w_cursor = old_pos;
                }
                if temp {
                    shift_line(true, false, 1, 1);
                }
            }
        }

        if curwin_mut().w_cursor.col > 0 && g_st().Ins.can_si && c == b'#' as c_int && inindent(0) {
            g_st().old_indent = get_indent();
            set_indent(0, SIN_CHANGED);
        }

        g_st().Ins.ai_col = g_st().Ins.ai_col.min(curwin_mut().w_cursor.col);
    }
}

/// Insert an indent (for `<Tab>` or `CTRL-T`) or delete an indent (for `CTRL-D`) (`change_indent`).
pub fn change_indent(type_: c_int, amount: c_int, round: c_int, call_changed_bytes: bool) {
    unsafe {
        let mut orig_col: colnr_T = 0;
        let mut orig_line: Vec<c_char> = Vec::new();

        if (g_st().State & VREPLACE_FLAG) != 0 {
            orig_line = std::slice::from_raw_parts(
                get_cursor_line_ptr(),
                get_cursor_line_len() as usize + 1,
            )
            .to_vec();
            orig_col = curwin_mut().w_cursor.col;
        }

        let save_p_list = curwin_mut().w_onebuf_opt.wo_list;
        curwin_mut().w_onebuf_opt.wo_list = 0;
        let vc = getvcol_nolist(&mut curwin_mut().w_cursor);
        let mut vcol: c_int = vc;

        let mut start_col: c_int = curwin_mut().w_cursor.col;

        let mut new_cursor_col: c_int = curwin_mut().w_cursor.col;
        beginline(BL_WHITE);
        new_cursor_col -= curwin_mut().w_cursor.col;

        let mut insstart_less: c_int = curwin_mut().w_cursor.col;

        if new_cursor_col < 0 {
            vcol = get_indent() - vcol;
        }

        if new_cursor_col > 0 {
            start_col = -1;
        }

        if type_ == INDENT_SET {
            set_indent(amount, if call_changed_bytes { SIN_CHANGED } else { 0 });
        } else {
            let save_State = g_st().State;
            if (g_st().State & VREPLACE_FLAG) != 0 {
                g_st().State = MODE_INSERT;
            }
            shift_line(
                type_ == INDENT_DEC,
                round != 0,
                1,
                c_int::from(call_changed_bytes),
            );
            g_st().State = save_State;
        }
        insstart_less -= curwin_mut().w_cursor.col;

        if new_cursor_col >= 0 {
            if new_cursor_col == 0 {
                insstart_less = MAXCOL;
            }
            new_cursor_col += curwin_mut().w_cursor.col;
        } else if (g_st().State & MODE_INSERT) == 0 {
            new_cursor_col = curwin_mut().w_cursor.col;
        } else {
            vcol = get_indent() - vcol;
            let end_vcol: colnr_T = vcol.max(0);
            curwin_mut().w_virtcol = end_vcol;

            new_cursor_col = 0;
            let line = get_cursor_line_ptr();
            vcol = 0;
            if *line != NUL {
                let mut csarg = CharsizeArg::new();
                let cstype: CSType = init_charsize_arg(&mut csarg, curwin_mut(), 0, line);
                let mut ci: StrCharInfo = utf_ptr2StrCharInfo(line);
                loop {
                    let cli: ClusterInfo = utf_ClusterInfo(ci);
                    let next_vcol = vcol
                        + win_charsize(
                            cstype,
                            vcol,
                            ci.ptr,
                            ci.chr.value,
                            &mut csarg,
                            cli.cells,
                        )
                        .width;
                    if next_vcol > end_vcol {
                        break;
                    }
                    vcol = next_vcol;
                    ci = cli.next;
                    if *ci.ptr == NUL {
                        break;
                    }
                }
                new_cursor_col = ci.ptr.offset_from(line) as c_int;
            }

            if vcol != curwin_mut().w_virtcol {
                curwin_mut().w_cursor.col = new_cursor_col;
                let ptrlen = (curwin_mut().w_virtcol - vcol) as usize;
                let mut ptr_vec = vec![b' ' as c_char; ptrlen + 1];
                ptr_vec[ptrlen] = NUL;
                new_cursor_col += ptrlen as c_int;
                ins_str(ptr_vec.as_mut_ptr(), ptrlen);
            }

            insstart_less = MAXCOL;
        }

        curwin_mut().w_onebuf_opt.wo_list = save_p_list;
        curwin_mut().w_cursor.col = new_cursor_col.max(0);
        curwin_mut().w_set_curswant = true;
        changed_cline_bef_curs(curwin_mut());

        if (g_st().State & MODE_INSERT) != 0 {
            if curwin_mut().w_cursor.lnum == g_st().Ins.start.lnum && g_st().Ins.start.col != 0 {
                if g_st().Ins.start.col <= insstart_less {
                    g_st().Ins.start.col = 0;
                } else {
                    g_st().Ins.start.col -= insstart_less;
                }
            }
            if g_st().Ins.ai_col <= insstart_less {
                g_st().Ins.ai_col = 0;
            } else {
                g_st().Ins.ai_col -= insstart_less;
            }
        }

        if replace_normal(g_st().State) && start_col >= 0 {
            while start_col > curwin_mut().w_cursor.col {
                replace_join(0);
                start_col -= 1;
            }
            while start_col < curwin_mut().w_cursor.col {
                replace_push_nul();
                start_col += 1;
            }
        }

        if (g_st().State & VREPLACE_FLAG) != 0 {
            let new_col = curwin_mut().w_cursor.col;
            let mut new_line =
                std::slice::from_raw_parts(get_cursor_line_ptr(), new_col as usize).to_vec();
            new_line.push(NUL);

            ml_replace(curwin_mut().w_cursor.lnum, orig_line.as_mut_ptr(), true);
            curwin_mut().w_cursor.col = orig_col;

            g_st().curbuf_splice_pending += 1;

            backspace_until_column(0);
            ins_bytes(new_line.as_mut_ptr());

            g_st().curbuf_splice_pending -= 1;

            let delta = orig_col - new_col;
            extmark_splice_cols(
                curbuf_mut(),
                curwin_mut().w_cursor.lnum - 1,
                new_col,
                if delta < 0 { -delta } else { 0 },
                if delta > 0 { delta } else { 0 },
                kExtmarkUndo,
            );
        }
    }
}

/// Copy the indent from `src` to the current line (and fill to `size`) (`copy_indent`).
pub fn copy_indent(size: c_int, src: *mut c_char) -> bool {
    unsafe {
        let mut p: *mut c_char = ptr::null_mut();
        let mut line_vec: Vec<c_char> = Vec::new();
        let mut line: *mut c_char = ptr::null_mut();
        let mut ind_len: c_int = 0;
        let mut line_len: c_int = 0;
        let mut tab_pad: c_int;

        for _round in 1..=2 {
            let mut todo = size;
            ind_len = 0;
            let mut ind_done: c_int = 0;
            let mut ind_col: c_int = 0;
            let mut s = src;

            while todo > 0 && ascii_iswhite(*s) {
                if *s == TAB {
                    tab_pad = tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                    if todo < tab_pad {
                        break;
                    }
                    todo -= tab_pad;
                    ind_done += tab_pad;
                    ind_col += tab_pad;
                } else {
                    todo -= 1;
                    ind_done += 1;
                    ind_col += 1;
                }
                ind_len += 1;

                if !p.is_null() {
                    *p = *s;
                    p = p.add(1);
                }
                s = s.add(1);
            }

            tab_pad = tabstop_padding(ind_done, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
            if todo >= tab_pad && curbuf_mut().b_p_et == 0 {
                todo -= tab_pad;
                ind_len += 1;
                ind_col += tab_pad;

                if !p.is_null() {
                    *p = TAB;
                    p = p.add(1);
                }
            }

            if curbuf_mut().b_p_et == 0 {
                loop {
                    tab_pad = tabstop_padding(ind_col, curbuf_mut().b_p_ts, &curbuf_mut().b_p_vts_array);
                    if todo < tab_pad {
                        break;
                    }
                    todo -= tab_pad;
                    ind_len += 1;
                    ind_col += tab_pad;
                    if !p.is_null() {
                        *p = TAB;
                        p = p.add(1);
                    }
                }
            }

            while todo > 0 {
                todo -= 1;
                ind_len += 1;

                if !p.is_null() {
                    *p = b' ' as c_char;
                    p = p.add(1);
                }
            }

            if p.is_null() {
                line_len = get_cursor_line_len() + 1;
                debug_assert!(ind_len + line_len >= 0);
                let line_size = (ind_len as usize)
                    .checked_add(line_len as usize)
                    .expect("line_size overflow");
                line_vec = vec![0 as c_char; line_size];
                line = line_vec.as_mut_ptr();
                p = line;
            }
        }

        ptr::copy(get_cursor_line_ptr(), p, line_len as usize);
        ml_replace(curwin_mut().w_cursor.lnum, line, true);
        drop(line_vec);
        curwin_mut().w_cursor.col = ind_len;
        true
    }
}

fn emsg_text_too_long() {
    emsg(gettext(e_resulting_text_too_long));
        if g_st().trylevel == 0 {
            g_st().got_int = true;
        }
}

/// `:retab` (`ex_retab`).
pub fn ex_retab(eap: &mut exarg_T) {
    unsafe {
        let mut got_tab = false;
        let mut num_spaces: c_int = 0;
        let mut start_col: c_int = 0;
        let mut start_vcol: i64 = 0;
        let mut new_line: *mut c_char = ptr::dangling_mut::<c_char>();
        let mut new_vts_array: Vec<colnr_T> = Vec::new();

        let mut first_line: linenr_T = 0;
        let mut last_line: linenr_T = 0;
        let mut is_indent_only = false;

        let mut ptr = eap.arg;
        if (crate::memory::cstr_prefix_bytes(ptr, 11) == &b"-indentonly"[..11]) && ascii_iswhite_or_nul(*ptr.add(11)) {
            is_indent_only = true;
            ptr = skipwhite(ptr.add(11));
        }

        let ts_start = ptr;
        let mut errbuf_storage = [0 as c_char; IOSIZE];
        let errbuf = CharBuf {
            data: errbuf_storage.as_mut_ptr(),
            size: IOSIZE,
        };
        let errmsg = tabstop_set(ptr, &mut new_vts_array, Some(errbuf));
        if !errmsg.is_null() {
            emsg(gettext(errmsg));
            return;
        }
        while ascii_isdigit(*ptr) || *ptr == b',' as c_char {
            ptr = ptr.add(1);
        }

        let new_ts_str = if new_vts_array.is_empty() {
            new_vts_array = curbuf_mut().b_p_vts_array.clone();
            None
        } else {
            Some(
                std::ffi::CString::new(std::slice::from_raw_parts(
                    ts_start.cast::<u8>(),
                    ptr.offset_from(ts_start) as usize,
                ))
                .unwrap_or_default(),
            )
        };

        let save_list = curwin_mut().w_onebuf_opt.wo_list;
        curwin_mut().w_onebuf_opt.wo_list = 0;

        let mut lnum = eap.line1;
        while !g_st().got_int && lnum <= eap.line2 {
            ptr = ml_get(lnum);
            let mut old_len = ml_get_len(lnum);
            let mut col: c_int = 0;
            let mut vcol: i64 = 0;
            let mut did_undo = false;
            loop {
                if ascii_iswhite(*ptr.add(col as usize)) {
                    if !got_tab && num_spaces == 0 {
                        start_vcol = vcol;
                        start_col = col;
                    }
                    if *ptr.add(col as usize) == b' ' as c_char {
                        num_spaces += 1;
                    } else {
                        got_tab = true;
                    }
                } else {
                    if got_tab || (eap.forceit != 0 && num_spaces > 1) {
                        num_spaces = (vcol - start_vcol) as c_int;
                        let mut len = num_spaces;
                        let mut num_tabs: c_int = 0;
                        if curbuf_mut().b_p_et == 0 {
                            let mut t: c_int = 0;
                            let mut s: c_int = 0;
                            tabstop_fromto(
                                start_vcol as colnr_T,
                                vcol as colnr_T,
                                curbuf_mut().b_p_ts as c_int,
                                &new_vts_array,
                                &mut t,
                                &mut s,
                            );
                            num_tabs = t;
                            num_spaces = s;
                        }
                        if curbuf_mut().b_p_et != 0 || got_tab || (num_spaces + num_tabs < len) {
                            if !did_undo {
                                did_undo = true;
                                if !u_save(lnum - 1, lnum + 1) {
                                    new_line = ptr::null_mut();
                                    break;
                                }
                            }

                            len = num_spaces + num_tabs;
                            let new_len = old_len - col + start_col + len + 1;
                            if new_len <= 0 || new_len == MAXCOL {
                                emsg_text_too_long();
                                break;
                            }
                            let mut new_line_vec = vec![0 as c_char; new_len as usize];
                            new_line = new_line_vec.as_mut_ptr();

                            if start_col > 0 {
                                ptr::copy(ptr, new_line, start_col as usize);
                            }
                            ptr::copy(
                                ptr.add(col as usize),
                                new_line.add((start_col + len) as usize),
                                (old_len - col + 1) as usize,
                            );
                            ptr = new_line.add(start_col as usize);
                            for c_idx in 0..len {
                                *ptr.add(c_idx as usize) = if c_idx < num_tabs {
                                    b'\t' as c_char
                                } else {
                                    b' ' as c_char
                                };
                            }
                            if ml_replace(lnum, new_line, true) {
                                new_line = curbuf_mut().b_ml.ml_line_ptr;
                                extmark_splice_cols(
                                    curbuf_mut(),
                                    lnum - 1,
                                    0,
                                    old_len,
                                    new_len - 1,
                                    kExtmarkUndo,
                                );
                            }
                            drop(new_line_vec);
                            if first_line == 0 {
                                first_line = lnum;
                            }
                            last_line = lnum;
                            ptr = new_line;
                            old_len = new_len - 1;
                            col = start_col + len;
                        }
                    }
                    got_tab = false;
                    num_spaces = 0;

                    if is_indent_only {
                        break;
                    }
                }
                if *ptr.add(col as usize) == NUL {
                    break;
                }
                vcol += i64::from(win_chartabsize(
                    curwin_mut(),
                    ptr.add(col as usize),
                    vcol as colnr_T,
                ));
                if vcol >= i64::from(MAXCOL) {
                    emsg_text_too_long();
                    break;
                }
                col += utfc_ptr2len(ptr.add(col as usize));
            }
            if new_line.is_null() {
                break;
            }
            line_breakcheck();
            lnum += 1;
        }
        if g_st().got_int {
            emsg(gettext(e_interr));
        }

        if first_line != 0 {
            changed_lines(curbuf_mut(), first_line, 0, last_line + 1, 0, true);
        }

        curwin_mut().w_onebuf_opt.wo_list = save_list;
        u_clearline(curbuf_mut());

        if let Some(ref ts_cstr) = new_ts_str {
            let win_handle: handle_T = curwin_mut().handle;
            let buf_handle: handle_T = curbuf_mut().b_fnum;

            if tabstop_count(&curbuf_mut().b_p_vts_array) > 0 || tabstop_count(&new_vts_array) > 1 {
                set_option_value(
                    kOptVartabstop,
                    cstr_as_obj(ts_cstr.as_ptr()),
                    OPT_LOCAL,
                    true,
                    None,
                );
            } else {
                set_option_value(
                    kOptTabstop,
                    integer_obj(i64::from(tabstop_first(&new_vts_array))),
                    OPT_LOCAL,
                    true,
                    None,
                );
            }
            if curwin_mut().handle != win_handle || curbuf_mut().b_fnum != buf_handle {
                return;
            }
        }
        coladvance(curwin_mut(), curwin_mut().w_curswant);
    }
}

/// Get indent level from `'indentexpr'` (`get_expr_indent`).
pub fn get_expr_indent() -> c_int {
     {
        let use_sandbox = was_set_insecurely(curwin_ptr(), kOptIndentexpr, OPT_LOCAL) != 0;
        let save_sctx = g_st().current_sctx;

        let save_pos = curwin_mut().w_cursor;
        let save_curswant = curwin_mut().w_curswant;
        let save_set_curswant = curwin_mut().w_set_curswant;
        set_vim_var_nr(VV_LNUM, i64::from(curwin_mut().w_cursor.lnum));

        g_st().textlock += 1;
        g_st().current_sctx = curbuf_mut().b_p_script_ctx[kBufOptIndentexpr];

        let mut indent = eval_expr_option_number(&mut curbuf_mut().b_p_inde, use_sandbox);

        g_st().textlock -= 1;
        g_st().current_sctx = save_sctx;

        let save_State = g_st().State;
        g_st().State = MODE_INSERT;
        curwin_mut().w_cursor = save_pos;
        curwin_mut().w_curswant = save_curswant;
        curwin_mut().w_set_curswant = save_set_curswant;
        check_cursor(curwin_mut());
        g_st().State = save_State;

        if g_st().did_throw && (vim_strchr(opt_vars().p_debug, b't' as c_int).is_null() || g_st().trylevel == 0) {
            handle_did_throw();
            g_st().did_throw = false;
        }

        if indent < 0 {
            indent = get_indent();
        }

        indent
    }
}

/// Compute Lisp indent for the current line (`get_lisp_indent`).
pub fn get_lisp_indent() -> c_int {
    unsafe {
        let mut paren: pos_T;
        let mut amount: c_int;

        let realpos = curwin_mut().w_cursor;
        curwin_mut().w_cursor.col = 0;

        let mut pos = findmatch(ptr::null_mut(), b'(' as c_int);
        if pos.is_null() {
            pos = findmatch(ptr::null_mut(), b'[' as c_int);
        } else {
            paren = *pos;
            pos = findmatch(ptr::null_mut(), b'[' as c_int);
            if pos.is_null() || lt(*pos, paren) {
                pos = &raw mut paren;
            }
        }

        if !pos.is_null() {
            amount = -1;
            let mut parencount: c_int = 0;

            while {
                curwin_mut().w_cursor.lnum -= 1;
                curwin_mut().w_cursor.lnum >= (*pos).lnum
            } {
                if linewhite(curwin_mut().w_cursor.lnum) {
                    continue;
                }

                let mut that = get_cursor_line_ptr();
                while *that != NUL {
                    if *that == b';' as c_char {
                        while *that.add(1) != NUL {
                            that = that.add(1);
                        }
                        that = that.add(1);
                        continue;
                    }

                    if *that == b'\\' as c_char {
                        if *that.add(1) != NUL {
                            that = that.add(1);
                        }
                        that = that.add(1);
                        continue;
                    }

                    if *that == b'"' as c_char && *that.add(1) != NUL {
                        loop {
                            that = that.add(1);
                            if *that == NUL || *that == b'"' as c_char {
                                break;
                            }
                            if *that == b'\\' as c_char {
                                that = that.add(1);
                                if *that == NUL {
                                    break;
                                }
                                if *that.add(1) == NUL {
                                    that = that.add(1);
                                    break;
                                }
                            }
                        }
                        if *that == NUL {
                            break;
                        }
                    }
                    if *that == b'(' as c_char || *that == b'[' as c_char {
                        parencount += 1;
                    } else if *that == b')' as c_char || *that == b']' as c_char {
                        parencount -= 1;
                    }
                    that = that.add(1);
                }

                if parencount == 0 {
                    amount = get_indent();
                    break;
                }
            }

            if amount == -1 {
                curwin_mut().w_cursor.lnum = (*pos).lnum;
                curwin_mut().w_cursor.col = (*pos).col;
                let mut col = (*pos).col;

                let line = get_cursor_line_ptr();
                let mut csarg = CharsizeArg::new();
                let cstype: CSType = init_charsize_arg(&mut csarg, curwin_mut(), (*pos).lnum, line);

                let mut sci: StrCharInfo = utf_ptr2StrCharInfo(line);
                amount = 0;
                while *sci.ptr != NUL && col > 0 {
                    let cli: ClusterInfo = utf_ClusterInfo(sci);
                    amount += win_charsize(
                        cstype,
                        amount,
                        sci.ptr,
                        sci.chr.value,
                        &mut csarg,
                        cli.cells,
                    )
                    .width;
                    sci = cli.next;
                    col -= 1;
                }
                let mut that = sci.ptr;

                if (*that == b'(' as c_char || *that == b'[' as c_char) && lisp_match(that.add(1)) {
                    amount += 2;
                } else {
                    if *that != NUL {
                        that = that.add(1);
                        amount += 1;
                    }
                    let mut firsttry: colnr_T = amount;

                    while ascii_iswhite(*that) {
                        amount += win_charsize(
                            cstype,
                            amount,
                            that,
                            i32::from(*that as u8),
                            &mut csarg,
                            1,
                        )
                        .width;
                        that = that.add(1);
                    }

                    if *that != NUL && *that != b';' as c_char {
                        if *that != b'(' as c_char && *that != b'[' as c_char {
                            firsttry += 1;
                        }

                        parencount = 0;

                        let mut ci: CharInfo = utf_ptr2CharInfo(that);
                        if ci.value != b'"' as i32
                            && ci.value != b'\'' as i32
                            && ci.value != b'#' as i32
                            && !(b'0' as i32..=b'9' as i32).contains(&ci.value)
                        {
                            let mut quotecount = false;
                            while *that != NUL
                                && (!ascii_iswhite(ci.value) || quotecount || parencount != 0)
                            {
                                if ci.value == b'"' as i32 {
                                    quotecount = !quotecount;
                                }
                                if (ci.value == b'(' as i32 || ci.value == b'[' as i32)
                                    && !quotecount
                                {
                                    parencount += 1;
                                }
                                if (ci.value == b')' as i32 || ci.value == b']' as i32)
                                    && !quotecount
                                {
                                    parencount -= 1;
                                }
                                if ci.value == b'\\' as i32 && *that.add(1) != NUL {
                                    let cli = utf_ClusterInfo(StrCharInfo { ptr: that, chr: ci });
                                    amount += win_charsize(
                                        cstype,
                                        amount,
                                        that,
                                        ci.value,
                                        &mut csarg,
                                        cli.cells,
                                    )
                                    .width;
                                    that = cli.next.ptr;
                                    ci = cli.next.chr;
                                }

                                let cli = utf_ClusterInfo(StrCharInfo { ptr: that, chr: ci });
                                amount += win_charsize(
                                    cstype,
                                    amount,
                                    that,
                                    ci.value,
                                    &mut csarg,
                                    cli.cells,
                                )
                                .width;
                                that = cli.next.ptr;
                                ci = cli.next.chr;
                            }
                        }

                        while ascii_iswhite(*that) {
                            amount += win_charsize(
                                cstype,
                                amount,
                                that,
                                i32::from(*that as u8),
                                &mut csarg,
                                1,
                            )
                            .width;
                            that = that.add(1);
                        }

                        if *that == NUL || *that == b';' as c_char {
                            amount = firsttry;
                        }
                    }
                }
            }
        } else {
            amount = 0;
        }
        curwin_mut().w_cursor = realpos;

        amount
    }
}

fn lisp_match(p: *const c_char) -> bool {
    unsafe {
        let mut buf = [0 as c_char; 512];
        let mut word = if !crate::memory::cstr_is_empty(curbuf_mut().b_p_lw) {
            curbuf_mut().b_p_lw
        } else {
            opt_vars().p_lispwords
        };

        while *word != NUL {
            let len = copy_option_part(&raw mut word, buf.as_mut_ptr(), buf.len(), c",");
            if (crate::memory::cstr_prefix_bytes(buf.as_ptr(), len) == crate::memory::cstr_prefix_bytes(p, len)) && ascii_iswhite_or_nul(*p.add(len)) {
                return true;
            }
        }
        false
    }
}

/// Re-indent the current line (`fixthisline`).
pub fn fixthisline(get_the_indent: IndentGetter) {
     {
        let amount = get_the_indent();
        if amount < 0 {
            return;
        }

        change_indent(INDENT_SET, amount, 0, true);
        if linewhite(curwin_mut().w_cursor.lnum) {
            g_st().Ins.did_ai = true;
        }
    }
}

/// Return true if `'indentexpr'` should be used for Lisp indenting (`use_indentexpr_for_lisp`).
pub fn use_indentexpr_for_lisp() -> bool {
     {
        curbuf_mut().b_p_lisp != 0
            && curbuf_mut().b_p_inde.cb_type() != kCallbackNone
            && (crate::memory::cstr_as_bytes(curbuf_mut().b_p_lop) == b"expr:1")
    }
}

/// Fix indent for `'lisp'` and `'cindent'` (`fix_indent`).
pub fn fix_indent() {
     {
        if opt_vars().p_paste != 0 {
            return;
        }
        if curbuf_mut().b_p_lisp != 0 && curbuf_mut().b_p_ai != 0 {
            if use_indentexpr_for_lisp() {
                do_c_expr_indent();
            } else {
                fixthisline(get_lisp_indent);
            }
        } else if cindent_on() {
            do_c_expr_indent();
        }
    }
}

/// `indent()` function (`f_indent`).
pub fn f_indent(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let lnum = tv_get_lnum(&argvars[0]);
        if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count {
            rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(get_indent_lnum(lnum)));
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }
    }
}

/// `lispindent(lnum)` function (`f_lispindent`).
pub fn f_lispindent(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        let pos = curwin_mut().w_cursor;
        let lnum = tv_get_lnum(&argvars[0]);
        if lnum >= 1 && lnum <= curbuf_mut().b_ml.ml_line_count {
            curwin_mut().w_cursor.lnum = lnum;
            rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(get_lisp_indent()));
            curwin_mut().w_cursor = pos;
        } else {
            rettv.vval = crate::eval::typval::TypvalVval::Number(-1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabstop_set_padding_and_at_work() {
        let mut vts: Vec<colnr_T> = Vec::new();
            assert!(tabstop_set(c"4,8", &mut vts, None).is_null());
            assert!(!vts.is_empty());
            assert_eq!(tabstop_count(&vts), 2);
            assert_eq!(tabstop_first(&vts), 4);
            assert_eq!(tabstop_padding(0, 8, &vts), 4);
            assert_eq!(tabstop_padding(4, 8, &vts), 8);
            assert_eq!(tabstop_at(0, 8, &vts, false), 4);

            assert_eq!(indent_size_ts(c"       a ", 100, &[]), 7);
            assert_eq!(indent_size_ts(c"   \t  \t \t\t   a ", 4, &[]), 19);
            assert_eq!(
                indent_size_ts(c"   \t  \t \t\t       a ", 4, &[]),
                23
            );
            let custom_vts = [7 as colnr_T, 2];
            assert_eq!(
                indent_size_ts(c"      \t  \t \t\t   a ", 4, &custom_vts),
                18
            );
    }
}
