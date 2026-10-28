//! Mouse input, clicking, dragging, and scrolling (`src/nvim/mouse.c`).

use crate::buffer::{VALID_TOPLINE, VAR_FIXED, bt_prompt, bt_quickfix};
use crate::charset::{NUL, ascii_iswhite, vim_iswordc};
use crate::cursor::{
    coladvance, Colnr, equalpos, get_cursor_pos_ptr, Linenr, lt, ltoreq, CursorPos, set_leftcol,
};
use crate::decoration::{decor_conceal_line, decor_virt_lines};
use crate::diff::diff_check_fill;
use crate::drawscreen::{
    DEFAULT_GRID_HANDLE, FR_LEAF, FR_ROW, UPD_INVERTED, UPD_VALID, VALID_BOTLINE,
    redraw_curbuf_later, redraw_later, redraw_statuslines, setcursor, update_screen,
};
use crate::eval::typval::{
    EvalFuncData, TypvalVval, VarType, tv_clear, tv_dict_add_nr, tv_dict_alloc_ret, TypVal,
    Varnumber,
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
use crate::input_cmdatom::{CmdSpec, Cmdarg};
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
use crate::ops::OP_NOP;
use crate::option::get_scrolloff_value;
use crate::option_vars::*;
use crate::plines::{
    CSType, CharsizeArg, getvcol, getvcols, init_charsize_arg, plines_win, plines_win_nofill,
    win_charsize, win_chartabsize, win_get_fill,
};
use crate::popupmenu::pum_visible;
use crate::register::{
    PUT_CURSEND, PUT_FIXINDENT, do_put, insert_reg, yank_register_mline, Yankreg,
};
use crate::search::{BACKWARD, FORWARD, findmatch};
use crate::state::{MODE_INSERT, MODE_NORMAL, REPLACE_FLAG, virtual_active};
use crate::statusline::stl_connected;
use crate::strings::vim_strchr;
use crate::textformat::Oparg;
use crate::ui::{ui_check_mouse, ui_cursor_shape, ui_flush, ui_mouse_has};
use crate::ui_compositor::ui_comp_mouse_focus;
use crate::window::{
    VALID_BOTLINE_AP, find_tabpage, global_stl_height, goto_tabpage, tabpage_index, tabpage_move,
    WinInfoInfo, win_drag_status_line, win_drag_vsep_line, win_enter, win_fdccol_count, win_valid,
};
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

use crate::cmdexpand::Ctrl_P;
use crate::drawline::{VALID_CROW, VALID_WROW};
use crate::eval::funcs::{kMTCharWise, kMTLineWise};
use crate::ex_getln::{Ctrl_G, Ctrl_O, Ctrl_R, Ctrl_RSB, Ctrl_T};
use crate::extmark::kNone;
use crate::insexpand::kWinInfo;
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
const ZERO_POS: CursorPos = CursorPos {
    lnum: 0,
    col: 0,
    coladd: 0,
};

pub(crate) const kStlClickDisabled: c_int = 0;
pub(crate) const kStlClickTabSwitch: c_int = 1;
pub(crate) const kStlClickTabClose: c_int = 2;
pub(crate) const kStlClickFuncRun: c_int = 3;

#[repr(C)]
#[derive(Clone)]
pub struct StlClickDefinition {
    pub type_: c_int,
    pub tabnr: c_int,
    pub func: Option<std::ffi::CString>,
}

impl StlClickDefinition {
    pub const fn new() -> Self {
        Self {
            type_: kStlClickDisabled,
            tabnr: 0,
            func: None,
        }
    }
}

impl Default for StlClickDefinition {
    fn default() -> Self {
        Self::new()
    }
}

struct MouseModuleState {
    orig_topline: Linenr,
    orig_topfill: c_int,
    got_click: bool,
    dragwin: *mut WinInfoInfo,
    in_tab_line: bool,
    orig_cursor: CursorPos,
    status_line_offset: c_int,
    sep_line_offset: c_int,
    on_status_line: bool,
    on_sep_line: bool,
    on_winbar: bool,
    on_statuscol: bool,
    prev_row: c_int,
    prev_col: c_int,
    did_drag: c_int,
}

impl MouseModuleState {
    const fn new() -> Self {
        Self {
            orig_topline: 0,
            orig_topfill: 0,
            got_click: false,
            dragwin: ptr::null_mut(),
            in_tab_line: false,
            orig_cursor: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            status_line_offset: 0,
            sep_line_offset: 0,
            on_status_line: false,
            on_sep_line: false,
            on_winbar: false,
            on_statuscol: false,
            prev_row: -1,
            prev_col: -1,
            did_drag: 0,
        }
    }
}

static MOUSE_STATE: GlobalState<MouseModuleState> = GlobalState::new(MouseModuleState::new());

#[inline]
fn mouse_st() -> &'static mut MouseModuleState {
    unsafe { &mut *MOUSE_STATE.as_ptr() }
}

/// Get class of a character for selection: same class means same word.
/// 0: blank
/// 1: punctuation groups
/// 2: normal word character
/// >2: multi-byte word character.
fn get_mouse_class(p: impl crate::strings::IntoCCharPtr) -> c_int { let p = p.into_c_char_ptr();
     {
        if utf8len_tab[crate::memory::cstr_first_byte(p) as usize] > 1 {
            return mb_get_class(p);
        }

        let c = c_int::from(crate::memory::cstr_first_byte(p));
        if c == b' ' as c_int || c == b'\t' as c_int {
            return 0;
        }
        if vim_iswordc(c) {
            return 2;
        }

        if c != 0 && !vim_strchr(c"-+*/%<>&|^!=", c).is_null() {
            return 1;
        }
        c
    }
}

/// Move "pos" back to the start of the word it's in.
fn find_start_of_word(pos: &mut CursorPos) {
    unsafe {
        let line = ml_get(pos.lnum);
        let cclass = get_mouse_class(line.add(pos.col as usize));

        while pos.col > 0 {
            let mut col = pos.col - 1;
            col -= utf_head_off(line, line.add(col as usize));
            if get_mouse_class(line.add(col as usize)) != cclass {
                break;
            }
            pos.col = col;
        }
    }
}

/// Move "pos" forward to the end of the word it's in.
/// When 'selection' is "exclusive", the position is just after the word.
fn find_end_of_word(pos: &mut CursorPos) {
    unsafe {
        let line = ml_get(pos.lnum);
        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' && pos.col > 0 {
            pos.col -= 1;
            pos.col -= utf_head_off(line, line.add(pos.col as usize));
        }
        let cclass = get_mouse_class(line.add(pos.col as usize));
        while crate::memory::cstr_byte_at(line, pos.col as usize) != 0 {
            let col = pos.col + utfc_ptr2len(line.add(pos.col as usize));
            if get_mouse_class(line.add(col as usize)) != cclass {
                if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                    pos.col = col;
                }
                break;
            }
            pos.col = col;
        }
    }
}

/// Move the current tab to tab in same column as mouse or to end of the
/// tabline if there is no tab there.
fn move_tab_to_mouse() {
     {
        let tabnr = g_st().tab_page_click_defs.as_slice()[g_st().mouse_col as usize].tabnr;
        if tabnr <= 0 {
            tabpage_move(9999);
        } else if tabnr < tabpage_index(g_st().curtab) {
            tabpage_move(tabnr - 1);
        } else {
            tabpage_move(tabnr);
        }
    }
}

/// Close the current or specified tab page.
fn mouse_tab_close(c1: c_int) {
    unsafe {
        let tp = if c1 == 999 { g_st().curtab } else { find_tabpage(c1) };
        if tp == g_st().curtab {
            if !(*g_st().first_tabpage).tp_next.is_null() {
                tabpage_close(0);
            }
        } else if !tp.is_null() {
            tabpage_close_other(&mut *tp, 0);
        }
    }
}

