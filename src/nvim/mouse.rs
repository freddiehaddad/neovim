//! Mouse input, clicking, dragging, and scrolling (`src/nvim/mouse.c`).

use crate::buffer::{VALID_TOPLINE, VAR_FIXED, bt_prompt, bt_quickfix};
use crate::charset::{NUL, ascii_iswhite, vim_iswordc};
use crate::cursor::{
    coladvance, colnr_T, equalpos, get_cursor_pos_ptr, linenr_T, lt, ltoreq, pos_T, set_leftcol,
};
use crate::decoration::{decor_conceal_line, decor_virt_lines};
use crate::diff::diff_check_fill;
use crate::drawscreen::{
    DEFAULT_GRID_HANDLE, FR_LEAF, FR_ROW, UPD_INVERTED, UPD_VALID, VALID_BOTLINE,
    redraw_curbuf_later, redraw_later, redraw_statuslines, setcursor, update_screen,
};
use crate::eval::typval::{
    EvalFuncData, TypvalVval, VarType, tv_clear, tv_dict_add_nr, tv_dict_alloc_ret, typval_T,
    varnumber_T,
};
use crate::eval::vars::{VV_LNUM, VV_VIRTNUM, get_vim_var_nr, set_vim_var_nr};
use crate::eval::{call_vim_function, eval_has_provider};
use crate::ex_docmd::{do_cmdline_cmd, tabpage_close, tabpage_close_other, tabpage_new};
use crate::fold::{closeFold, hasFolding, openFold};
use crate::globals::*;
use crate::grid::{get_win_by_grid_handle, grid_adjust};
use crate::input::{
    prep_redo, redo_append_char, safe_vgetc, stuffReadbuff, stuffcharReadbuff, stuffnumReadbuff,
    vpeekc, vungetc,
};
use crate::input_cmdatom::{CmdSpec, cmdarg_T};
use crate::insert::{
    MOD_MASK_CTRL, MOD_MASK_SHIFT, set_can_cindent, start_arrow, undisplay_dollar,
};
use crate::keycodes::{
    K_LEFTDRAG, K_LEFTMOUSE, K_LEFTMOUSE_NM, K_LEFTRELEASE, K_LEFTRELEASE_NM, K_MIDDLEDRAG,
    K_MIDDLEMOUSE, K_MIDDLERELEASE, K_MOUSEDOWN, K_MOUSELEFT, K_MOUSEMOVE, K_MOUSERIGHT, K_MOUSEUP,
    K_RIGHTDRAG, K_RIGHTMOUSE, K_RIGHTRELEASE, K_X1DRAG, K_X1MOUSE, K_X1RELEASE, K_X2DRAG,
    K_X2MOUSE, K_X2RELEASE, get_mouse_button, key2termcap1,
};
use crate::mbyte::{
    mb_get_class, utf_ClusterInfo, utf_head_off, utf_ptr2StrCharInfo, utf8len_tab, utfc_ptr2len,
};
use crate::mcursor::mc_toggle;
use crate::memline::{gchar_pos, inc, ml_get, ml_get_buf};
use crate::menu::show_popupmenu;
use crate::message::siemsg;
use crate::r#move::{check_topfill, pagescroll, scroll_redraw, win_col_off, win_col_off2};
use crate::normal::{
    clearop, clearopbeep, end_visual_mode, exec_stuffed, may_start_select, nv_scroll_line,
};
use crate::ops::{OP_NOP, clear_oparg};
use crate::option::get_scrolloff_value;
use crate::option_vars::*;
use crate::plines::{
    CSType, CharsizeArg, getvcol, getvcols, init_charsize_arg, plines_win, plines_win_nofill,
    win_charsize, win_chartabsize, win_get_fill,
};
use crate::popupmenu::pum_visible;
use crate::register::{
    PUT_CURSEND, PUT_FIXINDENT, do_put, insert_reg, yank_register_mline, yankreg_T,
};
use crate::search::{BACKWARD, FORWARD, findmatch};
use crate::state::{MODE_INSERT, MODE_NORMAL, REPLACE_FLAG, virtual_active};
use crate::statusline::stl_connected;
use crate::strings::vim_strchr;
use crate::textformat::oparg_T;
use crate::ui::{ui_check_mouse, ui_cursor_shape, ui_flush, ui_mouse_has};
use crate::ui_compositor::ui_comp_mouse_focus;
use crate::window::{
    VALID_BOTLINE_AP, find_tabpage, global_stl_height, goto_tabpage, tabpage_index, tabpage_move,
    win_T, win_drag_status_line, win_drag_vsep_line, win_enter, win_fdccol_count, win_valid,
};
use std::ffi::{CStr, c_char, c_int, c_long};
use std::ptr;

use crate::cmdexpand::Ctrl_P;
use crate::drawline::{VALID_CROW, VALID_WROW};
use crate::eval::funcs::{kMTCharWise, kMTLineWise};
use crate::ex_getln::{Ctrl_G, Ctrl_O, Ctrl_R, Ctrl_RSB, Ctrl_T};
use crate::extmark::kNone;
use crate::insexpand::kWinInfo;
use crate::math::FAIL;
use crate::regexp::Ctrl_V;

pub const IN_UNKNOWN: c_int = 0;
pub const IN_BUFFER: c_int = 1;
pub const IN_STATUS_LINE: c_int = 2;
pub const IN_SEP_LINE: c_int = 4;
pub const IN_OTHER_WIN: c_int = 8;
pub const CURSOR_MOVED: c_int = 0x100;
pub const MOUSE_FOLD_CLOSE: c_int = 0x200;
pub const MOUSE_FOLD_OPEN: c_int = 0x400;
pub const MOUSE_WINBAR: c_int = 0x800;
pub const MOUSE_STATUSCOL: c_int = 0x1000;

pub const MOUSE_FOCUS: c_int = 0x01;
pub const MOUSE_MAY_VIS: c_int = 0x02;
pub const MOUSE_DID_MOVE: c_int = 0x04;
pub const MOUSE_SETPOS: c_int = 0x08;
pub const MOUSE_MAY_STOP_VIS: c_int = 0x10;
pub const MOUSE_RELEASED: c_int = 0x20;

pub const MOUSE_LEFT: c_int = 0x00;
pub const MOUSE_MIDDLE: c_int = 0x01;
pub const MOUSE_RIGHT: c_int = 0x02;
pub const MOUSE_RELEASE: c_int = 0x03;
pub const MOUSE_X1: c_int = 0x300;
pub const MOUSE_X2: c_int = 0x400;

pub const MSCR_DOWN: c_int = 0;
pub const MSCR_UP: c_int = 1;
pub const MSCR_LEFT: c_int = -1;
pub const MSCR_RIGHT: c_int = -2;

const MOD_MASK_ALT: c_int = 0x08;
const MOD_MASK_META: c_int = 0x10;
const MOD_MASK_2CLICK: c_int = 0x20;
const MOD_MASK_3CLICK: c_int = 0x40;
const MOD_MASK_4CLICK: c_int = 0x60;
const MOD_MASK_MULTI_CLICK: c_int = MOD_MASK_2CLICK | MOD_MASK_3CLICK | MOD_MASK_4CLICK;

pub(crate) const kMouseVisual: c_int = b'v' as c_int;
const ZERO_POS: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

pub(crate) const kStlClickDisabled: c_int = 0;
pub(crate) const kStlClickTabSwitch: c_int = 1;
pub(crate) const kStlClickTabClose: c_int = 2;
pub(crate) const kStlClickFuncRun: c_int = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StlClickDefinition {
    pub type_: c_int,
    pub tabnr: c_int,
    pub func: *mut c_char,
}

impl Default for StlClickDefinition {
    fn default() -> Self {
        Self {
            type_: kStlClickDisabled,
            tabnr: 0,
            func: ptr::null_mut(),
        }
    }
}

static mut orig_topline: linenr_T = 0;
static mut orig_topfill: c_int = 0;
static mut got_click: bool = false;
static mut dragwin: *mut win_T = ptr::null_mut();

/// Get class of a character for selection: same class means same word.
/// 0: blank
/// 1: punctuation groups
/// 2: normal word character
/// >2: multi-byte word character.
unsafe fn get_mouse_class(p: *const c_char) -> c_int {
    unsafe {
        if utf8len_tab[*p as u8 as usize] > 1 {
            return mb_get_class(p);
        }

        let c = c_int::from(*p as u8);
        if c == b' ' as c_int || c == b'\t' as c_int {
            return 0;
        }
        if vim_iswordc(c) {
            return 2;
        }

        if c != 0 && !vim_strchr(c"-+*/%<>&|^!=".as_ptr(), c).is_null() {
            return 1;
        }
        c
    }
}

/// Move "pos" back to the start of the word it's in.
unsafe fn find_start_of_word(pos: *mut pos_T) {
    unsafe {
        let line = ml_get((*pos).lnum);
        let cclass = get_mouse_class(line.add((*pos).col as usize));

        while (*pos).col > 0 {
            let mut col = (*pos).col - 1;
            col -= utf_head_off(line, line.add(col as usize));
            if get_mouse_class(line.add(col as usize)) != cclass {
                break;
            }
            (*pos).col = col;
        }
    }
}

