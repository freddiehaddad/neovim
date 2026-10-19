//! Implementation of various operators: `op_shift`, `op_delete`, `op_tilde`,
//! `op_change`, `op_yank`, `do_join` (`src/nvim/ops.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::if_same_then_else,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::buffer::col_print;
#[allow(unused_imports)]
use crate::cursor::gchar_cursor;
use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::callback_call;
#[allow(unused_imports)]
use crate::eval::set_ref_in_callback;
#[allow(unused_imports)]
use crate::eval::typval::{callback_free, tv_clear, tv_dict_add_nr};
#[allow(unused_imports)]
use crate::ex_docmd::do_cmdline;
#[allow(unused_imports)]
use crate::ex_getln::getexline;
#[allow(unused_imports)]
use crate::fold::{deleteFold, foldCreate, foldOpenCursor, hasFolding, opFoldRange};
use crate::globals::*;
#[allow(unused_imports)]
use crate::input::beep_flush;
#[allow(unused_imports)]
use crate::insert::edit;
#[allow(unused_imports)]
use crate::insert::{beginline, display_dollar};
#[allow(unused_imports)]
use crate::memline::{
    dec, decl, inc, ml_append, ml_get, ml_get_buf_len, ml_get_buf_mut, ml_get_len, ml_get_pos,
    ml_replace, ml_replace_len,
};
#[allow(unused_imports)]
use crate::memline::{gchar_pos, ml_get_pos_len};
use crate::message::smsg;
#[allow(unused_imports)]
use crate::message::{emsg, internal_error, msg, msg_keep, msg_start, msgmore};
#[allow(unused_imports)]
use crate::normal::{
    clearop, clearopbeep, exec_stuffed, may_clear_cmdline, restore_visual_mode, unadjust_for_sel,
    unadjust_for_sel_inner, visualinfo,
};
#[allow(unused_imports)]
use crate::option::{get_equalprg, get_fileformat, get_ve_flags};
use crate::option_vars::*;
#[allow(unused_imports)]
use crate::state::virtual_active;
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::strings::vim_strchr;
#[allow(unused_imports)]
use crate::ui::vim_beep;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ptr;

use crate::api::autocmd::kCallbackNone;
use crate::api::tabpage::pos_T;
use crate::change::{
    appended_lines_mark, changed_bytes, changed_lines, del_bytes, del_char, del_lines,
    get_last_leader_offset, get_leader_len, ins_char, ins_str, truncate_line,
};
use crate::charset::{getwhitecols, getwhitecols_curline, skipwhite, uvarnumber_T, vim_str2nr};
use crate::clipboard::{TypvalVval, VarType, set_clipboard, typval_T, yankreg_T};
use crate::cursor::{
    check_cursor, check_cursor_col, check_pos, coladvance, coladvance_force, colnr_T, dec_cursor,
    get_cursor_line_len, get_cursor_line_ptr, get_cursor_pos_ptr, getviscol, getviscol2, getvpos,
    inc_cursor, linenr_T,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later, update_screen};
use crate::eval::buffer::varnumber_T;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::extmark::{ExtmarkOp, TriState, bcount_t, extmark_splice, extmark_splice_cols};
use crate::indent::{
    change_indent, fix_indent, get_expr_indent, get_indent, get_lisp_indent, get_sw_value_indent,
    inindent, may_do_si, op_reindent, preprocs_left, set_indent, tabstop_fromto,
    use_indentexpr_for_lisp,
};
use crate::indent_c::get_c_indent;
use crate::input::{
    RedoState, redo_append_str, redo_cancel, restore_redobuff, save_redobuff, stuffReadbuff,
    stuffcharReadbuff, stuffnumReadbuff,
};
use crate::input_cmdatom::{
    InsSession, VisualIns, atom_capture_op, atom_ins_end, atom_ins_start, atom_suppress,
    atom_visual_redoable, cmdarg_T, kVInsKeys, kVInsMotion, kVInsNone, kVInsOther,
};
use crate::keycodes::{K_COMMAND, KE_LUA, KS_EXTRA, termcap2key};
use crate::mark::{mark_col_adjust, mark_mb_adjustpos};
use crate::math::{FAIL, OK, trim_to_int};
use crate::mbyte::{
    bomb_size, mb_islower, mb_isupper, mb_prevptr, mb_tolower, mb_toupper, utf_char2bytes,
    utf_char2cells, utf_char2len, utf_eat_space, utf_head_off, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len,
};
use crate::mcursor::mc_zq;
use crate::memory::{xcalloc, xfree, xmalloc, xmallocz, xmemcpyz, xmemdupz};
use crate::mouse::setmouse;
use crate::r#move::validate_virtcol;
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::plines::{
    CSType, CharsizeArg, getvcol, getvcols, getvvcol, init_charsize_arg, linetabsize_col,
    utf_ClusterInfo, utf_ptr2StrCharInfo, win_charsize,
};
use crate::register::{
    YREG_YANK, block_def, do_autocmd_textyankpost, get_y_register, get_yank_register,
    is_append_register, op_yank, op_yank_reg, shift_delete_registers, valid_yank_reg,
};
use crate::state::{MODE_INSERT, MODE_REPLACE, VREPLACE_FLAG};
use crate::textformat::{
    MotionType, auto_format, has_format_option, op_format, op_formatexpr, oparg_T,
};
use crate::undo::{u_clearline, u_save, u_save_cursor};

pub type OptInt = i64;
pub type LineGetter = Option<unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>;

pub const OP_NOP: c_int = 0;
pub const OP_DELETE: c_int = 1;
pub const OP_YANK: c_int = 2;
pub const OP_CHANGE: c_int = 3;
pub const OP_LSHIFT: c_int = 4;
pub const OP_RSHIFT: c_int = 5;
pub const OP_FILTER: c_int = 6;
pub const OP_TILDE: c_int = 7;
pub const OP_INDENT: c_int = 8;
pub const OP_FORMAT: c_int = 9;
pub const OP_COLON: c_int = 10;
pub const OP_UPPER: c_int = 11;
pub const OP_LOWER: c_int = 12;
pub const OP_JOIN: c_int = 13;
pub const OP_JOIN_NS: c_int = 14;
pub const OP_ROT13: c_int = 15;
pub const OP_REPLACE: c_int = 16;
pub const OP_INSERT: c_int = 17;
pub const OP_APPEND: c_int = 18;
pub const OP_FOLD: c_int = 19;
pub const OP_FOLDOPEN: c_int = 20;
pub const OP_FOLDOPENREC: c_int = 21;
pub const OP_FOLDCLOSE: c_int = 22;
pub const OP_FOLDCLOSEREC: c_int = 23;
pub const OP_FOLDDEL: c_int = 24;
pub const OP_FOLDDELREC: c_int = 25;
pub const OP_FORMAT2: c_int = 26;
pub const OP_FUNCTION: c_int = 27;
pub const OP_NR_ADD: c_int = 28;
pub const OP_NR_SUB: c_int = 29;
pub const OP_MCURSOR: c_int = 30;

const OPF_LINES: u8 = 1;
const OPF_CHANGE: u8 = 2;

const NUL: c_char = 0;
const TAB: c_int = b'\t' as c_int;
const NL: c_int = b'\n' as c_int;
const CAR: c_int = b'\r' as c_int;
const Ctrl_A: c_int = 1;
const Ctrl_V: c_int = 22;
const Ctrl_X: c_int = 24;
const MAXCOL: colnr_T = 0x7fffffff;
const IOSIZE: usize = 1025;
const NUMBUFLEN: usize = 65;

const K_LUA: c_int = termcap2key(KS_EXTRA as c_int, KE_LUA as c_int);

const kMTCharWise: MotionType = 0;
const kMTLineWise: MotionType = 1;
const kMTBlockWise: MotionType = 2;

const kNone: TriState = -1;

const kExtmarkUndo: ExtmarkOp = 1;

const INDENT_SET: c_int = 1;
const SIN_CHANGED: c_int = 1;

const BL_WHITE: c_int = 1;
const BL_SOL: c_int = 2;
const BL_FIX: c_int = 4;

const ML_EMPTY: c_int = 0x01;
const CMOD_LOCKMARKS: c_int = 0x0800;

const REPLACE_CR_NCHAR: c_int = -1;
const REPLACE_NL_NCHAR: c_int = -2;

const CA_COMMAND_BUSY: c_int = 1;
const CA_NO_ADJ_OP_END: c_int = 2;

const VALID_WROW: c_int = 0x01;
const VALID_WCOL: c_int = 0x02;
const VALID_VIRTCOL: c_int = 0x04;

const COM_END: c_char = b'e' as c_char;

const STR2NR_BIN: c_int = 1 << 0;
const STR2NR_OCT: c_int = 1 << 1;
const STR2NR_HEX: c_int = 1 << 2;

const EOL_DOS: c_int = 1;

const kCpoDollar: c_int = b'$' as c_int;
const kCpoEmptyregion: c_int = b'E' as c_int;
const kCpoFilter: c_int = b'!' as c_int;
const kCpoJoincol: c_int = b'q' as c_int;
const kCpoYank: c_int = b'y' as c_int;

const kFoAuto: c_int = b'a' as c_int;
const kFoMbyteJoin: c_int = b'M' as c_int;
const kFoMbyteJoin2: c_int = b'B' as c_int;
const kFoRemoveComs: c_int = b'j' as c_int;

const kOptVeFlagAll: c_uint = 0x04;
const kOptVeFlagOnemore: c_uint = 0x08;
const kOptBoFlagOperator: c_uint = 0x4000;

const OPCHARS: [[u8; 3]; 31] = [
    [0, 0, 0],                            // OP_NOP
    [b'd', 0, OPF_CHANGE],                // OP_DELETE
    [b'y', 0, 0],                         // OP_YANK
    [b'c', 0, OPF_CHANGE],                // OP_CHANGE
    [b'<', 0, OPF_LINES | OPF_CHANGE],    // OP_LSHIFT
    [b'>', 0, OPF_LINES | OPF_CHANGE],    // OP_RSHIFT
    [b'!', 0, OPF_LINES | OPF_CHANGE],    // OP_FILTER
    [b'g', b'~', OPF_CHANGE],             // OP_TILDE
    [b'=', 0, OPF_LINES | OPF_CHANGE],    // OP_INDENT
    [b'g', b'q', OPF_LINES | OPF_CHANGE], // OP_FORMAT
    [b':', 0, OPF_LINES],                 // OP_COLON
    [b'g', b'U', OPF_CHANGE],             // OP_UPPER
    [b'g', b'u', OPF_CHANGE],             // OP_LOWER
    [b'J', 0, OPF_LINES | OPF_CHANGE],    // OP_JOIN
    [b'g', b'J', OPF_LINES | OPF_CHANGE], // OP_JOIN_NS
    [b'g', b'?', OPF_CHANGE],             // OP_ROT13
    [b'r', 0, OPF_CHANGE],                // OP_REPLACE
    [b'I', 0, OPF_CHANGE],                // OP_INSERT
    [b'A', 0, OPF_CHANGE],                // OP_APPEND
    [b'z', b'f', 0],                      // OP_FOLD
    [b'z', b'o', OPF_LINES],              // OP_FOLDOPEN
    [b'z', b'O', OPF_LINES],              // OP_FOLDOPENREC
    [b'z', b'c', OPF_LINES],              // OP_FOLDCLOSE
    [b'z', b'C', OPF_LINES],              // OP_FOLDCLOSEREC
    [b'z', b'd', OPF_LINES],              // OP_FOLDDEL
    [b'z', b'D', OPF_LINES],              // OP_FOLDDELREC
    [b'g', b'w', OPF_LINES | OPF_CHANGE], // OP_FORMAT2
    [b'g', b'@', OPF_CHANGE],             // OP_FUNCTION
    [Ctrl_A as u8, 0, OPF_CHANGE],        // OP_NR_ADD
    [Ctrl_X as u8, 0, OPF_CHANGE],        // OP_NR_SUB
    [b'z', b'q', 0],                      // OP_MCURSOR
];

static mut addsub_hexupper: bool = false;

unsafe extern "C" {

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn ngettext(msgid1: *const c_char, msgid2: *const c_char, n: c_ulong) -> *mut c_char;
}

#[inline]
unsafe fn tr(s: *const c_char) -> *const c_char {
    unsafe { gettext(s) }
}

#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int
}

#[inline]
const fn ascii_isdigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'9' as c_int
}

#[inline]
const fn ascii_isxdigit(c: c_int) -> bool {
    (c >= b'0' as c_int && c <= b'9' as c_int)
        || (c >= b'a' as c_int && c <= b'f' as c_int)
        || (c >= b'A' as c_int && c <= b'F' as c_int)
}

#[inline]
const fn ascii_isbdigit(c: c_int) -> bool {
    c == b'0' as c_int || c == b'1' as c_int
}

#[inline]
const fn ascii_isspace(c: c_int) -> bool {
    (c >= 9 && c <= 13) || c == b' ' as c_int
}

#[inline]
const fn ascii_isupper(c: c_int) -> bool {
    c >= b'A' as c_int && c <= b'Z' as c_int
}

#[inline]
const fn ascii_islower(c: c_int) -> bool {
    c >= b'a' as c_int && c <= b'z' as c_int
}

#[inline]
const fn ascii_isalpha(c: c_int) -> bool {
    ascii_isupper(c) || ascii_islower(c)
}

#[inline]
const fn char_ord(x: c_int) -> c_int {
    let b = x as u8;
    if b < b'a' {
        (b - b'A') as c_int
    } else {
        (b - b'a') as c_int
    }
}

#[inline]
const fn rot13(c: c_int, a: c_int) -> c_int {
    (((c - a) + 13) % 26) + a
}

#[inline]
const fn lt(a: pos_T, b: pos_T) -> bool {
    if a.lnum != b.lnum {
        a.lnum < b.lnum
    } else if a.col != b.col {
        a.col < b.col
    } else {
        a.coladd < b.coladd
    }
}

#[inline]
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
const fn ltoreq(a: pos_T, b: pos_T) -> bool {
    lt(a, b) || equalpos(a, b)
}

#[inline]
unsafe fn modifiable(buf: *const buf_T) -> bool {
    unsafe { (*buf).b_p_ma != 0 }
}

#[inline]
unsafe fn lineempty(lnum: linenr_T) -> bool {
    unsafe { *ml_get(lnum) == NUL }
}

#[inline]
unsafe fn linetabsize_str(s: *mut c_char) -> c_int {
    unsafe { linetabsize_col(0, s) }
}

#[inline]
unsafe fn buf_get_changedtick(buf: *const buf_T) -> varnumber_T {
    unsafe { (*buf).changedtick_di.di_tv.vval.v_number }
}

#[inline]
unsafe fn strcpy_raw(dst: *mut c_char, src: *const c_char) {
    unsafe {
        let len = CStr::from_ptr(src).to_bytes_with_nul().len();
        ptr::copy_nonoverlapping(src, dst, len);
    }
}

/// Translate a command name into an operator type.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_op_type(char1: c_int, char2: c_int) -> c_int {
    unsafe {
        if char1 == b'r' as c_int {
            return OP_REPLACE;
        }
        if char1 == b'~' as c_int {
            return OP_TILDE;
        }
        if char1 == b'g' as c_int && char2 == Ctrl_A {
            return OP_NR_ADD;
        }
        if char1 == b'g' as c_int && char2 == Ctrl_X {
            return OP_NR_SUB;
        }
        if char1 == b'z' as c_int && char2 == b'y' as c_int {
            return OP_YANK;
        }
        let mut i = 0;
        loop {
            if c_int::from(OPCHARS[i][0]) == char1 && c_int::from(OPCHARS[i][1]) == char2 {
                break;
            }
            if i == OPCHARS.len() - 1 {
                internal_error(c"get_op_type()".as_ptr());
                break;
            }
            i += 1;
        }
        i as c_int
    }
}

/// Return non-zero if operator `op` always works on whole lines.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_on_lines(op: c_int) -> c_int {
    c_int::from(OPCHARS[op as usize][2] & OPF_LINES)
}

/// Return non-zero if operator `op` changes text.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_is_change(op: c_int) -> c_int {
    c_int::from(OPCHARS[op as usize][2] & OPF_CHANGE)
}

/// Whether operator `op` builds a redo: text-changing operators do, ":" and the fold operators do
/// not. Yank with the 'y' flag in 'cpoptions'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_redoable(op: c_int, redo_yank: bool) -> bool {
    unsafe {
        if op == OP_YANK {
            redo_yank
        } else {
            op_is_change(op) != 0
        }
    }
}

/// Get first operator command character.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_op_char(optype: c_int) -> c_int {
    c_int::from(OPCHARS[optype as usize][0])
}

/// Get second operator command character.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_extra_op_char(optype: c_int) -> c_int {
    c_int::from(OPCHARS[optype as usize][1])
}

