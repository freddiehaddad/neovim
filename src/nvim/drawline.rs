//! Functions for drawing window lines on the screen (`src/nvim/drawline.c`).
//! This is the middle level, `drawscreen.rs` is the top and `grid.rs` the lower level.

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::vim::{HLF_CLN, HLF_CLS, HLF_LNA, HLF_LNB, HLF_N, HLF_SC, SIGN_SHOW_MAX};
use crate::autocmd::HLF_8;
use crate::buffer::{FileBuf, bt_quickfix, buf_meta_total};
use crate::change::TAB;
use crate::charset::{
    K_OPT_DY_FLAG_UHEX, NUL, ascii_isdigit, ascii_iswhite, rl_mirror_ascii, skiptowhite, skipwhite,
    transchar_buf, transchar_hex, transstr_buf, vim_isprintc,
};
use crate::cursor::{Colnr, CursorPos, Linenr, VALID_VIRTCOL, get_cursor_rel_lnum, ltoreq};
use crate::decoration::{
    DecorRange, DecorVirtText, K_VLLEFTCOL, K_VLOVERFLOW_AUTO, K_VLOVERFLOW_SCROLL,
    K_VPOS_END_OF_LINE, K_VPOS_END_OF_LINE_RIGHT_ALIGN, K_VPOS_INLINE, K_VPOS_RIGHT_ALIGN,
    K_VPOS_WIN_COL, K_VTREPEAT_LINEBREAK, NS, SCL_NUM, SignTextAttrs, VirtLines, VirtText,
    VirtTextChunk, decor_has_more_decorations, decor_init_draw_col, decor_recheck_draw_col,
    decor_redraw_col, decor_redraw_eol, decor_redraw_line, decor_redraw_signs,
    decor_virt_line_rows, decor_virt_lines, decor_virt_pos, decor_virt_pos_kind,
    next_virt_text_chunk,
};
use crate::decoration_provider::{decor_providers_invoke_line, decor_providers_invoke_range};
use crate::diff::{
    Diffline, HLF_ADD, HLF_CHD, HLF_TXA, HLF_TXD, diff_change_parse, diff_check_fill,
    diff_check_with_linestatus, diff_find_change,
};
use crate::drawscreen::{
    SIGN_WIDTH, Spellvars, WinExtmark, compute_foldcolumn, conceal_cursor_line, number_width,
    win_draw_end,
};
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_getln::{Disptick, Hlf};
use crate::extmark::{K_FALSE, K_TRUE, TriState};
use crate::fold::get_foldtext;
use crate::globals::*;
use crate::grid::{
    GridView, Schar, grid_adjust, grid_put_linebuf, linebuf_mirror, schar_from_ascii,
    schar_from_char, schar_from_cluster, schar_get, schar_get_adv, schar_get_ascii,
    schar_invisible, schar_len,
};
use crate::highlight::{
    hl_attr, hl_blend_attrs, hl_combine_attr, hl_get_underline, syn_attr2entry, win_bg_attr,
    win_hl_attr,
};
use crate::highlight_group::syn_id2attr;
use crate::indent::{get_breakindent_win, tabstop_padding};
use crate::insexpand::{ins_compl_col_range_attr, ins_compl_lnum_in_range, ins_compl_win_active};
use crate::marktree::K_MTMETA_INLINE;
use crate::r#match::{
    get_prevcol_hl_flag, get_search_match_hl, prepare_search_hl_line, update_search_hl,
};
use crate::mbyte::{
    mb_charlen, mb_off_next, mb_string2cells, utf_cluster_info, utf_head_off,
    utf_ptr2_str_char_info, utf_ptr2cells, utfc_ptr2len,
};
use crate::memline::{gchar_pos, ml_get_buf, ml_get_buf_len};
use crate::memory::xstrlcpy;
use crate::r#move::{set_empty_rows, validate_virtcol, win_col_off, win_col_off2};
use crate::option::get_showbreak_value;
use crate::option_vars::*;
use crate::optionstr::CharBuf;
use crate::os::fs::MAXPATHL;
use crate::plines::{
    CSType, CharInfo, CharSize, CharsizeArg, StrCharInfo, getvcol, getvvcol, init_charsize_arg,
    init_charsize_arg_skip_cur_text, vim_isbreak, win_charsize,
};
use crate::quickfix::qf_current_entry;
use crate::regexp::{CTRL_V_CONST, MAXCOL};
use crate::search::FORWARD;
use crate::spell::{check_need_cap, spell_cat_line, spell_check, spell_move_to, spell_to_word_end};
use crate::state::{MODE_INSERT, virtual_active};
use crate::statusline::{Foldinfo, Statuscol, build_statuscol_str};
use crate::statusline::{STL_FOLDCOL, STL_SIGNCOL};
use crate::strings::vim_snprintf;
use crate::syntax::{
    HL_CONCEAL, get_syntax_attr, get_syntax_info, syn_get_sub_char, syntax_present, syntax_start,
};
use crate::terminal::terminal_get_line_attributes;
use crate::ui::ui_rgb_attached;
use crate::window::WinInfoInfo;
use std::ffi::c_char;
use std::ptr;

type HlMode = u8;

const MB_FILLER_CHAR: u8 = b'<';
const TERM_ATTRS_MAX: usize = 1024;
pub(crate) const MAX_STCWIDTH: i32 = 20 + 9 * SIGN_WIDTH + 9;
const SPWORDLEN: usize = 150;
pub(crate) const SMT_ALL: i32 = 0;

pub(crate) const VALID_WROW: i32 = 0x01;
pub(crate) const VALID_WCOL: i32 = 0x02;
pub(crate) const VALID_CHEIGHT: i32 = 0x08;
pub(crate) const VALID_CROW: i32 = 0x10;

pub(crate) const SLF_RIGHTLEFT: i32 = 1;
pub(crate) const SLF_WRAP: i32 = 2;
pub(crate) const SLF_INC_VCOL: i32 = 4;

pub(crate) const K_OPT_FLAG_INSECURE: u32 = 1 << 18;
pub(crate) const K_OPT_CULOPT_FLAG_LINE: u8 = 0x01;
pub(crate) const K_OPT_CULOPT_FLAG_SCREENLINE: u8 = 0x02;
pub(crate) const K_OPT_CULOPT_FLAG_NUMBER: u8 = 0x04;
pub(crate) const K_OPT_SPO_FLAG_NOPLAINBUFFER: u32 = 0x02;
pub(crate) const K_CPO_NUMCOL: i32 = b'n' as i32;

pub(crate) const K_HL_MODE_REPLACE: HlMode = 1;
pub(crate) const K_HL_MODE_COMBINE: HlMode = 2;
pub(crate) const K_HL_MODE_BLEND: HlMode = 3;

pub(crate) const HL_UNDERLINE_MASK: i32 = 0x38;
pub(crate) const HL_STRIKETHROUGH: i32 = 0x0080;
pub(crate) const HL_OVERLINE: i32 = 0x20000;

pub(crate) const HLF_AT: i32 = 4;
const HLF_I: i32 = 7;
pub(crate) const HLF_CLF: i32 = 17;
const HLF_V: i32 = 24;
const HLF_FL: i32 = 28;
pub(crate) const HLF_FC: i32 = 29;
const HLF_DED: i32 = 32;
const HLF_CONCEAL: i32 = 36;
const HLF_CUC: i32 = 55;
const HLF_CUL: i32 = 56;
const HLF_MC: i32 = 57;
pub(crate) const HLF_QFL: i32 = 58;
pub(crate) const HLF_0: i32 = 59;
pub(crate) const HLF_COUNT: Hlf = 76;

/// Structure with variables passed between `win_line()` and other functions.
struct Winlinevars<'a> {
    lnum: Linenr,
    foldinfo: Foldinfo,
    startrow: i32,
    row: i32,
    vcol: Colnr,
    col: i32,
    boguscols: i32,
    old_boguscols: i32,
    vcol_off_co: i32,
    off: i32,
    cul_attr: i32,
    line_attr: i32,
    line_attr_lowprio: i32,
    sign_num_attr: i32,
    prev_num_attr: i32,
    sign_cul_attr: i32,
    fromcol: i32,
    tocol: i32,
    vcol_sbr: Colnr,
    need_showbreak: bool,
    char_attr: i32,
    n_extra: i32,
    n_attr: i32,
    p_extra: *mut c_char,
    extra_attr: i32,
    sc_extra: Schar,
    sc_final: Schar,
    extra_for_extmark: bool,
    extra: [c_char; 11],
    diff_hlf: Hlf,
    n_virt_lines: i32,
    n_virt_below: i32,
    filler_lines: i32,
    filler_todo: i32,
    virt_below_skip: i32,
    filler_lines_skip: i32,
    sattrs: [SignTextAttrs; SIGN_SHOW_MAX],
    need_lbr: bool,
    virt_inline: &'a [VirtTextChunk],
    virt_inline_i: usize,
    virt_inline_hl_mode: HlMode,
    reset_extra_attr: bool,
    skip_cells: i32,
    skipped_cells: i32,
    color_cols: &'a [i32],
}

struct DrawlineState {
    extra_buf: Vec<c_char>,
    saved_w_virtcol: i32,
    margin_prev_wp: *mut WinInfoInfo,
    prev_width1: i32,
    prev_width2: i32,
    prev_left_col: i32,
    prev_right_col: i32,
    stc_prev_virtnum: i32,
    stc_prev_wp: *mut WinInfoInfo,
    stc_prev_lnum: Linenr,
    stc_prev_tick: Disptick,
}

impl DrawlineState {
    const fn new() -> Self {
        Self {
            extra_buf: Vec::new(),
            saved_w_virtcol: 0,
            margin_prev_wp: ptr::null_mut(),
            prev_width1: 0,
            prev_width2: 0,
            prev_left_col: 0,
            prev_right_col: 0,
            stc_prev_virtnum: 0,
            stc_prev_wp: ptr::null_mut(),
            stc_prev_lnum: 0,
            stc_prev_tick: 0,
        }
    }
}

static DL_STATE: GlobalState<DrawlineState> = GlobalState::new(DrawlineState::new());

#[inline]
fn dl_st() -> &'static mut DrawlineState {
    unsafe { &mut *DL_STATE.as_ptr() }
}

#[inline]
const fn tristate_to_bool(val: TriState, default: bool) -> bool {
    if val == K_TRUE {
        true
    } else if val == K_FALSE {
        false
    } else {
        default
    }
}

#[inline]
const fn vcol_hlc(wlv: &Winlinevars) -> i32 {
    wlv.vcol - wlv.vcol_off_co
}

fn get_extra_buf(mut size: usize) -> *mut c_char {
    size = size.max(64);
    let buf = &mut dl_st().extra_buf;
    if buf.len() < size {
        buf.resize(size, 0);
    }
    buf.as_mut_ptr()
}

pub fn drawline_free_all_mem() {
    dl_st().extra_buf = Vec::new();
}

/// Get the 'listchars' "extends" characters to use for "wp", or NUL if it
/// shouldn't be used.
fn get_lcs_ext(wp: &mut WinInfoInfo) -> Schar {
    if wp.w_onebuf_opt.wo_wrap != 0 {
        return 0;
    }
    if (wp.w_onebuf_opt.wo_wrap_flags & K_OPT_FLAG_INSECURE) != 0 {
        return schar_from_ascii(b'>');
    }
    if wp.w_onebuf_opt.wo_list != 0 {
        wp.w_p_lcs_chars.ext
    } else {
        0
    }
}

/// Advance `wlv.color_cols` if not empty.
fn advance_color_col(wlv: &mut Winlinevars<'_>, vcol: i32) {
    while let Some((&first, rest)) = wlv.color_cols.split_first() {
        if first < 0 {
            wlv.color_cols = &[];
            break;
        }
        if vcol > first {
            wlv.color_cols = rest;
        } else {
            break;
        }
    }
}

/// Used when 'cursorlineopt' contains "screenline": compute the margins between
/// which the highlighting is used.
fn margin_columns_win(wp: &mut WinInfoInfo, left_col: &mut i32, right_col: &mut i32) {
    {
        let cur_col_off = win_col_off(wp);
        let width1 = wp.w_view_width - cur_col_off;
        let width2 = width1 + win_col_off2(&mut *wp);

        if dl_st().saved_w_virtcol == wp.w_virtcol
            && dl_st().margin_prev_wp == wp
            && dl_st().prev_width1 == width1
            && dl_st().prev_width2 == width2
        {
            *right_col = dl_st().prev_right_col;
            *left_col = dl_st().prev_left_col;
            return;
        }

        *left_col = 0;
        *right_col = width1;

        if wp.w_virtcol >= width1 && width2 > 0 {
            *right_col = width1 + ((wp.w_virtcol - width1) / width2 + 1) * width2;
        }
        if wp.w_virtcol >= width1 && width2 > 0 {
            *left_col = (wp.w_virtcol - width1) / width2 * width2 + width1;
        }

        dl_st().prev_left_col = *left_col;
        dl_st().prev_right_col = *right_col;
        dl_st().margin_prev_wp = wp;
        dl_st().prev_width1 = width1;
        dl_st().prev_width2 = width2;
        dl_st().saved_w_virtcol = wp.w_virtcol;
    }
}

/// Put a single char from an UTF-8 buffer into a line buffer.
///
/// If `*pp` is a double-width char and only one cell is left, emit a space,
/// and don't advance `*pp`.
fn draw_virt_text(
    wp: &mut WinInfoInfo,
    buf: &mut FileBuf,
    col_off: i32,
    end_col: &mut i32,
    win_row: i32,
) {
    unsafe {
        let max_col = wp.w_view_width;
        let mut right_pos = max_col;
        let do_eol = g_st().decor_state.eol_col > -1;

        let end = g_st().decor_state.current_end;

        let mut total_width_of_eol_right_aligned_virt_text: i32 = 0;

        for i in 0..end {
            let slot_idx = (&g_st().decor_state.ranges_i)[i as usize] as usize;
            let item: *mut DecorRange = (&mut g_st().decor_state.slots)[slot_idx].range_mut();
            let item = &mut *item;
            if !(item.start_row == g_st().decor_state.row && decor_virt_pos(&*item)) {
                continue;
            }

            let mut vt: *mut DecorVirtText = ptr::null_mut();
            if item.data.is_vt() {
                debug_assert!(!item.data.vt().is_null());
                vt = item.data.vt();
            }
            if decor_virt_pos(&*item) && item.draw_col == -1 {
                let mut updated = true;
                let pos = decor_virt_pos_kind(&*item);

                if do_eol && pos == K_VPOS_END_OF_LINE_RIGHT_ALIGN {
                    let mut eol_offset: i32 = 0;
                    if total_width_of_eol_right_aligned_virt_text == 0 {
                        for j in i..end {
                            let lookahead_idx = (&g_st().decor_state.ranges_i)[j as usize] as usize;
                            let lookahead_item: *mut DecorRange =
                                (&mut g_st().decor_state.slots)[lookahead_idx].range_mut();
                            let lookahead_item = &mut *lookahead_item;

                            if lookahead_item.start_row != g_st().decor_state.row
                                || !decor_virt_pos(&*lookahead_item)
                                || lookahead_item.draw_col != -1
                            {
                                continue;
                            }

                            let mut lookahead_vt: *mut DecorVirtText = ptr::null_mut();
                            if lookahead_item.data.is_vt() {
                                debug_assert!(!lookahead_item.data.vt().is_null());
                                lookahead_vt = lookahead_item.data.vt();
                            }

                            if decor_virt_pos_kind(&*lookahead_item)
                                == K_VPOS_END_OF_LINE_RIGHT_ALIGN
                            {
                                total_width_of_eol_right_aligned_virt_text +=
                                    (*lookahead_vt).width + 1;
                            }
                        }

                        total_width_of_eol_right_aligned_virt_text -= 1;

                        if total_width_of_eol_right_aligned_virt_text
                            <= (right_pos - g_st().decor_state.eol_col)
                        {
                            eol_offset = right_pos
                                - total_width_of_eol_right_aligned_virt_text
                                - g_st().decor_state.eol_col;
                        }
                    }

                    item.draw_col = g_st().decor_state.eol_col + eol_offset;
                } else if pos == K_VPOS_RIGHT_ALIGN {
                    right_pos -= (*vt).width;
                    item.draw_col = right_pos;
                } else if pos == K_VPOS_END_OF_LINE && do_eol {
                    item.draw_col = g_st().decor_state.eol_col;
                } else if pos == K_VPOS_WIN_COL {
                    item.draw_col = (col_off + (*vt).col).max(0);
                } else {
                    updated = false;
                }
                if updated && (item.draw_col < 0 || item.draw_col >= wp.w_view_width) {
                    item.draw_col = i32::MIN;
                }
            }
            if item.draw_col < 0 {
                continue;
            }
            if item.data.is_ui() {
                let ui = item.data.ui();
                let m = WinExtmark {
                    ns_id: ui.ns_id as NS,
                    mark_id: u64::from(ui.mark_id),
                    win_row,
                    win_col: item.draw_col,
                };
                g_st().win_extmark_arr.push(m);
            }
            if !vt.is_null() {
                let vt = { &mut *vt };
                let vcol = item.draw_col - col_off;
                let col = draw_virt_text_item(
                    buf,
                    item.draw_col,
                    vt.data.virt_text(),
                    vt.hl_mode,
                    max_col,
                    vcol,
                    0,
                    false,
                );
                if do_eol
                    && (vt.pos == K_VPOS_END_OF_LINE || vt.pos == K_VPOS_END_OF_LINE_RIGHT_ALIGN)
                {
                    g_st().decor_state.eol_col = col + 1;
                }
                *end_col = (*end_col).max(col);
            }
            if vt.is_null() || ((*vt).flags & K_VTREPEAT_LINEBREAK) == 0 {
                item.draw_col = i32::MIN;
            }
        }
    }
}