/// Call click definition function for column "col" in the "click_defs" array for button
/// "which_button".
fn call_click_def_func(
    click_defs: *mut StlClickDefinition,
    col: c_int,
    which_button: c_int,
) {
    unsafe {
        let clicks = if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_4CLICK {
            4
        } else if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_3CLICK {
            3
        } else if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
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
            if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
                b's' as c_char
            } else {
                b' ' as c_char
            },
            if (g_st().mod_mask & MOD_MASK_CTRL) != 0 {
                b'c' as c_char
            } else {
                b' ' as c_char
            },
            if (g_st().mod_mask & MOD_MASK_ALT) != 0 {
                b'a' as c_char
            } else {
                b' ' as c_char
            },
            if (g_st().mod_mask & MOD_MASK_META) != 0 {
                b'm' as c_char
            } else {
                b' ' as c_char
            },
            NUL,
        ];

        let mut argv: [TypVal; 4] = [
            TypVal {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval::Number(Varnumber::from((*click_defs.add(col as usize)).tabnr)),
            },
            TypVal {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_FIXED,
                vval: TypvalVval::Number(clicks),
            },
            TypVal {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval::String(btn_str.cast_mut()),
            },
            TypVal {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_FIXED,
                vval: TypvalVval::String(mod_str.as_mut_ptr()),
            },
        ];

        let mut rettv: TypVal = TypVal::new();
        call_vim_function(
            (*click_defs.add(col as usize))
                .func
                .as_ref()
                .map_or(c"".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut()),
            argv.len() as c_int,
            argv.as_mut_ptr(),
            &mut rettv,
        );
        tv_clear(&mut rettv);
        mouse_st().got_click = false;
    }
}