/// Handle a shift operation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_shift(oap: *mut oparg_T, curs_top: bool, amount: c_int) {
    unsafe {
        let mut block_col: c_int = 0;

        if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
            return;
        }

        if (*oap).motion_type == kMTBlockWise {
            block_col = (*curwin).w_cursor.col;
        }

        let mut i = (*oap).line_count - 1;
        while i >= 0 {
            let first_char = c_int::from(*get_cursor_line_ptr() as u8);
            if first_char == 0 {
                (*curwin).w_cursor.col = 0;
            } else if (*oap).motion_type == kMTBlockWise {
                shift_block(oap, amount);
            } else if first_char != b'#' as c_int || !preprocs_left() {
                shift_line((*oap).op_type == OP_LSHIFT, p_sr != 0, amount, 0);
            }
            (*curwin).w_cursor.lnum += 1;
            i -= 1;
        }

        if (*oap).motion_type == kMTBlockWise {
            (*curwin).w_cursor.lnum = (*oap).start.lnum;
            (*curwin).w_cursor.col = block_col;
        } else if curs_top {
            (*curwin).w_cursor.lnum = (*oap).start.lnum;
            beginline(BL_SOL | BL_FIX);
        } else {
            (*curwin).w_cursor.lnum -= 1;
        }
        foldOpenCursor();

        if i64::from((*oap).line_count) > p_report {
            let op = if (*oap).op_type == OP_RSHIFT {
                c">".as_ptr()
            } else {
                c"<".as_ptr()
            };

            let msg_line_single = ngettext(
                c"%lld line %sed %d time".as_ptr(),
                c"%lld line %sed %d times".as_ptr(),
                amount as c_ulong,
            );
            let msg_line_plural = ngettext(
                c"%lld lines %sed %d time".as_ptr(),
                c"%lld lines %sed %d times".as_ptr(),
                amount as c_ulong,
            );
            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                ngettext(
                    msg_line_single,
                    msg_line_plural,
                    (*oap).line_count as c_ulong,
                ),
                i64::from((*oap).line_count) as std::ffi::c_longlong,
                op,
                amount,
            );
            msg_keep(iobuff_ptr, 0, true, false);
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = (*oap).start;
            (*curbuf).b_op_end.lnum = (*oap).end.lnum;
            (*curbuf).b_op_end.col = ml_get_len((*oap).end.lnum);
            if (*curbuf).b_op_end.col > 0 {
                (*curbuf).b_op_end.col -= 1;
            }
        }

        changed_lines(curbuf, (*oap).start.lnum, 0, (*oap).end.lnum + 1, 0, true);
    }
}

unsafe fn get_vts(vts_array: *const c_int, index: c_int) -> c_int {
    unsafe {
        if index < 1 {
            0
        } else if index <= *vts_array {
            *vts_array.add(index as usize)
        } else {
            *vts_array.add(*vts_array as usize)
        }
    }
}

unsafe fn get_vts_sum(vts_array: *const c_int, index: c_int) -> c_int {
    unsafe {
        let mut sum: c_int = 0;
        let mut i: c_int = 1;
        while i <= index && i <= *vts_array {
            sum += *vts_array.add(i as usize);
            i += 1;
        }
        if i <= index {
            sum += *vts_array.add(*vts_array as usize) * (index - *vts_array);
        }
        sum
    }
}

unsafe fn get_new_sw_indent(left: bool, round: bool, mut amount: i64, sw_val: i64) -> i64 {
    unsafe {
        let mut count = i64::from(get_indent());

        if round {
            let mut i = i64::from(trim_to_int(count / sw_val));
            let j = i64::from(trim_to_int(count % sw_val));
            if j != 0 && left {
                amount -= 1;
            }
            if left {
                i = (i - amount).max(0);
            } else {
                i += amount;
            }
            count = i * sw_val;
        } else if left {
            count = (count - sw_val * amount).max(0);
        } else {
            count += sw_val * amount;
        }

        count
    }
}

unsafe fn get_new_vts_indent(
    left: bool,
    round: bool,
    amount: c_int,
    vts_array: *const c_int,
) -> i64 {
    unsafe {
        let mut indent = i64::from(get_indent());
        let mut vtsi: c_int = 0;
        let mut vts_indent: c_int = 0;
        let mut ts: c_int = 0;

        while i64::from(vts_indent) <= indent {
            vtsi += 1;
            ts = get_vts(vts_array, vtsi);
            vts_indent += ts;
        }
        vts_indent -= ts;
        vtsi -= 1;

        let offset = indent - i64::from(vts_indent);

        if round {
            if left {
                if offset == 0 {
                    indent = i64::from(get_vts_sum(vts_array, vtsi - amount));
                } else {
                    indent = i64::from(get_vts_sum(vts_array, vtsi - (amount - 1)));
                }
            } else {
                indent = i64::from(get_vts_sum(vts_array, vtsi + amount));
            }
        } else if left {
            if amount > vtsi {
                indent = 0;
            } else {
                indent = i64::from(get_vts_sum(vts_array, vtsi - amount)) + offset;
            }
        } else {
            indent = i64::from(get_vts_sum(vts_array, vtsi + amount)) + offset;
        }

        indent
    }
}

/// Shift the current line `amount` shiftwidth(s) left or right.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shift_line(
    left: bool,
    round: bool,
    amount: c_int,
    call_changed_bytes: c_int,
) {
    unsafe {
        let sw_val = (*curbuf).b_p_sw;
        let ts_val = (*curbuf).b_p_ts;
        let vts_array = (*curbuf).b_p_vts_array;

        let count = if sw_val != 0 {
            get_new_sw_indent(left, round, i64::from(amount), sw_val)
        } else if vts_array.is_null() || *vts_array == 0 {
            get_new_sw_indent(left, round, i64::from(amount), ts_val)
        } else {
            get_new_vts_indent(left, round, amount, vts_array)
        };

        if (State & VREPLACE_FLAG) != 0 {
            change_indent(INDENT_SET, trim_to_int(count), 0, call_changed_bytes != 0);
        } else {
            set_indent(
                trim_to_int(count),
                if call_changed_bytes != 0 {
                    SIN_CHANGED
                } else {
                    0
                },
            );
        }
    }
}

unsafe fn shift_block(oap: *mut oparg_T, amount: c_int) {
    unsafe {
        let left = (*oap).op_type == OP_LSHIFT;
        let oldstate = State;
        let oldcol = (*curwin).w_cursor.col;
        let sw_val = get_sw_value_indent(curbuf, left);
        let ts_val = (*curbuf).b_p_ts as c_int;
        let mut bd: block_def = std::mem::zeroed();
        let old_p_ri = p_ri;

        p_ri = 0;

        State = MODE_INSERT;
        block_prep(oap, &raw mut bd, (*curwin).w_cursor.lnum, true);
        if bd.is_short != 0 {
            return;
        }

        let mut total = (amount as c_uint).wrapping_mul(sw_val as c_uint) as c_int;
        if (total / sw_val) != amount {
            return;
        }

        let oldp = get_cursor_line_ptr();
        let old_line_len = get_cursor_line_len();

        let newp: *mut c_char;
        let startcol: c_int;
        let oldlen: c_int;
        let newlen: c_int;

        if !left {
            total += bd.pre_whitesp;
            let mut ws_vcol = bd.start_vcol - bd.pre_whitesp;
            let old_textstart = bd.textstart;
            if bd.startspaces != 0 {
                if utfc_ptr2len(bd.textstart) == 1 {
                    bd.textstart = bd.textstart.add(1);
                } else {
                    ws_vcol = 0;
                    bd.startspaces = 0;
                }
            }

            let mut csarg: CharsizeArg = std::mem::zeroed();
            let cstype: CSType = init_charsize_arg(
                &raw mut csarg,
                curwin,
                (*curwin).w_cursor.lnum,
                bd.textstart,
            );
            let mut ci = utf_ptr2StrCharInfo(bd.textstart);
            let mut vcol = bd.start_vcol;
            while ascii_iswhite(ci.chr.value) {
                let cli = utf_ClusterInfo(ci);
                let incr = win_charsize(
                    cstype,
                    vcol,
                    ci.ptr,
                    ci.chr.value,
                    &raw mut csarg,
                    cli.cells,
                )
                .width;
                ci = cli.next;
                total += incr;
                vcol += incr;
            }
            bd.textstart = ci.ptr.cast_mut();
            bd.start_vcol = vcol;

            let mut tabs: c_int = 0;
            let mut spaces: c_int = 0;
            if (*curbuf).b_p_et == 0 {
                tabstop_fromto(
                    ws_vcol,
                    ws_vcol + total,
                    ts_val,
                    (*curbuf).b_p_vts_array,
                    &raw mut tabs,
                    &raw mut spaces,
                );
            } else {
                spaces = total;
            }

            let col_pre = bd.pre_whitesp_c - c_int::from(bd.startspaces != 0);
            bd.textcol -= col_pre;

            let new_line_len = bd.textcol
                + tabs
                + spaces
                + (old_line_len - (bd.textstart.offset_from(oldp) as c_int));
            newp = xmalloc((new_line_len as usize) + 1).cast();
            ptr::copy(oldp, newp, bd.textcol as usize);
            startcol = bd.textcol;
            oldlen = (bd.textstart.offset_from(old_textstart) as c_int) + col_pre;
            newlen = tabs + spaces;
            ptr::write_bytes(newp.add(bd.textcol as usize), b'\t', tabs as usize);
            ptr::write_bytes(
                newp.add((bd.textcol + tabs) as usize),
                b' ',
                spaces as usize,
            );
            strcpy_raw(
                newp.add((bd.textcol + tabs + spaces) as usize),
                bd.textstart,
            );
            debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
        } else {
            let mut non_white = bd.textstart;

            if bd.startspaces != 0 {
                non_white = non_white.add(utfc_ptr2len(non_white) as usize);
            }

            let mut non_white_col = bd.start_vcol;

            let mut csarg: CharsizeArg = std::mem::zeroed();
            let mut cstype: CSType = init_charsize_arg(
                &raw mut csarg,
                curwin,
                (*curwin).w_cursor.lnum,
                bd.textstart,
            );
            while ascii_iswhite(c_int::from(*non_white as u8)) {
                let incr = win_charsize(
                    cstype,
                    non_white_col,
                    non_white,
                    i32::from(*non_white as u8),
                    &raw mut csarg,
                    1,
                )
                .width;
                non_white_col += incr;
                non_white = non_white.add(1);
            }

            let block_space_width = non_white_col - (*oap).start_vcol;
            let shift_amount = block_space_width.min(total);
            let destination_col = non_white_col - shift_amount;

            let mut verbatim_copy_end = bd.textstart;
            let mut verbatim_copy_width = bd.start_vcol;

            if bd.startspaces != 0 {
                verbatim_copy_width -= bd.start_char_vcols;
            }
            cstype = init_charsize_arg(&raw mut csarg, curwin, 0, bd.textstart);
            let mut ci = utf_ptr2StrCharInfo(verbatim_copy_end);
            while verbatim_copy_width < destination_col {
                let cli = utf_ClusterInfo(ci);
                let incr = win_charsize(
                    cstype,
                    verbatim_copy_width,
                    ci.ptr,
                    ci.chr.value,
                    &raw mut csarg,
                    cli.cells,
                )
                .width;
                if verbatim_copy_width + incr > destination_col {
                    break;
                }
                verbatim_copy_width += incr;
                ci = cli.next;
            }
            verbatim_copy_end = ci.ptr.cast_mut();

            debug_assert!(destination_col - verbatim_copy_width >= 0);
            let fill = destination_col - verbatim_copy_width;

            debug_assert!(verbatim_copy_end.offset_from(oldp) >= 0);
            let fixedlen = verbatim_copy_end.offset_from(oldp) as c_int;
            let new_line_len =
                fixedlen + fill + (old_line_len - (non_white.offset_from(oldp) as c_int));

            newp = xmalloc((new_line_len as usize) + 1).cast();
            startcol = fixedlen;
            oldlen = bd.textcol + (non_white.offset_from(bd.textstart) as c_int) - fixedlen;
            newlen = fill;
            ptr::copy(oldp, newp, fixedlen as usize);
            ptr::write_bytes(newp.add(fixedlen as usize), b' ', fill as usize);
            strcpy_raw(newp.add((fixedlen + fill) as usize), non_white);
            debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
        }
        ml_replace((*curwin).w_cursor.lnum, newp, false);
        changed_bytes((*curwin).w_cursor.lnum, bd.textcol);
        extmark_splice_cols(
            curbuf,
            (*curwin).w_cursor.lnum - 1,
            startcol,
            oldlen,
            newlen,
            kExtmarkUndo,
        );
        State = oldstate;
        (*curwin).w_cursor.col = oldcol;
        p_ri = old_p_ri;
    }
}

unsafe fn block_insert(
    oap: *mut oparg_T,
    s: *const c_char,
    slen: usize,
    b_insert: bool,
    bdp: *mut block_def,
) {
    unsafe {
        let mut ts_val: c_int;
        let mut count: c_int = 0;
        let mut spaces: c_int;
        let mut offset: colnr_T;
        let oldstate = State;
        State = MODE_INSERT;

        for lnum in ((*oap).start.lnum + 1)..=(*oap).end.lnum {
            block_prep(oap, bdp, lnum, true);
            if (*bdp).is_short != 0 && b_insert {
                continue;
            }

            let mut oldp = ml_get(lnum);

            if b_insert {
                ts_val = (*bdp).start_char_vcols;
                spaces = (*bdp).startspaces;
                if spaces != 0 {
                    count = ts_val - 1;
                }
                offset = (*bdp).textcol;
            } else {
                ts_val = (*bdp).end_char_vcols;
                if (*bdp).is_short == 0 {
                    spaces = if (*bdp).endspaces != 0 {
                        ts_val - (*bdp).endspaces
                    } else {
                        0
                    };
                    if spaces != 0 {
                        count = ts_val - 1;
                    }
                    offset = (*bdp).textcol + (*bdp).textlen - c_int::from(spaces != 0);
                } else {
                    if (*bdp).is_MAX == 0 {
                        spaces = ((*oap).end_vcol - (*bdp).end_vcol) + 1;
                    } else {
                        spaces = 0;
                    }
                    count = spaces;
                    offset = (*bdp).textcol + (*bdp).textlen;
                }
            }

            if spaces > 0 {
                offset -= utf_head_off(oldp, oldp.add(offset as usize));
            }
            spaces = spaces.max(0);

            debug_assert!(count >= 0);
            let alloc_len = (ml_get_len(lnum) as usize)
                + (spaces as usize)
                + slen
                + if spaces > 0 && (*bdp).is_short == 0 {
                    (ts_val - spaces) as usize
                } else {
                    0
                }
                + (count as usize)
                + 1;
            let newp: *mut c_char = xmalloc(alloc_len).cast();

            ptr::copy(oldp, newp, offset as usize);
            oldp = oldp.add(offset as usize);
            let startcol = offset;

            ptr::write_bytes(newp.add(offset as usize), b' ', spaces as usize);

            ptr::copy(s, newp.add((offset + spaces) as usize), slen);
            offset += slen as c_int;

            let mut skipped: c_int = 0;
            if spaces > 0 && (*bdp).is_short == 0 {
                if *oldp == b'\t' as c_char {
                    ptr::write_bytes(
                        newp.add((offset + spaces) as usize),
                        b' ',
                        (ts_val - spaces) as usize,
                    );
                    oldp = oldp.add(1);
                    count += 1;
                    skipped = 1;
                } else {
                    count = spaces;
                }
            }

            if spaces > 0 {
                offset += count;
            }
            strcpy_raw(newp.add(offset as usize), oldp);

            ml_replace(lnum, newp, false);
            extmark_splice_cols(
                curbuf,
                lnum - 1,
                startcol,
                skipped,
                offset - startcol,
                kExtmarkUndo,
            );

            if lnum == (*oap).end.lnum {
                (*curbuf).b_op_end.lnum = (*oap).end.lnum;
                (*curbuf).b_op_end.col = offset;
                if (*curbuf).b_visual.vi_end.coladd != 0 {
                    (*curbuf).b_visual.vi_end.col += (*curbuf).b_visual.vi_end.coladd;
                    (*curbuf).b_visual.vi_end.coladd = 0;
                }
            }
        }

        State = oldstate;

        if (*oap).start.lnum < (*oap).end.lnum {
            changed_lines(
                curbuf,
                (*oap).start.lnum + 1,
                0,
                (*oap).end.lnum + 1,
                0,
                true,
            );
        }
    }
}