fn draw_virt_text_item(
    buf: &mut FileBuf,
    mut col: i32,
    vt: &[VirtTextChunk],
    hl_mode: HlMode,
    max_col: i32,
    mut vcol: i32,
    mut skip_cells: i32,
    mut eol_hl: bool,
) -> i32 {
    {
        let mut virt_attr: i32 = 0;
        let mut virt_pos: usize = 0;
        eol_hl &= vt
            .last()
            .and_then(|c| c.text.as_ref())
            .is_some_and(|s| s.as_bytes().is_empty());

        let mut ci = StrCharInfo {
            ptr: c"".as_ptr(),
            chr: CharInfo { value: -1, len: 0 },
        };

        while col < max_col {
            debug_assert!(skip_cells >= 0);
            while crate::memory::cstr_is_empty(ci.ptr) {
                if virt_pos >= vt.len() {
                    if eol_hl {
                        ci = utf_ptr2_str_char_info(c" ");
                    } else {
                        break;
                    }
                } else {
                    virt_attr = 0;
                    let next = next_virt_text_chunk(vt, &mut virt_pos, &mut virt_attr);
                    if next.is_null() {
                        break;
                    }
                    ci = utf_ptr2_str_char_info(next);
                }
            }
            if crate::memory::cstr_is_empty(ci.ptr) {
                break;
            }

            let cli = utf_cluster_info(ci);
            let mut draw_cells = cli.cells;
            if ci.chr.value == i32::from(TAB) {
                draw_cells = tabstop_padding(vcol, buf.b_p_ts, &buf.b_p_vts_array);
            }
            if skip_cells > 0 {
                skip_cells -= draw_cells;
                vcol += draw_cells;
                ci = cli.next;
                if skip_cells < 0 {
                    draw_cells = -skip_cells;
                } else {
                    continue;
                }
            }

            let maxcells = max_col - col;
            let mut sc = schar_from_ascii(b' ');
            if skip_cells >= 0 && ci.chr.value != i32::from(TAB) && cli.cells <= maxcells {
                sc = schar_from_cluster(ci, cli);
            }
            draw_cells = draw_cells.min(maxcells);

            let mut through = false;
            let attr = if hl_mode == K_HL_MODE_COMBINE {
                hl_combine_attr(g_st().linebuf_attr[col as usize], virt_attr)
            } else if hl_mode == K_HL_MODE_BLEND {
                through = schar_invisible(sc);
                hl_blend_attrs(g_st().linebuf_attr[col as usize], virt_attr, &mut through)
            } else {
                virt_attr
            };

            if !through && g_st().linebuf_char[col as usize] == 0 {
                debug_assert!(col > 0);
                g_st().linebuf_char[(col - 1) as usize] = schar_from_ascii(b' ');
                g_st().linebuf_char[col as usize] = schar_from_ascii(b' ');
            }

            if !through {
                if sc == schar_from_ascii(b' ') {
                    for c in 0..draw_cells {
                        g_st().linebuf_char[(col + c) as usize] = schar_from_ascii(b' ');
                    }
                } else {
                    debug_assert!(draw_cells <= 2);
                    g_st().linebuf_char[col as usize] = sc;
                    if draw_cells == 2 {
                        g_st().linebuf_char[(col + 1) as usize] = 0;
                    }
                }
                if draw_cells < maxcells && g_st().linebuf_char[(col + draw_cells) as usize] == 0 {
                    g_st().linebuf_char[(col + draw_cells) as usize] = schar_from_ascii(b' ');
                }
            }
            for _ in 0..draw_cells {
                g_st().linebuf_attr[col as usize] = attr;
                col += 1;
            }
            if skip_cells < 0 {
                skip_cells = 0;
            } else {
                vcol += draw_cells;
                ci = cli.next;
            }
        }
        col
    }
}

fn draw_col_buf(
    wp: &mut WinInfoInfo,
    wlv: &mut Winlinevars,
    text: impl crate::strings::IntoCCharPtr,
    len: usize,
    attr: i32,
    mut fold_vcol: *const Colnr,
    inc_vcol: bool,
) {
    let text = text.into_c_char_ptr();
    unsafe {
        let mut ci = utf_ptr2_str_char_info(text);
        let end = text.wrapping_add(len);
        while ci.ptr < end && wlv.off < wp.w_view_width {
            let cli = utf_cluster_info(ci);
            let maxcells = wp.w_view_width - wlv.off;
            debug_assert!(cli.cells <= 2 && ci.chr.value >= 0x20);
            let mut cells = cli.cells;
            if cli.cells > maxcells {
                g_st().linebuf_char[wlv.off as usize] = schar_from_ascii(b' ');
                cells = 1;
            } else {
                g_st().linebuf_char[wlv.off as usize] = schar_from_cluster(ci, cli);
                if cells == 2 {
                    g_st().linebuf_char[(wlv.off + 1) as usize] = 0;
                }
            }

            let mut myattr = attr;
            if inc_vcol {
                advance_color_col(wlv, wlv.vcol);
                if wlv.color_cols.first().is_some_and(|&cc| cc == wlv.vcol) {
                    myattr = hl_combine_attr(win_hl_attr(&mut *wp, HLF_MC), myattr);
                }
            }
            for _ in 0..cells {
                g_st().linebuf_attr[wlv.off as usize] = myattr;
                g_st().linebuf_vcol[wlv.off as usize] = if inc_vcol {
                    let v = wlv.vcol;
                    wlv.vcol += 1;
                    v
                } else if !fold_vcol.is_null() {
                    let v = *fold_vcol;
                    fold_vcol = fold_vcol.wrapping_add(1);
                    v
                } else {
                    -1
                };
                wlv.off += 1;
            }
            ci = cli.next;
        }
    }
}

fn draw_col_fill(wlv: &mut Winlinevars, fillchar: Schar, width: i32, attr: i32) {
    for _ in 0..width {
        g_st().linebuf_char[wlv.off as usize] = fillchar;
        g_st().linebuf_attr[wlv.off as usize] = attr;
        wlv.off += 1;
    }
}

/// Return true if CursorLineSign / CursorLineFold highlight is to be used.
/// This depends only on 'cursorline', not on 'cursorlineopt'.
pub fn use_cursor_line_highlight(wp: &mut WinInfoInfo, lnum: Linenr) -> bool {
    wp.w_onebuf_opt.wo_cul != 0 && lnum == wp.w_cursorline
}

/// Setup for drawing the 'foldcolumn', if there is one.
fn draw_foldcolumn(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) {
    {
        let fdc = compute_foldcolumn(&mut *wp, 0);
        if fdc > 0 {
            let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
                HLF_CLF
            } else {
                HLF_FC
            };
            let attr = win_hl_attr(wp, hlf);
            let is_virt = wlv.filler_todo > 0;
            fill_foldcolumn(
                &mut *wp,
                wlv.foldinfo,
                wlv.lnum,
                attr,
                fdc,
                is_virt,
                &raw mut wlv.off,
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
    }
}

#[inline]
fn foldcolumn_sep_char(first_level: i32, i: i32, wp: &mut WinInfoInfo) -> Schar {
    {
        if first_level == 1 {
            wp.w_p_fcs_chars.foldsep
        } else if wp.w_p_fcs_chars.foldinner != 0 {
            wp.w_p_fcs_chars.foldinner
        } else if first_level + i <= 9 {
            schar_from_ascii(b'0' + (first_level + i) as u8)
        } else {
            schar_from_ascii(b'>')
        }
    }
}

/// Draw the foldcolumn or fill "out_buffer". Assume monocell characters.
pub fn fill_foldcolumn(
    wp: &mut WinInfoInfo,
    foldinfo: Foldinfo,
    lnum: Linenr,
    attr: i32,
    fdc: i32,
    is_virt: bool,
    wlv_off: *mut i32,
    out_vcol: *mut Colnr,
    out_buffer: *mut Schar,
) {
    unsafe {
        let closed = foldinfo.fi_level != 0 && foldinfo.fi_lines > 0;
        let level = foldinfo.fi_level;

        let first_level = (level - fdc - i32::from(closed) + 1).max(1);
        let closedcol = fdc.min(level);

        for i in 0..fdc {
            let mut symbol: Schar;
            if i >= level {
                symbol = schar_from_ascii(b' ');
            } else if i == closedcol - 1 && closed {
                symbol = wp.w_p_fcs_chars.foldclosed;
            } else if foldinfo.fi_lnum == lnum && first_level + i >= foldinfo.fi_low_level {
                symbol = wp.w_p_fcs_chars.foldopen;
            } else {
                symbol = foldcolumn_sep_char(first_level, i, wp);
            }

            if is_virt && foldinfo.fi_level != 0 && foldinfo.fi_lnum == lnum {
                let outer_level = (foldinfo.fi_low_level - 1).max(0);
                let outer_first_level = (outer_level - fdc + 1).max(1);
                if i >= outer_level {
                    symbol = schar_from_ascii(b' ');
                } else {
                    symbol = foldcolumn_sep_char(outer_first_level, i, wp);
                }
            }

            let vcol = if i >= level {
                -1
            } else if i == closedcol - 1 && closed {
                -2
            } else {
                -3
            };
            if !out_buffer.is_null() {
                *out_vcol.wrapping_add(i as usize) = vcol;
                *out_buffer.wrapping_add(i as usize) = symbol;
            } else {
                let off = *wlv_off as usize;
                g_st().linebuf_vcol[off] = vcol;
                g_st().linebuf_attr[off] = attr;
                g_st().linebuf_char[off] = symbol;
                *wlv_off += 1;
            }
        }
    }
}

fn draw_sign(nrcol: bool, wp: &mut WinInfoInfo, wlv: &mut Winlinevars, sign_idx: i32) {
    {
        let sattr = wlv.sattrs[sign_idx as usize];
        let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
            HLF_CLS
        } else {
            HLF_SC
        };
        let scl_attr = win_hl_attr(wp, hlf);

        if sattr.text[0] != 0 && wlv.row == wlv.startrow + wlv.filler_lines && wlv.filler_todo <= 0
        {
            let fill = if nrcol {
                number_width(&mut *wp) + 1
            } else {
                SIGN_WIDTH
            };
            let mut attr = if wlv.sign_cul_attr != 0 {
                wlv.sign_cul_attr
            } else if sattr.hl_id != 0 {
                syn_id2attr(sattr.hl_id)
            } else {
                0
            };
            attr = hl_combine_attr(scl_attr, attr);
            draw_col_fill(wlv, schar_from_ascii(b' '), fill, attr);
            let sign_pos = wlv.off - SIGN_WIDTH - i32::from(nrcol);
            debug_assert!(sign_pos >= 0);
            g_st().linebuf_char[sign_pos as usize] = sattr.text[0];
            g_st().linebuf_char[(sign_pos + 1) as usize] = sattr.text[1];
        } else {
            debug_assert!(!nrcol);
            draw_col_fill(wlv, schar_from_ascii(b' '), SIGN_WIDTH, scl_attr);
        }
    }
}

#[inline]
fn get_line_number_str(wp: &mut WinInfoInfo, lnum: Linenr, buf: *mut c_char, buf_len: usize) {
    let mut num: Linenr;
    let mut fmt: *const c_char = c"%*d ".as_ptr();

    if wp.w_onebuf_opt.wo_nu != 0 && wp.w_onebuf_opt.wo_rnu == 0 {
        num = lnum;
    } else {
        num = get_cursor_rel_lnum(&mut *wp, lnum).abs();
        if num == 0 && wp.w_onebuf_opt.wo_nu != 0 && wp.w_onebuf_opt.wo_rnu != 0 {
            num = lnum;
            fmt = c"%-*d ".as_ptr();
        }
    }

    vim_snprintf!(buf, buf_len, fmt, number_width(&mut *wp), num);
}

fn use_cursor_line_nr(wp: &mut WinInfoInfo, wlv: &Winlinevars) -> bool {
    wp.w_onebuf_opt.wo_cul != 0
        && wlv.lnum == wp.w_cursorline
        && (wp.w_p_culopt_flags & K_OPT_CULOPT_FLAG_NUMBER) != 0
        && (wlv.row == wlv.startrow + wlv.filler_lines
            || (wlv.row > wlv.startrow + wlv.filler_lines
                && (wp.w_p_culopt_flags & K_OPT_CULOPT_FLAG_LINE) != 0))
}

fn get_line_number_attr(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) -> i32 {
    {
        let mut numhl_attr = wlv.sign_num_attr;

        if (wlv.n_virt_lines - wlv.filler_todo) < wlv.n_virt_below {
            if wlv.prev_num_attr == -1 {
                decor_redraw_signs(
                    wp,
                    unsafe { &mut *(wp.w_buffer) },
                    wlv.lnum - 2,
                    ptr::null_mut(),
                    None,
                    None,
                    Some(&mut wlv.prev_num_attr),
                );
                if wlv.prev_num_attr > 0 {
                    wlv.prev_num_attr = syn_id2attr(wlv.prev_num_attr);
                }
            }
            numhl_attr = wlv.prev_num_attr;
        }

        if use_cursor_line_nr(wp, wlv) {
            return hl_combine_attr(win_hl_attr(&mut *wp, HLF_CLN), numhl_attr);
        }

        if wp.w_onebuf_opt.wo_rnu != 0 {
            if wlv.lnum < wp.w_cursor.lnum {
                return hl_combine_attr(win_hl_attr(&mut *wp, HLF_LNA), numhl_attr);
            }
            if wlv.lnum > wp.w_cursor.lnum {
                return hl_combine_attr(win_hl_attr(&mut *wp, HLF_LNB), numhl_attr);
            }
        }

        hl_combine_attr(win_hl_attr(&mut *wp, HLF_N), numhl_attr)
    }
}

fn draw_lnum_col(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) {
    unsafe {
        let has_cpo_n = crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_NUMCOL);

        if (wp.w_onebuf_opt.wo_nu != 0 || wp.w_onebuf_opt.wo_rnu != 0)
            && (wlv.row == wlv.startrow + wlv.filler_lines || !has_cpo_n)
            && !((has_cpo_n && wp.w_onebuf_opt.wo_bri == 0)
                && wp.w_skipcol > 0
                && wlv.lnum == wp.w_topline)
        {
            if wp.w_minscwidth == SCL_NUM
                && wlv.sattrs[0].text[0] != 0
                && wlv.row == wlv.startrow + wlv.filler_lines
                && wlv.filler_todo <= 0
            {
                draw_sign(true, &mut *wp, wlv, 0);
            } else {
                let width = number_width(&mut *wp) + 1;
                let attr = get_line_number_attr(&mut *wp, wlv);
                if wlv.row == wlv.startrow + wlv.filler_lines
                    && (wp.w_skipcol == 0
                        || wlv.row > 0
                        || (wp.w_onebuf_opt.wo_nu != 0 && wp.w_onebuf_opt.wo_rnu != 0))
                {
                    let mut buf = [0 as c_char; 32];
                    let buf_ptr = (&raw mut buf).cast::<c_char>();
                    get_line_number_str(wp, wlv.lnum, buf_ptr, buf.len());
                    if wp.w_skipcol > 0 && wlv.startrow == 0 {
                        let mut c = buf_ptr;
                        while crate::memory::cstr_first_byte(c) == b' ' {
                            *c = b'-' as c_char;
                            c = c.wrapping_add(1);
                        }
                    }
                    if wp.w_onebuf_opt.wo_rl != 0 {
                        let num = skipwhite(buf_ptr);
                        rl_mirror_ascii(num, skiptowhite(num));
                    }
                    draw_col_buf(
                        &mut *wp,
                        wlv,
                        buf_ptr,
                        width as usize,
                        attr,
                        ptr::null(),
                        false,
                    );
                } else {
                    draw_col_fill(wlv, schar_from_ascii(b' '), width, attr);
                }
            }
        }
    }
}