/// Move "pos" forward to the end of the word it's in.
/// When 'selection' is "exclusive", the position is just after the word.
unsafe fn find_end_of_word(pos: *mut pos_T) {
    unsafe {
        let line = ml_get((*pos).lnum);
        if *p_sel == b'e' as c_char && (*pos).col > 0 {
            (*pos).col -= 1;
            (*pos).col -= utf_head_off(line, line.add((*pos).col as usize));
        }
        let cclass = get_mouse_class(line.add((*pos).col as usize));
        while *line.add((*pos).col as usize) != NUL {
            let col = (*pos).col + utfc_ptr2len(line.add((*pos).col as usize));
            if get_mouse_class(line.add(col as usize)) != cclass {
                if *p_sel == b'e' as c_char {
                    (*pos).col = col;
                }
                break;
            }
            (*pos).col = col;
        }
    }
}

/// Move the current tab to tab in same column as mouse or to end of the
/// tabline if there is no tab there.
unsafe fn move_tab_to_mouse() {
    unsafe {
        let defs = &raw const tab_page_click_defs;
        let tabnr = (*defs).as_slice()[mouse_col as usize].tabnr;
        if tabnr <= 0 {
            tabpage_move(9999);
        } else if tabnr < tabpage_index(curtab) {
            tabpage_move(tabnr - 1);
        } else {
            tabpage_move(tabnr);
        }
    }
}

/// Close the current or specified tab page.
unsafe fn mouse_tab_close(c1: c_int) {
    unsafe {
        let tp = if c1 == 999 { curtab } else { find_tabpage(c1) };
        if tp == curtab {
            if !(*first_tabpage).tp_next.is_null() {
                tabpage_close(0);
            }
        } else if !tp.is_null() {
            tabpage_close_other(tp, 0);
        }
    }
}

/// Call click definition function for column "col" in the "click_defs" array for button
/// "which_button".
unsafe fn call_click_def_func(
    click_defs: *mut StlClickDefinition,
    col: c_int,
    which_button: c_int,
) {
    unsafe {
        let clicks = if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_4CLICK {
            4
        } else if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_3CLICK {
            3
        } else if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
            2
        } else {
            1
        };

        let btn_str = if which_button == MOUSE_LEFT {
            c"l".as_ptr()
        } else if which_button == MOUSE_RIGHT {
            c"r".as_ptr()
        } else if which_button == MOUSE_MIDDLE {
            c"m".as_ptr()
        } else if which_button == MOUSE_X1 {
            c"x1".as_ptr()
        } else if which_button == MOUSE_X2 {
            c"x2".as_ptr()
        } else {
            c"?".as_ptr()
        };

        let mut mod_str: [c_char; 5] = [
            if (mod_mask & MOD_MASK_SHIFT) != 0 {
                b's' as c_char
            } else {
                b' ' as c_char
            },
            if (mod_mask & MOD_MASK_CTRL) != 0 {
                b'c' as c_char
            } else {
                b' ' as c_char
            },
            if (mod_mask & MOD_MASK_ALT) != 0 {
                b'a' as c_char
            } else {
                b' ' as c_char
            },
            if (mod_mask & MOD_MASK_META) != 0 {
                b'm' as c_char
            } else {
                b' ' as c_char
            },
            NUL,
        ];

        let mut argv: [typval_T; 4] = [
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_number: varnumber_T::from((*click_defs.add(col as usize)).tabnr),
                },
            },
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval { v_number: clicks },
            },
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_string: btn_str.cast_mut(),
                },
            },
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval {
                    v_string: mod_str.as_mut_ptr(),
                },
            },
        ];

        let mut rettv: typval_T = std::mem::zeroed();
        call_vim_function(
            (*click_defs.add(col as usize)).func,
            argv.len() as c_int,
            argv.as_mut_ptr(),
            &raw mut rettv,
        );
        tv_clear(&raw mut rettv);
        got_click = false;
    }
}

/// Translate window coordinates to buffer position without any side effects.
/// Returns IN_BUFFER and sets `mpos` (0-based column) when in buffer text.
unsafe fn get_fpos_of_mouse(mpos: *mut pos_T, wpp: *mut *mut win_T) -> c_int {
    unsafe {
        let mut grid = mouse_grid;
        let mut row = mouse_row;
        let mut col = mouse_col;

        if row < 0 || col < 0 {
            return IN_UNKNOWN;
        }

        let wp = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
        if wp.is_null() {
            return IN_UNKNOWN;
        }
        if !wpp.is_null() {
            *wpp = wp;
        }
        let winrow = row;
        let wincol = col;

        let below_buffer = mouse_comp_pos(wp, &raw mut row, &raw mut col, &raw mut (*mpos).lnum);

        if !below_buffer
            && *(*wp).w_onebuf_opt.wo_stc != NUL
            && (if (*wp).w_onebuf_opt.wo_rl != 0 {
                wincol >= (*wp).w_view_width - win_col_off(wp)
            } else {
                wincol < win_col_off(wp)
            })
        {
            return MOUSE_STATUSCOL;
        }

        if winrow >= (*wp).w_view_height + (*wp).w_status_height {
            if mouse_grid <= 1
                && i64::from(mouse_row) < i64::from(Rows) - p_ch
                && i64::from(mouse_row) >= i64::from(Rows) - p_ch - i64::from(global_stl_height())
            {
                return IN_STATUS_LINE;
            }
            return IN_UNKNOWN;
        } else if winrow >= (*wp).w_view_height {
            return IN_STATUS_LINE;
        }

        if winrow < 0 && winrow + (*wp).w_winbar_height >= 0 {
            return MOUSE_WINBAR;
        }

        if wincol >= (*wp).w_view_width {
            return IN_SEP_LINE;
        }

        if (wpp.is_null() && wp != curwin) || below_buffer {
            return IN_UNKNOWN;
        }

        (*mpos).col = vcol2col(wp, (*mpos).lnum, col, &raw mut (*mpos).coladd);
        IN_BUFFER
    }
}

unsafe fn do_popup(which_button: c_int, m_pos_flag: c_int, mut m_pos: pos_T) -> c_int {
    unsafe {
        let mut jump_flags = 0;
        if CStr::from_ptr(p_mousem).to_bytes() == b"popup_setpos" {
            if Visual.active {
                if m_pos_flag != IN_BUFFER {
                    jump_flags = MOUSE_MAY_STOP_VIS;
                } else if Visual.mode == b'V' as c_int {
                    if ((*curwin).w_cursor.lnum <= Visual.start.lnum
                        && (m_pos.lnum < (*curwin).w_cursor.lnum || Visual.start.lnum < m_pos.lnum))
                        || (Visual.start.lnum < (*curwin).w_cursor.lnum
                            && (m_pos.lnum < Visual.start.lnum
                                || (*curwin).w_cursor.lnum < m_pos.lnum))
                    {
                        jump_flags = MOUSE_MAY_STOP_VIS;
                    }
                } else if (ltoreq((*curwin).w_cursor, Visual.start)
                    && (lt(m_pos, (*curwin).w_cursor) || lt(Visual.start, m_pos)))
                    || (lt(Visual.start, (*curwin).w_cursor)
                        && (lt(m_pos, Visual.start) || lt((*curwin).w_cursor, m_pos)))
                {
                    jump_flags = MOUSE_MAY_STOP_VIS;
                } else if Visual.mode == Ctrl_V {
                    let mut leftcol: colnr_T = 0;
                    let mut rightcol: colnr_T = 0;
                    getvcols(
                        curwin,
                        &raw mut (*curwin).w_cursor,
                        &raw mut Visual.start,
                        &raw mut leftcol,
                        &raw mut rightcol,
                        0,
                    );
                    getvcol(
                        curwin,
                        &raw mut m_pos,
                        ptr::null_mut(),
                        &raw mut m_pos.col,
                        ptr::null_mut(),
                        0,
                    );
                    if m_pos.col < leftcol || m_pos.col > rightcol {
                        jump_flags = MOUSE_MAY_STOP_VIS;
                    }
                }
            } else {
                jump_flags = MOUSE_MAY_STOP_VIS;
            }
        }
        if jump_flags != 0 {
            jump_flags = jump_to_mouse(jump_flags, ptr::null_mut(), which_button);
            redraw_curbuf_later(if Visual.active {
                UPD_INVERTED
            } else {
                UPD_VALID
            });
            update_screen();
            setcursor();
            ui_flush();
        }
        show_popupmenu();
        got_click = false;
        jump_flags
    }
}