/// Handle a delete operation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_delete(oap: *mut oparg_T) -> c_int {
    unsafe {
        let mut bd: block_def = std::mem::zeroed();
        let old_lcount = (*curbuf).b_ml.ml_line_count;

        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            return OK;
        }

        if (*oap).empty {
            return u_save_cursor();
        }

        if !modifiable(curbuf) {
            emsg(tr(e_modifiable.as_ptr()));
            return FAIL;
        }

        if Visual.select && (*oap).is_VIsual {
            (*oap).regname = Visual.select_reg;
        }

        mb_adjust_opend(oap);

        if (*oap).motion_type == kMTCharWise
            && !(*oap).is_VIsual
            && (*oap).line_count > 1
            && (*oap).motion_force == c_int::from(NUL)
            && (*oap).op_type == OP_DELETE
        {
            let mut ptr_line = ml_get((*oap).end.lnum).add((*oap).end.col as usize);
            if *ptr_line != NUL {
                ptr_line = ptr_line.add(usize::from((*oap).inclusive));
            }
            ptr_line = skipwhite(ptr_line);
            if *ptr_line == NUL && inindent(0) {
                (*oap).motion_type = kMTLineWise;
            }
        }

        let mut only_setmarks = false;
        if (*oap).motion_type != kMTLineWise
            && (*oap).line_count == 1
            && (*oap).op_type == OP_DELETE
            && *ml_get((*oap).start.lnum) == NUL
        {
            if virtual_op != 0 {
                only_setmarks = true;
            } else {
                if !vim_strchr(p_cpo, kCpoEmptyregion).is_null() {
                    beep_flush();
                }
                return OK;
            }
        }

        if !only_setmarks {
            if (*oap).regname != b'_' as c_int {
                let mut reg: *mut yankreg_T = ptr::null_mut();
                let mut did_yank = false;
                if (*oap).regname != 0 {
                    if !valid_yank_reg((*oap).regname, true) {
                        beep_flush();
                        return OK;
                    }
                    reg = get_yank_register((*oap).regname, YREG_YANK);
                    op_yank_reg(oap, false, reg, is_append_register((*oap).regname));
                    did_yank = true;
                }

                if (*oap).motion_type == kMTLineWise || (*oap).line_count > 1 || (*oap).use_reg_one
                {
                    shift_delete_registers(is_append_register((*oap).regname));
                    reg = get_y_register(1);
                    op_yank_reg(oap, false, reg, false);
                    did_yank = true;
                }

                if (*oap).regname == 0
                    && (*oap).motion_type != kMTLineWise
                    && (*oap).line_count == 1
                {
                    reg = get_yank_register(b'-' as c_int, YREG_YANK);
                    op_yank_reg(oap, false, reg, false);
                    did_yank = true;
                }

                if did_yank || (*oap).regname == 0 {
                    if reg.is_null() {
                        std::process::abort();
                    }
                    set_clipboard((*oap).regname, reg);
                    do_autocmd_textyankpost(oap, reg);
                }
            }

            if (*oap).motion_type == kMTBlockWise {
                if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
                    return FAIL;
                }

                for lnum in (*curwin).w_cursor.lnum..=(*oap).end.lnum {
                    block_prep(oap, &raw mut bd, lnum, true);
                    if bd.textlen == 0 {
                        continue;
                    }

                    if lnum == (*curwin).w_cursor.lnum {
                        (*curwin).w_cursor.col = bd.textcol + bd.startspaces;
                        (*curwin).w_cursor.coladd = 0;
                    }

                    let n = bd.textlen - bd.startspaces - bd.endspaces;
                    let oldp = ml_get(lnum);
                    let newp: *mut c_char = xmalloc(((ml_get_len(lnum) - n) as usize) + 1).cast();
                    ptr::copy(oldp, newp, bd.textcol as usize);
                    ptr::write_bytes(
                        newp.add(bd.textcol as usize),
                        b' ',
                        (bd.startspaces + bd.endspaces) as usize,
                    );
                    strcpy_raw(
                        newp.add((bd.textcol + bd.startspaces + bd.endspaces) as usize),
                        oldp.add((bd.textcol + bd.textlen) as usize),
                    );
                    ml_replace(lnum, newp, false);

                    extmark_splice_cols(
                        curbuf,
                        lnum - 1,
                        bd.textcol,
                        bd.textlen,
                        bd.startspaces + bd.endspaces,
                        kExtmarkUndo,
                    );
                }

                check_cursor_col(curwin);
                changed_lines(
                    curbuf,
                    (*curwin).w_cursor.lnum,
                    (*curwin).w_cursor.col,
                    (*oap).end.lnum + 1,
                    0,
                    true,
                );
                (*oap).line_count = 0;
            } else if (*oap).motion_type == kMTLineWise {
                if (*oap).op_type == OP_CHANGE {
                    if (*oap).line_count > 1 {
                        let lnum = (*curwin).w_cursor.lnum;
                        (*curwin).w_cursor.lnum += 1;
                        del_lines((*oap).line_count - 1, true);
                        (*curwin).w_cursor.lnum = lnum;
                    }
                    if u_save_cursor() == FAIL {
                        return FAIL;
                    }
                    if (*curbuf).b_p_ai != 0 {
                        beginline(BL_WHITE);
                        Ins.did_ai = true;
                        Ins.ai_col = (*curwin).w_cursor.col;
                    } else {
                        beginline(0);
                    }
                    truncate_line(0);
                    if (*oap).line_count > 1 {
                        u_clearline(curbuf);
                    }
                } else {
                    del_lines((*oap).line_count, true);
                    beginline(BL_WHITE | BL_FIX);
                    u_clearline(curbuf);
                }
            } else {
                if virtual_op != 0 {
                    if gchar_pos(&raw mut (*oap).start) == TAB {
                        let mut endcol: c_int = 0;
                        if u_save_cursor() == FAIL {
                            return FAIL;
                        }
                        if (*oap).line_count == 1 {
                            endcol = getviscol2((*oap).end.col, (*oap).end.coladd);
                        }
                        coladvance_force(getviscol2((*oap).start.col, (*oap).start.coladd));
                        (*oap).start = (*curwin).w_cursor;
                        if (*oap).line_count == 1 {
                            coladvance(curwin, endcol);
                            (*oap).end.col = (*curwin).w_cursor.col;
                            (*oap).end.coladd = (*curwin).w_cursor.coladd;
                            (*curwin).w_cursor = (*oap).start;
                        }
                    }

                    if gchar_pos(&raw mut (*oap).end) == TAB
                        && (*oap).end.coladd == 0
                        && (*oap).inclusive
                    {
                        if u_save((*oap).end.lnum - 1, (*oap).end.lnum + 1) == FAIL {
                            return FAIL;
                        }
                        (*curwin).w_cursor = (*oap).end;
                        coladvance_force(getviscol2((*oap).end.col, (*oap).end.coladd));
                        (*oap).end = (*curwin).w_cursor;
                        (*curwin).w_cursor = (*oap).start;
                    }
                    mb_adjust_opend(oap);
                }

                if (*oap).line_count == 1 {
                    if u_save_cursor() == FAIL {
                        return FAIL;
                    }

                    if !vim_strchr(p_cpo, kCpoDollar).is_null()
                        && (*oap).op_type == OP_CHANGE
                        && (*oap).end.lnum == (*curwin).w_cursor.lnum
                        && !(*oap).is_VIsual
                    {
                        display_dollar((*oap).end.col - c_int::from(!(*oap).inclusive));
                    }

                    let mut n =
                        (*oap).end.col - (*oap).start.col + 1 - c_int::from(!(*oap).inclusive);

                    if virtual_op != 0 {
                        let len = get_cursor_line_len();

                        if (*oap).end.coladd != 0
                            && (*oap).end.col >= len - 1
                            && !((*oap).start.coladd != 0 && (*oap).end.col >= len - 1)
                        {
                            n += 1;
                        }
                        if n == 0 && (*oap).start.coladd != (*oap).end.coladd {
                            n = 1;
                        }

                        if gchar_cursor() != c_int::from(NUL) {
                            (*curwin).w_cursor.coladd = 0;
                        }
                    }

                    del_bytes(
                        n,
                        virtual_op == 0,
                        (*oap).op_type == OP_DELETE && !(*oap).is_VIsual,
                    );
                } else {
                    if u_save(
                        (*curwin).w_cursor.lnum - 1,
                        (*curwin).w_cursor.lnum + (*oap).line_count,
                    ) == FAIL
                    {
                        return FAIL;
                    }

                    curbuf_splice_pending += 1;
                    let startpos = (*curwin).w_cursor;
                    let deleted_bytes = get_region_bytecount(
                        curbuf,
                        startpos.lnum,
                        (*oap).end.lnum,
                        startpos.col,
                        (*oap).end.col,
                    ) + isize::from((*oap).inclusive);
                    truncate_line(1);

                    let curpos = (*curwin).w_cursor;
                    (*curwin).w_cursor.lnum += 1;

                    del_lines((*oap).line_count - 2, false);

                    let n = (*oap).end.col + 1 - c_int::from(!(*oap).inclusive);
                    (*curwin).w_cursor.col = 0;
                    del_bytes(
                        n,
                        virtual_op == 0,
                        (*oap).op_type == OP_DELETE && !(*oap).is_VIsual,
                    );
                    (*curwin).w_cursor = curpos;
                    do_join(2, false, false, false, false);
                    curbuf_splice_pending -= 1;
                    extmark_splice(
                        curbuf,
                        startpos.lnum - 1,
                        startpos.col,
                        (*oap).line_count - 1,
                        n,
                        deleted_bytes,
                        0,
                        0,
                        0,
                        kExtmarkUndo,
                    );
                }
                if (*oap).op_type == OP_DELETE {
                    auto_format(false, true);
                }
            }

            msgmore((*curbuf).b_ml.ml_line_count - old_lcount);
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            if (*oap).motion_type == kMTBlockWise {
                (*curbuf).b_op_end.lnum = (*oap).end.lnum;
                (*curbuf).b_op_end.col = (*oap).start.col;
            } else {
                (*curbuf).b_op_end = (*oap).start;
            }
            (*curbuf).b_op_start = (*oap).start;
        }

        OK
    }
}

unsafe fn mb_adjust_opend(oap: *mut oparg_T) {
    unsafe {
        if !(*oap).inclusive {
            return;
        }

        let line = ml_get((*oap).end.lnum);
        let mut ptr_line = line.add((*oap).end.col as usize);
        if *ptr_line != NUL {
            ptr_line = ptr_line.sub(utf_head_off(line, ptr_line) as usize);
            ptr_line = ptr_line.add((utfc_ptr2len(ptr_line) - 1) as usize);
            (*oap).end.col = ptr_line.offset_from(line) as colnr_T;
        }
    }
}

unsafe fn pbyte(mut lp: pos_T, c: c_int) {
    unsafe {
        debug_assert!(c <= u8::MAX as c_int);
        let p = ml_get_buf_mut(curbuf, lp.lnum);
        let len = (*curbuf).b_ml.ml_line_textlen;

        if lp.col >= len {
            lp.col = if len > 1 { len - 2 } else { 0 };
        }
        *p.add(lp.col as usize) = c as c_char;
        if curbuf_splice_pending == 0 {
            extmark_splice_cols(curbuf, lp.lnum - 1, lp.col, 1, 1, kExtmarkUndo);
        }
    }
}

unsafe fn replace_character(c: c_int) {
    unsafe {
        let n = State;

        State = MODE_REPLACE;
        ins_char(c);
        State = n;
        dec_cursor();
    }
}

unsafe fn op_replace(oap: *mut oparg_T, mut c: c_int) -> c_int {
    unsafe {
        let mut n: c_int;
        let mut bd: block_def = std::mem::zeroed();
        let mut after_p: *mut c_char = ptr::null_mut();
        let mut had_ctrl_v_cr = false;

        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 || (*oap).empty {
            return OK;
        }
        if c == REPLACE_CR_NCHAR {
            had_ctrl_v_cr = true;
            c = CAR;
        } else if c == REPLACE_NL_NCHAR {
            had_ctrl_v_cr = true;
            c = NL;
        }

        mb_adjust_opend(oap);

        if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
            return FAIL;
        }

        if (*oap).motion_type == kMTBlockWise {
            bd.is_MAX = c_int::from((*curwin).w_curswant == MAXCOL);
            while (*curwin).w_cursor.lnum <= (*oap).end.lnum {
                (*curwin).w_cursor.col = 0;
                block_prep(oap, &raw mut bd, (*curwin).w_cursor.lnum, true);
                if bd.textlen == 0 && (virtual_op == 0 || bd.is_MAX != 0) {
                    (*curwin).w_cursor.lnum += 1;
                    continue;
                }

                if virtual_op != 0 && bd.is_short != 0 && *bd.textstart == NUL {
                    let mut vpos = pos_T {
                        lnum: (*curwin).w_cursor.lnum,
                        col: 0,
                        coladd: 0,
                    };
                    getvpos(curwin, &raw mut vpos, (*oap).start_vcol);
                    bd.startspaces += vpos.coladd;
                    n = bd.startspaces;
                } else {
                    n = if bd.startspaces != 0 {
                        bd.start_char_vcols - 1
                    } else {
                        0
                    };
                }

                n += if bd.endspaces != 0 && bd.is_oneChar == 0 && bd.end_char_vcols > 0 {
                    bd.end_char_vcols - 1
                } else {
                    0
                };
                let mut numc = (*oap).end_vcol - (*oap).start_vcol + 1;
                if bd.is_short != 0 && (virtual_op == 0 || bd.is_MAX != 0) {
                    numc -= ((*oap).end_vcol - bd.end_vcol) + 1;
                }

                if utf_char2cells(c) > 1 {
                    if (numc & 1) != 0 && bd.is_short == 0 {
                        bd.endspaces += 1;
                        n += 1;
                    }
                    numc /= 2;
                }
                let _ = n;

                let mut num_chars = numc;
                numc *= utf_char2len(c);

                let mut oldp = get_cursor_line_ptr();
                let oldlen = get_cursor_line_len();

                let mut newp_size = (bd.textcol as usize) + (bd.startspaces as usize);
                if had_ctrl_v_cr || (c != b'\r' as c_int && c != b'\n' as c_int) {
                    newp_size += numc as usize;
                    if bd.is_short == 0 {
                        newp_size += (bd.endspaces + oldlen - bd.textcol - bd.textlen) as usize;
                    }
                }
                let newp: *mut c_char = xmallocz(newp_size).cast();
                ptr::copy(oldp, newp, bd.textcol as usize);
                oldp = oldp.add((bd.textcol + bd.textlen) as usize);
                ptr::write_bytes(newp.add(bd.textcol as usize), b' ', bd.startspaces as usize);
                let mut after_p_len: usize = 0;
                let col = oldlen - bd.textcol - bd.textlen + 1;
                debug_assert!(col >= 0);
                let mut newrows: c_int = 0;
                let mut newcols: c_int = 0;
                if had_ctrl_v_cr || (c != b'\r' as c_int && c != b'\n' as c_int) {
                    let mut newp_len = bd.textcol + bd.startspaces;
                    while num_chars > 0 {
                        num_chars -= 1;
                        newp_len += utf_char2bytes(c, newp.add(newp_len as usize));
                    }
                    if bd.is_short == 0 {
                        ptr::write_bytes(newp.add(newp_len as usize), b' ', bd.endspaces as usize);
                        newp_len += bd.endspaces;
                        ptr::copy(oldp, newp.add(newp_len as usize), col as usize);
                    }
                    newcols = newp_len - bd.textcol;
                } else {
                    after_p_len = col as usize;
                    after_p = xmalloc(after_p_len).cast();
                    ptr::copy(oldp, after_p, after_p_len);
                    newrows = 1;
                }
                ml_replace((*curwin).w_cursor.lnum, newp, false);
                curbuf_splice_pending += 1;
                let baselnum = (*curwin).w_cursor.lnum;
                if !after_p.is_null() {
                    let cur_lnum = (*curwin).w_cursor.lnum;
                    (*curwin).w_cursor.lnum += 1;
                    ml_append(cur_lnum, after_p, after_p_len as c_int, false);
                    appended_lines_mark((*curwin).w_cursor.lnum, 1);
                    (*oap).end.lnum += 1;
                    xfree(after_p.cast());
                    after_p = ptr::null_mut();
                }
                curbuf_splice_pending -= 1;
                extmark_splice(
                    curbuf,
                    baselnum - 1,
                    bd.textcol,
                    0,
                    bd.textlen,
                    bd.textlen as isize,
                    newrows,
                    newcols,
                    (newrows + newcols) as isize,
                    kExtmarkUndo,
                );
                (*curwin).w_cursor.lnum += 1;
            }
        } else {
            if (*oap).motion_type == kMTLineWise {
                (*oap).start.col = 0;
                (*curwin).w_cursor.col = 0;
                (*oap).end.col = ml_get_len((*oap).end.lnum);
                if (*oap).end.col != 0 {
                    (*oap).end.col -= 1;
                }
            } else if !(*oap).inclusive {
                dec(&raw mut (*oap).end);
            }

            while ltoreq((*curwin).w_cursor, (*oap).end) {
                let mut done = false;

                n = gchar_cursor();
                if n != c_int::from(NUL) {
                    let new_byte_len = utf_char2len(c);
                    let old_byte_len = utfc_ptr2len(get_cursor_pos_ptr());

                    if new_byte_len > 1 || old_byte_len > 1 {
                        if (*curwin).w_cursor.lnum == (*oap).end.lnum {
                            (*oap).end.col += new_byte_len - old_byte_len;
                        }
                        replace_character(c);
                        done = true;
                    } else {
                        if n == TAB {
                            let mut end_vcol: c_int = 0;

                            if (*curwin).w_cursor.lnum == (*oap).end.lnum {
                                end_vcol = getviscol2((*oap).end.col, (*oap).end.coladd);
                            }
                            coladvance_force(getviscol());
                            if (*curwin).w_cursor.lnum == (*oap).end.lnum {
                                getvpos(curwin, &raw mut (*oap).end, end_vcol);
                            }
                        }
                        if gchar_cursor() != c_int::from(NUL) {
                            pbyte((*curwin).w_cursor, c);
                            done = true;
                        }
                    }
                }
                if !done && virtual_op != 0 && (*curwin).w_cursor.lnum == (*oap).end.lnum {
                    let mut virtcols = (*oap).end.coladd;

                    if (*curwin).w_cursor.lnum == (*oap).start.lnum
                        && (*oap).start.col == (*oap).end.col
                        && (*oap).start.coladd != 0
                    {
                        virtcols -= (*oap).start.coladd;
                    }

                    coladvance_force(getviscol2((*oap).end.col, (*oap).end.coladd) + 1);
                    (*curwin).w_cursor.col -= virtcols + 1;
                    while virtcols >= 0 {
                        if utf_char2len(c) > 1 {
                            replace_character(c);
                        } else {
                            pbyte((*curwin).w_cursor, c);
                        }
                        if inc(&raw mut (*curwin).w_cursor) == -1 {
                            break;
                        }
                        virtcols -= 1;
                    }
                }

                if inc_cursor() == -1 {
                    break;
                }
            }
        }

        (*curwin).w_cursor = (*oap).start;
        check_cursor(curwin);
        changed_lines(
            curbuf,
            (*oap).start.lnum,
            (*oap).start.col,
            (*oap).end.lnum + 1,
            0,
            true,
        );

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = (*oap).start;
            (*curbuf).b_op_end = (*oap).end;
        }

        OK
    }
}