fn draw_statuscol(
    wp: &mut WinInfoInfo,
    wlv: &mut Winlinevars,
    col_rows: i32,
    stcp: &mut Statuscol,
) {
    {
        let lnum = wlv.lnum - i32::from((wlv.n_virt_lines - wlv.filler_todo) < wlv.n_virt_below);

        let reset = dl_st().stc_prev_wp != wp
            || dl_st().stc_prev_tick != display_tick_val()
            || lnum != dl_st().stc_prev_lnum;
        let mut reset_virt = if lnum == wlv.lnum {
            wlv.filler_lines_skip
        } else {
            wlv.virt_below_skip
        };
        if reset && lnum < wp.w_topline {
            let mut virt_below_prev: i32 = 0;
            let virt_lines_prev =
                decor_virt_lines(wp, lnum - 1, lnum, Some(&mut virt_below_prev), None, true);
            let diff_fill_prev = diff_check_fill(&mut *wp, lnum - 1);
            reset_virt += virt_lines_prev + diff_fill_prev - virt_below_prev;
        }
        dl_st().stc_prev_virtnum = (if reset {
            -reset_virt
        } else {
            dl_st().stc_prev_virtnum
        }) - i32::from(wlv.filler_todo > 0);
        let virtnum = if wlv.filler_todo > 0 {
            dl_st().stc_prev_virtnum
        } else {
            wlv.row - wlv.startrow - wlv.filler_lines
        };
        let relnum: Linenr = if virtnum == -reset_virt - 1 || virtnum == 0 {
            get_cursor_rel_lnum(wp, lnum).abs()
        } else {
            -1
        };

        dl_st().stc_prev_tick = display_tick_val();
        dl_st().stc_prev_lnum = lnum;
        dl_st().stc_prev_wp = wp;

        let mut buf = [0 as c_char; MAXPATHL];
        let buf_ptr = (&raw mut buf).cast::<c_char>();
        if wp.w_statuscol_line_count != wp.w_nrwidth_line_count {
            wp.w_statuscol_line_count = wp.w_nrwidth_line_count;
            let width = build_statuscol_str(
                wp,
                wp.w_nrwidth_line_count,
                wp.w_nrwidth_line_count,
                0,
                CharBuf {
                    data: buf_ptr,
                    size: MAXPATHL,
                },
                stcp,
            );
            if width > stcp.width {
                let addwidth = (width - stcp.width).min(MAX_STCWIDTH - stcp.width);
                wp.w_nrwidth += addwidth;
                wp.w_nrwidth_width = wp.w_nrwidth;
                if col_rows > 0 {
                    wp.w_redr_statuscol = true;
                    dl_st().stc_prev_lnum = 0;
                    return;
                }
                stcp.width += addwidth;
                wp.w_valid &= !VALID_WCOL;
            }
        }

        let width = build_statuscol_str(
            wp,
            lnum,
            relnum,
            virtnum,
            CharBuf {
                data: buf_ptr,
                size: MAXPATHL,
            },
            stcp,
        );
        if crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_stc)
            || (width > stcp.width && stcp.width < MAX_STCWIDTH)
        {
            if crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_stc) {
                wp.w_nrwidth_line_count = 0;
                wp.w_nrwidth = i32::from(wp.w_onebuf_opt.wo_nu != 0 || wp.w_onebuf_opt.wo_rnu != 0)
                    * number_width(&mut *wp);
            } else {
                wp.w_nrwidth += (width - stcp.width).min(MAX_STCWIDTH - stcp.width);
                wp.w_nrwidth_width = wp.w_nrwidth;
            }
            wp.w_redr_statuscol = true;
            dl_st().stc_prev_lnum = 0;
            return;
        }

        let mut p = buf_ptr;
        let mut transbuf = [0 as c_char; MAXPATHL];
        let transbuf_ptr = (&raw mut transbuf).cast::<c_char>();
        let mut fold_vcol: *const Colnr = ptr::null();
        let len = crate::memory::cstr_len(buf_ptr);
        let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
            HLF_CLS
        } else {
            HLF_SC
        };
        let scl_attr = win_hl_attr(wp, hlf);
        let num_attr = get_line_number_attr(wp, wlv);
        let mut cur_attr = num_attr;

        for sp in &stcp.hlrec {
            let textlen = crate::memory::ptr_offset_from(sp.start, p);
            let translen = transstr_buf(p, textlen, transbuf_ptr, MAXPATHL, true);
            draw_col_buf(wp, wlv, transbuf_ptr, translen, cur_attr, fold_vcol, false);
            let attr = if sp.item == i32::from(STL_SIGNCOL as u8) {
                scl_attr
            } else if sp.item == i32::from(STL_FOLDCOL as u8) {
                0
            } else {
                num_attr
            };
            cur_attr = hl_combine_attr(
                attr,
                if sp.userhl < 0 {
                    syn_id2attr(-sp.userhl)
                } else {
                    0
                },
            );
            fold_vcol = if sp.item == i32::from(STL_FOLDCOL as u8) {
                (&raw const stcp.fold_vcol).cast::<Colnr>()
            } else {
                ptr::null()
            };
            p = sp.start;
        }
        let translen = transstr_buf(
            p,
            crate::memory::ptr_offset_from(buf_ptr.wrapping_add(len), p),
            transbuf_ptr,
            MAXPATHL,
            true,
        );
        draw_col_buf(wp, wlv, transbuf_ptr, translen, cur_attr, fold_vcol, false);
        draw_col_fill(wlv, schar_from_ascii(b' '), stcp.width - width, cur_attr);
    }
}

fn attr_has_line_deco(attr: i32) -> bool {
    let ae = syn_attr2entry(attr);
    let mask: i32 = HL_UNDERLINE_MASK | HL_STRIKETHROUGH | HL_OVERLINE;
    (ae.rgb_ae_attr & mask) != 0 || (ae.cterm_ae_attr & mask) != 0
}

fn handle_breakindent(wp: &mut WinInfoInfo, wlv: &mut Winlinevars, gap_decor_attr: i32) {
    if wp.w_onebuf_opt.wo_bri != 0
        && (wlv.row > wlv.startrow + wlv.filler_lines || wlv.need_showbreak)
    {
        let mut attr = gap_decor_attr;
        if wlv.diff_hlf != 0 {
            attr = hl_combine_attr(attr, win_hl_attr(&mut *wp, wlv.diff_hlf));
        }
        let mut num = get_breakindent_win(wp, ml_get_buf(wp.buf_mut(), wlv.lnum));
        if wlv.row == wlv.startrow {
            num -= win_col_off2(wp);
            if wlv.n_extra < 0 {
                num = 0;
            }
        }

        let vcol_before = wlv.vcol;

        for _ in 0..num {
            g_st().linebuf_char[wlv.off as usize] = schar_from_ascii(b' ');

            advance_color_col(wlv, wlv.vcol);
            let mut myattr = attr;
            if wlv.color_cols.first().is_some_and(|&cc| cc == wlv.vcol) {
                myattr = hl_combine_attr(win_hl_attr(&mut *wp, HLF_MC), myattr);
            }
            g_st().linebuf_attr[wlv.off as usize] = myattr;
            g_st().linebuf_vcol[wlv.off as usize] = wlv.vcol;
            wlv.vcol += 1;
            wlv.off += 1;
        }

        if wlv.fromcol >= vcol_before && wlv.fromcol < wlv.vcol {
            wlv.fromcol = wlv.vcol;
        }

        if wlv.tocol == vcol_before {
            wlv.tocol = wlv.vcol;
        }
    }

    if wp.w_skipcol > 0 && wlv.startrow == 0 && wp.w_onebuf_opt.wo_wrap != 0 && wp.w_briopt_sbr {
        wlv.need_showbreak = false;
    }
}

fn handle_showbreak_and_filler(wp: &mut WinInfoInfo, wlv: &mut Winlinevars, gap_decor_attr: i32) {
    {
        let remaining = wp.w_view_width - wlv.off;
        if wlv.filler_todo > wlv.filler_lines - wlv.n_virt_lines {
            draw_col_fill(wlv, schar_from_ascii(b' '), remaining, 0);
        } else if wlv.filler_todo > 0 {
            let c = wp.w_p_fcs_chars.diff;
            draw_col_fill(wlv, c, remaining, win_hl_attr(&mut *wp, HLF_DED));
        }

        let sbr = get_showbreak_value(&mut *wp);
        if !crate::memory::cstr_is_empty(sbr) && wlv.need_showbreak {
            let mut attr = hl_combine_attr(wlv.cul_attr, gap_decor_attr);
            attr = hl_combine_attr(attr, win_hl_attr(&mut *wp, HLF_AT));
            let vcol_before = wlv.vcol;
            draw_col_buf(
                &mut *wp,
                wlv,
                sbr,
                crate::memory::cstr_len(sbr),
                attr,
                ptr::null(),
                true,
            );
            wlv.vcol_sbr = wlv.vcol;

            if wlv.fromcol >= vcol_before && wlv.fromcol < wlv.vcol {
                wlv.fromcol = wlv.vcol;
            }

            if wlv.tocol == vcol_before {
                wlv.tocol = wlv.vcol;
            }
        }

        if wp.w_skipcol == 0 || wlv.startrow > 0 || wp.w_onebuf_opt.wo_wrap == 0 || !wp.w_briopt_sbr
        {
            wlv.need_showbreak = false;
        }
    }
}

fn apply_cursorline_highlight(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.cul_attr = win_hl_attr(&mut *wp, HLF_CUL);
    let ae = syn_attr2entry(wlv.cul_attr);
    if ae.rgb_fg_color == -1 && ae.cterm_fg_color == 0 {
        wlv.line_attr_lowprio = wlv.cul_attr;
    } else if (g_st().state_global & MODE_INSERT) == 0
        && bt_quickfix(unsafe { wp.w_buffer.as_ref() })
        && qf_current_entry(wp) == wlv.lnum
    {
        wlv.line_attr = hl_combine_attr(wlv.cul_attr, wlv.line_attr);
    } else {
        wlv.line_attr = wlv.cul_attr;
    }
}

fn set_line_attr_for_diff(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.line_attr = win_hl_attr(&mut *wp, wlv.diff_hlf);
    if wlv.cul_attr != 0 {
        wlv.line_attr = if wlv.line_attr_lowprio != 0 {
            hl_combine_attr(
                hl_combine_attr(wlv.cul_attr, wlv.line_attr),
                hl_get_underline(),
            )
        } else {
            hl_combine_attr(wlv.line_attr, wlv.cul_attr)
        };
    }
}

fn has_more_inline_virt(wlv: &Winlinevars, v: isize) -> bool {
    unsafe {
        if wlv.virt_inline_i < wlv.virt_inline.len() {
            return true;
        }

        let count = g_st().decor_state.ranges_i.len() as i32;
        let cur_end = g_st().decor_state.current_end;
        let fut_beg = g_st().decor_state.future_begin;

        let beg_pos = [0, fut_beg];
        let end_pos = [cur_end, count];

        for pos_i in 0..2 {
            for i in beg_pos[pos_i]..end_pos[pos_i] {
                let slot_idx = (&g_st().decor_state.ranges_i)[i as usize] as usize;
                let item = (&g_st().decor_state.slots)[slot_idx].range();
                if item.start_row != g_st().decor_state.row
                    || !item.data.is_vt()
                    || (*item.data.vt()).pos != K_VPOS_INLINE
                    || (*item.data.vt()).width == 0
                {
                    continue;
                }
                if item.draw_col >= -1 && isize::try_from(item.start_col).unwrap_or(0) >= v {
                    return true;
                }
            }
        }
        false
    }
}

fn handle_inline_virtual_text(wlv: &mut Winlinevars, v: isize, selected: bool) {
    unsafe {
        while wlv.n_extra == 0 {
            if wlv.virt_inline_i >= wlv.virt_inline.len() {
                wlv.virt_inline = &[];
                wlv.virt_inline_i = 0;
                let end = g_st().decor_state.current_end;

                for i in 0..end {
                    let slot_idx = (&g_st().decor_state.ranges_i)[i as usize] as usize;
                    let item: *mut DecorRange =
                        (&mut g_st().decor_state.slots)[slot_idx].range_mut();
                    let item = &mut *item;
                    if item.draw_col == -3 {
                        decor_init_draw_col(wlv.off, selected, &mut *item);
                    }
                    if item.start_row != g_st().decor_state.row
                        || !item.data.is_vt()
                        || (*item.data.vt()).pos != K_VPOS_INLINE
                        || (*item.data.vt()).width == 0
                    {
                        continue;
                    }
                    if item.draw_col >= -1 && isize::try_from(item.start_col).unwrap_or(-1) == v {
                        wlv.virt_inline = (*item.data.vt()).data.virt_text().as_slice();
                        wlv.virt_inline_hl_mode = (*item.data.vt()).hl_mode;
                        item.draw_col = i32::MIN;
                        break;
                    }
                }
                if wlv.virt_inline.is_empty() {
                    break;
                }
            } else {
                let mut attr: i32 = 0;
                let text = next_virt_text_chunk(wlv.virt_inline, &mut wlv.virt_inline_i, &mut attr);
                if text.is_null() {
                    continue;
                }
                wlv.p_extra = text;
                wlv.n_extra = crate::memory::cstr_len(text) as i32;
                if wlv.n_extra == 0 {
                    continue;
                }
                wlv.sc_extra = 0;
                wlv.sc_final = 0;
                wlv.extra_attr = attr;
                wlv.n_attr = mb_charlen(text);
                if wlv.skip_cells > 0 {
                    let virt_text_width = mb_string2cells(wlv.p_extra) as i32;
                    if virt_text_width > wlv.skip_cells {
                        let mut skip_cells_remaining = wlv.skip_cells;
                        while skip_cells_remaining > 0 {
                            let cells = utf_ptr2cells(wlv.p_extra);
                            if cells > skip_cells_remaining {
                                break;
                            }
                            let c_len = utfc_ptr2len(wlv.p_extra);
                            skip_cells_remaining -= cells;
                            wlv.p_extra = wlv.p_extra.wrapping_add(c_len as usize);
                            wlv.n_extra -= c_len;
                            wlv.n_attr -= 1;
                        }
                        wlv.skipped_cells += wlv.skip_cells - skip_cells_remaining;
                        wlv.skip_cells = skip_cells_remaining;
                    } else {
                        wlv.skip_cells -= virt_text_width;
                        wlv.skipped_cells += virt_text_width;
                        wlv.n_attr = 0;
                        wlv.n_extra = 0;
                        continue;
                    }
                }
                debug_assert!(wlv.n_extra > 0);
                wlv.extra_for_extmark = true;
            }
        }
    }
}

fn win_line_start(wp: &mut WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.col = 0;
    wlv.off = 0;
    wlv.need_lbr = false;
    for i in 0..wp.w_view_width as usize {
        g_st().linebuf_char[i] = schar_from_ascii(b' ');
        g_st().linebuf_attr[i] = 0;
        g_st().linebuf_vcol[i] = -1;
    }
}

fn fix_for_boguscols(wlv: &mut Winlinevars) {
    wlv.n_extra += wlv.vcol_off_co;
    wlv.vcol -= wlv.vcol_off_co;
    wlv.vcol_off_co = 0;
    wlv.col -= wlv.boguscols;
    wlv.old_boguscols = wlv.boguscols;
    wlv.boguscols = 0;
}

fn get_rightmost_vcol(wp: &mut WinInfoInfo, color_cols: &[i32]) -> i32 {
    let mut ret: i32 = 0;

    if wp.w_onebuf_opt.wo_cuc != 0 {
        ret = wp.w_virtcol;
    }

    for &cc in color_cols {
        if cc < 0 {
            break;
        }
        ret = ret.max(cc);
    }

    ret
}

#[inline]
fn schar_get_single_width(mb_schar: Schar) -> i32 {
    #[cfg(debug_assertions)]
    {
        let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
        schar_get(sc_buf.as_mut_ptr(), mb_schar);
        debug_assert_eq!(crate::mbyte::ptr2cells(sc_buf.as_ptr()), 1);
    }
    let _ = mb_schar;
    1
}

