#![forbid(unsafe_code)]

//! Implementation of various operators: `op_shift`, `op_delete`, `op_tilde`,
//! `op_change`, `op_yank`, `do_join` (`src/nvim/ops.c`).

use crate::buffer::{FileBuf, buf_get_changedtick, col_print, modifiable};
use crate::change::{
    COM_END, K_CPO_DOLLAR, K_OPT_VE_FLAG_ONEMORE, appended_lines_mark, changed_bytes,
    changed_lines, del_bytes, del_char, del_lines, get_last_leader_offset, get_leader_len,
    ins_char, ins_str, truncate_line,
};
use crate::charset::{
    NUL, STR2NR_BIN, STR2NR_HEX, STR2NR_OCT, Uvarnumber, ascii_isalpha, ascii_isbdigit,
    ascii_isdigit, ascii_isspace, ascii_isupper, ascii_iswhite, ascii_isxdigit, getwhitecols,
    getwhitecols_curline, vim_str2nr,
};
use crate::clipboard::set_clipboard;
use crate::cursor::{
    Colnr, CursorPos, K_OPT_VE_FLAG_ALL, Linenr, VALID_VIRTCOL, check_cursor, check_cursor_col,
    check_pos, coladvance, coladvance_force, dec_cursor, equalpos, gchar_cursor,
    get_cursor_line_len, get_cursor_line_ptr, getviscol, getviscol2, getvpos, inc_cursor, lt,
    ltoreq,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later, update_screen};
use crate::errors::*;
use crate::eval::typval::{
    K_CALLBACK_NONE_CONST, TvDict, TypVal, TypvalVval, VarType, Varnumber, callback_free, tv_clear,
    tv_dict_add_nr,
};
use crate::eval::{callback_call, set_ref_in_callback};
use crate::ex_docmd::{CMOD_LOCKMARKS, do_cmdline};
use crate::ex_getln::{CTRL_A, K_LUA};
use crate::extmark::{
    Bcount, K_EXTMARK_UNDO, K_NONE, TriState, extmark_splice, extmark_splice_cols,
};
use crate::fold::{delete_fold, fold_create, fold_open_cursor, has_folding, op_fold_range};
use crate::globals::*;
use crate::indent::{
    INDENT_SET, SIN_CHANGED, change_indent, fix_indent, get_expr_indent, get_indent,
    get_lisp_indent, get_sw_value_indent, inindent, may_do_si, op_reindent, preprocs_left,
    set_indent, tabstop_fromto, use_indentexpr_for_lisp,
};
use crate::indent_c::get_c_indent;
use crate::input::{
    RedoState, beep_flush, redo_append_str, redo_cancel, restore_redobuff, save_redobuff,
    stuff_readbuff, stuffchar_readbuff, stuffnum_readbuff,
};
use crate::input_cmdatom::{
    Cmdarg, InsSession, K_VINS_KEYS, K_VINS_MOTION, K_VINS_NONE, K_VINS_OTHER, VisualIns,
    atom_capture_op, atom_ins_end, atom_ins_start, atom_suppress, atom_visual_redoable,
};
use crate::insert::{CTRL_X_CONST, K_FO_AUTO, beginline, display_dollar, edit, linetabsize_str};
use crate::keycodes::{CAR, K_COMMAND, NL, TAB};
use crate::mark::{mark_col_adjust, mark_mb_adjustpos};
use crate::math::trim_to_int;
use crate::mbyte::{
    bomb_size, mb_islower, mb_isupper, mb_tolower, mb_toupper, utf_char2cells, utf_char2len,
    utf_char2slice, utf_cluster_info, utf_eat_space, utf_ptr2_str_char_info, utfc_ptr2len,
};
use crate::mcursor::mc_zq;
use crate::memline::{
    dec, decl, gchar_pos, inc, ml_append, ml_get, ml_get_buf_len, ml_get_len, ml_get_pos_len,
    ml_replace, ml_replace_len,
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
    BlockDef, YankRegMode, do_autocmd_textyankpost, get_y_register, get_yank_register,
    is_append_register, op_yank, op_yank_reg, shift_delete_registers, valid_yank_reg,
};
use crate::state::{MODE_INSERT, MODE_REPLACE, VREPLACE_FLAG, virtual_active};
use crate::strings::{format_to_buf, ngettext, tr};
use crate::textformat::{Oparg, auto_format, has_format_option, op_format, op_formatexpr};
use crate::ui::vim_beep;
use crate::undo::{u_clearline, u_save, u_save_cursor};

use crate::arglist::ML_EMPTY;
use crate::diff::K_OPT_BO_FLAG_OPERATOR;
use crate::drawline::{VALID_WCOL, VALID_WROW};
use crate::eval::funcs::{MT_BLOCK_WISE, MT_CHAR_WISE, MT_LINE_WISE};
use crate::ex_cmds::{BL_FIX, BL_SOL, BL_WHITE};
use crate::fileio::EOL_DOS;
use crate::regexp::{CTRL_V_CONST, MAXCOL, NUMBUFLEN};

pub const OP_NOP: i32 = 0;
pub const OP_DELETE: i32 = 1;
pub const OP_YANK: i32 = 2;
pub const OP_CHANGE: i32 = 3;
pub const OP_LSHIFT: i32 = 4;
pub const OP_RSHIFT: i32 = 5;
pub const OP_FILTER: i32 = 6;
pub const OP_TILDE: i32 = 7;
pub const OP_INDENT: i32 = 8;
pub const OP_FORMAT: i32 = 9;
pub const OP_COLON: i32 = 10;
pub const OP_UPPER: i32 = 11;
pub const OP_LOWER: i32 = 12;
pub const OP_JOIN: i32 = 13;
pub const OP_JOIN_NS: i32 = 14;
pub const OP_ROT13: i32 = 15;
pub const OP_REPLACE: i32 = 16;
pub const OP_INSERT: i32 = 17;
pub const OP_APPEND: i32 = 18;
pub const OP_FOLD: i32 = 19;
pub const OP_FOLDOPEN: i32 = 20;
pub const OP_FOLDOPENREC: i32 = 21;
pub const OP_FOLDCLOSE: i32 = 22;
pub const OP_FOLDCLOSEREC: i32 = 23;
pub const OP_FOLDDEL: i32 = 24;
pub const OP_FOLDDELREC: i32 = 25;
pub const OP_FORMAT2: i32 = 26;
pub const OP_FUNCTION: i32 = 27;
pub const OP_NR_ADD: i32 = 28;
pub const OP_NR_SUB: i32 = 29;
pub const OP_MCURSOR: i32 = 30;

const OPF_LINES: u8 = 1;
const OPF_CHANGE: u8 = 2;

const K_CPO_EMPTYREGION: i32 = b'E' as i32;
const K_CPO_FILTER: i32 = b'!' as i32;
const K_CPO_JOINCOL: i32 = b'q' as i32;
const K_CPO_YANK: i32 = b'y' as i32;

const K_FO_MBYTE_JOIN: i32 = b'M' as i32;
const K_FO_MBYTE_JOIN2: i32 = b'B' as i32;
const K_FO_REMOVE_COMS: i32 = b'j' as i32;

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
    [CTRL_A as u8, 0, OPF_CHANGE],        // OP_NR_ADD
    [CTRL_X_CONST as u8, 0, OPF_CHANGE],  // OP_NR_SUB
    [b'z', b'q', 0],                      // OP_MCURSOR
];

std::thread_local! {
    static ADDSUB_HEXUPPER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
const fn char_ord(x: i32) -> i32 {
    let b = x as u8;
    if b < b'a' {
        (b - b'A') as i32
    } else {
        (b - b'a') as i32
    }
}

#[inline]
const fn rot13(c: i32, a: i32) -> i32 {
    (((c - a) + 13) % 26) + a
}

/// Translate a command name into an operator type.
pub fn get_op_type(char1: i32, char2: i32) -> i32 {
    if char1 == b'r' as i32 {
        return OP_REPLACE;
    }
    if char1 == b'~' as i32 {
        return OP_TILDE;
    }
    if char1 == b'g' as i32 && char2 == CTRL_A {
        return OP_NR_ADD;
    }
    if char1 == b'g' as i32 && char2 == CTRL_X_CONST {
        return OP_NR_SUB;
    }
    if char1 == b'z' as i32 && char2 == b'y' as i32 {
        return OP_YANK;
    }
    let mut i = 0;
    loop {
        if i32::from(OPCHARS[i][0]) == char1 && i32::from(OPCHARS[i][1]) == char2 {
            break;
        }
        if i == OPCHARS.len() - 1 {
            internal_error(c"get_op_type()");
            break;
        }
        i += 1;
    }
    i as i32
}

/// Return non-zero if operator `op` always works on whole lines.
pub fn op_on_lines(op: i32) -> i32 {
    i32::from(OPCHARS[op as usize][2] & OPF_LINES)
}

/// Return non-zero if operator `op` changes text.
pub fn op_is_change(op: i32) -> i32 {
    i32::from(OPCHARS[op as usize][2] & OPF_CHANGE)
}

/// Whether operator `op` builds a redo: text-changing operators do, ":" and the fold operators do
/// not. Yank with the 'y' flag in 'cpoptions'.
pub fn op_redoable(op: i32, redo_yank: bool) -> bool {
    if op == OP_YANK {
        redo_yank
    } else {
        op_is_change(op) != 0
    }
}

/// Get first operator command character.
pub fn get_op_char(optype: i32) -> i32 {
    i32::from(OPCHARS[optype as usize][0])
}

/// Get second operator command character.
pub fn get_extra_op_char(optype: i32) -> i32 {
    i32::from(OPCHARS[optype as usize][1])
}

/// Handle a shift operation.
pub fn op_shift(oap: &mut Oparg, curs_top: bool, amount: i32) {
    let mut block_col: i32 = 0;

    if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
        return;
    }

    if oap.motion_type == MT_BLOCK_WISE {
        block_col = curwin_mut().w_cursor.get().col;
    }

    let mut i = oap.line_count - 1;
    while i >= 0 {
        let first_char = i32::from(
            crate::cursor::get_cursor_line_slice()
                .first()
                .copied()
                .unwrap_or(0),
        );
        if first_char == 0 {
            curwin_mut().w_cursor.set_col(0);
        } else if oap.motion_type == MT_BLOCK_WISE {
            shift_block(oap, amount);
        } else if first_char != b'#' as i32 || !preprocs_left() {
            shift_line(oap.op_type == OP_LSHIFT, opt_vars().p_sr.get(), amount, 0);
        }
        curwin_mut().w_cursor.add_lnum(1);
        i -= 1;
    }

    if oap.motion_type == MT_BLOCK_WISE {
        curwin_mut().w_cursor.set_lnum(oap.start.lnum);
        curwin_mut().w_cursor.set_col(block_col);
    } else if curs_top {
        curwin_mut().w_cursor.set_lnum(oap.start.lnum);
        beginline(BL_SOL | BL_FIX);
    } else {
        curwin_mut().w_cursor.sub_lnum(1);
    }
    fold_open_cursor();

    if i64::from(oap.line_count) > opt_vars().p_report.get() {
        let op = if oap.op_type == OP_RSHIFT { c">" } else { c"<" };

        let msg_line_single = ngettext(
            c"%lld line %sed %d time",
            c"%lld line %sed %d times",
            amount as u64,
        );
        let msg_line_plural = ngettext(
            c"%lld lines %sed %d time",
            c"%lld lines %sed %d times",
            amount as u64,
        );
        format_to_buf!(
            &mut *g_st().iobuff.borrow_mut(),
            ngettext(msg_line_single, msg_line_plural, oap.line_count as u64),
            i64::from(oap.line_count),
            op,
            amount,
        );
        msg_keep(&g_st().iobuff_cstr(), 0, true, false);
    }

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.set(oap.start);
        curbuf_mut().b_op_end.set_lnum(oap.end.lnum);
        curbuf_mut().b_op_end.set_col(ml_get_len(oap.end.lnum));
        if curbuf_mut().b_op_end.get().col > 0 {
            curbuf_mut().b_op_end.sub_col(1);
        }
    }

    changed_lines(&curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
}

fn get_vts(vts_array: &[i32], index: i32) -> i32 {
    if index < 1 {
        0
    } else if (index as usize) <= vts_array.len() {
        vts_array[index as usize - 1]
    } else {
        *vts_array.last().unwrap()
    }
}