/// Handle the (non-standard vi) tilde operator. Also for "gu", "gU" and "g?".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_tilde(oap: *mut oparg_T) {
    unsafe {
        let mut bd: block_def = std::mem::zeroed();
        let mut did_change = false;

        if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
            return;
        }

        let mut pos = (*oap).start;
        if (*oap).motion_type == kMTBlockWise {
            while pos.lnum <= (*oap).end.lnum {
                block_prep(oap, &raw mut bd, pos.lnum, false);
                pos.col = bd.textcol;
                let one_change = swapchars((*oap).op_type, &raw mut pos, bd.textlen) != 0;
                did_change |= one_change;
                pos.lnum += 1;
            }
            if did_change {
                changed_lines(curbuf, (*oap).start.lnum, 0, (*oap).end.lnum + 1, 0, true);
            }
        } else {
            if (*oap).motion_type == kMTLineWise {
                (*oap).start.col = 0;
                pos.col = 0;
                (*oap).end.col = ml_get_len((*oap).end.lnum);
                if (*oap).end.col != 0 {
                    (*oap).end.col -= 1;
                }
            } else if !(*oap).inclusive {
                dec(&raw mut (*oap).end);
            }

            if pos.lnum == (*oap).end.lnum {
                did_change =
                    swapchars((*oap).op_type, &raw mut pos, (*oap).end.col - pos.col + 1) != 0;
            } else {
                loop {
                    let len = if pos.lnum == (*oap).end.lnum {
                        (*oap).end.col + 1
                    } else {
                        ml_get_pos_len(&raw mut pos)
                    };
                    did_change |= swapchars((*oap).op_type, &raw mut pos, len) != 0;
                    if ltoreq((*oap).end, pos) || inc(&raw mut pos) == -1 {
                        break;
                    }
                }
            }
            if did_change {
                changed_lines(
                    curbuf,
                    (*oap).start.lnum,
                    (*oap).start.col,
                    (*oap).end.lnum + 1,
                    0,
                    true,
                );
            }
        }

        if !did_change && (*oap).is_VIsual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = (*oap).start;
            (*curbuf).b_op_end = (*oap).end;
        }

        if i64::from((*oap).line_count) > p_report {
            smsg!(
                0,
                ngettext(
                    c"%lld line changed".as_ptr(),
                    c"%lld lines changed".as_ptr(),
                    (*oap).line_count as c_ulong,
                ),
                i64::from((*oap).line_count) as std::ffi::c_longlong,
            );
        }
    }
}

unsafe fn swapchars(op_type: c_int, pos: *mut pos_T, length: c_int) -> c_int {
    unsafe {
        let mut did_change: c_int = 0;
        let mut todo = length;
        while todo > 0 {
            let len = utfc_ptr2len(ml_get_pos(pos));

            if len > 0 {
                todo -= len - 1;
            }
            did_change |= c_int::from(swapchar(op_type, pos));
            if inc(pos) == -1 {
                break;
            }
            todo -= 1;
        }
        did_change
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn swapchar(op_type: c_int, pos: *mut pos_T) -> bool {
    unsafe {
        let c = gchar_pos(pos);

        if c >= 0x80 && op_type == OP_ROT13 {
            return false;
        }

        let mut nc = c;
        if mb_islower(c) {
            if op_type == OP_ROT13 {
                nc = rot13(c, b'a' as c_int);
            } else if op_type != OP_LOWER {
                nc = mb_toupper(c);
            }
        } else if mb_isupper(c) {
            if op_type == OP_ROT13 {
                nc = rot13(c, b'A' as c_int);
            } else if op_type != OP_UPPER {
                nc = mb_tolower(c);
            }
        }
        if nc != c {
            if c >= 0x80 || nc >= 0x80 {
                let sp = (*curwin).w_cursor;

                (*curwin).w_cursor = *pos;
                del_bytes(utf_ptr2len(get_cursor_pos_ptr()), false, false);
                ins_char(nc);
                (*curwin).w_cursor = sp;
            } else {
                pbyte(*pos, nc);
            }
            return true;
        }
        false
    }
}

/// Insert and append operators for Visual mode.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_insert(oap: *mut oparg_T, count1: c_int) {
    unsafe {
        let mut pre_textlen: c_int = 0;
        let mut ind_pre_col: colnr_T = 0;
        let mut ind_pre_vcol: c_int = 0;
        let mut bd: block_def = std::mem::zeroed();

        bd.is_MAX = c_int::from((*curwin).w_curswant == MAXCOL);

        (*curwin).w_cursor.lnum = (*oap).start.lnum;
        redraw_curbuf_later(UPD_INVERTED);
        update_screen();

        if (*oap).motion_type == kMTBlockWise {
            if (*curwin).w_cursor.coladd > 0 {
                let old_ve_flags = (*curwin).w_onebuf_opt.wo_ve_flags;

                if u_save_cursor() == FAIL {
                    return;
                }
                (*curwin).w_onebuf_opt.wo_ve_flags = kOptVeFlagAll;
                coladvance_force(if (*oap).op_type == OP_APPEND {
                    (*oap).end_vcol + 1
                } else {
                    getviscol()
                });
                if (*oap).op_type == OP_APPEND {
                    (*curwin).w_cursor.col -= 1;
                }
                (*curwin).w_onebuf_opt.wo_ve_flags = old_ve_flags;
            }
            block_prep(oap, &raw mut bd, (*oap).start.lnum, true);
            ind_pre_col = getwhitecols_curline() as colnr_T;
            ind_pre_vcol = get_indent();
            pre_textlen = ml_get_len((*oap).start.lnum) - bd.textcol;
            if (*oap).op_type == OP_APPEND {
                pre_textlen -= bd.textlen;
            }
        }

        if (*oap).op_type == OP_APPEND {
            if (*oap).motion_type == kMTBlockWise && (*curwin).w_cursor.coladd == 0 {
                (*curwin).w_set_curswant = true;
                while *get_cursor_pos_ptr() != NUL
                    && (*curwin).w_cursor.col < bd.textcol + bd.textlen
                {
                    (*curwin).w_cursor.col += 1;
                }
                if bd.is_short != 0 && bd.is_MAX == 0 {
                    if u_save_cursor() == FAIL {
                        return;
                    }
                    for _ in 0..bd.endspaces {
                        ins_char(b' ' as c_int);
                    }
                    bd.textlen += bd.endspaces;
                }
            } else {
                (*curwin).w_cursor = (*oap).end;
                check_cursor_col(curwin);

                if !lineempty((*curwin).w_cursor.lnum) && (*oap).start_vcol != (*oap).end_vcol {
                    inc_cursor();
                }
            }
        }

        let t1 = (*oap).start;
        let start_insert = (*curwin).w_cursor;
        edit(c_int::from(NUL), false, count1);

        if t1.lnum == (*curbuf).b_op_start_orig.lnum && lt((*curbuf).b_op_start_orig, t1) {
            (*oap).start = (*curbuf).b_op_start_orig;
        }

        if (*curwin).w_cursor.lnum != (*oap).start.lnum || got_int {
            return;
        }

        if (*oap).motion_type == kMTBlockWise {
            let mut ind_post_vcol: c_int = 0;
            let mut bd2: block_def = std::mem::zeroed();
            let mut did_indent = false;

            let ind_post_col = getwhitecols_curline() as colnr_T;
            if (*curbuf).b_op_start.col > ind_pre_col && ind_post_col > ind_pre_col {
                bd.textcol += ind_post_col - ind_pre_col;
                ind_post_vcol = get_indent();
                bd.start_vcol += ind_post_vcol - ind_pre_vcol;
                did_indent = true;
            }

            if (*oap).start.lnum == (*curbuf).b_op_start_orig.lnum && bd.is_MAX == 0 && !did_indent
            {
                let t = getviscol2(
                    (*curbuf).b_op_start_orig.col,
                    (*curbuf).b_op_start_orig.coladd,
                );

                if (*oap).op_type == OP_INSERT
                    && (*oap).start.col + (*oap).start.coladd
                        != (*curbuf).b_op_start_orig.col + (*curbuf).b_op_start_orig.coladd
                {
                    (*oap).start.col = (*curbuf).b_op_start_orig.col;
                    pre_textlen -= t - (*oap).start_vcol;
                    (*oap).start_vcol = t;
                } else if (*oap).op_type == OP_APPEND
                    && (*oap).start.col + (*oap).start.coladd
                        >= (*curbuf).b_op_start_orig.col + (*curbuf).b_op_start_orig.coladd
                {
                    (*oap).start.col = (*curbuf).b_op_start_orig.col;
                    pre_textlen += bd.textlen;
                    pre_textlen -= t - (*oap).start_vcol;
                    (*oap).start_vcol = t;
                    (*oap).op_type = OP_INSERT;
                }
            }

            if did_indent && bd.textcol - ind_post_col > 0 {
                (*oap).start.col += ind_post_col - ind_pre_col;
                (*oap).start_vcol += ind_post_vcol - ind_pre_vcol;
                (*oap).end.col += ind_post_col - ind_pre_col;
                (*oap).end_vcol += ind_post_vcol - ind_pre_vcol;
            }
            block_prep(oap, &raw mut bd2, (*oap).start.lnum, true);
            if did_indent && bd.textcol - ind_post_col > 0 {
                (*oap).start.col -= ind_post_col - ind_pre_col;
                (*oap).start_vcol -= ind_post_vcol - ind_pre_vcol;
                (*oap).end.col -= ind_post_col - ind_pre_col;
                (*oap).end_vcol -= ind_post_vcol - ind_pre_vcol;
            }
            if bd.is_MAX == 0 || bd2.textlen < bd.textlen {
                if (*oap).op_type == OP_APPEND {
                    pre_textlen += bd2.textlen - bd.textlen;
                    if bd2.endspaces != 0 {
                        bd2.textlen -= 1;
                    }
                }
                bd.textcol = bd2.textcol;
                bd.textlen = bd2.textlen;
            }

            let mut firstline = ml_get((*oap).start.lnum);
            let mut len = ml_get_len((*oap).start.lnum);
            let mut add = bd.textcol;
            let mut offset: colnr_T = 0;
            if (*oap).op_type == OP_APPEND {
                add += bd.textlen;
                if bd.is_MAX != 0
                    && start_insert.lnum == Ins.start.lnum
                    && start_insert.col > Ins.start.col
                {
                    offset = start_insert.col - Ins.start.col;
                    add -= offset;
                    if (*oap).end_vcol > offset {
                        (*oap).end_vcol -= offset + 1;
                    } else {
                        return;
                    }
                }
            }
            add = add.min(len);
            firstline = firstline.add(add as usize);
            len -= add;
            let ins_len = len - pre_textlen - offset;
            if pre_textlen >= 0 && ins_len > 0 {
                let ins_text: *mut c_char = xmemdupz(firstline.cast(), ins_len as usize).cast();
                if u_save((*oap).start.lnum, (*oap).end.lnum + 1) == OK {
                    block_insert(
                        oap,
                        ins_text,
                        ins_len as usize,
                        (*oap).op_type == OP_INSERT,
                        &raw mut bd,
                    );
                }

                (*curwin).w_cursor.col = (*oap).start.col;
                check_cursor(curwin);
                xfree(ins_text.cast());
            }
        }
    }
}

/// Handle a change operation.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_change(oap: *mut oparg_T) -> c_int {
    unsafe {
        let mut pre_textlen: c_int = 0;
        let mut pre_indent: c_int = 0;
        let mut firstline: *mut c_char;
        let mut bd: block_def = std::mem::zeroed();

        let mut l = (*oap).start.col;
        if (*oap).motion_type == kMTLineWise {
            l = 0;
            Ins.can_si = may_do_si();
        }

        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            if u_save_cursor() == FAIL {
                return 0;
            }
        } else if op_delete(oap) == FAIL {
            return 0;
        }

        if l > (*curwin).w_cursor.col && !lineempty((*curwin).w_cursor.lnum) && virtual_op == 0 {
            inc_cursor();
        }

        if (*oap).motion_type == kMTBlockWise {
            if virtual_op != 0
                && ((*curwin).w_cursor.coladd > 0 || gchar_cursor() == c_int::from(NUL))
            {
                coladvance_force(getviscol());
            }
            firstline = ml_get((*oap).start.lnum);
            pre_textlen = ml_get_len((*oap).start.lnum);
            pre_indent = getwhitecols(firstline) as c_int;
            bd.textcol = (*curwin).w_cursor.col;
        }

        if (*oap).motion_type == kMTLineWise {
            fix_indent();
        }

        let save_finish_op = finish_op;
        finish_op = false;

        let retval = c_int::from(edit(c_int::from(NUL), false, 1));

        finish_op = save_finish_op;

        if (*oap).motion_type == kMTBlockWise && (*oap).start.lnum != (*oap).end.lnum && !got_int {
            firstline = ml_get((*oap).start.lnum);
            if bd.textcol > pre_indent {
                let new_indent = getwhitecols(firstline) as c_int;
                pre_textlen += new_indent - pre_indent;
                bd.textcol += new_indent - pre_indent;
            }

            let ins_len = ml_get_len((*oap).start.lnum) - pre_textlen;
            if ins_len > 0 {
                let ins_text: *mut c_char = xmalloc((ins_len as usize) + 1).cast();
                xmemcpyz(
                    ins_text.cast(),
                    firstline.add(bd.textcol as usize).cast(),
                    ins_len as usize,
                );
                for linenr in ((*oap).start.lnum + 1)..=(*oap).end.lnum {
                    block_prep(oap, &raw mut bd, linenr, true);
                    if bd.is_short == 0 || virtual_op != 0 {
                        let mut vpos = pos_T {
                            lnum: linenr,
                            col: 0,
                            coladd: 0,
                        };
                        if bd.is_short != 0 {
                            getvpos(curwin, &raw mut vpos, (*oap).start_vcol);
                        } else {
                            vpos.coladd = 0;
                        }
                        let oldp = ml_get(linenr);
                        let newp: *mut c_char = xmalloc(
                            (ml_get_len(linenr) as usize)
                                + (vpos.coladd as usize)
                                + (ins_len as usize)
                                + 1,
                        )
                        .cast();
                        ptr::copy(oldp, newp, bd.textcol as usize);
                        let mut newlen = bd.textcol;
                        ptr::write_bytes(newp.add(newlen as usize), b' ', vpos.coladd as usize);
                        newlen += vpos.coladd;
                        ptr::copy(ins_text, newp.add(newlen as usize), ins_len as usize);
                        newlen += ins_len;
                        strcpy_raw(newp.add(newlen as usize), oldp.add(bd.textcol as usize));
                        ml_replace(linenr, newp, false);
                        extmark_splice_cols(
                            curbuf,
                            linenr - 1,
                            bd.textcol,
                            0,
                            vpos.coladd + ins_len,
                            kExtmarkUndo,
                        );
                    }
                }
                check_cursor(curwin);
                changed_lines(
                    curbuf,
                    (*oap).start.lnum + 1,
                    0,
                    (*oap).end.lnum + 1,
                    0,
                    true,
                );
                xfree(ins_text.cast());
            }
        }
        auto_format(false, true);

        retval
    }
}