/// Display line "lnum" of window "wp" on the screen.
pub fn win_line(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    startrow: i32,
    endrow: i32,
    col_rows: i32,
    concealed: bool,
    spv: &mut Spellvars,
    foldinfo: Foldinfo,
) -> i32 {
    unsafe {
        let grid: *mut GridView = &raw mut wp.w_grid;
        let grid = &mut *grid;
        let view_width = wp.w_view_width;
        let view_height = wp.w_view_height;

        let in_curline = ptr::eq(wp, curwin_ptr()) && lnum == curwin_mut().w_cursor.lnum;
        let has_fold = foldinfo.fi_level != 0 && foldinfo.fi_lines > 0;
        let has_foldtext = has_fold && wp.w_onebuf_opt.wo_fdt.cb_type() != K_CALLBACK_NONE_CONST;

        let is_wrapped = wp.w_onebuf_opt.wo_wrap != 0 && !has_fold;

        let mut saved_attr2: i32 = 0;
        let mut n_attr3: i32 = 0;
        let mut saved_attr3: i32 = 0;

        let mut lnum_in_visual_area = false;

        let mut char_attr_pri: i32 = 0;
        let mut char_attr_base: i32;
        let mut area_highlighting = false;
        let mut vi_attr: i32 = 0;
        let mut area_attr: i32 = 0;
        let mut search_attr: i32 = 0;
        let mut vcol_save_attr: i32;
        let mut decor_attr: i32 = 0;
        let mut syntax_attr: i32;
        let mut gap_attr_save: i32 = 0;
        let mut has_syntax = false;
        let mut folded_attr: i32 = 0;
        let mut eol_hl_off: i32 = 0;
        let mut nextline = [0 as c_char; SPWORDLEN * 2];
        let nextline_ptr = (&raw mut nextline).cast::<c_char>();
        let mut nextlinecol: i32 = 0;
        let mut nextline_idx: i32 = 0;
        let mut spell_attr: i32 = 0;
        let mut word_end: i32 = 0;
        let mut cur_checked_col: i32 = 0;
        let mut extra_check = false;
        let mut multi_attr: i32 = 0;
        let mut mb_schar: Schar;
        let mut mb_schar_width: i32 = 0;
        let mut change_start: i32 = MAXCOL;
        let mut change_end: i32 = -1;
        let mut in_multispace = false;
        let mut multispace_pos: usize = 0;

        let mut n_extra_next: i32 = 0;
        let mut extra_attr_next: i32 = -1;

        let mut search_attr_from_match = false;
        let mut has_decor = false;

        let mut saved_search_attr: i32 = 0;
        let mut saved_area_attr: i32 = 0;
        let mut saved_decor_attr: i32 = 0;
        let mut saved_search_attr_from_match = false;

        let mut win_col_offset: i32 = 0;
        let mut area_active = false;
        let mut decor_need_recheck = false;

        let mut fold_vt: VirtText = Vec::new();
        let mut foldtext_free: Option<std::ffi::CString> = None;

        let mut cul_screenline = false;
        let mut left_curline_col: i32 = 0;
        let mut right_curline_col: i32 = 0;

        let mut match_conc: i32 = 0;
        let mut on_last_col = false;
        let mut syntax_flags: i32 = 0;
        let mut syntax_seqnr: i32 = 0;
        let mut prev_syntax_id: i32 = 0;
        let conceal_attr = win_hl_attr(&mut *wp, HLF_CONCEAL);
        let mut is_concealing = false;
        let mut did_wcol = false;

        debug_assert!(startrow < endrow);

        let mut wlv = Winlinevars {
            lnum,
            foldinfo,
            startrow,
            row: startrow,
            vcol: 0,
            col: 0,
            boguscols: 0,
            old_boguscols: 0,
            vcol_off_co: 0,
            off: 0,
            cul_attr: 0,
            line_attr: 0,
            line_attr_lowprio: 0,
            sign_num_attr: 0,
            prev_num_attr: -1,
            sign_cul_attr: 0,
            fromcol: -10,
            tocol: MAXCOL,
            vcol_sbr: -1,
            need_showbreak: false,
            char_attr: 0,
            n_extra: 0,
            n_attr: 0,
            p_extra: ptr::null_mut(),
            extra_attr: 0,
            sc_extra: 0,
            sc_final: 0,
            extra_for_extmark: false,
            extra: [0; 11],
            diff_hlf: 0,
            n_virt_lines: 0,
            n_virt_below: 0,
            filler_lines: 0,
            filler_todo: 0,
            virt_below_skip: 0,
            filler_lines_skip: 0,
            sattrs: [SignTextAttrs {
                text: [0, 0],
                hl_id: 0,
            }; SIGN_SHOW_MAX],
            need_lbr: false,
            virt_inline: &[],
            virt_inline_i: 0,
            virt_inline_hl_mode: 0,
            reset_extra_attr: false,
            skip_cells: 0,
            skipped_cells: 0,
            color_cols: &[],
        };

        let buf = wp.w_buffer;
        let draw_text = !concealed && (lnum != (*buf).b_ml.ml_line_count + 1);

        let mut decor_provider_end_col: i32 = 0;
        let mut check_decor_providers = false;

        if col_rows == 0 && draw_text {
            extra_check = wp.w_onebuf_opt.wo_lbr != 0;
            let w_s = wp.w_s_ptr();
            if syntax_present(&mut *wp) && !w_s.b_syn_error && !w_s.b_syn_slow && !has_foldtext {
                let save_did_emsg = g_st().did_emsg;
                g_st().did_emsg = 0;
                syntax_start(wp, lnum);
                if g_st().did_emsg != 0 {
                    w_s.b_syn_error = true;
                } else {
                    g_st().did_emsg = save_did_emsg;
                    if !w_s.b_syn_slow {
                        has_syntax = true;
                        extra_check = true;
                    }
                }
            }

            check_decor_providers = true;

            wlv.color_cols = if !wp.buf_mut().terminal.is_null() || wp.w_p_cc_cols.is_empty() {
                &[]
            } else {
                std::slice::from_raw_parts(wp.w_p_cc_cols.as_ptr(), wp.w_p_cc_cols.len())
            };
            let hlc = vcol_hlc(&wlv);
            advance_color_col(&mut wlv, hlc);

            if g_st().visual_global.active && wp.w_buffer == curwin_mut().w_buffer {
                let (top, bot) = if ltoreq(curwin_mut().w_cursor, g_st().visual_global.start) {
                    (
                        &raw mut curwin_mut().w_cursor,
                        &raw mut g_st().visual_global.start,
                    )
                } else {
                    (
                        &raw mut g_st().visual_global.start,
                        &raw mut curwin_mut().w_cursor,
                    )
                };
                lnum_in_visual_area = lnum >= (*top).lnum && lnum <= (*bot).lnum;
                if g_st().visual_global.mode == CTRL_V_CONST {
                    if lnum_in_visual_area {
                        wlv.fromcol = wp.w_old_cursor_fcol;
                        wlv.tocol = wp.w_old_cursor_lcol;
                    }
                } else {
                    if lnum > (*top).lnum && lnum <= (*bot).lnum {
                        wlv.fromcol = 0;
                    } else if lnum == (*top).lnum {
                        if g_st().visual_global.mode == b'V' as i32 {
                            wlv.fromcol = 0;
                        } else {
                            getvvcol(&mut *wp, &mut *top, Some(&mut wlv.fromcol), None, None, 0);
                            if gchar_pos(&mut *top) == i32::from(NUL) {
                                wlv.tocol = wlv.fromcol + 1;
                            }
                        }
                    }
                    if g_st().visual_global.mode != b'V' as i32 && lnum == (*bot).lnum {
                        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                            && (*bot).col == 0
                            && (*bot).coladd == 0
                        {
                            wlv.fromcol = -10;
                            wlv.tocol = MAXCOL;
                        } else if (*bot).col == MAXCOL {
                            wlv.tocol = MAXCOL;
                        } else {
                            let mut pos = *bot;
                            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                                getvvcol(&mut *wp, &mut pos, Some(&mut wlv.tocol), None, None, 0);
                            } else {
                                getvvcol(&mut *wp, &mut pos, None, None, Some(&mut wlv.tocol), 0);
                                wlv.tocol += 1;
                            }
                        }
                    }
                }

                if wlv.fromcol >= 0 {
                    area_highlighting = true;
                    vi_attr = win_hl_attr(&mut *wp, HLF_V);
                }
            } else if g_st().search_global.hl_match
                && ptr::eq(wp, curwin_ptr())
                && !has_foldtext
                && lnum >= curwin_mut().w_cursor.lnum
                && lnum <= curwin_mut().w_cursor.lnum + g_st().search_global.match_lines
            {
                if lnum == curwin_mut().w_cursor.lnum {
                    getvcol(
                        curwin_mut(),
                        &mut curwin_mut().w_cursor,
                        Some(&mut wlv.fromcol),
                        None,
                        None,
                        0,
                    );
                } else {
                    wlv.fromcol = 0;
                }
                if lnum == curwin_mut().w_cursor.lnum + g_st().search_global.match_lines {
                    let mut pos = CursorPos {
                        lnum,
                        col: g_st().search_global.match_endcol,
                        coladd: 0,
                    };
                    getvcol(curwin_mut(), &mut pos, Some(&mut wlv.tocol), None, None, 0);
                }
                if wlv.fromcol == wlv.tocol && g_st().search_global.match_endcol != 0 {
                    wlv.tocol = wlv.fromcol + 1;
                }
                area_highlighting = true;
                vi_attr = win_hl_attr(&mut *wp, HLF_I);
            }
        }

        let bg_attr = win_bg_attr(wp);

        let mut linestatus: i32 = 0;
        wlv.filler_lines = diff_check_with_linestatus(&mut *wp, lnum, Some(&mut linestatus));
        let mut line_changes = Diffline::default();
        let mut change_index: i32 = -1;
        if linestatus < 0 {
            if linestatus == -1 {
                if diff_find_change(wp, lnum, &mut line_changes) {
                    wlv.diff_hlf = HLF_ADD;
                } else if !line_changes.changes.is_empty() {
                    let added = diff_change_parse(
                        &line_changes,
                        &line_changes.changes[0],
                        &mut change_start,
                        &mut change_end,
                    );
                    if change_start == 0 {
                        if added {
                            wlv.diff_hlf = HLF_TXA;
                        } else {
                            wlv.diff_hlf = HLF_TXD;
                        }
                    } else {
                        wlv.diff_hlf = HLF_CHD;
                    }
                    change_index = 0;
                } else {
                    wlv.diff_hlf = HLF_CHD;
                    change_index = 0;
                }
            } else {
                wlv.diff_hlf = HLF_ADD;
            }
            area_highlighting = true;
        }
        let mut virt_lines: VirtLines = Vec::new();
        wlv.n_virt_lines = decor_virt_lines(
            &mut *wp,
            lnum - 1,
            lnum,
            Some(&mut wlv.n_virt_below),
            Some(&mut virt_lines),
            true,
        );
        let total_virt_rows = wlv.n_virt_lines;
        wlv.filler_lines += wlv.n_virt_lines;
        if lnum == wp.w_topline {
            wlv.virt_below_skip = wlv.n_virt_below.min(wlv.n_virt_lines - wp.w_topfill);
            wlv.n_virt_below -= wlv.virt_below_skip;
            wlv.filler_lines_skip = wlv.filler_lines - wlv.virt_below_skip - wp.w_topfill;
            wlv.filler_lines = wp.w_topfill;
            wlv.n_virt_lines = wlv.n_virt_lines.min(wlv.filler_lines);
        }
        wlv.filler_todo = wlv.filler_lines;

        if wp.w_onebuf_opt.wo_cul != 0
            && wp.w_p_culopt_flags != K_OPT_CULOPT_FLAG_NUMBER
            && lnum == wp.w_cursorline
            && !(ptr::eq(wp, curwin_ptr()) && g_st().visual_global.active)
        {
            cul_screenline =
                is_wrapped && (wp.w_p_culopt_flags & K_OPT_CULOPT_FLAG_SCREENLINE) != 0;
            if !cul_screenline {
                apply_cursorline_highlight(wp, &mut wlv);
            } else {
                margin_columns_win(&mut *wp, &mut left_curline_col, &mut right_curline_col);
            }
            area_highlighting = true;
        }

        let mut sign_line_attr: i32 = 0;
        decor_redraw_signs(
            &raw mut *wp,
            buf,
            wlv.lnum - 1,
            (&raw mut wlv.sattrs).cast::<SignTextAttrs>(),
            Some(&mut sign_line_attr),
            Some(&mut wlv.sign_cul_attr),
            Some(&mut wlv.sign_num_attr),
        );

        let mut statuscol = Statuscol::new();
        if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_stc) {
            statuscol.draw = true;
            statuscol.sattrs = wlv.sattrs;
            statuscol.lnum = lnum;
            statuscol.foldinfo = foldinfo;
            statuscol.width = win_col_off(&raw mut *wp);
            statuscol.sign_cul_id = if use_cursor_line_highlight(&mut *wp, lnum) {
                wlv.sign_cul_attr
            } else {
                0
            };
        } else if wlv.sign_cul_attr > 0 {
            wlv.sign_cul_attr = if use_cursor_line_highlight(&mut *wp, lnum) {
                syn_id2attr(wlv.sign_cul_attr)
            } else {
                0
            };
        }
        if wlv.sign_num_attr > 0 {
            wlv.sign_num_attr = syn_id2attr(wlv.sign_num_attr);
        }
        if sign_line_attr > 0 {
            wlv.line_attr = syn_id2attr(sign_line_attr);
        }

        if bt_quickfix(wp.w_buffer.as_ref()) && qf_current_entry(wp) == lnum {
            wlv.line_attr = win_hl_attr(&mut *wp, HLF_QFL);
        }

        if wlv.line_attr_lowprio != 0 || wlv.line_attr != 0 {
            area_highlighting = true;
        }

        let line_attr_save = wlv.line_attr;
        let line_attr_lowprio_save = wlv.line_attr_lowprio;

        if spv.spv_has_spell && col_rows == 0 && draw_text {
            extra_check = true;

            if lnum == spv.spv_checked_lnum {
                cur_checked_col = spv.spv_checked_col;
            }
            if spv.spv_capcol_lnum == 0 && check_need_cap(wp, lnum, 0) {
                spv.spv_cap_col = 0;
            } else if lnum != spv.spv_capcol_lnum {
                spv.spv_cap_col = -1;
            }
            spv.spv_checked_lnum = 0;

            nextline[SPWORDLEN] = NUL;
            if lnum < wp.buf_mut().b_ml.ml_line_count {
                let line = ml_get_buf(wp.buf_mut(), lnum + 1);
                spell_cat_line(nextline_ptr.wrapping_add(SPWORDLEN), line, SPWORDLEN as i32);
            }
            let line = ml_get_buf(wp.buf_mut(), lnum);

            let ptr_sw = skipwhite(line);
            if crate::memory::cstr_is_empty(ptr_sw) {
                spv.spv_cap_col = 0;
                spv.spv_capcol_lnum = lnum + 1;
            } else if spv.spv_cap_col == 0 {
                spv.spv_cap_col = crate::memory::ptr_offset_from(ptr_sw, line) as i32;
            }

            if nextline[SPWORDLEN] == NUL {
                nextlinecol = MAXCOL;
                nextline_idx = 0;
            } else {
                let line_len = ml_get_buf_len(wp.buf_mut(), lnum);
                if (line_len as usize) < SPWORDLEN {
                    nextlinecol = 0;
                    ptr::copy(line, nextline_ptr, line_len as usize);
                    crate::memory::cstr_move(
                        nextline_ptr.wrapping_add(line_len as usize),
                        nextline_ptr.wrapping_add(SPWORDLEN),
                    );
                    nextline_idx = line_len + 1;
                } else {
                    nextlinecol = line_len - SPWORDLEN as i32;
                    ptr::copy(
                        line.wrapping_add(nextlinecol as usize),
                        nextline_ptr,
                        SPWORDLEN,
                    );
                    nextline_idx = SPWORDLEN as i32 + 1;
                }
            }
        }

        let mut line: *mut c_char = if draw_text {
            ml_get_buf(wp.buf_mut(), lnum)
        } else {
            c"".as_ptr().cast_mut()
        };
        let mut ptr: *const c_char = line;

        let mut trailcol: Colnr = MAXCOL;
        let mut leadcol: Colnr = 0;

        let mut lcs_eol_todo = true;
        let lcs_eol = wp.w_p_lcs_chars.eol;
        let mut lcs_prec_todo = wp.w_p_lcs_chars.prec;

        if wp.w_onebuf_opt.wo_list != 0 && !has_foldtext && draw_text {
            if wp.w_p_lcs_chars.space != 0
                || !wp.w_p_lcs_chars.multispace.is_empty()
                || !wp.w_p_lcs_chars.leadmultispace.is_empty()
                || wp.w_p_lcs_chars.trail != 0
                || wp.w_p_lcs_chars.lead != 0
                || wp.w_p_lcs_chars.nbsp != 0
            {
                extra_check = true;
            }
            if wp.w_p_lcs_chars.trail != 0 {
                trailcol = ml_get_buf_len(wp.buf_mut(), lnum);
                while trailcol > 0 && ascii_iswhite(*ptr.wrapping_add((trailcol - 1) as usize)) {
                    trailcol -= 1;
                }
                trailcol += crate::memory::ptr_offset_from(ptr, line) as Colnr;
            }
            if wp.w_p_lcs_chars.lead != 0
                || !wp.w_p_lcs_chars.leadmultispace.is_empty()
                || wp.w_p_lcs_chars.leadtab1 != 0
            {
                leadcol = 0;
                while ascii_iswhite(crate::memory::cstr_char_at(ptr, leadcol as usize)) {
                    leadcol += 1;
                }
                if crate::memory::cstr_byte_at(ptr, leadcol as usize) == 0 {
                    leadcol = 0;
                } else {
                    leadcol += crate::memory::ptr_offset_from(ptr, line) as Colnr + 1;
                }
            }
        }

        let start_vcol = if wp.w_onebuf_opt.wo_wrap != 0 {
            if startrow == 0 { wp.w_skipcol } else { 0 }
        } else {
            wp.w_leftcol
        };

        if has_foldtext {
            wlv.vcol = start_vcol;
        } else if start_vcol > 0 && col_rows == 0 {
            let mut prev_ptr = ptr;
            let mut cs = CharSize::default();

            let mut csarg = CharsizeArg::new();
            let cstype: CSType = init_charsize_arg(&mut csarg, wp, lnum, line);
            csarg.max_head_vcol = start_vcol;
            let mut vcol = wlv.vcol;
            let mut ci = utf_ptr2_str_char_info(ptr);
            while vcol < start_vcol {
                let cli = utf_cluster_info(ci);
                cs = win_charsize(cstype, vcol, ci.ptr, ci.chr.value, &mut csarg, cli.cells);
                vcol += cs.width;
                prev_ptr = ci.ptr;
                if crate::memory::cstr_is_empty(prev_ptr) {
                    break;
                }
                ci = cli.next;
                if wp.w_onebuf_opt.wo_list != 0 {
                    in_multispace = crate::memory::cstr_first_byte(prev_ptr) == b' '
                        && (crate::memory::cstr_first_byte(ci.ptr) == b' '
                            || (prev_ptr > line as *const c_char
                                && *prev_ptr.wrapping_sub(1) == b' ' as c_char));
                    if !in_multispace {
                        multispace_pos = 0;
                    } else if ci.ptr >= (line as *const c_char).wrapping_add(leadcol as usize)
                        && !wp.w_p_lcs_chars.multispace.is_empty()
                    {
                        multispace_pos += 1;
                        if multispace_pos >= wp.w_p_lcs_chars.multispace.len() {
                            multispace_pos = 0;
                        }
                    } else if ci.ptr < (line as *const c_char).wrapping_add(leadcol as usize)
                        && !wp.w_p_lcs_chars.leadmultispace.is_empty()
                    {
                        multispace_pos += 1;
                        if multispace_pos >= wp.w_p_lcs_chars.leadmultispace.len() {
                            multispace_pos = 0;
                        }
                    }
                }
            }
            wlv.vcol = vcol;
            ptr = ci.ptr;
            let charsize = cs.width;
            let head = cs.head;

            if wlv.vcol < start_vcol
                && (wp.w_onebuf_opt.wo_cuc != 0
                    || !wlv.color_cols.is_empty()
                    || virtual_active(wp)
                    || (g_st().visual_global.active && wp.w_buffer == curwin_mut().w_buffer)
                    || has_fold)
            {
                wlv.vcol = start_vcol;
            }

            if wlv.vcol > start_vcol {
                wlv.vcol -= charsize;
                ptr = prev_ptr;
            }

            if start_vcol > wlv.vcol {
                wlv.skip_cells = start_vcol - wlv.vcol - head;
            }

            if wlv.tocol <= wlv.vcol {
                wlv.fromcol = 0;
            } else if wlv.fromcol >= 0 && wlv.fromcol < wlv.vcol {
                wlv.fromcol = wlv.vcol;
            }

            if wp.w_onebuf_opt.wo_wrap != 0 {
                wlv.need_showbreak = true;
            }
            if spv.spv_has_spell {
                let linecol = crate::memory::ptr_offset_from(ptr, line) as Colnr;
                let mut spell_hlf: Hlf = HLF_COUNT;

                let pos = wp.w_cursor;
                wp.w_cursor.lnum = lnum;
                wp.w_cursor.col = linecol;
                let len = spell_move_to(&mut *wp, FORWARD, SMT_ALL, true, &raw mut spell_hlf);

                line = ml_get_buf(wp.buf_mut(), lnum);
                ptr = line.wrapping_add(linecol as usize);

                if len == 0 || wp.w_cursor.col > linecol {
                    word_end = crate::memory::ptr_offset_from(
                        spell_to_word_end(ptr.cast_mut(), &mut *wp),
                        line,
                    ) as i32
                        + 1;
                } else {
                    debug_assert!(len <= i32::MAX as usize);
                    word_end = wp.w_cursor.col + len as i32 + 1;

                    if spell_hlf != HLF_COUNT {
                        spell_attr = g_st().highlight_attr[spell_hlf as usize];
                    }
                }
                wp.w_cursor = pos;

                if has_syntax {
                    syntax_start(wp, lnum);
                }
            }
        }

        if check_decor_providers {
            let col = crate::memory::ptr_offset_from(ptr, line) as i32;
            decor_provider_end_col =
                decor_providers_setup(endrow - startrow, start_vcol == 0, lnum, col, wp);
            line = ml_get_buf(wp.buf_mut(), lnum);
            ptr = line.wrapping_add(col as usize);
        }

        decor_redraw_line(&mut *wp, lnum - 1, &mut g_st().decor_state);
        if !has_decor && decor_has_more_decorations(&mut g_st().decor_state, lnum - 1) {
            has_decor = true;
            extra_check = true;
        }

        if wlv.fromcol >= wlv.tocol {
            wlv.fromcol = -1;
        }

        if col_rows == 0 && draw_text && !has_foldtext {
            let v = crate::memory::ptr_offset_from(ptr, line) as i32;
            area_highlighting |= prepare_search_hl_line(
                wp,
                lnum,
                v,
                &raw mut line,
                &mut g_st().screen_search_hl,
                &mut search_attr,
                &mut search_attr_from_match,
            );
            ptr = line.wrapping_add(v as usize);
        }

        if (g_st().state_global & MODE_INSERT) != 0
            && ins_compl_win_active(wp)
            && (in_curline || ins_compl_lnum_in_range(lnum))
        {
            area_highlighting = true;
        }

        win_line_start(wp, &mut wlv);
        let mut draw_cols = true;
        let mut leftcols_width: i32 = 0;

        let mut term_attrs = [0_i32; TERM_ATTRS_MAX];
        if !wp.buf_mut().terminal.is_null() {
            terminal_get_line_attributes(
                &mut *wp.buf_mut().terminal,
                lnum,
                (&raw mut term_attrs).cast::<i32>(),
            );
            extra_check = true;
        }

        let may_have_inline_virt =
            !has_foldtext && buf_meta_total(wp.buf_mut(), K_MTMETA_INLINE) > 0;
        let mut has_virt_line = false;
        let mut virt_line_flags: i32 = 0;
        let mut virt_line_index: i32 = 0;
        let mut virt_line_start_row: i32 = 0;
        let mut virt_line_skip_cells: i32 = 0;

        loop {
            let mut has_match_conc: i32 = 0;
            let mut decor_conceal: i32 = 0;

            let mut did_decrement_ptr = false;

            if check_decor_providers
                && (crate::memory::ptr_offset_from(ptr, line) as i32) >= decor_provider_end_col
            {
                let col = crate::memory::ptr_offset_from(ptr, line) as i32;
                decor_provider_end_col = invoke_range_next(wp, lnum, col, 100);
                line = ml_get_buf(wp.buf_mut(), lnum);
                ptr = line.wrapping_add(col as usize);
                if !has_decor && decor_has_more_decorations(&mut g_st().decor_state, lnum - 1) {
                    has_decor = true;
                    extra_check = true;
                }
            }

            let mut skip_to_end_check = false;
            if draw_cols {
                if cul_screenline {
                    wlv.cul_attr = 0;
                    wlv.line_attr = line_attr_save;
                    wlv.line_attr_lowprio = line_attr_lowprio_save;
                }

                debug_assert!(wlv.off == 0);

                if wlv.filler_todo > 0 {
                    let virt_rows_todo = wlv.filler_todo - (wlv.filler_lines - wlv.n_virt_lines);
                    if virt_rows_todo > 0 {
                        let target_row = total_virt_rows - virt_rows_todo;
                        while (virt_line_index as usize) < virt_lines.len() {
                            let vl_ptr = &raw const virt_lines[virt_line_index as usize];
                            let line_rows = decor_virt_line_rows(&mut *wp, &*vl_ptr, 0, None);
                            if target_row < virt_line_start_row + line_rows {
                                has_virt_line = true;
                                virt_line_flags = (*vl_ptr).flags;
                                decor_virt_line_rows(
                                    &mut *wp,
                                    &*vl_ptr,
                                    target_row - virt_line_start_row,
                                    Some(&mut virt_line_skip_cells),
                                );
                                break;
                            }
                            virt_line_start_row += line_rows;
                            virt_line_index += 1;
                        }
                    }
                }

                if has_virt_line && (virt_line_flags & K_VLLEFTCOL) != 0 {
                    // skip columns
                } else if statuscol.draw {
                    let v = crate::memory::ptr_offset_from(ptr, line) as i32;
                    draw_statuscol(&mut *wp, &mut wlv, col_rows, &mut statuscol);
                    if wp.w_redr_statuscol {
                        break;
                    }
                    if draw_text {
                        line = ml_get_buf(wp.buf_mut(), lnum);
                        ptr = line.wrapping_add(v as usize);
                    }
                } else {
                    draw_foldcolumn(&mut *wp, &mut wlv);

                    for sign_idx in 0..wp.w_scwidth {
                        draw_sign(false, &mut *wp, &mut wlv, sign_idx);
                    }

                    draw_lnum_col(wp, &mut wlv);
                }

                win_col_offset = wlv.off;

                if col_rows > 0 {
                    wlv_put_linebuf(wp, &wlv, wlv.off.min(view_width), false, bg_attr, 0);
                    if (wlv.row + 1 - wlv.startrow < col_rows
                        && (statuscol.draw
                            || win_hl_attr(&mut *wp, HLF_LNA) != win_hl_attr(&mut *wp, HLF_N)
                            || win_hl_attr(&mut *wp, HLF_LNB) != win_hl_attr(&mut *wp, HLF_N)))
                        || wlv.filler_todo > 0
                    {
                        wlv.row += 1;
                        if wlv.row == endrow {
                            break;
                        }
                        wlv.filler_todo -= 1;
                        has_virt_line = false;
                        if wlv.filler_todo == 0 && (wp.w_botfill || !draw_text) {
                            break;
                        }
                        wlv.col = 0;
                        wlv.off = 0;
                        continue;
                    } else {
                        break;
                    }
                }

                let gap_decor_attr = if attr_has_line_deco(gap_attr_save) {
                    0
                } else {
                    gap_attr_save
                };
                if !wp.w_briopt_sbr {
                    handle_breakindent(wp, &mut wlv, gap_decor_attr);
                }
                handle_showbreak_and_filler(wp, &mut wlv, gap_decor_attr);
                if wp.w_briopt_sbr {
                    handle_breakindent(wp, &mut wlv, gap_decor_attr);
                }

                wlv.col = wlv.off;
                draw_cols = false;
                if wlv.filler_todo <= 0 {
                    leftcols_width = wlv.off;
                }
                if has_decor && wlv.row == startrow + wlv.filler_lines {
                    decor_redraw_col(
                        &mut *wp,
                        (crate::memory::ptr_offset_from(ptr, line) as Colnr) - 1,
                        wlv.off,
                        true,
                        &mut g_st().decor_state,
                        decor_provider_end_col - 1,
                    );
                }
                if wlv.col >= view_width {
                    wlv.col = view_width;
                    wlv.off = view_width;
                    skip_to_end_check = true;
                }
            }

            if !skip_to_end_check {
                if cul_screenline
                    && wlv.filler_todo <= 0
                    && wlv.vcol >= left_curline_col
                    && wlv.vcol < right_curline_col
                {
                    apply_cursorline_highlight(wp, &mut wlv);
                }

                if g_st().dollar_vcol >= 0 && in_curline && wlv.vcol >= wp.w_virtcol {
                    draw_virt_text(wp, &mut *buf, win_col_offset, &mut wlv.col, wlv.row);
                    wlv_put_linebuf(wp, &wlv, wlv.col, false, bg_attr, 0);
                    if wp.w_onebuf_opt.wo_cuc != 0 {
                        wlv.row = wp.w_cline_row + wp.w_cline_height;
                    } else {
                        wlv.row = view_height;
                    }
                    break;
                }

                let draw_folded = has_fold && wlv.row == startrow + wlv.filler_lines;
                if draw_folded && wlv.n_extra == 0 {
                    folded_attr = win_hl_attr(&mut *wp, HLF_FL);
                    wlv.char_attr = folded_attr;
                    decor_attr = 0;
                }

                let mut extmark_attr: i32 = 0;
                if wlv.filler_todo <= 0 && (area_highlighting || spv.spv_has_spell || extra_check) {
                    if wlv.n_extra == 0 || !wlv.extra_for_extmark {
                        wlv.reset_extra_attr = false;
                    }

                    if has_decor && wlv.n_extra == 0 {
                        if wlv.vcol == wlv.fromcol
                            || (wlv.vcol + 1 == wlv.fromcol
                                && (wlv.n_extra == 0 && utf_ptr2cells(ptr) > 1))
                        {
                            area_active = true;
                        } else if area_active && wlv.vcol == wlv.tocol {
                            area_active = false;
                        }

                        let selected = area_active;
                        if decor_need_recheck {
                            if !may_have_inline_virt {
                                decor_recheck_draw_col(wlv.off, selected, &mut g_st().decor_state);
                            }
                            decor_need_recheck = false;
                        }
                        extmark_attr = decor_redraw_col(
                            &mut *wp,
                            crate::memory::ptr_offset_from(ptr, line) as Colnr,
                            if may_have_inline_virt { -3 } else { wlv.off },
                            selected,
                            &mut g_st().decor_state,
                            decor_provider_end_col - 1,
                        );
                        if may_have_inline_virt {
                            handle_inline_virtual_text(
                                &mut wlv,
                                crate::memory::ptr_offset_from(ptr, line),
                                selected,
                            );
                            if wlv.n_extra > 0 && wlv.virt_inline_hl_mode <= K_HL_MODE_REPLACE {
                                saved_search_attr = search_attr;
                                saved_area_attr = area_attr;
                                saved_decor_attr = decor_attr;
                                saved_search_attr_from_match = search_attr_from_match;
                                search_attr = 0;
                                area_attr = 0;
                                decor_attr = 0;
                                search_attr_from_match = false;
                            }
                        }
                    }

                    let area_attr_p =
                        if wlv.extra_for_extmark && wlv.virt_inline_hl_mode <= K_HL_MODE_REPLACE {
                            &raw mut saved_area_attr
                        } else {
                            &raw mut area_attr
                        };

                    if wlv.vcol == wlv.fromcol
                        || (wlv.vcol + 1 == wlv.fromcol
                            && ((wlv.n_extra == 0 && utf_ptr2cells(ptr) > 1)
                                || (wlv.n_extra > 0
                                    && !wlv.p_extra.is_null()
                                    && utf_ptr2cells(wlv.p_extra) > 1)))
                    {
                        *area_attr_p = vi_attr;
                        area_active = true;
                    } else if *area_attr_p != 0 && wlv.vcol == wlv.tocol {
                        *area_attr_p = 0;
                        area_active = false;
                    }

                    if !has_foldtext && wlv.n_extra == 0 {
                        let v = crate::memory::ptr_offset_from(ptr, line) as i32;
                        search_attr = update_search_hl(
                            wp,
                            lnum,
                            v,
                            &raw mut line,
                            &mut g_st().screen_search_hl,
                            &mut has_match_conc,
                            &mut match_conc,
                            lcs_eol_todo,
                            &mut on_last_col,
                            &mut search_attr_from_match,
                        );
                        ptr = line.wrapping_add(v as usize);

                        if crate::memory::cstr_is_empty(ptr) {
                            has_match_conc = 0;
                        }

                        if (g_st().state_global & MODE_INSERT) != 0
                            && ins_compl_win_active(wp)
                            && (in_curline || ins_compl_lnum_in_range(lnum))
                        {
                            let ins_match_attr = ins_compl_col_range_attr(
                                lnum,
                                crate::memory::ptr_offset_from(ptr, line) as i32,
                            );
                            if ins_match_attr > 0 {
                                search_attr = hl_combine_attr(search_attr, ins_match_attr);
                            }
                        }
                    }

                    if wlv.diff_hlf != 0 {
                        let num_changes = line_changes.changes.len() as i32;
                        if num_changes > 0 && change_index >= 0 && change_index < num_changes - 1 {
                            let next_change = &line_changes.changes[(change_index + 1) as usize];
                            if (crate::memory::ptr_offset_from(ptr, line) as i32)
                                >= next_change.dc_start[line_changes.bufidx as usize]
                            {
                                change_index += 1;
                            }
                        }
                        let mut added = false;
                        if num_changes > 0 && change_index >= 0 && change_index < num_changes {
                            added = diff_change_parse(
                                &line_changes,
                                &line_changes.changes[change_index as usize],
                                &mut change_start,
                                &mut change_end,
                            );
                        }
                        if wlv.diff_hlf == HLF_CHD
                            && (crate::memory::ptr_offset_from(ptr, line) as i32) >= change_start
                            && wlv.n_extra == 0
                        {
                            wlv.diff_hlf = if added { HLF_TXA } else { HLF_TXD };
                        }
                        if (wlv.diff_hlf == HLF_TXD || wlv.diff_hlf == HLF_TXA)
                            && (((crate::memory::ptr_offset_from(ptr, line) as i32) >= change_end
                                && wlv.n_extra == 0)
                                || (wlv.n_extra > 0 && wlv.extra_for_extmark))
                        {
                            wlv.diff_hlf = HLF_CHD;
                        }
                        set_line_attr_for_diff(&mut *wp, &mut wlv);
                    }

                    if area_attr != 0 {
                        char_attr_pri = hl_combine_attr(wlv.line_attr, area_attr);
                        if !g_st().search_global.hl_match {
                            char_attr_pri = hl_combine_attr(search_attr, char_attr_pri);
                        }
                    } else if search_attr != 0 {
                        char_attr_pri = hl_combine_attr(wlv.line_attr, search_attr);
                    } else if wlv.line_attr != 0
                        && ((wlv.fromcol == -10 && wlv.tocol == MAXCOL)
                            || wlv.vcol < wlv.fromcol
                            || wlv.vcol >= wlv.tocol)
                    {
                        char_attr_pri = wlv.line_attr;
                    } else {
                        char_attr_pri = 0;
                    }
                    char_attr_base = hl_combine_attr(folded_attr, decor_attr);
                    wlv.char_attr = hl_combine_attr(char_attr_base, char_attr_pri);
                }

                if draw_folded && has_foldtext && wlv.n_extra == 0 && wlv.col == win_col_offset {
                    let v = crate::memory::ptr_offset_from(ptr, line) as i32;
                    let lnume = lnum + foldinfo.fi_lines - 1;
                    debug_assert!(foldtext_free.is_none());
                    let fdt = get_foldtext(wp, lnum, lnume, foldinfo, &mut fold_vt);
                    wlv.n_extra = fdt.to_bytes().len() as i32;
                    foldtext_free = Some(fdt);
                    wlv.p_extra = foldtext_free.as_ref().unwrap().as_ptr().cast_mut();
                    wlv.sc_extra = 0;
                    wlv.sc_final = 0;

                    line = ml_get_buf(wp.buf_mut(), lnum);
                    ptr = line.wrapping_add(v as usize);
                }

                if draw_folded
                    && wlv.n_extra == 0
                    && wlv.col < view_width
                    && (has_foldtext
                        || (crate::memory::cstr_is_empty(ptr)
                            && (wp.w_onebuf_opt.wo_list == 0 || !lcs_eol_todo || lcs_eol == 0)))
                {
                    wlv.sc_extra = wp.w_p_fcs_chars.fold;
                    wlv.sc_final = 0;
                    wlv.n_extra = view_width - wlv.col;
                    search_attr = 0;
                }

                if draw_folded && wlv.n_extra != 0 && wlv.col >= view_width {
                    wlv.n_extra = 0;
                }

                if wlv.n_extra > 0 {
                    if wlv.sc_extra != 0 || (wlv.n_extra == 1 && wlv.sc_final != 0) {
                        mb_schar = if wlv.n_extra == 1 && wlv.sc_final != 0 {
                            wlv.sc_final
                        } else {
                            wlv.sc_extra
                        };
                        mb_schar_width = schar_get_single_width(mb_schar);
                        wlv.n_extra -= 1;
                    } else {
                        debug_assert!(!wlv.p_extra.is_null());
                        let ci = utf_ptr2_str_char_info(wlv.p_extra);
                        let cli = utf_cluster_info(ci);
                        let mut mb_l = crate::memory::ptr_offset_from(cli.next.ptr, ci.ptr) as i32;
                        mb_schar = schar_from_cluster(ci, cli);
                        mb_schar_width = cli.cells;
                        if mb_l > wlv.n_extra || mb_l == 0 {
                            mb_l = 1;
                        }

                        if wlv.col >= view_width - 1 && mb_schar_width == 2 {
                            mb_l = 1;
                            mb_schar = schar_from_ascii(b'>');
                            mb_schar_width = 1;
                            multi_attr = win_hl_attr(&mut *wp, HLF_AT);

                            if wlv.cul_attr != 0 {
                                multi_attr = if wlv.line_attr_lowprio != 0 {
                                    hl_combine_attr(wlv.cul_attr, multi_attr)
                                } else {
                                    hl_combine_attr(multi_attr, wlv.cul_attr)
                                };
                            }
                        } else {
                            wlv.n_extra -= mb_l;
                            wlv.p_extra = wlv.p_extra.wrapping_add(mb_l as usize);
                        }

                        if wlv.filler_todo <= 0 && wlv.skip_cells > 0 && mb_l > 1 {
                            if wlv.n_extra > 0 {
                                n_extra_next = wlv.n_extra;
                                extra_attr_next = wlv.extra_attr;
                            }
                            wlv.n_extra = 1;
                            wlv.sc_extra = schar_from_ascii(MB_FILLER_CHAR);
                            wlv.sc_final = 0;
                            mb_schar = schar_from_ascii(b' ');
                            mb_schar_width = 1;
                            wlv.n_attr += 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_AT);
                        }
                    }

                    if wlv.n_extra <= 0 {
                        if n_extra_next <= 0 {
                            if search_attr == 0 {
                                search_attr = saved_search_attr;
                                saved_search_attr = 0;
                            }
                            if area_attr == 0 && !crate::memory::cstr_is_empty(ptr) {
                                area_attr = saved_area_attr;
                                saved_area_attr = 0;
                            }
                            if decor_attr == 0 {
                                decor_attr = saved_decor_attr;
                                saved_decor_attr = 0;
                            }
                            if wlv.extra_for_extmark {
                                wlv.reset_extra_attr = true;
                                extra_attr_next = -1;
                            }
                            wlv.extra_for_extmark = false;
                        } else {
                            debug_assert!(wlv.sc_extra != 0 || wlv.sc_final != 0);
                            debug_assert!(!wlv.p_extra.is_null());
                            wlv.sc_extra = 0;
                            wlv.sc_final = 0;
                            wlv.n_extra = n_extra_next;
                            n_extra_next = 0;
                            wlv.reset_extra_attr = true;
                            debug_assert!(extra_attr_next >= 0);
                        }
                    }
                } else if wlv.filler_todo > 0 {
                    mb_schar = schar_from_ascii(b' ');
                    mb_schar_width = 1;
                } else if has_foldtext || (has_fold && wlv.col >= view_width) {
                    mb_schar = 0;
                } else {
                    let mut prev_ptr = ptr;

                    if crate::memory::cstr_is_empty(ptr) {
                        wlv.skip_cells = 0;
                    }

                    let ci = utf_ptr2_str_char_info(ptr);
                    let cli = utf_cluster_info(ci);
                    let mut mb_l = crate::memory::ptr_offset_from(cli.next.ptr, ci.ptr) as i32;
                    let mb_c = ci.chr.value;
                    let mut unprintable_ascii = false;

                    if mb_c < 0 || (mb_l >= 1 && mb_c == 0) || (mb_l > 1 && !vim_isprintc(mb_c)) {
                        let extra_ptr = (&raw mut wlv.extra).cast::<c_char>();
                        transchar_hex(
                            extra_ptr,
                            if mb_c < 0 {
                                i32::from(crate::memory::cstr_first_byte(ptr))
                            } else {
                                mb_c
                            },
                        );
                        if wp.w_onebuf_opt.wo_rl != 0 {
                            rl_mirror_ascii(extra_ptr, ptr::null_mut());
                        }

                        wlv.p_extra = extra_ptr.wrapping_add(1);
                        mb_schar = schar_from_ascii(wlv.extra[0] as u8);
                        mb_schar_width = 1;
                        wlv.n_extra = crate::memory::cstr_len(wlv.p_extra) as i32;
                        wlv.sc_extra = 0;
                        wlv.sc_final = 0;
                        if area_attr == 0 && search_attr == 0 {
                            wlv.n_attr = wlv.n_extra + 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_8);
                            saved_attr2 = wlv.char_attr;
                        }
                    } else if mb_l == 0 {
                        mb_l = 1;
                        mb_schar = 0;
                        mb_schar_width = 1;
                        unprintable_ascii = true;
                    } else {
                        mb_schar = schar_from_cluster(ci, cli);
                        mb_schar_width = cli.cells;
                        unprintable_ascii = mb_c < 0x80 && !vim_isprintc(mb_c);
                    }
                    if wlv.col >= view_width - 1 && mb_schar_width == 2 && !unprintable_ascii {
                        mb_schar = schar_from_ascii(b'>');
                        mb_schar_width = 1;
                        mb_l = 1;
                        multi_attr = win_hl_attr(&mut *wp, HLF_AT);
                        ptr = ptr.wrapping_sub(1);
                        did_decrement_ptr = true;
                    } else if !crate::memory::cstr_is_empty(ptr) {
                        ptr = ptr.wrapping_add((mb_l - 1) as usize);
                    }

                    if wlv.skip_cells > 0 && mb_l > 1 && wlv.n_extra == 0 {
                        wlv.n_extra = 1;
                        wlv.sc_extra = schar_from_ascii(MB_FILLER_CHAR);
                        wlv.sc_final = 0;
                        mb_schar = schar_from_ascii(b' ');
                        mb_schar_width = 1;
                        mb_l = 1;
                        if area_attr == 0 && search_attr == 0 {
                            wlv.n_attr = wlv.n_extra + 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_AT);
                            saved_attr2 = wlv.char_attr;
                        }
                    }
                    ptr = ptr.wrapping_add(1);

                    decor_attr = 0;
                    syntax_attr = 0;
                    if extra_check {
                        let w_s = wp.w_s_ptr();
                        let no_plain_buffer =
                            (w_s.b_p_spo_flags & K_OPT_SPO_FLAG_NOPLAINBUFFER) != 0;
                        let mut can_spell = !no_plain_buffer;

                        let v = crate::memory::ptr_offset_from(ptr, line) as i32;
                        let prev_v = crate::memory::ptr_offset_from(prev_ptr, line);
                        if has_syntax && v > 0 {
                            let save_did_emsg = g_st().did_emsg;
                            g_st().did_emsg = 0;

                            decor_attr = get_syntax_attr(
                                v - 1,
                                if spv.spv_has_spell {
                                    &raw mut can_spell
                                } else {
                                    ptr::null_mut()
                                },
                                false,
                            );

                            if g_st().did_emsg != 0 {
                                w_s.b_syn_error = true;
                                has_syntax = false;
                            } else {
                                g_st().did_emsg = save_did_emsg;
                            }

                            if w_s.b_syn_slow {
                                has_syntax = false;
                            }

                            line = ml_get_buf(wp.buf_mut(), lnum);
                            ptr = line.wrapping_add(v as usize);
                            prev_ptr = line.wrapping_offset(prev_v);

                            syntax_flags = if mb_schar == 0 {
                                0
                            } else {
                                get_syntax_info(&mut syntax_seqnr)
                            };
                        }

                        if has_decor && v > 0 {
                            syntax_attr = decor_attr;
                            decor_attr = hl_combine_attr(decor_attr, extmark_attr);
                            decor_conceal = g_st().decor_state.conceal;
                            can_spell = tristate_to_bool(g_st().decor_state.spell, can_spell);
                        }

                        char_attr_base = hl_combine_attr(folded_attr, decor_attr);
                        wlv.char_attr = hl_combine_attr(char_attr_base, char_attr_pri);

                        let mut v1 = crate::memory::ptr_offset_from(ptr, line) as i32;
                        if spv.spv_has_spell && v1 >= word_end && v1 > cur_checked_col {
                            spell_attr = 0;
                            if mb_schar != 0 && *skipwhite(prev_ptr) != NUL && can_spell {
                                let mut spell_hlf: Hlf = HLF_COUNT;
                                v1 -= mb_l - 1;

                                let p: *mut c_char =
                                    if (crate::memory::ptr_offset_from(prev_ptr, line) as i32)
                                        - nextlinecol
                                        >= 0
                                    {
                                        nextline_ptr.wrapping_add(
                                            ((crate::memory::ptr_offset_from(prev_ptr, line)
                                                as i32)
                                                - nextlinecol)
                                                as usize,
                                        )
                                    } else {
                                        prev_ptr.cast_mut()
                                    };
                                spv.spv_cap_col -=
                                    crate::memory::ptr_offset_from(prev_ptr, line) as i32;
                                let tmplen = spell_check(
                                    wp,
                                    p,
                                    &mut spell_hlf,
                                    Some(&mut spv.spv_cap_col),
                                    spv.spv_unchanged,
                                );
                                debug_assert!(tmplen <= i32::MAX as usize);
                                let len = tmplen as i32;
                                word_end = v1 + len;

                                if spell_hlf != HLF_COUNT
                                    && (g_st().state_global & MODE_INSERT) != 0
                                    && wp.w_cursor.lnum == lnum
                                    && wp.w_cursor.col
                                        >= (crate::memory::ptr_offset_from(prev_ptr, line) as Colnr)
                                    && wp.w_cursor.col < word_end
                                {
                                    spell_hlf = HLF_COUNT;
                                    g_st().spell_redraw_lnum = lnum;
                                }

                                if spell_hlf == HLF_COUNT
                                    && !ptr::eq(p, prev_ptr)
                                    && (crate::memory::ptr_offset_from(p, nextline_ptr) as i32)
                                        + len
                                        > nextline_idx
                                {
                                    spv.spv_checked_lnum = lnum + 1;
                                    spv.spv_checked_col =
                                        (crate::memory::ptr_offset_from(p, nextline_ptr) as i32)
                                            + len
                                            - nextline_idx;
                                }

                                if spell_hlf != HLF_COUNT {
                                    spell_attr = g_st().highlight_attr[spell_hlf as usize];
                                }

                                if spv.spv_cap_col > 0 {
                                    if !ptr::eq(p, prev_ptr)
                                        && (crate::memory::ptr_offset_from(p, nextline_ptr) as i32)
                                            + spv.spv_cap_col
                                            >= nextline_idx
                                    {
                                        spv.spv_capcol_lnum = lnum + 1;
                                        spv.spv_cap_col =
                                            (crate::memory::ptr_offset_from(p, nextline_ptr)
                                                as i32)
                                                + spv.spv_cap_col
                                                - nextline_idx;
                                    } else {
                                        spv.spv_cap_col +=
                                            crate::memory::ptr_offset_from(prev_ptr, line) as i32;
                                    }
                                }
                            }
                        }
                        if spell_attr != 0 {
                            char_attr_base = hl_combine_attr(char_attr_base, spell_attr);
                            wlv.char_attr = hl_combine_attr(char_attr_base, char_attr_pri);
                        }

                        if !wp.buf_mut().terminal.is_null() {
                            wlv.char_attr = hl_combine_attr(
                                if (wlv.vcol as usize) < TERM_ATTRS_MAX {
                                    term_attrs[wlv.vcol as usize]
                                } else {
                                    0
                                },
                                wlv.char_attr,
                            );
                        }

                        gap_attr_save = if has_decor {
                            hl_combine_attr(syntax_attr, g_st().decor_state.current_hl_eol)
                        } else {
                            decor_attr
                        };

                        if wp.w_onebuf_opt.wo_lbr != 0
                            && !wlv.need_lbr
                            && mb_schar != 0
                            && !vim_isbreak(i32::from(crate::memory::cstr_first_byte(ptr)))
                        {
                            wlv.need_lbr = true;
                        }
                        if wp.w_onebuf_opt.wo_lbr != 0
                            && mb_c > 0
                            && mb_c < 128
                            && wlv.need_lbr
                            && vim_isbreak(mb_c)
                            && !vim_isbreak(i32::from(crate::memory::cstr_first_byte(ptr)))
                        {
                            let mb_off = utf_head_off(line, ptr.wrapping_sub(1));
                            let p = ptr.wrapping_sub((mb_off + 1) as usize);

                            let mut csarg = CharsizeArg::new();
                            let cstype =
                                init_charsize_arg_skip_cur_text(&mut csarg, wp, lnum, line);
                            let cs_ci = utf_ptr2_str_char_info(p);
                            let cs_cli = utf_cluster_info(cs_ci);
                            wlv.n_extra = win_charsize(
                                cstype,
                                wlv.vcol,
                                p,
                                cs_ci.chr.value,
                                &mut csarg,
                                cs_cli.cells,
                            )
                            .width
                                - 1;

                            if mb_c != i32::from(TAB) {
                                if on_last_col || attr_has_line_deco(search_attr) {
                                    search_attr = 0;
                                }
                                if has_decor {
                                    decor_attr = gap_attr_save;
                                }
                                if attr_has_line_deco(decor_attr) {
                                    decor_attr = 0;
                                }
                                if attr_has_line_deco(area_attr) {
                                    area_attr = 0;
                                }
                            }

                            if mb_c == i32::from(TAB) && wlv.n_extra + wlv.col > view_width {
                                wlv.n_extra = tabstop_padding(
                                    wlv.vcol,
                                    wp.buf_mut().b_p_ts,
                                    &wp.buf_mut().b_p_vts_array,
                                ) - 1;
                            }
                            wlv.sc_extra =
                                schar_from_ascii(if mb_off > 0 { MB_FILLER_CHAR } else { b' ' });
                            wlv.sc_final = 0;
                            if mb_c < 128 && ascii_iswhite(mb_c as c_char) {
                                if mb_c == i32::from(TAB) {
                                    fix_for_boguscols(&mut wlv);
                                }
                                if wp.w_onebuf_opt.wo_list == 0 {
                                    mb_schar = schar_from_ascii(b' ');
                                    mb_schar_width = 1;
                                    unprintable_ascii = false;
                                }
                            }
                        }

                        if wp.w_onebuf_opt.wo_list != 0 {
                            in_multispace = mb_c == b' ' as i32
                                && (crate::memory::cstr_first_byte(ptr) == b' '
                                    || (prev_ptr > line as *const c_char
                                        && *prev_ptr.wrapping_sub(1) == b' ' as c_char));
                            if !in_multispace {
                                multispace_pos = 0;
                            }
                        }

                        if wp.w_onebuf_opt.wo_list != 0
                            && ((((mb_c == 160 && mb_l == 2) || (mb_c == 0x202f && mb_l == 3))
                                && wp.w_p_lcs_chars.nbsp != 0)
                                || (mb_c == b' ' as i32
                                    && mb_l == 1
                                    && (wp.w_p_lcs_chars.space != 0
                                        || (in_multispace
                                            && !wp.w_p_lcs_chars.multispace.is_empty()))
                                    && (crate::memory::ptr_offset_from(ptr, line) as Colnr)
                                        >= leadcol
                                    && (crate::memory::ptr_offset_from(ptr, line) as Colnr)
                                        <= trailcol))
                        {
                            if in_multispace && !wp.w_p_lcs_chars.multispace.is_empty() {
                                mb_schar = wp.w_p_lcs_chars.multispace.as_slice()[multispace_pos];
                                multispace_pos += 1;
                                if multispace_pos >= wp.w_p_lcs_chars.multispace.len() {
                                    multispace_pos = 0;
                                }
                            } else {
                                mb_schar = if mb_c == b' ' as i32 {
                                    wp.w_p_lcs_chars.space
                                } else {
                                    wp.w_p_lcs_chars.nbsp
                                };
                            }
                            wlv.n_attr = 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_0);
                            saved_attr2 = wlv.char_attr;
                            mb_schar_width = schar_get_single_width(mb_schar);
                        }

                        if mb_c == b' ' as i32
                            && mb_l == 1
                            && ((trailcol != MAXCOL
                                && ptr > (line as *const c_char).wrapping_add(trailcol as usize))
                                || (leadcol != 0
                                    && ptr
                                        < (line as *const c_char).wrapping_add(leadcol as usize)))
                        {
                            if leadcol != 0
                                && in_multispace
                                && ptr < (line as *const c_char).wrapping_add(leadcol as usize)
                                && !wp.w_p_lcs_chars.leadmultispace.is_empty()
                            {
                                mb_schar =
                                    wp.w_p_lcs_chars.leadmultispace.as_slice()[multispace_pos];
                                multispace_pos += 1;
                                if multispace_pos >= wp.w_p_lcs_chars.leadmultispace.len() {
                                    multispace_pos = 0;
                                }
                            } else if ptr > (line as *const c_char).wrapping_add(trailcol as usize)
                                && wp.w_p_lcs_chars.trail != 0
                            {
                                mb_schar = wp.w_p_lcs_chars.trail;
                            } else if ptr < (line as *const c_char).wrapping_add(leadcol as usize)
                                && wp.w_p_lcs_chars.lead != 0
                            {
                                mb_schar = wp.w_p_lcs_chars.lead;
                            } else if leadcol != 0 && wp.w_p_lcs_chars.space != 0 {
                                mb_schar = wp.w_p_lcs_chars.space;
                            }

                            wlv.n_attr = 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_0);
                            saved_attr2 = wlv.char_attr;
                            mb_schar_width = schar_get_single_width(mb_schar);
                        }
                    }

                    if unprintable_ascii {
                        if mb_c == i32::from(TAB)
                            && (wp.w_onebuf_opt.wo_list == 0 || wp.w_p_lcs_chars.tab1 != 0)
                        {
                            let mut tab_len: i32;
                            let mut vcol_adjusted = wlv.vcol;
                            let mut lcs_tab1 = wp.w_p_lcs_chars.tab1;
                            let mut lcs_tab2 = wp.w_p_lcs_chars.tab2;
                            let mut lcs_tab3 = wp.w_p_lcs_chars.tab3;
                            if wp.w_onebuf_opt.wo_list != 0
                                && wp.w_p_lcs_chars.leadtab1 != 0
                                && ptr < (line as *const c_char).wrapping_add(leadcol as usize)
                            {
                                lcs_tab1 = wp.w_p_lcs_chars.leadtab1;
                                lcs_tab2 = wp.w_p_lcs_chars.leadtab2;
                                lcs_tab3 = wp.w_p_lcs_chars.leadtab3;
                            }
                            let sbr = get_showbreak_value(&mut *wp);

                            if !crate::memory::cstr_is_empty(sbr)
                                && wlv.vcol == wlv.vcol_sbr
                                && wp.w_onebuf_opt.wo_wrap != 0
                            {
                                vcol_adjusted = wlv.vcol - mb_charlen(sbr);
                            }
                            tab_len = tabstop_padding(
                                vcol_adjusted,
                                wp.buf_mut().b_p_ts,
                                &wp.buf_mut().b_p_vts_array,
                            ) - 1;

                            if wp.w_onebuf_opt.wo_lbr == 0 || wp.w_onebuf_opt.wo_list == 0 {
                                wlv.n_extra = tab_len;
                            } else {
                                let saved_nextra = wlv.n_extra;

                                if wlv.vcol_off_co > 0 {
                                    tab_len += wlv.vcol_off_co;
                                }
                                if lcs_tab1 != 0 && wlv.old_boguscols > 0 && wlv.n_extra > tab_len {
                                    tab_len += wlv.n_extra - tab_len;
                                }

                                if tab_len > 0 {
                                    let tab2_len = schar_len(lcs_tab2);
                                    let mut len = (tab_len as usize) * tab2_len;
                                    if lcs_tab3 != 0 {
                                        len = len + schar_len(lcs_tab3) - tab2_len;
                                    }
                                    if wlv.n_extra > 0 {
                                        len += (wlv.n_extra - tab_len) as usize;
                                    }
                                    let mut p = get_extra_buf(len + 1);
                                    ptr::write_bytes(p, b' ', len);
                                    *p.wrapping_add(len) = NUL;
                                    wlv.p_extra = p;
                                    for i in 0..tab_len {
                                        if crate::memory::cstr_is_empty(p) {
                                            tab_len = i;
                                            break;
                                        }
                                        let mut lcs = lcs_tab2;
                                        if lcs_tab3 != 0 && i == tab_len - 1 {
                                            lcs = lcs_tab3;
                                        }
                                        let slen = schar_get_adv(&raw mut p, lcs);
                                        wlv.n_extra +=
                                            slen as i32 - if saved_nextra > 0 { 1 } else { 0 };
                                    }

                                    if wlv.vcol_off_co > 0 {
                                        wlv.n_extra -= wlv.vcol_off_co;
                                    }
                                }
                            }

                            {
                                let vc_saved = wlv.vcol_off_co;
                                fix_for_boguscols(&mut wlv);
                                if wlv.n_extra == tab_len + vc_saved
                                    && wp.w_onebuf_opt.wo_list != 0
                                    && wp.w_p_lcs_chars.tab1 != 0
                                {
                                    tab_len += vc_saved;
                                }
                            }

                            if wp.w_onebuf_opt.wo_list != 0 {
                                mb_schar = if wlv.n_extra == 0 && lcs_tab3 != 0 {
                                    lcs_tab3
                                } else {
                                    lcs_tab1
                                };
                                if wp.w_onebuf_opt.wo_lbr != 0
                                    && !crate::memory::cstr_is_empty(wlv.p_extra)
                                {
                                    wlv.sc_extra = 0;
                                } else {
                                    wlv.sc_extra = lcs_tab2;
                                }
                                wlv.sc_final = lcs_tab3;
                                wlv.n_attr = tab_len + 1;
                                wlv.extra_attr = win_hl_attr(&mut *wp, HLF_0);
                                saved_attr2 = wlv.char_attr;
                            } else {
                                wlv.sc_final = 0;
                                wlv.sc_extra = schar_from_ascii(b' ');
                                mb_schar = schar_from_ascii(b' ');
                            }
                            mb_schar_width = schar_get_single_width(mb_schar);
                        } else if mb_schar == 0
                            && (wp.w_onebuf_opt.wo_list != 0
                                || (wlv.fromcol >= 0
                                    && wlv.tocol > wlv.vcol
                                    && g_st().visual_global.mode != CTRL_V_CONST
                                    && wlv.col < view_width))
                            && lcs_eol_todo
                            && lcs_eol != 0
                        {
                            if wlv.diff_hlf == 0 && wlv.line_attr == 0 && wlv.line_attr_lowprio == 0
                            {
                                if !(area_highlighting
                                    && virtual_active(wp)
                                    && wlv.tocol != MAXCOL
                                    && wlv.vcol < wlv.tocol)
                                {
                                    wlv.p_extra = c"".as_ptr().cast_mut();
                                }
                                wlv.n_extra = 0;
                            }
                            if wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.eol > 0 {
                                mb_schar = wp.w_p_lcs_chars.eol;
                            } else {
                                mb_schar = schar_from_ascii(b' ');
                            }
                            lcs_eol_todo = false;
                            ptr = ptr.wrapping_sub(1);
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_AT);
                            wlv.n_attr = 1;
                            mb_schar_width = schar_get_single_width(mb_schar);
                        } else if mb_schar != 0 {
                            let extra_ptr = (&raw mut wlv.extra).cast::<c_char>();
                            xstrlcpy(
                                extra_ptr,
                                transchar_buf(wp.w_buffer.as_ref(), mb_c),
                                wlv.extra.len(),
                            );
                            wlv.p_extra = extra_ptr;
                            if wlv.n_extra == 0 {
                                wlv.n_extra = opt_vars().dy_escape_width - 1;
                            }
                            if (opt_vars().dy_flags & K_OPT_DY_FLAG_UHEX) != 0
                                && wp.w_onebuf_opt.wo_rl != 0
                            {
                                rl_mirror_ascii(wlv.p_extra, ptr::null_mut());
                            }
                            wlv.sc_extra = 0;
                            wlv.sc_final = 0;
                            mb_schar =
                                schar_from_ascii(crate::memory::cstr_first_byte(wlv.p_extra));
                            mb_schar_width = 1;
                            if wp.w_onebuf_opt.wo_lbr != 0 {
                                let p = get_extra_buf((wlv.n_extra as usize) + 1);
                                ptr::write_bytes(p, b' ', wlv.n_extra as usize);
                                let rem_len =
                                    crate::memory::cstr_len(wlv.p_extra).saturating_sub(1);
                                ptr::copy_nonoverlapping(wlv.p_extra.wrapping_add(1), p, rem_len);
                                *p.wrapping_add(wlv.n_extra as usize) = NUL;
                                wlv.p_extra = p;
                            } else {
                                wlv.n_extra = opt_vars().dy_escape_width - 1;
                                wlv.p_extra = wlv.p_extra.wrapping_add(1);
                            }
                            wlv.n_attr = wlv.n_extra + 1;
                            wlv.extra_attr = win_hl_attr(&mut *wp, HLF_8);
                            saved_attr2 = wlv.char_attr;
                        } else if g_st().visual_global.active
                            && (g_st().visual_global.mode == CTRL_V_CONST
                                || g_st().visual_global.mode == b'v' as i32)
                            && virtual_active(wp)
                            && wlv.tocol != MAXCOL
                            && wlv.vcol < wlv.tocol
                            && wlv.col < view_width
                        {
                            mb_schar = schar_from_ascii(b' ');
                            mb_schar_width = 1;
                            ptr = ptr.wrapping_sub(1);
                        }
                    }

                    if wp.w_onebuf_opt.wo_cole > 0
                        && (!ptr::eq(wp, curwin_ptr())
                            || lnum != wp.w_cursor.lnum
                            || conceal_cursor_line(&*wp))
                        && ((syntax_flags & HL_CONCEAL) != 0
                            || has_match_conc > 0
                            || decor_conceal > 0)
                        && !(lnum_in_visual_area
                            && !crate::strings::vim_haschr(wp.w_onebuf_opt.wo_cocu, b'v' as i32))
                    {
                        let syntax_conceal = (syntax_flags & HL_CONCEAL) != 0;
                        wlv.char_attr = conceal_attr;
                        if ((prev_syntax_id != syntax_seqnr && syntax_conceal)
                            || has_match_conc > 1
                            || decor_conceal > 1)
                            && ((syntax_conceal && syn_get_sub_char() != i32::from(NUL))
                                || (has_match_conc != 0 && match_conc != 0)
                                || (decor_conceal != 0 && g_st().decor_state.conceal_char != 0)
                                || wp.w_onebuf_opt.wo_cole == 1)
                            && wp.w_onebuf_opt.wo_cole != 3
                        {
                            if mb_schar_width > 1 {
                                wlv.n_extra += 1;
                            }

                            if has_match_conc != 0 && match_conc != 0 {
                                mb_schar = schar_from_char(match_conc);
                            } else if decor_conceal != 0 && g_st().decor_state.conceal_char != 0 {
                                mb_schar = g_st().decor_state.conceal_char;
                                if g_st().decor_state.conceal_attr != 0 {
                                    wlv.char_attr = g_st().decor_state.conceal_attr;
                                }
                            } else if syntax_conceal && syn_get_sub_char() != i32::from(NUL) {
                                mb_schar = schar_from_char(syn_get_sub_char());
                            } else if wp.w_p_lcs_chars.conceal != 0 {
                                mb_schar = wp.w_p_lcs_chars.conceal;
                            } else {
                                mb_schar = schar_from_ascii(b' ');
                            }

                            let mut sc_buf = [0 as c_char; MAX_SCHAR_SIZE];
                            schar_get(sc_buf.as_mut_ptr(), mb_schar);
                            mb_schar_width = utf_ptr2cells(sc_buf.as_ptr());

                            prev_syntax_id = syntax_seqnr;

                            if wlv.n_extra > 0 {
                                wlv.vcol_off_co += wlv.n_extra;
                            }
                            wlv.vcol += wlv.n_extra;
                            if is_wrapped && wlv.n_extra > 0 {
                                wlv.boguscols += wlv.n_extra;
                                wlv.col += wlv.n_extra;
                            }
                            wlv.n_extra = 0;
                            wlv.n_attr = 0;
                        } else if wlv.skip_cells == 0 {
                            is_concealing = true;
                            wlv.skip_cells = 1;
                        }
                    } else {
                        prev_syntax_id = 0;
                        is_concealing = false;
                    }

                    if wlv.skip_cells > 0 && did_decrement_ptr {
                        ptr = ptr.wrapping_add(1);
                    }
                }

                if !did_wcol
                    && wlv.filler_todo <= 0
                    && in_curline
                    && conceal_cursor_line(&*wp)
                    && (wlv.vcol + wlv.skip_cells >= wp.w_virtcol || mb_schar == 0)
                {
                    wp.w_wcol = wlv.col - wlv.boguscols;
                    wp.w_wcol_conceal_off = wlv.vcol_off_co + wlv.skip_cells;
                    if wlv.vcol + wlv.skip_cells < wp.w_virtcol {
                        wp.w_wcol += wp.w_virtcol - wlv.vcol - wlv.skip_cells;
                    }
                    wp.w_wrow = wlv.row;
                    did_wcol = true;
                    wp.w_valid |= VALID_WCOL | VALID_WROW | VALID_VIRTCOL;
                }

                if wlv.n_attr > 0 && !search_attr_from_match {
                    wlv.char_attr = hl_combine_attr(wlv.char_attr, wlv.extra_attr);
                    if wlv.reset_extra_attr {
                        wlv.reset_extra_attr = false;
                        if extra_attr_next >= 0 {
                            wlv.extra_attr = extra_attr_next;
                            extra_attr_next = -1;
                        } else {
                            wlv.extra_attr = 0;
                            search_attr_from_match = saved_search_attr_from_match;
                        }
                    }
                }

                if lcs_prec_todo != 0
                    && wp.w_onebuf_opt.wo_list != 0
                    && (if wp.w_onebuf_opt.wo_wrap != 0 {
                        wp.w_skipcol > 0 && wlv.row == 0
                    } else {
                        wp.w_leftcol > 0
                    })
                    && wlv.filler_todo <= 0
                    && wlv.skip_cells <= 0
                    && mb_schar != 0
                {
                    lcs_prec_todo = 0;
                    if mb_schar_width > 1 {
                        wlv.sc_extra = schar_from_ascii(MB_FILLER_CHAR);
                        wlv.sc_final = 0;
                        if wlv.n_extra > 0 {
                            debug_assert!(!wlv.p_extra.is_null());
                            n_extra_next = wlv.n_extra;
                            extra_attr_next = wlv.extra_attr;
                            wlv.n_attr = (wlv.n_attr + 1).max(2);
                        } else {
                            wlv.n_attr = 2;
                        }
                        wlv.n_extra = 1;
                        wlv.extra_attr = win_hl_attr(&mut *wp, HLF_AT);
                    }
                    mb_schar = wp.w_p_lcs_chars.prec;
                    mb_schar_width = schar_get_single_width(mb_schar);
                    saved_attr3 = wlv.char_attr;
                    wlv.char_attr = win_hl_attr(&mut *wp, HLF_AT);
                    n_attr3 = 1;
                }

                if mb_schar == 0 && eol_hl_off == 0 {
                    let prevcol_hl_flag = get_prevcol_hl_flag(
                        wp,
                        &mut g_st().screen_search_hl,
                        (crate::memory::ptr_offset_from(ptr, line) as Colnr) - 1,
                    );

                    if lcs_eol_todo
                        && ((area_attr != 0
                            && wlv.vcol == wlv.fromcol
                            && (g_st().visual_global.mode != CTRL_V_CONST
                                || lnum == g_st().visual_global.start.lnum
                                || lnum == curwin_mut().w_cursor.lnum))
                            || prevcol_hl_flag)
                    {
                        let n = if wlv.col >= view_width { -1 } else { 0 };
                        if n != 0 {
                            wlv.off += n;
                            wlv.col += n;
                        } else {
                            g_st().linebuf_char[wlv.off as usize] = schar_from_ascii(b' ');
                        }
                        if area_attr == 0 && !has_fold {
                            get_search_match_hl(
                                wp,
                                &mut g_st().screen_search_hl,
                                crate::memory::ptr_offset_from(ptr, line) as Colnr,
                                &mut wlv.char_attr,
                            );
                        }

                        let eol_attr = if wlv.cul_attr != 0 {
                            hl_combine_attr(wlv.cul_attr, wlv.char_attr)
                        } else {
                            wlv.char_attr
                        };

                        g_st().linebuf_attr[wlv.off as usize] = eol_attr;
                        g_st().linebuf_vcol[wlv.off as usize] = wlv.vcol;
                        wlv.col += 1;
                        wlv.off += 1;
                        wlv.vcol += 1;
                        eol_hl_off = 1;
                    }
                }

                if mb_schar == 0 {
                    wlv.vcol = wlv
                        .vcol
                        .max(start_vcol + wlv.col - win_col_off(&raw mut *wp));
                    wlv.col -= wlv.boguscols;
                    wlv.boguscols = 0;

                    let hlc = vcol_hlc(&wlv);
                    advance_color_col(&mut wlv, hlc);

                    let eol_skip =
                        if eol_hl_off == 0 && (lcs_eol_todo || wp.w_onebuf_opt.wo_list != 0) {
                            1
                        } else {
                            0
                        };

                    if has_decor {
                        decor_redraw_eol(
                            &mut *wp,
                            &mut g_st().decor_state,
                            &mut wlv.line_attr,
                            wlv.col + eol_skip,
                        );
                    }

                    for i in wlv.col..view_width {
                        g_st().linebuf_vcol[(wlv.off + (i - wlv.col)) as usize] =
                            wlv.vcol + (i - wlv.col);
                    }

                    if (wp.w_onebuf_opt.wo_cuc != 0
                        && wp.w_virtcol >= vcol_hlc(&wlv) - eol_hl_off
                        && i64::from(wp.w_virtcol)
                            < i64::from(view_width) * i64::from(wlv.row - startrow + 1)
                                + i64::from(start_vcol)
                        && lnum != wp.w_cursor.lnum)
                        || !wlv.color_cols.is_empty()
                        || wlv.line_attr_lowprio != 0
                        || wlv.line_attr != 0
                        || wlv.diff_hlf != 0
                        || !wp.buf_mut().terminal.is_null()
                    {
                        let mut rightmost_vcol = get_rightmost_vcol(wp, wlv.color_cols);
                        let cuc_attr = win_hl_attr(&mut *wp, HLF_CUC);
                        let mc_attr = win_hl_attr(&mut *wp, HLF_MC);

                        if wlv.diff_hlf == HLF_TXD || wlv.diff_hlf == HLF_TXA {
                            wlv.diff_hlf = HLF_CHD;
                            set_line_attr_for_diff(&mut *wp, &mut wlv);
                        }

                        let diff_attr = if wlv.diff_hlf != 0 {
                            win_hl_attr(&mut *wp, wlv.diff_hlf)
                        } else {
                            0
                        };

                        let base_attr = hl_combine_attr(wlv.line_attr_lowprio, diff_attr);
                        if base_attr != 0 || wlv.line_attr != 0 || !wp.buf_mut().terminal.is_null()
                        {
                            rightmost_vcol = i32::MAX;
                        }

                        while wlv.col < view_width {
                            g_st().linebuf_char[wlv.off as usize] = schar_from_ascii(b' ');

                            let hlc = vcol_hlc(&wlv);
                            advance_color_col(&mut wlv, hlc);

                            let mut col_attr = base_attr;

                            if wp.w_onebuf_opt.wo_cuc != 0
                                && vcol_hlc(&wlv) == wp.w_virtcol
                                && lnum != wp.w_cursor.lnum
                            {
                                col_attr = hl_combine_attr(col_attr, cuc_attr);
                            } else if wlv
                                .color_cols
                                .first()
                                .is_some_and(|&cc| cc == vcol_hlc(&wlv))
                            {
                                col_attr = hl_combine_attr(col_attr, mc_attr);
                            }

                            if !wp.buf_mut().terminal.is_null()
                                && (wlv.vcol as usize) < TERM_ATTRS_MAX
                            {
                                col_attr = hl_combine_attr(col_attr, term_attrs[wlv.vcol as usize]);
                            }

                            col_attr = hl_combine_attr(col_attr, wlv.line_attr);

                            g_st().linebuf_attr[wlv.off as usize] = col_attr;
                            wlv.off += 1;
                            wlv.col += 1;
                            wlv.vcol += 1;

                            if vcol_hlc(&wlv) > rightmost_vcol {
                                break;
                            }
                        }
                    }

                    if !fold_vt.is_empty() {
                        draw_virt_text_item(
                            &mut *buf,
                            win_col_offset,
                            &fold_vt,
                            K_HL_MODE_COMBINE,
                            view_width,
                            0,
                            0,
                            false,
                        );
                    }
                    draw_virt_text(wp, &mut *buf, win_col_offset, &mut wlv.col, wlv.row);
                    wlv_put_linebuf(wp, &wlv, wlv.col, true, bg_attr, SLF_INC_VCOL);
                    wlv.row += 1;

                    if in_curline {
                        curwin_mut().w_cline_row = startrow;
                        curwin_mut().w_cline_height = wlv.row - startrow;
                        curwin_mut().w_cline_folded = has_fold;
                        curwin_mut().w_valid |= VALID_CHEIGHT | VALID_CROW;
                    }

                    break;
                }

                let lcs_ext = get_lcs_ext(wp);
                if lcs_ext != 0
                    && wlv.filler_todo <= 0
                    && wlv.col == view_width - 1
                    && !has_foldtext
                {
                    if has_decor
                        && crate::memory::cstr_is_empty(ptr)
                        && lcs_eol == 0
                        && lcs_eol_todo
                    {
                        decor_redraw_col(
                            &mut *wp,
                            crate::memory::ptr_offset_from(ptr, line) as Colnr,
                            -1,
                            false,
                            &mut g_st().decor_state,
                            decor_provider_end_col - 1,
                        );
                    }
                    if !crate::memory::cstr_is_empty(ptr)
                        || (lcs_eol > 0 && lcs_eol_todo)
                        || (wlv.n_extra > 0
                            && (wlv.sc_extra != 0 || !crate::memory::cstr_is_empty(wlv.p_extra)))
                        || (may_have_inline_virt
                            && has_more_inline_virt(
                                &wlv,
                                crate::memory::ptr_offset_from(ptr, line),
                            ))
                    {
                        mb_schar = lcs_ext;
                        mb_schar_width = schar_get_single_width(mb_schar);
                        wlv.char_attr = win_hl_attr(&mut *wp, HLF_AT);
                    }
                }

                let hlc = vcol_hlc(&wlv);
                advance_color_col(&mut wlv, hlc);

                vcol_save_attr = -1;
                if !lnum_in_visual_area
                    && search_attr == 0
                    && area_attr == 0
                    && wlv.filler_todo <= 0
                {
                    if wp.w_onebuf_opt.wo_cuc != 0
                        && vcol_hlc(&wlv) == wp.w_virtcol
                        && lnum != wp.w_cursor.lnum
                    {
                        vcol_save_attr = wlv.char_attr;
                        wlv.char_attr =
                            hl_combine_attr(win_hl_attr(&mut *wp, HLF_CUC), wlv.char_attr);
                    } else if wlv
                        .color_cols
                        .first()
                        .is_some_and(|&cc| cc == vcol_hlc(&wlv))
                    {
                        vcol_save_attr = wlv.char_attr;
                        wlv.char_attr =
                            hl_combine_attr(win_hl_attr(&mut *wp, HLF_MC), wlv.char_attr);
                    }
                }

                if wlv.filler_todo <= 0 {
                    let mut low = wlv.line_attr_lowprio;
                    let mut high = wlv.char_attr;

                    if wlv.line_attr_lowprio != 0 {
                        let line_ae = syn_attr2entry(wlv.line_attr_lowprio);
                        let char_ae = syn_attr2entry(wlv.char_attr);
                        let mut win_normal_bg = g_st().normal_bg;
                        let mut win_normal_cterm_bg = g_st().cterm_normal_bg_color;

                        if bg_attr != 0 {
                            let norm_ae = syn_attr2entry(bg_attr);
                            win_normal_bg = norm_ae.rgb_bg_color;
                            win_normal_cterm_bg = i32::from(norm_ae.cterm_bg_color);
                        }
                        let char_is_normal_bg = if ui_rgb_attached() {
                            char_ae.rgb_bg_color == win_normal_bg
                        } else {
                            i32::from(char_ae.cterm_bg_color) == win_normal_cterm_bg
                        };

                        if (line_ae.rgb_bg_color >= 0 || line_ae.cterm_bg_color > 0)
                            && char_is_normal_bg
                        {
                            low = wlv.char_attr;
                            high = wlv.line_attr_lowprio;
                        }
                    }
                    wlv.char_attr = hl_combine_attr(low, high);
                }

                if wlv.filler_todo > 0 {
                    // nothing
                } else if wlv.skip_cells <= 0 {
                    g_st().linebuf_char[wlv.off as usize] = mb_schar;
                    if multi_attr != 0 {
                        g_st().linebuf_attr[wlv.off as usize] = multi_attr;
                        multi_attr = 0;
                    } else {
                        g_st().linebuf_attr[wlv.off as usize] = wlv.char_attr;
                    }

                    g_st().linebuf_vcol[wlv.off as usize] = wlv.vcol;

                    if mb_schar_width > 1 {
                        wlv.off += 1;
                        wlv.col += 1;
                        g_st().linebuf_char[wlv.off as usize] = 0;
                        g_st().linebuf_attr[wlv.off as usize] =
                            g_st().linebuf_attr[(wlv.off - 1) as usize];

                        wlv.vcol += 1;
                        g_st().linebuf_vcol[wlv.off as usize] = wlv.vcol;

                        if wlv.tocol == wlv.vcol {
                            wlv.tocol += 1;
                        }
                    }
                    wlv.off += 1;
                    wlv.col += 1;
                } else if wp.w_onebuf_opt.wo_cole > 0 && is_concealing {
                    let concealed_wide = mb_schar_width > 1;

                    wlv.skip_cells -= 1;
                    wlv.vcol_off_co += 1;
                    if concealed_wide {
                        wlv.vcol += 1;
                        wlv.vcol_off_co += 1;
                    }

                    if wlv.n_extra > 0 {
                        wlv.vcol_off_co += wlv.n_extra;
                    }

                    if is_wrapped {
                        if wlv.n_extra > 0 {
                            wlv.vcol += wlv.n_extra;
                            wlv.col += wlv.n_extra;
                            wlv.boguscols += wlv.n_extra;
                            wlv.n_extra = 0;
                            wlv.n_attr = 0;
                        }

                        if concealed_wide {
                            wlv.boguscols += 1;
                            wlv.col += 1;
                        }

                        wlv.boguscols += 1;
                        wlv.col += 1;
                    } else if wlv.n_extra > 0 {
                        wlv.vcol += wlv.n_extra;
                        wlv.n_extra = 0;
                        wlv.n_attr = 0;
                    }
                } else {
                    wlv.skip_cells -= 1;
                }

                if wlv.skipped_cells > 0 {
                    wlv.vcol += wlv.skipped_cells;
                    wlv.skipped_cells = 0;
                }

                if wlv.filler_todo <= 0 {
                    wlv.vcol += 1;
                }

                if vcol_save_attr >= 0 {
                    wlv.char_attr = vcol_save_attr;
                }

                if n_attr3 > 0 {
                    n_attr3 -= 1;
                    if n_attr3 == 0 {
                        wlv.char_attr = saved_attr3;
                    }
                }

                if wlv.n_attr > 0 {
                    wlv.n_attr -= 1;
                    if wlv.n_attr == 0 {
                        wlv.char_attr = saved_attr2;
                    }
                }

                if has_decor && wlv.filler_todo <= 0 && wlv.col >= view_width {
                    if is_wrapped && wlv.n_extra == 0 {
                        decor_redraw_col(
                            &mut *wp,
                            crate::memory::ptr_offset_from(ptr, line) as Colnr,
                            -3,
                            false,
                            &mut g_st().decor_state,
                            decor_provider_end_col - 1,
                        );
                        decor_need_recheck = true;
                    } else if !is_wrapped {
                        decor_recheck_draw_col(-1, true, &mut g_st().decor_state);
                        decor_redraw_col(
                            &mut *wp,
                            MAXCOL,
                            -1,
                            true,
                            &mut g_st().decor_state,
                            decor_provider_end_col - 1,
                        );
                    }
                }
            }

            // end_check:
            if wlv.col >= view_width
                && (!has_foldtext || wlv.filler_todo > 0)
                && (wlv.col <= leftcols_width
                    || !crate::memory::cstr_is_empty(ptr)
                    || wlv.filler_todo > 0
                    || (wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.eol != 0 && lcs_eol_todo)
                    || (wlv.n_extra != 0
                        && (wlv.sc_extra != 0 || !crate::memory::cstr_is_empty(wlv.p_extra)))
                    || (may_have_inline_virt
                        && has_more_inline_virt(&wlv, crate::memory::ptr_offset_from(ptr, line))))
            {
                let grid_width = (*wp.w_grid.target).cols;
                let wrap = is_wrapped
                    && wlv.filler_todo <= 0
                    && lcs_eol_todo
                    && wlv.row != endrow - 1
                    && view_width == grid_width
                    && wp.w_onebuf_opt.wo_rl == 0;

                let mut draw_col = wlv.col - wlv.boguscols;

                for i in draw_col..view_width {
                    g_st().linebuf_vcol[(wlv.off + (i - draw_col)) as usize] = wlv.vcol - 1;
                }

                if wlv.boguscols != 0 && (wlv.line_attr_lowprio != 0 || wlv.line_attr != 0) {
                    let attr = hl_combine_attr(wlv.line_attr_lowprio, wlv.line_attr);
                    while draw_col < view_width {
                        g_st().linebuf_char[wlv.off as usize] = schar_from_char(b' ' as i32);
                        g_st().linebuf_attr[wlv.off as usize] = attr;
                        wlv.off += 1;
                        draw_col += 1;
                    }
                }

                if has_virt_line {
                    let vl = &virt_lines[virt_line_index as usize];
                    let overflow = vl.overflow;
                    let virt_row_scroll = overflow == K_VLOVERFLOW_SCROLL
                        || (overflow == K_VLOVERFLOW_AUTO && wp.w_onebuf_opt.wo_wrap == 0);
                    if virt_row_scroll {
                        virt_line_skip_cells = wp.w_leftcol;
                    }
                    draw_virt_text_item(
                        &mut *buf,
                        if (virt_line_flags & K_VLLEFTCOL) != 0 {
                            0
                        } else {
                            win_col_offset
                        },
                        &vl.line,
                        K_HL_MODE_REPLACE,
                        view_width,
                        0,
                        virt_line_skip_cells,
                        true,
                    );
                } else if wlv.filler_todo <= 0 {
                    draw_virt_text(wp, &mut *buf, win_col_offset, &mut draw_col, wlv.row);
                }

                wlv_put_linebuf(
                    wp,
                    &wlv,
                    draw_col,
                    true,
                    bg_attr,
                    if wrap { SLF_WRAP } else { 0 },
                );
                if wrap {
                    let mut current_row = wlv.row;
                    let mut dummy_col: i32 = 0;
                    let current_grid =
                        &mut *grid_adjust(&mut *grid, &mut current_row, &mut dummy_col);

                    let next_row_off = current_grid.line_offset[(current_row + 1) as usize];
                    current_grid.attrs[next_row_off] = -1;
                }

                wlv.boguscols = 0;
                wlv.vcol_off_co = 0;
                wlv.row += 1;

                if !is_wrapped && wlv.filler_todo <= 0 {
                    break;
                }

                if wlv.col <= leftcols_width {
                    win_draw_end(
                        wp,
                        schar_from_ascii(b'@'),
                        true,
                        wlv.row,
                        wp.w_view_height,
                        HLF_AT,
                    );
                    set_empty_rows(&mut *wp, wlv.row);
                    wlv.row = endrow;
                }

                if wlv.row == endrow {
                    wlv.row += 1;
                    break;
                }

                win_line_start(wp, &mut wlv);
                draw_cols = true;

                lcs_prec_todo = wp.w_p_lcs_chars.prec;
                if wlv.filler_todo <= 0 {
                    wlv.need_showbreak = true;
                }
                if statuscol.draw
                    && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_NUMCOL)
                    && wlv.row > startrow + wlv.filler_lines
                {
                    statuscol.draw = false;
                }
                wlv.filler_todo -= 1;
                has_virt_line = false;
                virt_line_flags = 0;
                if wlv.filler_todo == 0 && (wp.w_botfill || !draw_text) {
                    break;
                }
            }
        }

        drop(fold_vt);
        drop(virt_lines);
        drop(foldtext_free);
        wlv.row
    }
}

