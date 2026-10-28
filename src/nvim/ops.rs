//! Implementation of various operators: `op_shift`, `op_delete`, `op_tilde`,
//! `op_change`, `op_yank`, `do_join` (`src/nvim/ops.c`).

use crate::buffer::{FileBuf, buf_get_changedtick, col_print, modifiable};
use crate::change::{
    COM_END, appended_lines_mark, changed_bytes, changed_lines, del_bytes, del_char, del_lines,
    get_last_leader_offset, get_leader_len, ins_char, ins_str, kCpoDollar, kOptVeFlagOnemore,
    truncate_line,
};
use crate::charset::{
    NUL, STR2NR_BIN, STR2NR_HEX, STR2NR_OCT, ascii_isalpha, ascii_isbdigit, ascii_isdigit,
    ascii_isspace, ascii_isupper, ascii_iswhite, ascii_isxdigit, getwhitecols,
    getwhitecols_curline, skipwhite, Uvarnumber, vim_str2nr,
};
use crate::clipboard::set_clipboard;
use crate::cursor::{
    VALID_VIRTCOL, check_cursor, check_cursor_col, check_pos, coladvance, coladvance_force,
    Colnr, dec_cursor, equalpos, gchar_cursor, get_cursor_line_len, get_cursor_line_ptr,
    get_cursor_pos_ptr, getviscol, getviscol2, getvpos, inc_cursor, kOptVeFlagAll, Linenr, lt,
    ltoreq, CursorPos,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later, update_screen};
use crate::errors::*;
use crate::eval::typval::{
    TypvalVval, VarType, callback_free, TvDict, kCallbackNone, tv_clear, tv_dict_add_nr, TypVal,
    Varnumber,
};
use crate::eval::{callback_call, set_ref_in_callback};
use crate::ex_docmd::{CMOD_LOCKMARKS, do_cmdline};
use crate::ex_getln::{Ctrl_A, K_LUA};
use crate::extmark::{
    TriState, Bcount, extmark_splice, extmark_splice_cols, kExtmarkUndo, kNone,
};
use crate::fold::{deleteFold, foldCreate, foldOpenCursor, hasFolding, opFoldRange};
use crate::globals::*;
use crate::indent::{
    INDENT_SET, SIN_CHANGED, change_indent, fix_indent, get_expr_indent, get_indent,
    get_lisp_indent, get_sw_value_indent, inindent, may_do_si, op_reindent, preprocs_left,
    set_indent, tabstop_fromto, use_indentexpr_for_lisp,
};
use crate::indent_c::get_c_indent;
use crate::input::{
    RedoState, beep_flush, redo_append_str, redo_cancel, restore_redobuff, save_redobuff,
    stuffReadbuff, stuffcharReadbuff, stuffnumReadbuff,
};
use crate::input_cmdatom::{
    InsSession, VisualIns, atom_capture_op, atom_ins_end, atom_ins_start, atom_suppress,
    atom_visual_redoable, Cmdarg, kVInsKeys, kVInsMotion, kVInsNone, kVInsOther,
};
use crate::insert::{Ctrl_X, beginline, display_dollar, edit, kFoAuto, linetabsize_str};
use crate::keycodes::{CAR, K_COMMAND, NL, TAB};
use crate::mark::{mark_col_adjust, mark_mb_adjustpos};
use crate::math::trim_to_int;
use crate::mbyte::{
    bomb_size, mb_islower, mb_isupper, mb_prevptr, mb_tolower, mb_toupper, utf_ClusterInfo,
    utf_char2bytes, utf_char2cells, utf_char2len, utf_eat_space, utf_head_off, utf_ptr2StrCharInfo,
    utf_ptr2char, utf_ptr2len, utfc_ptr2len,
};
use crate::mcursor::mc_zq;
use crate::memline::{
    dec, decl, gchar_pos, inc, ml_append, ml_get, ml_get_buf_len, ml_get_buf_mut, ml_get_len,
    ml_get_pos, ml_get_pos_len, ml_replace, ml_replace_len,
};
use crate::message::{emsg, internal_error, msg, msg_keep, msg_start, msgmore, smsg};
use crate::mouse::setmouse;
use crate::r#move::validate_virtcol;
use crate::normal::{
    CA_COMMAND_BUSY, CA_NO_ADJ_OP_END, REPLACE_CR_NCHAR, REPLACE_NL_NCHAR, clearop, clearopbeep,
    exec_stuffed, lineempty, may_clear_cmdline, restore_visual_mode, unadjust_for_sel,
    unadjust_for_sel_inner, visualinfo,
};
use crate::option::{get_equalprg, get_fileformat, get_ve_flags};
use crate::option_vars::*;
use crate::os::input::{line_breakcheck, os_breakcheck};
use crate::plines::{
    CSType, CharsizeArg, getvcol, getvcols, getvvcol, init_charsize_arg, win_charsize,
};
use crate::register::{
    YREG_YANK, BlockDef, do_autocmd_textyankpost, get_y_register, get_yank_register,
    is_append_register, op_yank, op_yank_reg, shift_delete_registers, valid_yank_reg, Yankreg,
};
use crate::state::{MODE_INSERT, MODE_REPLACE, VREPLACE_FLAG, virtual_active};
use crate::strings::{ngettext, tr, vim_snprintf, vim_strchr};
use crate::textformat::{auto_format, has_format_option, op_format, op_formatexpr, Oparg};
use crate::ui::vim_beep;
use crate::undo::{u_clearline, u_save, u_save_cursor};
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong};
use std::ptr;

use crate::arglist::ML_EMPTY;
use crate::diff::kOptBoFlagOperator;
use crate::drawline::{VALID_WCOL, VALID_WROW};
use crate::eval::funcs::{kMTBlockWise, kMTCharWise, kMTLineWise};
use crate::ex_cmds::{BL_FIX, BL_SOL, BL_WHITE};
use crate::fileio::EOL_DOS;
use crate::globals::IOSIZE;
use crate::regexp::{Ctrl_V, MAXCOL, NUMBUFLEN};

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

const kCpoEmptyregion: c_int = b'E' as c_int;
const kCpoFilter: c_int = b'!' as c_int;
const kCpoJoincol: c_int = b'q' as c_int;
const kCpoYank: c_int = b'y' as c_int;

const kFoMbyteJoin: c_int = b'M' as c_int;
const kFoMbyteJoin2: c_int = b'B' as c_int;
const kFoRemoveComs: c_int = b'j' as c_int;

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

struct OpsModuleState {
    addsub_hexupper: bool,
}

impl OpsModuleState {
    const fn new() -> Self {
        Self {
            addsub_hexupper: false,
        }
    }
}

static OPS_STATE: crate::globals::GlobalState<OpsModuleState> = crate::globals::GlobalState::new(OpsModuleState::new());

#[inline]
fn ops_st() -> &'static mut OpsModuleState {
    unsafe { &mut *OPS_STATE.as_ptr() }
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

/// Translate a command name into an operator type.
pub fn get_op_type(char1: c_int, char2: c_int) -> c_int {
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
                internal_error(c"get_op_type()");
                break;
            }
            i += 1;
        }
        i as c_int
}

/// Return non-zero if operator `op` always works on whole lines.
pub fn op_on_lines(op: c_int) -> c_int {
    c_int::from(OPCHARS[op as usize][2] & OPF_LINES)
}

/// Return non-zero if operator `op` changes text.
pub fn op_is_change(op: c_int) -> c_int {
    c_int::from(OPCHARS[op as usize][2] & OPF_CHANGE)
}

/// Whether operator `op` builds a redo: text-changing operators do, ":" and the fold operators do
/// not. Yank with the 'y' flag in 'cpoptions'.
pub fn op_redoable(op: c_int, redo_yank: bool) -> bool {
    if op == OP_YANK {
            redo_yank
        } else {
            op_is_change(op) != 0
        }
}

/// Get first operator command character.
pub fn get_op_char(optype: c_int) -> c_int {
    c_int::from(OPCHARS[optype as usize][0])
}

/// Get second operator command character.
pub fn get_extra_op_char(optype: c_int) -> c_int {
    c_int::from(OPCHARS[optype as usize][1])
}

/// Handle a shift operation.
pub fn op_shift(oap: &mut Oparg, curs_top: bool, amount: c_int) {
     {
        let mut block_col: c_int = 0;

        if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
            return;
        }

        if oap.motion_type == kMTBlockWise {
            block_col = curwin_mut().w_cursor.col;
        }

        let mut i = oap.line_count - 1;
        while i >= 0 {
            let first_char = c_int::from(crate::memory::cstr_first_byte(get_cursor_line_ptr()));
            if first_char == 0 {
                curwin_mut().w_cursor.col = 0;
            } else if oap.motion_type == kMTBlockWise {
                shift_block(oap, amount);
            } else if first_char != b'#' as c_int || !preprocs_left() {
                shift_line(oap.op_type == OP_LSHIFT, opt_vars().p_sr != 0, amount, 0);
            }
            curwin_mut().w_cursor.lnum += 1;
            i -= 1;
        }

        if oap.motion_type == kMTBlockWise {
            curwin_mut().w_cursor.lnum = oap.start.lnum;
            curwin_mut().w_cursor.col = block_col;
        } else if curs_top {
            curwin_mut().w_cursor.lnum = oap.start.lnum;
            beginline(BL_SOL | BL_FIX);
        } else {
            curwin_mut().w_cursor.lnum -= 1;
        }
        foldOpenCursor();

        if i64::from(oap.line_count) > opt_vars().p_report {
            let op = if oap.op_type == OP_RSHIFT {
                c">".as_ptr()
            } else {
                c"<".as_ptr()
            };

            let msg_line_single = ngettext(
                c"%lld line %sed %d time",
                c"%lld line %sed %d times",
                amount as c_ulong,
            );
            let msg_line_plural = ngettext(
                c"%lld lines %sed %d time",
                c"%lld lines %sed %d times",
                amount as c_ulong,
            );
            let iobuff_ptr = g_st().IObuff.as_mut_ptr();
            vim_snprintf!(
                iobuff_ptr,
                IOSIZE,
                ngettext(
                    msg_line_single,
                    msg_line_plural,
                    oap.line_count as c_ulong,
                ),
                i64::from(oap.line_count) as std::ffi::c_longlong,
                op,
                amount,
            );
            msg_keep(iobuff_ptr, 0, true, false);
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end.lnum = oap.end.lnum;
            curbuf_mut().b_op_end.col = ml_get_len(oap.end.lnum);
            if curbuf_mut().b_op_end.col > 0 {
                curbuf_mut().b_op_end.col -= 1;
            }
        }

        changed_lines(curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
    }
}

fn get_vts(vts_array: &[c_int], index: c_int) -> c_int {
    if index < 1 {
        0
    } else if (index as usize) <= vts_array.len() {
        vts_array[index as usize - 1]
    } else {
        *vts_array.last().unwrap()
    }
}

fn get_vts_sum(vts_array: &[c_int], index: c_int) -> c_int {
    let mut sum: c_int = 0;
    let mut i: c_int = 1;
    let len = vts_array.len() as c_int;
    while i <= index && i <= len {
        sum += vts_array[i as usize - 1];
        i += 1;
    }
    if i <= index {
        sum += *vts_array.last().unwrap() * (index - len);
    }
    sum
}