fn get_vts_sum(vts_array: &[i32], index: i32) -> i32 {
    let mut sum: i32 = 0;
    let mut i: i32 = 1;
    let len = vts_array.len() as i32;
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

fn get_new_vts_indent(left: bool, round: bool, amount: i32, vts_array: &[i32]) -> i64 {
    let mut indent = i64::from(get_indent());
    let mut vtsi: i32 = 0;
    let mut vts_indent: i32 = 0;
    let mut ts: i32 = 0;

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
pub fn shift_line(left: bool, round: bool, amount: i32, call_changed_bytes: i32) {
    let sw_val = curbuf_mut().b_p_sw.get();
    let ts_val = curbuf_mut().b_p_ts.get();
    let cb = curbuf_mut();
    let vts_array = &cb.b_p_vts_array.borrow();

    let count = if sw_val != 0 {
        get_new_sw_indent(left, round, i64::from(amount), sw_val)
    } else if vts_array.is_empty() {
        get_new_sw_indent(left, round, i64::from(amount), ts_val)
    } else {
        get_new_vts_indent(left, round, amount, vts_array)
    };

    if (g_st().state_global.get() & VREPLACE_FLAG) != 0 {
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

fn shift_block(oap: &mut Oparg, amount: i32) {
    let left = oap.op_type == OP_LSHIFT;
    let oldstate = g_st().state_global.get();
    let oldcol = curwin_mut().w_cursor.get().col;
    let sw_val = get_sw_value_indent(&curbuf_mut(), left);
    let ts_val = curbuf_mut().b_p_ts.get() as i32;
    let mut bd = BlockDef::new();
    let old_p_ri = opt_vars().p_ri.get();

    opt_vars().p_ri.set(false);

    g_st().state_global.set(MODE_INSERT);
    block_prep(oap, &mut bd, curwin_mut().w_cursor.get().lnum, true);
    if bd.is_short {
        return;
    }

    let mut total = (amount as u32).wrapping_mul(sw_val as u32) as i32;
    if (total / sw_val) != amount {
        return;
    }

    let oldp_cstr = get_cursor_line_ptr();
    let oldp_with_nul = oldp_cstr.to_bytes_with_nul();
    let oldp_addr = oldp_cstr.as_ptr() as isize;
    let old_line_len = get_cursor_line_len();

    let mut newp_vec: Vec<u8>;
    let startcol: i32;
    let oldlen: i32;
    let newlen: i32;

    if !left {
        total += bd.pre_whitesp;
        let mut ws_vcol = bd.start_vcol - bd.pre_whitesp;
        let old_textstart = bd.textstart.as_ptr();
        if bd.startspaces != 0 {
            if crate::mbyte::utfc_slice2len(bd.textstart.to_bytes()) == 1 {
                bd.textstart = bd.textstart.slice(1);
            } else {
                ws_vcol = 0;
                bd.startspaces = 0;
            }
        }

        let mut csarg = CharsizeArg::new();
        let cstype: CSType = init_charsize_arg(
            &mut csarg,
            &curwin_mut(),
            curwin_mut().w_cursor.get().lnum,
            &bd.textstart,
        );
        let textstart_rc = bd.textstart.clone();
        let mut ci = utf_ptr2_str_char_info(&textstart_rc);
        let mut vcol = bd.start_vcol;
        while ascii_iswhite(ci.chr.value) {
            let cli = utf_cluster_info(ci);
            let incr =
                win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells).width;
            ci = cli.next;
            total += incr;
            vcol += incr;
        }
        bd.textstart = textstart_rc.slice(ci.offset_from(&textstart_rc));
        bd.start_vcol = vcol;

        let mut tabs: i32 = 0;
        let mut spaces: i32 = 0;
        if !curbuf_mut().b_p_et.get() {
            tabstop_fromto(
                ws_vcol,
                ws_vcol + total,
                ts_val,
                &curbuf_mut().b_p_vts_array.borrow(),
                &mut tabs,
                &mut spaces,
            );
        } else {
            spaces = total;
        }

        let col_pre = bd.pre_whitesp_c - i32::from(bd.startspaces != 0);
        bd.textcol -= col_pre;

        let new_line_len = bd.textcol
            + tabs
            + spaces
            + (old_line_len - (((bd.textstart.as_ptr() as isize) - oldp_addr) as i32));
        let textcol = bd.textcol as usize;
        let tabs_len = tabs as usize;
        let spaces_len = spaces as usize;
        let prefix_len = textcol + tabs_len + spaces_len;
        newp_vec = vec![0u8; (new_line_len as usize) + 1];
        newp_vec[..textcol].copy_from_slice(&oldp_with_nul[..textcol]);
        startcol = bd.textcol;
        oldlen = (((bd.textstart.as_ptr() as isize) - (old_textstart as isize)) as i32) + col_pre;
        newlen = tabs + spaces;
        newp_vec[textcol..textcol + tabs_len].fill(b'\t');
        newp_vec[textcol + tabs_len..prefix_len].fill(b' ');
        let tail = bd.textstart.to_bytes();
        newp_vec[prefix_len..prefix_len + tail.len()].copy_from_slice(tail);
        debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
    } else {
        let mut non_white_cstr = bd.textstart.clone();

        if bd.startspaces != 0 {
            non_white_cstr = non_white_cstr.slice(utfc_ptr2len(non_white_cstr.to_bytes()) as usize);
        }

        let mut non_white_col = bd.start_vcol;

        let mut csarg = CharsizeArg::new();
        let mut cstype: CSType = init_charsize_arg(
            &mut csarg,
            &curwin_mut(),
            curwin_mut().w_cursor.get().lnum,
            &bd.textstart,
        );
        while let Some(&b0) = non_white_cstr.to_bytes().first()
            && ascii_iswhite(i32::from(b0))
        {
            let incr = win_charsize(
                cstype,
                non_white_col,
                &non_white_cstr.to_bytes()[..1],
                i32::from(b0),
                &mut csarg,
                1,
            )
            .width;
            non_white_col += incr;
            non_white_cstr = non_white_cstr.slice(1);
        }

        let block_space_width = non_white_col - oap.start_vcol;
        let shift_amount = block_space_width.min(total);
        let destination_col = non_white_col - shift_amount;

        let mut verbatim_copy_width = bd.start_vcol;

        if bd.startspaces != 0 {
            verbatim_copy_width -= bd.start_char_vcols;
        }
        cstype = init_charsize_arg(&mut csarg, &curwin_mut(), 0, &bd.textstart);
        let mut ci = utf_ptr2_str_char_info(&bd.textstart);
        while verbatim_copy_width < destination_col {
            let cli = utf_cluster_info(ci);
            let incr = win_charsize(
                cstype,
                verbatim_copy_width,
                ci.cstr(),
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
        let verbatim_copy_end =
            crate::memory::CStrSubslice::advance(&bd.textstart, ci.offset_from(&bd.textstart));

        debug_assert!(destination_col - verbatim_copy_width >= 0);
        let fill = destination_col - verbatim_copy_width;

        debug_assert!((verbatim_copy_end.as_ptr() as isize) - oldp_addr >= 0);
        let fixedlen = ((verbatim_copy_end.as_ptr() as isize) - oldp_addr) as i32;
        let new_line_len = fixedlen
            + fill
            + (old_line_len - (((non_white_cstr.as_ptr() as isize) - oldp_addr) as i32));

        let fixed_usize = fixedlen as usize;
        let fill_usize = fill as usize;
        let prefix_len = fixed_usize + fill_usize;
        newp_vec = vec![0u8; (new_line_len as usize) + 1];
        startcol = fixedlen;
        oldlen = bd.textcol
            + (((non_white_cstr.as_ptr() as isize) - (bd.textstart.as_ptr() as isize)) as i32)
            - fixedlen;
        newlen = fill;
        newp_vec[..fixed_usize].copy_from_slice(&oldp_with_nul[..fixed_usize]);
        newp_vec[fixed_usize..prefix_len].fill(b' ');
        let tail = non_white_cstr.to_bytes();
        newp_vec[prefix_len..prefix_len + tail.len()].copy_from_slice(tail);
        debug_assert_eq!(newlen - oldlen, new_line_len - old_line_len);
    }
    ml_replace(
        curwin_mut().w_cursor.get().lnum,
        crate::memory::bytes_until_nul(&newp_vec),
        true,
    );
    drop(newp_vec);
    changed_bytes(curwin_mut().w_cursor.get().lnum, bd.textcol);
    extmark_splice_cols(
        &curbuf_mut(),
        curwin_mut().w_cursor.get().lnum - 1,
        startcol,
        oldlen,
        newlen,
        K_EXTMARK_UNDO,
    );
    g_st().state_global.set(oldstate);
    curwin_mut().w_cursor.set_col(oldcol);
    opt_vars().p_ri.set(old_p_ri);
}

fn block_insert(oap: &mut Oparg, s_slice: &[u8], b_insert: bool, bdp: &mut BlockDef) {
    let slen = s_slice.len();
    {
        let mut ts_val: i32;
        let mut count: i32 = 0;
        let mut spaces: i32;
        let mut offset: Colnr;
        let oldstate = g_st().state_global.get();
        g_st().state_global.set(MODE_INSERT);

        for lnum in (oap.start.lnum + 1)..=oap.end.lnum {
            block_prep(oap, bdp, lnum, true);
            if bdp.is_short && b_insert {
                continue;
            }

            let oldp_cstr = ml_get(lnum);
            let oldp_with_nul = oldp_cstr.to_bytes_with_nul();
            let oldp_bytes = &oldp_with_nul[..oldp_with_nul.len() - 1];

            if b_insert {
                ts_val = bdp.start_char_vcols;
                spaces = bdp.startspaces;
                if spaces != 0 {
                    count = ts_val - 1;
                }
                offset = bdp.textcol;
            } else {
                ts_val = bdp.end_char_vcols;
                if !bdp.is_short {
                    spaces = if bdp.endspaces != 0 {
                        ts_val - bdp.endspaces
                    } else {
                        0
                    };
                    if spaces != 0 {
                        count = ts_val - 1;
                    }
                    offset = bdp.textcol + bdp.textlen - i32::from(spaces != 0);
                } else {
                    if !bdp.is_max {
                        spaces = (oap.end_vcol - bdp.end_vcol) + 1;
                    } else {
                        spaces = 0;
                    }
                    count = spaces;
                    offset = bdp.textcol + bdp.textlen;
                }
            }

            if spaces > 0 {
                offset -= crate::mbyte::utf_head_off_slice(oldp_bytes, offset as usize);
            }
            spaces = spaces.max(0);

            debug_assert!(count >= 0);
            let alloc_len = (ml_get_len(lnum) as usize)
                + (spaces as usize)
                + slen
                + if spaces > 0 && !bdp.is_short {
                    (ts_val - spaces) as usize
                } else {
                    0
                }
                + (count as usize)
                + 1;
            let mut newp_vec = vec![0u8; alloc_len];
            let offset_usize = offset as usize;
            let spaces_usize = spaces as usize;

            newp_vec[..offset_usize].copy_from_slice(&oldp_with_nul[..offset_usize]);
            let mut oldp_rem = &oldp_bytes[offset_usize.min(oldp_bytes.len())..];
            let startcol = offset;

            newp_vec[offset_usize..offset_usize + spaces_usize].fill(b' ');
            newp_vec[offset_usize + spaces_usize..offset_usize + spaces_usize + slen]
                .copy_from_slice(s_slice);
            offset += slen as i32;

            let mut skipped: i32 = 0;
            if spaces > 0 && !bdp.is_short {
                if oldp_rem.first() == Some(&b'\t') {
                    let pad_start = (offset + spaces) as usize;
                    let pad_len = (ts_val - spaces) as usize;
                    newp_vec[pad_start..pad_start + pad_len].fill(b' ');
                    oldp_rem = &oldp_rem[1..];
                    count += 1;
                    skipped = 1;
                } else {
                    count = spaces;
                }
            }

            if spaces > 0 {
                offset += count;
            }
            let tail = oldp_rem;
            newp_vec[offset as usize..offset as usize + tail.len()].copy_from_slice(tail);

            ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
            drop(newp_vec);
            extmark_splice_cols(
                &curbuf_mut(),
                lnum - 1,
                startcol,
                skipped,
                offset - startcol,
                K_EXTMARK_UNDO,
            );

            if lnum == oap.end.lnum {
                curbuf_mut().b_op_end.set_lnum(oap.end.lnum);
                curbuf_mut().b_op_end.set_col(offset);
                if curbuf_mut().b_visual.borrow().vi_end.coladd != 0 {
                    let cb = curbuf_mut();
                    let mut vis = cb.b_visual.borrow_mut();
                    vis.vi_end.col += vis.vi_end.coladd;
                    vis.vi_end.coladd = 0;
                }
            }
        }

        g_st().state_global.set(oldstate);

        if oap.start.lnum < oap.end.lnum {
            changed_lines(
                &curbuf_mut(),
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
    let mut bd = BlockDef::new();
    let old_lcount = curbuf_mut().b_ml.ml_line_count.get();

    if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
        return true;
    }

    if oap.empty {
        return u_save_cursor();
    }

    if !modifiable(&curbuf_mut()) {
        emsg(tr(E_MODIFIABLE));
        return false;
    }

    if g_st().visual_global.borrow().select && oap.is_visual {
        oap.regname = g_st().visual_global.borrow().select_reg;
    }

    mb_adjust_opend(oap);

    if oap.motion_type == MT_CHAR_WISE
        && !oap.is_visual
        && oap.line_count > 1
        && oap.motion_force == i32::from(NUL)
        && oap.op_type == OP_DELETE
    {
        let line_bytes = crate::memline::ml_get_slice(oap.end.lnum);
        let mut ptr_line = line_bytes.get(oap.end.col as usize..).unwrap_or(&[]);
        if !ptr_line.is_empty() && oap.inclusive {
            ptr_line = &ptr_line[1..];
        }
        ptr_line = crate::charset::skipwhite_slice(ptr_line);
        if ptr_line.is_empty() && inindent(0) {
            oap.motion_type = MT_LINE_WISE;
        }
    }

    let mut only_setmarks = false;
    if oap.motion_type != MT_LINE_WISE
        && oap.line_count == 1
        && oap.op_type == OP_DELETE
        && crate::memline::ml_get_slice(oap.start.lnum).is_empty()
    {
        if g_st().virtual_op.get() != 0 {
            only_setmarks = true;
        } else {
            if crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_EMPTYREGION) {
                beep_flush();
            }
            return true;
        }
    }

    if !only_setmarks {
        if oap.regname != b'_' as i32 {
            let mut reg: Option<crate::register::RegIdx> = None;
            let mut did_yank = false;
            if oap.regname != 0 {
                if !valid_yank_reg(oap.regname, true) {
                    beep_flush();
                    return true;
                }
                let r = get_yank_register(oap.regname, YankRegMode::Yank);
                op_yank_reg(oap, false, r, is_append_register(oap.regname));
                reg = Some(r);
                did_yank = true;
            }

            if oap.motion_type == MT_LINE_WISE || oap.line_count > 1 || oap.use_reg_one {
                shift_delete_registers(is_append_register(oap.regname));
                let r = get_y_register(1);
                op_yank_reg(oap, false, r, false);
                reg = Some(r);
                did_yank = true;
            }

            if oap.regname == 0 && oap.motion_type != MT_LINE_WISE && oap.line_count == 1 {
                let r = get_yank_register(b'-' as i32, YankRegMode::Yank);
                op_yank_reg(oap, false, r, false);
                reg = Some(r);
                did_yank = true;
            }

            if did_yank || oap.regname == 0 {
                let Some(reg) = reg else {
                    std::process::abort();
                };
                set_clipboard(oap.regname, reg);
                do_autocmd_textyankpost(oap, reg);
            }
        }

        if oap.motion_type == MT_BLOCK_WISE {
            if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
                return false;
            }

            for lnum in curwin_mut().w_cursor.get().lnum..=oap.end.lnum {
                block_prep(oap, &mut bd, lnum, true);
                if bd.textlen == 0 {
                    continue;
                }

                if lnum == curwin_mut().w_cursor.get().lnum {
                    curwin_mut().w_cursor.set_col(bd.textcol + bd.startspaces);
                    curwin_mut().w_cursor.set_coladd(0);
                }

                let old_slice = crate::memline::ml_get_slice(lnum);
                let textcol = bd.textcol as usize;
                let pad_len = (bd.startspaces + bd.endspaces) as usize;
                let tail = &old_slice[((bd.textcol + bd.textlen) as usize).min(old_slice.len())..];
                let mut newp_vec = vec![0u8; textcol + pad_len + tail.len() + 1];
                newp_vec[..textcol].copy_from_slice(&old_slice[..textcol]);
                newp_vec[textcol..textcol + pad_len].fill(b' ');
                newp_vec[textcol + pad_len..textcol + pad_len + tail.len()].copy_from_slice(tail);
                ml_replace(lnum, crate::memory::bytes_until_nul(&newp_vec), true);
                drop(newp_vec);

                extmark_splice_cols(
                    &curbuf_mut(),
                    lnum - 1,
                    bd.textcol,
                    bd.textlen,
                    bd.startspaces + bd.endspaces,
                    K_EXTMARK_UNDO,
                );
            }

            check_cursor_col(&curwin_mut());
            changed_lines(
                &curbuf_mut(),
                curwin_mut().w_cursor.get().lnum,
                curwin_mut().w_cursor.get().col,
                oap.end.lnum + 1,
                0,
                true,
            );
            oap.line_count = 0;
        } else if oap.motion_type == MT_LINE_WISE {
            if oap.op_type == OP_CHANGE {
                if oap.line_count > 1 {
                    let lnum = curwin_mut().w_cursor.get().lnum;
                    curwin_mut().w_cursor.add_lnum(1);
                    del_lines(oap.line_count - 1, true);
                    curwin_mut().w_cursor.set_lnum(lnum);
                }
                if !u_save_cursor() {
                    return false;
                }
                if curbuf_mut().b_p_ai.get() {
                    beginline(BL_WHITE);
                    g_st().ins_global.borrow_mut().did_ai = true;
                    g_st().ins_global.borrow_mut().ai_col = curwin_mut().w_cursor.get().col;
                } else {
                    beginline(0);
                }
                truncate_line(0);
                if oap.line_count > 1 {
                    u_clearline(&curbuf_mut());
                }
            } else {
                del_lines(oap.line_count, true);
                beginline(BL_WHITE | BL_FIX);
                u_clearline(&curbuf_mut());
            }
        } else {
            if g_st().virtual_op.get() != 0 {
                if gchar_pos(&mut oap.start) == TAB {
                    let mut endcol: i32 = 0;
                    if !u_save_cursor() {
                        return false;
                    }
                    if oap.line_count == 1 {
                        endcol = getviscol2(oap.end.col, oap.end.coladd);
                    }
                    coladvance_force(getviscol2(oap.start.col, oap.start.coladd));
                    oap.start = curwin_mut().w_cursor.get();
                    if oap.line_count == 1 {
                        coladvance(&curwin_mut(), endcol);
                        oap.end.col = curwin_mut().w_cursor.get().col;
                        oap.end.coladd = curwin_mut().w_cursor.get().coladd;
                        curwin_mut().w_cursor.set(oap.start);
                    }
                }

                if gchar_pos(&mut oap.end) == TAB && oap.end.coladd == 0 && oap.inclusive {
                    if !u_save(oap.end.lnum - 1, oap.end.lnum + 1) {
                        return false;
                    }
                    curwin_mut().w_cursor.set(oap.end);
                    coladvance_force(getviscol2(oap.end.col, oap.end.coladd));
                    oap.end = curwin_mut().w_cursor.get();
                    curwin_mut().w_cursor.set(oap.start);
                }
                mb_adjust_opend(oap);
            }

            if oap.line_count == 1 {
                if !u_save_cursor() {
                    return false;
                }

                if crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_DOLLAR)
                    && oap.op_type == OP_CHANGE
                    && oap.end.lnum == curwin_mut().w_cursor.get().lnum
                    && !oap.is_visual
                {
                    display_dollar(oap.end.col - i32::from(!oap.inclusive));
                }

                let mut n = oap.end.col - oap.start.col + 1 - i32::from(!oap.inclusive);

                if g_st().virtual_op.get() != 0 {
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

                    if gchar_cursor() != i32::from(NUL) {
                        curwin_mut().w_cursor.set_coladd(0);
                    }
                }

                del_bytes(
                    n,
                    g_st().virtual_op.get() == 0,
                    oap.op_type == OP_DELETE && !oap.is_visual,
                );
            } else {
                if !u_save(
                    curwin_mut().w_cursor.get().lnum - 1,
                    curwin_mut().w_cursor.get().lnum + oap.line_count,
                ) {
                    return false;
                }

                g_st()
                    .curbuf_splice_pending
                    .set(g_st().curbuf_splice_pending.get() + (1));
                let startpos = curwin_mut().w_cursor.get();
                let deleted_bytes = get_region_bytecount(
                    &curbuf_mut(),
                    startpos.lnum,
                    oap.end.lnum,
                    startpos.col,
                    oap.end.col,
                ) + isize::from(oap.inclusive);
                truncate_line(1);

                let curpos = curwin_mut().w_cursor.get();
                curwin_mut().w_cursor.add_lnum(1);

                del_lines(oap.line_count - 2, false);

                let n = oap.end.col + 1 - i32::from(!oap.inclusive);
                curwin_mut().w_cursor.set_col(0);
                del_bytes(
                    n,
                    g_st().virtual_op.get() == 0,
                    oap.op_type == OP_DELETE && !oap.is_visual,
                );
                curwin_mut().w_cursor.set(curpos);
                do_join(2, false, false, false, false);
                g_st()
                    .curbuf_splice_pending
                    .set(g_st().curbuf_splice_pending.get() - (1));
                extmark_splice(
                    &curbuf_mut(),
                    startpos.lnum - 1,
                    startpos.col,
                    oap.line_count - 1,
                    n,
                    deleted_bytes,
                    0,
                    0,
                    0,
                    K_EXTMARK_UNDO,
                );
            }
            if oap.op_type == OP_DELETE {
                auto_format(false, true);
            }
        }

        msgmore(curbuf_mut().b_ml.ml_line_count.get() - old_lcount);
    }

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        if oap.motion_type == MT_BLOCK_WISE {
            curbuf_mut().b_op_end.set_lnum(oap.end.lnum);
            curbuf_mut().b_op_end.set_col(oap.start.col);
        } else {
            curbuf_mut().b_op_end.set(oap.start);
        }
        curbuf_mut().b_op_start.set(oap.start);
    }

    true
}

fn mb_adjust_opend(oap: &mut Oparg) {
    if !oap.inclusive {
        return;
    }

    let line_slice = crate::memline::ml_get_slice(oap.end.lnum);
    let end_col = oap.end.col as usize;
    if end_col < line_slice.len() {
        let head_off = crate::mbyte::utf_head_off_slice(&line_slice, end_col) as usize;
        let base_col = end_col.saturating_sub(head_off);
        let clen = (utfc_ptr2len(&line_slice[base_col..]) - 1).max(0) as usize;
        oap.end.col = (base_col + clen) as Colnr;
    }
}

fn pbyte(mut lp: CursorPos, c: i32) {
    debug_assert!(c <= u8::MAX as i32);
    crate::memline::with_ml_get_buf_mut_slice(&curbuf_mut(), lp.lnum, |slice| {
        let len = curbuf_mut().b_ml.ml_line_textlen.get();

        if lp.col >= len {
            lp.col = if len > 1 { len - 2 } else { 0 };
        }

        if let Some(slot) = slice.get_mut(lp.col as usize) {
            *slot = c as u8;
        }
    });

    if g_st().curbuf_splice_pending.get() == 0 {
        extmark_splice_cols(&curbuf_mut(), lp.lnum - 1, lp.col, 1, 1, K_EXTMARK_UNDO);
    }
}

fn replace_character(c: i32) {
    let n = g_st().state_global.get();

    g_st().state_global.set(MODE_REPLACE);
    ins_char(c);
    g_st().state_global.set(n);
    dec_cursor();
}

fn op_replace(oap: &mut Oparg, mut c: i32) -> bool {
    let mut n: i32;
    let mut bd = BlockDef::new();
    let mut after_p: Vec<u8> = Vec::new();
    let mut had_ctrl_v_cr = false;

    if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 || oap.empty {
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

    if oap.motion_type == MT_BLOCK_WISE {
        bd.is_max = curwin_mut().w_curswant.get() == MAXCOL;
        while curwin_mut().w_cursor.get().lnum <= oap.end.lnum {
            curwin_mut().w_cursor.set_col(0);
            block_prep(oap, &mut bd, curwin_mut().w_cursor.get().lnum, true);
            if bd.textlen == 0 && (g_st().virtual_op.get() == 0 || bd.is_max) {
                curwin_mut().w_cursor.add_lnum(1);
                continue;
            }

            if g_st().virtual_op.get() != 0 && bd.is_short && bd.textstart.to_bytes().is_empty() {
                let mut vpos = CursorPos {
                    lnum: curwin_mut().w_cursor.get().lnum,
                    col: 0,
                    coladd: 0,
                };
                getvpos(&curwin_mut(), &mut vpos, oap.start_vcol);
                bd.startspaces += vpos.coladd;
                n = bd.startspaces;
            } else {
                n = if bd.startspaces != 0 {
                    bd.start_char_vcols - 1
                } else {
                    0
                };
            }

            n += if bd.endspaces != 0 && !bd.is_one_char && bd.end_char_vcols > 0 {
                bd.end_char_vcols - 1
            } else {
                0
            };
            let mut numc = oap.end_vcol - oap.start_vcol + 1;
            if bd.is_short && (g_st().virtual_op.get() == 0 || bd.is_max) {
                numc -= (oap.end_vcol - bd.end_vcol) + 1;
            }

            if utf_char2cells(c) > 1 {
                if (numc & 1) != 0 && !bd.is_short {
                    bd.endspaces += 1;
                    n += 1;
                }
                numc /= 2;
            }
            let _ = n;

            let mut num_chars = numc;
            numc *= utf_char2len(c);

            let oldp_cstr = get_cursor_line_ptr();
            let oldp_with_nul = oldp_cstr.to_bytes_with_nul();
            let oldlen = get_cursor_line_len();

            let mut newp_size = (bd.textcol as usize) + (bd.startspaces as usize);
            if had_ctrl_v_cr || (c != b'\r' as i32 && c != b'\n' as i32) {
                newp_size += numc as usize;
                if !bd.is_short {
                    newp_size += (bd.endspaces + oldlen - bd.textcol - bd.textlen) as usize;
                }
            }
            let mut newp_vec = vec![0u8; newp_size + 1];
            let textcol = bd.textcol as usize;
            let startspaces = bd.startspaces as usize;
            newp_vec[..textcol].copy_from_slice(&oldp_with_nul[..textcol]);
            let tail_off = ((bd.textcol + bd.textlen) as usize).min(oldp_with_nul.len());
            let oldp_tail = &oldp_with_nul[tail_off..];
            newp_vec[textcol..textcol + startspaces].fill(b' ');
            let col = oldlen - bd.textcol - bd.textlen + 1;
            debug_assert!(col >= 0);
            let col_u = (col as usize).min(oldp_tail.len());
            let mut newrows: i32 = 0;
            let mut newcols: i32 = 0;
            if had_ctrl_v_cr || (c != b'\r' as i32 && c != b'\n' as i32) {
                let mut newp_len = textcol + startspaces;
                while num_chars > 0 {
                    num_chars -= 1;
                    newp_len += utf_char2slice(c, &mut newp_vec[newp_len..]);
                }
                if !bd.is_short {
                    let endspaces = bd.endspaces as usize;
                    newp_vec[newp_len..newp_len + endspaces].fill(b' ');
                    newp_len += endspaces;
                    newp_vec[newp_len..newp_len + col_u].copy_from_slice(&oldp_tail[..col_u]);
                }
                newcols = (newp_len as i32) - bd.textcol;
            } else {
                after_p.clear();
                after_p.extend_from_slice(&oldp_tail[..col_u]);
                newrows = 1;
            }
            ml_replace(
                curwin_mut().w_cursor.get().lnum,
                crate::memory::bytes_until_nul(&newp_vec),
                true,
            );
            drop(newp_vec);
            g_st()
                .curbuf_splice_pending
                .set(g_st().curbuf_splice_pending.get() + (1));
            let baselnum = curwin_mut().w_cursor.get().lnum;
            if !after_p.is_empty() {
                let cur_lnum = curwin_mut().w_cursor.get().lnum;
                curwin_mut().w_cursor.add_lnum(1);
                let after_len = after_p.len();
                ml_append(
                    cur_lnum,
                    &after_p[..after_len.saturating_sub(1)],
                    after_len as i32,
                    false,
                );
                appended_lines_mark(curwin_mut().w_cursor.get().lnum, 1);
                oap.end.lnum += 1;
                after_p.clear();
            }
            g_st()
                .curbuf_splice_pending
                .set(g_st().curbuf_splice_pending.get() - (1));
            extmark_splice(
                &curbuf_mut(),
                baselnum - 1,
                bd.textcol,
                0,
                bd.textlen,
                bd.textlen as isize,
                newrows,
                newcols,
                (newrows + newcols) as isize,
                K_EXTMARK_UNDO,
            );
            curwin_mut().w_cursor.add_lnum(1);
        }
    } else {
        if oap.motion_type == MT_LINE_WISE {
            oap.start.col = 0;
            curwin_mut().w_cursor.set_col(0);
            oap.end.col = ml_get_len(oap.end.lnum);
            if oap.end.col != 0 {
                oap.end.col -= 1;
            }
        } else if !oap.inclusive {
            dec(&mut oap.end);
        }

        while ltoreq(curwin_mut().w_cursor.get(), oap.end) {
            let mut done = false;

            n = gchar_cursor();
            if n != i32::from(NUL) {
                let new_byte_len = utf_char2len(c);
                let old_byte_len =
                    crate::mbyte::utfc_slice2len(&crate::cursor::get_cursor_pos_slice()) as i32;

                if new_byte_len > 1 || old_byte_len > 1 {
                    if curwin_mut().w_cursor.get().lnum == oap.end.lnum {
                        oap.end.col += new_byte_len - old_byte_len;
                    }
                    replace_character(c);
                    done = true;
                } else {
                    if n == TAB {
                        let mut end_vcol: i32 = 0;

                        if curwin_mut().w_cursor.get().lnum == oap.end.lnum {
                            end_vcol = getviscol2(oap.end.col, oap.end.coladd);
                        }
                        coladvance_force(getviscol());
                        if curwin_mut().w_cursor.get().lnum == oap.end.lnum {
                            getvpos(&curwin_mut(), &mut oap.end, end_vcol);
                        }
                    }
                    if gchar_cursor() != i32::from(NUL) {
                        pbyte(curwin_mut().w_cursor.get(), c);
                        done = true;
                    }
                }
            }
            if !done
                && g_st().virtual_op.get() != 0
                && curwin_mut().w_cursor.get().lnum == oap.end.lnum
            {
                let mut virtcols = oap.end.coladd;

                if curwin_mut().w_cursor.get().lnum == oap.start.lnum
                    && oap.start.col == oap.end.col
                    && oap.start.coladd != 0
                {
                    virtcols -= oap.start.coladd;
                }

                coladvance_force(getviscol2(oap.end.col, oap.end.coladd) + 1);
                curwin_mut().w_cursor.sub_col(virtcols + 1);
                while virtcols >= 0 {
                    if utf_char2len(c) > 1 {
                        replace_character(c);
                    } else {
                        pbyte(curwin_mut().w_cursor.get(), c);
                    }
                    if inc_cursor() == -1 {
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

    curwin_mut().w_cursor.set(oap.start);
    check_cursor(&curwin_mut());
    changed_lines(
        &curbuf_mut(),
        oap.start.lnum,
        oap.start.col,
        oap.end.lnum + 1,
        0,
        true,
    );

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.set(oap.start);
        curbuf_mut().b_op_end.set(oap.end);
    }

    true
}

/// Handle the (non-standard vi) tilde operator. Also for "gu", "gU" and "g?".
pub fn op_tilde(oap: &mut Oparg) {
    let mut bd = BlockDef::new();
    let mut did_change = false;

    if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
        return;
    }

    let mut pos = oap.start;
    if oap.motion_type == MT_BLOCK_WISE {
        while pos.lnum <= oap.end.lnum {
            block_prep(oap, &mut bd, pos.lnum, false);
            pos.col = bd.textcol;
            let one_change = swapchars(oap.op_type, &mut pos, bd.textlen) != 0;
            did_change |= one_change;
            pos.lnum += 1;
        }
        if did_change {
            changed_lines(&curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
        }
    } else {
        if oap.motion_type == MT_LINE_WISE {
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
                &curbuf_mut(),
                oap.start.lnum,
                oap.start.col,
                oap.end.lnum + 1,
                0,
                true,
            );
        }
    }

    if !did_change && oap.is_visual {
        redraw_curbuf_later(UPD_INVERTED);
    }

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.set(oap.start);
        curbuf_mut().b_op_end.set(oap.end);
    }

    if i64::from(oap.line_count) > opt_vars().p_report.get() {
        smsg!(
            0,
            ngettext(
                c"%lld line changed",
                c"%lld lines changed",
                oap.line_count as u64,
            ),
            i64::from(oap.line_count),
        );
    }
}

fn swapchars(op_type: i32, pos: &mut CursorPos, length: i32) -> i32 {
    let mut did_change: i32 = 0;
    let mut todo = length;
    while todo > 0 {
        let len = crate::mbyte::utfc_slice2len(&crate::memline::ml_get_pos_slice(pos)) as i32;

        if len > 0 {
            todo -= len - 1;
        }
        did_change |= i32::from(swapchar(op_type, pos));
        if inc(pos) == -1 {
            break;
        }
        todo -= 1;
    }
    did_change
}

pub fn swapchar(op_type: i32, pos: &mut CursorPos) -> bool {
    let c = gchar_pos(pos);

    if c >= 0x80 && op_type == OP_ROT13 {
        return false;
    }

    let mut nc = c;
    if mb_islower(c) {
        if op_type == OP_ROT13 {
            nc = rot13(c, b'a' as i32);
        } else if op_type != OP_LOWER {
            nc = mb_toupper(c);
        }
    } else if mb_isupper(c) {
        if op_type == OP_ROT13 {
            nc = rot13(c, b'A' as i32);
        } else if op_type != OP_UPPER {
            nc = mb_tolower(c);
        }
    }
    if nc != c {
        if c >= 0x80 || nc >= 0x80 {
            let sp = curwin_mut().w_cursor.get();

            curwin_mut().w_cursor.set(*pos);
            del_bytes(
                crate::mbyte::utf_slice2len(&crate::cursor::get_cursor_pos_slice()) as Colnr,
                false,
                false,
            );
            ins_char(nc);
            curwin_mut().w_cursor.set(sp);
        } else {
            pbyte(*pos, nc);
        }
        return true;
    }
    false
}

/// Insert and append operators for Visual mode.
pub fn op_insert(oap: &mut Oparg, count1: i32) {
    let mut pre_textlen: i32 = 0;
    let mut ind_pre_col: Colnr = 0;
    let mut ind_pre_vcol: i32 = 0;
    let mut bd = BlockDef::new();

    bd.is_max = curwin_mut().w_curswant.get() == MAXCOL;

    curwin_mut().w_cursor.set_lnum(oap.start.lnum);
    redraw_curbuf_later(UPD_INVERTED);
    update_screen();

    if oap.motion_type == MT_BLOCK_WISE {
        if curwin_mut().w_cursor.get().coladd > 0 {
            let old_ve_flags = curwin_mut().w_onebuf_opt.wo_ve_flags.get();

            if !u_save_cursor() {
                return;
            }
            curwin_mut().w_onebuf_opt.wo_ve_flags.set(K_OPT_VE_FLAG_ALL);
            coladvance_force(if oap.op_type == OP_APPEND {
                oap.end_vcol + 1
            } else {
                getviscol()
            });
            if oap.op_type == OP_APPEND {
                curwin_mut().w_cursor.sub_col(1);
            }
            curwin_mut().w_onebuf_opt.wo_ve_flags.set(old_ve_flags);
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
        if oap.motion_type == MT_BLOCK_WISE && curwin_mut().w_cursor.get().coladd == 0 {
            curwin_mut().w_set_curswant.set(true);
            while !crate::cursor::get_cursor_pos_slice().is_empty()
                && curwin_mut().w_cursor.get().col < bd.textcol + bd.textlen
            {
                curwin_mut().w_cursor.add_col(1);
            }
            if bd.is_short && !bd.is_max {
                if !u_save_cursor() {
                    return;
                }
                for _ in 0..bd.endspaces {
                    ins_char(b' ' as i32);
                }
                bd.textlen += bd.endspaces;
            }
        } else {
            curwin_mut().w_cursor.set(oap.end);
            check_cursor_col(&curwin_mut());

            if !lineempty(curwin_mut().w_cursor.get().lnum) && oap.start_vcol != oap.end_vcol {
                inc_cursor();
            }
        }
    }

    let t1 = oap.start;
    let start_insert = curwin_mut().w_cursor.get();
    edit(i32::from(NUL), false, count1);

    if t1.lnum == curbuf_mut().b_op_start_orig.get().lnum
        && lt(curbuf_mut().b_op_start_orig.get(), t1)
    {
        oap.start = curbuf_mut().b_op_start_orig.get();
    }

    if curwin_mut().w_cursor.get().lnum != oap.start.lnum || g_st().got_int.get() {
        return;
    }

    if oap.motion_type == MT_BLOCK_WISE {
        let mut ind_post_vcol: i32 = 0;
        let mut bd2 = BlockDef::new();
        let mut did_indent = false;

        let ind_post_col = getwhitecols_curline() as Colnr;
        if curbuf_mut().b_op_start.get().col > ind_pre_col && ind_post_col > ind_pre_col {
            bd.textcol += ind_post_col - ind_pre_col;
            ind_post_vcol = get_indent();
            bd.start_vcol += ind_post_vcol - ind_pre_vcol;
            did_indent = true;
        }

        if oap.start.lnum == curbuf_mut().b_op_start_orig.get().lnum && !bd.is_max && !did_indent {
            let t = getviscol2(
                curbuf_mut().b_op_start_orig.get().col,
                curbuf_mut().b_op_start_orig.get().coladd,
            );

            if oap.op_type == OP_INSERT
                && oap.start.col + oap.start.coladd
                    != curbuf_mut().b_op_start_orig.get().col
                        + curbuf_mut().b_op_start_orig.get().coladd
            {
                oap.start.col = curbuf_mut().b_op_start_orig.get().col;
                pre_textlen -= t - oap.start_vcol;
                oap.start_vcol = t;
            } else if oap.op_type == OP_APPEND
                && oap.start.col + oap.start.coladd
                    >= curbuf_mut().b_op_start_orig.get().col
                        + curbuf_mut().b_op_start_orig.get().coladd
            {
                oap.start.col = curbuf_mut().b_op_start_orig.get().col;
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
        if !bd.is_max || bd2.textlen < bd.textlen {
            if oap.op_type == OP_APPEND {
                pre_textlen += bd2.textlen - bd.textlen;
                if bd2.endspaces != 0 {
                    bd2.textlen -= 1;
                }
            }
            bd.textcol = bd2.textcol;
            bd.textlen = bd2.textlen;
        }

        let firstline_slice = crate::memline::ml_get_slice(oap.start.lnum);
        let mut len = ml_get_len(oap.start.lnum);
        let mut add = bd.textcol;
        let mut offset: Colnr = 0;
        if oap.op_type == OP_APPEND {
            add += bd.textlen;
            if bd.is_max
                && start_insert.lnum == g_st().ins_global.borrow().start.lnum
                && start_insert.col > g_st().ins_global.borrow().start.col
            {
                offset = start_insert.col - g_st().ins_global.borrow().start.col;
                add -= offset;
                if oap.end_vcol > offset {
                    oap.end_vcol -= offset + 1;
                } else {
                    return;
                }
            }
        }
        add = add.min(len);
        let add_u = (add.max(0) as usize).min(firstline_slice.len());
        let firstline_rem = &firstline_slice[add_u..];
        len -= add;
        let ins_len = len - pre_textlen - offset;
        if pre_textlen >= 0 && ins_len > 0 {
            let ins_u = (ins_len as usize).min(firstline_rem.len());
            let ins_text = firstline_rem[..ins_u].to_vec();
            if u_save(oap.start.lnum, oap.end.lnum + 1) {
                block_insert(oap, &ins_text, oap.op_type == OP_INSERT, &mut bd);
            }

            curwin_mut().w_cursor.set_col(oap.start.col);
            check_cursor(&curwin_mut());
        }
    }
}

/// Handle a change operation.
pub fn op_change(oap: &mut Oparg) -> i32 {
    let mut pre_textlen: i32 = 0;
    let mut pre_indent: i32 = 0;
    let mut bd = BlockDef::new();

    let mut l = oap.start.col;
    if oap.motion_type == MT_LINE_WISE {
        l = 0;
        g_st().ins_global.borrow_mut().can_si = may_do_si();
    }

    if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
        if !u_save_cursor() {
            return 0;
        }
    } else if !op_delete(oap) {
        return 0;
    }

    if l > curwin_mut().w_cursor.get().col
        && !lineempty(curwin_mut().w_cursor.get().lnum)
        && g_st().virtual_op.get() == 0
    {
        inc_cursor();
    }

    if oap.motion_type == MT_BLOCK_WISE {
        if g_st().virtual_op.get() != 0
            && (curwin_mut().w_cursor.get().coladd > 0 || gchar_cursor() == i32::from(NUL))
        {
            coladvance_force(getviscol());
        }
        let firstline_slice = crate::memline::ml_get_slice(oap.start.lnum);
        pre_textlen = firstline_slice.len() as Colnr;
        pre_indent = getwhitecols(&firstline_slice) as i32;
        bd.textcol = curwin_mut().w_cursor.get().col;
    }

    if oap.motion_type == MT_LINE_WISE {
        fix_indent();
    }

    let save_finish_op = g_st().finish_op.get();
    g_st().finish_op.set(false);

    let retval = i32::from(edit(i32::from(NUL), false, 1));

    g_st().finish_op.set(save_finish_op);

    if oap.motion_type == MT_BLOCK_WISE && oap.start.lnum != oap.end.lnum && !g_st().got_int.get() {
        let firstline_slice = crate::memline::ml_get_slice(oap.start.lnum);
        if bd.textcol > pre_indent {
            let new_indent = getwhitecols(&firstline_slice) as i32;
            pre_textlen += new_indent - pre_indent;
            bd.textcol += new_indent - pre_indent;
        }

        let ins_len = (firstline_slice.len() as Colnr) - pre_textlen;
        if ins_len > 0 {
            let ins_text = firstline_slice
                [bd.textcol as usize..bd.textcol as usize + ins_len as usize]
                .to_vec();
            for linenr in (oap.start.lnum + 1)..=oap.end.lnum {
                block_prep(oap, &mut bd, linenr, true);
                if !bd.is_short || g_st().virtual_op.get() != 0 {
                    let mut vpos = CursorPos {
                        lnum: linenr,
                        col: 0,
                        coladd: 0,
                    };
                    if bd.is_short {
                        getvpos(&curwin_mut(), &mut vpos, oap.start_vcol);
                    } else {
                        vpos.coladd = 0;
                    }
                    let old_slice = crate::memline::ml_get_slice(linenr);
                    let textcol = (bd.textcol as usize).min(old_slice.len());
                    let coladd = vpos.coladd as usize;
                    let ins_usize = ins_len as usize;
                    let tail = &old_slice[textcol..];
                    let mut newp_vec = vec![0u8; old_slice.len() + coladd + ins_usize + 1];
                    newp_vec[..textcol].copy_from_slice(&old_slice[..textcol]);
                    let mut newlen = textcol;
                    newp_vec[newlen..newlen + coladd].fill(b' ');
                    newlen += coladd;
                    newp_vec[newlen..newlen + ins_usize].copy_from_slice(&ins_text[..ins_usize]);
                    newlen += ins_usize;
                    newp_vec[newlen..newlen + tail.len()].copy_from_slice(tail);
                    ml_replace(linenr, crate::memory::bytes_until_nul(&newp_vec), true);
                    drop(newp_vec);
                    extmark_splice_cols(
                        &curbuf_mut(),
                        linenr - 1,
                        bd.textcol,
                        0,
                        vpos.coladd + ins_len,
                        K_EXTMARK_UNDO,
                    );
                }
            }
            check_cursor(&curwin_mut());
            changed_lines(
                &curbuf_mut(),
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

/// When the cursor is on the NUL past the end of the line and it should not be
/// there move it left.
pub fn adjust_cursor_eol() {
    let cur_ve_flags = get_ve_flags(&curwin_mut());

    let adj_cursor = curwin_mut().w_cursor.get().col > 0
        && gchar_cursor() == i32::from(NUL)
        && (cur_ve_flags & K_OPT_VE_FLAG_ONEMORE) == 0
        && (cur_ve_flags & K_OPT_VE_FLAG_ALL) == 0
        && !(g_st().restart_edit.get() != 0 || (g_st().state_global.get() & MODE_INSERT) != 0);
    if !adj_cursor {
        return;
    }

    dec_cursor();

    if cur_ve_flags == K_OPT_VE_FLAG_ALL {
        let mut scol: Colnr = 0;
        let mut ecol: Colnr = 0;
        let mut cursor = curwin_mut().w_cursor.get();

        getvcol(
            &curwin_mut(),
            &mut cursor,
            Some(&mut scol),
            None,
            Some(&mut ecol),
            0,
        );
        curwin_mut().w_cursor.set(cursor);
        curwin_mut().w_cursor.set_coladd(ecol - scol + 1);
    }
}

pub fn skip_comment<'a>(
    mut line: &'a [u8],
    process: bool,
    include_space: bool,
    is_comment: &mut bool,
) -> &'a [u8] {
    let mut comment_flags: Option<crate::option_vars::RcCStr> = None;
    let leader_offset = get_last_leader_offset(line, Some(&mut comment_flags));

    *is_comment = false;
    if leader_offset != -1
        && let Some(ref flags) = comment_flags
    {
        let end_idx = flags
            .to_bytes()
            .iter()
            .position(|&b| b == COM_END || b == b':');
        if end_idx.is_none_or(|i| flags.to_bytes()[i] != COM_END) {
            *is_comment = true;
        }
    }

    if !process {
        return line;
    }

    let lead_len = get_leader_len(line, Some(&mut comment_flags), false, include_space);

    if lead_len == 0 {
        return line;
    }

    let mut is_end = false;
    if let Some(flags) = comment_flags {
        let end_idx = flags
            .to_bytes()
            .iter()
            .position(|&b| b == COM_END || b == b':');
        if let Some(i) = end_idx
            && flags.to_bytes()[i] == COM_END
        {
            is_end = true;
        }
    }

    if !is_end {
        line = &line[(lead_len as usize).min(line.len())..];
    }

    line
}

pub fn do_join(
    count: usize,
    insert_space: bool,
    save_undo: bool,
    use_formatoptions: bool,
    setmark: bool,
) -> bool {
    let mut curr: &[u8] = &[];
    let mut curr_start: &[u8] = &[];
    let mut endcurr1: i32 = i32::from(NUL);
    let mut endcurr2: i32 = i32::from(NUL);
    let mut currsize: i32 = 0;
    let mut sumsize: i32 = 0;
    let mut ret = true;
    let remove_comments = use_formatoptions && has_format_option(K_FO_REMOVE_COMS);
    let mut prev_was_comment = false;
    debug_assert!(count >= 1);

    if save_undo
        && !u_save(
            curwin_mut().w_cursor.get().lnum - 1,
            curwin_mut().w_cursor.get().lnum + (count as Linenr),
        )
    {
        return false;
    }
    let mut spaces_vec = vec![0u8; count];
    let mut comments_vec = if remove_comments {
        vec![0_i32; count]
    } else {
        Vec::new()
    };

    let mut aborted = false;
    let mut curr_rc;
    for t in 0..(count as Linenr) {
        curr_rc = crate::memline::ml_get_slice(curwin_mut().w_cursor.get().lnum + t);
        curr_start = &curr_rc;
        curr = curr_start;
        if t == 0 && setmark && (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
            curwin_mut()
                .buf_mut()
                .b_op_start
                .set_lnum(curwin_mut().w_cursor.get().lnum);
            curwin_mut()
                .buf_mut()
                .b_op_start
                .set_col(curr.len() as Colnr);
        }
        if remove_comments {
            if t > 0 && prev_was_comment {
                let new_curr = skip_comment(curr, true, insert_space, &mut prev_was_comment);
                comments_vec[t as usize] = (curr.len() - new_curr.len()) as i32;
                curr = new_curr;
            } else {
                curr = skip_comment(curr, false, insert_space, &mut prev_was_comment);
            }
        }

        if insert_space && t > 0 {
            curr = crate::charset::skipwhite_slice(curr);
            if !curr.is_empty()
                && curr[0] != b')'
                && sumsize != 0
                && endcurr1 != TAB
                && (!has_format_option(K_FO_MBYTE_JOIN)
                    || (crate::mbyte::utf_slice2char(curr) < 0x100 && endcurr1 < 0x100))
                && (!has_format_option(K_FO_MBYTE_JOIN2)
                    || (crate::mbyte::utf_slice2char(curr) < 0x100 && !utf_eat_space(endcurr1))
                    || (endcurr1 < 0x100 && !utf_eat_space(crate::mbyte::utf_slice2char(curr))))
            {
                if endcurr1 == b' ' as i32 {
                    endcurr1 = endcurr2;
                } else {
                    spaces_vec[t as usize] += 1;
                }
                if opt_vars().p_js.get()
                    && (endcurr1 == b'.' as i32
                        || endcurr1 == b'?' as i32
                        || endcurr1 == b'!' as i32)
                {
                    spaces_vec[t as usize] += 1;
                }
            }
        }

        let space_t = i32::from(spaces_vec[t as usize]);
        if t > 0 && g_st().curbuf_splice_pending.get() == 0 {
            let removed = (curr_start.len() - curr.len()) as Colnr;
            extmark_splice(
                &curbuf_mut(),
                curwin_mut().w_cursor.get().lnum - 1,
                sumsize,
                1,
                removed,
                (removed + 1) as isize,
                0,
                space_t,
                space_t as isize,
                K_EXTMARK_UNDO,
            );
        }
        currsize = curr.len() as i32;
        sumsize += currsize + space_t;
        endcurr1 = i32::from(NUL);
        endcurr2 = i32::from(NUL);
        if insert_space && currsize > 0 {
            let mut cend_idx = curr.len();
            cend_idx -= (crate::mbyte::utf_head_off_slice(curr, cend_idx - 1) as usize) + 1;
            endcurr1 = crate::mbyte::utf_slice2char(&curr[cend_idx..]);
            if cend_idx > 0 {
                cend_idx -= (crate::mbyte::utf_head_off_slice(curr, cend_idx - 1) as usize) + 1;
                endcurr2 = crate::mbyte::utf_slice2char(&curr[cend_idx..]);
            }
        }
        line_breakcheck();
        if g_st().got_int.get() {
            ret = false;
            aborted = true;
            break;
        }
    }

    if !aborted {
        let col = sumsize - currsize - i32::from(spaces_vec[count - 1]);

        let newp_len = sumsize as usize;
        let mut newp_vec = vec![0u8; newp_len + 1];
        let mut cend_idx = sumsize as usize;

        g_st()
            .curbuf_splice_pending
            .set(g_st().curbuf_splice_pending.get() + (1));

        let mut t = (count as Linenr) - 1;
        loop {
            let csize_u = currsize as usize;
            cend_idx -= csize_u;
            newp_vec[cend_idx..cend_idx + csize_u].copy_from_slice(&curr[..csize_u]);

            let space_t = i32::from(spaces_vec[t as usize]);
            if space_t > 0 {
                let space_u = space_t as usize;
                cend_idx -= space_u;
                newp_vec[cend_idx..cend_idx + space_u].fill(b' ');
            }

            let spaces_removed = ((curr_start.len() - curr.len()) as i32) - space_t;
            let lnum = curwin_mut().w_cursor.get().lnum + t;
            let mincol: Colnr = 0;
            let lnum_amount = -t;
            let col_amount = (cend_idx as Colnr) - spaces_removed;

            mark_col_adjust(lnum, mincol, lnum_amount, col_amount, spaces_removed);

            if t == 0 {
                break;
            }

            curr_rc = crate::memline::ml_get_slice(curwin_mut().w_cursor.get().lnum + t - 1);
            curr_start = &curr_rc;
            curr = curr_start;
            if remove_comments {
                curr = &curr[(comments_vec[(t - 1) as usize] as usize).min(curr.len())..];
            }
            if insert_space && t > 1 {
                curr = crate::charset::skipwhite_slice(curr);
            }
            currsize = curr.len() as i32;
            t -= 1;
        }

        ml_replace_len(
            curwin_mut().w_cursor.get().lnum,
            &newp_vec[..newp_len.min(newp_vec.len())],
            newp_len,
            true,
        );
        drop(newp_vec);

        if setmark && (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
            curwin_mut()
                .buf_mut()
                .b_op_end
                .set_lnum(curwin_mut().w_cursor.get().lnum);
            curwin_mut().buf_mut().b_op_end.set_col(sumsize);
        }

        changed_lines(
            &curbuf_mut(),
            curwin_mut().w_cursor.get().lnum,
            currsize,
            curwin_mut().w_cursor.get().lnum + 1,
            0,
            true,
        );

        let saved_lnum = curwin_mut().w_cursor.get().lnum;
        curwin_mut().w_cursor.add_lnum(1);
        del_lines((count as i32) - 1, false);
        curwin_mut().w_cursor.set_lnum(saved_lnum);
        g_st()
            .curbuf_splice_pending
            .set(g_st().curbuf_splice_pending.get() - (1));
        curbuf_mut().deleted_bytes2.set(0);

        curwin_mut().w_cursor.set_col(
            if crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_JOINCOL) {
                currsize
            } else {
                col
            },
        );
        check_cursor_col(&curwin_mut());

        curwin_mut().w_cursor.set_coladd(0);
        curwin_mut().w_set_curswant.set(true);
    }

    ret
}

/// Reset 'linebreak' and take care of side effects.
pub fn reset_lbr() -> bool {
    if !curwin_mut().w_onebuf_opt.wo_lbr.get() {
        return false;
    }
    curwin_mut().w_onebuf_opt.wo_lbr.set(false);
    curwin_mut()
        .w_valid
        .set(curwin_mut().w_valid.get() & (!(VALID_WROW | VALID_WCOL | VALID_VIRTCOL)));
    true
}

/// Restore 'linebreak' and take care of side effects.
pub fn restore_lbr(lbr_saved: bool) {
    if curwin_mut().w_onebuf_opt.wo_lbr.get() || !lbr_saved {
        return;
    }

    curwin_mut().w_onebuf_opt.wo_lbr.set(true);
    curwin_mut()
        .w_valid
        .set(curwin_mut().w_valid.get() & (!(VALID_WROW | VALID_WCOL | VALID_VIRTCOL)));
}

/// Prepare a few things for block mode yank/delete/tilde.
pub fn block_prep(oap: &mut Oparg, bdp: &mut BlockDef, lnum: Linenr, is_del: bool) {
    let mut incr: i32 = 0;
    let lbr_saved = reset_lbr();

    bdp.startspaces = 0;
    bdp.endspaces = 0;
    bdp.textlen = 0;
    bdp.start_vcol = 0;
    bdp.end_vcol = 0;
    bdp.is_short = false;
    bdp.is_one_char = false;
    bdp.pre_whitesp = 0;
    bdp.pre_whitesp_c = 0;
    bdp.end_char_vcols = 0;
    bdp.start_char_vcols = 0;

    let line_cstr = ml_get(lnum);
    let mut prev_pstart_off = 0usize;

    let mut csarg = CharsizeArg::new();
    let mut cstype = init_charsize_arg(&mut csarg, &curwin_mut(), lnum, &line_cstr);
    let mut ci = utf_ptr2_str_char_info(&line_cstr);
    let mut vcol = bdp.start_vcol;
    while vcol < oap.start_vcol && !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        incr = win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells).width;
        vcol += incr;
        if ascii_iswhite(ci.chr.value) {
            bdp.pre_whitesp += incr;
            bdp.pre_whitesp_c += 1;
        } else {
            bdp.pre_whitesp = 0;
            bdp.pre_whitesp_c = 0;
        }
        prev_pstart_off = ci.offset_from(&line_cstr);
        ci = cli.next;
    }
    bdp.start_vcol = vcol;
    let mut pstart_off = ci.offset_from(&line_cstr);

    bdp.start_char_vcols = incr;
    if bdp.start_vcol < oap.start_vcol {
        bdp.end_vcol = bdp.start_vcol;
        bdp.is_short = true;
        if !is_del || oap.op_type == OP_APPEND {
            bdp.endspaces = oap.end_vcol - oap.start_vcol + 1;
        }
    } else {
        bdp.startspaces = bdp.start_vcol - oap.start_vcol;
        if is_del && bdp.startspaces != 0 {
            bdp.startspaces = bdp.start_char_vcols - bdp.startspaces;
        }
        let mut pend_off = pstart_off;
        bdp.end_vcol = bdp.start_vcol;
        if bdp.end_vcol > oap.end_vcol {
            bdp.is_one_char = true;
            if oap.op_type == OP_INSERT {
                bdp.endspaces = bdp.start_char_vcols - bdp.startspaces;
            } else if oap.op_type == OP_APPEND {
                bdp.startspaces += oap.end_vcol - oap.start_vcol + 1;
                bdp.endspaces = bdp.start_char_vcols - bdp.startspaces;
            } else {
                bdp.startspaces = oap.end_vcol - oap.start_vcol + 1;
                if is_del && oap.op_type != OP_LSHIFT {
                    bdp.startspaces = bdp.start_char_vcols - (bdp.start_vcol - oap.start_vcol);
                    bdp.endspaces = bdp.end_vcol - oap.end_vcol - 1;
                }
            }
        } else {
            cstype = init_charsize_arg(&mut csarg, &curwin_mut(), lnum, &line_cstr);
            ci = utf_ptr2_str_char_info(&line_cstr[pend_off..]);
            vcol = bdp.end_vcol;
            let mut prev_pend_off = pend_off;
            while vcol <= oap.end_vcol && !ci.ptr.is_empty() {
                prev_pend_off = ci.offset_from(&line_cstr);
                let cli = utf_cluster_info(ci);
                incr = win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells)
                    .width;
                vcol += incr;
                ci = cli.next;
            }
            bdp.end_vcol = vcol;
            pend_off = ci.offset_from(&line_cstr);

            if bdp.end_vcol <= oap.end_vcol
                && (!is_del || oap.op_type == OP_APPEND || oap.op_type == OP_REPLACE)
            {
                bdp.is_short = true;
                if oap.op_type == OP_APPEND || g_st().virtual_op.get() != 0 {
                    bdp.endspaces = oap.end_vcol - bdp.end_vcol + i32::from(oap.inclusive);
                }
            } else if bdp.end_vcol > oap.end_vcol {
                bdp.endspaces = bdp.end_vcol - oap.end_vcol - 1;
                if !is_del && bdp.endspaces != 0 {
                    bdp.endspaces = incr - bdp.endspaces;
                    if pend_off != pstart_off {
                        pend_off = prev_pend_off;
                    }
                }
            }
        }
        bdp.end_char_vcols = incr;
        if is_del && bdp.startspaces != 0 {
            pstart_off = prev_pstart_off;
        }
        bdp.textlen = ((pend_off as isize) - (pstart_off as isize)) as i32;
    }
    bdp.textcol = pstart_off as Colnr;
    bdp.textstart = line_cstr.slice(pstart_off);
    restore_lbr(lbr_saved);
}

pub fn getregionpos_prep(
    p1: &mut CursorPos,
    p2: &mut CursorPos,
    region_type: i32,
    exclusive: bool,
    block_width: i32,
    inclusive: &mut bool,
    oap: &mut Oparg,
) {
    *inclusive = true;
    if !lt(*p1, *p2) {
        std::mem::swap(p1, p2);
    }

    if region_type == MT_CHAR_WISE {
        if exclusive && !equalpos(*p1, *p2) {
            *inclusive = !unadjust_for_sel_inner(p2);
        }
        if *inclusive
            && g_st().virtual_op.get() == 0
            && crate::memline::ml_get_pos_slice(p2).is_empty()
        {
            *inclusive = false;
        }
    } else if region_type == MT_BLOCK_WISE {
        let mut sc1: Colnr = 0;
        let mut ec1: Colnr = 0;
        let mut sc2: Colnr = 0;
        let mut ec2: Colnr = 0;
        let lbr_saved = reset_lbr();
        getvvcol(&curwin_mut(), p1, Some(&mut sc1), None, Some(&mut ec1), 0);
        getvvcol(&curwin_mut(), p2, Some(&mut sc2), None, Some(&mut ec2), 0);
        restore_lbr(lbr_saved);
        oap.motion_type = MT_BLOCK_WISE;
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

    let l = crate::mbyte::utfc_slice2len(&crate::memline::ml_get_pos_slice(p2)) as Colnr;
    if l > 1 {
        p2.col += l - 1;
    }
}

pub fn getregionpos_line(
    lnum: Linenr,
    p1: CursorPos,
    p2: CursorPos,
    inclusive: bool,
    region_type: i32,
    oap: &mut Oparg,
    allow_eol: bool,
    ret_p1: &mut CursorPos,
    ret_p2: &mut CursorPos,
) {
    let line_cstr = ml_get(lnum);
    let line = line_cstr.as_ptr().cast_mut();
    let line_len = ml_get_len(lnum);

    if region_type == MT_LINE_WISE {
        ret_p1.col = 1;
        ret_p1.coladd = 0;
        ret_p2.col = MAXCOL;
        ret_p2.coladd = 0;
    } else {
        let mut bd = BlockDef::new();

        if region_type == MT_BLOCK_WISE {
            block_prep(oap, &mut bd, lnum, false);
        } else {
            charwise_block_prep(p1, p2, &mut bd, lnum, inclusive);
        }

        if bd.is_one_char {
            if region_type == MT_BLOCK_WISE {
                let off = (bd.textstart.as_ptr() as usize).saturating_sub(line as usize);
                let prev_off = if off > 0 {
                    off - 1
                        - (crate::mbyte::utf_head_off_slice(line_cstr.to_bytes(), off - 1) as usize)
                } else {
                    0
                };
                ret_p1.col = (prev_off as Colnr) + 1;
                ret_p1.coladd = bd.start_char_vcols - (bd.start_vcol - oap.start_vcol);
            } else {
                ret_p1.col = p1.col + 1;
                ret_p1.coladd = p1.coladd;
            }
        } else if region_type == MT_BLOCK_WISE && oap.start_vcol > bd.start_vcol {
            ret_p1.col = MAXCOL;
            ret_p1.coladd = oap.start_vcol - bd.start_vcol;
            bd.is_one_char = true;
        } else if bd.startspaces > 0 {
            let off = (bd.textstart.as_ptr() as usize).saturating_sub(line as usize);
            let prev_off = if off > 0 {
                off - 1 - (crate::mbyte::utf_head_off_slice(line_cstr.to_bytes(), off - 1) as usize)
            } else {
                0
            };
            ret_p1.col = (prev_off as Colnr) + 1;
            ret_p1.coladd = bd.start_char_vcols - bd.startspaces;
        } else {
            ret_p1.col = bd.textcol + 1;
            ret_p1.coladd = 0;
        }

        if bd.is_one_char {
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

pub fn charwise_block_prep(
    mut start: CursorPos,
    mut end: CursorPos,
    bdp: &mut BlockDef,
    lnum: Linenr,
    inclusive: bool,
) {
    let mut startcol: Colnr = 0;
    let mut endcol: Colnr = MAXCOL;
    let mut cs: Colnr = 0;
    let mut ce: Colnr = 0;
    let p_cstr = ml_get(lnum);
    let p_bytes = p_cstr.to_bytes();
    let plen = ml_get_len(lnum);

    bdp.startspaces = 0;
    bdp.endspaces = 0;
    bdp.is_one_char = false;
    bdp.start_char_vcols = 0;

    if lnum == start.lnum {
        startcol = start.col;
        if g_st().virtual_op.get() != 0 {
            getvcol(
                &curwin_mut(),
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
        if g_st().virtual_op.get() != 0 {
            getvcol(
                &curwin_mut(),
                &mut end,
                Some(&mut cs),
                None,
                Some(&mut ce),
                0,
            );
            if (endcol as usize) >= p_bytes.len()
                || (cs + end.coladd < ce
                    && crate::mbyte::utf_head_off_slice(p_bytes, endcol as usize) == 0)
            {
                if start.lnum == end.lnum && start.col == end.col {
                    bdp.is_one_char = true;
                    bdp.startspaces = end.coladd - start.coladd + i32::from(inclusive);
                    endcol = startcol;
                } else {
                    bdp.endspaces = end.coladd + i32::from(inclusive);
                    endcol -= i32::from(inclusive);
                }
            }
        }
    }
    if endcol == MAXCOL {
        endcol = ml_get_len(lnum);
    }
    if startcol > endcol || bdp.is_one_char {
        bdp.textlen = 0;
    } else {
        bdp.textlen = endcol - startcol + i32::from(inclusive);
    }
    bdp.textcol = startcol;
    bdp.textstart = if startcol <= plen {
        p_cstr.slice(startcol as usize)
    } else {
        p_cstr
    };
}

pub fn op_addsub(oap: &mut Oparg, prenum1: Linenr, g_cmd: bool) {
    let mut bd = BlockDef::new();
    let mut change_cnt: isize = 0;
    let mut amount = prenum1;

    g_st()
        .disable_fold_update
        .set(g_st().disable_fold_update.get() + (1));

    if !g_st().visual_global.borrow().active {
        let mut pos = curwin_mut().w_cursor.get();
        if !u_save_cursor() {
            g_st()
                .disable_fold_update
                .set(g_st().disable_fold_update.get() - (1));
            return;
        }
        change_cnt = isize::from(do_addsub(oap.op_type, &mut pos, 0, amount));
        g_st()
            .disable_fold_update
            .set(g_st().disable_fold_update.get() - (1));
        if change_cnt != 0 {
            changed_lines(&curbuf_mut(), pos.lnum, 0, pos.lnum + 1, 0, true);
        }
    } else {
        let mut length: i32;
        let mut startpos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        if !u_save(oap.start.lnum - 1, oap.end.lnum + 1) {
            g_st()
                .disable_fold_update
                .set(g_st().disable_fold_update.get() - (1));
            return;
        }

        let mut pos = oap.start;
        while pos.lnum <= oap.end.lnum {
            if oap.motion_type == MT_BLOCK_WISE {
                block_prep(oap, &mut bd, pos.lnum, false);
                pos.col = bd.textcol;
                length = bd.textlen;
            } else if oap.motion_type == MT_LINE_WISE {
                curwin_mut().w_cursor.set_col(0);
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
                    startpos = curbuf_mut().b_op_start.get();
                }
                change_cnt += 1;
            }

            if g_cmd && one_change {
                amount += prenum1;
            }
            pos.lnum += 1;
        }

        g_st()
            .disable_fold_update
            .set(g_st().disable_fold_update.get() - (1));
        if change_cnt != 0 {
            changed_lines(&curbuf_mut(), oap.start.lnum, 0, oap.end.lnum + 1, 0, true);
        }

        if change_cnt == 0 && oap.is_visual {
            redraw_curbuf_later(UPD_INVERTED);
        }

        if change_cnt > 0 && (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
            curbuf_mut().b_op_start.set(startpos);
        }

        if (change_cnt as i64) > opt_vars().p_report.get() {
            smsg!(
                0,
                ngettext(
                    c"%lld lines changed",
                    c"%lld lines changed",
                    change_cnt as u64,
                ),
                change_cnt as i64,
            );
        }
    }
}

pub fn do_addsub(op_type: i32, pos: &mut CursorPos, mut length: i32, prenum1: Linenr) -> bool {
    let mut pre: i32 = 0;
    let mut n: Uvarnumber = 0;
    let mut blank_unsigned = false;
    let mut negative = false;
    let mut was_positive = true;
    let visual = g_st().visual_global.borrow().active;
    let did_change: bool;
    let save_cursor = curwin_mut().w_cursor.get();
    let mut maxlen: i32 = 0;
    let startpos: CursorPos;
    let endpos: CursorPos;
    let mut save_coladd: Colnr = 0;

    let do_hex = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'x' as i32);
    let do_oct = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'o' as i32);
    let do_bin = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'b' as i32);
    let do_alpha = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'p' as i32);
    let do_unsigned = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'u' as i32);
    let do_blank = crate::strings::vim_haschr(&curbuf_mut().b_p_nf.as_cstr(), b'k' as i32);

    if virtual_active(&curwin_mut()) {
        save_coladd = pos.coladd;
        pos.coladd = 0;
    }

    curwin_mut().w_cursor.set(*pos);
    let ptr_line = ml_get(pos.lnum);
    let linelen = ml_get_len(pos.lnum);
    let line_bytes = ptr_line.to_bytes_with_nul();
    let mut col = pos.col;

    if col + i32::from(save_coladd != 0) >= linelen {
        if visual {
            curwin_mut().w_cursor.set(save_cursor);
        } else if virtual_active(&curwin_mut()) {
            curwin_mut().w_cursor.set_coladd(save_coladd);
        }
        return false;
    }

    if !g_st().visual_global.borrow().active {
        if do_bin {
            while col > 0 && ascii_isbdigit(i32::from(line_bytes[col as usize])) {
                col -= 1;
                col -= crate::mbyte::utf_head_off_slice(line_bytes, col as usize);
            }
        }

        if do_hex {
            while col > 0 && ascii_isxdigit(i32::from(line_bytes[col as usize])) {
                col -= 1;
                col -= crate::mbyte::utf_head_off_slice(line_bytes, col as usize);
            }
        }
        if do_bin
            && do_hex
            && !(col > 0
                && matches!(line_bytes[col as usize], b'X' | b'x')
                && line_bytes[(col - 1) as usize] == b'0'
                && crate::mbyte::utf_head_off_slice(line_bytes, (col - 1) as usize) == 0
                && ascii_isxdigit(i32::from(line_bytes[(col + 1) as usize])))
        {
            col = curwin_mut().w_cursor.get().col;

            while col > 0 && ascii_isdigit(i32::from(line_bytes[col as usize])) {
                col -= 1;
                col -= crate::mbyte::utf_head_off_slice(line_bytes, col as usize);
            }
        }

        if (do_hex
            && col > 0
            && matches!(line_bytes[col as usize], b'X' | b'x')
            && line_bytes[(col - 1) as usize] == b'0'
            && crate::mbyte::utf_head_off_slice(line_bytes, (col - 1) as usize) == 0
            && ascii_isxdigit(i32::from(line_bytes[(col + 1) as usize])))
            || (do_bin
                && col > 0
                && matches!(line_bytes[col as usize], b'B' | b'b')
                && line_bytes[(col - 1) as usize] == b'0'
                && crate::mbyte::utf_head_off_slice(line_bytes, (col - 1) as usize) == 0
                && ascii_isbdigit(i32::from(line_bytes[(col + 1) as usize])))
        {
            col -= 1;
            col -= crate::mbyte::utf_head_off_slice(line_bytes, col as usize);
        } else {
            col = pos.col;

            while line_bytes[col as usize] != 0
                && !ascii_isdigit(i32::from(line_bytes[col as usize]))
                && !(do_alpha && ascii_isalpha(i32::from(line_bytes[col as usize])))
            {
                col += 1;
            }

            while col > 0
                && ascii_isdigit(i32::from(line_bytes[(col - 1) as usize]))
                && !(do_alpha && ascii_isalpha(i32::from(line_bytes[col as usize])))
            {
                col -= 1;
            }
        }
    }

    if visual {
        while line_bytes[col as usize] != 0
            && length > 0
            && !ascii_isdigit(i32::from(line_bytes[col as usize]))
            && !(do_alpha && ascii_isalpha(i32::from(line_bytes[col as usize])))
        {
            let mb_len = utfc_ptr2len(&line_bytes[col as usize..]);
            col += mb_len;
            length -= mb_len;
        }

        if length == 0 {
            curwin_mut().w_cursor.set(save_cursor);
            return false;
        }

        if col > pos.col
            && line_bytes[(col - 1) as usize] == b'-'
            && crate::mbyte::utf_head_off_slice(line_bytes, (col - 1) as usize) == 0
            && !do_unsigned
        {
            if do_blank && col >= 2 && !ascii_iswhite(i32::from(line_bytes[(col - 2) as usize])) {
                blank_unsigned = true;
            } else {
                negative = true;
                was_positive = false;
            }
        }
    }

    let mut firstdigit = i32::from(line_bytes[col as usize]);
    if !ascii_isdigit(firstdigit) && !(do_alpha && ascii_isalpha(firstdigit)) {
        beep_flush();
        if visual {
            curwin_mut().w_cursor.set(save_cursor);
        } else if virtual_active(&curwin_mut()) {
            curwin_mut().w_cursor.set_coladd(save_coladd);
        }
        return false;
    }

    if do_alpha && ascii_isalpha(firstdigit) {
        if op_type == OP_NR_SUB {
            if char_ord(firstdigit) < prenum1 {
                firstdigit = if ascii_isupper(firstdigit) {
                    b'A' as i32
                } else {
                    b'a' as i32
                };
            } else {
                firstdigit -= prenum1;
            }
        } else if 26 - char_ord(firstdigit) - 1 < prenum1 {
            firstdigit = if ascii_isupper(firstdigit) {
                b'Z' as i32
            } else {
                b'z' as i32
            };
        } else {
            firstdigit += prenum1;
        }
        curwin_mut().w_cursor.set_col(col);
        startpos = curwin_mut().w_cursor.get();
        did_change = true;
        del_char(false);
        ins_char(firstdigit);
        endpos = curwin_mut().w_cursor.get();
        curwin_mut().w_cursor.set_col(col);
    } else {
        if col > 0
            && line_bytes[(col - 1) as usize] == b'-'
            && crate::mbyte::utf_head_off_slice(line_bytes, (col - 1) as usize) == 0
            && !visual
            && !do_unsigned
        {
            if do_blank && col >= 2 && !ascii_iswhite(i32::from(line_bytes[(col - 2) as usize])) {
                blank_unsigned = true;
            } else {
                col -= 1;
                negative = true;
            }
        }

        if visual && g_st().visual_global.borrow().mode != b'V' as i32 {
            maxlen = if curbuf_mut().b_visual.borrow().vi_curswant == MAXCOL {
                linelen - col
            } else {
                length
            };
        }

        let mut overflow = false;
        vim_str2nr(
            crate::memory::bytes_until_nul(&line_bytes[col as usize..]),
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
                n.wrapping_sub(prenum1 as Uvarnumber)
            } else {
                n.wrapping_add(prenum1 as Uvarnumber)
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

        curwin_mut().w_cursor.set_col(col);
        startpos = curwin_mut().w_cursor.get();
        did_change = true;
        let mut todel = length;
        let mut c = gchar_cursor();

        if c == b'-' as i32 {
            length -= 1;
        }
        while todel > 0 {
            todel -= 1;
            if c < 0x100 && ascii_isalpha(c) {
                ADDSUB_HEXUPPER.set(ascii_isupper(c));
            }
            del_char(false);
            c = gchar_cursor();
        }

        let mut buf1_vec = Vec::with_capacity((length as usize) + NUMBUFLEN);
        if negative && (!visual || was_positive) {
            buf1_vec.push(b'-');
        }
        if pre != 0 {
            buf1_vec.push(b'0');
            length -= 1;
        }
        if pre == b'b' as i32 || pre == b'B' as i32 || pre == b'x' as i32 || pre == b'X' as i32 {
            buf1_vec.push(pre as u8);
            length -= 1;
        }

        let mut buf2 = [0u8; NUMBUFLEN];
        let mut buf2len: i32 = 0;
        if pre == b'b' as i32 || pre == b'B' as i32 {
            let mut bits = Uvarnumber::BITS as usize;
            while bits > 0 {
                if ((n >> (bits - 1)) & 0x1) != 0 {
                    break;
                }
                bits -= 1;
            }

            while bits > 0 && (buf2len as usize) < NUMBUFLEN - 1 {
                bits -= 1;

                buf2[buf2len as usize] = if ((n >> bits) & 0x1) != 0 { b'1' } else { b'0' };

                buf2len += 1;
            }

            buf2[buf2len as usize] = 0;
        } else if pre == 0 {
            buf2len = format_to_buf!(&mut buf2, c"%llu", n as u64);
        } else if pre == b'0' as i32 {
            buf2len = format_to_buf!(&mut buf2, c"%llo", n as u64);
        } else if ADDSUB_HEXUPPER.get() {
            buf2len = format_to_buf!(&mut buf2, c"%llX", n as u64);
        } else {
            buf2len = format_to_buf!(&mut buf2, c"%llx", n as u64);
        }
        length -= buf2len;

        if firstdigit == b'0' as i32 && !(do_oct && pre == 0) {
            while length > 0 {
                length -= 1;
                buf1_vec.push(b'0');
            }
        }

        buf1_vec.extend_from_slice(&buf2[..buf2len as usize]);
        let buf1len = buf1_vec.len();
        buf1_vec.push(0);

        ins_str(&buf1_vec, buf1len);
        drop(buf1_vec);

        endpos = curwin_mut().w_cursor.get();
        if curwin_mut().w_cursor.get().col != 0 {
            curwin_mut().w_cursor.sub_col(1);
        }
    }

    if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) == 0 {
        curbuf_mut().b_op_start.set(startpos);
        curbuf_mut().b_op_end.set(endpos);
        if curbuf_mut().b_op_end.get().col > 0 {
            curbuf_mut().b_op_end.sub_col(1);
        }
    }

    if visual {
        curwin_mut().w_cursor.set(save_cursor);
    } else if did_change {
        curwin_mut().w_set_curswant.set(true);
    } else if virtual_active(&curwin_mut()) {
        curwin_mut().w_cursor.set_coladd(save_coladd);
    }

    did_change
}

fn line_count_info(
    line_bytes: &[u8],
    wc: &mut Varnumber,
    cc: &mut Varnumber,
    limit: Varnumber,
    eol_size: i32,
) -> Varnumber {
    let mut i: Varnumber = 0;
    let mut words: Varnumber = 0;
    let mut chars: Varnumber = 0;
    let mut is_word = false;

    while i < limit && (i as usize) < line_bytes.len() {
        let ch = i32::from(line_bytes[i as usize]);
        if is_word {
            if ascii_isspace(ch) {
                words += 1;
                is_word = false;
            }
        } else if !ascii_isspace(ch) {
            is_word = true;
        }
        chars += 1;
        i += Varnumber::from(crate::mbyte::utfc_slice2len(&line_bytes[i as usize..]) as i32);
    }

    if is_word {
        words += 1;
    }
    *wc += words;

    if i < limit && (i as usize) >= line_bytes.len() {
        i += Varnumber::from(eol_size);
        chars += Varnumber::from(eol_size);
    }
    *cc += chars;
    i
}

pub fn cursor_pos_info(dict: Option<&TvDict>) {
    let mut buf1 = [0u8; 50];
    let mut buf2 = [0u8; 40];
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
    let l_visual_active = g_st().visual_global.borrow().active;
    let l_visual_mode = g_st().visual_global.borrow().mode;

    if (curbuf_mut().b_ml.ml_flags.get() & ML_EMPTY) != 0 {
        if dict.is_none() {
            msg(tr(NO_LINES_MSG), 0);
            return;
        }
    } else {
        let mut last_check: Varnumber = 100000;
        let mut line_count_selected: i32 = 0;
        let eol_size = if get_fileformat(&curbuf_mut()) == EOL_DOS {
            2
        } else {
            1
        };

        if l_visual_active {
            if lt(
                g_st().visual_global.borrow().start,
                curwin_mut().w_cursor.get(),
            ) {
                min_pos = g_st().visual_global.borrow().start;
                max_pos = curwin_mut().w_cursor.get();
            } else {
                min_pos = curwin_mut().w_cursor.get();
                max_pos = g_st().visual_global.borrow().start;
            }
            if opt_vars().p_sel.first_byte() == b'e' && max_pos.col > 0 {
                max_pos.col -= 1;
            }

            if l_visual_mode == CTRL_V_CONST {
                let saved_sbr = opt_vars().p_sbr.take();
                let saved_w_sbr = curwin_mut().w_onebuf_opt.wo_sbr.take();

                oparg.is_visual = true;
                oparg.motion_type = MT_BLOCK_WISE;
                oparg.op_type = OP_NOP;
                getvcols(
                    &curwin_mut(),
                    &mut min_pos,
                    &mut max_pos,
                    &mut oparg.start_vcol,
                    &mut oparg.end_vcol,
                    0,
                );
                opt_vars().p_sbr.set(saved_sbr);
                curwin_mut().w_onebuf_opt.wo_sbr.set(saved_w_sbr);
                if curwin_mut().w_curswant.get() == MAXCOL {
                    oparg.end_vcol = MAXCOL;
                }
                if oparg.end_vcol < oparg.start_vcol {
                    std::mem::swap(&mut oparg.start_vcol, &mut oparg.end_vcol);
                }
            }
            line_count_selected = max_pos.lnum - min_pos.lnum + 1;
        }

        for lnum in 1..=curbuf_mut().b_ml.ml_line_count.get() {
            if byte_count > last_check {
                os_breakcheck();
                if g_st().got_int.get() {
                    return;
                }
                last_check = byte_count + 100000;
            }

            if l_visual_active && lnum >= min_pos.lnum && lnum <= max_pos.lnum {
                let mut s_opt: Option<&[u8]> = None;
                let mut len: i32 = 0;

                let line_rc;
                if l_visual_mode == CTRL_V_CONST {
                    g_st()
                        .virtual_op
                        .set(TriState::from(virtual_active(&curwin_mut())));
                    block_prep(&mut oparg, &mut bd, lnum, false);
                    g_st().virtual_op.set(K_NONE);
                    s_opt = Some(bd.textstart.to_bytes());
                    len = bd.textlen;
                } else if l_visual_mode == b'V' as i32 {
                    line_rc = ml_get(lnum);
                    s_opt = Some(line_rc.to_bytes());
                    len = MAXCOL;
                } else if l_visual_mode == b'v' as i32 {
                    let start_col = if lnum == min_pos.lnum { min_pos.col } else { 0 };
                    let end_col = if lnum == max_pos.lnum {
                        max_pos.col - start_col + 1
                    } else {
                        MAXCOL
                    };

                    line_rc = ml_get(lnum);
                    let line_bytes = line_rc.to_bytes();
                    s_opt = Some(&line_bytes[(start_col as usize).min(line_bytes.len())..]);
                    len = end_col;
                }
                if let Some(s_bytes) = s_opt {
                    byte_count_cursor += line_count_info(
                        s_bytes,
                        &mut word_count_cursor,
                        &mut char_count_cursor,
                        Varnumber::from(len),
                        eol_size,
                    );
                    if lnum == curbuf_mut().b_ml.ml_line_count.get()
                        && !curbuf_mut().b_p_eol.get()
                        && (curbuf_mut().b_p_bin.get() || !curbuf_mut().b_p_fixeol.get())
                        && (s_bytes.len() as i32) < len
                    {
                        byte_count_cursor -= Varnumber::from(eol_size);
                    }
                }
            } else if lnum == curwin_mut().w_cursor.get().lnum {
                word_count_cursor += word_count;
                char_count_cursor += char_count;
                byte_count_cursor = byte_count
                    + line_count_info(
                        &crate::memline::ml_get_slice(lnum),
                        &mut word_count_cursor,
                        &mut char_count_cursor,
                        Varnumber::from(curwin_mut().w_cursor.get().col) + 1,
                        eol_size,
                    );
            }
            byte_count += line_count_info(
                &crate::memline::ml_get_slice(lnum),
                &mut word_count,
                &mut char_count,
                Varnumber::from(MAXCOL),
                eol_size,
            );
        }

        if !curbuf_mut().b_p_eol.get()
            && (curbuf_mut().b_p_bin.get() || !curbuf_mut().b_p_fixeol.get())
        {
            byte_count -= Varnumber::from(eol_size);
        }

        let _iobuff_ptr = g_st().iobuff.borrow_mut().as_mut_ptr();
        if dict.is_none() {
            if l_visual_active {
                if l_visual_mode == CTRL_V_CONST && curwin_mut().w_curswant.get() < MAXCOL {
                    let mut min_col = min_pos.col;
                    let mut max_col = max_pos.col;
                    getvcols(
                        &curwin_mut(),
                        &mut min_pos,
                        &mut max_pos,
                        &mut min_col,
                        &mut max_col,
                        0,
                    );
                    let cols = i64::from(oparg.end_vcol + 1)
                        .checked_sub(i64::from(oparg.start_vcol))
                        .unwrap_or_else(|| std::process::abort());
                    format_to_buf!(&mut buf1, tr(c"%lld Cols; "), cols);
                } else {
                    buf1[0] = 0;
                }

                let buf1_cstr = std::ffi::CStr::from_bytes_until_nul(&buf1).unwrap_or_default();
                if char_count_cursor == byte_count_cursor && char_count == byte_count {
                    format_to_buf!(
                        &mut *g_st().iobuff.borrow_mut(),
                        tr(
                            c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Bytes",
                        ),
                        buf1_cstr,
                        i64::from(line_count_selected),
                        i64::from(curbuf_mut().b_ml.ml_line_count.get()),
                        word_count_cursor as i64,
                        word_count as i64,
                        byte_count_cursor as i64,
                        byte_count as i64,
                    );
                } else {
                    format_to_buf!(
                        &mut *g_st().iobuff.borrow_mut(),
                        tr(
                            c"Selected %s%lld of %lld Lines; %lld of %lld Words; %lld of %lld Chars; %lld of %lld Bytes",
                        ),
                        buf1_cstr,
                        i64::from(line_count_selected),
                        i64::from(curbuf_mut().b_ml.ml_line_count.get()),
                        word_count_cursor as i64,
                        word_count as i64,
                        char_count_cursor as i64,
                        char_count as i64,
                        byte_count_cursor as i64,
                        byte_count as i64,
                    );
                }
            } else {
                validate_virtcol(&curwin_mut());
                col_print(
                    &mut buf1,
                    curwin_mut().w_cursor.get().col + 1,
                    curwin_mut().w_virtcol.get() + 1,
                );
                col_print(
                    &mut buf2,
                    get_cursor_line_len(),
                    linetabsize_str(&crate::cursor::get_cursor_line_slice()),
                );
                let buf1_cstr = std::ffi::CStr::from_bytes_until_nul(&buf1).unwrap_or_default();
                let buf2_cstr = std::ffi::CStr::from_bytes_until_nul(&buf2).unwrap_or_default();

                if char_count_cursor == byte_count_cursor && char_count == byte_count {
                    format_to_buf!(
                        &mut *g_st().iobuff.borrow_mut(),
                        tr(
                            c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Byte %lld of %lld",
                        ),
                        buf1_cstr,
                        buf2_cstr,
                        i64::from(curwin_mut().w_cursor.get().lnum),
                        i64::from(curbuf_mut().b_ml.ml_line_count.get()),
                        word_count_cursor as i64,
                        word_count as i64,
                        byte_count_cursor as i64,
                        byte_count as i64,
                    );
                } else {
                    format_to_buf!(
                        &mut *g_st().iobuff.borrow_mut(),
                        tr(
                            c"Col %s of %s; Line %lld of %lld; Word %lld of %lld; Char %lld of %lld; Byte %lld of %lld",
                        ),
                        buf1_cstr,
                        buf2_cstr,
                        i64::from(curwin_mut().w_cursor.get().lnum),
                        i64::from(curbuf_mut().b_ml.ml_line_count.get()),
                        word_count_cursor as i64,
                        word_count as i64,
                        char_count_cursor as i64,
                        char_count as i64,
                        byte_count_cursor as i64,
                        byte_count as i64,
                    );
                }
            }
        }

        bom_count = Varnumber::from(bomb_size());
        if dict.is_none() && bom_count > 0 {
            let len = g_st().iobuff_cstr().to_bytes().len();
            format_to_buf!(
                &mut g_st().iobuff.borrow_mut()[len..],
                tr(c"(+%lld for BOM)"),
                bom_count as i64,
            );
        }
        if dict.is_none() {
            let p = opt_vars().p_shm.take();
            if opt_vars().p_ch.get() < 1 {
                msg_start();
                g_st().msg_scroll.set(1);
            }
            msg(&g_st().iobuff_cstr(), 0);
            opt_vars().p_shm.set(p);
        }
    }

    if let Some(dict) = dict {
        tv_dict_add_nr(dict, c"words", word_count);
        tv_dict_add_nr(dict, c"chars", char_count);
        tv_dict_add_nr(dict, c"bytes", byte_count + bom_count);

        tv_dict_add_nr(
            dict,
            if l_visual_active {
                c"visual_bytes"
            } else {
                c"cursor_bytes"
            },
            byte_count_cursor,
        );
        tv_dict_add_nr(
            dict,
            if l_visual_active {
                c"visual_chars"
            } else {
                c"cursor_chars"
            },
            char_count_cursor,
        );
        tv_dict_add_nr(
            dict,
            if l_visual_active {
                c"visual_words"
            } else {
                c"cursor_words"
            },
            word_count_cursor,
        );
    }
}

fn op_colon(oap: &mut Oparg) {
    if oap.is_visual {
        stuff_readbuff(c"'<,'>");
    } else {
        if oap.start.lnum == curwin_mut().w_cursor.get().lnum {
            stuffchar_readbuff(b'.' as i32);
        } else {
            stuffnum_readbuff(oap.start.lnum);
        }

        let mut end_of_start_fold = oap.start.lnum;
        has_folding(
            &curwin_mut(),
            oap.start.lnum,
            None,
            Some(&mut end_of_start_fold),
        );
        if oap.end.lnum != oap.start.lnum && oap.end.lnum != end_of_start_fold {
            stuffchar_readbuff(b',' as i32);
            if oap.end.lnum == curwin_mut().w_cursor.get().lnum {
                stuffchar_readbuff(b'.' as i32);
            } else if oap.end.lnum == curbuf_mut().b_ml.ml_line_count.get() {
                stuffchar_readbuff(b'$' as i32);
            } else if oap.start.lnum == curwin_mut().w_cursor.get().lnum
                && !has_folding(&curwin_mut(), oap.end.lnum, None, None)
            {
                stuff_readbuff(c".+");
                stuffnum_readbuff(oap.line_count - 1);
            } else {
                stuffnum_readbuff(oap.end.lnum);
            }
        }
    }
    if oap.op_type != OP_COLON {
        stuff_readbuff(c"!");
    }
    if oap.op_type == OP_INDENT {
        stuff_readbuff(&get_equalprg());
        stuff_readbuff(c"\n");
    } else if oap.op_type == OP_FORMAT {
        if !curbuf_mut().b_p_fp.is_empty() {
            stuff_readbuff(&curbuf_mut().b_p_fp.as_cstr());
        } else if !opt_vars().p_fp.is_empty() {
            stuff_readbuff(&opt_vars().p_fp.as_cstr());
        } else {
            stuff_readbuff(c"fmt");
        }
        stuff_readbuff(c"\n']");
    }

    do_cmdline(None, crate::cmdexpand::LineGetter::ExLine, 0);
    atom_suppress(true);
    exec_stuffed(None);
    atom_suppress(false);
}

pub fn free_operatorfunc_option() {
    callback_free(&opt_vars().p_opfunc);
}

pub fn set_ref_in_opfunc(copy_id: i32) -> bool {
    set_ref_in_callback(&opt_vars().p_opfunc, copy_id, None, None)
}

fn op_function(oap: &Oparg) {
    let orig_start = curbuf_mut().b_op_start.get();
    let orig_end = curbuf_mut().b_op_end.get();

    if crate::eval::typval::Callback::raw_cb_type(&opt_vars().p_opfunc) == K_CALLBACK_NONE_CONST {
        emsg(tr(c"E774: 'operatorfunc' is empty"));
    } else {
        curbuf_mut().b_op_start.set(oap.start);
        curbuf_mut().b_op_end.set(oap.end);
        if oap.motion_type == MT_CHAR_WISE && !oap.inclusive {
            curbuf_mut().b_op_end.update_mut(decl);
        }

        let mode_str = match oap.motion_type {
            MT_BLOCK_WISE => c"block",
            MT_LINE_WISE => c"line",
            _ => c"char",
        };
        let mut argv = [
            TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::from_cstr(mode_str),
            },
            TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::NULL_STRING,
            },
        ];

        let save_virtual_op = g_st().virtual_op.get();
        g_st().virtual_op.set(K_NONE);

        let save_finish_op = g_st().finish_op.get();
        g_st().finish_op.set(false);

        let mut save_redo = RedoState::new();
        save_redobuff(&mut save_redo);

        let mut rettv: TypVal = TypVal::new();
        if callback_call(&opt_vars().p_opfunc, 1, &mut argv, &mut rettv) {
            tv_clear(&mut rettv);
        }
        restore_redobuff(&mut save_redo);

        g_st().virtual_op.set(save_virtual_op);
        g_st().finish_op.set(save_finish_op);
        if (g_st().cmdmod.borrow().cmod_flags & CMOD_LOCKMARKS) != 0 {
            curbuf_mut().b_op_start.set(orig_start);
            curbuf_mut().b_op_end.set(orig_end);
        }
    }
}

fn get_op_vcol(oap: &mut Oparg, initial: bool) {
    let mut start: Colnr = 0;
    let mut end: Colnr = 0;

    if g_st().visual_global.borrow().mode != CTRL_V_CONST
        || (!initial && oap.end.col < curwin_mut().w_view_width.get())
    {
        return;
    }

    mark_mb_adjustpos(&curwin_mut().buf_mut(), &mut oap.end);

    getvvcol(
        &curwin_mut(),
        &mut oap.start,
        Some(&mut oap.start_vcol),
        None,
        Some(&mut oap.end_vcol),
        0,
    );
    getvvcol(
        &curwin_mut(),
        &mut oap.end,
        Some(&mut start),
        None,
        Some(&mut end),
        0,
    );

    oap.start_vcol = oap.start_vcol.min(start);
    if end > oap.end_vcol {
        if initial && opt_vars().p_sel.first_byte() == b'e' && start >= 1 && start > oap.end_vcol {
            oap.end_vcol = start - 1;
        } else {
            oap.end_vcol = end;
        }
    }

    if curwin_mut().w_curswant.get() == MAXCOL {
        curwin_mut().w_cursor.set_col(MAXCOL);
        oap.end_vcol = 0;
        curwin_mut().w_cursor.set_lnum(oap.start.lnum);
        while curwin_mut().w_cursor.get().lnum <= oap.end.lnum {
            let mut cursor = curwin_mut().w_cursor.get();
            getvvcol(&curwin_mut(), &mut cursor, None, None, Some(&mut end), 0);
            curwin_mut().w_cursor.set(cursor);
            oap.end_vcol = oap.end_vcol.max(end);
            curwin_mut().w_cursor.add_lnum(1);
        }
    }

    curwin_mut().w_cursor.set_lnum(oap.end.lnum);
    coladvance(&curwin_mut(), oap.end_vcol);
    oap.end = curwin_mut().w_cursor.get();

    curwin_mut().w_cursor.set(oap.start);
    coladvance(&curwin_mut(), oap.start_vcol);
    oap.start = curwin_mut().w_cursor.get();
}

fn is_ex_cmdchar(cap: &Cmdarg) -> bool {
    cap.cmdchar == b':' as i32 || cap.cmdchar == K_COMMAND
}

pub fn op_self_select(cap: &Cmdarg) -> bool {
    cap.cmdchar == b'g' as i32
        && (cap.nchar == b'n' as i32 || cap.nchar == b'N' as i32 || cap.nchar == b'v' as i32)
}

fn op_ins_visual(cap: &Cmdarg) -> VisualIns {
    let oap = &cap.oap;
    if !oap.is_visual {
        return K_VINS_NONE;
    }
    if is_ex_cmdchar(cap) || cap.cmdchar == K_LUA {
        return K_VINS_MOTION;
    }
    if op_self_select(cap) && cap.nchar != b'v' as i32 && oap.motion_force == i32::from(NUL) {
        return K_VINS_MOTION;
    }
    if oap.motion_force != i32::from(NUL) || op_self_select(cap) {
        return K_VINS_OTHER;
    }
    if atom_visual_redoable() {
        K_VINS_KEYS
    } else {
        K_VINS_OTHER
    }
}

pub fn do_pending_operator(cap: &mut Cmdarg, old_col: i32, gui_yank: bool) {
    let lbr_saved = curwin_mut().w_onebuf_opt.wo_lbr.get();

    let old_cursor = curwin_mut().w_cursor.get();
    let op_origin = cap.oap.start;

    if (g_st().finish_op.get() || g_st().visual_global.borrow().active) && cap.oap.op_type != OP_NOP
    {
        let mut include_line_break = false;
        let redo_yank =
            crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_YANK) && !gui_yank;

        reset_lbr();
        let oap = &mut cap.oap;
        oap.is_visual = g_st().visual_global.borrow().active;
        if oap.motion_force == b'V' as i32 {
            oap.motion_type = MT_LINE_WISE;
        } else if oap.motion_force == b'v' as i32 {
            if oap.motion_type == MT_LINE_WISE {
                oap.inclusive = false;
            } else if oap.motion_type == MT_CHAR_WISE {
                oap.inclusive = !oap.inclusive;
            }
            oap.motion_type = MT_CHAR_WISE;
        } else if oap.motion_force == CTRL_V_CONST {
            if !g_st().visual_global.borrow().active {
                g_st().visual_global.borrow_mut().active = true;
                g_st().visual_global.borrow_mut().start = oap.start;
            }
            g_st().visual_global.borrow_mut().mode = CTRL_V_CONST;
            g_st().visual_global.borrow_mut().select = false;
            g_st().visual_global.borrow_mut().reselect = 0;
        }

        atom_capture_op(cap, redo_yank);
        let oap = &mut cap.oap;

        if g_st().visual_global.borrow().active {
            if !gui_yank && !g_st().finish_op.get() {
                *curbuf_mut().b_visual.borrow_mut() = visualinfo();
                restore_visual_mode();
                curbuf_mut()
                    .b_visual_mode_eval
                    .set(g_st().visual_global.borrow().mode);
            }

            if g_st().visual_global.borrow().select
                && g_st().visual_global.borrow().mode == b'V' as i32
                && oap.op_type != OP_DELETE
            {
                if lt(
                    g_st().visual_global.borrow().start,
                    curwin_mut().w_cursor.get(),
                ) {
                    g_st().visual_global.borrow_mut().start.col = 0;
                    curwin_mut()
                        .w_cursor
                        .set_col(ml_get_len(curwin_mut().w_cursor.get().lnum));
                } else {
                    curwin_mut().w_cursor.set_col(0);
                    let slnum = g_st().visual_global.borrow().start.lnum;
                    g_st().visual_global.borrow_mut().start.col = ml_get_len(slnum);
                }
                g_st().visual_global.borrow_mut().mode = b'v' as i32;
            } else if g_st().visual_global.borrow().mode == b'v' as i32 {
                include_line_break = unadjust_for_sel();
            }

            oap.start = g_st().visual_global.borrow().start;
            if g_st().visual_global.borrow().mode == b'V' as i32 {
                oap.start.col = 0;
                oap.start.coladd = 0;
            }
        }

        if lt(oap.start, curwin_mut().w_cursor.get()) {
            if !g_st().visual_global.borrow().active {
                if has_folding(
                    &curwin_mut(),
                    oap.start.lnum,
                    Some(&mut oap.start.lnum),
                    None,
                ) {
                    oap.start.col = 0;
                }
                if curwin_mut().w_cursor.get().col > 0
                    || oap.inclusive
                    || oap.motion_type == MT_LINE_WISE
                {
                    let mut cur_lnum = curwin_mut().w_cursor.get().lnum;
                    if has_folding(&curwin_mut(), cur_lnum, None, Some(&mut cur_lnum)) {
                        curwin_mut().w_cursor.set_lnum(cur_lnum);
                        curwin_mut().w_cursor.set_col(get_cursor_line_len());
                    }
                }
            }
            oap.end = curwin_mut().w_cursor.get();
            curwin_mut().w_cursor.set(oap.start);

            curwin_mut()
                .w_valid
                .set(curwin_mut().w_valid.get() & (!VALID_VIRTCOL));
        } else {
            if !g_st().visual_global.borrow().active && oap.motion_type == MT_LINE_WISE {
                let mut cur_lnum = curwin_mut().w_cursor.get().lnum;
                if has_folding(&curwin_mut(), cur_lnum, Some(&mut cur_lnum), None) {
                    curwin_mut().w_cursor.set_lnum(cur_lnum);
                    curwin_mut().w_cursor.set_col(0);
                }
                if has_folding(
                    &curwin_mut(),
                    oap.start.lnum,
                    None,
                    Some(&mut oap.start.lnum),
                ) {
                    oap.start.col = ml_get_len(oap.start.lnum);
                }
            }
            oap.end = oap.start;
            oap.start = curwin_mut().w_cursor.get();
        }

        check_pos(&curwin_mut().buf_mut(), &mut oap.end);
        oap.line_count = oap.end.lnum - oap.start.lnum + 1;

        g_st()
            .virtual_op
            .set(TriState::from(virtual_active(&curwin_mut())));

        if g_st().visual_global.borrow().active {
            get_op_vcol(&mut *oap, true);

            if !gui_yank {
                let vmode = g_st().visual_global.borrow().mode;
                g_st().visual_global.borrow_mut().resel.mode = vmode;
                if curwin_mut().w_curswant.get() == MAXCOL {
                    g_st().visual_global.borrow_mut().resel.vcol = MAXCOL;
                } else {
                    if g_st().visual_global.borrow().mode != CTRL_V_CONST {
                        getvvcol(
                            &curwin_mut(),
                            &mut oap.end,
                            None,
                            None,
                            Some(&mut oap.end_vcol),
                            0,
                        );
                    }
                    if g_st().visual_global.borrow().mode == CTRL_V_CONST || oap.line_count <= 1 {
                        if g_st().visual_global.borrow().mode != CTRL_V_CONST {
                            getvvcol(
                                &curwin_mut(),
                                &mut oap.start,
                                Some(&mut oap.start_vcol),
                                None,
                                None,
                                0,
                            );
                        }
                        g_st().visual_global.borrow_mut().resel.vcol =
                            oap.end_vcol - oap.start_vcol + 1;
                    } else {
                        g_st().visual_global.borrow_mut().resel.vcol = oap.end_vcol;
                    }
                }
                g_st().visual_global.borrow_mut().resel.line_count = oap.line_count;
            }

            if oap.motion_force == i32::from(NUL) || oap.motion_type == MT_LINE_WISE {
                oap.inclusive = true;
            }
            if g_st().visual_global.borrow().mode == b'V' as i32 {
                oap.motion_type = MT_LINE_WISE;
            } else if g_st().visual_global.borrow().mode == b'v' as i32 {
                oap.motion_type = MT_CHAR_WISE;
                if crate::memline::ml_get_pos_slice(&oap.end).is_empty()
                    && (include_line_break || g_st().virtual_op.get() == 0)
                {
                    oap.inclusive = false;
                    if opt_vars().p_sel.first_byte() != b'o'
                        && op_on_lines(oap.op_type) == 0
                        && oap.end.lnum < curbuf_mut().b_ml.ml_line_count.get()
                    {
                        oap.end.lnum += 1;
                        oap.end.col = 0;
                        oap.end.coladd = 0;
                        oap.line_count += 1;
                    }
                }
            } else if g_st().visual_global.borrow().mode == CTRL_V_CONST {
                oap.motion_type = MT_BLOCK_WISE;
            }

            if !gui_yank {
                g_st().visual_global.borrow_mut().active = false;
                setmouse();
                g_st().mouse_dragging.set(0);
                may_clear_cmdline();
                if (oap.op_type == OP_YANK
                    || oap.op_type == OP_COLON
                    || oap.op_type == OP_FUNCTION
                    || oap.op_type == OP_FILTER)
                    && oap.motion_force == i32::from(NUL)
                {
                    restore_lbr(lbr_saved);
                    redraw_curbuf_later(UPD_INVERTED);
                }
            }
        }

        if oap.inclusive {
            let l =
                crate::mbyte::utfc_slice2len(&crate::memline::ml_get_pos_slice(&oap.end)) as Colnr;
            if l > 1 {
                oap.end.col += l - 1;
            }
        }
        curwin_mut().w_set_curswant.set(true);

        oap.empty = oap.motion_type != MT_LINE_WISE
            && ((!oap.inclusive && oap.motion_type != MT_BLOCK_WISE)
                || (oap.op_type == OP_YANK && gchar_pos(&mut oap.end) == i32::from(NUL)))
            && equalpos(oap.start, oap.end)
            && !(g_st().virtual_op.get() != 0 && oap.start.coladd != oap.end.coladd);
        let empty_region_error =
            oap.empty && crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_EMPTYREGION);

        if oap.is_visual && (oap.empty || !modifiable(&curbuf_mut()) || oap.op_type == OP_FOLD) {
            restore_lbr(lbr_saved);
            redraw_curbuf_later(UPD_INVERTED);
        }

        if oap.motion_type == MT_CHAR_WISE
            && !oap.inclusive
            && (cap.retval & CA_NO_ADJ_OP_END) == 0
            && oap.end.col == 0
            && (!oap.is_visual || opt_vars().p_sel.first_byte() == b'o')
            && oap.line_count > 1
        {
            oap.end_adjusted = true;
            oap.line_count -= 1;
            oap.end.lnum -= 1;
            if inindent(0) {
                oap.motion_type = MT_LINE_WISE;
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
                op_shift(oap, true, if oap.is_visual { cap.count1 } else { 1 });
                auto_format(false, true);
            }
            OP_JOIN_NS | OP_JOIN => {
                oap.line_count = oap.line_count.max(2);
                if curwin_mut().w_cursor.get().lnum + oap.line_count - 1
                    > curbuf_mut().b_ml.ml_line_count.get()
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
                g_st().visual_global.borrow_mut().reselect = 0;
                if empty_region_error {
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    op_delete(oap);
                    if oap.motion_type == MT_LINE_WISE
                        && has_format_option(K_FO_AUTO)
                        && u_save_cursor()
                    {
                        auto_format(false, true);
                    }
                }
            }
            OP_YANK => {
                if empty_region_error {
                    if !gui_yank {
                        vim_beep(K_OPT_BO_FLAG_OPERATOR);
                        redo_cancel();
                    }
                } else {
                    restore_lbr(lbr_saved);
                    oap.excl_tr_ws = cap.cmdchar == b'z' as i32;
                    if oap.restore_cursor {
                        curwin_mut().w_cursor.set(oap.cursor_start);
                    }
                    op_yank(oap, !gui_yank);
                }
                check_cursor_col(&curwin_mut());
            }
            OP_CHANGE => {
                g_st().visual_global.borrow_mut().reselect = 0;
                if empty_region_error {
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    let restart_edit_save = if !g_st().key_typed.get() {
                        g_st().restart_edit.get()
                    } else {
                        0
                    };
                    g_st().restart_edit.set(0);

                    restore_lbr(lbr_saved);

                    curbuf_mut()
                        .b_last_changedtick_i
                        .set(buf_get_changedtick(&curbuf_mut()));

                    let v_ins = op_ins_visual(cap);
                    let oap = &mut cap.oap;
                    let mut session: InsSession = atom_ins_start(
                        b'c' as i32,
                        i64::from(cap.count0),
                        v_ins,
                        oap.is_visual && oap.motion_type == MT_BLOCK_WISE,
                    );
                    let busy = op_change(oap) != 0;
                    if busy {
                        cap.retval |= CA_COMMAND_BUSY;
                    }
                    if g_st().restart_edit.get() == 0 {
                        g_st().restart_edit.set(restart_edit_save);
                    }
                    atom_ins_end(&mut session, busy);
                }
            }
            OP_FILTER | OP_INDENT | OP_COLON => {
                if oap.op_type == OP_FILTER {
                    if crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_FILTER) {
                        redo_append_str(b"!\r");
                    } else {
                        g_st().bangredo.set(true);
                    }
                }

                if oap.op_type == OP_INDENT && get_equalprg().is_empty() {
                    if curbuf_mut().b_p_lisp.get() {
                        if use_indentexpr_for_lisp() {
                            op_reindent(oap, get_expr_indent);
                        } else {
                            op_reindent(oap, get_lisp_indent);
                        }
                    } else {
                        op_reindent(
                            oap,
                            if curbuf_mut().b_p_inde.cb_type() != K_CALLBACK_NONE_CONST {
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
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    op_tilde(oap);
                }
                check_cursor_col(&curwin_mut());
            }
            OP_FORMAT => {
                if curbuf_mut().b_p_fex.cb_type() != K_CALLBACK_NONE_CONST {
                    op_formatexpr(oap);
                } else if !opt_vars().p_fp.is_empty() || !curbuf_mut().b_p_fp.is_empty() {
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
                g_st().visual_global.borrow_mut().reselect = 0;
                if empty_region_error {
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    let restart_edit_save = g_st().restart_edit.get();
                    g_st().restart_edit.set(0);

                    restore_lbr(lbr_saved);

                    curbuf_mut()
                        .b_last_changedtick_i
                        .set(buf_get_changedtick(&curbuf_mut()));

                    let v_ins = op_ins_visual(cap);
                    let oap = &mut cap.oap;
                    let mut session: InsSession = atom_ins_start(
                        b'I' as i32,
                        i64::from(cap.count1),
                        v_ins,
                        oap.motion_type == MT_BLOCK_WISE,
                    );
                    op_insert(oap, cap.count1);
                    atom_ins_end(&mut session, false);

                    reset_lbr();
                    auto_format(false, true);

                    if g_st().restart_edit.get() == 0 {
                        g_st().restart_edit.set(restart_edit_save);
                    } else {
                        cap.retval |= CA_COMMAND_BUSY;
                    }
                }
            }
            OP_REPLACE => {
                g_st().visual_global.borrow_mut().reselect = 0;
                if empty_region_error {
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    restore_lbr(lbr_saved);
                    op_replace(oap, cap.nchar);
                }
            }
            OP_FOLD => {
                g_st().visual_global.borrow_mut().reselect = 0;
                fold_create(&curwin_mut(), oap.start, oap.end);
            }
            OP_MCURSOR => {
                restore_lbr(lbr_saved);
                mc_zq(cap, op_origin);
            }
            OP_FOLDOPEN | OP_FOLDOPENREC | OP_FOLDCLOSE | OP_FOLDCLOSEREC => {
                g_st().visual_global.borrow_mut().reselect = 0;
                op_fold_range(
                    oap.start,
                    oap.end,
                    oap.op_type == OP_FOLDOPEN || oap.op_type == OP_FOLDOPENREC,
                    oap.op_type == OP_FOLDOPENREC || oap.op_type == OP_FOLDCLOSEREC,
                    oap.is_visual,
                );
            }
            OP_FOLDDEL | OP_FOLDDELREC => {
                g_st().visual_global.borrow_mut().reselect = 0;
                delete_fold(
                    &curwin_mut(),
                    oap.start.lnum,
                    oap.end.lnum,
                    i32::from(oap.op_type == OP_FOLDDELREC),
                    oap.is_visual,
                );
            }
            OP_NR_ADD | OP_NR_SUB => {
                if empty_region_error {
                    vim_beep(K_OPT_BO_FLAG_OPERATOR);
                    redo_cancel();
                } else {
                    g_st().visual_global.borrow_mut().active = true;
                    restore_lbr(lbr_saved);
                    op_addsub(oap, cap.count1, cap.arg != 0);
                    g_st().visual_global.borrow_mut().active = false;
                }
                check_cursor_col(&curwin_mut());
            }
            _ => {
                clearopbeep(oap);
            }
        }
        let oap = &mut cap.oap;
        g_st().virtual_op.set(K_NONE);
        if !gui_yank {
            if !opt_vars().p_sol.get()
                && oap.motion_type == MT_LINE_WISE
                && !oap.end_adjusted
                && (oap.op_type == OP_LSHIFT
                    || oap.op_type == OP_RSHIFT
                    || oap.op_type == OP_DELETE)
            {
                reset_lbr();
                curwin_mut().w_curswant.set(old_col);
                coladvance(&curwin_mut(), old_col);
            }
        } else {
            curwin_mut().w_cursor.set(old_cursor);
        }
        clearop(oap);
        g_st().motion_force.set(i32::from(NUL));
    }
    restore_lbr(lbr_saved);
}

/// Get the byte count of buffer region. End-exclusive.
pub fn get_region_bytecount(
    buf: &FileBuf,
    start_lnum: Linenr,
    end_lnum: Linenr,
    start_col: Colnr,
    end_col: Colnr,
) -> Bcount {
    let max_lnum = buf.b_ml.ml_line_count.get();
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
        assert_eq!(get_op_char(OP_UPPER), b'g' as i32);
        assert_eq!(get_extra_op_char(OP_UPPER), b'U' as i32);
        assert_ne!(op_on_lines(OP_INDENT), 0);
        assert_eq!(op_on_lines(OP_DELETE), 0);
        assert!(op_redoable(OP_DELETE, false));
        assert!(!op_redoable(OP_YANK, false));
        assert!(op_redoable(OP_YANK, true));
        assert_eq!(rot13(b'a' as i32, b'a' as i32), b'n' as i32);
        assert_eq!(rot13(b'n' as i32, b'a' as i32), b'a' as i32);
        assert_eq!(char_ord(b'C' as i32), 2);
        assert_eq!(char_ord(b'c' as i32), 2);
    }
}