/// Do the appropriate action for the current mouse click in the current mode.
pub unsafe fn do_mouse(
    oap: *mut oparg_T,
    c: c_int,
    mut dir: c_int,
    count: c_int,
    fixindent: bool,
) -> bool {
    unsafe {
        let mut which_button: c_int;
        let mut is_click = false;
        let mut is_drag = false;
        static mut in_tab_line: bool = false;
        static mut orig_cursor: pos_T = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };

        loop {
            which_button = get_mouse_button(key2termcap1(c), &raw mut is_click, &raw mut is_drag);
            if is_drag && KeyStuffed == 0 && vpeekc() != 0 {
                let save_mouse_grid = mouse_grid;
                let save_mouse_row = mouse_row;
                let save_mouse_col = mouse_col;

                let nc = safe_vgetc();
                if c == nc {
                    continue;
                }
                vungetc(nc);
                mouse_grid = save_mouse_grid;
                mouse_row = save_mouse_row;
                mouse_col = save_mouse_col;
            }
            break;
        }

        if c == K_MOUSEMOVE {
            return false;
        }

        if is_click {
            got_click = true;
            in_tab_line = false;
        } else {
            if !got_click {
                return false;
            }
            if !is_drag {
                got_click = false;
                if in_tab_line {
                    in_tab_line = false;
                    return false;
                }
            }
        }

        if is_click && (mod_mask & MOD_MASK_CTRL) != 0 && which_button == MOUSE_RIGHT {
            if (State & MODE_INSERT) != 0 {
                stuffcharReadbuff(Ctrl_O);
            }
            if count > 1 {
                stuffnumReadbuff(count);
            }
            stuffcharReadbuff(Ctrl_T);
            got_click = false;
            if (State & MODE_INSERT) == 0 {
                exec_stuffed(ptr::null_mut());
            }
            return false;
        }

        if (mod_mask & MOD_MASK_CTRL) != 0 && which_button != MOUSE_LEFT {
            return false;
        }

        if (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL | MOD_MASK_ALT | MOD_MASK_META)) != 0
            && (!is_click || (mod_mask & MOD_MASK_MULTI_CLICK) != 0 || which_button == MOUSE_MIDDLE)
            && !((mod_mask & (MOD_MASK_SHIFT | MOD_MASK_ALT)) != 0
                && mouse_model_popup()
                && which_button == MOUSE_LEFT)
            && !((mod_mask & MOD_MASK_ALT) != 0
                && !mouse_model_popup()
                && which_button == MOUSE_RIGHT)
        {
            return false;
        }

        if !is_click && which_button == MOUSE_MIDDLE {
            return false;
        }

        let mut regname = if !oap.is_null() { (*oap).regname } else { 0 };
        if which_button == MOUSE_MIDDLE {
            if State == MODE_NORMAL {
                if !oap.is_null() && (*oap).op_type != OP_NOP {
                    clearopbeep(oap);
                    return false;
                }

                if Visual.active {
                    if Visual.select {
                        stuffcharReadbuff(Ctrl_G);
                        stuffReadbuff(c"\"+p".as_ptr());
                    } else {
                        if regname != 0 {
                            stuffcharReadbuff(b'"' as c_int);
                            stuffcharReadbuff(regname);
                        }
                        stuffcharReadbuff(b'y' as c_int);
                        stuffcharReadbuff(K_MIDDLEMOUSE);
                    }
                    exec_stuffed(ptr::null_mut());
                    return false;
                }
            } else if (State & MODE_INSERT) == 0 {
                return false;
            }

            if (State & MODE_INSERT) != 0 {
                if regname == b'.' as c_int {
                    insert_reg(regname, ptr::null_mut(), true);
                } else {
                    if regname == 0 && eval_has_provider(c"clipboard".as_ptr(), false) {
                        regname = b'*' as c_int;
                    }
                    let mut reg: *mut yankreg_T = ptr::null_mut();
                    if (State & REPLACE_FLAG) != 0 && !yank_register_mline(regname, &raw mut reg) {
                        insert_reg(regname, reg, true);
                    } else {
                        do_put(
                            regname,
                            reg,
                            BACKWARD,
                            1,
                            (if fixindent { PUT_FIXINDENT } else { 0 }) | PUT_CURSEND,
                        );

                        redo_append_char(Ctrl_R);
                        redo_append_char(if fixindent { Ctrl_P } else { Ctrl_O });
                        redo_append_char(if regname == 0 { b'"' as c_int } else { regname });
                    }
                }
                return false;
            }
        }

        let mut jump_flags = if is_click {
            0
        } else {
            MOUSE_FOCUS | MOUSE_DID_MOVE
        };
        let old_curwin = curwin;

        let defs = &raw mut tab_page_click_defs;
        if !(*defs).is_empty() {
            if mouse_grid <= 1 && mouse_row == 0 && (*firstwin).w_winrow > 0 {
                if is_drag {
                    if in_tab_line {
                        move_tab_to_mouse();
                    }
                    return false;
                }

                if is_click && mouse_col < Columns {
                    let tabnr = (*defs).as_slice()[mouse_col as usize].tabnr;
                    in_tab_line = true;

                    match (*defs).as_slice()[mouse_col as usize].type_ {
                        kStlClickDisabled => {}
                        kStlClickTabSwitch => {
                            if which_button != MOUSE_MIDDLE {
                                if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                                    end_visual_mode();
                                    tabpage_new();
                                    tabpage_move(if tabnr == 0 { 9999 } else { tabnr - 1 });
                                } else {
                                    goto_tabpage(tabnr);
                                    if curwin != old_curwin {
                                        end_visual_mode();
                                    }
                                }
                            } else {
                                mouse_tab_close(tabnr);
                            }
                        }
                        kStlClickTabClose => {
                            mouse_tab_close(tabnr);
                        }
                        kStlClickFuncRun => {
                            call_click_def_func((*defs).as_mut_ptr(), mouse_col, which_button);
                        }
                        _ => {}
                    }
                }
                return true;
            } else if is_drag && in_tab_line {
                move_tab_to_mouse();
                return false;
            }
        }

        if is_click && (mod_mask & MOD_MASK_CTRL) != 0 && which_button == MOUSE_LEFT {
            let mut pos = ZERO_POS;
            let mut wp: *mut win_T = ptr::null_mut();
            if get_fpos_of_mouse(&raw mut pos, &raw mut wp) == IN_BUFFER
                && !bt_quickfix((*wp).w_buffer)
            {
                got_click = false;
                if (State & MODE_INSERT) == 0 {
                    pos.coladd = 0;
                    mc_toggle((*wp).w_buffer, pos, false, kNone);
                }
                return false;
            }
        }

        let mut m_pos_flag = 0;
        let mut m_pos = ZERO_POS;
        if mouse_model_popup() {
            m_pos_flag = get_fpos_of_mouse(&raw mut m_pos, ptr::null_mut());
            if (m_pos_flag & (IN_STATUS_LINE | MOUSE_WINBAR | MOUSE_STATUSCOL)) == 0
                && which_button == MOUSE_RIGHT
                && (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
            {
                if !is_click {
                    return false;
                }
                return (do_popup(which_button, m_pos_flag, m_pos) & CURSOR_MOVED) != 0;
            }
            if (m_pos_flag & (IN_STATUS_LINE | MOUSE_WINBAR | MOUSE_STATUSCOL)) == 0
                && which_button == MOUSE_LEFT
                && (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_ALT)) != 0
            {
                which_button = MOUSE_RIGHT;
                mod_mask &= !MOD_MASK_SHIFT;
            }
        }

        let mut end_visual = ZERO_POS;
        let mut start_visual = ZERO_POS;
        let mouse_can_visual = ui_mouse_has(kMouseVisual);
        if (State & (MODE_NORMAL | MODE_INSERT)) != 0
            && (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
        {
            if which_button == MOUSE_LEFT && mouse_can_visual {
                if is_click {
                    if Visual.active {
                        jump_flags |= MOUSE_MAY_STOP_VIS;
                    }
                } else {
                    jump_flags |= MOUSE_MAY_VIS;
                }
            } else if which_button == MOUSE_RIGHT && mouse_can_visual {
                if is_click && Visual.active {
                    if lt((*curwin).w_cursor, Visual.start) {
                        start_visual = (*curwin).w_cursor;
                        end_visual = Visual.start;
                    } else {
                        start_visual = Visual.start;
                        end_visual = (*curwin).w_cursor;
                    }
                }
                jump_flags |= MOUSE_MAY_VIS;
                jump_flags |= MOUSE_FOCUS;
            } else if which_button == MOUSE_RIGHT {
                jump_flags |= MOUSE_FOCUS;
            }
        }

        if !is_drag && !oap.is_null() && (*oap).op_type != OP_NOP {
            got_click = false;
            (*oap).motion_type = kMTCharWise;
        }

        if !is_click && !is_drag {
            jump_flags |= MOUSE_RELEASED;
        }

        let old_active = Visual.active;
        let save_cursor = (*curwin).w_cursor;

        if !Visual.active || mouse_can_visual {
            jump_flags = jump_to_mouse(
                jump_flags,
                if oap.is_null() {
                    ptr::null_mut()
                } else {
                    &raw mut (*oap).inclusive
                },
                which_button,
            );
        }

        if mod_mask == 0
            && !is_drag
            && (jump_flags & (MOUSE_FOLD_CLOSE | MOUSE_FOLD_OPEN)) != 0
            && which_button == MOUSE_LEFT
        {
            if (jump_flags & MOUSE_FOLD_OPEN) != 0 {
                openFold((*curwin).w_cursor, 1);
            } else {
                closeFold((*curwin).w_cursor, 1);
            }
            if curwin == old_curwin {
                (*curwin).w_cursor = save_cursor;
            }
        }

        let moved = (jump_flags & CURSOR_MOVED) != 0;
        let in_winbar = (jump_flags & MOUSE_WINBAR) != 0;
        let in_statuscol = (jump_flags & MOUSE_STATUSCOL) != 0;
        let in_status_line = (jump_flags & IN_STATUS_LINE) != 0;
        let in_global_statusline = in_status_line && global_stl_height() > 0;
        let in_sep_line = (jump_flags & IN_SEP_LINE) != 0;

        if (in_winbar || in_status_line || in_statuscol) && is_click {
            let mut click_grid = mouse_grid;
            let mut click_row = mouse_row;
            let mut click_col = mouse_col;
            let wp =
                mouse_find_win_inner(&raw mut click_grid, &raw mut click_row, &raw mut click_col);
            if wp.is_null() {
                return false;
            }

            let mut click_defs: *mut StlClickDefinition = if in_status_line {
                if (*wp).w_status_click_defs.is_empty() {
                    ptr::null_mut()
                } else {
                    (*wp).w_status_click_defs.as_mut_ptr()
                }
            } else if in_winbar {
                if (*wp).w_winbar_click_defs.is_empty() {
                    ptr::null_mut()
                } else {
                    (*wp).w_winbar_click_defs.as_mut_ptr()
                }
            } else {
                ptr::null_mut()
            };
            if in_statuscol && (*wp).w_onebuf_opt.wo_rl != 0 {
                click_col = (*wp).w_view_width - click_col - 1;
            }

            if in_statuscol {
                let lnum = get_vim_var_nr(VV_LNUM) as c_int;
                let virtnum = get_vim_var_nr(VV_VIRTNUM) as c_int;
                let Some(row_click_defs) = (*wp)
                    .w_statuscol_click_defs
                    .get_mut(&lnum)
                    .and_then(|m| m.get_mut(&virtnum))
                else {
                    return false;
                };
                if click_col >= row_click_defs.len() as c_int {
                    return false;
                }
                click_defs = row_click_defs.as_mut_ptr();
            }

            if in_global_statusline {
                click_defs = if (*curwin).w_status_click_defs.is_empty() {
                    ptr::null_mut()
                } else {
                    (*curwin).w_status_click_defs.as_mut_ptr()
                };
                click_col = mouse_col;
            }

            if in_status_line {
                let stl_wp = if in_global_statusline { curwin } else { wp };
                if click_col >= (*stl_wp).w_status_click_defs.len() as c_int {
                    return false;
                }
            }

            if !click_defs.is_null() {
                match (*click_defs.add(click_col as usize)).type_ {
                    kStlClickDisabled => {
                        if in_statuscol
                            && mouse_model_popup()
                            && which_button == MOUSE_RIGHT
                            && (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
                        {
                            do_popup(which_button, m_pos_flag, m_pos);
                        }
                    }
                    kStlClickFuncRun => {
                        call_click_def_func(click_defs, click_col, which_button);
                    }
                    _ => {
                        debug_assert!(
                            false,
                            "winbar, statusline and statuscolumn only support %@ for clicks"
                        );
                    }
                }
            }
            return false;
        } else if in_winbar || in_statuscol {
            return false;
        }

        if curwin != old_curwin && !oap.is_null() && (*oap).op_type != OP_NOP {
            clearop(oap);
        }

        if Visual.active && is_drag && get_scrolloff_value(curwin) != 0 {
            if mouse_row == 0 {
                mouse_dragging = 2;
            } else {
                mouse_dragging = 1;
            }
        }

        if is_drag && mouse_row < 0 && !in_status_line {
            scroll_redraw(0, 1);
            mouse_row = 0;
        }

        let old_mode = Visual.mode;
        if start_visual.lnum != 0 {
            if (mod_mask & MOD_MASK_ALT) != 0 {
                Visual.mode = Ctrl_V;
            }

            if Visual.mode == Ctrl_V {
                let mut leftcol: colnr_T = 0;
                let mut rightcol: colnr_T = 0;
                getvcols(
                    curwin,
                    &raw mut start_visual,
                    &raw mut end_visual,
                    &raw mut leftcol,
                    &raw mut rightcol,
                    0,
                );
                if (*curwin).w_curswant > (leftcol + rightcol) / 2 {
                    end_visual.col = leftcol;
                } else {
                    end_visual.col = rightcol;
                }
                if (*curwin).w_cursor.lnum >= (start_visual.lnum + end_visual.lnum) / 2 {
                    end_visual.lnum = start_visual.lnum;
                }

                start_visual = (*curwin).w_cursor;
                (*curwin).w_cursor = end_visual;
                coladvance(curwin, end_visual.col);
                Visual.start = (*curwin).w_cursor;
                (*curwin).w_cursor = start_visual;
            } else if lt((*curwin).w_cursor, start_visual) {
                Visual.start = end_visual;
            } else if lt(end_visual, (*curwin).w_cursor) {
                Visual.start = start_visual;
            } else if end_visual.lnum == start_visual.lnum {
                if (*curwin).w_cursor.col - start_visual.col
                    > end_visual.col - (*curwin).w_cursor.col
                {
                    Visual.start = start_visual;
                } else {
                    Visual.start = end_visual;
                }
            } else {
                let diff = ((*curwin).w_cursor.lnum - start_visual.lnum)
                    - (end_visual.lnum - (*curwin).w_cursor.lnum);

                if diff > 0 {
                    Visual.start = start_visual;
                } else if diff < 0
                    || (*curwin).w_cursor.col < (start_visual.col + end_visual.col) / 2
                {
                    Visual.start = end_visual;
                } else {
                    Visual.start = start_visual;
                }
            }
        } else if (State & MODE_INSERT) != 0 && Visual.active {
            stuffcharReadbuff(Ctrl_O);
        }

        if which_button == MOUSE_MIDDLE {
            if regname == 0 && eval_has_provider(c"clipboard".as_ptr(), false) {
                regname = b'*' as c_int;
            }
            let mut reg: *mut yankreg_T = ptr::null_mut();
            if yank_register_mline(regname, &raw mut reg) {
                if mouse_past_bottom {
                    dir = FORWARD;
                }
            } else if mouse_past_eol {
                dir = FORWARD;
            }

            let (c1, c2) = if fixindent {
                (
                    if dir == BACKWARD {
                        b'[' as c_int
                    } else {
                        b']' as c_int
                    },
                    b'p' as c_int,
                )
            } else {
                (
                    if dir == FORWARD {
                        b'p' as c_int
                    } else {
                        b'P' as c_int
                    },
                    0,
                )
            };
            prep_redo(
                true,
                false,
                CmdSpec {
                    regname,
                    count: count as c_long,
                    cmd: c1,
                    cmd2: c2,
                    ..CmdSpec::default()
                },
            );

            if restart_edit != 0 {
                where_paste_started = (*curwin).w_cursor;
            }
            do_put(
                regname,
                reg,
                dir,
                count,
                (if fixindent { PUT_FIXINDENT } else { 0 }) | PUT_CURSEND,
            );
        } else if ((mod_mask & MOD_MASK_CTRL) != 0
            || (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK)
            && bt_quickfix(curbuf)
        {
            if (*curwin).w_llist_ref.is_null() {
                do_cmdline_cmd(c".cc".as_ptr());
            } else {
                do_cmdline_cmd(c".ll".as_ptr());
            }
            got_click = false;
        } else if (*curbuf).b_help && (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
            if (State & MODE_INSERT) != 0 {
                stuffcharReadbuff(Ctrl_O);
            }
            stuffcharReadbuff(Ctrl_RSB);
            got_click = false;
            if (State & MODE_INSERT) == 0 {
                exec_stuffed(ptr::null_mut());
            }
        } else if (mod_mask & MOD_MASK_SHIFT) != 0 {
            if (State & MODE_INSERT) != 0 || (Visual.active && Visual.select) {
                stuffcharReadbuff(Ctrl_O);
            }
            if which_button == MOUSE_LEFT {
                stuffcharReadbuff(b'*' as c_int);
            } else {
                stuffcharReadbuff(b'#' as c_int);
            }
            if (State & MODE_INSERT) == 0 {
                exec_stuffed(ptr::null_mut());
            }
        } else if in_status_line || in_sep_line {
            // Do nothing if on status line or vertical separator
        } else if (mod_mask & MOD_MASK_MULTI_CLICK) != 0
            && (State & (MODE_NORMAL | MODE_INSERT)) != 0
            && mouse_can_visual
        {
            if is_click || !Visual.active {
                if Visual.active {
                    orig_cursor = Visual.start;
                } else {
                    Visual.start = (*curwin).w_cursor;
                    orig_cursor = Visual.start;
                    Visual.active = true;
                    Visual.reselect = 1;
                    may_start_select(b'o' as c_int);
                    setmouse();
                }
                if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                    if (mod_mask & MOD_MASK_ALT) != 0 {
                        Visual.mode = Ctrl_V;
                    } else {
                        Visual.mode = b'v' as c_int;
                    }
                } else if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_3CLICK {
                    Visual.mode = b'V' as c_int;
                } else if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_4CLICK {
                    Visual.mode = Ctrl_V;
                }
            }
            if (mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                let mut pos: *mut pos_T = ptr::null_mut();

                if is_click {
                    end_visual = (*curwin).w_cursor;
                    while ascii_iswhite(gchar_pos(&raw mut end_visual)) {
                        inc(&raw mut end_visual);
                    }
                    if !oap.is_null() {
                        (*oap).motion_type = kMTCharWise;
                    }
                    if !oap.is_null()
                        && Visual.mode == b'v' as c_int
                        && !vim_iswordc(gchar_pos(&raw mut end_visual))
                        && equalpos((*curwin).w_cursor, Visual.start)
                    {
                        pos = findmatch(oap, 0);
                        if !pos.is_null() {
                            (*curwin).w_cursor = *pos;
                            if (*oap).motion_type == kMTLineWise {
                                Visual.mode = b'V' as c_int;
                            } else if *p_sel == b'e' as c_char {
                                if lt((*curwin).w_cursor, Visual.start) {
                                    Visual.start.col += 1;
                                } else {
                                    (*curwin).w_cursor.col += 1;
                                }
                            }
                        }
                    }
                }

                if pos.is_null() && (is_click || is_drag) {
                    if lt((*curwin).w_cursor, orig_cursor) {
                        find_start_of_word(&raw mut (*curwin).w_cursor);
                        find_end_of_word(&raw mut Visual.start);
                    } else {
                        find_start_of_word(&raw mut Visual.start);
                        if *p_sel == b'e' as c_char && *get_cursor_pos_ptr() != NUL {
                            (*curwin).w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                        }
                        find_end_of_word(&raw mut (*curwin).w_cursor);
                    }
                }
                (*curwin).w_set_curswant = true;
            }
            if is_click {
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else if Visual.active && !old_active {
            if (mod_mask & MOD_MASK_ALT) != 0 {
                Visual.mode = Ctrl_V;
            } else {
                Visual.mode = b'v' as c_int;
            }
        }

        if (!Visual.active && old_active && mode_displayed)
            || (Visual.active
                && p_smd != 0
                && msg_silent == 0
                && (!old_active || Visual.mode != old_mode))
        {
            redraw_cmdline = true;
        }

        moved
    }
}

pub unsafe fn ins_mouse(c: c_int) {
    unsafe {
        let old_curwin = curwin;

        undisplay_dollar();
        let mut tpos = (*curwin).w_cursor;
        if do_mouse(ptr::null_mut(), c, BACKWARD, 1, false) {
            let new_curwin = curwin;

            if curwin != old_curwin && win_valid(old_curwin) {
                curwin = old_curwin;
                curbuf = (*curwin).w_buffer;
                if bt_prompt(curbuf) {
                    (*curbuf).b_prompt_insert = b'A' as c_int;
                }
            }
            start_arrow(
                if curwin == old_curwin {
                    &raw mut tpos
                } else {
                    ptr::null_mut()
                },
                true,
                0,
            );
            if curwin != new_curwin && win_valid(new_curwin) {
                curwin = new_curwin;
                curbuf = (*curwin).w_buffer;
            }
            set_can_cindent(true);
        }

        redraw_statuslines();
    }
}

/// Common mouse wheel scrolling, shared between Insert mode and NV modes.
pub unsafe fn do_mousescroll(cap: *mut cmdarg_T) {
    unsafe {
        let shift_or_ctrl = (mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0;

        if (*cap).arg == MSCR_UP || (*cap).arg == MSCR_DOWN {
            if (State & MODE_NORMAL) != 0 && shift_or_ctrl {
                pagescroll(if (*cap).arg != 0 { FORWARD } else { BACKWARD }, 1, false);
            } else {
                if shift_or_ctrl {
                    (*cap).count1 = (*curwin).w_botline - (*curwin).w_topline;
                } else {
                    (*cap).count1 = p_mousescroll_vert as c_int;
                }
                if (*cap).count1 > 0 {
                    (*cap).count0 = (*cap).count1;
                    nv_scroll_line(cap);
                }
            }
        } else {
            let step = if shift_or_ctrl {
                (*curwin).w_view_width
            } else {
                p_mousescroll_hor as c_int
            };
            let leftcol = ((*curwin).w_leftcol
                + if (*cap).arg == MSCR_RIGHT {
                    -step
                } else {
                    step
                })
            .max(0);
            do_mousescroll_horiz(leftcol);
        }
    }
}

/// Implementation for scrolling in Insert mode in direction "dir", which is one
/// of the MSCR_ values.
pub unsafe fn ins_mousescroll(dir: c_int) {
    unsafe {
        let mut cap: cmdarg_T = std::mem::zeroed();
        let mut oa: oparg_T = std::mem::zeroed();
        clear_oparg(&raw mut oa);
        cap.oap = &raw mut oa;
        cap.arg = dir;

        match dir {
            MSCR_UP => cap.cmdchar = K_MOUSEUP,
            MSCR_DOWN => cap.cmdchar = K_MOUSEDOWN,
            MSCR_LEFT => cap.cmdchar = K_MOUSELEFT,
            MSCR_RIGHT => cap.cmdchar = K_MOUSERIGHT,
            _ => siemsg!(c"Invalid ins_mousescroll() argument: %d".as_ptr(), dir),
        }

        let old_curwin = curwin;
        if mouse_row >= 0 && mouse_col >= 0 {
            let mut grid = mouse_grid;
            let mut row = mouse_row;
            let mut col = mouse_col;
            curwin = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
            if curwin.is_null() {
                curwin = old_curwin;
                return;
            }
            curbuf = (*curwin).w_buffer;
        }

        if curwin == old_curwin {
            if pum_visible() {
                return;
            }

            undisplay_dollar();
        }

        let mut orig_cursor = (*curwin).w_cursor;

        do_mousescroll(&raw mut cap);

        (*curwin).w_redr_status = true;
        curwin = old_curwin;
        curbuf = (*curwin).w_buffer;

        if !equalpos((*curwin).w_cursor, orig_cursor) {
            start_arrow(&raw mut orig_cursor, true, 0);
            set_can_cindent(true);
        }
    }
}

/// Command-line mode implementation for scrolling in direction "dir", which is
/// one of the MSCR_ values. Scrolls the completion info popup when the mouse
/// pointer is on top of it.
/// Returns true when the info popup was scrolled.
pub unsafe fn cmdline_mousescroll(dir: c_int) -> bool {
    unsafe {
        let mut cap: cmdarg_T = std::mem::zeroed();
        let mut oa: oparg_T = std::mem::zeroed();
        clear_oparg(&raw mut oa);
        cap.oap = &raw mut oa;
        cap.arg = dir;

        match dir {
            MSCR_UP => cap.cmdchar = K_MOUSEUP,
            MSCR_DOWN => cap.cmdchar = K_MOUSEDOWN,
            MSCR_LEFT => cap.cmdchar = K_MOUSELEFT,
            MSCR_RIGHT => cap.cmdchar = K_MOUSERIGHT,
            _ => {}
        }

        if mouse_row < 0 || mouse_col < 0 {
            return false;
        }

        let mut grid = mouse_grid;
        let mut row = mouse_row;
        let mut col = mouse_col;

        let wp = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
        if wp.is_null() || (*wp).w_kind != kWinInfo {
            return false;
        }

        let old_curwin = curwin;

        curwin = wp;
        curbuf = (*wp).w_buffer;
        do_mousescroll(&raw mut cap);
        curwin = old_curwin;
        curbuf = (*curwin).w_buffer;

        update_screen();
        true
    }
}

/// Return true if "c" is a mouse key.
pub fn is_mouse_key(c: c_int) -> bool {
    matches!(
        c,
        K_LEFTMOUSE
            | K_LEFTMOUSE_NM
            | K_LEFTDRAG
            | K_LEFTRELEASE
            | K_LEFTRELEASE_NM
            | K_MOUSEMOVE
            | K_MIDDLEMOUSE
            | K_MIDDLEDRAG
            | K_MIDDLERELEASE
            | K_RIGHTMOUSE
            | K_RIGHTDRAG
            | K_RIGHTRELEASE
            | K_MOUSEDOWN
            | K_MOUSEUP
            | K_MOUSELEFT
            | K_MOUSERIGHT
            | K_X1MOUSE
            | K_X1DRAG
            | K_X1RELEASE
            | K_X2MOUSE
            | K_X2DRAG
            | K_X2RELEASE
    )
}

/// Return true when 'mousemodel' is set to "popup" or "popup_setpos".
unsafe fn mouse_model_popup() -> bool {
    unsafe { *p_mousem == b'p' as c_char }
}

/// Reset the window being dragged. To be called when switching tab page.
pub unsafe fn reset_dragwin() {
    unsafe {
        dragwin = ptr::null_mut();
    }
}

#[inline]
unsafe fn jump_to_mouse_retnomove(
    flags: c_int,
    status_line_offset: c_int,
    sep_line_offset: c_int,
    on_winbar: bool,
    on_statuscol: bool,
) -> c_int {
    unsafe {
        if status_line_offset != 0 {
            return IN_STATUS_LINE;
        }
        if sep_line_offset != 0 {
            return IN_SEP_LINE;
        }
        if on_winbar {
            return IN_OTHER_WIN | MOUSE_WINBAR;
        }
        if on_statuscol {
            return IN_OTHER_WIN | MOUSE_STATUSCOL;
        }
        if (flags & MOUSE_MAY_STOP_VIS) != 0 {
            end_visual_mode();
            redraw_curbuf_later(UPD_INVERTED);
        }
        IN_BUFFER
    }
}

/// Move the cursor to the specified row and column on the screen.
/// Change current window if necessary. Returns an integer with the
/// CURSOR_MOVED bit set if the cursor has moved or unset otherwise.
pub unsafe fn jump_to_mouse(mut flags: c_int, inclusive: *mut bool, which_button: c_int) -> c_int {
    unsafe {
        static mut status_line_offset: c_int = 0;
        static mut sep_line_offset: c_int = 0;
        static mut on_status_line: bool = false;
        static mut on_sep_line: bool = false;
        static mut on_winbar: bool = false;
        static mut on_statuscol: bool = false;
        static mut prev_row: c_int = -1;
        static mut prev_col: c_int = -1;
        static mut did_drag: c_int = 0;

        let mut row = mouse_row;
        let mut col = mouse_col;
        let mut grid = mouse_grid;
        let keep_focus = (flags & MOUSE_FOCUS) != 0;

        mouse_past_bottom = false;
        mouse_past_eol = false;

        if (flags & MOUSE_RELEASED) != 0 {
            if !dragwin.is_null() && did_drag == 0 {
                flags &= !(MOUSE_FOCUS | MOUSE_DID_MOVE);
            }
            dragwin = ptr::null_mut();
            did_drag = 0;
        }

        if (flags & MOUSE_DID_MOVE) != 0 && prev_row == mouse_row && prev_col == mouse_col {
            return jump_to_mouse_retnomove(
                flags,
                status_line_offset,
                sep_line_offset,
                on_winbar,
                on_statuscol,
            );
        }

        prev_row = mouse_row;
        prev_col = mouse_col;

        if (flags & MOUSE_SETPOS) != 0 {
            return jump_to_mouse_retnomove(
                flags,
                status_line_offset,
                sep_line_offset,
                on_winbar,
                on_statuscol,
            );
        }

        if row < 0 || col < 0 {
            return IN_UNKNOWN;
        }

        let wp = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
        if wp.is_null() {
            return IN_UNKNOWN;
        }

        let below_window =
            grid == DEFAULT_GRID_HANDLE && row + (*wp).w_winbar_height >= (*wp).w_height;
        on_status_line = below_window && row + (*wp).w_winbar_height - (*wp).w_height + 1 == 1;
        on_sep_line =
            grid == DEFAULT_GRID_HANDLE && col >= (*wp).w_width && col - (*wp).w_width + 1 == 1;
        on_winbar = row < 0 && row + (*wp).w_winbar_height >= 0;
        on_statuscol = !below_window
            && !on_status_line
            && !on_sep_line
            && !on_winbar
            && *(*wp).w_onebuf_opt.wo_stc != NUL
            && (if (*wp).w_onebuf_opt.wo_rl != 0 {
                col >= (*wp).w_view_width - win_col_off(wp)
            } else {
                col < win_col_off(wp)
            });

        if on_status_line && on_sep_line {
            if stl_connected(wp) {
                on_sep_line = false;
            } else {
                on_status_line = false;
            }
        }

        if keep_focus {
            row = mouse_row;
            col = mouse_col;
            grid = mouse_grid;
        }

        let old_curwin = curwin;
        let old_cursor = (*curwin).w_cursor;
        if !keep_focus {
            if on_winbar {
                return IN_OTHER_WIN | MOUSE_WINBAR;
            }

            if !on_statuscol {
                let fdc = win_fdccol_count(wp);
                dragwin = ptr::null_mut();

                if below_window {
                    status_line_offset = row + (*wp).w_winbar_height - (*wp).w_height + 1;
                    dragwin = wp;
                } else {
                    status_line_offset = 0;
                }

                if grid == DEFAULT_GRID_HANDLE && col >= (*wp).w_width {
                    sep_line_offset = col - (*wp).w_width + 1;
                    dragwin = wp;
                } else {
                    sep_line_offset = 0;
                }

                if status_line_offset != 0 && sep_line_offset != 0 {
                    if stl_connected(wp) {
                        sep_line_offset = 0;
                    } else {
                        status_line_offset = 0;
                    }
                }

                if Visual.active
                    && ((*wp).w_buffer != (*curwin).w_buffer
                        || (status_line_offset == 0
                            && sep_line_offset == 0
                            && (if (*wp).w_onebuf_opt.wo_rl != 0 {
                                col < (*wp).w_view_width - fdc
                            } else {
                                col >= fdc
                            })
                            && (flags & MOUSE_MAY_STOP_VIS) != 0))
                {
                    end_visual_mode();
                    redraw_curbuf_later(UPD_INVERTED);
                }

                if dragwin.is_null() || (flags & MOUSE_RELEASED) != 0 {
                    win_enter(wp, true);
                }
                if curwin != old_curwin {
                    set_mouse_topline(curwin);
                }
                if status_line_offset != 0 {
                    if curwin == old_curwin {
                        return IN_STATUS_LINE;
                    }
                    return IN_STATUS_LINE | CURSOR_MOVED;
                }
                if sep_line_offset != 0 {
                    if curwin == old_curwin {
                        return IN_SEP_LINE;
                    }
                    return IN_SEP_LINE | CURSOR_MOVED;
                }

                (*curwin).w_cursor.lnum = (*curwin).w_topline;
            }
        } else if status_line_offset != 0 {
            if which_button == MOUSE_LEFT && !dragwin.is_null() {
                let count =
                    row - (*dragwin).w_winrow - (*dragwin).w_height + 1 - status_line_offset;
                win_drag_status_line(dragwin, count);
                did_drag |= count;
            }
            return IN_STATUS_LINE;
        } else if sep_line_offset != 0 && which_button == MOUSE_LEFT {
            if !dragwin.is_null() {
                let count = col - (*dragwin).w_wincol - (*dragwin).w_width + 1 - sep_line_offset;
                win_drag_vsep_line(dragwin, count);
                did_drag |= count;
            }
            return IN_SEP_LINE;
        } else if on_status_line && which_button == MOUSE_RIGHT {
            return IN_STATUS_LINE;
        } else if on_winbar && which_button == MOUSE_RIGHT {
            return IN_OTHER_WIN | MOUSE_WINBAR;
        } else if on_statuscol && which_button == MOUSE_RIGHT {
            return IN_OTHER_WIN | MOUSE_STATUSCOL;
        } else {
            if (flags & MOUSE_MAY_STOP_VIS) != 0 {
                end_visual_mode();
                redraw_curbuf_later(UPD_INVERTED);
            }

            if grid == 0 {
                row -= (*curwin).w_grid_alloc.comp_row + (*curwin).w_grid.row_offset;
                col -= (*curwin).w_grid_alloc.comp_col + (*curwin).w_grid.col_offset;
            } else if grid != DEFAULT_GRID_HANDLE {
                row -= (*curwin).w_grid.row_offset;
                col -= (*curwin).w_grid.col_offset;
            }

            if row < 0 {
                let mut count = 0;
                let mut first = true;
                while (*curwin).w_topline > 1 {
                    if (*curwin).w_topfill < win_get_fill(curwin, (*curwin).w_topline) {
                        count += 1;
                    } else {
                        count += plines_win(curwin, (*curwin).w_topline - 1, true);
                    }
                    if !first && count > -row {
                        break;
                    }
                    first = false;
                    hasFolding(
                        curwin,
                        (*curwin).w_topline,
                        &raw mut (*curwin).w_topline,
                        ptr::null_mut(),
                    );
                    if (*curwin).w_topfill < win_get_fill(curwin, (*curwin).w_topline) {
                        (*curwin).w_topfill += 1;
                    } else {
                        (*curwin).w_topline -= 1;
                        (*curwin).w_topfill = 0;
                    }
                }
                check_topfill(curwin, false);
                (*curwin).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
                redraw_later(curwin, UPD_VALID);
                row = 0;
            } else if row >= (*curwin).w_view_height {
                let mut count = 0;
                let mut first = true;
                while (*curwin).w_topline < (*curbuf).b_ml.ml_line_count {
                    if (*curwin).w_topfill > 0 {
                        count += 1;
                    } else {
                        count += plines_win(curwin, (*curwin).w_topline, true);
                    }

                    if !first && count > row - (*curwin).w_view_height + 1 {
                        break;
                    }
                    first = false;

                    if (*curwin).w_topfill > 0 {
                        (*curwin).w_topfill -= 1;
                    } else {
                        if hasFolding(
                            curwin,
                            (*curwin).w_topline,
                            ptr::null_mut(),
                            &raw mut (*curwin).w_topline,
                        ) && (*curwin).w_topline == (*curbuf).b_ml.ml_line_count
                        {
                            break;
                        }
                        (*curwin).w_topline += 1;
                        (*curwin).w_topfill = win_get_fill(curwin, (*curwin).w_topline);
                    }
                }
                check_topfill(curwin, false);
                redraw_later(curwin, UPD_VALID);
                (*curwin).w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
                row = (*curwin).w_view_height - 1;
            } else if row == 0
                && mouse_dragging > 0
                && (*curwin).w_cursor.lnum == (*(*curwin).w_buffer).b_ml.ml_line_count
                && (*curwin).w_cursor.lnum == (*curwin).w_topline
            {
                (*curwin).w_valid &= !VALID_TOPLINE;
            }
        }

        let mut col_from_screen: colnr_T = -1;
        let mut mouse_fold_flags: c_int = 0;
        mouse_check_grid(&raw mut col_from_screen, &raw mut mouse_fold_flags);

        if mouse_comp_pos(
            curwin,
            &raw mut row,
            &raw mut col,
            &raw mut (*curwin).w_cursor.lnum,
        ) {
            mouse_past_bottom = true;
        }

        if (flags & MOUSE_MAY_VIS) != 0 && !Visual.active {
            Visual.start = old_cursor;
            Visual.active = true;
            Visual.reselect = 1;
            may_start_select(b'o' as c_int);
            setmouse();

            if p_smd != 0 && msg_silent == 0 {
                redraw_cmdline = true;
            }
        }

        if col_from_screen >= 0 {
            col = col_from_screen;
        }

        (*curwin).w_curswant = col;
        (*curwin).w_set_curswant = false;
        if coladvance(curwin, col) == FAIL {
            if !inclusive.is_null() {
                *inclusive = true;
            }
            mouse_past_eol = true;
        } else if !inclusive.is_null() {
            *inclusive = false;
        }

        let mut count = if on_statuscol {
            IN_OTHER_WIN | MOUSE_STATUSCOL
        } else {
            IN_BUFFER
        };
        if curwin != old_curwin
            || (*curwin).w_cursor.lnum != old_cursor.lnum
            || (*curwin).w_cursor.col != old_cursor.col
        {
            count |= CURSOR_MOVED;
        }

        count |= mouse_fold_flags;

        count
    }
}

/// Make a horizontal scroll to "leftcol".
/// Return true if the cursor moved, false otherwise.
unsafe fn do_mousescroll_horiz(leftcol: colnr_T) -> bool {
    unsafe {
        if (*curwin).w_onebuf_opt.wo_wrap != 0 {
            return false;
        }
        if (*curwin).w_leftcol == leftcol {
            return false;
        }

        if !virtual_active(curwin) && leftcol > scroll_line_len((*curwin).w_cursor.lnum) {
            (*curwin).w_cursor.lnum = find_longest_lnum();
            (*curwin).w_cursor.col = 0;
        }

        set_leftcol(leftcol)
    }
}

/// Normal and Visual modes implementation for scrolling in direction
/// "cap->arg", which is one of the MSCR_ values.
pub unsafe fn nv_mousescroll(cap: *mut cmdarg_T) {
    unsafe {
        let old_curwin = curwin;

        if mouse_row >= 0 && mouse_col >= 0 {
            let mut grid = mouse_grid;
            let mut row = mouse_row;
            let mut col = mouse_col;
            curwin = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
            if curwin.is_null() {
                curwin = old_curwin;
                return;
            }
            curbuf = (*curwin).w_buffer;
        }

        do_mousescroll(cap);

        (*curwin).w_redr_status = true;
        curwin = old_curwin;
        curbuf = (*curwin).w_buffer;
    }
}

/// Mouse clicks and drags.
pub unsafe fn nv_mouse(cap: *mut cmdarg_T) {
    unsafe {
        do_mouse((*cap).oap, (*cap).cmdchar, BACKWARD, (*cap).count1, false);
    }
}

/// Compute the buffer line position from the screen position "rowp" / "colp" in
/// window "win".
/// Returns true if the position is below the last line.
pub unsafe fn mouse_comp_pos(
    win: *mut win_T,
    rowp: *mut c_int,
    colp: *mut c_int,
    lnump: *mut linenr_T,
) -> bool {
    unsafe {
        let mut col = *colp;
        let mut row = *rowp;
        let mut retval = false;

        if (*win).w_onebuf_opt.wo_rl != 0 {
            col = (*win).w_view_width - 1 - col;
        }

        let mut lnum = (*win).w_topline;

        while row > 0 {
            let filler_lines = if lnum == (*win).w_topline {
                (*win).w_topfill
            } else {
                win_get_fill(win, lnum)
            };
            let mut count = plines_win_nofill(win, lnum, false);

            if (*win).w_skipcol > 0 && lnum == (*win).w_topline {
                let width1 = (*win).w_view_width - win_col_off(win);

                if width1 > 0 {
                    let skip_lines = if (*win).w_skipcol > width1 {
                        ((*win).w_skipcol - width1) / (width1 + win_col_off2(win)) + 1
                    } else if (*win).w_skipcol > 0 {
                        1
                    } else {
                        0
                    };

                    count -= skip_lines;
                }
            }

            if count + filler_lines > row {
                break;
            }

            hasFolding(win, lnum, ptr::null_mut(), &raw mut lnum);

            if lnum == (*(*win).w_buffer).b_ml.ml_line_count {
                retval = true;
                break;
            }
            row -= count + filler_lines;
            lnum += 1;
        }

        let mut virt_below: c_int = 0;
        let virt_lines = decor_virt_lines(
            win,
            lnum - 1,
            lnum,
            &raw mut virt_below,
            ptr::null_mut(),
            true,
        );
        let diff_fill = diff_check_fill(win, lnum);
        let skip_fill = if lnum == (*win).w_topline {
            virt_lines + diff_fill - (*win).w_topfill
        } else {
            0
        };

        let mut virtnum = row - (virt_lines + diff_fill - skip_fill);
        if row < virt_below - skip_fill {
            lnum -= 1;
            let mut virt_below_prev: c_int = 0;
            let virt_lines_prev = decor_virt_lines(
                win,
                lnum - 1,
                lnum,
                &raw mut virt_below_prev,
                ptr::null_mut(),
                true,
            );
            let diff_fill_prev = diff_check_fill(win, lnum - 1);
            virtnum = -row - 1 - virt_lines_prev - diff_fill_prev + virt_below_prev;
        } else if row < virt_lines + diff_fill - skip_fill {
            virtnum = -row;
        }
        set_vim_var_nr(VV_LNUM, varnumber_T::from(lnum));
        set_vim_var_nr(VV_VIRTNUM, varnumber_T::from(virtnum));

        row -= virt_lines + diff_fill - skip_fill;

        while lnum < (*(*win).w_buffer).b_ml.ml_line_count
            && decor_conceal_line(win, lnum - 1, false)
        {
            lnum += 1;
            hasFolding(win, lnum, ptr::null_mut(), &raw mut lnum);
        }

        if !retval {
            let off = win_col_off(win) - win_col_off2(win);
            col = col.max(off);
            col += row * ((*win).w_view_width - off);

            if lnum == (*win).w_topline {
                col += (*win).w_skipcol;
            }
        }

        if (*win).w_onebuf_opt.wo_wrap == 0 {
            col += (*win).w_leftcol;
        }

        col -= win_col_off(win);
        col = col.max(0);

        *colp = col;
        *rowp = row;
        *lnump = lnum;
        retval
    }
}

/// Find the window at "grid" position "*rowp" and "*colp". The positions are
/// updated to become relative to the top-left of the window inner area.
pub unsafe fn mouse_find_win_inner(
    gridp: *mut c_int,
    rowp: *mut c_int,
    colp: *mut c_int,
) -> *mut win_T {
    unsafe {
        let wp_grid = mouse_find_grid_win(gridp, rowp, colp);
        if !wp_grid.is_null() {
            return wp_grid;
        } else if *gridp > 1 {
            return ptr::null_mut();
        }

        let mut fp = topframe;
        *rowp -= (*firstwin).w_winrow;
        loop {
            if (*fp).fr_layout == FR_LEAF {
                break;
            }
            if (*fp).fr_layout == FR_ROW {
                fp = (*fp).fr_child;
                while !(*fp).fr_next.is_null() {
                    if *colp < (*fp).fr_width {
                        break;
                    }
                    *colp -= (*fp).fr_width;
                    fp = (*fp).fr_next;
                }
            } else {
                fp = (*fp).fr_child;
                while !(*fp).fr_next.is_null() {
                    if *rowp < (*fp).fr_height {
                        break;
                    }
                    *rowp -= (*fp).fr_height;
                    fp = (*fp).fr_next;
                }
            }
        }

        let mut wp = firstwin;
        while !wp.is_null() {
            if wp == (*fp).fr_win {
                *rowp -= (*wp).w_winbar_height;
                return wp;
            }
            wp = (*wp).w_next;
        }
        ptr::null_mut()
    }
}

/// Find the window at "grid" position "*rowp" and "*colp". The positions are
/// updated to become relative to the top-left of the window.
pub unsafe fn mouse_find_win_outer(
    gridp: *mut c_int,
    rowp: *mut c_int,
    colp: *mut c_int,
) -> *mut win_T {
    unsafe {
        let wp = mouse_find_win_inner(gridp, rowp, colp);
        if !wp.is_null() {
            *rowp += (*wp).w_winrow_off;
            *colp += (*wp).w_wincol_off;
        }
        wp
    }
}

unsafe fn mouse_find_grid_win(gridp: *mut c_int, rowp: *mut c_int, colp: *mut c_int) -> *mut win_T {
    unsafe {
        if *gridp == msg_grid.handle {
            *rowp += msg_grid_pos;
            *gridp = DEFAULT_GRID_HANDLE;
        } else if *gridp > 1 {
            let wp = get_win_by_grid_handle(*gridp);
            if !wp.is_null()
                && !(*wp).w_grid_alloc.chars.is_empty()
                && !((*wp).w_floating && !(*wp).w_config.mouse)
            {
                *rowp = (*rowp - (*wp).w_grid.row_offset).min((*wp).w_view_height - 1);
                *colp = (*colp - (*wp).w_grid.col_offset).min((*wp).w_view_width - 1);
                return wp;
            }
        } else if *gridp == 0 {
            let grid = ui_comp_mouse_focus(*rowp, *colp);
            if ptr::eq(grid, &raw const pum_grid) {
                *gridp = (*grid).handle;
                *rowp -= (*grid).comp_row;
                *colp -= (*grid).comp_col;
                return ptr::null_mut();
            } else {
                let mut wp = firstwin;
                while !wp.is_null() {
                    if !ptr::eq(&raw const (*wp).w_grid_alloc, grid) {
                        wp = (*wp).w_next;
                        continue;
                    }
                    *gridp = (*grid).handle;
                    *rowp -= (*wp).w_winrow + (*wp).w_grid.row_offset;
                    *colp -= (*wp).w_wincol + (*wp).w_grid.col_offset;
                    return wp;
                }
            }

            *gridp = DEFAULT_GRID_HANDLE;
        }
        ptr::null_mut()
    }
}

/// Convert a virtual (screen) column to a character column.
/// The first column is zero.
pub unsafe fn vcol2col(
    wp: *mut win_T,
    lnum: linenr_T,
    vcol: colnr_T,
    coladdp: *mut colnr_T,
) -> colnr_T {
    unsafe {
        let line = ml_get_buf((*wp).w_buffer, lnum);
        let mut csarg: CharsizeArg = std::mem::zeroed();
        let cstype: CSType = init_charsize_arg(&raw mut csarg, wp, lnum, line);
        let mut ci = utf_ptr2StrCharInfo(line);
        let mut cur_vcol: c_int = 0;
        while cur_vcol < vcol && *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            let next_vcol = cur_vcol
                + win_charsize(
                    cstype,
                    cur_vcol,
                    ci.ptr.cast_mut(),
                    ci.chr.value,
                    &raw mut csarg,
                    cli.cells,
                )
                .width;
            if next_vcol > vcol {
                break;
            }
            cur_vcol = next_vcol;
            ci = cli.next;
        }

        if !coladdp.is_null() {
            *coladdp = vcol - cur_vcol;
        }
        ci.ptr.offset_from(line.cast_const()) as colnr_T
    }
}

/// Set UI mouse depending on current mode and 'mouse'.
pub unsafe fn setmouse() {
    unsafe {
        ui_cursor_shape();
        ui_check_mouse();
    }
}

unsafe fn set_mouse_topline(wp: *mut win_T) {
    unsafe {
        orig_topline = (*wp).w_topline;
        orig_topfill = (*wp).w_topfill;
    }
}

/// Return length of line "lnum" for horizontal scrolling.
unsafe fn scroll_line_len(lnum: linenr_T) -> colnr_T {
    unsafe {
        let mut col: colnr_T = 0;
        let mut line = ml_get(lnum);
        if *line != NUL {
            loop {
                let numchar = win_chartabsize(curwin, line, col);
                line = line.add(utfc_ptr2len(line) as usize);
                if *line == NUL {
                    break;
                }
                col += numchar;
            }
        }
        col
    }
}

/// Find longest visible line number.
unsafe fn find_longest_lnum() -> linenr_T {
    unsafe {
        let mut ret: linenr_T = 0;

        if (*curwin).w_topline <= (*curwin).w_cursor.lnum
            && (*curwin).w_botline > (*curwin).w_cursor.lnum
            && (*curwin).w_botline <= (*curbuf).b_ml.ml_line_count + 1
        {
            let mut max: colnr_T = 0;

            for lnum in (*curwin).w_topline..(*curwin).w_botline {
                let len = scroll_line_len(lnum);
                if len > max {
                    max = len;
                    ret = lnum;
                } else if len == max
                    && (lnum - (*curwin).w_cursor.lnum).abs()
                        < (ret - (*curwin).w_cursor.lnum).abs()
                {
                    ret = lnum;
                }
            }
        } else {
            ret = (*curwin).w_cursor.lnum;
        }

        ret
    }
}

/// Check clicked cell on its grid
unsafe fn mouse_check_grid(vcolp: *mut colnr_T, flagsp: *mut c_int) {
    unsafe {
        let mut click_grid = mouse_grid;
        let mut click_row = mouse_row;
        let mut click_col = mouse_col;

        if mouse_find_win_inner(&raw mut click_grid, &raw mut click_row, &raw mut click_col)
            != curwin
            || (*curwin).w_redr_type != 0
        {
            return;
        }
        let mut start_row: c_int = 0;
        let mut start_col: c_int = 0;
        let gp = &*grid_adjust(
            &raw mut (*curwin).w_grid,
            &raw mut start_row,
            &raw mut start_col,
        );
        if gp.handle != click_grid || gp.chars.is_empty() {
            return;
        }
        click_row += start_row;
        click_col += start_col;
        if click_row < 0 || click_row >= gp.rows || click_col < 0 || click_col >= gp.cols {
            return;
        }

        let off = gp.line_offset[click_row as usize] + click_col as usize;
        let col_from_screen = gp.vcols[off];

        if col_from_screen >= 0 {
            *vcolp = col_from_screen;
        }

        if col_from_screen == -2 {
            *flagsp |= MOUSE_FOLD_OPEN;
        } else if col_from_screen == -3 {
            *flagsp |= MOUSE_FOLD_CLOSE;
        }
    }
}

/// "getmousepos()" function
pub unsafe fn f_getmousepos(_argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut row = mouse_row;
        let mut col = mouse_col;
        let mut grid = mouse_grid;
        let mut winid: varnumber_T = 0;
        let mut winrow: varnumber_T = 0;
        let mut wincol: varnumber_T = 0;
        let mut lnum: linenr_T = 0;
        let mut column: varnumber_T = 0;
        let mut coladd: colnr_T = 0;

        tv_dict_alloc_ret(rettv);
        let d = (*rettv).vval.v_dict;

        tv_dict_add_nr(
            d,
            c"screenrow".as_ptr(),
            "screenrow".len(),
            varnumber_T::from(mouse_row) + 1,
        );
        tv_dict_add_nr(
            d,
            c"screencol".as_ptr(),
            "screencol".len(),
            varnumber_T::from(mouse_col) + 1,
        );

        let wp = mouse_find_win_inner(&raw mut grid, &raw mut row, &raw mut col);
        if !wp.is_null() {
            let height = (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height;
            if row < height + (*wp).w_border_adj[2] {
                winid = varnumber_T::from((*wp).handle);
                winrow = varnumber_T::from(row + 1 + (*wp).w_winrow_off);
                wincol = varnumber_T::from(col + 1 + (*wp).w_wincol_off);
                if row >= 0 && row < (*wp).w_height && col >= 0 && col < (*wp).w_width {
                    mouse_comp_pos(wp, &raw mut row, &raw mut col, &raw mut lnum);
                    col = vcol2col(wp, lnum, col, &raw mut coladd);
                    column = varnumber_T::from(col) + 1;
                }
            }
        }
        tv_dict_add_nr(d, c"winid".as_ptr(), "winid".len(), winid);
        tv_dict_add_nr(d, c"winrow".as_ptr(), "winrow".len(), winrow);
        tv_dict_add_nr(d, c"wincol".as_ptr(), "wincol".len(), wincol);
        tv_dict_add_nr(d, c"line".as_ptr(), "line".len(), varnumber_T::from(lnum));
        tv_dict_add_nr(d, c"column".as_ptr(), "column".len(), column);
        tv_dict_add_nr(
            d,
            c"coladd".as_ptr(),
            "coladd".len(),
            varnumber_T::from(coladd),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_mouse_key_identifies_mouse_events() {
        assert!(is_mouse_key(K_LEFTMOUSE));
        assert!(is_mouse_key(K_LEFTMOUSE_NM));
        assert!(is_mouse_key(K_LEFTDRAG));
        assert!(is_mouse_key(K_LEFTRELEASE));
        assert!(is_mouse_key(K_MOUSEMOVE));
        assert!(is_mouse_key(K_MIDDLEMOUSE));
        assert!(is_mouse_key(K_RIGHTMOUSE));
        assert!(is_mouse_key(K_MOUSEDOWN));
        assert!(is_mouse_key(K_MOUSEUP));
        assert!(is_mouse_key(K_MOUSELEFT));
        assert!(is_mouse_key(K_MOUSERIGHT));
        assert!(is_mouse_key(K_X1MOUSE));
        assert!(is_mouse_key(K_X2RELEASE));
        assert!(!is_mouse_key(b'a' as c_int));
        assert!(!is_mouse_key(0));
    }

    #[test]
    fn pos_comparisons_respect_lnum_col_coladd() {
        let p1 = pos_T {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let p2 = pos_T {
            lnum: 1,
            col: 2,
            coladd: 1,
        };
        let p3 = pos_T {
            lnum: 2,
            col: 0,
            coladd: 0,
        };
        assert!(lt(p1, p2));
        assert!(lt(p2, p3));
        assert!(!lt(p2, p1));
        assert!(equalpos(p1, p1));
        assert!(ltoreq(p1, p1));
        assert!(ltoreq(p1, p2));
    }
}