fn wlv_put_linebuf(
    wp: &mut WinInfoInfo,
    wlv: &Winlinevars,
    mut endcol: i32,
    clear_end: bool,
    bg_attr: i32,
    mut flags: i32,
) {
    unsafe {
        let grid: *mut GridView = &raw mut wp.w_grid;
        let grid = &mut *grid;

        let mut startcol: i32 = 0;
        let mut clear_width = if clear_end { wp.w_view_width } else { endcol };

        debug_assert!((flags & SLF_RIGHTLEFT) == 0);
        if wp.w_onebuf_opt.wo_rl != 0 {
            linebuf_mirror(
                &mut startcol,
                &mut endcol,
                &mut clear_width,
                wp.w_view_width,
            );
            flags |= SLF_RIGHTLEFT;
        }

        if wlv.row == 0
            && wp.w_skipcol > 0
            && *get_showbreak_value(&mut *wp) == NUL
            && !(wp.w_onebuf_opt.wo_list != 0 && wp.w_p_lcs_chars.prec != 0)
        {
            let mut off: i32 = 0;
            if wp.w_onebuf_opt.wo_nu != 0 && wp.w_onebuf_opt.wo_rnu != 0 {
                while off < wp.w_view_width
                    && ascii_isdigit(schar_get_ascii(g_st().linebuf_char[off as usize]))
                {
                    off += 1;
                }
            }

            for _ in 0..3 {
                if off >= wp.w_view_width {
                    break;
                }
                if off + 1 < wp.w_view_width && g_st().linebuf_char[(off + 1) as usize] == 0 {
                    g_st().linebuf_char[(off + 1) as usize] = schar_from_ascii(b' ');
                }
                g_st().linebuf_char[off as usize] = schar_from_ascii(b'<');
                g_st().linebuf_attr[off as usize] = hl_attr(HLF_AT);
                off += 1;
            }
        }

        let mut row = wlv.row;
        let mut coloff: i32 = 0;
        let g = grid_adjust(&mut *grid, &mut row, &mut coloff);
        grid_put_linebuf(
            &mut *g,
            row,
            coloff,
            startcol,
            endcol,
            clear_width,
            bg_attr,
            0,
            wlv.vcol - 1,
            flags,
        );
    }
}