/// When the cursor is on the NUL past the end of the line and it should not be
/// there move it left.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn adjust_cursor_eol() {
    unsafe {
        let cur_ve_flags = get_ve_flags(curwin);

        let adj_cursor = (*curwin).w_cursor.col > 0
            && gchar_cursor() == c_int::from(NUL)
            && (cur_ve_flags & kOptVeFlagOnemore) == 0
            && (cur_ve_flags & kOptVeFlagAll) == 0
            && !(restart_edit != 0 || (State & MODE_INSERT) != 0);
        if !adj_cursor {
            return;
        }

        dec_cursor();

        if cur_ve_flags == kOptVeFlagAll {
            let mut scol: colnr_T = 0;
            let mut ecol: colnr_T = 0;

            getvcol(
                curwin,
                &raw mut (*curwin).w_cursor,
                &raw mut scol,
                ptr::null_mut(),
                &raw mut ecol,
                0,
            );
            (*curwin).w_cursor.coladd = ecol - scol + 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn skip_comment(
    mut line: *mut c_char,
    process: bool,
    include_space: bool,
    is_comment: *mut bool,
) -> *mut c_char {
    unsafe {
        let mut comment_flags: *mut c_char = ptr::null_mut();
        let leader_offset = get_last_leader_offset(line, &raw mut comment_flags);

        *is_comment = false;
        if leader_offset != -1 {
            while *comment_flags != NUL {
                if *comment_flags == COM_END || *comment_flags == b':' as c_char {
                    break;
                }
                comment_flags = comment_flags.add(1);
            }
            if *comment_flags != COM_END {
                *is_comment = true;
            }
        }

        if !process {
            return line;
        }

        let lead_len = get_leader_len(line, &raw mut comment_flags, false, include_space);

        if lead_len == 0 {
            return line;
        }

        while *comment_flags != NUL {
            if *comment_flags == COM_END || *comment_flags == b':' as c_char {
                break;
            }
            comment_flags = comment_flags.add(1);
        }

        if *comment_flags == b':' as c_char || *comment_flags == NUL {
            line = line.add(lead_len as usize);
        }

        line
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_join(
    count: usize,
    insert_space: bool,
    save_undo: bool,
    use_formatoptions: bool,
    setmark: bool,
) -> c_int {
    unsafe {
        let mut curr: *mut c_char = ptr::null_mut();
        let mut curr_start: *mut c_char = ptr::null_mut();
        let mut cend: *mut c_char;
        let mut endcurr1: c_int = c_int::from(NUL);
        let mut endcurr2: c_int = c_int::from(NUL);
        let mut currsize: c_int = 0;
        let mut sumsize: c_int = 0;
        let mut ret = OK;
        let mut comments: *mut c_int = ptr::null_mut();
        let remove_comments = use_formatoptions && has_format_option(kFoRemoveComs);
        let mut prev_was_comment = false;
        debug_assert!(count >= 1);

        if save_undo
            && u_save(
                (*curwin).w_cursor.lnum - 1,
                (*curwin).w_cursor.lnum + (count as linenr_T),
            ) == FAIL
        {
            return FAIL;
        }
        let spaces: *mut c_char = xcalloc(count, 1).cast();
        if remove_comments {
            comments = xcalloc(count, size_of::<c_int>()).cast();
        }

        let mut aborted = false;
        for t in 0..(count as linenr_T) {
            curr_start = ml_get((*curwin).w_cursor.lnum + t);
            curr = curr_start;
            if t == 0 && setmark && (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*(*curwin).w_buffer).b_op_start.lnum = (*curwin).w_cursor.lnum;
                (*(*curwin).w_buffer).b_op_start.col =
                    CStr::from_ptr(curr).to_bytes().len() as colnr_T;
            }
            if remove_comments {
                if t > 0 && prev_was_comment {
                    let new_curr =
                        skip_comment(curr, true, insert_space, &raw mut prev_was_comment);
                    *comments.add(t as usize) = new_curr.offset_from(curr) as c_int;
                    curr = new_curr;
                } else {
                    curr = skip_comment(curr, false, insert_space, &raw mut prev_was_comment);
                }
            }

            if insert_space && t > 0 {
                curr = skipwhite(curr);
                if *curr != NUL
                    && *curr != b')' as c_char
                    && sumsize != 0
                    && endcurr1 != TAB
                    && (!has_format_option(kFoMbyteJoin)
                        || (utf_ptr2char(curr) < 0x100 && endcurr1 < 0x100))
                    && (!has_format_option(kFoMbyteJoin2)
                        || (utf_ptr2char(curr) < 0x100 && !utf_eat_space(endcurr1))
                        || (endcurr1 < 0x100 && !utf_eat_space(utf_ptr2char(curr))))
                {
                    if endcurr1 == b' ' as c_int {
                        endcurr1 = endcurr2;
                    } else {
                        *spaces.add(t as usize) += 1;
                    }
                    if p_js != 0
                        && (endcurr1 == b'.' as c_int
                            || endcurr1 == b'?' as c_int
                            || endcurr1 == b'!' as c_int)
                    {
                        *spaces.add(t as usize) += 1;
                    }
                }
            }

            let space_t = c_int::from(*spaces.add(t as usize));
            if t > 0 && curbuf_splice_pending == 0 {
                let removed = curr.offset_from(curr_start) as colnr_T;
                extmark_splice(
                    curbuf,
                    (*curwin).w_cursor.lnum - 1,
                    sumsize,
                    1,
                    removed,
                    (removed + 1) as isize,
                    0,
                    space_t,
                    space_t as isize,
                    kExtmarkUndo,
                );
            }
            currsize = CStr::from_ptr(curr).to_bytes().len() as c_int;
            sumsize += currsize + space_t;
            endcurr1 = c_int::from(NUL);
            endcurr2 = c_int::from(NUL);
            if insert_space && currsize > 0 {
                cend = curr.add(currsize as usize);
                cend = cend.sub((utf_head_off(curr, cend.sub(1)) + 1) as usize);
                endcurr1 = utf_ptr2char(cend);
                if cend > curr {
                    cend = cend.sub((utf_head_off(curr, cend.sub(1)) + 1) as usize);
                    endcurr2 = utf_ptr2char(cend);
                }
            }
            line_breakcheck();
            if got_int {
                ret = FAIL;
                aborted = true;
                break;
            }
        }

        if !aborted {
            let col = sumsize - currsize - c_int::from(*spaces.add(count - 1));

            let newp_len = sumsize as usize;
            let newp: *mut c_char = xmallocz(newp_len).cast();
            cend = newp.add(sumsize as usize);

            curbuf_splice_pending += 1;

            let mut t = (count as linenr_T) - 1;
            loop {
                cend = cend.sub(currsize as usize);
                ptr::copy(curr, cend, currsize as usize);

                let space_t = c_int::from(*spaces.add(t as usize));
                if space_t > 0 {
                    cend = cend.sub(space_t as usize);
                    ptr::write_bytes(cend, b' ', space_t as usize);
                }

                let spaces_removed = (curr.offset_from(curr_start) as c_int) - space_t;
                let lnum = (*curwin).w_cursor.lnum + t;
                let mincol: colnr_T = 0;
                let lnum_amount = -t;
                let col_amount = (cend.offset_from(newp) as colnr_T) - spaces_removed;

                mark_col_adjust(lnum, mincol, lnum_amount, col_amount, spaces_removed);

                if t == 0 {
                    break;
                }

                curr_start = ml_get((*curwin).w_cursor.lnum + t - 1);
                curr = curr_start;
                if remove_comments {
                    curr = curr.add(*comments.add((t - 1) as usize) as usize);
                }
                if insert_space && t > 1 {
                    curr = skipwhite(curr);
                }
                currsize = CStr::from_ptr(curr).to_bytes().len() as c_int;
                t -= 1;
            }

            ml_replace_len((*curwin).w_cursor.lnum, newp, newp_len, false);

            if setmark && (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*(*curwin).w_buffer).b_op_end.lnum = (*curwin).w_cursor.lnum;
                (*(*curwin).w_buffer).b_op_end.col = sumsize;
            }

            changed_lines(
                curbuf,
                (*curwin).w_cursor.lnum,
                currsize,
                (*curwin).w_cursor.lnum + 1,
                0,
                true,
            );

            let saved_lnum = (*curwin).w_cursor.lnum;
            (*curwin).w_cursor.lnum += 1;
            del_lines((count as c_int) - 1, false);
            (*curwin).w_cursor.lnum = saved_lnum;
            curbuf_splice_pending -= 1;
            (*curbuf).deleted_bytes2 = 0;

            (*curwin).w_cursor.col = if !vim_strchr(p_cpo, kCpoJoincol).is_null() {
                currsize
            } else {
                col
            };
            check_cursor_col(curwin);

            (*curwin).w_cursor.coladd = 0;
            (*curwin).w_set_curswant = true;
        }

        xfree(spaces.cast());
        if remove_comments {
            xfree(comments.cast());
        }
        ret
    }
}

/// Reset 'linebreak' and take care of side effects.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn reset_lbr() -> bool {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_lbr == 0 {
            return false;
        }
        (*curwin).w_onebuf_opt.wo_lbr = 0;
        (*curwin).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        true
    }
}

/// Restore 'linebreak' and take care of side effects.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn restore_lbr(lbr_saved: bool) {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_lbr != 0 || !lbr_saved {
            return;
        }

        (*curwin).w_onebuf_opt.wo_lbr = 1;
        (*curwin).w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
    }
}