fn get_new_sw_indent(left: bool, round: bool, mut amount: i64, sw_val: i64) -> i64 {
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

fn get_new_vts_indent(left: bool, round: bool, amount: c_int, vts_array: &[c_int]) -> i64 {
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

/// Shift the current line `amount` shiftwidth(s) left or right.
pub fn shift_line(left: bool, round: bool, amount: c_int, call_changed_bytes: c_int) {
     {
        let sw_val = curbuf_mut().b_p_sw;
        let ts_val = curbuf_mut().b_p_ts;
        let vts_array = &curbuf_mut().b_p_vts_array;

        let count = if sw_val != 0 {
            get_new_sw_indent(left, round, i64::from(amount), sw_val)
        } else if vts_array.is_empty() {
            get_new_sw_indent(left, round, i64::from(amount), ts_val)
        } else {
            get_new_vts_indent(left, round, amount, vts_array)
        };

        if (g_st().State & VREPLACE_FLAG) != 0 {
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

fn shift_block(oap: &mut Oparg, amount: c_int) {
    unsafe {
        let left = oap.op_type == OP_LSHIFT;
        let oldstate = g_st().State;
        let oldcol = curwin_mut().w_cursor.col;
        let sw_val = get_sw_value_indent(curbuf_mut(), left);
        let ts_val = curbuf_mut().b_p_ts as c_int;
        let mut bd = BlockDef::new();
        let old_p_ri = opt_vars().p_ri;

        opt_vars().p_ri = 0;

        g_st().State = MODE_INSERT;
        block_prep(oap, &mut bd, curwin_mut().w_cursor.lnum, true);
        if bd.is_short != 0 {
            return;
        }

        let mut total = (amount as c_uint).wrapping_mul(sw_val as c_uint) as c_int;
        if (total / sw_val) != amount {
            return;
        }

        let oldp = get_cursor_line_ptr();
        let old_line_len = get_cursor_line_len();

        let mut newp_vec: Vec<c_char>;
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

            let mut csarg = CharsizeArg::new();
            let cstype: CSType = init_charsize_arg(
                &mut csarg,
                curwin_mut(),
                curwin_mut().w_cursor.lnum,
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
                    &mut csarg,
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
            if curbuf_mut().b_p_et == 0 {
                tabstop_fromto(
                    ws_vcol,
                    ws_vcol + total,
                    ts_val,
                    &curbuf_mut().b_p_vts_array,
                    &mut tabs,
                    &mut spaces,
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
            newp_vec = vec![0 as c_char; (new_line_len as usize) + 1];
            newp = newp_vec.as_mut_ptr();
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
            crate::memory::cstr_cpy(newp.add((bd.textcol + tabs + spaces) as usize), bd.textstart);
            debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
        } else {
            let mut non_white = bd.textstart;

            if bd.startspaces != 0 {
                non_white = non_white.add(utfc_ptr2len(non_white) as usize);
            }

            let mut non_white_col = bd.start_vcol;

            let mut csarg = CharsizeArg::new();
            let mut cstype: CSType = init_charsize_arg(
                &mut csarg,
                curwin_mut(),
                curwin_mut().w_cursor.lnum,
                bd.textstart,
            );
            while ascii_iswhite(c_int::from(crate::memory::cstr_first_byte(non_white))) {
                let incr = win_charsize(
                    cstype,
                    non_white_col,
                    non_white,
                    i32::from(crate::memory::cstr_first_byte(non_white)),
                    &mut csarg,
                    1,
                )
                .width;
                non_white_col += incr;
                non_white = non_white.add(1);
            }

            let block_space_width = non_white_col - oap.start_vcol;
            let shift_amount = block_space_width.min(total);
            let destination_col = non_white_col - shift_amount;

            let mut verbatim_copy_end = bd.textstart;
            let mut verbatim_copy_width = bd.start_vcol;

            if bd.startspaces != 0 {
                verbatim_copy_width -= bd.start_char_vcols;
            }
            cstype = init_charsize_arg(&mut csarg, curwin_mut(), 0, bd.textstart);
            let mut ci = utf_ptr2StrCharInfo(verbatim_copy_end);
            while verbatim_copy_width < destination_col {
                let cli = utf_ClusterInfo(ci);
                let incr = win_charsize(
                    cstype,
                    verbatim_copy_width,
                    ci.ptr,
                    ci.chr.value,
                    &mut csarg,
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

            newp_vec = vec![0 as c_char; (new_line_len as usize) + 1];
            newp = newp_vec.as_mut_ptr();
            startcol = fixedlen;
            oldlen = bd.textcol + (non_white.offset_from(bd.textstart) as c_int) - fixedlen;
            newlen = fill;
            ptr::copy(oldp, newp, fixedlen as usize);
            ptr::write_bytes(newp.add(fixedlen as usize), b' ', fill as usize);
            crate::memory::cstr_cpy(newp.add((fixedlen + fill) as usize), non_white);
            debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
        }
        ml_replace(curwin_mut().w_cursor.lnum, newp, true);
        drop(newp_vec);
        changed_bytes(curwin_mut().w_cursor.lnum, bd.textcol);
        extmark_splice_cols(
            curbuf_mut(),
            curwin_mut().w_cursor.lnum - 1,
            startcol,
            oldlen,
            newlen,
            kExtmarkUndo,
        );
        g_st().State = oldstate;
        curwin_mut().w_cursor.col = oldcol;
        opt_vars().p_ri = old_p_ri;
    }
}

fn block_insert(
    oap: &mut Oparg,
    s: impl crate::strings::IntoCCharPtr,
    slen: usize,
    b_insert: bool,
    bdp: &mut BlockDef,
) { let s = s.into_c_char_ptr();
    unsafe {
        let mut ts_val: c_int;
        let mut count: c_int = 0;
        let mut spaces: c_int;
        let mut offset: Colnr;
        let oldstate = g_st().State;
        g_st().State = MODE_INSERT;

        for lnum in (oap.start.lnum + 1)..=oap.end.lnum {
            block_prep(oap, bdp, lnum, true);
            if bdp.is_short != 0 && b_insert {
                continue;
            }

            let mut oldp = ml_get(lnum);

            if b_insert {
                ts_val = bdp.start_char_vcols;
                spaces = bdp.startspaces;
                if spaces != 0 {
                    count = ts_val - 1;
                }
                offset = bdp.textcol;
            } else {
                ts_val = bdp.end_char_vcols;
                if bdp.is_short == 0 {
                    spaces = if bdp.endspaces != 0 {
                        ts_val - bdp.endspaces
                    } else {
                        0
                    };
                    if spaces != 0 {
                        count = ts_val - 1;
                    }
                    offset = bdp.textcol + bdp.textlen - c_int::from(spaces != 0);
                } else {
                    if bdp.is_MAX == 0 {
                        spaces = (oap.end_vcol - bdp.end_vcol) + 1;
                    } else {
                        spaces = 0;
                    }
                    count = spaces;
                    offset = bdp.textcol + bdp.textlen;
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
                + if spaces > 0 && bdp.is_short == 0 {
                    (ts_val - spaces) as usize
                } else {
                    0
                }
                + (count as usize)
                + 1;
            let mut newp_vec = vec![0 as c_char; alloc_len];
            let newp: *mut c_char = newp_vec.as_mut_ptr();

            ptr::copy(oldp, newp, offset as usize);
            oldp = oldp.add(offset as usize);
            let startcol = offset;

            ptr::write_bytes(newp.add(offset as usize), b' ', spaces as usize);

            ptr::copy(s, newp.add((offset + spaces) as usize), slen);
            offset += slen as c_int;

            let mut skipped: c_int = 0;
            if spaces > 0 && bdp.is_short == 0 {
                if crate::memory::cstr_first_byte(oldp) == b'\t' {
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
            crate::memory::cstr_cpy(newp.add(offset as usize), oldp);

            ml_replace(lnum, newp, true);
            drop(newp_vec);
            extmark_splice_cols(
                curbuf_mut(),
                lnum - 1,
                startcol,
                skipped,
                offset - startcol,
                kExtmarkUndo,
            );

            if lnum == oap.end.lnum {
                curbuf_mut().b_op_end.lnum = oap.end.lnum;
                curbuf_mut().b_op_end.col = offset;
                if curbuf_mut().b_visual.vi_end.coladd != 0 {
                    curbuf_mut().b_visual.vi_end.col += curbuf_mut().b_visual.vi_end.coladd;
                    curbuf_mut().b_visual.vi_end.coladd = 0;
                }
            }
        }

        g_st().State = oldstate;

        if oap.start.lnum < oap.end.lnum {
            changed_lines(
                curbuf_mut(),
                oap.start.lnum + 1,
                0,
                oap.end.lnum + 1,
                0,
                true,
            );
        }
    }
}

/// Handle a delete operation.
pub fn op_delete(oap: &mut Oparg) -> bool {
    unsafe {
        let mut bd = BlockDef::new();
        let old_lcount = curbuf_mut().b_ml.ml_line_count;

        if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
            return true;
        }

        if oap.empty {
            return u_save_cursor();
        }

        if !modifiable(curbuf_mut()) {
            emsg(tr(e_modifiable));
            return false;
        }

        if g_st().Visual.select && oap.is_VIsual {
            oap.regname = g_st().Visual.select_reg;
        }

        mb_adjust_opend(oap);

        if oap.motion_type == kMTCharWise
            && !oap.is_VIsual
            && oap.line_count > 1
            && oap.motion_force == c_int::from(NUL)
            && oap.op_type == OP_DELETE
        {
            let mut ptr_line = ml_get(oap.end.lnum).add(oap.end.col as usize);
            if !crate::memory::cstr_is_empty(ptr_line) {
                ptr_line = ptr_line.add(usize::from(oap.inclusive));
            }
            ptr_line = skipwhite(ptr_line);
            if crate::memory::cstr_is_empty(ptr_line) && inindent(0) {
                oap.motion_type = kMTLineWise;
            }
        }

        let mut only_setmarks = false;
        if oap.motion_type != kMTLineWise
            && oap.line_count == 1
            && oap.op_type == OP_DELETE
            && *ml_get(oap.start.lnum) == NUL
        {
            if g_st().virtual_op != 0 {
                only_setmarks = true;
            } else {
                if !vim_strchr(opt_vars().p_cpo, kCpoEmptyregion).is_null() {
                    beep_flush();
                }
                return true;
            }
        }

        if !only_setmarks {
            if oap.regname != b'_' as c_int {
                let mut reg: *mut Yankreg = ptr::null_mut();
                let mut did_yank = false;
                if oap.regname != 0 {
                    if !valid_yank_reg(oap.regname, true) {
                        beep_flush();
                        return true;
                    }
                    reg = get_yank_register(oap.regname, YREG_YANK);
                    op_yank_reg(oap, false, reg, is_append_register(oap.regname));
                    did_yank = true;
                }

                if oap.motion_type == kMTLineWise || oap.line_count > 1 || oap.use_reg_one
                {
                    shift_delete_registers(is_append_register(oap.regname));
                    reg = get_y_register(1);
                    op_yank_reg(oap, false, reg, false);
                    did_yank = true;
                }

                if oap.regname == 0
                    && oap.motion_type != kMTLineWise
                    && oap.line_count == 1
                {
                    reg = get_yank_register(b'-' as c_int, YREG_YANK);
                    op_yank_reg(oap, false, reg, false);
                    did_yank = true;
                }

                if did_yank || oap.regname == 0 {
                    if reg.is_null() {
                        std::process::abort();
                    }
                    set_clipboard(oap.regname, &*reg);
                    do_autocmd_textyankpost(oap, &mut *reg);
                }
            }

            if oap.motion_type == kMTBlockWise {
                if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
                    return false;
                }

                for lnum in curwin_mut().w_cursor.lnum..=oap.end.lnum {
                    block_prep(oap, &mut bd, lnum, true);
                    if bd.textlen == 0 {
                        continue;
                    }

                    if lnum == curwin_mut().w_cursor.lnum {
                        curwin_mut().w_cursor.col = bd.textcol + bd.startspaces;
                        curwin_mut().w_cursor.coladd = 0;
                    }

                    let n = bd.textlen - bd.startspaces - bd.endspaces;
                    let oldp = ml_get(lnum);
                    let mut newp_vec = vec![0 as c_char; ((ml_get_len(lnum) - n) as usize) + 1];
                    let newp: *mut c_char = newp_vec.as_mut_ptr();
                    ptr::copy(oldp, newp, bd.textcol as usize);
                    ptr::write_bytes(
                        newp.add(bd.textcol as usize),
                        b' ',
                        (bd.startspaces + bd.endspaces) as usize,
                    );
                    crate::memory::cstr_cpy(newp.add((bd.textcol + bd.startspaces + bd.endspaces) as usize), oldp.add((bd.textcol + bd.textlen) as usize));
                    ml_replace(lnum, newp, true);
                    drop(newp_vec);

                    extmark_splice_cols(
                        curbuf_mut(),
                        lnum - 1,
                        bd.textcol,
                        bd.textlen,
                        bd.startspaces + bd.endspaces,
                        kExtmarkUndo,
                    );
                }

                check_cursor_col(curwin_mut());
                changed_lines(
                    curbuf_mut(),
                    curwin_mut().w_cursor.lnum,
                    curwin_mut().w_cursor.col,
                    oap.end.lnum + 1,
                    0,
                    true,
                );
                oap.line_count = 0;
            } else if oap.motion_type == kMTLineWise {
                if oap.op_type == OP_CHANGE {
                    if oap.line_count > 1 {
                        let lnum = curwin_mut().w_cursor.lnum;
                        curwin_mut().w_cursor.lnum += 1;
                        del_lines(oap.line_count - 1, true);
                        curwin_mut().w_cursor.lnum = lnum;
                    }
                    if !u_save_cursor() {
                        return false;
                    }
                    if curbuf_mut().b_p_ai != 0 {
                        beginline(BL_WHITE);
                        g_st().Ins.did_ai = true;
                        g_st().Ins.ai_col = curwin_mut().w_cursor.col;
                    } else {
                        beginline(0);
                    }
                    truncate_line(0);
                    if oap.line_count > 1 {
                        u_clearline(curbuf_mut());
                    }
                } else {
                    del_lines(oap.line_count, true);
                    beginline(BL_WHITE | BL_FIX);
                    u_clearline(curbuf_mut());
                }
            } else {
                if g_st().virtual_op != 0 {
                    if gchar_pos(&mut oap.start) == TAB {
                        let mut endcol: c_int = 0;
                        if !u_save_cursor() {
                            return false;
                        }
                        if oap.line_count == 1 {
                            endcol = getviscol2(oap.end.col, oap.end.coladd);
                        }
                        coladvance_force(getviscol2(oap.start.col, oap.start.coladd));
                        oap.start = curwin_mut().w_cursor;
                        if oap.line_count == 1 {
                            coladvance(curwin_mut(), endcol);
                            oap.end.col = curwin_mut().w_cursor.col;
                            oap.end.coladd = curwin_mut().w_cursor.coladd;
                            curwin_mut().w_cursor = oap.start;
                        }
                    }

                    if gchar_pos(&mut oap.end) == TAB
                        && oap.end.coladd == 0
                        && oap.inclusive
                    {
                        if !u_save(oap.end.lnum - 1, oap.end.lnum + 1) {
                            return false;
                        }
                        curwin_mut().w_cursor = oap.end;
                        coladvance_force(getviscol2(oap.end.col, oap.end.coladd));
                        oap.end = curwin_mut().w_cursor;
                        curwin_mut().w_cursor = oap.start;
                    }
                    mb_adjust_opend(oap);
                }

                if oap.line_count == 1 {
                    if !u_save_cursor() {
                        return false;
                    }

                    if !vim_strchr(opt_vars().p_cpo, kCpoDollar).is_null()
                        && oap.op_type == OP_CHANGE
                        && oap.end.lnum == curwin_mut().w_cursor.lnum
                        && !oap.is_VIsual
                    {
                        display_dollar(oap.end.col - c_int::from(!oap.inclusive));
                    }

                    let mut n =
                        oap.end.col - oap.start.col + 1 - c_int::from(!oap.inclusive);

                    if g_st().virtual_op != 0 {
                        let len = get_cursor_line_len();

                        if oap.end.coladd != 0
                            && oap.end.col >= len - 1
                            && !(oap.start.coladd != 0 && oap.end.col >= len - 1)
                        {
                            n += 1;
                        }
                        if n == 0 && oap.start.coladd != oap.end.coladd {
                            n = 1;
                        }

                        if gchar_cursor() != c_int::from(NUL) {
                            curwin_mut().w_cursor.coladd = 0;
                        }
                    }

                    del_bytes(
                        n,
                        g_st().virtual_op == 0,
                        oap.op_type == OP_DELETE && !oap.is_VIsual,
                    );
                } else {
                    if !u_save(
                        curwin_mut().w_cursor.lnum - 1,
                        curwin_mut().w_cursor.lnum + oap.line_count,
                    )
                    {
                        return false;
                    }

                    g_st().curbuf_splice_pending += 1;
                    let startpos = curwin_mut().w_cursor;
                    let deleted_bytes = get_region_bytecount(
                        curbuf_mut(),
                        startpos.lnum,
                        oap.end.lnum,
                        startpos.col,
                        oap.end.col,
                    ) + isize::from(oap.inclusive);
                    truncate_line(1);

                    let curpos = curwin_mut().w_cursor;
                    curwin_mut().w_cursor.lnum += 1;

                    del_lines(oap.line_count - 2, false);

                    let n = oap.end.col + 1 - c_int::from(!oap.inclusive);
                    curwin_mut().w_cursor.col = 0;
                    del_bytes(
                        n,
                        g_st().virtual_op == 0,
                        oap.op_type == OP_DELETE && !oap.is_VIsual,
                    );
                    curwin_mut().w_cursor = curpos;
                    do_join(2, false, false, false, false);
                    g_st().curbuf_splice_pending -= 1;
                    extmark_splice(
                        curbuf_mut(),
                        startpos.lnum - 1,
                        startpos.col,
                        oap.line_count - 1,
                        n,
                        deleted_bytes,
                        0,
                        0,
                        0,
                        kExtmarkUndo,
                    );
                }
                if oap.op_type == OP_DELETE {
                    auto_format(false, true);
                }
            }

            msgmore(curbuf_mut().b_ml.ml_line_count - old_lcount);
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            if oap.motion_type == kMTBlockWise {
                curbuf_mut().b_op_end.lnum = oap.end.lnum;
                curbuf_mut().b_op_end.col = oap.start.col;
            } else {
                curbuf_mut().b_op_end = oap.start;
            }
            curbuf_mut().b_op_start = oap.start;
        }

        true
    }
}

fn mb_adjust_opend(oap: &mut Oparg) {
    unsafe {
        if !oap.inclusive {
            return;
        }

        let line = ml_get(oap.end.lnum);
        let mut ptr_line = line.add(oap.end.col as usize);
        if !crate::memory::cstr_is_empty(ptr_line) {
            ptr_line = ptr_line.sub(utf_head_off(line, ptr_line) as usize);
            ptr_line = ptr_line.add((utfc_ptr2len(ptr_line) - 1) as usize);
            oap.end.col = ptr_line.offset_from(line) as Colnr;
        }
    }
}

fn pbyte(mut lp: CursorPos, c: c_int) {
    unsafe {
        debug_assert!(c <= u8::MAX as c_int);
        let p = ml_get_buf_mut(curbuf_mut(), lp.lnum);
        let len = curbuf_mut().b_ml.ml_line_textlen;

        if lp.col >= len {
            lp.col = if len > 1 { len - 2 } else { 0 };
        }
        *p.add(lp.col as usize) = c as c_char;
        if g_st().curbuf_splice_pending == 0 {
            extmark_splice_cols(curbuf_mut(), lp.lnum - 1, lp.col, 1, 1, kExtmarkUndo);
        }
    }
}

fn replace_character(c: c_int) {
    let n = g_st().State;

        g_st().State = MODE_REPLACE;
        ins_char(c);
        g_st().State = n;
        dec_cursor();
}

fn op_replace(oap: &mut Oparg, mut c: c_int) -> bool {
    unsafe {
        let mut n: c_int;
        let mut bd = BlockDef::new();
        let mut after_p: Vec<c_char> = Vec::new();
        let mut had_ctrl_v_cr = false;

        if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 || oap.empty {
            return true;
        }
        if c == REPLACE_CR_NCHAR {
            had_ctrl_v_cr = true;
            c = CAR;
        } else if c == REPLACE_NL_NCHAR {
            had_ctrl_v_cr = true;
            c = NL;
        }

        mb_adjust_opend(oap);

        if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
            return false;
        }

        if oap.motion_type == kMTBlockWise {
            bd.is_MAX = c_int::from(curwin_mut().w_curswant == MAXCOL);
            while curwin_mut().w_cursor.lnum <= oap.end.lnum {
                curwin_mut().w_cursor.col = 0;
                block_prep(oap, &mut bd, curwin_mut().w_cursor.lnum, true);
                if bd.textlen == 0 && (g_st().virtual_op == 0 || bd.is_MAX != 0) {
                    curwin_mut().w_cursor.lnum += 1;
                    continue;
                }

                if g_st().virtual_op != 0 && bd.is_short != 0 && crate::memory::cstr_is_empty(bd.textstart) {
                    let mut vpos = CursorPos {
                        lnum: curwin_mut().w_cursor.lnum,
                        col: 0,
                        coladd: 0,
                    };
                    getvpos(curwin_ptr(), &mut vpos, oap.start_vcol);
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
                let mut numc = oap.end_vcol - oap.start_vcol + 1;
                if bd.is_short != 0 && (g_st().virtual_op == 0 || bd.is_MAX != 0) {
                    numc -= (oap.end_vcol - bd.end_vcol) + 1;
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
                let mut newp_vec = vec![0 as c_char; newp_size + 1];
                let newp: *mut c_char = newp_vec.as_mut_ptr();
                ptr::copy(oldp, newp, bd.textcol as usize);
                oldp = oldp.add((bd.textcol + bd.textlen) as usize);
                ptr::write_bytes(newp.add(bd.textcol as usize), b' ', bd.startspaces as usize);
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
                    after_p.clear();
                    after_p.extend_from_slice(std::slice::from_raw_parts(oldp, col as usize));
                    newrows = 1;
                }
                ml_replace(curwin_mut().w_cursor.lnum, newp, true);
                drop(newp_vec);
                g_st().curbuf_splice_pending += 1;
                let baselnum = curwin_mut().w_cursor.lnum;
                if !after_p.is_empty() {
                    let cur_lnum = curwin_mut().w_cursor.lnum;
                    curwin_mut().w_cursor.lnum += 1;
                    ml_append(
                        cur_lnum,
                        after_p.as_mut_ptr(),
                        after_p.len() as c_int,
                        false,
                    );
                    appended_lines_mark(curwin_mut().w_cursor.lnum, 1);
                    oap.end.lnum += 1;
                    after_p.clear();
                }
                g_st().curbuf_splice_pending -= 1;
                extmark_splice(
                    curbuf_mut(),
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
                curwin_mut().w_cursor.lnum += 1;
            }
        } else {
            if oap.motion_type == kMTLineWise {
                oap.start.col = 0;
                curwin_mut().w_cursor.col = 0;
                oap.end.col = ml_get_len(oap.end.lnum);
                if oap.end.col != 0 {
                    oap.end.col -= 1;
                }
            } else if !oap.inclusive {
                dec(&mut oap.end);
            }

            while ltoreq(curwin_mut().w_cursor, oap.end) {
                let mut done = false;

                n = gchar_cursor();
                if n != c_int::from(NUL) {
                    let new_byte_len = utf_char2len(c);
                    let old_byte_len = utfc_ptr2len(get_cursor_pos_ptr());

                    if new_byte_len > 1 || old_byte_len > 1 {
                        if curwin_mut().w_cursor.lnum == oap.end.lnum {
                            oap.end.col += new_byte_len - old_byte_len;
                        }
                        replace_character(c);
                        done = true;
                    } else {
                        if n == TAB {
                            let mut end_vcol: c_int = 0;

                            if curwin_mut().w_cursor.lnum == oap.end.lnum {
                                end_vcol = getviscol2(oap.end.col, oap.end.coladd);
                            }
                            coladvance_force(getviscol());
                            if curwin_mut().w_cursor.lnum == oap.end.lnum {
                                getvpos(curwin_ptr(), &mut oap.end, end_vcol);
                            }
                        }
                        if gchar_cursor() != c_int::from(NUL) {
                            pbyte(curwin_mut().w_cursor, c);
                            done = true;
                        }
                    }
                }
                if !done && g_st().virtual_op != 0 && curwin_mut().w_cursor.lnum == oap.end.lnum {
                    let mut virtcols = oap.end.coladd;

                    if curwin_mut().w_cursor.lnum == oap.start.lnum
                        && oap.start.col == oap.end.col
                        && oap.start.coladd != 0
                    {
                        virtcols -= oap.start.coladd;
                    }

                    coladvance_force(getviscol2(oap.end.col, oap.end.coladd) + 1);
                    curwin_mut().w_cursor.col -= virtcols + 1;
                    while virtcols >= 0 {
                        if utf_char2len(c) > 1 {
                            replace_character(c);
                        } else {
                            pbyte(curwin_mut().w_cursor, c);
                        }
                        if inc(&mut curwin_mut().w_cursor) == -1 {
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

        curwin_mut().w_cursor = oap.start;
        check_cursor(curwin_mut());
        changed_lines(
            curbuf_mut(),
            oap.start.lnum,
            oap.start.col,
            oap.end.lnum + 1,
            0,
            true,
        );

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end = oap.end;
        }

        true
    }
}

/// Handle the (non-standard vi) tilde operator. Also for "gu", "gU" and "g?".
pub fn op_tilde(oap: &mut Oparg) {
     {
        let mut bd = BlockDef::new();
        let mut did_change = false;

        if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
            return;
        }

        let mut pos = oap.start;
        if oap.motion_type == kMTBlockWise {
            while pos.lnum <= oap.end.lnum {
                block_prep(oap, &mut bd, pos.lnum, false);
                pos.col = bd.textcol;
                let one_change = swapchars(oap.op_type, &mut pos, bd.textlen) != 0;
                did_change |= one_change;
                pos.lnum += 1;
            }
            if did_change {
                changed_lines(curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
            }
        } else {
            if oap.motion_type == kMTLineWise {
                oap.start.col = 0;
                pos.col = 0;
                oap.end.col = ml_get_len(oap.end.lnum);
                if oap.end.col != 0 {
                    oap.end.col -= 1;
                }
            } else if !oap.inclusive {
                dec(&mut oap.end);
            }

            if pos.lnum == oap.end.lnum {
                let len = oap.end.col - pos.col + 1;
                did_change = swapchars(oap.op_type, &mut pos, len) != 0;
            } else {
                loop {
                    let len = if pos.lnum == oap.end.lnum {
                        oap.end.col + 1
                    } else {
                        ml_get_pos_len(&mut pos)
                    };
                    did_change |= swapchars(oap.op_type, &mut pos, len) != 0;
                    if ltoreq(oap.end, pos) || inc(&mut pos) == -1 {
                        break;
                    }
                }
            }
            if did_change {
                changed_lines(
                    curbuf_mut(),
                    oap.start.lnum,
                    oap.start.col,
                    oap.end.lnum + 1,
                    0,
                    true,
                );
            }
        }

        if !did_change && oap.is_VIsual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end = oap.end;
        }

        if i64::from(oap.line_count) > opt_vars().p_report {
            smsg!(
                0,
                ngettext(
                    c"%lld line changed",
                    c"%lld lines changed",
                    oap.line_count as c_ulong,
                ),
                i64::from(oap.line_count) as std::ffi::c_longlong,
            );
        }
    }
}

fn swapchars(op_type: c_int, pos: &mut CursorPos, length: c_int) -> c_int {
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

pub fn swapchar(op_type: c_int, pos: &mut CursorPos) -> bool {
     {
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
                let sp = curwin_mut().w_cursor;

                curwin_mut().w_cursor = *pos;
                del_bytes(utf_ptr2len(get_cursor_pos_ptr()), false, false);
                ins_char(nc);
                curwin_mut().w_cursor = sp;
            } else {
                pbyte(*pos, nc);
            }
            return true;
        }
        false
    }
}

/// Insert and append operators for Visual mode.
pub fn op_insert(oap: &mut Oparg, count1: c_int) {
    unsafe {
        let mut pre_textlen: c_int = 0;
        let mut ind_pre_col: Colnr = 0;
        let mut ind_pre_vcol: c_int = 0;
        let mut bd = BlockDef::new();

        bd.is_MAX = c_int::from(curwin_mut().w_curswant == MAXCOL);

        curwin_mut().w_cursor.lnum = oap.start.lnum;
        redraw_curbuf_later(UPD_INVERTED);
        update_screen();

        if oap.motion_type == kMTBlockWise {
            if curwin_mut().w_cursor.coladd > 0 {
                let old_ve_flags = curwin_mut().w_onebuf_opt.wo_ve_flags;

                if !u_save_cursor() {
                    return;
                }
                curwin_mut().w_onebuf_opt.wo_ve_flags = kOptVeFlagAll;
                coladvance_force(if oap.op_type == OP_APPEND {
                    oap.end_vcol + 1
                } else {
                    getviscol()
                });
                if oap.op_type == OP_APPEND {
                    curwin_mut().w_cursor.col -= 1;
                }
                curwin_mut().w_onebuf_opt.wo_ve_flags = old_ve_flags;
            }
            block_prep(oap, &mut bd, oap.start.lnum, true);
            ind_pre_col = getwhitecols_curline() as Colnr;
            ind_pre_vcol = get_indent();
            pre_textlen = ml_get_len(oap.start.lnum) - bd.textcol;
            if oap.op_type == OP_APPEND {
                pre_textlen -= bd.textlen;
            }
        }

        if oap.op_type == OP_APPEND {
            if oap.motion_type == kMTBlockWise && curwin_mut().w_cursor.coladd == 0 {
                curwin_mut().w_set_curswant = true;
                while !crate::memory::cstr_is_empty(get_cursor_pos_ptr())
                    && curwin_mut().w_cursor.col < bd.textcol + bd.textlen
                {
                    curwin_mut().w_cursor.col += 1;
                }
                if bd.is_short != 0 && bd.is_MAX == 0 {
                    if !u_save_cursor() {
                        return;
                    }
                    for _ in 0..bd.endspaces {
                        ins_char(b' ' as c_int);
                    }
                    bd.textlen += bd.endspaces;
                }
            } else {
                curwin_mut().w_cursor = oap.end;
                check_cursor_col(curwin_mut());

                if !lineempty(curwin_mut().w_cursor.lnum) && oap.start_vcol != oap.end_vcol {
                    inc_cursor();
                }
            }
        }

        let t1 = oap.start;
        let start_insert = curwin_mut().w_cursor;
        edit(c_int::from(NUL), false, count1);

        if t1.lnum == curbuf_mut().b_op_start_orig.lnum && lt(curbuf_mut().b_op_start_orig, t1) {
            oap.start = curbuf_mut().b_op_start_orig;
        }

        if curwin_mut().w_cursor.lnum != oap.start.lnum || g_st().got_int {
            return;
        }

        if oap.motion_type == kMTBlockWise {
            let mut ind_post_vcol: c_int = 0;
            let mut bd2 = BlockDef::new();
            let mut did_indent = false;

            let ind_post_col = getwhitecols_curline() as Colnr;
            if curbuf_mut().b_op_start.col > ind_pre_col && ind_post_col > ind_pre_col {
                bd.textcol += ind_post_col - ind_pre_col;
                ind_post_vcol = get_indent();
                bd.start_vcol += ind_post_vcol - ind_pre_vcol;
                did_indent = true;
            }

            if oap.start.lnum == curbuf_mut().b_op_start_orig.lnum && bd.is_MAX == 0 && !did_indent
            {
                let t = getviscol2(
                    curbuf_mut().b_op_start_orig.col,
                    curbuf_mut().b_op_start_orig.coladd,
                );

                if oap.op_type == OP_INSERT
                    && oap.start.col + oap.start.coladd
                        != curbuf_mut().b_op_start_orig.col + curbuf_mut().b_op_start_orig.coladd
                {
                    oap.start.col = curbuf_mut().b_op_start_orig.col;
                    pre_textlen -= t - oap.start_vcol;
                    oap.start_vcol = t;
                } else if oap.op_type == OP_APPEND
                    && oap.start.col + oap.start.coladd
                        >= curbuf_mut().b_op_start_orig.col + curbuf_mut().b_op_start_orig.coladd
                {
                    oap.start.col = curbuf_mut().b_op_start_orig.col;
                    pre_textlen += bd.textlen;
                    pre_textlen -= t - oap.start_vcol;
                    oap.start_vcol = t;
                    oap.op_type = OP_INSERT;
                }
            }

            if did_indent && bd.textcol - ind_post_col > 0 {
                oap.start.col += ind_post_col - ind_pre_col;
                oap.start_vcol += ind_post_vcol - ind_pre_vcol;
                oap.end.col += ind_post_col - ind_pre_col;
                oap.end_vcol += ind_post_vcol - ind_pre_vcol;
            }
            block_prep(oap, &mut bd2, oap.start.lnum, true);
            if did_indent && bd.textcol - ind_post_col > 0 {
                oap.start.col -= ind_post_col - ind_pre_col;
                oap.start_vcol -= ind_post_vcol - ind_pre_vcol;
                oap.end.col -= ind_post_col - ind_pre_col;
                oap.end_vcol -= ind_post_vcol - ind_pre_vcol;
            }
            if bd.is_MAX == 0 || bd2.textlen < bd.textlen {
                if oap.op_type == OP_APPEND {
                    pre_textlen += bd2.textlen - bd.textlen;
                    if bd2.endspaces != 0 {
                        bd2.textlen -= 1;
                    }
                }
                bd.textcol = bd2.textcol;
                bd.textlen = bd2.textlen;
            }

            let mut firstline = ml_get(oap.start.lnum);
            let mut len = ml_get_len(oap.start.lnum);
            let mut add = bd.textcol;
            let mut offset: Colnr = 0;
            if oap.op_type == OP_APPEND {
                add += bd.textlen;
                if bd.is_MAX != 0
                    && start_insert.lnum == g_st().Ins.start.lnum
                    && start_insert.col > g_st().Ins.start.col
                {
                    offset = start_insert.col - g_st().Ins.start.col;
                    add -= offset;
                    if oap.end_vcol > offset {
                        oap.end_vcol -= offset + 1;
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
                let mut ins_text = Vec::with_capacity(ins_len as usize + 1);
                ins_text.extend_from_slice(std::slice::from_raw_parts(firstline, ins_len as usize));
                ins_text.push(NUL);
                if u_save(oap.start.lnum, oap.end.lnum + 1) {
                    block_insert(
                        oap,
                        ins_text.as_ptr(),
                        ins_len as usize,
                        oap.op_type == OP_INSERT,
                        &mut bd,
                    );
                }

                curwin_mut().w_cursor.col = oap.start.col;
                check_cursor(curwin_mut());
            }
        }
    }
}

/// Handle a change operation.
pub fn op_change(oap: &mut Oparg) -> c_int {
    unsafe {
        let mut pre_textlen: c_int = 0;
        let mut pre_indent: c_int = 0;
        let mut firstline: *mut c_char;
        let mut bd = BlockDef::new();

        let mut l = oap.start.col;
        if oap.motion_type == kMTLineWise {
            l = 0;
            g_st().Ins.can_si = may_do_si();
        }

        if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
            if !u_save_cursor() {
                return 0;
            }
        } else if !op_delete(oap) {
            return 0;
        }

        if l > curwin_mut().w_cursor.col && !lineempty(curwin_mut().w_cursor.lnum) && g_st().virtual_op == 0 {
            inc_cursor();
        }

        if oap.motion_type == kMTBlockWise {
            if g_st().virtual_op != 0
                && (curwin_mut().w_cursor.coladd > 0 || gchar_cursor() == c_int::from(NUL))
            {
                coladvance_force(getviscol());
            }
            firstline = ml_get(oap.start.lnum);
            pre_textlen = ml_get_len(oap.start.lnum);
            pre_indent = getwhitecols(firstline) as c_int;
            bd.textcol = curwin_mut().w_cursor.col;
        }

        if oap.motion_type == kMTLineWise {
            fix_indent();
        }

        let save_finish_op = g_st().finish_op;
        g_st().finish_op = false;

        let retval = c_int::from(edit(c_int::from(NUL), false, 1));

        g_st().finish_op = save_finish_op;

        if oap.motion_type == kMTBlockWise && oap.start.lnum != oap.end.lnum && !g_st().got_int {
            firstline = ml_get(oap.start.lnum);
            if bd.textcol > pre_indent {
                let new_indent = getwhitecols(firstline) as c_int;
                pre_textlen += new_indent - pre_indent;
                bd.textcol += new_indent - pre_indent;
            }

            let ins_len = ml_get_len(oap.start.lnum) - pre_textlen;
            if ins_len > 0 {
                let mut ins_text = vec![0 as c_char; (ins_len as usize) + 1];
                crate::memory::cstr_ncpy(ins_text.as_mut_ptr(), firstline.add(bd.textcol as usize), ins_len as usize);
                for linenr in (oap.start.lnum + 1)..=oap.end.lnum {
                    block_prep(oap, &mut bd, linenr, true);
                    if bd.is_short == 0 || g_st().virtual_op != 0 {
                        let mut vpos = CursorPos {
                            lnum: linenr,
                            col: 0,
                            coladd: 0,
                        };
                        if bd.is_short != 0 {
                            getvpos(curwin_ptr(), &mut vpos, oap.start_vcol);
                        } else {
                            vpos.coladd = 0;
                        }
                        let oldp = ml_get(linenr);
                        let mut newp_vec = vec![
                            0 as c_char;
                            (ml_get_len(linenr) as usize)
                                + (vpos.coladd as usize)
                                + (ins_len as usize)
                                + 1
                        ];
                        let newp: *mut c_char = newp_vec.as_mut_ptr();
                        ptr::copy(oldp, newp, bd.textcol as usize);
                        let mut newlen = bd.textcol;
                        ptr::write_bytes(newp.add(newlen as usize), b' ', vpos.coladd as usize);
                        newlen += vpos.coladd;
                        ptr::copy(
                            ins_text.as_ptr(),
                            newp.add(newlen as usize),
                            ins_len as usize,
                        );
                        newlen += ins_len;
                        crate::memory::cstr_cpy(newp.add(newlen as usize), oldp.add(bd.textcol as usize));
                        ml_replace(linenr, newp, true);
                        drop(newp_vec);
                        extmark_splice_cols(
                            curbuf_mut(),
                            linenr - 1,
                            bd.textcol,
                            0,
                            vpos.coladd + ins_len,
                            kExtmarkUndo,
                        );
                    }
                }
                check_cursor(curwin_mut());
                changed_lines(
                    curbuf_mut(),
                    oap.start.lnum + 1,
                    0,
                    oap.end.lnum + 1,
                    0,
                    true,
                );
            }
        }
        auto_format(false, true);

        retval
    }
}

/// When the cursor is on the NUL past the end of the line and it should not be
/// there move it left.
pub fn adjust_cursor_eol() {
     {
        let cur_ve_flags = get_ve_flags(curwin_mut());

        let adj_cursor = curwin_mut().w_cursor.col > 0
            && gchar_cursor() == c_int::from(NUL)
            && (cur_ve_flags & kOptVeFlagOnemore) == 0
            && (cur_ve_flags & kOptVeFlagAll) == 0
            && !(g_st().restart_edit != 0 || (g_st().State & MODE_INSERT) != 0);
        if !adj_cursor {
            return;
        }

        dec_cursor();

        if cur_ve_flags == kOptVeFlagAll {
            let mut scol: Colnr = 0;
            let mut ecol: Colnr = 0;

            getvcol(
                curwin_mut(),
                &mut curwin_mut().w_cursor,
                Some(&mut scol),
                None,
                Some(&mut ecol),
                0,
            );
            curwin_mut().w_cursor.coladd = ecol - scol + 1;
        }
    }
}

pub fn skip_comment(
    mut line: *mut c_char,
    process: bool,
    include_space: bool,
    is_comment: &mut bool,
) -> *mut c_char {
    unsafe {
        let mut comment_flags: *mut c_char = ptr::null_mut();
        let leader_offset = get_last_leader_offset(line, &raw mut comment_flags);

        *is_comment = false;
        if leader_offset != -1 {
            while !crate::memory::cstr_is_empty(comment_flags) {
                if *comment_flags == COM_END || crate::memory::cstr_first_byte(comment_flags) == b':' {
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

        while !crate::memory::cstr_is_empty(comment_flags) {
            if *comment_flags == COM_END || crate::memory::cstr_first_byte(comment_flags) == b':' {
                break;
            }
            comment_flags = comment_flags.add(1);
        }

        if crate::memory::cstr_first_byte(comment_flags) == b':' || crate::memory::cstr_is_empty(comment_flags) {
            line = line.add(lead_len as usize);
        }

        line
    }
}

pub fn do_join(
    count: usize,
    insert_space: bool,
    save_undo: bool,
    use_formatoptions: bool,
    setmark: bool,
) -> bool {
    unsafe {
        let mut curr: *mut c_char = ptr::null_mut();
        let mut curr_start: *mut c_char = ptr::null_mut();
        let mut cend: *mut c_char;
        let mut endcurr1: c_int = c_int::from(NUL);
        let mut endcurr2: c_int = c_int::from(NUL);
        let mut currsize: c_int = 0;
        let mut sumsize: c_int = 0;
        let mut ret = true;
        let remove_comments = use_formatoptions && has_format_option(kFoRemoveComs);
        let mut prev_was_comment = false;
        debug_assert!(count >= 1);

        if save_undo
            && !u_save(
                curwin_mut().w_cursor.lnum - 1,
                curwin_mut().w_cursor.lnum + (count as Linenr),
            )
        {
            return false;
        }
        let mut spaces_vec = vec![0 as c_char; count];
        let spaces: *mut c_char = spaces_vec.as_mut_ptr();
        let mut comments_vec = if remove_comments {
            vec![0 as c_int; count]
        } else {
            Vec::new()
        };
        let comments: *mut c_int = comments_vec.as_mut_ptr();

        let mut aborted = false;
        for t in 0..(count as Linenr) {
            curr_start = ml_get(curwin_mut().w_cursor.lnum + t);
            curr = curr_start;
            if t == 0 && setmark && (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                curwin_mut().buf_mut().b_op_start.lnum = curwin_mut().w_cursor.lnum;
                curwin_mut().buf_mut().b_op_start.col =
                    crate::memory::cstr_len(curr) as Colnr;
            }
            if remove_comments {
                if t > 0 && prev_was_comment {
                    let new_curr =
                        skip_comment(curr, true, insert_space, &mut prev_was_comment);
                    *comments.add(t as usize) = new_curr.offset_from(curr) as c_int;
                    curr = new_curr;
                } else {
                    curr = skip_comment(curr, false, insert_space, &mut prev_was_comment);
                }
            }

            if insert_space && t > 0 {
                curr = skipwhite(curr);
                if !crate::memory::cstr_is_empty(curr)
                    && crate::memory::cstr_first_byte(curr) != b')'
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
                    if opt_vars().p_js != 0
                        && (endcurr1 == b'.' as c_int
                            || endcurr1 == b'?' as c_int
                            || endcurr1 == b'!' as c_int)
                    {
                        *spaces.add(t as usize) += 1;
                    }
                }
            }

            let space_t = crate::memory::cstr_char_at(spaces, t as usize);
            if t > 0 && g_st().curbuf_splice_pending == 0 {
                let removed = curr.offset_from(curr_start) as Colnr;
                extmark_splice(
                    curbuf_mut(),
                    curwin_mut().w_cursor.lnum - 1,
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
            currsize = crate::memory::cstr_len(curr) as c_int;
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
            if g_st().got_int {
                ret = false;
                aborted = true;
                break;
            }
        }

        if !aborted {
            let col = sumsize - currsize - crate::memory::cstr_char_at(spaces, count - 1);

            let newp_len = sumsize as usize;
            let mut newp_vec = vec![0 as c_char; newp_len + 1];
            let newp: *mut c_char = newp_vec.as_mut_ptr();
            cend = newp.add(sumsize as usize);

            g_st().curbuf_splice_pending += 1;

            let mut t = (count as Linenr) - 1;
            loop {
                cend = cend.sub(currsize as usize);
                ptr::copy(curr, cend, currsize as usize);

                let space_t = crate::memory::cstr_char_at(spaces, t as usize);
                if space_t > 0 {
                    cend = cend.sub(space_t as usize);
                    ptr::write_bytes(cend, b' ', space_t as usize);
                }

                let spaces_removed = (curr.offset_from(curr_start) as c_int) - space_t;
                let lnum = curwin_mut().w_cursor.lnum + t;
                let mincol: Colnr = 0;
                let lnum_amount = -t;
                let col_amount = (cend.offset_from(newp) as Colnr) - spaces_removed;

                mark_col_adjust(lnum, mincol, lnum_amount, col_amount, spaces_removed);

                if t == 0 {
                    break;
                }

                curr_start = ml_get(curwin_mut().w_cursor.lnum + t - 1);
                curr = curr_start;
                if remove_comments {
                    curr = curr.add(*comments.add((t - 1) as usize) as usize);
                }
                if insert_space && t > 1 {
                    curr = skipwhite(curr);
                }
                currsize = crate::memory::cstr_len(curr) as c_int;
                t -= 1;
            }

            ml_replace_len(curwin_mut().w_cursor.lnum, newp, newp_len, true);
            drop(newp_vec);

            if setmark && (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                curwin_mut().buf_mut().b_op_end.lnum = curwin_mut().w_cursor.lnum;
                curwin_mut().buf_mut().b_op_end.col = sumsize;
            }

            changed_lines(
                curbuf_mut(),
                curwin_mut().w_cursor.lnum,
                currsize,
                curwin_mut().w_cursor.lnum + 1,
                0,
                true,
            );

            let saved_lnum = curwin_mut().w_cursor.lnum;
            curwin_mut().w_cursor.lnum += 1;
            del_lines((count as c_int) - 1, false);
            curwin_mut().w_cursor.lnum = saved_lnum;
            g_st().curbuf_splice_pending -= 1;
            curbuf_mut().deleted_bytes2 = 0;

            curwin_mut().w_cursor.col = if !vim_strchr(opt_vars().p_cpo, kCpoJoincol).is_null() {
                currsize
            } else {
                col
            };
            check_cursor_col(curwin_mut());

            curwin_mut().w_cursor.coladd = 0;
            curwin_mut().w_set_curswant = true;
        }

        ret
    }
}

/// Reset 'linebreak' and take care of side effects.
pub fn reset_lbr() -> bool {
     {
        if curwin_mut().w_onebuf_opt.wo_lbr == 0 {
            return false;
        }
        curwin_mut().w_onebuf_opt.wo_lbr = 0;
        curwin_mut().w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
        true
    }
}

/// Restore 'linebreak' and take care of side effects.
pub fn restore_lbr(lbr_saved: bool) {
     {
        if curwin_mut().w_onebuf_opt.wo_lbr != 0 || !lbr_saved {
            return;
        }

        curwin_mut().w_onebuf_opt.wo_lbr = 1;
        curwin_mut().w_valid &= !(VALID_WROW | VALID_WCOL | VALID_VIRTCOL);
    }
}

/// Prepare a few things for block mode yank/delete/tilde.
pub fn block_prep(oap: &mut Oparg, bdp: &mut BlockDef, lnum: Linenr, is_del: bool) {
    unsafe {
        let mut incr: c_int = 0;
        let lbr_saved = reset_lbr();

        bdp.startspaces = 0;
        bdp.endspaces = 0;
        bdp.textlen = 0;
        bdp.start_vcol = 0;
        bdp.end_vcol = 0;
        bdp.is_short = 0;
        bdp.is_oneChar = 0;
        bdp.pre_whitesp = 0;
        bdp.pre_whitesp_c = 0;
        bdp.end_char_vcols = 0;
        bdp.start_char_vcols = 0;

        let line = ml_get(lnum);
        let mut prev_pstart = line;

        let mut csarg = CharsizeArg::new();
        let mut cstype = init_charsize_arg(&mut csarg, curwin_mut(), lnum, line);
        let mut ci = utf_ptr2StrCharInfo(line);
        let mut vcol = bdp.start_vcol;
        while vcol < oap.start_vcol && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_ClusterInfo(ci);
            incr = win_charsize(
                cstype,
                vcol,
                ci.ptr,
                ci.chr.value,
                &mut csarg,
                cli.cells,
            )
            .width;
            vcol += incr;
            if ascii_iswhite(ci.chr.value) {
                bdp.pre_whitesp += incr;
                bdp.pre_whitesp_c += 1;
            } else {
                bdp.pre_whitesp = 0;
                bdp.pre_whitesp_c = 0;
            }
            prev_pstart = ci.ptr.cast_mut();
            ci = cli.next;
        }
        bdp.start_vcol = vcol;
        let mut pstart = ci.ptr.cast_mut();

        bdp.start_char_vcols = incr;
        if bdp.start_vcol < oap.start_vcol {
            bdp.end_vcol = bdp.start_vcol;
            bdp.is_short = 1;
            if !is_del || oap.op_type == OP_APPEND {
                bdp.endspaces = oap.end_vcol - oap.start_vcol + 1;
            }
        } else {
            bdp.startspaces = bdp.start_vcol - oap.start_vcol;
            if is_del && bdp.startspaces != 0 {
                bdp.startspaces = bdp.start_char_vcols - bdp.startspaces;
            }
            let mut pend: *const c_char = pstart;
            bdp.end_vcol = bdp.start_vcol;
            if bdp.end_vcol > oap.end_vcol {
                bdp.is_oneChar = 1;
                if oap.op_type == OP_INSERT {
                    bdp.endspaces = bdp.start_char_vcols - bdp.startspaces;
                } else if oap.op_type == OP_APPEND {
                    bdp.startspaces += oap.end_vcol - oap.start_vcol + 1;
                    bdp.endspaces = bdp.start_char_vcols - bdp.startspaces;
                } else {
                    bdp.startspaces = oap.end_vcol - oap.start_vcol + 1;
                    if is_del && oap.op_type != OP_LSHIFT {
                        bdp.startspaces =
                            bdp.start_char_vcols - (bdp.start_vcol - oap.start_vcol);
                        bdp.endspaces = bdp.end_vcol - oap.end_vcol - 1;
                    }
                }
            } else {
                cstype = init_charsize_arg(&mut csarg, curwin_mut(), lnum, line);
                ci = utf_ptr2StrCharInfo(pend);
                vcol = bdp.end_vcol;
                let mut prev_pend = pend;
                while vcol <= oap.end_vcol && !crate::memory::cstr_is_empty(ci.ptr) {
                    prev_pend = ci.ptr;
                    let cli = utf_ClusterInfo(ci);
                    incr = win_charsize(
                        cstype,
                        vcol,
                        ci.ptr,
                        ci.chr.value,
                        &mut csarg,
                        cli.cells,
                    )
                    .width;
                    vcol += incr;
                    ci = cli.next;
                }
                bdp.end_vcol = vcol;
                pend = ci.ptr;

                if bdp.end_vcol <= oap.end_vcol
                    && (!is_del || oap.op_type == OP_APPEND || oap.op_type == OP_REPLACE)
                {
                    bdp.is_short = 1;
                    if oap.op_type == OP_APPEND || g_st().virtual_op != 0 {
                        bdp.endspaces =
                            oap.end_vcol - bdp.end_vcol + c_int::from(oap.inclusive);
                    }
                } else if bdp.end_vcol > oap.end_vcol {
                    bdp.endspaces = bdp.end_vcol - oap.end_vcol - 1;
                    if !is_del && bdp.endspaces != 0 {
                        bdp.endspaces = incr - bdp.endspaces;
                        if !ptr::eq(pend, pstart) {
                            pend = prev_pend;
                        }
                    }
                }
            }
            bdp.end_char_vcols = incr;
            if is_del && bdp.startspaces != 0 {
                pstart = prev_pstart;
            }
            bdp.textlen = pend.offset_from(pstart) as c_int;
        }
        bdp.textcol = pstart.offset_from(line) as Colnr;
        bdp.textstart = pstart;
        restore_lbr(lbr_saved);
    }
}

pub fn getregionpos_prep(
    p1: &mut CursorPos,
    p2: &mut CursorPos,
    region_type: c_int,
    exclusive: bool,
    block_width: c_int,inclusive: &mut bool,
    oap: &mut Oparg,
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
            if *inclusive && g_st().virtual_op == 0 && *ml_get_pos(p2) == NUL {
                *inclusive = false;
            }
        } else if region_type == kMTBlockWise {
            let mut sc1: Colnr = 0;
            let mut ec1: Colnr = 0;
            let mut sc2: Colnr = 0;
            let mut ec2: Colnr = 0;
            let lbr_saved = reset_lbr();
            getvvcol(curwin_ptr(), p1, Some(&mut sc1), None, Some(&mut ec1), 0);
            getvvcol(curwin_ptr(), p2, Some(&mut sc2), None, Some(&mut ec2), 0);
            restore_lbr(lbr_saved);
            oap.motion_type = kMTBlockWise;
            oap.inclusive = true;
            oap.op_type = OP_NOP;
            oap.start = *p1;
            oap.end = *p2;
            oap.start_vcol = sc1.min(sc2);
            if block_width > 0 {
                oap.end_vcol = oap.start_vcol + block_width - 1;
            } else if exclusive && ec1 < sc2 && 0 < sc2 && ec2 > ec1 {
                oap.end_vcol = sc2 - 1;
            } else {
                oap.end_vcol = ec1.max(ec2);
            }
        }

        let l = utfc_ptr2len(ml_get_pos(p2));
        if l > 1 {
            p2.col += l - 1;
        }
    }
}

pub fn getregionpos_line(
    lnum: Linenr,
    p1: CursorPos,
    p2: CursorPos,
    inclusive: bool,
    region_type: c_int,
    oap: &mut Oparg,
    allow_eol: bool,
    ret_p1: &mut CursorPos,
    ret_p2: &mut CursorPos,
) {
    unsafe {
        let line = ml_get(lnum);
        let line_len = ml_get_len(lnum);

        if region_type == kMTLineWise {
            ret_p1.col = 1;
            ret_p1.coladd = 0;
            ret_p2.col = MAXCOL;
            ret_p2.coladd = 0;
        } else {
            let mut bd = BlockDef::new();

            if region_type == kMTBlockWise {
                block_prep(oap, &mut bd, lnum, false);
            } else {
                charwise_block_prep(p1, p2, &mut bd, lnum, inclusive);
            }

            if bd.is_oneChar != 0 {
                if region_type == kMTBlockWise {
                    ret_p1.col =
                        (mb_prevptr(line, bd.textstart).offset_from(line) as Colnr) + 1;
                    ret_p1.coladd = bd.start_char_vcols - (bd.start_vcol - oap.start_vcol);
                } else {
                    ret_p1.col = p1.col + 1;
                    ret_p1.coladd = p1.coladd;
                }
            } else if region_type == kMTBlockWise && oap.start_vcol > bd.start_vcol {
                ret_p1.col = MAXCOL;
                ret_p1.coladd = oap.start_vcol - bd.start_vcol;
                bd.is_oneChar = 1;
            } else if bd.startspaces > 0 {
                ret_p1.col = (mb_prevptr(line, bd.textstart).offset_from(line) as Colnr) + 1;
                ret_p1.coladd = bd.start_char_vcols - bd.startspaces;
            } else {
                ret_p1.col = bd.textcol + 1;
                ret_p1.coladd = 0;
            }

            if bd.is_oneChar != 0 {
                ret_p2.col = ret_p1.col;
                ret_p2.coladd = ret_p1.coladd + bd.startspaces + bd.endspaces;
            } else if bd.endspaces > 0 {
                ret_p2.col = bd.textcol + bd.textlen + 1;
                ret_p2.coladd = bd.endspaces;
            } else {
                ret_p2.col = bd.textcol + bd.textlen;
                ret_p2.coladd = 0;
            }
        }

        if !allow_eol && ret_p1.col > line_len {
            ret_p1.col = 0;
            ret_p1.coladd = 0;
        } else if ret_p1.col > line_len + 1 {
            ret_p1.col = line_len + 1;
        }

        if !allow_eol && ret_p2.col > line_len {
            ret_p2.col = if ret_p1.col == 0 { 0 } else { line_len };
            ret_p2.coladd = 0;
        } else if ret_p2.col > line_len + 1 {
            ret_p2.col = line_len + 1;
        }

        ret_p1.lnum = lnum;
        ret_p2.lnum = lnum;
    }
}

pub fn charwise_block_prep(
    mut start: CursorPos,
    mut end: CursorPos,
    bdp: &mut BlockDef,
    lnum: Linenr,
    inclusive: bool,
) {
    unsafe {
        let mut startcol: Colnr = 0;
        let mut endcol: Colnr = MAXCOL;
        let mut cs: Colnr = 0;
        let mut ce: Colnr = 0;
        let p = ml_get(lnum);
        let plen = ml_get_len(lnum);

        bdp.startspaces = 0;
        bdp.endspaces = 0;
        bdp.is_oneChar = 0;
        bdp.start_char_vcols = 0;

        if lnum == start.lnum {
            startcol = start.col;
            if g_st().virtual_op != 0 {
                getvcol(
                    curwin_mut(),
                    &mut start,
                    Some(&mut cs),
                    None,
                    Some(&mut ce),
                    0,
                );
                if ce != cs && start.coladd > 0 {
                    bdp.start_char_vcols = ce - cs + 1;
                    bdp.startspaces = (bdp.start_char_vcols - start.coladd).max(0);
                    startcol += 1;
                }
            }
        }

        if lnum == end.lnum {
            endcol = end.col;
            if g_st().virtual_op != 0 {
                getvcol(
                    curwin_mut(),
                    &mut end,
                    Some(&mut cs),
                    None,
                    Some(&mut ce),
                    0,
                );
                if crate::memory::cstr_byte_at(p, endcol as usize) == 0
                    || (cs + end.coladd < ce && utf_head_off(p, p.add(endcol as usize)) == 0)
                {
                    if start.lnum == end.lnum && start.col == end.col {
                        bdp.is_oneChar = 1;
                        bdp.startspaces = end.coladd - start.coladd + c_int::from(inclusive);
                        endcol = startcol;
                    } else {
                        bdp.endspaces = end.coladd + c_int::from(inclusive);
                        endcol -= c_int::from(inclusive);
                    }
                }
            }
        }
        if endcol == MAXCOL {
            endcol = ml_get_len(lnum);
        }
        if startcol > endcol || bdp.is_oneChar != 0 {
            bdp.textlen = 0;
        } else {
            bdp.textlen = endcol - startcol + c_int::from(inclusive);
        }
        bdp.textcol = startcol;
        bdp.textstart = if startcol <= plen {
            p.add(startcol as usize)
        } else {
            p
        };
    }
}

pub fn op_addsub(oap: &mut Oparg, Prenum1: Linenr, g_cmd: bool) {
     {
        let mut bd = BlockDef::new();
        let mut change_cnt: isize = 0;
        let mut amount = Prenum1;

        g_st().disable_fold_update += 1;

        if !g_st().Visual.active {
            let mut pos = curwin_mut().w_cursor;
            if !u_save_cursor() {
                g_st().disable_fold_update -= 1;
                return;
            }
            change_cnt = isize::from(do_addsub(oap.op_type, &mut pos, 0, amount));
            g_st().disable_fold_update -= 1;
            if change_cnt != 0 {
                changed_lines(curbuf_mut(), pos.lnum, 0, pos.lnum + 1, 0, true);
            }
        } else {
            let mut length: c_int;
            let mut startpos = CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            };

            if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
                g_st().disable_fold_update -= 1;
                return;
            }

            let mut pos = oap.start;
            while pos.lnum <= oap.end.lnum {
                if oap.motion_type == kMTBlockWise {
                    block_prep(oap, &mut bd, pos.lnum, false);
                    pos.col = bd.textcol;
                    length = bd.textlen;
                } else if oap.motion_type == kMTLineWise {
                    curwin_mut().w_cursor.col = 0;
                    pos.col = 0;
                    length = ml_get_len(pos.lnum);
                } else {
                    if pos.lnum == oap.start.lnum && !oap.inclusive {
                        dec(&mut oap.end);
                    }
                    length = ml_get_len(pos.lnum);
                    pos.col = 0;
                    if pos.lnum == oap.start.lnum {
                        pos.col += oap.start.col;
                        length -= oap.start.col;
                    }
                    if pos.lnum == oap.end.lnum {
                        length = ml_get_len(oap.end.lnum);
                        oap.end.col = oap.end.col.min(length - 1);
                        length = oap.end.col - pos.col + 1;
                    }
                }
                let one_change = do_addsub(oap.op_type, &mut pos, length, amount);
                if one_change {
                    if change_cnt == 0 {
                        startpos = curbuf_mut().b_op_start;
                    }
                    change_cnt += 1;
                }

                if g_cmd && one_change {
                    amount += Prenum1;
                }
                pos.lnum += 1;
            }

            g_st().disable_fold_update -= 1;
            if change_cnt != 0 {
                changed_lines(curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
            }

            if change_cnt == 0 && oap.is_VIsual {
                redraw_curbuf_later(UPD_INVERTED);
            }

            if change_cnt > 0 && (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
                curbuf_mut().b_op_start = startpos;
            }

            if (change_cnt as i64) > opt_vars().p_report {
                smsg!(
                    0,
                    ngettext(
                        c"%lld lines changed",
                        c"%lld lines changed",
                        change_cnt as c_ulong,
                    ),
                    change_cnt as std::ffi::c_longlong,
                );
            }
        }
    }
}

pub fn do_addsub(
    op_type: c_int,
    pos: &mut CursorPos,
    mut length: c_int,
    Prenum1: Linenr,
) -> bool {
    unsafe {
        let mut pre: c_int = 0;
        let mut n: Uvarnumber = 0;
        let mut blank_unsigned = false;
        let mut negative = false;
        let mut was_positive = true;
        let visual = g_st().Visual.active;
        let did_change: bool;
        let save_cursor = curwin_mut().w_cursor;
        let mut maxlen: c_int = 0;
        let startpos: CursorPos;
        let endpos: CursorPos;
        let mut save_coladd: Colnr = 0;

        let do_hex = !vim_strchr(curbuf_mut().b_p_nf, b'x' as c_int).is_null();
        let do_oct = !vim_strchr(curbuf_mut().b_p_nf, b'o' as c_int).is_null();
        let do_bin = !vim_strchr(curbuf_mut().b_p_nf, b'b' as c_int).is_null();
        let do_alpha = !vim_strchr(curbuf_mut().b_p_nf, b'p' as c_int).is_null();
        let do_unsigned = !vim_strchr(curbuf_mut().b_p_nf, b'u' as c_int).is_null();
        let do_blank = !vim_strchr(curbuf_mut().b_p_nf, b'k' as c_int).is_null();

        if virtual_active(curwin_mut()) {
            save_coladd = pos.coladd;
            pos.coladd = 0;
        }

        curwin_mut().w_cursor = *pos;
        let mut ptr_line = ml_get(pos.lnum);
        let linelen = ml_get_len(pos.lnum);
        let mut col = pos.col;

        if col + c_int::from(save_coladd != 0) >= linelen {
            if visual {
                curwin_mut().w_cursor = save_cursor;
            } else if virtual_active(curwin_mut()) {
                curwin_mut().w_cursor.coladd = save_coladd;
            }
            return false;
        }

        if !g_st().Visual.active {
            if do_bin {
                while col > 0 && ascii_isbdigit(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }

            if do_hex {
                while col > 0 && ascii_isxdigit(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }
            if do_bin
                && do_hex
                && !(col > 0
                    && (crate::memory::cstr_byte_at(ptr_line, col as usize) == b'X'
                        || crate::memory::cstr_byte_at(ptr_line, col as usize) == b'x')
                    && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                    && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                    && ascii_isxdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
            {
                col = curwin_mut().w_cursor.col;

                while col > 0 && ascii_isdigit(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))) {
                    col -= 1;
                    col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
                }
            }

            if (do_hex
                && col > 0
                && (crate::memory::cstr_byte_at(ptr_line, col as usize) == b'X'
                    || crate::memory::cstr_byte_at(ptr_line, col as usize) == b'x')
                && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                && ascii_isxdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
                || (do_bin
                    && col > 0
                    && (crate::memory::cstr_byte_at(ptr_line, col as usize) == b'B'
                        || crate::memory::cstr_byte_at(ptr_line, col as usize) == b'b')
                    && *ptr_line.add((col - 1) as usize) == b'0' as c_char
                    && utf_head_off(ptr_line, ptr_line.add((col - 1) as usize)) == 0
                    && ascii_isbdigit(c_int::from(*ptr_line.add((col + 1) as usize) as u8)))
            {
                col -= 1;
                col -= utf_head_off(ptr_line, ptr_line.add(col as usize));
            } else {
                col = pos.col;

                while crate::memory::cstr_byte_at(ptr_line, col as usize) != 0
                    && !ascii_isdigit(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize)))
                    && !(do_alpha && ascii_isalpha(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))))
                {
                    col += 1;
                }

                while col > 0
                    && ascii_isdigit(c_int::from(*ptr_line.add((col - 1) as usize) as u8))
                    && !(do_alpha && ascii_isalpha(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))))
                {
                    col -= 1;
                }
            }
        }

        if visual {
            while crate::memory::cstr_byte_at(ptr_line, col as usize) != 0
                && length > 0
                && !ascii_isdigit(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize)))
                && !(do_alpha && ascii_isalpha(c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize))))
            {
                let mb_len = utfc_ptr2len(ptr_line.add(col as usize));
                col += mb_len;
                length -= mb_len;
            }

            if length == 0 {
                curwin_mut().w_cursor = save_cursor;
                return false;
            }

            if col > pos.col
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

        let mut firstdigit = c_int::from(crate::memory::cstr_byte_at(ptr_line, col as usize));
        if !ascii_isdigit(firstdigit) && !(do_alpha && ascii_isalpha(firstdigit)) {
            beep_flush();
            if visual {
                curwin_mut().w_cursor = save_cursor;
            } else if virtual_active(curwin_mut()) {
                curwin_mut().w_cursor.coladd = save_coladd;
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
            curwin_mut().w_cursor.col = col;
            startpos = curwin_mut().w_cursor;
            did_change = true;
            del_char(false);
            ins_char(firstdigit);
            endpos = curwin_mut().w_cursor;
            curwin_mut().w_cursor.col = col;
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

            if visual && g_st().Visual.mode != b'V' as c_int {
                maxlen = if curbuf_mut().b_visual.vi_curswant == MAXCOL {
                    linelen - col
                } else {
                    length
                };
            }

            let mut overflow = false;
            vim_str2nr(
                ptr_line.add(col as usize),
                Some(&mut pre),
                Some(&mut length),
                (if do_bin { STR2NR_BIN } else { 0 })
                    + (if do_oct { STR2NR_OCT } else { 0 })
                    + (if do_hex { STR2NR_HEX } else { 0 }),
                None,
                Some(&mut n),
                maxlen,
                false,
                Some(&mut overflow),
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
                    n.wrapping_sub(Prenum1 as Uvarnumber)
                } else {
                    n.wrapping_add(Prenum1 as Uvarnumber)
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

            curwin_mut().w_cursor.col = col;
            startpos = curwin_mut().w_cursor;
            did_change = true;
            let mut todel = length;
            let mut c = gchar_cursor();

            if c == b'-' as c_int {
                length -= 1;
            }
            while todel > 0 {
                todel -= 1;
                if c < 0x100 && ascii_isalpha(c) {
                    ops_st().addsub_hexupper = ascii_isupper(c);
                }
                del_char(false);
                c = gchar_cursor();
            }

            let mut buf1_vec = vec![0 as c_char; (length as usize) + NUMBUFLEN];
            let buf1 = buf1_vec.as_mut_ptr();
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
                let mut bits = Uvarnumber::BITS as usize;
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
                    c"%llu",
                    n as std::ffi::c_ulonglong,
                );
            } else if pre == b'0' as c_int {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llo",
                    n as std::ffi::c_ulonglong,
                );
            } else if ops_st().addsub_hexupper {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llX",
                    n as std::ffi::c_ulonglong,
                );
            } else {
                buf2len = vim_snprintf!(
                    buf2_ptr,
                    NUMBUFLEN,
                    c"%llx",
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

            crate::memory::cstr_cpy(buf1.add(buf1len as usize), buf2_ptr);
            buf1len += buf2len;

            ins_str(buf1, buf1len as usize);
            drop(buf1_vec);

            endpos = curwin_mut().w_cursor;
            if curwin_mut().w_cursor.col != 0 {
                curwin_mut().w_cursor.col -= 1;
            }
        }

        if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start = startpos;
            curbuf_mut().b_op_end = endpos;
            if curbuf_mut().b_op_end.col > 0 {
                curbuf_mut().b_op_end.col -= 1;
            }
        }

        if visual {
            curwin_mut().w_cursor = save_cursor;
        } else if did_change {
            curwin_mut().w_set_curswant = true;
        } else if virtual_active(curwin_mut()) {
            curwin_mut().w_cursor.coladd = save_coladd;
        }

        did_change
    }
}

fn line_count_info(
    line: impl crate::strings::IntoCCharPtr,
    wc: &mut Varnumber,
    cc: &mut Varnumber,
    limit: Varnumber,
    eol_size: c_int,
) -> Varnumber { let line = line.into_c_char_ptr();
    unsafe {
        let mut i: Varnumber = 0;
        let mut words: Varnumber = 0;
        let mut chars: Varnumber = 0;
        let mut is_word = false;

        while i < limit && crate::memory::cstr_byte_at(line, i as usize) != 0 {
            let ch = c_int::from(crate::memory::cstr_byte_at(line, i as usize));
            if is_word {
                if ascii_isspace(ch) {
                    words += 1;
                    is_word = false;
                }
            } else if !ascii_isspace(ch) {
                is_word = true;
            }
            chars += 1;
            i += Varnumber::from(utfc_ptr2len(line.add(i as usize)));
        }

        if is_word {
            words += 1;
        }
        *wc += words;

        if i < limit && crate::memory::cstr_byte_at(line, i as usize) == 0 {
            i += Varnumber::from(eol_size);
            chars += Varnumber::from(eol_size);
        }
        *cc += chars;
        i
    }
}

pub fn cursor_pos_info(mut dict: Option<&mut TvDict>) {
    unsafe {
        let mut buf1 = [0 as c_char; 50];
        let buf1_ptr = (&raw mut buf1).cast::<c_char>();
        let mut buf2 = [0 as c_char; 40];
        let buf2_ptr = (&raw mut buf2).cast::<c_char>();
        let mut byte_count: Varnumber = 0;
        let mut bom_count: Varnumber = 0;
        let mut byte_count_cursor: Varnumber = 0;
        let mut char_count: Varnumber = 0;
        let mut char_count_cursor: Varnumber = 0;
        let mut word_count: Varnumber = 0;
        let mut word_count_cursor: Varnumber = 0;
        let mut min_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut max_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut oparg = Oparg::new();
        let mut bd = BlockDef::new();
        let l_VIsual_active = g_st().Visual.active;
        let l_VIsual_mode = g_st().Visual.mode;

        if (curbuf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
            if dict.is_none() {
                msg(tr(no_lines_msg), 0);
                return;
            }
        } else {
            let mut last_check: Varnumber = 100000;
            let mut line_count_selected: c_int = 0;
            let eol_size = if get_fileformat(curbuf_mut()) == EOL_DOS {
                2
            } else {
                1
            };

            if l_VIsual_active {
                if lt(g_st().Visual.start, curwin_mut().w_cursor) {
                    min_pos = g_st().Visual.start;
                    max_pos = curwin_mut().w_cursor;
                } else {
                    min_pos = curwin_mut().w_cursor;
                    max_pos = g_st().Visual.start;
                }
                if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' && max_pos.col > 0 {
                    max_pos.col -= 1;
                }

                if l_VIsual_mode == Ctrl_V {
                    let saved_sbr = opt_vars().p_sbr;
                    let saved_w_sbr = curwin_mut().w_onebuf_opt.wo_sbr;

                    opt_vars().p_sbr = empty_string_option.as_ptr().cast::<c_char>();
                    curwin_mut().w_onebuf_opt.wo_sbr = empty_string_option.as_ptr().cast::<c_char>();
                    oparg.is_VIsual = true;
                    oparg.motion_type = kMTBlockWise;
                    oparg.op_type = OP_NOP;
                    getvcols(
                        curwin_ptr(),
                        &mut min_pos,
                        &mut max_pos,
                        &mut oparg.start_vcol,
                        &mut oparg.end_vcol,
                        0,
                    );
                    opt_vars().p_sbr = saved_sbr;
                    curwin_mut().w_onebuf_opt.wo_sbr = saved_w_sbr;
                    if curwin_mut().w_curswant == MAXCOL {
                        oparg.end_vcol = MAXCOL;
                    }
                    if oparg.end_vcol < oparg.start_vcol {
                        std::mem::swap(&mut oparg.start_vcol, &mut oparg.end_vcol);
                    }
                }
                line_count_selected = max_pos.lnum - min_pos.lnum + 1;
            }

            for lnum in 1..=curbuf_mut().b_ml.ml_line_count {
                if byte_count > last_check {
                    os_breakcheck();
                    if g_st().got_int {
                        return;
                    }
                    last_check = byte_count + 100000;
                }

                if l_VIsual_active && lnum >= min_pos.lnum && lnum <= max_pos.lnum {
                    let mut s: *mut c_char = ptr::null_mut();
                    let mut len: c_int = 0;

                    if l_VIsual_mode == Ctrl_V {
                        g_st().virtual_op = TriState::from(virtual_active(curwin_mut()));
                        block_prep(&mut oparg, &mut bd, lnum, false);
                        g_st().virtual_op = kNone;
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
                            &mut word_count_cursor,
                            &mut char_count_cursor,
                            Varnumber::from(len),
                            eol_size,
                        );
                        if lnum == curbuf_mut().b_ml.ml_line_count
                            && curbuf_mut().b_p_eol == 0
                            && (curbuf_mut().b_p_bin != 0 || curbuf_mut().b_p_fixeol == 0)
                            && (crate::memory::cstr_len(s) as c_int) < len
                        {
                            byte_count_cursor -= Varnumber::from(eol_size);
                        }
                    }
                } else if lnum == curwin_mut().w_cursor.lnum {
                    word_count_cursor += word_count;
                    char_count_cursor += char_count;
                    byte_count_cursor = byte_count
                        + line_count_info(
                            ml_get(lnum),
                            &mut word_count_cursor,
                            &mut char_count_cursor,
                            Varnumber::from(curwin_mut().w_cursor.col) + 1,
                            eol_size,
                        );
                }
                byte_count += line_count_info(
                    ml_get(lnum),
                    &mut word_count,
                    &mut char_count,
                    Varnumber::from(MAXCOL),
                    eol_size,
                );
            }

            if curbuf_mut().b_p_eol == 0 && (curbuf_mut().b_p_bin != 0 || curbuf_mut().b_p_fixeol == 0) {
                byte_count -= Varnumber::from(eol_size);
            }

            let iobuff_ptr = g_st().IObuff.as_mut_ptr();
            if dict.is_none() {
                if l_VIsual_active {
                    if l_VIsual_mode == Ctrl_V && curwin_mut().w_curswant < MAXCOL {
                        let mut min_col = min_pos.col;
                        let mut max_col = max_pos.col;
                        getvcols(
                            curwin_ptr(),
                            &mut min_pos,
                            &mut max_pos,
                            &mut min_col,
                            &mut max_col,
                            0,
                        );
                        let cols = i64::from(oparg.end_vcol + 1)
                            .checked_sub(i64::from(oparg.start_vcol))
                            .unwrap_or_else(|| std::process::abort());
                        vim_snprintf!(
                            buf1_ptr,
                            buf1.len(),
                            tr(c"%lld Cols; "),
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
                                c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Bytes",
                            ),
                            buf1_ptr,
                            i64::from(line_count_selected) as std::ffi::c_longlong,
                            i64::from(curbuf_mut().b_ml.ml_line_count) as std::ffi::c_longlong,
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
                                c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Chars; %lld of %lld Bytes",
                            ),
                            buf1_ptr,
                            i64::from(line_count_selected) as std::ffi::c_longlong,
                            i64::from(curbuf_mut().b_ml.ml_line_count) as std::ffi::c_longlong,
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
                    validate_virtcol(curwin_mut());
                    col_print(
                        buf1_ptr,
                        buf1.len(),
                        curwin_mut().w_cursor.col + 1,
                        curwin_mut().w_virtcol + 1,
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
                                c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Byte %lld of %lld",
                            ),
                            buf1_ptr,
                            buf2_ptr,
                            i64::from(curwin_mut().w_cursor.lnum) as std::ffi::c_longlong,
                            i64::from(curbuf_mut().b_ml.ml_line_count) as std::ffi::c_longlong,
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
                                c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Char %lld of %lld; Byte %lld of %lld",
                            ),
                            buf1_ptr,
                            buf2_ptr,
                            i64::from(curwin_mut().w_cursor.lnum) as std::ffi::c_longlong,
                            i64::from(curbuf_mut().b_ml.ml_line_count) as std::ffi::c_longlong,
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

            bom_count = Varnumber::from(bomb_size());
            if dict.is_none() && bom_count > 0 {
                let len = crate::memory::cstr_len(iobuff_ptr);
                vim_snprintf!(
                    iobuff_ptr.add(len),
                    IOSIZE - len,
                    tr(c"(+%lld for BOM)"),
                    bom_count as std::ffi::c_longlong,
                );
            }
            if dict.is_none() {
                let p = opt_vars().p_shm;
                opt_vars().p_shm = c"".as_ptr().cast_mut();
                if opt_vars().p_ch < 1 {
                    msg_start();
                    g_st().msg_scroll = 1;
                }
                msg(iobuff_ptr, 0);
                opt_vars().p_shm = p;
            }
        }

        if dict.is_some() {
            tv_dict_add_nr(dict.as_deref_mut().unwrap(), c"words", word_count);
            tv_dict_add_nr(dict.as_deref_mut().unwrap(), c"chars", char_count);
            tv_dict_add_nr(dict.as_deref_mut().unwrap(), c"bytes", byte_count + bom_count);

            tv_dict_add_nr(
                dict.as_deref_mut().unwrap(),
                if l_VIsual_active {
                    c"visual_bytes"
                } else {
                    c"cursor_bytes"
                },
                byte_count_cursor,
            );
            tv_dict_add_nr(
                dict.as_deref_mut().unwrap(),
                if l_VIsual_active {
                    c"visual_chars"
                } else {
                    c"cursor_chars"
                },
                char_count_cursor,
            );
            tv_dict_add_nr(
                dict.unwrap(),
                if l_VIsual_active {
                    c"visual_words"
                } else {
                    c"cursor_words"
                },
                word_count_cursor,
            );
        }
    }
}

fn op_colon(oap: &mut Oparg) {
     {
        if oap.is_VIsual {
            stuffReadbuff(c"'<,'>");
        } else {
            if oap.start.lnum == curwin_mut().w_cursor.lnum {
                stuffcharReadbuff(b'.' as c_int);
            } else {
                stuffnumReadbuff(oap.start.lnum);
            }

            let mut endOfStartFold = oap.start.lnum;
            hasFolding(
                curwin_ptr(),
                oap.start.lnum,
                None,
                &raw mut endOfStartFold,
            );
            if oap.end.lnum != oap.start.lnum && oap.end.lnum != endOfStartFold {
                stuffcharReadbuff(b',' as c_int);
                if oap.end.lnum == curwin_mut().w_cursor.lnum {
                    stuffcharReadbuff(b'.' as c_int);
                } else if oap.end.lnum == curbuf_mut().b_ml.ml_line_count {
                    stuffcharReadbuff(b'$' as c_int);
                } else if oap.start.lnum == curwin_mut().w_cursor.lnum
                    && !hasFolding(curwin_ptr(), oap.end.lnum, None, ptr::null_mut())
                {
                    stuffReadbuff(c".+");
                    stuffnumReadbuff(oap.line_count - 1);
                } else {
                    stuffnumReadbuff(oap.end.lnum);
                }
            }
        }
        if oap.op_type != OP_COLON {
            stuffReadbuff(c"!");
        }
        if oap.op_type == OP_INDENT {
            stuffReadbuff(get_equalprg());
            stuffReadbuff(c"\n");
        } else if oap.op_type == OP_FORMAT {
            if !crate::memory::cstr_is_empty(curbuf_mut().b_p_fp) {
                stuffReadbuff(curbuf_mut().b_p_fp);
            } else if !crate::memory::cstr_is_empty(opt_vars().p_fp) {
                stuffReadbuff(opt_vars().p_fp);
            } else {
                stuffReadbuff(c"fmt");
            }
            stuffReadbuff(c"\n']");
        }

        do_cmdline(ptr::null_mut(), crate::cmdexpand::LineGetter::ExLine, 0);
        atom_suppress(true);
        exec_stuffed(None);
        atom_suppress(false);
    }
}

pub fn free_operatorfunc_option() {
    callback_free(&mut opt_vars().p_opfunc);
}

pub fn set_ref_in_opfunc(copyID: c_int) -> bool {
    set_ref_in_callback(&mut opt_vars().p_opfunc, copyID, ptr::null_mut(), ptr::null_mut())
}

fn op_function(oap: &Oparg) {
     {
        let orig_start = curbuf_mut().b_op_start;
        let orig_end = curbuf_mut().b_op_end;

        if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_opfunc) == kCallbackNone {
            emsg(tr(c"E774: 'operatorfunc' is empty"));
        } else {
            curbuf_mut().b_op_start = oap.start;
            curbuf_mut().b_op_end = oap.end;
            if oap.motion_type == kMTCharWise && !oap.inclusive {
                decl(&mut curbuf_mut().b_op_end);
            }

            let mode_str = match oap.motion_type {
                kMTBlockWise => c"block".as_ptr(),
                kMTLineWise => c"line".as_ptr(),
                _ => c"char".as_ptr(),
            };
            let mut argv = [
                TypVal {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval::String(mode_str.cast_mut()),
                },
                TypVal {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: 0,
                    vval: TypvalVval::String(ptr::null_mut()),
                },
            ];

            let save_virtual_op = g_st().virtual_op;
            g_st().virtual_op = kNone;

            let save_finish_op = g_st().finish_op;
            g_st().finish_op = false;

            let mut save_redo = RedoState::new();
            save_redobuff(&mut save_redo);

            let mut rettv: TypVal = TypVal::new();
            if callback_call(
                &mut opt_vars().p_opfunc,
                1,
                (&raw mut argv).cast::<TypVal>(),
                &mut rettv,
            ) {
                tv_clear(&mut rettv);
            }
            restore_redobuff(&mut save_redo);

            g_st().virtual_op = save_virtual_op;
            g_st().finish_op = save_finish_op;
            if (g_st().cmdmod.cmod_flags & CMOD_LOCKMARKS) != 0 {
                curbuf_mut().b_op_start = orig_start;
                curbuf_mut().b_op_end = orig_end;
            }
        }
    }
}

fn get_op_vcol(oap: &mut Oparg, initial: bool) {
     {
        let mut start: Colnr = 0;
        let mut end: Colnr = 0;

        if g_st().Visual.mode != Ctrl_V || (!initial && oap.end.col < curwin_mut().w_view_width) {
            return;
        }

        mark_mb_adjustpos(curwin_mut().w_buffer, &mut oap.end);

        getvvcol(
            curwin_ptr(),
            &mut oap.start,
            Some(&mut oap.start_vcol),
            None,
            Some(&mut oap.end_vcol),
            0,
        );
        getvvcol(
            curwin_ptr(),
            &mut oap.end,
            Some(&mut start),
            None,
            Some(&mut end),
            0,
        );

        oap.start_vcol = oap.start_vcol.min(start);
        if end > oap.end_vcol {
            if initial && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' && start >= 1 && start > oap.end_vcol {
                oap.end_vcol = start - 1;
            } else {
                oap.end_vcol = end;
            }
        }

        if curwin_mut().w_curswant == MAXCOL {
            curwin_mut().w_cursor.col = MAXCOL;
            oap.end_vcol = 0;
            curwin_mut().w_cursor.lnum = oap.start.lnum;
            while curwin_mut().w_cursor.lnum <= oap.end.lnum {
                getvvcol(
                    curwin_ptr(),
                    &mut curwin_mut().w_cursor,
                    None,
                    None,
                    Some(&mut end),
                    0,
                );
                oap.end_vcol = oap.end_vcol.max(end);
                curwin_mut().w_cursor.lnum += 1;
            }
        }

        curwin_mut().w_cursor.lnum = oap.end.lnum;
        coladvance(curwin_mut(), oap.end_vcol);
        oap.end = curwin_mut().w_cursor;

        curwin_mut().w_cursor = oap.start;
        coladvance(curwin_mut(), oap.start_vcol);
        oap.start = curwin_mut().w_cursor;
    }
}

fn is_ex_cmdchar(cap: &Cmdarg) -> bool {
    cap.cmdchar == b':' as c_int || cap.cmdchar == K_COMMAND
}

pub fn op_self_select(cap: &Cmdarg) -> bool {
    cap.cmdchar == b'g' as c_int
        && (cap.nchar == b'n' as c_int
            || cap.nchar == b'N' as c_int
            || cap.nchar == b'v' as c_int)
}

fn op_ins_visual(oap: &mut Oparg, cap: &mut Cmdarg) -> VisualIns {
    if !oap.is_VIsual {
            return kVInsNone;
        }
        if is_ex_cmdchar(cap) || cap.cmdchar == K_LUA {
            return kVInsMotion;
        }
        if op_self_select(cap)
            && cap.nchar != b'v' as c_int
            && oap.motion_force == c_int::from(NUL)
        {
            return kVInsMotion;
        }
        if oap.motion_force != c_int::from(NUL) || op_self_select(cap) {
            return kVInsOther;
        }
        if atom_visual_redoable() {
            kVInsKeys
        } else {
            kVInsOther
        }
}

pub fn do_pending_operator(cap: &mut Cmdarg, old_col: c_int, gui_yank: bool) {
    unsafe {
        let oap = cap.oap_mut();
        let lbr_saved = curwin_mut().w_onebuf_opt.wo_lbr != 0;

        let old_cursor = curwin_mut().w_cursor;
        let op_origin = oap.start;

        if (g_st().finish_op || g_st().Visual.active) && oap.op_type != OP_NOP {
            let mut include_line_break = false;
            let redo_yank = !vim_strchr(opt_vars().p_cpo, kCpoYank).is_null() && !gui_yank;

            reset_lbr();
            oap.is_VIsual = g_st().Visual.active;
            if oap.motion_force == b'V' as c_int {
                oap.motion_type = kMTLineWise;
            } else if oap.motion_force == b'v' as c_int {
                if oap.motion_type == kMTLineWise {
                    oap.inclusive = false;
                } else if oap.motion_type == kMTCharWise {
                    oap.inclusive = !oap.inclusive;
                }
                oap.motion_type = kMTCharWise;
            } else if oap.motion_force == Ctrl_V {
                if !g_st().Visual.active {
                    g_st().Visual.active = true;
                    g_st().Visual.start = oap.start;
                }
                g_st().Visual.mode = Ctrl_V;
                g_st().Visual.select = false;
                g_st().Visual.reselect = 0;
            }

            atom_capture_op(oap, cap, redo_yank);

            if g_st().Visual.active {
                if !gui_yank && !g_st().finish_op {
                    curbuf_mut().b_visual = visualinfo();
                    restore_visual_mode();
                    curbuf_mut().b_visual_mode_eval = g_st().Visual.mode;
                }

                if g_st().Visual.select
                    && g_st().Visual.mode == b'V' as c_int
                    && cap.oap_mut().op_type != OP_DELETE
                {
                    if lt(g_st().Visual.start, curwin_mut().w_cursor) {
                        g_st().Visual.start.col = 0;
                        curwin_mut().w_cursor.col = ml_get_len(curwin_mut().w_cursor.lnum);
                    } else {
                        curwin_mut().w_cursor.col = 0;
                        g_st().Visual.start.col = ml_get_len(g_st().Visual.start.lnum);
                    }
                    g_st().Visual.mode = b'v' as c_int;
                } else if g_st().Visual.mode == b'v' as c_int {
                    include_line_break = unadjust_for_sel();
                }

                oap.start = g_st().Visual.start;
                if g_st().Visual.mode == b'V' as c_int {
                    oap.start.col = 0;
                    oap.start.coladd = 0;
                }
            }

            if lt(oap.start, curwin_mut().w_cursor) {
                if !g_st().Visual.active {
                    if hasFolding(
                        curwin_ptr(),
                        oap.start.lnum,
                        Some(&mut oap.start.lnum),
                        ptr::null_mut(),
                    ) {
                        oap.start.col = 0;
                    }
                    if (curwin_mut().w_cursor.col > 0
                        || oap.inclusive
                        || oap.motion_type == kMTLineWise)
                        && hasFolding(
                            curwin_ptr(),
                            curwin_mut().w_cursor.lnum,
                            None,
                            &raw mut (*curwin_ptr()).w_cursor.lnum,
                        )
                    {
                        curwin_mut().w_cursor.col = get_cursor_line_len();
                    }
                }
                oap.end = curwin_mut().w_cursor;
                curwin_mut().w_cursor = oap.start;

                curwin_mut().w_valid &= !VALID_VIRTCOL;
            } else {
                if !g_st().Visual.active && oap.motion_type == kMTLineWise {
                    if hasFolding(
                        curwin_ptr(),
                        curwin_mut().w_cursor.lnum,
                        Some(&mut curwin_mut().w_cursor.lnum),
                        ptr::null_mut(),
                    ) {
                        curwin_mut().w_cursor.col = 0;
                    }
                    if hasFolding(
                        curwin_ptr(),
                        oap.start.lnum,
                        None,
                        &raw mut oap.start.lnum,
                    ) {
                        oap.start.col = ml_get_len(oap.start.lnum);
                    }
                }
                oap.end = oap.start;
                oap.start = curwin_mut().w_cursor;
            }

            check_pos(curwin_mut().buf_mut(), &mut oap.end);
            oap.line_count = oap.end.lnum - oap.start.lnum + 1;

            g_st().virtual_op = TriState::from(virtual_active(curwin_mut()));

            if g_st().Visual.active {
                get_op_vcol(&mut *oap, true);

                if !gui_yank {
                    g_st().Visual.resel.mode = g_st().Visual.mode;
                    if curwin_mut().w_curswant == MAXCOL {
                        g_st().Visual.resel.vcol = MAXCOL;
                    } else {
                        if g_st().Visual.mode != Ctrl_V {
                            getvvcol(
                                curwin_ptr(),
                                &mut oap.end,
                                None,
                                None,
                                Some(&mut oap.end_vcol),
                                0,
                            );
                        }
                        if g_st().Visual.mode == Ctrl_V || oap.line_count <= 1 {
                            if g_st().Visual.mode != Ctrl_V {
                                getvvcol(
                                    curwin_ptr(),
                                    &mut oap.start,
                                    Some(&mut oap.start_vcol),
                                    None,
                                    None,
                                    0,
                                );
                            }
                            g_st().Visual.resel.vcol = oap.end_vcol - oap.start_vcol + 1;
                        } else {
                            g_st().Visual.resel.vcol = oap.end_vcol;
                        }
                    }
                    g_st().Visual.resel.line_count = oap.line_count;
                }

                if oap.motion_force == c_int::from(NUL) || oap.motion_type == kMTLineWise {
                    oap.inclusive = true;
                }
                if g_st().Visual.mode == b'V' as c_int {
                    oap.motion_type = kMTLineWise;
                } else if g_st().Visual.mode == b'v' as c_int {
                    oap.motion_type = kMTCharWise;
                    if *ml_get_pos(&oap.end) == NUL
                        && (include_line_break || g_st().virtual_op == 0)
                    {
                        oap.inclusive = false;
                        if crate::memory::cstr_first_byte(opt_vars().p_sel) != b'o'
                            && op_on_lines(oap.op_type) == 0
                            && oap.end.lnum < curbuf_mut().b_ml.ml_line_count
                        {
                            oap.end.lnum += 1;
                            oap.end.col = 0;
                            oap.end.coladd = 0;
                            oap.line_count += 1;
                        }
                    }
                } else if g_st().Visual.mode == Ctrl_V {
                    oap.motion_type = kMTBlockWise;
                }

                if !gui_yank {
                    g_st().Visual.active = false;
                    setmouse();
                    g_st().mouse_dragging = 0;
                    may_clear_cmdline();
                    if (oap.op_type == OP_YANK
                        || oap.op_type == OP_COLON
                        || oap.op_type == OP_FUNCTION
                        || oap.op_type == OP_FILTER)
                        && oap.motion_force == c_int::from(NUL)
                    {
                        restore_lbr(lbr_saved);
                        redraw_curbuf_later(UPD_INVERTED);
                    }
                }
            }

            if oap.inclusive {
                let l = utfc_ptr2len(ml_get_pos(&oap.end));
                if l > 1 {
                    oap.end.col += l - 1;
                }
            }
            curwin_mut().w_set_curswant = true;

            oap.empty = oap.motion_type != kMTLineWise
                && ((!oap.inclusive && oap.motion_type != kMTBlockWise)
                    || (oap.op_type == OP_YANK
                        && gchar_pos(&mut oap.end) == c_int::from(NUL)))
                && equalpos(oap.start, oap.end)
                && !(g_st().virtual_op != 0 && oap.start.coladd != oap.end.coladd);
            let empty_region_error = oap.empty && !vim_strchr(opt_vars().p_cpo, kCpoEmptyregion).is_null();

            if oap.is_VIsual
                && (oap.empty || !modifiable(curbuf_mut()) || oap.op_type == OP_FOLD)
            {
                restore_lbr(lbr_saved);
                redraw_curbuf_later(UPD_INVERTED);
            }

            if oap.motion_type == kMTCharWise
                && !oap.inclusive
                && (cap.retval & CA_NO_ADJ_OP_END) == 0
                && oap.end.col == 0
                && (!oap.is_VIsual || crate::memory::cstr_first_byte(opt_vars().p_sel) == b'o')
                && oap.line_count > 1
            {
                oap.end_adjusted = true;
                oap.line_count -= 1;
                oap.end.lnum -= 1;
                if inindent(0) {
                    oap.motion_type = kMTLineWise;
                } else {
                    oap.end.col = ml_get_len(oap.end.lnum);
                    if oap.end.col != 0 {
                        oap.end.col -= 1;
                        oap.inclusive = true;
                    }
                }
            } else {
                oap.end_adjusted = false;
            }

            match oap.op_type {
                OP_LSHIFT | OP_RSHIFT => {
                    op_shift(oap, true, if oap.is_VIsual { cap.count1 } else { 1 });
                    auto_format(false, true);
                }
                OP_JOIN_NS | OP_JOIN => {
                    oap.line_count = oap.line_count.max(2);
                    if curwin_mut().w_cursor.lnum + oap.line_count - 1
                        > curbuf_mut().b_ml.ml_line_count
                    {
                        beep_flush();
                    } else {
                        do_join(
                            oap.line_count as usize,
                            oap.op_type == OP_JOIN,
                            true,
                            true,
                            true,
                        );
                        auto_format(false, true);
                    }
                }
                OP_DELETE => {
                    g_st().Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        op_delete(oap);
                        if oap.motion_type == kMTLineWise
                            && has_format_option(kFoAuto)
                            && u_save_cursor()
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
                        oap.excl_tr_ws = cap.cmdchar == b'z' as c_int;
                        if oap.restore_cursor {
                            curwin_mut().w_cursor = oap.cursor_start;
                        }
                        op_yank(oap, !gui_yank);
                    }
                    check_cursor_col(curwin_mut());
                }
                OP_CHANGE => {
                    g_st().Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        let restart_edit_save = if !g_st().KeyTyped { g_st().restart_edit } else { 0 };
                        g_st().restart_edit = 0;

                        restore_lbr(lbr_saved);

                        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());

                        let mut session: InsSession = atom_ins_start(
                            b'c' as c_int,
                            c_long::from(cap.count0),
                            op_ins_visual(oap, cap),
                            oap.is_VIsual && oap.motion_type == kMTBlockWise,
                        );
                        let busy = op_change(oap) != 0;
                        if busy {
                            cap.retval |= CA_COMMAND_BUSY;
                        }
                        if g_st().restart_edit == 0 {
                            g_st().restart_edit = restart_edit_save;
                        }
                        atom_ins_end(&mut session, busy);
                    }
                }
                OP_FILTER | OP_INDENT | OP_COLON => {
                    if oap.op_type == OP_FILTER {
                        if !vim_strchr(opt_vars().p_cpo, kCpoFilter).is_null() {
                            redo_append_str(c"!\r", 2);
                        } else {
                            g_st().bangredo = true;
                        }
                    }

                    if oap.op_type == OP_INDENT && crate::memory::cstr_is_empty(get_equalprg()) {
                        if curbuf_mut().b_p_lisp != 0 {
                            if use_indentexpr_for_lisp() {
                                op_reindent(oap, get_expr_indent);
                            } else {
                                op_reindent(oap, get_lisp_indent);
                            }
                        } else {
                            op_reindent(
                                oap,
                                if curbuf_mut().b_p_inde.cb_type() != kCallbackNone {
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
                    check_cursor_col(curwin_mut());
                }
                OP_FORMAT => {
                    if curbuf_mut().b_p_fex.cb_type() != kCallbackNone {
                        op_formatexpr(oap);
                    } else if !crate::memory::cstr_is_empty(opt_vars().p_fp) || !crate::memory::cstr_is_empty(curbuf_mut().b_p_fp) {
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
                    g_st().Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        let restart_edit_save = g_st().restart_edit;
                        g_st().restart_edit = 0;

                        restore_lbr(lbr_saved);

                        curbuf_mut().b_last_changedtick_i = buf_get_changedtick(curbuf_mut());

                        let mut session: InsSession = atom_ins_start(
                            b'I' as c_int,
                            c_long::from(cap.count1),
                            op_ins_visual(oap, cap),
                            oap.motion_type == kMTBlockWise,
                        );
                        op_insert(oap, cap.count1);
                        atom_ins_end(&mut session, false);

                        reset_lbr();
                        auto_format(false, true);

                        if g_st().restart_edit == 0 {
                            g_st().restart_edit = restart_edit_save;
                        } else {
                            cap.retval |= CA_COMMAND_BUSY;
                        }
                    }
                }
                OP_REPLACE => {
                    g_st().Visual.reselect = 0;
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        restore_lbr(lbr_saved);
                        op_replace(oap, cap.nchar);
                    }
                }
                OP_FOLD => {
                    g_st().Visual.reselect = 0;
                    foldCreate(curwin_mut(), oap.start, oap.end);
                }
                OP_MCURSOR => {
                    restore_lbr(lbr_saved);
                    mc_zq(oap, cap, op_origin);
                }
                OP_FOLDOPEN | OP_FOLDOPENREC | OP_FOLDCLOSE | OP_FOLDCLOSEREC => {
                    g_st().Visual.reselect = 0;
                    opFoldRange(
                        oap.start,
                        oap.end,
                        c_int::from(
                            oap.op_type == OP_FOLDOPEN || oap.op_type == OP_FOLDOPENREC,
                        ),
                        c_int::from(
                            oap.op_type == OP_FOLDOPENREC || oap.op_type == OP_FOLDCLOSEREC,
                        ),
                        oap.is_VIsual,
                    );
                }
                OP_FOLDDEL | OP_FOLDDELREC => {
                    g_st().Visual.reselect = 0;
                    deleteFold(
                        curwin_mut(),
                        oap.start.lnum,
                        oap.end.lnum,
                        c_int::from(oap.op_type == OP_FOLDDELREC),
                        oap.is_VIsual,
                    );
                }
                OP_NR_ADD | OP_NR_SUB => {
                    if empty_region_error {
                        vim_beep(kOptBoFlagOperator);
                        redo_cancel();
                    } else {
                        g_st().Visual.active = true;
                        restore_lbr(lbr_saved);
                        op_addsub(oap, cap.count1, cap.arg != 0);
                        g_st().Visual.active = false;
                    }
                    check_cursor_col(curwin_mut());
                }
                _ => {
                    clearopbeep(oap);
                }
            }
            g_st().virtual_op = kNone;
            if !gui_yank {
                if opt_vars().p_sol == 0
                    && oap.motion_type == kMTLineWise
                    && !oap.end_adjusted
                    && (oap.op_type == OP_LSHIFT
                        || oap.op_type == OP_RSHIFT
                        || oap.op_type == OP_DELETE)
                {
                    reset_lbr();
                    curwin_mut().w_curswant = old_col;
                    coladvance(curwin_mut(), old_col);
                }
            } else {
                curwin_mut().w_cursor = old_cursor;
            }
            clearop(oap);
            g_st().motion_force = c_int::from(NUL);
        }
        restore_lbr(lbr_saved);
    }
}

/// Get the byte count of buffer region. End-exclusive.
pub fn get_region_bytecount(
    buf: &mut FileBuf,
    start_lnum: Linenr,
    end_lnum: Linenr,
    start_col: Colnr,
    end_col: Colnr,
) -> Bcount {
    let max_lnum = buf.b_ml.ml_line_count;
        if start_lnum > max_lnum {
            return 0;
        }
        if start_lnum == end_lnum {
            return (end_col - start_col) as Bcount;
        }
        let mut deleted_bytes = (ml_get_buf_len(buf, start_lnum) - start_col + 1) as Bcount;

        for i in 1..=(end_lnum - start_lnum - 1) {
            if start_lnum + i > max_lnum {
                return deleted_bytes;
            }
            deleted_bytes += (ml_get_buf_len(buf, start_lnum + i) + 1) as Bcount;
        }
        if end_lnum > max_lnum {
            return deleted_bytes;
        }
        deleted_bytes + (end_col as Bcount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_char_and_rot13_helpers() {
        assert_eq!(get_op_char(OP_UPPER), b'g' as c_int);
            assert_eq!(get_extra_op_char(OP_UPPER), b'U' as c_int);
            assert_ne!(op_on_lines(OP_INDENT), 0);
            assert_eq!(op_on_lines(OP_DELETE), 0);
            assert!(op_redoable(OP_DELETE, false));
            assert!(!op_redoable(OP_YANK, false));
            assert!(op_redoable(OP_YANK, true));
        assert_eq!(rot13(b'a' as c_int, b'a' as c_int), b'n' as c_int);
        assert_eq!(rot13(b'n' as c_int, b'a' as c_int), b'a' as c_int);
        assert_eq!(char_ord(b'C' as c_int), 2);
        assert_eq!(char_ord(b'c' as c_int), 2);
    }
}