fn decor_providers_setup(
    rows_to_draw: i32,
    draw_from_line_start: bool,
    lnum: Linenr,
    col: Colnr,
    wp: &mut WinInfoInfo,
) -> i32 {
    {
        let rem_vcols = if wp.w_onebuf_opt.wo_wrap != 0 {
            let width = wp.w_view_width - win_col_off(&raw mut *wp);
            let width2 = width + win_col_off2(wp);

            let first_row_width = if draw_from_line_start { width } else { width2 };
            first_row_width + (rows_to_draw - 1) * width2
        } else {
            wp.w_view_width - win_col_off(&raw mut *wp)
        };

        decor_providers_invoke_line(wp, lnum - 1);
        validate_virtcol(wp);

        invoke_range_next(wp, lnum, col, rem_vcols + 1)
    }
}

fn invoke_range_next(wp: &mut WinInfoInfo, lnum: i32, begin_col: Colnr, mut col_off: Colnr) -> i32 {
    {
        let line = ml_get_buf(wp.buf_mut(), lnum);
        let line_len = ml_get_buf_len(wp.buf_mut(), lnum);
        col_off = col_off.max(1);

        if col_off <= line_len - begin_col {
            let mut end_col = begin_col + col_off;
            end_col += mb_off_next(line, line.wrapping_add(end_col as usize));
            decor_providers_invoke_range(wp, lnum - 1, begin_col, lnum - 1, end_col);
            validate_virtcol(wp);
            end_col
        } else {
            decor_providers_invoke_range(wp, lnum - 1, begin_col, lnum, 0);
            validate_virtcol(wp);
            i32::MAX
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tristate_and_ascii_helpers() {
        assert!(tristate_to_bool(K_TRUE, false));
        assert!(!tristate_to_bool(K_FALSE, true));
        assert!(tristate_to_bool(-1, true));
        assert!(!tristate_to_bool(-1, false));
        assert!(ascii_iswhite(b' ' as c_char));
        assert!(ascii_iswhite(b'\t' as c_char));
        assert!(!ascii_iswhite(b'a' as c_char));
        assert!(ascii_isdigit(b'5' as c_char));
        assert!(!ascii_isdigit(b'x' as c_char));
    }
}