/// Prepare a few things for block mode yank/delete/tilde.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn block_prep(
    oap: *mut oparg_T,
    bdp: *mut block_def,
    lnum: linenr_T,
    is_del: bool,
) {
    unsafe {
        let mut incr: c_int = 0;
        let lbr_saved = reset_lbr();

        (*bdp).startspaces = 0;
        (*bdp).endspaces = 0;
        (*bdp).textlen = 0;
        (*bdp).start_vcol = 0;
        (*bdp).end_vcol = 0;
        (*bdp).is_short = 0;
        (*bdp).is_oneChar = 0;
        (*bdp).pre_whitesp = 0;
        (*bdp).pre_whitesp_c = 0;
        (*bdp).end_char_vcols = 0;
        (*bdp).start_char_vcols = 0;

        let line = ml_get(lnum);
        let mut prev_pstart = line;

        let mut csarg: CharsizeArg = std::mem::zeroed();
        let mut cstype = init_charsize_arg(&raw mut csarg, curwin, lnum, line);
        let mut ci = utf_ptr2StrCharInfo(line);
        let mut vcol = (*bdp).start_vcol;
        while vcol < (*oap).start_vcol && *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            incr = win_charsize(
                cstype,
                vcol,
                ci.ptr,
                ci.chr.value,
                &raw mut csarg,
                cli.cells,
            )
            .width;
            vcol += incr;
            if ascii_iswhite(ci.chr.value) {
                (*bdp).pre_whitesp += incr;
                (*bdp).pre_whitesp_c += 1;
            } else {
                (*bdp).pre_whitesp = 0;
                (*bdp).pre_whitesp_c = 0;
            }
            prev_pstart = ci.ptr.cast_mut();
            ci = cli.next;
        }
        (*bdp).start_vcol = vcol;
        let mut pstart = ci.ptr.cast_mut();

        (*bdp).start_char_vcols = incr;
        if (*bdp).start_vcol < (*oap).start_vcol {
            (*bdp).end_vcol = (*bdp).start_vcol;
            (*bdp).is_short = 1;
            if !is_del || (*oap).op_type == OP_APPEND {
                (*bdp).endspaces = (*oap).end_vcol - (*oap).start_vcol + 1;
            }
        } else {
            (*bdp).startspaces = (*bdp).start_vcol - (*oap).start_vcol;
            if is_del && (*bdp).startspaces != 0 {
                (*bdp).startspaces = (*bdp).start_char_vcols - (*bdp).startspaces;
            }
            let mut pend: *const c_char = pstart;
            (*bdp).end_vcol = (*bdp).start_vcol;
            if (*bdp).end_vcol > (*oap).end_vcol {
                (*bdp).is_oneChar = 1;
                if (*oap).op_type == OP_INSERT {
                    (*bdp).endspaces = (*bdp).start_char_vcols - (*bdp).startspaces;
                } else if (*oap).op_type == OP_APPEND {
                    (*bdp).startspaces += (*oap).end_vcol - (*oap).start_vcol + 1;
                    (*bdp).endspaces = (*bdp).start_char_vcols - (*bdp).startspaces;
                } else {
                    (*bdp).startspaces = (*oap).end_vcol - (*oap).start_vcol + 1;
                    if is_del && (*oap).op_type != OP_LSHIFT {
                        (*bdp).startspaces =
                            (*bdp).start_char_vcols - ((*bdp).start_vcol - (*oap).start_vcol);
                        (*bdp).endspaces = (*bdp).end_vcol - (*oap).end_vcol - 1;
                    }
                }
            } else {
                cstype = init_charsize_arg(&raw mut csarg, curwin, lnum, line);
                ci = utf_ptr2StrCharInfo(pend);
                vcol = (*bdp).end_vcol;
                let mut prev_pend = pend;
                while vcol <= (*oap).end_vcol && *ci.ptr != NUL {
                    prev_pend = ci.ptr;
                    let cli = utf_ClusterInfo(ci);
                    incr = win_charsize(
                        cstype,
                        vcol,
                        ci.ptr,
                        ci.chr.value,
                        &raw mut csarg,
                        cli.cells,
                    )
                    .width;
                    vcol += incr;
                    ci = cli.next;
                }
                (*bdp).end_vcol = vcol;
                pend = ci.ptr;

                if (*bdp).end_vcol <= (*oap).end_vcol
                    && (!is_del || (*oap).op_type == OP_APPEND || (*oap).op_type == OP_REPLACE)
                {
                    (*bdp).is_short = 1;
                    if (*oap).op_type == OP_APPEND || virtual_op != 0 {
                        (*bdp).endspaces =
                            (*oap).end_vcol - (*bdp).end_vcol + c_int::from((*oap).inclusive);
                    }
                } else if (*bdp).end_vcol > (*oap).end_vcol {
                    (*bdp).endspaces = (*bdp).end_vcol - (*oap).end_vcol - 1;
                    if !is_del && (*bdp).endspaces != 0 {
                        (*bdp).endspaces = incr - (*bdp).endspaces;
                        if !ptr::eq(pend, pstart) {
                            pend = prev_pend;
                        }
                    }
                }
            }
            (*bdp).end_char_vcols = incr;
            if is_del && (*bdp).startspaces != 0 {
                pstart = prev_pstart;
            }
            (*bdp).textlen = pend.offset_from(pstart) as c_int;
        }
        (*bdp).textcol = pstart.offset_from(line) as colnr_T;
        (*bdp).textstart = pstart;
        restore_lbr(lbr_saved);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getregionpos_prep(
    p1: *mut pos_T,
    p2: *mut pos_T,
    region_type: MotionType,
    exclusive: bool,
    block_width: c_int,
    inclusive: *mut bool,
    oap: *mut oparg_T,
) {
    unsafe {
        *inclusive = true;
        if !lt(*p1, *p2) {
            ptr::swap(p1, p2);
        }

        if region_type == kMTCharWise {
            if exclusive && !equalpos(*p1, *p2) {
                *inclusive = !unadjust_for_sel_inner(p2);
            }
            if *inclusive && virtual_op == 0 && *ml_get_pos(p2) == NUL {
                *inclusive = false;
            }
        } else if region_type == kMTBlockWise {
            let mut sc1: colnr_T = 0;
            let mut ec1: colnr_T = 0;
            let mut sc2: colnr_T = 0;
            let mut ec2: colnr_T = 0;
            let lbr_saved = reset_lbr();
            getvvcol(curwin, p1, &raw mut sc1, ptr::null_mut(), &raw mut ec1, 0);
            getvvcol(curwin, p2, &raw mut sc2, ptr::null_mut(), &raw mut ec2, 0);
            restore_lbr(lbr_saved);
            (*oap).motion_type = kMTBlockWise;
            (*oap).inclusive = true;
            (*oap).op_type = OP_NOP;
            (*oap).start = *p1;
            (*oap).end = *p2;
            (*oap).start_vcol = sc1.min(sc2);
            if block_width > 0 {
                (*oap).end_vcol = (*oap).start_vcol + block_width - 1;
            } else if exclusive && ec1 < sc2 && 0 < sc2 && ec2 > ec1 {
                (*oap).end_vcol = sc2 - 1;
            } else {
                (*oap).end_vcol = ec1.max(ec2);
            }
        }

        let l = utfc_ptr2len(ml_get_pos(p2));
        if l > 1 {
            (*p2).col += l - 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getregionpos_line(
    lnum: linenr_T,
    p1: pos_T,
    p2: pos_T,
    inclusive: bool,
    region_type: MotionType,
    oap: *mut oparg_T,
    allow_eol: bool,
    ret_p1: *mut pos_T,
    ret_p2: *mut pos_T,
) {
    unsafe {
        let line = ml_get(lnum);
        let line_len = ml_get_len(lnum);

        if region_type == kMTLineWise {
            (*ret_p1).col = 1;
            (*ret_p1).coladd = 0;
            (*ret_p2).col = MAXCOL;
            (*ret_p2).coladd = 0;
        } else {
            let mut bd: block_def = std::mem::zeroed();

            if region_type == kMTBlockWise {
                block_prep(oap, &raw mut bd, lnum, false);
            } else {
                charwise_block_prep(p1, p2, &raw mut bd, lnum, inclusive);
            }

            if bd.is_oneChar != 0 {
                if region_type == kMTBlockWise {
                    (*ret_p1).col =
                        (mb_prevptr(line, bd.textstart).offset_from(line) as colnr_T) + 1;
                    (*ret_p1).coladd = bd.start_char_vcols - (bd.start_vcol - (*oap).start_vcol);
                } else {
                    (*ret_p1).col = p1.col + 1;
                    (*ret_p1).coladd = p1.coladd;
                }
            } else if region_type == kMTBlockWise && (*oap).start_vcol > bd.start_vcol {
                (*ret_p1).col = MAXCOL;
                (*ret_p1).coladd = (*oap).start_vcol - bd.start_vcol;
                bd.is_oneChar = 1;
            } else if bd.startspaces > 0 {
                (*ret_p1).col = (mb_prevptr(line, bd.textstart).offset_from(line) as colnr_T) + 1;
                (*ret_p1).coladd = bd.start_char_vcols - bd.startspaces;
            } else {
                (*ret_p1).col = bd.textcol + 1;
                (*ret_p1).coladd = 0;
            }

            if bd.is_oneChar != 0 {
                (*ret_p2).col = (*ret_p1).col;
                (*ret_p2).coladd = (*ret_p1).coladd + bd.startspaces + bd.endspaces;
            } else if bd.endspaces > 0 {
                (*ret_p2).col = bd.textcol + bd.textlen + 1;
                (*ret_p2).coladd = bd.endspaces;
            } else {
                (*ret_p2).col = bd.textcol + bd.textlen;
                (*ret_p2).coladd = 0;
            }
        }

        if !allow_eol && (*ret_p1).col > line_len {
            (*ret_p1).col = 0;
            (*ret_p1).coladd = 0;
        } else if (*ret_p1).col > line_len + 1 {
            (*ret_p1).col = line_len + 1;
        }

        if !allow_eol && (*ret_p2).col > line_len {
            (*ret_p2).col = if (*ret_p1).col == 0 { 0 } else { line_len };
            (*ret_p2).coladd = 0;
        } else if (*ret_p2).col > line_len + 1 {
            (*ret_p2).col = line_len + 1;
        }

        (*ret_p1).lnum = lnum;
        (*ret_p2).lnum = lnum;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn charwise_block_prep(
    mut start: pos_T,
    mut end: pos_T,
    bdp: *mut block_def,
    lnum: linenr_T,
    inclusive: bool,
) {
    unsafe {
        let mut startcol: colnr_T = 0;
        let mut endcol: colnr_T = MAXCOL;
        let mut cs: colnr_T = 0;
        let mut ce: colnr_T = 0;
        let p = ml_get(lnum);
        let plen = ml_get_len(lnum);

        (*bdp).startspaces = 0;
        (*bdp).endspaces = 0;
        (*bdp).is_oneChar = 0;
        (*bdp).start_char_vcols = 0;

        if lnum == start.lnum {
            startcol = start.col;
            if virtual_op != 0 {
                getvcol(
                    curwin,
                    &raw mut start,
                    &raw mut cs,
                    ptr::null_mut(),
                    &raw mut ce,
                    0,
                );
                if ce != cs && start.coladd > 0 {
                    (*bdp).start_char_vcols = ce - cs + 1;
                    (*bdp).startspaces = ((*bdp).start_char_vcols - start.coladd).max(0);
                    startcol += 1;
                }
            }
        }

        if lnum == end.lnum {
            endcol = end.col;
            if virtual_op != 0 {
                getvcol(
                    curwin,
                    &raw mut end,
                    &raw mut cs,
                    ptr::null_mut(),
                    &raw mut ce,
                    0,
                );
                if *p.add(endcol as usize) == NUL
                    || (cs + end.coladd < ce && utf_head_off(p, p.add(endcol as usize)) == 0)
                {
                    if start.lnum == end.lnum && start.col == end.col {
                        (*bdp).is_oneChar = 1;
                        (*bdp).startspaces = end.coladd - start.coladd + c_int::from(inclusive);
                        endcol = startcol;
                    } else {
                        (*bdp).endspaces = end.coladd + c_int::from(inclusive);
                        endcol -= c_int::from(inclusive);
                    }
                }
            }
        }
        if endcol == MAXCOL {
            endcol = ml_get_len(lnum);
        }
        if startcol > endcol || (*bdp).is_oneChar != 0 {
            (*bdp).textlen = 0;
        } else {
            (*bdp).textlen = endcol - startcol + c_int::from(inclusive);
        }
        (*bdp).textcol = startcol;
        (*bdp).textstart = if startcol <= plen {
            p.add(startcol as usize)
        } else {
            p
        };
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_addsub(oap: *mut oparg_T, Prenum1: linenr_T, g_cmd: bool) {
    unsafe {
        let mut bd: block_def = std::mem::zeroed();
        let mut change_cnt: isize = 0;
        let mut amount = Prenum1;

        disable_fold_update += 1;

        if !Visual.active {
            let mut pos = (*curwin).w_cursor;
            if u_save_cursor() == FAIL {
                disable_fold_update -= 1;
                return;
            }
            change_cnt = isize::from(do_addsub((*oap).op_type, &raw mut pos, 0, amount));
            disable_fold_update -= 1;
            if change_cnt != 0 {
                changed_lines(curbuf, pos.lnum, 0, pos.lnum + 1, 0, true);
            }
        } else {
            let mut length: c_int;
            let mut startpos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };

            if u_save((*oap).start.lnum - 1, (*oap).end.lnum + 1) == FAIL {
                disable_fold_update -= 1;
                return;
            }

            let mut pos = (*oap).start;
            while pos.lnum <= (*oap).end.lnum {
                if (*oap).motion_type == kMTBlockWise {
                    block_prep(oap, &raw mut bd, pos.lnum, false);
                    pos.col = bd.textcol;
                    length = bd.textlen;
                } else if (*oap).motion_type == kMTLineWise {
                    (*curwin).w_cursor.col = 0;
                    pos.col = 0;
                    length = ml_get_len(pos.lnum);
                } else {
                    if pos.lnum == (*oap).start.lnum && !(*oap).inclusive {
                        dec(&raw mut (*oap).end);
                    }
                    length = ml_get_len(pos.lnum);
                    pos.col = 0;
                    if pos.lnum == (*oap).start.lnum {
                        pos.col += (*oap).start.col;
                        length -= (*oap).start.col;
                    }
                    if pos.lnum == (*oap).end.lnum {
                        length = ml_get_len((*oap).end.lnum);
                        (*oap).end.col = (*oap).end.col.min(length - 1);
                        length = (*oap).end.col - pos.col + 1;
                    }
                }
                let one_change = do_addsub((*oap).op_type, &raw mut pos, length, amount);
                if one_change {
                    if change_cnt == 0 {
                        startpos = (*curbuf).b_op_start;
                    }
                    change_cnt += 1;
                }

                if g_cmd && one_change {
                    amount += Prenum1;
                }
                pos.lnum += 1;
            }

            disable_fold_update -= 1;
            if change_cnt != 0 {
                changed_lines(curbuf, (*oap).start.lnum, 0, (*oap).end.lnum + 1, 0, true);
            }

            if change_cnt == 0 && (*oap).is_VIsual {
                redraw_curbuf_later(UPD_INVERTED);
            }

            if change_cnt > 0 && (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                (*curbuf).b_op_start = startpos;
            }

            if (change_cnt as i64) > p_report {
                smsg!(
                    0,
                    ngettext(
                        c"%lld lines changed".as_ptr(),
                        c"%lld lines changed".as_ptr(),
                        change_cnt as c_ulong,
                    ),
                    change_cnt as std::ffi::c_longlong,
                );
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_addsub(
    op_type: c_int,
    pos: *mut pos_T,
    mut length: c_int,
    Prenum1: linenr_T,
) -> bool {
    unsafe {
        let mut pre: c_int = 0;
        let mut n: uvarnumber_T = 0;
        let mut blank_unsigned = false;
        let mut negative = false;
        let mut was_positive = true;
        let visual = Visual.active;
        let did_change: bool;
        let save_cursor = (*curwin).w_cursor;
        let mut maxlen: c_int = 0;
        let startpos: pos_T;
        let endpos: pos_T;
        let mut save_coladd: colnr_T = 0;

        let do_hex = !vim_strchr((*curbuf).b_p_nf, b'x' as c_int).is_null();
        let do_oct = !vim_strchr((*curbuf).b_p_nf, b'o' as c_int).is_null();
        let do_bin = !vim_strchr((*curbuf).b_p_nf, b'b' as c_int).is_null();
        let do_alpha = !vim_strchr((*curbuf).b_p_nf, b'p' as c_int).is_null();
        let do_unsigned = !vim_strchr((*curbuf).b_p_nf, b'u' as c_int).is_null();
        let do_blank = !vim_strchr((*curbuf).b_p_nf, b'k' as c_int).is_null();

        if virtual_active(curwin) {
            save_coladd = (*pos).coladd;
            (*pos).coladd = 0;
        }

        (*curwin).w_cursor = *pos;
        let mut ptr_line = ml_get((*pos).lnum);
        let linelen = ml_get_len((*pos).lnum);
        let mut col = (*pos).col;

        if col + c_int::from(save_coladd != 0) >= linelen {
            if visual {
                (*curwin).w_cursor = save_cursor;
            } else if virtual_active(curwin) {
                (*curwin).w_cursor.coladd = save_coladd;
            }
            return false;
        }

        if !Visual.active {
            if do_bin {
                while col > 0 && ascii_isbdigit(c_int::from(*ptr_line.add(col as usize) as u8)) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }

            if do_hex {
                while col > 0 && ascii_isxdigit(c_int::from(*ptr_line.add(col as usize) as u8)) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }
            if do_bin
                && do_hex
                && !(col > 0
                    && (*ptr_line.add(col as usize) == b'X' as c_char
                        || *ptr_line.add(col as usize) == b'x' as c_char)
                    && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                    && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                    && ascii_isxdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
            {
                col = (*curwin).w_cursor.col;

                while col > 0 && ascii_isdigit(c_int::from(*ptr_line.add(col as usize) as u8)) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }

            if (do_hex
                && col > 0
                && (*ptr_line.add(col as usize) == b'X' as c_char
                    || *ptr_line.add(col as usize) == b'x' as c_char)
                && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                && ascii_isxdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
                || (do_bin
                    && col > 0
                    && (*ptr_line.add(col as usize) == b'B' as c_char
                        || *ptr_line.add(col as usize) == b'b' as c_char)
                    && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                    && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                    && ascii_isbdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
            {
                col -= 1;
                col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
            } else {
                col = (*pos).col;

                while *ptr_line.add(col as usize) != NUL
                    && !ascii_isdigit(c_int::from(*ptr_line.add(col as usize) as u8))
                    && !(do_alpha && ascii_isalpha(c_int::from(*ptr_line.add(col as usize) as u8)))
                {
                    col += 1;
                }

                while col > 0
                    && ascii_isdigit(c_int::from(*ptr_line.add((col - 1) as usize) as u8))
                    && !(do_alpha && ascii_isalpha(c_int::from(*ptr_line.add(col as usize) as u8)))
                {
                    col -= 1;
                }
            }
        }

        if visual {
            while *ptr_line.add(col as usize) != NUL
                && length > 0
                && !ascii_isdigit(c_int::from(*ptr_line.add(col as usize) as u8))
                && !(do_alpha && ascii_isalpha(c_int::from(*ptr_line.add(col as usize) as u8)))
            {
                let mb_len = utfc_ptr2len(ptr_line.add(col as usize));
                col += mb_len;
                length -= mb_len;
            }

            if length == 0 {
                (*curwin).w_cursor = save_cursor;
                return false;
            }

            if col > (*pos).col
                && *ptr_line.add((col - 1) as usize) == b'-' as c_char
                && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                && !do_unsigned
            {
                if do_blank
                    && col >= 2
                    && !ascii_iswhite(c_int::from(*ptr_line.add((col - 2) as usize) as u8))
                {
                    blank_unsigned = true;
                } else {
                    negative = true;
                    was_positive = false;
                }
            }
        }

        let mut firstdigit = c_int::from(*ptr_line.add(col as usize) as u8);
        if !ascii_isdigit(firstdigit) && !(do_alpha && ascii_isalpha(firstdigit)) {
            beep_flush();
            if visual {
                (*curwin).w_cursor = save_cursor;
            } else if virtual_active(curwin) {
                (*curwin).w_cursor.coladd = save_coladd;
            }
            return false;
        }

        if do_alpha && ascii_isalpha(firstdigit) {
            if op_type == OP_NR_SUB {
                if char_ord(firstdigit) < Prenum1 {
                    firstdigit = if ascii_isupper(firstdigit) {
                        b'A' as c_int
                    } else {
                        b'a' as c_int
                    };
                } else {
                    firstdigit -= Prenum1;
                }
            } else if 26 - char_ord(firstdigit) - 1 < Prenum1 {
                firstdigit = if ascii_isupper(firstdigit) {
                    b'Z' as c_int
                } else {
                    b'z' as c_int
                };
            } else {
                firstdigit += Prenum1;
            }
            (*curwin).w_cursor.col = col;
            startpos = (*curwin).w_cursor;
            did_change = true;
            del_char(false);
            ins_char(firstdigit);
            endpos = (*curwin).w_cursor;
            (*curwin).w_cursor.col = col;
        } else {
            if col > 0
                && *ptr_line.add((col - 1) as usize) == b'-' as c_char
                && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                && !visual
                && !do_unsigned
            {
                if do_blank
                    && col >= 2
                    && !ascii_iswhite(c_int::from(*ptr_line.add((col - 2) as usize) as u8))
                {
                    blank_unsigned = true;
                } else {
                    col -= 1;
                    negative = true;
                }
            }

            if visual && Visual.mode != b'V' as c_int {
                maxlen = if (*curbuf).b_visual.vi_curswant == MAXCOL {
                    linelen - col
                } else {
                    length
                };
            }

            let mut overflow = false;
            vim_str2nr(
                ptr_line.add(col as usize),
                &raw mut pre,
                &raw mut length,
                (if do_bin { STR2NR_BIN } else { 0 })
                    + (if do_oct { STR2NR_OCT } else { 0 })
                    + (if do_hex { STR2NR_HEX } else { 0 }),
                ptr::null_mut(),
                &raw mut n,
                maxlen,
                false,
                &raw mut overflow,
            );

            if pre != 0 && negative {
                col += 1;
                length -= 1;
                negative = false;
            }

            let mut subtract = false;
            if op_type == OP_NR_SUB {
                subtract ^= true;
            }
            if negative {
                subtract ^= true;
            }

            let oldn = n;

            if !overflow {
                n = if subtract {
                    n.wrapping_sub(Prenum1 as uvarnumber_T)
                } else {
                    n.wrapping_add(Prenum1 as uvarnumber_T)
                };
            }

            if pre == 0 {
                if subtract {
                    if n > oldn {
                        n = 1u64.wrapping_add(n ^ u64::MAX);
                        negative ^= true;
                    }
                } else if n < oldn {
                    n ^= u64::MAX;
                    negative ^= true;
                }
                if n == 0 {
                    negative = false;
                }
            }

            if (do_unsigned || blank_unsigned) && negative {
                if subtract {
                    n = 0;
                } else {
                    n = u64::MAX;
                }
                negative = false;
            }

            if visual && !was_positive && !negative && col > 0 {
                col -= 1;
                length += 1;
            }

            (*curwin).w_cursor.col = col;
            startpos = (*curwin).w_cursor;
            did_change = true;
            let mut todel = length;
            let mut c = gchar_cursor();

            if c == b'-' as c_int {
                length -= 1;
            }
            while todel > 0 {
                todel -= 1;
                if c < 0x100 && ascii_isalpha(c) {
                    addsub_hexupper = ascii_isupper(c);
                }
                del_char(false);
                c = gchar_cursor();
            }

            let buf1: *mut c_char = xmalloc((length as usize) + NUMBUFLEN).cast();
            ptr_line = buf1;
            if negative && (!visual || was_positive) {
                *ptr_line = b'-' as c_char;
                ptr_line = ptr_line.add(1);
            }
            if pre != 0 {
                *ptr_line = b'0' as c_char;
                ptr_line = ptr_line.add(1);
                length -= 1;
            }
            if pre == b'b' as c_int
                || pre == b'B' as c_int
                || pre == b'x' as c_int
                || pre == b'X' as c_int
            {
                *ptr_line = pre as c_char;
                ptr_line = ptr_line.add(1);
                length -= 1;
            }

            let mut buf2 = [0 as c_char; NUMBUFLEN];
            let buf2_ptr = (&raw mut buf2).cast::<c_char>();
            let mut buf2len: c_int = 0;
            if pre == b'b' as c_int || pre == b'B' as c_int {
                let mut bits = uvarnumber_T::BITS as usize;
                while bits > 0 {
                    if ((n >> (bits - 1)) & 0x1) != 0 {
                        break;
                    }
                    bits -= 1;
                }

                while bits > 0 && (buf2len as usize) < NUMBUFLEN - 1 {
                    bits -= 1;
                    *buf2_ptr.add(buf2len as usize) = if ((n >> bits) & 0x1) != 0 {
                        b'1' as c_char
                    } else {
                        b'0' as c_char
                    };
                    buf2len += 1;
                }

                *buf2_ptr.add(buf2len as usize) = NUL;
            } else if pre == 0 {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llu".as_ptr(),
                    n as std::ffi::c_ulonglong,
                );
            } else if pre == b'0' as c_int {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llo".as_ptr(),
                    n as std::ffi::c_ulonglong,
                );
            } else if addsub_hexupper {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llX".as_ptr(),
                    n as std::ffi::c_ulonglong,
                );
            } else {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llx".as_ptr(),
                    n as std::ffi::c_ulonglong,
                );
            }
            length -= buf2len;

            if firstdigit == b'0' as c_int && !(do_oct && pre == 0) {
                while length > 0 {
                    length -= 1;
                    *ptr_line = b'0' as c_char;
                    ptr_line = ptr_line.add(1);
                }
            }
            *ptr_line = NUL;
            let mut buf1len = ptr_line.offset_from(buf1) as c_int;

            strcpy_raw(buf1.add(buf1len as usize), buf2_ptr);
            buf1len += buf2len;

            ins_str(buf1, buf1len as usize);
            xfree(buf1.cast());

            endpos = (*curwin).w_cursor;
            if (*curwin).w_cursor.col != 0 {
                (*curwin).w_cursor.col -= 1;
            }
        }

        if (cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            (*curbuf).b_op_start = startpos;
            (*curbuf).b_op_end = endpos;
            if (*curbuf).b_op_end.col > 0 {
                (*curbuf).b_op_end.col -= 1;
            }
        }

        if visual {
            (*curwin).w_cursor = save_cursor;
        } else if did_change {
            (*curwin).w_set_curswant = true;
        } else if virtual_active(curwin) {
            (*curwin).w_cursor.coladd = save_coladd;
        }

        did_change
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_oparg(oap: *mut oparg_T) {
    unsafe {
        ptr::write_bytes(oap, 0, 1);
    }
}

unsafe fn line_count_info(
    line: *const c_char,
    wc: *mut varnumber_T,
    cc: *mut varnumber_T,
    limit: varnumber_T,
    eol_size: c_int,
) -> varnumber_T {
    unsafe {
        let mut i: varnumber_T = 0;
        let mut words: varnumber_T = 0;
        let mut chars: varnumber_T = 0;
        let mut is_word = false;

        while i < limit && *line.add(i as usize) != NUL {
            let ch = c_int::from(*line.add(i as usize) as u8);
            if is_word {
                if ascii_isspace(ch) {
                    words += 1;
                    is_word = false;
                }
            } else if !ascii_isspace(ch) {
                is_word = true;
            }
            chars += 1;
            i += varnumber_T::from(utfc_ptr2len(line.add(i as usize)));
        }

        if is_word {
            words += 1;
        }
        *wc += words;

        if i < limit && *line.add(i as usize) == NUL {
            i += varnumber_T::from(eol_size);
            chars += varnumber_T::from(eol_size);
        }
        *cc += chars;
        i
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cursor_pos_info(dict: *mut dict_T) {
    unsafe {
        let mut buf1 = [0 as c_char; 50];
        let buf1_ptr = (&raw mut buf1).cast::<c_char>();
        let mut buf2 = [0 as c_char; 40];
        let buf2_ptr = (&raw mut buf2).cast::<c_char>();
        let mut byte_count: varnumber_T = 0;
        let mut bom_count: varnumber_T = 0;
        let mut byte_count_cursor: varnumber_T = 0;
        let mut char_count: varnumber_T = 0;
        let mut char_count_cursor: varnumber_T = 0;
        let mut word_count: varnumber_T = 0;
        let mut word_count_cursor: varnumber_T = 0;
        let mut min_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut max_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut oparg: oparg_T = std::mem::zeroed();
        let mut bd: block_def = std::mem::zeroed();
        let l_VIsual_active = Visual.active;
        let l_VIsual_mode = Visual.mode;

        if ((*curbuf).b_ml.ml_flags & ML_EMPTY) != 0 {
            if dict.is_null() {
                msg(tr(no_lines_msg.as_ptr()), 0);
                return;
            }
        } else {
            let mut last_check: varnumber_T = 100000;
            let mut line_count_selected: c_int = 0;
            let eol_size = if get_fileformat(curbuf) == EOL_DOS {
                2
            } else {
                1
            };

            if l_VIsual_active {
                if lt(Visual.start, (*curwin).w_cursor) {
                    min_pos = Visual.start;
                    max_pos = (*curwin).w_cursor;
                } else {
                    min_pos = (*curwin).w_cursor;
                    max_pos = Visual.start;
                }
                if *p_sel == b'e' as c_char && max_pos.col > 0 {
                    max_pos.col -= 1;
                }

                if l_VIsual_mode == Ctrl_V {
                    let saved_sbr = p_sbr;
                    let saved_w_sbr = (*curwin).w_onebuf_opt.wo_sbr;

                    p_sbr = (&raw mut empty_string_option).cast::<c_char>();
                    (*curwin).w_onebuf_opt.wo_sbr = (&raw mut empty_string_option).cast::<c_char>();
                    oparg.is_VIsual = true;
                    oparg.motion_type = kMTBlockWise;
                    oparg.op_type = OP_NOP;
                    getvcols(
                        curwin,
                        &raw mut min_pos,
                        &raw mut max_pos,
                        &raw mut oparg.start_vcol,
                        &raw mut oparg.end_vcol,
                        0,
                    );
                    p_sbr = saved_sbr;
                    (*curwin).w_onebuf_opt.wo_sbr = saved_w_sbr;
                    if (*curwin).w_curswant == MAXCOL {
                        oparg.end_vcol = MAXCOL;
                    }
                    if oparg.end_vcol < oparg.start_vcol {
                        std::mem::swap(&mut oparg.start_vcol, &mut oparg.end_vcol);
                    }
                }
                line_count_selected = max_pos.lnum - min_pos.lnum + 1;
            }

            for lnum in 1..=(*curbuf).b_ml.ml_line_count {
                if byte_count > last_check {
                    os_breakcheck();
                    if got_int {
                        return;
                    }
                    last_check = byte_count + 100000;
                }

                if l_VIsual_active && lnum >= min_pos.lnum && lnum <= max_pos.lnum {
                    let mut s: *mut c_char = ptr::null_mut();
                    let mut len: c_int = 0;

                    if l_VIsual_mode == Ctrl_V {
                        virtual_op = TriState::from(virtual_active(curwin));
                        block_prep(&raw mut oparg, &raw mut bd, lnum, false);
                        virtual_op = kNone;
                        s = bd.textstart;
                        len = bd.textlen;
                    } else if l_VIsual_mode == b'V' as c_int {
                        s = ml_get(lnum);
                        len = MAXCOL;
                    } else if l_VIsual_mode == b'v' as c_int {
                        let start_col = if lnum == min_pos.lnum { min_pos.col } else { 0 };
                        let end_col = if lnum == max_pos.lnum {
                            max_pos.col - start_col + 1
                        } else {
                            MAXCOL
                        };

                        s = ml_get(lnum).add(start_col as usize);
                        len = end_col;
                    }
                    if !s.is_null() {
                        byte_count_cursor += line_count_info(
                            s,
                            &raw mut word_count_cursor,
                            &raw mut char_count_cursor,
                            varnumber_T::from(len),
                            eol_size,
                        );
                        if lnum == (*curbuf).b_ml.ml_line_count
                            && (*curbuf).b_p_eol == 0
                            && ((*curbuf).b_p_bin != 0 || (*curbuf).b_p_fixeol == 0)
                            && (CStr::from_ptr(s).to_bytes().len() as c_int) < len
                        {
                            byte_count_cursor -= varnumber_T::from(eol_size);
                        }
                    }
                } else if lnum == (*curwin).w_cursor.lnum {
                    word_count_cursor += word_count;
                    char_count_cursor += char_count;
                    byte_count_cursor = byte_count
                        + line_count_info(
                            ml_get(lnum),
                            &raw mut word_count_cursor,
                            &raw mut char_count_cursor,
                            varnumber_T::from((*curwin).w_cursor.col) + 1,
                            eol_size,
                        );
                }
                byte_count += line_count_info(
                    ml_get(lnum),
                    &raw mut word_count,
                    &raw mut char_count,
                    varnumber_T::from(MAXCOL),
                    eol_size,
                );
            }

            if (*curbuf).b_p_eol == 0 && ((*curbuf).b_p_bin != 0 || (*curbuf).b_p_fixeol == 0) {
                byte_count -= varnumber_T::from(eol_size);
            }

            let iobuff_ptr = (&raw mut IObuff).cast::<c_char>();
            if dict.is_null() {
                if l_VIsual_active {
                    if l_VIsual_mode == Ctrl_V && (*curwin).w_curswant < MAXCOL {
                        getvcols(
                            curwin,
                            &raw mut min_pos,
                            &raw mut max_pos,
                            &raw mut min_pos.col,
                            &raw mut max_pos.col,
                            0,
                        );
                        let cols = i64::from(oparg.end_vcol + 1)
                            .checked_sub(i64::from(oparg.start_vcol))
                            .unwrap_or_else(|| std::process::abort());
                        vim_snprintf!(
                            buf1_ptr,
                            buf1.len(),
                            tr(c"%lld Cols; ".as_ptr()),
                            cols as std::ffi::c_longlong,
                        );
                    } else {
                        *buf1_ptr = NUL;
                    }

                    if char_count_cursor == byte_count_cursor && char_count == byte_count {
                        vim_snprintf!(
                            iobuff_ptr,
                            IOSIZE,
                            tr(
                                c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Bytes"
                                    .as_ptr(),
                            ),
                            buf1_ptr,
                            i64::from(line_count_selected) as std::ffi::c_longlong,
                            i64::from((*curbuf).b_ml.ml_line_count) as std::ffi::c_longlong,
                            word_count_cursor as std::ffi::c_longlong,
                            word_count as std::ffi::c_longlong,
                            byte_count_cursor as std::ffi::c_longlong,
                            byte_count as std::ffi::c_longlong,
                        );
                    } else {
                        vim_snprintf!(
                            iobuff_ptr,
                            IOSIZE,
                            tr(
                                c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Chars; %lld of %lld Bytes"
                                    .as_ptr(),
                            ),
                            buf1_ptr,
                            i64::from(line_count_selected) as std::ffi::c_longlong,
                            i64::from((*curbuf).b_ml.ml_line_count) as std::ffi::c_longlong,
                            word_count_cursor as std::ffi::c_longlong,
                            word_count as std::ffi::c_longlong,
                            char_count_cursor as std::ffi::c_longlong,
                            char_count as std::ffi::c_longlong,
                            byte_count_cursor as std::ffi::c_longlong,
                            byte_count as std::ffi::c_longlong,
                        );
                    }
                } else {
                    let p = get_cursor_line_ptr();
                    validate_virtcol(curwin);
                    col_print(
                        buf1_ptr,
                        buf1.len(),
                        (*curwin).w_cursor.col + 1,
                        (*curwin).w_virtcol + 1,
                    );
                    col_print(
                        buf2_ptr,
                        buf2.len(),
                        get_cursor_line_len(),
                        linetabsize_str(p),
                    );

                    if char_count_cursor == byte_count_cursor && char_count == byte_count {
                        vim_snprintf!(
                            iobuff_ptr,
                            IOSIZE,
                            tr(
                                c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Byte %lld of %lld"
                                    .as_ptr(),
                            ),
                            buf1_ptr,
                            buf2_ptr,
                            i64::from((*curwin).w_cursor.lnum) as std::ffi::c_longlong,
                            i64::from((*curbuf).b_ml.ml_line_count) as std::ffi::c_longlong,
                            word_count_cursor as std::ffi::c_longlong,
                            word_count as std::ffi::c_longlong,
                            byte_count_cursor as std::ffi::c_longlong,
                            byte_count as std::ffi::c_longlong,
                        );
                    } else {
                        vim_snprintf!(
                            iobuff_ptr,
                            IOSIZE,
                            tr(
                                c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Char %lld of %lld; Byte %lld of %lld"
                                    .as_ptr(),
                            ),
                            buf1_ptr,
                            buf2_ptr,
                            i64::from((*curwin).w_cursor.lnum) as std::ffi::c_longlong,
                            i64::from((*curbuf).b_ml.ml_line_count) as std::ffi::c_longlong,
                            word_count_cursor as std::ffi::c_longlong,
                            word_count as std::ffi::c_longlong,
                            char_count_cursor as std::ffi::c_longlong,
                            char_count as std::ffi::c_longlong,
                            byte_count_cursor as std::ffi::c_longlong,
                            byte_count as std::ffi::c_longlong,
                        );
                    }
                }
            }

            bom_count = varnumber_T::from(bomb_size());
            if dict.is_null() && bom_count > 0 {
                let len = CStr::from_ptr(iobuff_ptr).to_bytes().len();
                vim_snprintf!(
                    iobuff_ptr.add(len),
                    IOSIZE - len,
                    tr(c"(+%lld for BOM)".as_ptr()),
                    bom_count as std::ffi::c_longlong,
                );
            }
            if dict.is_null() {
                let p = p_shm;
                p_shm = c"".as_ptr().cast_mut();
                if p_ch < 1 {
                    msg_start();
                    msg_scroll = 1;
                }
                msg(iobuff_ptr, 0);
                p_shm = p;
            }
        }

        if !dict.is_null() {
            tv_dict_add_nr(dict, c"words".as_ptr(), 5, word_count);
            tv_dict_add_nr(dict, c"chars".as_ptr(), 5, char_count);
            tv_dict_add_nr(dict, c"bytes".as_ptr(), 5, byte_count + bom_count);

            tv_dict_add_nr(
                dict,
                if l_VIsual_active {
                    c"visual_bytes".as_ptr()
                } else {
                    c"cursor_bytes".as_ptr()
                },
                12,
                byte_count_cursor,
            );
            tv_dict_add_nr(
                dict,
                if l_VIsual_active {
                    c"visual_chars".as_ptr()
                } else {
                    c"cursor_chars".as_ptr()
                },
                12,
                char_count_cursor,
            );
            tv_dict_add_nr(
                dict,
                if l_VIsual_active {
                    c"visual_words".as_ptr()
                } else {
                    c"cursor_words".as_ptr()
                },
                12,
                word_count_cursor,
            );
        }
    }
}

unsafe fn op_colon(oap: *mut oparg_T) {
    unsafe {
        if (*oap).is_VIsual {
            stuffReadbuff(c"'<,'>".as_ptr());
        } else {
            if (*oap).start.lnum == (*curwin).w_cursor.lnum {
                stuffcharReadbuff(b'.' as c_int);
            } else {
                stuffnumReadbuff((*oap).start.lnum);
            }

            let mut endOfStartFold = (*oap).start.lnum;
            hasFolding(
                curwin,
                (*oap).start.lnum,
                ptr::null_mut(),
                &raw mut endOfStartFold,
            );
            if (*oap).end.lnum != (*oap).start.lnum && (*oap).end.lnum != endOfStartFold {
                stuffcharReadbuff(b',' as c_int);
                if (*oap).end.lnum == (*curwin).w_cursor.lnum {
                    stuffcharReadbuff(b'.' as c_int);
                } else if (*oap).end.lnum == (*curbuf).b_ml.ml_line_count {
                    stuffcharReadbuff(b'$' as c_int);
                } else if (*oap).start.lnum == (*curwin).w_cursor.lnum
                    && !hasFolding(curwin, (*oap).end.lnum, ptr::null_mut(), ptr::null_mut())
                {
                    stuffReadbuff(c".+".as_ptr());
                    stuffnumReadbuff((*oap).line_count - 1);
                } else {
                    stuffnumReadbuff((*oap).end.lnum);
                }
            }
        }
        if (*oap).op_type != OP_COLON {
            stuffReadbuff(c"!".as_ptr());
        }
        if (*oap).op_type == OP_INDENT {
            stuffReadbuff(get_equalprg());
            stuffReadbuff(c"\n".as_ptr());
        } else if (*oap).op_type == OP_FORMAT {
            if *(*curbuf).b_p_fp != NUL {
                stuffReadbuff((*curbuf).b_p_fp);
            } else if *p_fp != NUL {
                stuffReadbuff(p_fp);
            } else {
                stuffReadbuff(c"fmt".as_ptr());
            }
            stuffReadbuff(c"\n']".as_ptr());
        }

        do_cmdline(ptr::null_mut(), Some(getexline), ptr::null_mut(), 0);
        atom_suppress(true);
        exec_stuffed(ptr::null_mut());
        atom_suppress(false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_operatorfunc_option() {
    unsafe {
        callback_free(&raw mut p_opfunc);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_ref_in_opfunc(copyID: c_int) -> bool {
    unsafe { set_ref_in_callback(&raw mut p_opfunc, copyID, ptr::null_mut(), ptr::null_mut()) }
}

unsafe fn op_function(oap: *const oparg_T) {
    unsafe {
        let orig_start = (*curbuf).b_op_start;
        let orig_end = (*curbuf).b_op_end;

        if p_opfunc.cb_type == kCallbackNone {
            emsg(tr(c"E774: 'operatorfunc' is empty".as_ptr()));
        } else {
            (*curbuf).b_op_start = (*oap).start;
            (*curbuf).b_op_end = (*oap).end;
            if (*oap).motion_type == kMTCharWise && !(*oap).inclusive {
                decl(&raw mut (*curbuf).b_op_end);
            }

            let mode_str = match (*oap).motion_type {
                kMTBlockWise => c"block".as_ptr(),
                kMTLineWise => c"line".as_ptr(),
                _ => c"char".as_ptr(),
            };
            let mut argv = [
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval {
                        v_string: mode_str.cast_mut(),
                    },
                },
                typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval {
                        v_string: ptr::null_mut(),
                    },
                },
            ];

            let save_virtual_op = virtual_op;
            virtual_op = kNone;

            let save_finish_op = finish_op;
            finish_op = false;

            let mut save_redo: RedoState = std::mem::zeroed();
            save_redobuff(&raw mut save_redo);

            let mut rettv: typval_T = std::mem::zeroed();
            if callback_call(
                &raw mut p_opfunc,
                1,
                (&raw mut argv).cast::<typval_T>(),
                &raw mut rettv,
            ) {
                tv_clear(&raw mut rettv);
            }
            restore_redobuff(&raw mut save_redo);

            virtual_op = save_virtual_op;
            finish_op = save_finish_op;
            if (cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
                (*curbuf).b_op_start = orig_start;
                (*curbuf).b_op_end = orig_end;
            }
        }
    }
}

unsafe fn get_op_vcol(oap: *mut oparg_T, initial: bool) {
    unsafe {
        let mut start: colnr_T = 0;
        let mut end: colnr_T = 0;

        if Visual.mode != Ctrl_V || (!initial && (*oap).end.col < (*curwin).w_view_width) {
            return;
        }

        mark_mb_adjustpos((*curwin).w_buffer, &raw mut (*oap).end);

        getvvcol(
            curwin,
            &raw mut (*oap).start,
            &raw mut (*oap).start_vcol,
            ptr::null_mut(),
            &raw mut (*oap).end_vcol,
            0,
        );
        getvvcol(
            curwin,
            &raw mut (*oap).end,
            &raw mut start,
            ptr::null_mut(),
            &raw mut end,
            0,
        );

        (*oap).start_vcol = (*oap).start_vcol.min(start);
        if end > (*oap).end_vcol {
            if initial && *p_sel == b'e' as c_char && start >= 1 && start > (*oap).end_vcol {
                (*oap).end_vcol = start - 1;
            } else {
                (*oap).end_vcol = end;
            }
        }

        if (*curwin).w_curswant == MAXCOL {
            (*curwin).w_cursor.col = MAXCOL;
            (*oap).end_vcol = 0;
            (*curwin).w_cursor.lnum = (*oap).start.lnum;
            while (*curwin).w_cursor.lnum <= (*oap).end.lnum {
                getvvcol(
                    curwin,
                    &raw mut (*curwin).w_cursor,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &raw mut end,
                    0,
                );
                (*oap).end_vcol = (*oap).end_vcol.max(end);
                (*curwin).w_cursor.lnum += 1;
            }
        }

        (*curwin).w_cursor.lnum = (*oap).end.lnum;
        coladvance(curwin, (*oap).end_vcol);
        (*oap).end = (*curwin).w_cursor;

        (*curwin).w_cursor = (*oap).start;
        coladvance(curwin, (*oap).start_vcol);
        (*oap).start = (*curwin).w_cursor;
    }
}

unsafe fn is_ex_cmdchar(cap: *const cmdarg_T) -> bool {
    unsafe { (*cap).cmdchar == b':' as c_int || (*cap).cmdchar == K_COMMAND }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn op_self_select(cap: *const cmdarg_T) -> bool {
    unsafe {
        (*cap).cmdchar == b'g' as c_int
            && ((*cap).nchar == b'n' as c_int
                || (*cap).nchar == b'N' as c_int
                || (*cap).nchar == b'v' as c_int)
    }
}

unsafe fn op_ins_visual(oap: *mut oparg_T, cap: *mut cmdarg_T) -> VisualIns {
    unsafe {
        if !(*oap).is_VIsual {
            return kVInsNone;
        }
        if is_ex_cmdchar(cap) || (*cap).cmdchar == K_LUA {
            return kVInsMotion;
        }
        if op_self_select(cap)
            && (*cap).nchar != b'v' as c_int
            && (*oap).motion_force == c_int::from(NUL)
        {
            return kVInsMotion;
        }
        if (*oap).motion_force != c_int::from(NUL) || op_self_select(cap) {
            return kVInsOther;
        }
        if atom_visual_redoable() {
            kVInsKeys
        } else {
            kVInsOther
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_pending_operator(cap: *mut cmdarg_T, old_col: c_int, gui_yank: bool) {
    unsafe {
        let oap = (*cap).oap;
        let lbr_saved = (*curwin).w_onebuf_opt.wo_lbr != 0;

        let old_cursor = (*curwin).w_cursor;
        let op_origin = (*oap).start;

        if (finish_op || Visual.active) && (*oap).op_type != OP_NOP {
            let mut include_line_break = false;
            let redo_yank = !vim_strchr(p_cpo, kCpoYank).is_null() && !gui_yank;

            reset_lbr();
            (*oap).is_VIsual = Visual.active;
            if (*oap).motion_force == b'V' as c_int {
                (*oap).motion_type = kMTLineWise;
            } else if (*oap).motion_force == b'v' as c_int {
                if (*oap).motion_type == kMTLineWise {
                    (*oap).inclusive = false;
                } else if (*oap).motion_type == kMTCharWise {
                    (*oap).inclusive = !(*oap).inclusive;
                }
                (*oap).motion_type = kMTCharWise;
            } else if (*oap).motion_force == Ctrl_V {
                if !Visual.active {
                    Visual.active = true;
                    Visual.start = (*oap).start;
                }
                Visual.mode = Ctrl_V;
                Visual.select = false;
                Visual.reselect = 0;
            }

            atom_capture_op(oap, cap, redo_yank);

            if Visual.active {
                if !gui_yank && !finish_op {
                    (*curbuf).b_visual = visualinfo();
                    restore_visual_mode();
                    (*curbuf).b_visual_mode_eval = Visual.mode;
                }

                if Visual.select
                    && Visual.mode == b'V' as c_int
                    && (*(*cap).oap).op_type != OP_DELETE
                {
                    if lt(Visual.start, (*curwin).w_cursor) {
                        Visual.start.col = 0;
                        (*curwin).w_cursor.col = ml_get_len((*curwin).w_cursor.lnum);
                    } else {
                        (*curwin).w_cursor.col = 0;
                        Visual.start.col = ml_get_len(Visual.start.lnum);
                    }
                    Visual.mode = b'v' as c_int;
                } else if Visual.mode == b'v' as c_int {
                    include_line_break = unadjust_for_sel();
                }

                (*oap).start = Visual.start;
                if Visual.mode == b'V' as c_int {
                    (*oap).start.col = 0;
                    (*oap).start.coladd = 0;
                }
            }

            if lt((*oap).start, (*curwin).w_cursor) {
                if !Visual.active {
                    if hasFolding(
                        curwin,
                        (*oap).start.lnum,
                        &raw mut (*oap).start.lnum,
                        ptr::null_mut(),
                    ) {
                        (*oap).start.col = 0;
                    }
                    if ((*curwin).w_cursor.col > 0
                        || (*oap).inclusive
                        || (*oap).motion_type == kMTLineWise)
                        && hasFolding(
                            curwin,
                            (*curwin).w_cursor.lnum,
                            ptr::null_mut(),
                            &raw mut (*curwin).w_cursor.lnum,
                        )
                    {
                        (*curwin).w_cursor.col = get_cursor_line_len();
                    }
                }
                (*oap).end = (*curwin).w_cursor;
                (*curwin).w_cursor = (*oap).start;

                (*curwin).w_valid &= !VALID_VIRTCOL;
            } else {
                if !Visual.active && (*oap).motion_type == kMTLineWise {
                    if hasFolding(
                        curwin,
                        (*curwin).w_cursor.lnum,
                        &raw mut (*curwin).w_cursor.lnum,
                        ptr::null_mut(),
                    ) {
                        (*curwin).w_cursor.col = 0;
                    }
                    if hasFolding(
                        curwin,
                        (*oap).start.lnum,
                        ptr::null_mut(),
                        &raw mut (*oap).start.lnum,
                    ) {
                        (*oap).start.col = ml_get_len((*oap).start.lnum);
                    }
                }
                (*oap).end = (*oap).start;
                (*oap).start = (*curwin).w_cursor;
            }

            check_pos((*curwin).w_buffer, &raw mut (*oap).end);
            (*oap).line_count = (*oap).end.lnum - (*oap).start.lnum + 1;

            virtual_op = TriState::from(virtual_active(curwin));

            if Visual.active {
                get_op_vcol(oap, true);

                if !gui_yank {
                    Visual.resel.mode = Visual.mode;
                    if (*curwin).w_curswant == MAXCOL {
                        Visual.resel.vcol = MAXCOL;
                    } else {
                        if Visual.mode != Ctrl_V {
                            getvvcol(
                                curwin,
                                &raw mut (*oap).end,
                                ptr::null_mut(),
                                ptr::null_mut(),
                                &raw mut (*oap).end_vcol,
                                0,
                            );
                        }
                        if Visual.mode == Ctrl_V || (*oap).line_count <= 1 {
                            if Visual.mode != Ctrl_V {
                                getvvcol(
                                    curwin,
                                    &raw mut (*oap).start,
                                    &raw mut (*oap).start_vcol,
                                    ptr::null_mut(),
                                    ptr::null_mut(),
                                    0,
                                );
                            }
                            Visual.resel.vcol = (*oap).end_vcol - (*oap).start_vcol + 1;
                        } else {
                            Visual.resel.vcol = (*oap).end_vcol;
                        }
                    }
                    Visual.resel.line_count = (*oap).line_count;
                }

                if (*oap).motion_force == c_int::from(NUL) || (*oap).motion_type == kMTLineWise {
                    (*oap).inclusive = true;
                }
                if Visual.mode == b'V' as c_int {
                    (*oap).motion_type = kMTLineWise;
                } else if Visual.mode == b'v' as c_int {
                    (*oap).motion_type = kMTCharWise;
                    if *ml_get_pos(&raw const (*oap).end) == NUL
                        && (include_line_break || virtual_op == 0)
                    {
                        (*oap).inclusive = false;
                        if *p_sel != b'o' as c_char
                            && op_on_lines((*oap).op_type) == 0
                            && (*oap).end.lnum < (*curbuf).b_ml.ml_line_count
                        {
                            (*oap).end.lnum += 1;
                            (*oap).end.col = 0;
                            (*oap).end.coladd = 0;
                            (*oap).line_count += 1;
                        }
                    }
                } else if Visual.mode == Ctrl_V {
                    (*oap).motion_type = kMTBlockWise;
                }

                if !gui_yank {
                    Visual.active = false;
                    setmouse();
                    mouse_dragging = 0;
                    may_clear_cmdline();
                    if ((*oap).op_type == OP_YANK
                        || (*oap).op_type == OP_COLON
                        || (*oap).op_type == OP_FUNCTION
                        || (*oap).op_type == OP_FILTER)
                        && (*oap).motion_force == c_int::from(NUL)
                    {
                        restore_lbr(lbr_saved);
                        redraw_curbuf_later(UPD_INVERTED);
                    }
                }
            }

            if (*oap).inclusive {
                let l = utfc_ptr2len(ml_get_pos(&raw const (*oap).end));
                if l > 1 {
                    (*oap).end.col += l - 1;
                }
            }
            (*curwin).w_set_curswant = true;

            (*oap).empty = (*oap).motion_type != kMTLineWise
                && (!(*oap).inclusive
                    || ((*oap).op_type == OP_YANK
                        && gchar_pos(&raw mut (*oap).end) == c_int::from(NUL)))
                && equalpos((*oap).start, (*oap).end)
                && !(virtual_op != 0 && (*oap).start.coladd != (*oap).end.coladd);
            let empty_region_error = (*oap).empty && !vim_strchr(p_cpo, kCpoEmptyregion).is_null();

            if (*oap).is_VIsual
                && ((*oap).empty || !modifiable(curbuf) || (*oap).op_type == OP_FOLD)
            {
                restore_lbr(lbr_saved);
                redraw_curbuf_later(UPD_INVERTED);
            }

            if (*oap).motion_type == kMTCharWise
                && !(*oap).inclusive
                && ((*cap).retval & CA_NO_ADJ_OP_END) == 0
                && (*oap).end.col == 0
                && (!(*oap).is_VIsual || *p_sel == b'o' as c_char)
                && (*oap).line_count > 1
            {
                (*oap).end_adjusted = true;
                (*oap).line_count -= 1;
                (*oap).end.lnum -= 1;
                if inindent(0) {
                    (*oap).motion_type = kMTLineWise;
                } else {
                    (*oap).end.col = ml_get_len((*oap).end.lnum);
                    if (*oap).end.col != 0 {
                        (*oap).end.col -= 1;
                        (*oap).inclusive = true;
                    }
                }
            } else {
                (*oap).end_adjusted = false;
            }

            match (*oap).op_type {
                OP_LSHIFT | OP_RSHIFT => {
                    op_shift(oap, true, if (*oap).is_VIsual { (*cap).count1 } else { 1 });
                    auto_format(false, true);
                }
                OP_JOIN_NS | OP_JOIN => {
                    (*oap).line_count = (*oap).line_count.max(2);
                    if (*curwin).w_cursor.lnum + (*oap).line_count - 1
                        > (*curbuf).b_ml.ml_line_count
                    {
                        beep_flush();
                    } else {
                        do_join(
                            (*oap).line_count as usize,
                            (*oap).op_type == OP_JOIN,
                            true,
                            true,
                            true,
                        );
                        auto_format(false, true);
                    }
                }
                OP_DELETE => {
                    Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        op_delete(oap);
                        if (*oap).motion_type == kMTLineWise
                            && has_format_option(kFoAuto)
                            && u_save_cursor() == OK
                        {
                            auto_format(false, true);
                        }
                    }
                }
                OP_YANK => {
                    if empty_region_error {
                        if !gui_yank {
                            vim_beep(kOptBoFlagOperator);
                            redo_cancel();
                        }
                    } else {
                        restore_lbr(lbr_saved);
                        (*oap).excl_tr_ws = (*cap).cmdchar == b'z' as c_int;
                        if (*oap).restore_cursor {
                            (*curwin).w_cursor = (*oap).cursor_start;
                        }
                        op_yank(oap, !gui_yank);
                    }
                    check_cursor_col(curwin);
                }
                OP_CHANGE => {
                    Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        let restart_edit_save = if !KeyTyped { restart_edit } else { 0 };
                        restart_edit = 0;

                        restore_lbr(lbr_saved);

                        (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);

                        let mut session: InsSession = atom_ins_start(
                            b'c' as c_int,
                            c_long::from((*cap).count0),
                            op_ins_visual(oap, cap),
                            (*oap).is_VIsual && (*oap).motion_type == kMTBlockWise,
                        );
                        let busy = op_change(oap) != 0;
                        if busy {
                            (*cap).retval |= CA_COMMAND_BUSY;
                        }
                        if restart_edit == 0 {
                            restart_edit = restart_edit_save;
                        }
                        atom_ins_end(&raw mut session, busy);
                    }
                }
                OP_FILTER | OP_INDENT | OP_COLON => {
                    if (*oap).op_type == OP_FILTER {
                        if !vim_strchr(p_cpo, kCpoFilter).is_null() {
                            redo_append_str(c"!\r".as_ptr(), 2);
                        } else {
                            bangredo = true;
                        }
                    }

                    if (*oap).op_type == OP_INDENT && *get_equalprg() == NUL {
                        if (*curbuf).b_p_lisp != 0 {
                            if use_indentexpr_for_lisp() {
                                op_reindent(oap, get_expr_indent);
                            } else {
                                op_reindent(oap, get_lisp_indent);
                            }
                        } else {
                            op_reindent(
                                oap,
                                if (*curbuf).b_p_inde.cb_type != kCallbackNone {
                                    get_expr_indent
                                } else {
                                    get_c_indent
                                },
                            );
                        }
                    } else {
                        op_colon(oap);
                    }
                }
                OP_TILDE | OP_UPPER | OP_LOWER | OP_ROT13 => {
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        op_tilde(oap);
                    }
                    check_cursor_col(curwin);
                }
                OP_FORMAT => {
                    if (*curbuf).b_p_fex.cb_type != kCallbackNone {
                        op_formatexpr(oap);
                    } else if *p_fp != NUL || *(*curbuf).b_p_fp != NUL {
                        op_colon(oap);
                    } else {
                        op_format(oap, false);
                    }
                }
                OP_FORMAT2 => {
                    op_format(oap, true);
                }
                OP_FUNCTION => {
                    restore_lbr(lbr_saved);
                    op_function(oap);
                }
                OP_INSERT | OP_APPEND => {
                    Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        let restart_edit_save = restart_edit;
                        restart_edit = 0;

                        restore_lbr(lbr_saved);

                        (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);

                        let mut session: InsSession = atom_ins_start(
                            b'I' as c_int,
                            c_long::from((*cap).count1),
                            op_ins_visual(oap, cap),
                            (*oap).motion_type == kMTBlockWise,
                        );
                        op_insert(oap, (*cap).count1);
                        atom_ins_end(&raw mut session, false);

                        reset_lbr();
                        auto_format(false, true);

                        if restart_edit == 0 {
                            restart_edit = restart_edit_save;
                        } else {
                            (*cap).retval |= CA_COMMAND_BUSY;
                        }
                    }
                }
                OP_REPLACE => {
                    Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        restore_lbr(lbr_saved);
                        op_replace(oap, (*cap).nchar);
                    }
                }
                OP_FOLD => {
                    Visual.reselect = 0;
                    foldCreate(curwin, (*oap).start, (*oap).end);
                }
                OP_MCURSOR => {
                    restore_lbr(lbr_saved);
                    mc_zq(oap, cap, op_origin);
                }
                OP_FOLDOPEN | OP_FOLDOPENREC | OP_FOLDCLOSE | OP_FOLDCLOSEREC => {
                    Visual.reselect = 0;
                    opFoldRange(
                        (*oap).start,
                        (*oap).end,
                        c_int::from(
                            (*oap).op_type == OP_FOLDOPEN || (*oap).op_type == OP_FOLDOPENREC,
                        ),
                        c_int::from(
                            (*oap).op_type == OP_FOLDOPENREC || (*oap).op_type == OP_FOLDCLOSEREC,
                        ),
                        (*oap).is_VIsual,
                    );
                }
                OP_FOLDDEL | OP_FOLDDELREC => {
                    Visual.reselect = 0;
                    deleteFold(
                        curwin,
                        (*oap).start.lnum,
                        (*oap).end.lnum,
                        c_int::from((*oap).op_type == OP_FOLDDELREC),
                        (*oap).is_VIsual,
                    );
                }
                OP_NR_ADD | OP_NR_SUB => {
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        Visual.active = true;
                        restore_lbr(lbr_saved);
                        op_addsub(oap, (*cap).count1, (*cap).arg != 0);
                        Visual.active = false;
                    }
                    check_cursor_col(curwin);
                }
                _ => {
                    clearopbeep(oap);
                }
            }
            virtual_op = kNone;
            if !gui_yank {
                if p_sol == 0
                    && (*oap).motion_type == kMTLineWise
                    && !(*oap).end_adjusted
                    && ((*oap).op_type == OP_LSHIFT
                        || (*oap).op_type == OP_RSHIFT
                        || (*oap).op_type == OP_DELETE)
                {
                    reset_lbr();
                    (*curwin).w_curswant = old_col;
                    coladvance(curwin, old_col);
                }
            } else {
                (*curwin).w_cursor = old_cursor;
            }
            clearop(oap);
            motion_force = c_int::from(NUL);
        }
        restore_lbr(lbr_saved);
    }
}

/// Get the byte count of buffer region. End-exclusive.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_region_bytecount(
    buf: *mut buf_T,
    start_lnum: linenr_T,
    end_lnum: linenr_T,
    start_col: colnr_T,
    end_col: colnr_T,
) -> bcount_t {
    unsafe {
        let max_lnum = (*buf).b_ml.ml_line_count;
        if start_lnum > max_lnum {
            return 0;
        }
        if start_lnum == end_lnum {
            return (end_col - start_col) as bcount_t;
        }
        let mut deleted_bytes = (ml_get_buf_len(buf, start_lnum) - start_col + 1) as bcount_t;

        for i in 1..=(end_lnum - start_lnum - 1) {
            if start_lnum + i > max_lnum {
                return deleted_bytes;
            }
            deleted_bytes += (ml_get_buf_len(buf, start_lnum + i) + 1) as bcount_t;
        }
        if end_lnum > max_lnum {
            return deleted_bytes;
        }
        deleted_bytes + (end_col as bcount_t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_char_and_rot13_helpers() {
        unsafe {
            assert_eq!(get_op_char(OP_UPPER), b'g' as c_int);
            assert_eq!(get_extra_op_char(OP_UPPER), b'U' as c_int);
            assert_ne!(op_on_lines(OP_INDENT), 0);
            assert_eq!(op_on_lines(OP_DELETE), 0);
            assert!(op_redoable(OP_DELETE, false));
            assert!(!op_redoable(OP_YANK, false));
            assert!(op_redoable(OP_YANK, true));
        }
        assert_eq!(rot13(b'a' as c_int, b'a' as c_int), b'n' as c_int);
        assert_eq!(rot13(b'n' as c_int, b'a' as c_int), b'a' as c_int);
        assert_eq!(char_ord(b'C' as c_int), 2);
        assert_eq!(char_ord(b'c' as c_int), 2);
    }
}