/// Translate window coordinates to buffer position without any side effects.
/// Returns IN_BUFFER and sets `mpos` (0-based column) when in buffer text.
fn get_fpos_of_mouse(mpos: &mut CursorPos, wpp: *mut *mut WinInfoInfo) -> c_int {
    unsafe {
        let mut grid = g_st().mouse_grid;
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;

        if row < 0 || col < 0 {
            return IN_UNKNOWN;
        }

        let wp = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if wp.is_null() {
            return IN_UNKNOWN;
        }
        if !wpp.is_null() {
            *wpp = wp;
        }
        let winrow = row;
        let wincol = col;

        let below_buffer = mouse_comp_pos(&mut *wp, &mut row, &mut col, &mut mpos.lnum);

        if !below_buffer
            && !crate::memory::cstr_is_empty((*wp).w_onebuf_opt.wo_stc)
            && (if (*wp).w_onebuf_opt.wo_rl != 0 {
                wincol >= (*wp).w_view_width - win_col_off(wp)
            } else {
                wincol < win_col_off(wp)
            })
        {
            return MOUSE_STATUSCOL;
        }

        if winrow >= (*wp).w_view_height + (*wp).w_status_height {
            if g_st().mouse_grid <= 1
                && i64::from(g_st().mouse_row) < i64::from(g_st().Rows) - opt_vars().p_ch
                && i64::from(g_st().mouse_row) >= i64::from(g_st().Rows) - opt_vars().p_ch - i64::from(global_stl_height())
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

        if (wpp.is_null() && wp != curwin_ptr()) || below_buffer {
            return IN_UNKNOWN;
        }

        mpos.col = vcol2col(&mut *wp, mpos.lnum, col, Some(&mut mpos.coladd));
        IN_BUFFER
    }
}

fn do_popup(which_button: c_int, m_pos_flag: c_int, mut m_pos: CursorPos) -> c_int {
     {
        let mut jump_flags = 0;
        if crate::memory::cstr_as_bytes(opt_vars().p_mousem) == b"popup_setpos" {
            if g_st().Visual.active {
                if m_pos_flag != IN_BUFFER {
                    jump_flags = MOUSE_MAY_STOP_VIS;
                } else if g_st().Visual.mode == b'V' as c_int {
                    if (curwin_mut().w_cursor.lnum <= g_st().Visual.start.lnum
                        && (m_pos.lnum < curwin_mut().w_cursor.lnum || g_st().Visual.start.lnum < m_pos.lnum))
                        || (g_st().Visual.start.lnum < curwin_mut().w_cursor.lnum
                            && (m_pos.lnum < g_st().Visual.start.lnum
                                || curwin_mut().w_cursor.lnum < m_pos.lnum))
                    {
                        jump_flags = MOUSE_MAY_STOP_VIS;
                    }
                } else if (ltoreq(curwin_mut().w_cursor, g_st().Visual.start)
                    && (lt(m_pos, curwin_mut().w_cursor) || lt(g_st().Visual.start, m_pos)))
                    || (lt(g_st().Visual.start, curwin_mut().w_cursor)
                        && (lt(m_pos, g_st().Visual.start) || lt(curwin_mut().w_cursor, m_pos)))
                {
                    jump_flags = MOUSE_MAY_STOP_VIS;
                } else if g_st().Visual.mode == Ctrl_V {
                    let mut leftcol: Colnr = 0;
                    let mut rightcol: Colnr = 0;
                    getvcols(
                        curwin_ptr(),
                        &mut curwin_mut().w_cursor,
                        &mut g_st().Visual.start,
                        &mut leftcol,
                        &mut rightcol,
                        0,
                    );
                    let mut col = m_pos.col;
                    getvcol(
                        curwin_mut(),
                        &mut m_pos,
                        None,
                        Some(&mut col),
                        None,
                        0,
                    );
                    m_pos.col = col;
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
            redraw_curbuf_later(if g_st().Visual.active {
                UPD_INVERTED
            } else {
                UPD_VALID
            });
            update_screen();
            setcursor();
            ui_flush();
        }
        show_popupmenu();
        mouse_st().got_click = false;
        jump_flags
    }
}

/// Do the appropriate action for the current mouse click in the current mode.
pub fn do_mouse(
    mut oap: Option<&mut Oparg>,
    c: c_int,
    mut dir: c_int,
    count: c_int,
    fixindent: bool,
) -> bool {
    unsafe {
        let mut which_button: c_int;
        let mut is_click = false;
        let mut is_drag = false;

        loop {
            which_button = get_mouse_button(key2termcap1(c), &mut is_click, &mut is_drag);
            if is_drag && g_st().KeyStuffed == 0 && vpeekc() != 0 {
                let save_mouse_grid = g_st().mouse_grid;
                let save_mouse_row = g_st().mouse_row;
                let save_mouse_col = g_st().mouse_col;

                let nc = safe_vgetc();
                if c == nc {
                    continue;
                }
                vungetc(nc);
                g_st().mouse_grid = save_mouse_grid;
                g_st().mouse_row = save_mouse_row;
                g_st().mouse_col = save_mouse_col;
            }
            break;
        }

        if c == K_MOUSEMOVE {
            return false;
        }

        if is_click {
            mouse_st().got_click = true;
            mouse_st().in_tab_line = false;
        } else {
            if !mouse_st().got_click {
                return false;
            }
            if !is_drag {
                mouse_st().got_click = false;
                if mouse_st().in_tab_line {
                    mouse_st().in_tab_line = false;
                    return false;
                }
            }
        }

        if is_click && (g_st().mod_mask & MOD_MASK_CTRL) != 0 && which_button == MOUSE_RIGHT {
            if (g_st().State & MODE_INSERT) != 0 {
                stuffcharReadbuff(Ctrl_O);
            }
            if count > 1 {
                stuffnumReadbuff(count);
            }
            stuffcharReadbuff(Ctrl_T);
            mouse_st().got_click = false;
            if (g_st().State & MODE_INSERT) == 0 {
                exec_stuffed(None);
            }
            return false;
        }

        if (g_st().mod_mask & MOD_MASK_CTRL) != 0 && which_button != MOUSE_LEFT {
            return false;
        }

        if (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL | MOD_MASK_ALT | MOD_MASK_META)) != 0
            && (!is_click || (g_st().mod_mask & MOD_MASK_MULTI_CLICK) != 0 || which_button == MOUSE_MIDDLE)
            && !((g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_ALT)) != 0
                && mouse_model_popup()
                && which_button == MOUSE_LEFT)
            && !((g_st().mod_mask & MOD_MASK_ALT) != 0
                && !mouse_model_popup()
                && which_button == MOUSE_RIGHT)
        {
            return false;
        }

        if !is_click && which_button == MOUSE_MIDDLE {
            return false;
        }

        let mut regname = if oap.is_some() { oap.as_deref_mut().unwrap().regname } else { 0 };
        if which_button == MOUSE_MIDDLE {
            if g_st().State == MODE_NORMAL {
                if oap.is_some() && oap.as_deref_mut().unwrap().op_type != OP_NOP {
                    clearopbeep(oap.as_deref_mut().unwrap());
                    return false;
                }

                if g_st().Visual.active {
                    if g_st().Visual.select {
                        stuffcharReadbuff(Ctrl_G);
                        stuffReadbuff(c"\"+p");
                    } else {
                        if regname != 0 {
                            stuffcharReadbuff(b'"' as c_int);
                            stuffcharReadbuff(regname);
                        }
                        stuffcharReadbuff(b'y' as c_int);
                        stuffcharReadbuff(K_MIDDLEMOUSE);
                    }
                    exec_stuffed(None);
                    return false;
                }
            } else if (g_st().State & MODE_INSERT) == 0 {
                return false;
            }

            if (g_st().State & MODE_INSERT) != 0 {
                if regname == b'.' as c_int {
                    insert_reg(regname, ptr::null_mut(), true);
                } else {
                    if regname == 0 && eval_has_provider(c"clipboard", false) {
                        regname = b'*' as c_int;
                    }
                    let mut reg: *mut Yankreg = ptr::null_mut();
                    if (g_st().State & REPLACE_FLAG) != 0 && !yank_register_mline(regname, &raw mut reg) {
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
        let old_curwin = curwin_ptr();

        if !g_st().tab_page_click_defs.is_empty() {
            if g_st().mouse_grid <= 1 && g_st().mouse_row == 0 && firstwin_mut().w_winrow > 0 {
                if is_drag {
                    if mouse_st().in_tab_line {
                        move_tab_to_mouse();
                    }
                    return false;
                }

                if is_click && g_st().mouse_col < g_st().Columns {
                    let tabnr = g_st().tab_page_click_defs.as_slice()[g_st().mouse_col as usize].tabnr;
                    mouse_st().in_tab_line = true;

                    match g_st().tab_page_click_defs.as_slice()[g_st().mouse_col as usize].type_ {
                        kStlClickDisabled => {}
                        kStlClickTabSwitch => {
                            if which_button != MOUSE_MIDDLE {
                                if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                                    end_visual_mode();
                                    tabpage_new();
                                    tabpage_move(if tabnr == 0 { 9999 } else { tabnr - 1 });
                                } else {
                                    goto_tabpage(tabnr);
                                    if curwin_ptr() != old_curwin {
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
                            call_click_def_func(g_st().tab_page_click_defs.as_mut_ptr(), g_st().mouse_col, which_button);
                        }
                        _ => {}
                    }
                }
                return true;
            } else if is_drag && mouse_st().in_tab_line {
                move_tab_to_mouse();
                return false;
            }
        }

        if is_click && (g_st().mod_mask & MOD_MASK_CTRL) != 0 && which_button == MOUSE_LEFT {
            let mut pos = ZERO_POS;
            let mut wp: *mut WinInfoInfo = ptr::null_mut();
            if get_fpos_of_mouse(&mut pos, &raw mut wp) == IN_BUFFER
                && !bt_quickfix(((*wp).w_buffer).as_ref())
            {
                mouse_st().got_click = false;
                if (g_st().State & MODE_INSERT) == 0 {
                    pos.coladd = 0;
                    mc_toggle((*wp).buf_mut(), pos, false, kNone);
                }
                return false;
            }
        }

        let mut m_pos_flag = 0;
        let mut m_pos = ZERO_POS;
        if mouse_model_popup() {
            m_pos_flag = get_fpos_of_mouse(&mut m_pos, ptr::null_mut());
            if (m_pos_flag & (IN_STATUS_LINE | MOUSE_WINBAR | MOUSE_STATUSCOL)) == 0
                && which_button == MOUSE_RIGHT
                && (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
            {
                if !is_click {
                    return false;
                }
                return (do_popup(which_button, m_pos_flag, m_pos) & CURSOR_MOVED) != 0;
            }
            if (m_pos_flag & (IN_STATUS_LINE | MOUSE_WINBAR | MOUSE_STATUSCOL)) == 0
                && which_button == MOUSE_LEFT
                && (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_ALT)) != 0
            {
                which_button = MOUSE_RIGHT;
                g_st().mod_mask &= !MOD_MASK_SHIFT;
            }
        }

        let mut end_visual = ZERO_POS;
        let mut start_visual = ZERO_POS;
        let mouse_can_visual = ui_mouse_has(kMouseVisual);
        if (g_st().State & (MODE_NORMAL | MODE_INSERT)) != 0
            && (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
        {
            if which_button == MOUSE_LEFT && mouse_can_visual {
                if is_click {
                    if g_st().Visual.active {
                        jump_flags |= MOUSE_MAY_STOP_VIS;
                    }
                } else {
                    jump_flags |= MOUSE_MAY_VIS;
                }
            } else if which_button == MOUSE_RIGHT && mouse_can_visual {
                if is_click && g_st().Visual.active {
                    if lt(curwin_mut().w_cursor, g_st().Visual.start) {
                        start_visual = curwin_mut().w_cursor;
                        end_visual = g_st().Visual.start;
                    } else {
                        start_visual = g_st().Visual.start;
                        end_visual = curwin_mut().w_cursor;
                    }
                }
                jump_flags |= MOUSE_MAY_VIS;
                jump_flags |= MOUSE_FOCUS;
            } else if which_button == MOUSE_RIGHT {
                jump_flags |= MOUSE_FOCUS;
            }
        }

        if !is_drag && oap.is_some() && oap.as_deref_mut().unwrap().op_type != OP_NOP {
            mouse_st().got_click = false;
            oap.as_deref_mut().unwrap().motion_type = kMTCharWise;
        }

        if !is_click && !is_drag {
            jump_flags |= MOUSE_RELEASED;
        }

        let old_active = g_st().Visual.active;
        let save_cursor = curwin_mut().w_cursor;

        if !g_st().Visual.active || mouse_can_visual {
            jump_flags = jump_to_mouse(
                jump_flags,
                if oap.is_none() {
                    ptr::null_mut()
                } else {
                    &raw mut oap.as_deref_mut().unwrap().inclusive
                },
                which_button,
            );
        }

        if g_st().mod_mask == 0
            && !is_drag
            && (jump_flags & (MOUSE_FOLD_CLOSE | MOUSE_FOLD_OPEN)) != 0
            && which_button == MOUSE_LEFT
        {
            if (jump_flags & MOUSE_FOLD_OPEN) != 0 {
                openFold(curwin_mut().w_cursor, 1);
            } else {
                closeFold(curwin_mut().w_cursor, 1);
            }
            if curwin_ptr() == old_curwin {
                curwin_mut().w_cursor = save_cursor;
            }
        }

        let moved = (jump_flags & CURSOR_MOVED) != 0;
        let in_winbar = (jump_flags & MOUSE_WINBAR) != 0;
        let in_statuscol = (jump_flags & MOUSE_STATUSCOL) != 0;
        let in_status_line = (jump_flags & IN_STATUS_LINE) != 0;
        let in_global_statusline = in_status_line && global_stl_height() > 0;
        let in_sep_line = (jump_flags & IN_SEP_LINE) != 0;

        if (in_winbar || in_status_line || in_statuscol) && is_click {
            let mut click_grid = g_st().mouse_grid;
            let mut click_row = g_st().mouse_row;
            let mut click_col = g_st().mouse_col;
            let wp =
                mouse_find_win_inner(&mut click_grid, &mut click_row, &mut click_col);
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
                click_defs = if curwin_mut().w_status_click_defs.is_empty() {
                    ptr::null_mut()
                } else {
                    curwin_mut().w_status_click_defs.as_mut_ptr()
                };
                click_col = g_st().mouse_col;
            }

            if in_status_line {
                let stl_wp = if in_global_statusline { curwin_ptr() } else { wp };
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
                            && (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) == 0
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

        if curwin_ptr() != old_curwin && oap.is_some() && oap.as_deref_mut().unwrap().op_type != OP_NOP {
            clearop(oap.as_deref_mut().unwrap());
        }

        if g_st().Visual.active && is_drag && get_scrolloff_value(curwin_mut()) != 0 {
            if g_st().mouse_row == 0 {
                g_st().mouse_dragging = 2;
            } else {
                g_st().mouse_dragging = 1;
            }
        }

        if is_drag && g_st().mouse_row < 0 && !in_status_line {
            scroll_redraw(0, 1);
            g_st().mouse_row = 0;
        }

        let old_mode = g_st().Visual.mode;
        if start_visual.lnum != 0 {
            if (g_st().mod_mask & MOD_MASK_ALT) != 0 {
                g_st().Visual.mode = Ctrl_V;
            }

            if g_st().Visual.mode == Ctrl_V {
                let mut leftcol: Colnr = 0;
                let mut rightcol: Colnr = 0;
                getvcols(
                    curwin_ptr(),
                    &mut start_visual,
                    &mut end_visual,
                    &mut leftcol,
                    &mut rightcol,
                    0,
                );
                if curwin_mut().w_curswant > (leftcol + rightcol) / 2 {
                    end_visual.col = leftcol;
                } else {
                    end_visual.col = rightcol;
                }
                if curwin_mut().w_cursor.lnum >= (start_visual.lnum + end_visual.lnum) / 2 {
                    end_visual.lnum = start_visual.lnum;
                }

                start_visual = curwin_mut().w_cursor;
                curwin_mut().w_cursor = end_visual;
                coladvance(curwin_mut(), end_visual.col);
                g_st().Visual.start = curwin_mut().w_cursor;
                curwin_mut().w_cursor = start_visual;
            } else if lt(curwin_mut().w_cursor, start_visual) {
                g_st().Visual.start = end_visual;
            } else if lt(end_visual, curwin_mut().w_cursor) {
                g_st().Visual.start = start_visual;
            } else if end_visual.lnum == start_visual.lnum {
                if curwin_mut().w_cursor.col - start_visual.col
                    > end_visual.col - curwin_mut().w_cursor.col
                {
                    g_st().Visual.start = start_visual;
                } else {
                    g_st().Visual.start = end_visual;
                }
            } else {
                let diff = (curwin_mut().w_cursor.lnum - start_visual.lnum)
                    - (end_visual.lnum - curwin_mut().w_cursor.lnum);

                if diff > 0 {
                    g_st().Visual.start = start_visual;
                } else if diff < 0
                    || curwin_mut().w_cursor.col < (start_visual.col + end_visual.col) / 2
                {
                    g_st().Visual.start = end_visual;
                } else {
                    g_st().Visual.start = start_visual;
                }
            }
        } else if (g_st().State & MODE_INSERT) != 0 && g_st().Visual.active {
            stuffcharReadbuff(Ctrl_O);
        }

        if which_button == MOUSE_MIDDLE {
            if regname == 0 && eval_has_provider(c"clipboard", false) {
                regname = b'*' as c_int;
            }
            let mut reg: *mut Yankreg = ptr::null_mut();
            if yank_register_mline(regname, &raw mut reg) {
                if g_st().mouse_past_bottom {
                    dir = FORWARD;
                }
            } else if g_st().mouse_past_eol {
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

            if g_st().restart_edit != 0 {
                g_st().where_paste_started = curwin_mut().w_cursor;
            }
            do_put(
                regname,
                reg,
                dir,
                count,
                (if fixindent { PUT_FIXINDENT } else { 0 }) | PUT_CURSEND,
            );
        } else if ((g_st().mod_mask & MOD_MASK_CTRL) != 0
            || (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK)
            && bt_quickfix(Some(crate::globals::curbuf_mut()))
        {
            if curwin_mut().w_llist_ref.is_null() {
                do_cmdline_cmd(c".cc");
            } else {
                do_cmdline_cmd(c".ll");
            }
            mouse_st().got_click = false;
        } else if curbuf_mut().b_help && (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
            if (g_st().State & MODE_INSERT) != 0 {
                stuffcharReadbuff(Ctrl_O);
            }
            stuffcharReadbuff(Ctrl_RSB);
            mouse_st().got_click = false;
            if (g_st().State & MODE_INSERT) == 0 {
                exec_stuffed(None);
            }
        } else if (g_st().mod_mask & MOD_MASK_SHIFT) != 0 {
            if (g_st().State & MODE_INSERT) != 0 || (g_st().Visual.active && g_st().Visual.select) {
                stuffcharReadbuff(Ctrl_O);
            }
            if which_button == MOUSE_LEFT {
                stuffcharReadbuff(b'*' as c_int);
            } else {
                stuffcharReadbuff(b'#' as c_int);
            }
            if (g_st().State & MODE_INSERT) == 0 {
                exec_stuffed(None);
            }
        } else if in_status_line || in_sep_line {
            // Do nothing if on status line or vertical separator
        } else if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) != 0
            && (g_st().State & (MODE_NORMAL | MODE_INSERT)) != 0
            && mouse_can_visual
        {
            if is_click || !g_st().Visual.active {
                if g_st().Visual.active {
                    mouse_st().orig_cursor = g_st().Visual.start;
                } else {
                    g_st().Visual.start = curwin_mut().w_cursor;
                    mouse_st().orig_cursor = g_st().Visual.start;
                    g_st().Visual.active = true;
                    g_st().Visual.reselect = 1;
                    may_start_select(b'o' as c_int);
                    setmouse();
                }
                if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                    if (g_st().mod_mask & MOD_MASK_ALT) != 0 {
                        g_st().Visual.mode = Ctrl_V;
                    } else {
                        g_st().Visual.mode = b'v' as c_int;
                    }
                } else if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_3CLICK {
                    g_st().Visual.mode = b'V' as c_int;
                } else if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_4CLICK {
                    g_st().Visual.mode = Ctrl_V;
                }
            }
            if (g_st().mod_mask & MOD_MASK_MULTI_CLICK) == MOD_MASK_2CLICK {
                let mut pos: *mut CursorPos = ptr::null_mut();

                if is_click {
                    end_visual = curwin_mut().w_cursor;
                    while ascii_iswhite(gchar_pos(&mut end_visual)) {
                        inc(&mut end_visual);
                    }
                    if oap.is_some() {
                        oap.as_deref_mut().unwrap().motion_type = kMTCharWise;
                    }
                    if oap.is_some()
                        && g_st().Visual.mode == b'v' as c_int
                        && !vim_iswordc(gchar_pos(&mut end_visual))
                        && equalpos(curwin_mut().w_cursor, g_st().Visual.start)
                    {
                        pos = findmatch(oap.as_deref_mut(), 0);
                        if !pos.is_null() {
                            curwin_mut().w_cursor = *pos;
                            if oap.unwrap().motion_type == kMTLineWise {
                                g_st().Visual.mode = b'V' as c_int;
                            } else if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                                if lt(curwin_mut().w_cursor, g_st().Visual.start) {
                                    g_st().Visual.start.col += 1;
                                } else {
                                    curwin_mut().w_cursor.col += 1;
                                }
                            }
                        }
                    }
                }

                if pos.is_null() && (is_click || is_drag) {
                    if lt(curwin_mut().w_cursor, mouse_st().orig_cursor) {
                        find_start_of_word(&mut curwin_mut().w_cursor);
                        find_end_of_word(&mut g_st().Visual.start);
                    } else {
                        find_start_of_word(&mut g_st().Visual.start);
                        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' && !crate::memory::cstr_is_empty(get_cursor_pos_ptr()) {
                            curwin_mut().w_cursor.col += utfc_ptr2len(get_cursor_pos_ptr());
                        }
                        find_end_of_word(&mut curwin_mut().w_cursor);
                    }
                }
                curwin_mut().w_set_curswant = true;
            }
            if is_click {
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else if g_st().Visual.active && !old_active {
            if (g_st().mod_mask & MOD_MASK_ALT) != 0 {
                g_st().Visual.mode = Ctrl_V;
            } else {
                g_st().Visual.mode = b'v' as c_int;
            }
        }

        if (!g_st().Visual.active && old_active && g_st().mode_displayed)
            || (g_st().Visual.active
                && opt_vars().p_smd != 0
                && g_st().msg_silent == 0
                && (!old_active || g_st().Visual.mode != old_mode))
        {
            g_st().redraw_cmdline = true;
        }

        moved
    }
}

pub fn ins_mouse(c: c_int) {
     {
        let old_curwin = curwin_ptr();

        undisplay_dollar();
        let mut tpos = curwin_mut().w_cursor;
        if do_mouse(None, c, BACKWARD, 1, false) {
            let new_curwin = curwin_ptr();

            if curwin_ptr() != old_curwin && win_valid(old_curwin) {
                set_curwin(old_curwin);
                g_st().curbuf = curwin_mut().w_buffer;
                if bt_prompt(Some(crate::globals::curbuf_mut())) {
                    curbuf_mut().b_prompt_insert = b'A' as c_int;
                }
            }
            start_arrow(
                if curwin_ptr() == old_curwin {
                    &raw mut tpos
                } else {
                    ptr::null_mut()
                },
                true,
                0,
            );
            if curwin_ptr() != new_curwin && win_valid(new_curwin) {
                set_curwin(new_curwin);
                g_st().curbuf = curwin_mut().w_buffer;
            }
            set_can_cindent(true);
        }

        redraw_statuslines();
    }
}

/// Common mouse wheel scrolling, shared between Insert mode and NV modes.
pub fn do_mousescroll(cap: &mut Cmdarg) {
     {
        let shift_or_ctrl = (g_st().mod_mask & (MOD_MASK_SHIFT | MOD_MASK_CTRL)) != 0;

        if cap.arg == MSCR_UP || cap.arg == MSCR_DOWN {
            if (g_st().State & MODE_NORMAL) != 0 && shift_or_ctrl {
                pagescroll(if cap.arg != 0 { FORWARD } else { BACKWARD }, 1, false);
            } else {
                if shift_or_ctrl {
                    cap.count1 = curwin_mut().w_botline - curwin_mut().w_topline;
                } else {
                    cap.count1 = opt_vars().p_mousescroll_vert as c_int;
                }
                if cap.count1 > 0 {
                    cap.count0 = cap.count1;
                    nv_scroll_line(cap);
                }
            }
        } else {
            let step = if shift_or_ctrl {
                curwin_mut().w_view_width
            } else {
                opt_vars().p_mousescroll_hor as c_int
            };
            let leftcol = (curwin_mut().w_leftcol
                + if cap.arg == MSCR_RIGHT {
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
pub fn ins_mousescroll(dir: c_int) {
     {
        let mut cap = Cmdarg::new();
        let mut oa = Oparg::new();
        cap.oap = &raw mut oa;
        cap.arg = dir;

        match dir {
            MSCR_UP => cap.cmdchar = K_MOUSEUP,
            MSCR_DOWN => cap.cmdchar = K_MOUSEDOWN,
            MSCR_LEFT => cap.cmdchar = K_MOUSELEFT,
            MSCR_RIGHT => cap.cmdchar = K_MOUSERIGHT,
            _ => siemsg!(c"Invalid ins_mousescroll() argument: %d", dir),
        }

        let old_curwin = curwin_ptr();
        if g_st().mouse_row >= 0 && g_st().mouse_col >= 0 {
            let mut grid = g_st().mouse_grid;
            let mut row = g_st().mouse_row;
            let mut col = g_st().mouse_col;
            set_curwin(mouse_find_win_inner(&mut grid, &mut row, &mut col));
            if (curwin_ptr()).is_null() {
                set_curwin(old_curwin);
                return;
            }
            g_st().curbuf = curwin_mut().w_buffer;
        }

        if curwin_ptr() == old_curwin {
            if pum_visible() {
                return;
            }

            undisplay_dollar();
        }

        let mut orig_cursor = curwin_mut().w_cursor;

        do_mousescroll(&mut cap);

        curwin_mut().w_redr_status = true;
        set_curwin(old_curwin);
        g_st().curbuf = curwin_mut().w_buffer;

        if !equalpos(curwin_mut().w_cursor, orig_cursor) {
            start_arrow(&raw mut orig_cursor, true, 0);
            set_can_cindent(true);
        }
    }
}

/// Command-line mode implementation for scrolling in direction "dir", which is
/// one of the MSCR_ values. Scrolls the completion info popup when the mouse
/// pointer is on top of it.
/// Returns true when the info popup was scrolled.
pub fn cmdline_mousescroll(dir: c_int) -> bool {
    unsafe {
        let mut cap = Cmdarg::new();
        let mut oa = Oparg::new();
        cap.oap = &raw mut oa;
        cap.arg = dir;

        match dir {
            MSCR_UP => cap.cmdchar = K_MOUSEUP,
            MSCR_DOWN => cap.cmdchar = K_MOUSEDOWN,
            MSCR_LEFT => cap.cmdchar = K_MOUSELEFT,
            MSCR_RIGHT => cap.cmdchar = K_MOUSERIGHT,
            _ => {}
        }

        if g_st().mouse_row < 0 || g_st().mouse_col < 0 {
            return false;
        }

        let mut grid = g_st().mouse_grid;
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;

        let wp = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if wp.is_null() || (*wp).w_kind != kWinInfo {
            return false;
        }

        let old_curwin = curwin_ptr();

        set_curwin(wp);
        g_st().curbuf = (*wp).w_buffer;
        do_mousescroll(&mut cap);
        set_curwin(old_curwin);
        g_st().curbuf = curwin_mut().w_buffer;

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
fn mouse_model_popup() -> bool {
     crate::memory::cstr_first_byte(opt_vars().p_mousem) == b'p'
}

/// Reset the window being dragged. To be called when switching tab page.
pub fn reset_dragwin() {
    mouse_st().dragwin = ptr::null_mut();
}

#[inline]
fn jump_to_mouse_retnomove(
    flags: c_int,
    status_line_offset: c_int,
    sep_line_offset: c_int,
    on_winbar: bool,
    on_statuscol: bool,
) -> c_int {
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

/// Move the cursor to the specified row and column on the screen.
/// Change current window if necessary. Returns an integer with the
/// CURSOR_MOVED bit set if the cursor has moved or unset otherwise.
pub fn jump_to_mouse(mut flags: c_int, inclusive: *mut bool, which_button: c_int) -> c_int {
    unsafe {

        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;
        let mut grid = g_st().mouse_grid;
        let keep_focus = (flags & MOUSE_FOCUS) != 0;

        g_st().mouse_past_bottom = false;
        g_st().mouse_past_eol = false;

        if (flags & MOUSE_RELEASED) != 0 {
            if !mouse_st().dragwin.is_null() && mouse_st().did_drag == 0 {
                flags &= !(MOUSE_FOCUS | MOUSE_DID_MOVE);
            }
            mouse_st().dragwin = ptr::null_mut();
            mouse_st().did_drag = 0;
        }

        if (flags & MOUSE_DID_MOVE) != 0 && mouse_st().prev_row == g_st().mouse_row && mouse_st().prev_col == g_st().mouse_col {
            return jump_to_mouse_retnomove(
                flags,
                mouse_st().status_line_offset,
                mouse_st().sep_line_offset,
                mouse_st().on_winbar,
                mouse_st().on_statuscol,
            );
        }

        mouse_st().prev_row = g_st().mouse_row;
        mouse_st().prev_col = g_st().mouse_col;

        if (flags & MOUSE_SETPOS) != 0 {
            return jump_to_mouse_retnomove(
                flags,
                mouse_st().status_line_offset,
                mouse_st().sep_line_offset,
                mouse_st().on_winbar,
                mouse_st().on_statuscol,
            );
        }

        if row < 0 || col < 0 {
            return IN_UNKNOWN;
        }

        let wp = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if wp.is_null() {
            return IN_UNKNOWN;
        }

        let below_window =
            grid == DEFAULT_GRID_HANDLE && row + (*wp).w_winbar_height >= (*wp).w_height;
        mouse_st().on_status_line = below_window && row + (*wp).w_winbar_height - (*wp).w_height + 1 == 1;
        mouse_st().on_sep_line =
            grid == DEFAULT_GRID_HANDLE && col >= (*wp).w_width && col - (*wp).w_width + 1 == 1;
        mouse_st().on_winbar = row < 0 && row + (*wp).w_winbar_height >= 0;
        mouse_st().on_statuscol = !below_window
            && !mouse_st().on_status_line
            && !mouse_st().on_sep_line
            && !mouse_st().on_winbar
            && !crate::memory::cstr_is_empty((*wp).w_onebuf_opt.wo_stc)
            && (if (*wp).w_onebuf_opt.wo_rl != 0 {
                col >= (*wp).w_view_width - win_col_off(wp)
            } else {
                col < win_col_off(wp)
            });

        if mouse_st().on_status_line && mouse_st().on_sep_line {
            if stl_connected(&mut *wp) {
                mouse_st().on_sep_line = false;
            } else {
                mouse_st().on_status_line = false;
            }
        }

        if keep_focus {
            row = g_st().mouse_row;
            col = g_st().mouse_col;
            grid = g_st().mouse_grid;
        }

        let old_curwin = curwin_ptr();
        let old_cursor = curwin_mut().w_cursor;
        if !keep_focus {
            if mouse_st().on_winbar {
                return IN_OTHER_WIN | MOUSE_WINBAR;
            }

            if !mouse_st().on_statuscol {
                let fdc = win_fdccol_count(&mut *wp);
                mouse_st().dragwin = ptr::null_mut();

                if below_window {
                    mouse_st().status_line_offset = row + (*wp).w_winbar_height - (*wp).w_height + 1;
                    mouse_st().dragwin = wp;
                } else {
                    mouse_st().status_line_offset = 0;
                }

                if grid == DEFAULT_GRID_HANDLE && col >= (*wp).w_width {
                    mouse_st().sep_line_offset = col - (*wp).w_width + 1;
                    mouse_st().dragwin = wp;
                } else {
                    mouse_st().sep_line_offset = 0;
                }

                if mouse_st().status_line_offset != 0 && mouse_st().sep_line_offset != 0 {
                    if stl_connected(&mut *wp) {
                        mouse_st().sep_line_offset = 0;
                    } else {
                        mouse_st().status_line_offset = 0;
                    }
                }

                if g_st().Visual.active
                    && ((*wp).w_buffer != curwin_mut().w_buffer
                        || (mouse_st().status_line_offset == 0
                            && mouse_st().sep_line_offset == 0
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

                if mouse_st().dragwin.is_null() || (flags & MOUSE_RELEASED) != 0 {
                    win_enter(wp, true);
                }
                if curwin_ptr() != old_curwin {
                    set_mouse_topline(curwin_mut());
                }
                if mouse_st().status_line_offset != 0 {
                    if curwin_ptr() == old_curwin {
                        return IN_STATUS_LINE;
                    }
                    return IN_STATUS_LINE | CURSOR_MOVED;
                }
                if mouse_st().sep_line_offset != 0 {
                    if curwin_ptr() == old_curwin {
                        return IN_SEP_LINE;
                    }
                    return IN_SEP_LINE | CURSOR_MOVED;
                }

                curwin_mut().w_cursor.lnum = curwin_mut().w_topline;
            }
        } else if mouse_st().status_line_offset != 0 {
            if which_button == MOUSE_LEFT && !mouse_st().dragwin.is_null() {
                let count =
                    row - (*mouse_st().dragwin).w_winrow - (*mouse_st().dragwin).w_height + 1 - mouse_st().status_line_offset;
                win_drag_status_line(&mut *mouse_st().dragwin, count);
                mouse_st().did_drag |= count;
            }
            return IN_STATUS_LINE;
        } else if mouse_st().sep_line_offset != 0 && which_button == MOUSE_LEFT {
            if !mouse_st().dragwin.is_null() {
                let count = col - (*mouse_st().dragwin).w_wincol - (*mouse_st().dragwin).w_width + 1 - mouse_st().sep_line_offset;
                win_drag_vsep_line(&mut *mouse_st().dragwin, count);
                mouse_st().did_drag |= count;
            }
            return IN_SEP_LINE;
        } else if mouse_st().on_status_line && which_button == MOUSE_RIGHT {
            return IN_STATUS_LINE;
        } else if mouse_st().on_winbar && which_button == MOUSE_RIGHT {
            return IN_OTHER_WIN | MOUSE_WINBAR;
        } else if mouse_st().on_statuscol && which_button == MOUSE_RIGHT {
            return IN_OTHER_WIN | MOUSE_STATUSCOL;
        } else {
            if (flags & MOUSE_MAY_STOP_VIS) != 0 {
                end_visual_mode();
                redraw_curbuf_later(UPD_INVERTED);
            }

            if grid == 0 {
                row -= curwin_mut().w_grid_alloc.comp_row + curwin_mut().w_grid.row_offset;
                col -= curwin_mut().w_grid_alloc.comp_col + curwin_mut().w_grid.col_offset;
            } else if grid != DEFAULT_GRID_HANDLE {
                row -= curwin_mut().w_grid.row_offset;
                col -= curwin_mut().w_grid.col_offset;
            }

            if row < 0 {
                let mut count = 0;
                let mut first = true;
                while curwin_mut().w_topline > 1 {
                    if curwin_mut().w_topfill < win_get_fill(crate::globals::curwin_mut(), curwin_mut().w_topline) {
                        count += 1;
                    } else {
                        count += plines_win(curwin_ptr(), curwin_mut().w_topline - 1, true);
                    }
                    if !first && count > -row {
                        break;
                    }
                    first = false;
                    hasFolding(
                        curwin_ptr(),
                        curwin_mut().w_topline,
                        Some(&mut curwin_mut().w_topline),
                        ptr::null_mut(),
                    );
                    if curwin_mut().w_topfill < win_get_fill(crate::globals::curwin_mut(), curwin_mut().w_topline) {
                        curwin_mut().w_topfill += 1;
                    } else {
                        curwin_mut().w_topline -= 1;
                        curwin_mut().w_topfill = 0;
                    }
                }
                check_topfill(curwin_mut(), false);
                curwin_mut().w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                row = 0;
            } else if row >= curwin_mut().w_view_height {
                let mut count = 0;
                let mut first = true;
                while curwin_mut().w_topline < curbuf_mut().b_ml.ml_line_count {
                    if curwin_mut().w_topfill > 0 {
                        count += 1;
                    } else {
                        count += plines_win(curwin_ptr(), curwin_mut().w_topline, true);
                    }

                    if !first && count > row - curwin_mut().w_view_height + 1 {
                        break;
                    }
                    first = false;

                    if curwin_mut().w_topfill > 0 {
                        curwin_mut().w_topfill -= 1;
                    } else {
                        if hasFolding(
                            curwin_ptr(),
                            curwin_mut().w_topline,
                            None,
                            &raw mut (*curwin_ptr()).w_topline,
                        ) && curwin_mut().w_topline == curbuf_mut().b_ml.ml_line_count
                        {
                            break;
                        }
                        curwin_mut().w_topline += 1;
                        curwin_mut().w_topfill = win_get_fill(crate::globals::curwin_mut(), curwin_mut().w_topline);
                    }
                }
                check_topfill(curwin_mut(), false);
                redraw_later(Some(crate::globals::curwin_mut()), UPD_VALID);
                curwin_mut().w_valid &= !(VALID_WROW | VALID_CROW | VALID_BOTLINE | VALID_BOTLINE_AP);
                row = curwin_mut().w_view_height - 1;
            } else if row == 0
                && g_st().mouse_dragging > 0
                && curwin_mut().w_cursor.lnum == curwin_mut().buf_mut().b_ml.ml_line_count
                && curwin_mut().w_cursor.lnum == curwin_mut().w_topline
            {
                curwin_mut().w_valid &= !VALID_TOPLINE;
            }
        }

        let mut col_from_screen: Colnr = -1;
        let mut mouse_fold_flags: c_int = 0;
        mouse_check_grid(&mut col_from_screen, &mut mouse_fold_flags);

        if mouse_comp_pos(
            curwin_mut(),
            &mut row,
            &mut col,
            &mut curwin_mut().w_cursor.lnum,
        ) {
            g_st().mouse_past_bottom = true;
        }

        if (flags & MOUSE_MAY_VIS) != 0 && !g_st().Visual.active {
            g_st().Visual.start = old_cursor;
            g_st().Visual.active = true;
            g_st().Visual.reselect = 1;
            may_start_select(b'o' as c_int);
            setmouse();

            if opt_vars().p_smd != 0 && g_st().msg_silent == 0 {
                g_st().redraw_cmdline = true;
            }
        }

        if col_from_screen >= 0 {
            col = col_from_screen;
        }

        curwin_mut().w_curswant = col;
        curwin_mut().w_set_curswant = false;
        if !coladvance(curwin_mut(), col) {
            if !inclusive.is_null() {
                *inclusive = true;
            }
            g_st().mouse_past_eol = true;
        } else if !inclusive.is_null() {
            *inclusive = false;
        }

        let mut count = if mouse_st().on_statuscol {
            IN_OTHER_WIN | MOUSE_STATUSCOL
        } else {
            IN_BUFFER
        };
        if curwin_ptr() != old_curwin
            || curwin_mut().w_cursor.lnum != old_cursor.lnum
            || curwin_mut().w_cursor.col != old_cursor.col
        {
            count |= CURSOR_MOVED;
        }

        count |= mouse_fold_flags;

        count
    }
}

/// Make a horizontal scroll to "leftcol".
/// Return true if the cursor moved, false otherwise.
fn do_mousescroll_horiz(leftcol: Colnr) -> bool {
     {
        if curwin_mut().w_onebuf_opt.wo_wrap != 0 {
            return false;
        }
        if curwin_mut().w_leftcol == leftcol {
            return false;
        }

        if !virtual_active(curwin_mut()) && leftcol > scroll_line_len(curwin_mut().w_cursor.lnum) {
            curwin_mut().w_cursor.lnum = find_longest_lnum();
            curwin_mut().w_cursor.col = 0;
        }

        set_leftcol(leftcol)
    }
}

/// Normal and Visual modes implementation for scrolling in direction
/// "cap->arg", which is one of the MSCR_ values.
pub fn nv_mousescroll(cap: &mut Cmdarg) {
     {
        let old_curwin = curwin_ptr();

        if g_st().mouse_row >= 0 && g_st().mouse_col >= 0 {
            let mut grid = g_st().mouse_grid;
            let mut row = g_st().mouse_row;
            let mut col = g_st().mouse_col;
            set_curwin(mouse_find_win_inner(&mut grid, &mut row, &mut col));
            if (curwin_ptr()).is_null() {
                set_curwin(old_curwin);
                return;
            }
            g_st().curbuf = curwin_mut().w_buffer;
        }

        do_mousescroll(cap);

        curwin_mut().w_redr_status = true;
        set_curwin(old_curwin);
        g_st().curbuf = curwin_mut().w_buffer;
    }
}

/// Mouse clicks and drags.
pub fn nv_mouse(cap: &mut Cmdarg) {
    do_mouse(Some(cap.oap_mut()), cap.cmdchar, BACKWARD, cap.count1, false);
}

/// Compute the buffer line position from the screen position "rowp" / "colp" in
/// window "win".
/// Returns true if the position is below the last line.
pub fn mouse_comp_pos(
    win: &mut WinInfoInfo,
    rowp: &mut c_int,
    colp: &mut c_int,
    lnump: &mut Linenr,
) -> bool {
    unsafe {
        let mut col = *colp;
        let mut row = *rowp;
        let mut retval = false;

        if win.w_onebuf_opt.wo_rl != 0 {
            col = win.w_view_width - 1 - col;
        }

        let mut lnum = win.w_topline;

        while row > 0 {
            let filler_lines = if lnum == win.w_topline {
                win.w_topfill
            } else {
                win_get_fill(win, lnum)
            };
            let mut count = plines_win_nofill(&raw mut *win, lnum, false);

            if win.w_skipcol > 0 && lnum == win.w_topline {
                let width1 = win.w_view_width - win_col_off(&raw mut *win);

                if width1 > 0 {
                    let skip_lines = if win.w_skipcol > width1 {
                        (win.w_skipcol - width1) / (width1 + win_col_off2(win)) + 1
                    } else if win.w_skipcol > 0 {
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

            hasFolding(&raw mut *win, lnum, None, &raw mut lnum);

            if lnum == win.buf_mut().b_ml.ml_line_count {
                retval = true;
                break;
            }
            row -= count + filler_lines;
            lnum += 1;
        }

        let mut virt_below: c_int = 0;
        let virt_lines = decor_virt_lines(
            &mut *win,
            lnum - 1,
            lnum,
            Some(&mut virt_below),
            None,
            true,
        );
        let diff_fill = diff_check_fill(&mut *win, lnum);
        let skip_fill = if lnum == win.w_topline {
            virt_lines + diff_fill - win.w_topfill
        } else {
            0
        };

        let mut virtnum = row - (virt_lines + diff_fill - skip_fill);
        if row < virt_below - skip_fill {
            lnum -= 1;
            let mut virt_below_prev: c_int = 0;
            let virt_lines_prev = decor_virt_lines(
                &mut *win,
                lnum - 1,
                lnum,
                Some(&mut virt_below_prev),
                None,
                true,
            );
            let diff_fill_prev = diff_check_fill(&mut *win, lnum - 1);
            virtnum = -row - 1 - virt_lines_prev - diff_fill_prev + virt_below_prev;
        } else if row < virt_lines + diff_fill - skip_fill {
            virtnum = -row;
        }
        set_vim_var_nr(VV_LNUM, Varnumber::from(lnum));
        set_vim_var_nr(VV_VIRTNUM, Varnumber::from(virtnum));

        row -= virt_lines + diff_fill - skip_fill;

        while lnum < win.buf_mut().b_ml.ml_line_count
            && decor_conceal_line(&raw mut *win, lnum - 1, false)
        {
            lnum += 1;
            hasFolding(&raw mut *win, lnum, None, &raw mut lnum);
        }

        if !retval {
            let off = win_col_off(&raw mut *win) - win_col_off2(win);
            col = col.max(off);
            col += row * (win.w_view_width - off);

            if lnum == win.w_topline {
                col += win.w_skipcol;
            }
        }

        if win.w_onebuf_opt.wo_wrap == 0 {
            col += win.w_leftcol;
        }

        col -= win_col_off(&raw mut *win);
        col = col.max(0);

        *colp = col;
        *rowp = row;
        *lnump = lnum;
        retval
    }
}

/// Find the window at "grid" position "*rowp" and "*colp". The positions are
/// updated to become relative to the top-left of the window inner area.
pub fn mouse_find_win_inner(
    gridp: &mut c_int,
    rowp: &mut c_int,
    colp: &mut c_int,
) -> *mut WinInfoInfo {
    unsafe {
        let wp_grid = mouse_find_grid_win(&mut *gridp, &mut *rowp, &mut *colp);
        if !wp_grid.is_null() {
            return wp_grid;
        } else if *gridp > 1 {
            return ptr::null_mut();
        }

        let mut fp = g_st().topframe;
        *rowp -= firstwin_mut().w_winrow;
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

        for wp in for_all_windows_ptr() {
            if wp == (*fp).fr_win {
                *rowp -= (*wp).w_winbar_height;
                return wp;
            }
        }
        ptr::null_mut()
    }
}

/// Find the window at "grid" position "*rowp" and "*colp". The positions are
/// updated to become relative to the top-left of the window.
pub fn mouse_find_win_outer(
    gridp: &mut c_int,
    rowp: &mut c_int,
    colp: &mut c_int,
) -> *mut WinInfoInfo {
    unsafe {
        let wp = mouse_find_win_inner(gridp, rowp, colp);
        if !wp.is_null() {
            *rowp += (*wp).w_winrow_off;
            *colp += (*wp).w_wincol_off;
        }
        wp
    }
}

fn mouse_find_grid_win(gridp: &mut c_int, rowp: &mut c_int, colp: &mut c_int) -> *mut WinInfoInfo {
    unsafe {
        if *gridp == g_st().msg_grid.handle {
            *rowp += g_st().msg_grid_pos;
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
            if ptr::eq(grid, &raw const g_st().pum_grid) {
                *gridp = (*grid).handle;
                *rowp -= (*grid).comp_row;
                *colp -= (*grid).comp_col;
                return ptr::null_mut();
            } else {
                let mut wp = g_st().firstwin;
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
pub fn vcol2col(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    vcol: Colnr,
    coladdp: Option<&mut Colnr>,
) -> Colnr {
    unsafe {
        let line = ml_get_buf(wp.buf_mut(), lnum);
        let mut csarg = CharsizeArg::new();
        let cstype: CSType = init_charsize_arg(&mut csarg, wp, lnum, line);
        let mut ci = utf_ptr2StrCharInfo(line);
        let mut cur_vcol: c_int = 0;
        while cur_vcol < vcol && !crate::memory::cstr_is_empty(ci.ptr) {
            let cli = utf_ClusterInfo(ci);
            let next_vcol = cur_vcol
                + win_charsize(
                    cstype,
                    cur_vcol,
                    ci.ptr.cast_mut(),
                    ci.chr.value,
                    &mut csarg,
                    cli.cells,
                )
                .width;
            if next_vcol > vcol {
                break;
            }
            cur_vcol = next_vcol;
            ci = cli.next;
        }

        if let Some(coladdp) = coladdp {
            *coladdp = vcol - cur_vcol;
        }
        ci.ptr.offset_from(line.cast_const()) as Colnr
    }
}

/// Set UI mouse depending on current mode and 'mouse'.
pub fn setmouse() {
    ui_cursor_shape();
        ui_check_mouse();
}

fn set_mouse_topline(wp: &mut WinInfoInfo) {
    mouse_st().orig_topline = wp.w_topline;
        mouse_st().orig_topfill = wp.w_topfill;
}

/// Return length of line "lnum" for horizontal scrolling.
fn scroll_line_len(lnum: Linenr) -> Colnr {
    unsafe {
        let mut col: Colnr = 0;
        let mut line = ml_get(lnum);
        if !crate::memory::cstr_is_empty(line) {
            loop {
                let numchar = win_chartabsize(curwin_mut(), line, col);
                line = line.add(utfc_ptr2len(line) as usize);
                if crate::memory::cstr_is_empty(line) {
                    break;
                }
                col += numchar;
            }
        }
        col
    }
}

/// Find longest visible line number.
fn find_longest_lnum() -> Linenr {
     {
        let mut ret: Linenr = 0;

        if curwin_mut().w_topline <= curwin_mut().w_cursor.lnum
            && curwin_mut().w_botline > curwin_mut().w_cursor.lnum
            && curwin_mut().w_botline <= curbuf_mut().b_ml.ml_line_count + 1
        {
            let mut max: Colnr = 0;

            for lnum in curwin_mut().w_topline..curwin_mut().w_botline {
                let len = scroll_line_len(lnum);
                if len > max {
                    max = len;
                    ret = lnum;
                } else if len == max
                    && (lnum - curwin_mut().w_cursor.lnum).abs()
                        < (ret - curwin_mut().w_cursor.lnum).abs()
                {
                    ret = lnum;
                }
            }
        } else {
            ret = curwin_mut().w_cursor.lnum;
        }

        ret
    }
}

/// Check clicked cell on its grid
fn mouse_check_grid(vcolp: &mut Colnr, flagsp: &mut c_int) {
    unsafe {
        let mut click_grid = g_st().mouse_grid;
        let mut click_row = g_st().mouse_row;
        let mut click_col = g_st().mouse_col;

        if mouse_find_win_inner(&mut click_grid, &mut click_row, &mut click_col)
            != curwin_ptr()
            || curwin_mut().w_redr_type != 0
        {
            return;
        }
        let mut start_row: c_int = 0;
        let mut start_col: c_int = 0;
        let gp = &*grid_adjust(
            &mut (*curwin_ptr()).w_grid,
            &mut start_row,
            &mut start_col,
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
pub fn f_getmousepos(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData){
    unsafe {
        let mut row = g_st().mouse_row;
        let mut col = g_st().mouse_col;
        let mut grid = g_st().mouse_grid;
        let mut winid: Varnumber = 0;
        let mut winrow: Varnumber = 0;
        let mut wincol: Varnumber = 0;
        let mut lnum: Linenr = 0;
        let mut column: Varnumber = 0;
        let mut coladd: Colnr = 0;

        let d = tv_dict_alloc_ret(rettv);

        tv_dict_add_nr(
            d,
            c"screenrow",
            Varnumber::from(g_st().mouse_row) + 1,
        );
        tv_dict_add_nr(
            d,
            c"screencol",
            Varnumber::from(g_st().mouse_col) + 1,
        );

        let wp = mouse_find_win_inner(&mut grid, &mut row, &mut col);
        if !wp.is_null() {
            let height = (*wp).w_height + (*wp).w_hsep_height + (*wp).w_status_height;
            if row < height + (*wp).w_border_adj[2] {
                winid = Varnumber::from((*wp).handle);
                winrow = Varnumber::from(row + 1 + (*wp).w_winrow_off);
                wincol = Varnumber::from(col + 1 + (*wp).w_wincol_off);
                if row >= 0 && row < (*wp).w_height && col >= 0 && col < (*wp).w_width {
                    mouse_comp_pos(&mut *wp, &mut row, &mut col, &mut lnum);
                    col = vcol2col(&mut *wp, lnum, col, Some(&mut coladd));
                    column = Varnumber::from(col) + 1;
                }
            }
        }
        tv_dict_add_nr(d, c"winid", winid);
        tv_dict_add_nr(d, c"winrow", winrow);
        tv_dict_add_nr(d, c"wincol", wincol);
        tv_dict_add_nr(d, c"line", Varnumber::from(lnum));
        tv_dict_add_nr(d, c"column", column);
        tv_dict_add_nr(
            d,
            c"coladd",
            Varnumber::from(coladd),
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
        let p1 = CursorPos {
            lnum: 1,
            col: 2,
            coladd: 0,
        };
        let p2 = CursorPos {
            lnum: 1,
            col: 2,
            coladd: 1,
        };
        let p3 = CursorPos {
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
