#![forbid(unsafe_code)]

//! Functions for drawing window lines on the screen (`src/nvim/drawline.c`).
//! This is the middle level, `drawscreen.rs` is the top and `grid.rs` the lower level.

#[cfg(debug_assertions)]
use crate::api::ui::MAX_SCHAR_SIZE;
use crate::api::vim::{HLF_CLN, HLF_CLS, HLF_LNA, HLF_LNB, HLF_N, HLF_SC, SIGN_SHOW_MAX};
use crate::autocmd::HLF_8;
use crate::buffer::{FileBuf, bt_quickfix, buf_meta_total};
use crate::change::TAB;
use crate::charset::{
    K_OPT_DY_FLAG_UHEX, NUL, ascii_isdigit, ascii_iswhite, rl_mirror_ascii, transchar_buf,
    transchar_hex, transstr_buf, vim_isprintc,
};
use crate::cursor::{Colnr, CursorPos, Linenr, VALID_VIRTCOL, get_cursor_rel_lnum, ltoreq};
use crate::decoration::{
    K_VLLEFTCOL, K_VLOVERFLOW_AUTO, K_VLOVERFLOW_SCROLL, K_VPOS_END_OF_LINE,
    K_VPOS_END_OF_LINE_RIGHT_ALIGN, K_VPOS_INLINE, K_VPOS_RIGHT_ALIGN, K_VPOS_WIN_COL,
    K_VTREPEAT_LINEBREAK, NS, SCL_NUM, SignTextAttrs, VirtLines, VirtText, VirtTextChunk,
    conceal_replacement, decor_conceal_char, decor_extconceal_char, decor_has_more_decorations,
    decor_init_draw_col, decor_recheck_draw_col, decor_redraw_col, decor_redraw_eol,
    decor_redraw_line, decor_redraw_signs, decor_virt_line_rows, decor_virt_lines, decor_virt_pos,
    decor_virt_pos_kind, next_virt_text_chunk,
};
use crate::decoration_provider::{decor_providers_invoke_line, decor_providers_invoke_range};
use crate::diff::{
    Diffline, HLF_ADD, HLF_CHD, HLF_TXA, HLF_TXD, diff_change_parse, diff_check_fill,
    diff_check_with_linestatus, diff_find_change,
};
use crate::drawscreen::{
    SIGN_WIDTH, Spellvars, WinExtmark, compute_foldcolumn, conceal_cursor_line,
    conceal_cursor_reveals_line, number_width, win_draw_end,
};
use crate::eval::typval::K_CALLBACK_NONE_CONST;
use crate::ex_getln::{Disptick, Hlf};
use crate::extmark::{K_FALSE, K_TRUE, TriState};
use crate::fold::get_foldtext;
use crate::globals::*;
#[cfg(debug_assertions)]
use crate::grid::schar_get;
use crate::grid::{
    Schar, grid_adjust, grid_put_linebuf, linebuf_mirror, schar_cells, schar_from_ascii,
    schar_from_char, schar_from_cluster, schar_get_adv, schar_get_ascii, schar_get_first_codepoint,
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
    mb_charlen, mb_string2cells, utf_cluster_info, utf_ptr2_str_char_info, utf_ptr2cells,
    utfc_ptr2len,
};
use crate::memline::{gchar_pos, ml_get_buf, ml_get_buf_len};
use crate::r#move::{
    cursor_disp_vcol, set_empty_rows, validate_virtcol, win_col_off, win_col_off2,
};
use crate::option::get_showbreak_value;
use crate::option_vars::*;
use crate::os::fs::MAXPATHL;
use crate::plines::{
    CharInfo, CharSize, CharsizeArg, ConcealSpans, GETVCOL_DISP, StrCharInfo,
    conceal_spans_collect, getvcol, getvvcol, init_charsize_arg, init_charsize_arg_extconceal,
    init_charsize_arg_skip_cur_text, line_has_extconceal, linetabsize_disp_eol, vcol2disp_vcol,
    vim_isbreak, win_charsize,
};
use crate::quickfix::qf_current_entry;
use crate::regexp::{CTRL_V_CONST, MAXCOL};
use crate::search::FORWARD;
use crate::spell::{check_need_cap, spell_cat_line, spell_check, spell_move_to, spell_to_word_end};
use crate::state::{MODE_INSERT, virtual_active};
use crate::statusline::{Foldinfo, Statuscol, build_statuscol_str};
use crate::statusline::{STL_FOLDCOL, STL_SIGNCOL};
use crate::strings::format_to_buf;
use crate::syntax::{
    HL_CONCEAL, get_syntax_attr, get_syntax_info, syn_get_sub_char, syntax_present, syntax_start,
};
use crate::terminal::terminal_get_line_attributes;
use crate::ui::ui_rgb_attached;
use crate::window::WinInfoInfo;

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
    extconceal_off: i32,
    off: i32,
    cul_attr: i32,
    line_attr: i32,
    line_attr_lowprio: i32,
    sign_num_attr: i32,
    prev_num_attr: i32,
    sign_cul_attr: i32,
    fromcol: i32,
    tocol: i32,
    need_showbreak: bool,
    breakindent: i32,
    char_attr: i32,
    n_extra: i32,
    n_attr: i32,
    p_extra_buf: Option<std::ffi::CString>,
    p_extra_off: usize,
    extra_attr: i32,
    sc_extra: Schar,
    sc_final: Schar,
    extra_for_extmark: bool,
    extra: [u8; 11],
    diff_hlf: Hlf,
    n_virt_lines: i32,
    n_virt_below: i32,
    filler_lines: i32,
    filler_todo: i32,
    virt_below_skip: i32,
    filler_lines_skip: i32,
    sattrs: [SignTextAttrs; SIGN_SHOW_MAX],
    virt_inline: Vec<VirtTextChunk>,
    virt_inline_i: usize,
    virt_inline_hl_mode: HlMode,
    reset_extra_attr: bool,
    skip_cells: i32,
    skipped_cells: i32,
    color_cols: &'a [i32],
}

impl Winlinevars<'_> {
    #[inline]
    fn p_extra(&self) -> Option<&std::ffi::CStr> {
        self.p_extra_buf
            .as_deref()
            .map(|s| crate::memory::CStrSubslice::advance(s, self.p_extra_off))
    }

    #[inline]
    fn set_p_extra(&mut self, s: &std::ffi::CStr) {
        self.p_extra_buf = Some(s.to_owned());
        self.p_extra_off = 0;
    }

    #[inline]
    fn set_p_extra_owned(&mut self, s: std::ffi::CString) {
        self.p_extra_buf = Some(s);
        self.p_extra_off = 0;
    }

    #[inline]
    fn advance_p_extra(&mut self, n: usize) {
        self.p_extra_off += n;
    }
}

struct DrawlineState {
    extra_buf: std::cell::RefCell<Vec<u8>>,
    saved_w_virtcol: std::cell::Cell<i32>,
    margin_prev_wp: crate::globals::CloneCell<crate::globals::WinPtr>,
    prev_width1: std::cell::Cell<i32>,
    prev_width2: std::cell::Cell<i32>,
    prev_left_col: std::cell::Cell<i32>,
    prev_right_col: std::cell::Cell<i32>,
    stc_prev_virtnum: std::cell::Cell<i32>,
    stc_prev_wp: crate::globals::CloneCell<crate::globals::WinPtr>,
    stc_prev_lnum: std::cell::Cell<Linenr>,
    stc_prev_tick: std::cell::Cell<Disptick>,
}

impl DrawlineState {
    const fn new() -> Self {
        Self {
            extra_buf: std::cell::RefCell::new(Vec::new()),
            saved_w_virtcol: std::cell::Cell::new(0),
            margin_prev_wp: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            prev_width1: std::cell::Cell::new(0),
            prev_width2: std::cell::Cell::new(0),
            prev_left_col: std::cell::Cell::new(0),
            prev_right_col: std::cell::Cell::new(0),
            stc_prev_virtnum: std::cell::Cell::new(0),
            stc_prev_wp: crate::globals::CloneCell::new(crate::globals::WinPtr::NULL),
            stc_prev_lnum: std::cell::Cell::new(0),
            stc_prev_tick: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static DL_STATE: DrawlineState = const { DrawlineState::new() };
}

#[inline]
fn dl_st<R>(f: impl FnOnce(&DrawlineState) -> R) -> R {
    DL_STATE.with(f)
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

fn with_extra_buf<R>(mut size: usize, f: impl FnOnce(&mut [u8]) -> R) -> R {
    size = size.max(64);
    dl_st(|st| {
        let mut buf = st.extra_buf.borrow_mut();
        if buf.len() < size {
            buf.resize(size, 0);
        }
        f(&mut buf[..size])
    })
}

pub fn drawline_free_all_mem() {
    dl_st(|st| *st.extra_buf.borrow_mut() = Vec::new());
}

/// Get the 'listchars' "extends" characters to use for "wp", or NUL if it
/// shouldn't be used.
fn get_lcs_ext(wp: &WinInfoInfo) -> Schar {
    if wp.w_onebuf_opt.wo_wrap.get() {
        return 0;
    }
    if (wp.w_onebuf_opt.wo_wrap_flags.get() & K_OPT_FLAG_INSECURE) != 0 {
        return schar_from_ascii(b'>');
    }
    if wp.w_onebuf_opt.wo_list.get() {
        wp.w_p_lcs_chars.borrow().ext
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

/// Used when 'cursorlineopt' contains "screenline": compute the margins between which the
/// highlighting is used, for the cursor at displayed vcol `vcol`.
fn margin_columns_win(wp: &WinInfoInfo, vcol: Colnr, left_col: &mut i32, right_col: &mut i32) {
    let cur_col_off = win_col_off(wp);
    let width1 = wp.w_view_width.get() - cur_col_off;
    let width2 = width1 + win_col_off2(wp);

    let wp_ptr = crate::globals::WinPtr::from(wp);
    let cached = dl_st(|st| {
        if st.saved_w_virtcol.get() == vcol
            && st.margin_prev_wp.get() == wp_ptr
            && st.prev_width1.get() == width1
            && st.prev_width2.get() == width2
        {
            Some((st.prev_left_col.get(), st.prev_right_col.get()))
        } else {
            None
        }
    });
    if let Some((l, r)) = cached {
        *left_col = l;
        *right_col = r;
        return;
    }

    *left_col = 0;
    *right_col = width1;

    if vcol >= width1 && width2 > 0 {
        *right_col = width1 + ((vcol - width1) / width2 + 1) * width2;
    }
    if vcol >= width1 && width2 > 0 {
        *left_col = (vcol - width1) / width2 * width2 + width1;
    }

    dl_st(|st| {
        st.prev_left_col.set(*left_col);
        st.prev_right_col.set(*right_col);
        st.margin_prev_wp.set(wp_ptr);
        st.prev_width1.set(width1);
        st.prev_width2.set(width2);
        st.saved_w_virtcol.set(vcol);
    });
}

/// Put a single char from an UTF-8 buffer into a line buffer.
///
/// If `*pp` is a double-width char and only one cell is left, emit a space,
/// and don't advance `*pp`.
fn draw_virt_text(wp: &WinInfoInfo, buf: &FileBuf, col_off: i32, end_col: &mut i32, win_row: i32) {
    let max_col = wp.w_view_width.get();
    let mut right_pos = max_col;
    let do_eol = g_st().decor_state.borrow().eol_col > -1;

    let end = g_st().decor_state.borrow().current_end;

    let mut total_width_of_eol_right_aligned_virt_text: i32 = 0;

    for i in 0..end {
        let slot_idx = g_st().decor_state.borrow().ranges_i[i as usize] as usize;
        let (draw_col, data) = {
            let row = g_st().decor_state.borrow().row;
            let eol_col = g_st().decor_state.borrow().eol_col;
            let is_vp = {
                let ds = g_st().decor_state.borrow();
                let item = ds.slots[slot_idx].range();
                item.start_row == row && decor_virt_pos(item)
            };
            if !is_vp {
                continue;
            }
            let cur_draw_col = g_st().decor_state.borrow().slots[slot_idx].range().draw_col;
            if cur_draw_col == -1 {
                let mut updated = true;
                let pos = decor_virt_pos_kind(g_st().decor_state.borrow().slots[slot_idx].range());
                let mut new_draw_col = cur_draw_col;

                if do_eol && pos == K_VPOS_END_OF_LINE_RIGHT_ALIGN {
                    let mut eol_offset: i32 = 0;
                    if total_width_of_eol_right_aligned_virt_text == 0 {
                        let ds = g_st().decor_state.borrow();
                        for j in i..end {
                            let lookahead_idx = ds.ranges_i[j as usize] as usize;
                            let lookahead_item = ds.slots[lookahead_idx].range();

                            if lookahead_item.start_row != row
                                || !decor_virt_pos(lookahead_item)
                                || lookahead_item.draw_col != -1
                            {
                                continue;
                            }

                            if decor_virt_pos_kind(lookahead_item) == K_VPOS_END_OF_LINE_RIGHT_ALIGN
                                && let Some(w) = lookahead_item.data.with_vt(|vt| vt.width)
                            {
                                total_width_of_eol_right_aligned_virt_text += w + 1;
                            }
                        }

                        total_width_of_eol_right_aligned_virt_text -= 1;

                        if total_width_of_eol_right_aligned_virt_text <= (right_pos - eol_col) {
                            eol_offset =
                                right_pos - total_width_of_eol_right_aligned_virt_text - eol_col;
                        }
                    }

                    new_draw_col = eol_col + eol_offset;
                } else if pos == K_VPOS_RIGHT_ALIGN {
                    right_pos -= g_st().decor_state.borrow().slots[slot_idx]
                        .range()
                        .data
                        .with_vt(|vt| vt.width)
                        .unwrap();
                    new_draw_col = right_pos;
                } else if pos == K_VPOS_END_OF_LINE && do_eol {
                    new_draw_col = eol_col;
                } else if pos == K_VPOS_WIN_COL {
                    new_draw_col = (col_off
                        + g_st().decor_state.borrow().slots[slot_idx]
                            .range()
                            .data
                            .with_vt(|vt| vt.col)
                            .unwrap())
                    .max(0);
                } else {
                    updated = false;
                }
                if updated && (new_draw_col < 0 || new_draw_col >= wp.w_view_width.get()) {
                    new_draw_col = i32::MIN;
                }
                g_st().decor_state.borrow_mut().slots[slot_idx]
                    .range_mut()
                    .draw_col = new_draw_col;
            }
            let ds = g_st().decor_state.borrow();
            let item = ds.slots[slot_idx].range();
            (item.draw_col, item.data.clone())
        };
        if draw_col < 0 {
            continue;
        }
        if data.is_ui() {
            let ui = data.ui();
            let m = WinExtmark {
                ns_id: ui.ns_id as NS,
                mark_id: u64::from(ui.mark_id),
                win_row,
                win_col: draw_col,
            };
            g_st().win_extmark_arr.borrow_mut().push(m);
        }
        let repeat_linebreak = data
            .with_vt(|vt| {
                let vcol = draw_col - col_off;
                let col = draw_virt_text_item(
                    buf,
                    draw_col,
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
                    g_st().decor_state.borrow_mut().eol_col = col + 1;
                }
                *end_col = (*end_col).max(col);
                (vt.flags & K_VTREPEAT_LINEBREAK) != 0
            })
            .unwrap_or(false);
        if !repeat_linebreak {
            g_st().decor_state.borrow_mut().slots[slot_idx]
                .range_mut()
                .draw_col = i32::MIN;
        }
    }
}

fn draw_virt_text_item(
    buf: &FileBuf,
    mut col: i32,
    vt: &[VirtTextChunk],
    hl_mode: HlMode,
    max_col: i32,
    mut vcol: i32,
    mut skip_cells: i32,
    mut eol_hl: bool,
) -> i32 {
    let mut virt_attr: i32 = 0;
    let mut virt_pos: usize = 0;
    eol_hl &= vt
        .last()
        .and_then(|c| c.text.as_ref())
        .is_some_and(|s| s.as_bytes().is_empty());

    let mut ci = StrCharInfo {
        ptr: b"",
        chr: CharInfo { value: -1, len: 0 },
    };

    while col < max_col {
        debug_assert!(skip_cells >= 0);
        while ci.is_empty() {
            if virt_pos >= vt.len() {
                if eol_hl {
                    ci = utf_ptr2_str_char_info(c" ");
                } else {
                    break;
                }
            } else {
                virt_attr = 0;
                let Some(next) = next_virt_text_chunk(vt, &mut virt_pos, &mut virt_attr) else {
                    break;
                };
                ci = utf_ptr2_str_char_info(next);
            }
        }
        if ci.is_empty() {
            break;
        }

        let cli = utf_cluster_info(ci);
        let mut draw_cells = cli.cells;
        if ci.chr.value == i32::from(TAB) {
            draw_cells = tabstop_padding(vcol, buf.b_p_ts.get(), &buf.b_p_vts_array.borrow());
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
            hl_combine_attr(g_st().linebuf_attr.borrow()[col as usize], virt_attr)
        } else if hl_mode == K_HL_MODE_BLEND {
            through = schar_invisible(sc);
            hl_blend_attrs(
                g_st().linebuf_attr.borrow()[col as usize],
                virt_attr,
                &mut through,
            )
        } else {
            virt_attr
        };

        if !through && g_st().linebuf_char.borrow()[col as usize] == 0 {
            debug_assert!(col > 0);
            g_st().linebuf_char.borrow_mut()[(col - 1) as usize] = schar_from_ascii(b' ');
            g_st().linebuf_char.borrow_mut()[col as usize] = schar_from_ascii(b' ');
        }

        if !through {
            if sc == schar_from_ascii(b' ') {
                for c in 0..draw_cells {
                    g_st().linebuf_char.borrow_mut()[(col + c) as usize] = schar_from_ascii(b' ');
                }
            } else {
                debug_assert!(draw_cells <= 2);
                g_st().linebuf_char.borrow_mut()[col as usize] = sc;
                if draw_cells == 2 {
                    g_st().linebuf_char.borrow_mut()[(col + 1) as usize] = 0;
                }
            }
            if draw_cells < maxcells
                && g_st().linebuf_char.borrow()[(col + draw_cells) as usize] == 0
            {
                g_st().linebuf_char.borrow_mut()[(col + draw_cells) as usize] =
                    schar_from_ascii(b' ');
            }
        }
        for _ in 0..draw_cells {
            g_st().linebuf_attr.borrow_mut()[col as usize] = attr;
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

fn draw_col_buf(
    wp: &WinInfoInfo,
    wlv: &mut Winlinevars,
    text_buf: &[u8],
    len: usize,
    attr: i32,
    mut fold_vcol: Option<&[Colnr]>,
    inc_vcol: bool,
) {
    let text_cstr = crate::memory::bytes_as_cstr(text_buf);
    {
        let mut ci = utf_ptr2_str_char_info(text_cstr);
        while ci.offset_from(text_cstr) < len && wlv.off < wp.w_view_width.get() {
            let cli = utf_cluster_info(ci);
            let maxcells = wp.w_view_width.get() - wlv.off;
            debug_assert!(cli.cells <= 2 && ci.chr.value >= 0x20);
            let mut cells = cli.cells;
            if cli.cells > maxcells {
                g_st().linebuf_char.borrow_mut()[wlv.off as usize] = schar_from_ascii(b' ');
                cells = 1;
            } else {
                g_st().linebuf_char.borrow_mut()[wlv.off as usize] = schar_from_cluster(ci, cli);
                if cells == 2 {
                    g_st().linebuf_char.borrow_mut()[(wlv.off + 1) as usize] = 0;
                }
            }

            let mut myattr = attr;
            if inc_vcol {
                advance_color_col(wlv, wlv.vcol);
                if wlv.color_cols.first().is_some_and(|&cc| cc == wlv.vcol) {
                    myattr = hl_combine_attr(win_hl_attr(wp, HLF_MC), myattr);
                }
            }
            for _ in 0..cells {
                g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = myattr;
                g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = if inc_vcol {
                    let v = wlv.vcol;
                    wlv.vcol += 1;
                    v
                } else if let Some(fv) = fold_vcol.as_mut() {
                    let v = fv.first().copied().unwrap_or(-1);
                    if !fv.is_empty() {
                        *fv = &fv[1..];
                    }
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
        g_st().linebuf_char.borrow_mut()[wlv.off as usize] = fillchar;
        g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = attr;
        wlv.off += 1;
    }
}

/// Return true if CursorLineSign / CursorLineFold highlight is to be used.
/// This depends only on 'cursorline', not on 'cursorlineopt'.
pub fn use_cursor_line_highlight(wp: &WinInfoInfo, lnum: Linenr) -> bool {
    wp.w_onebuf_opt.wo_cul.get() && lnum == wp.w_cursorline.get()
}

/// Setup for drawing the 'foldcolumn', if there is one.
fn draw_foldcolumn(wp: &WinInfoInfo, wlv: &mut Winlinevars) {
    let fdc = compute_foldcolumn(wp, 0);
    if fdc > 0 {
        let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
            HLF_CLF
        } else {
            HLF_FC
        };
        let attr = win_hl_attr(wp, hlf);
        let is_virt = wlv.filler_todo > 0;
        fill_foldcolumn(
            wp,
            wlv.foldinfo,
            wlv.lnum,
            attr,
            fdc,
            is_virt,
            Some(&mut wlv.off),
            None,
        );
    }
}

#[inline]
fn foldcolumn_sep_char(first_level: i32, i: i32, wp: &WinInfoInfo) -> Schar {
    if first_level == 1 {
        wp.w_p_fcs_chars.get().foldsep
    } else if wp.w_p_fcs_chars.get().foldinner != 0 {
        wp.w_p_fcs_chars.get().foldinner
    } else if first_level + i <= 9 {
        schar_from_ascii(b'0' + (first_level + i) as u8)
    } else {
        schar_from_ascii(b'>')
    }
}

/// Draw the foldcolumn or fill "out_buffer". Assume monocell characters.
pub fn fill_foldcolumn(
    wp: &WinInfoInfo,
    foldinfo: Foldinfo,
    lnum: Linenr,
    attr: i32,
    fdc: i32,
    is_virt: bool,
    mut wlv_off: Option<&mut i32>,
    mut out: Option<(&mut [Colnr], &mut [Schar])>,
) {
    let closed = foldinfo.fi_level != 0 && foldinfo.fi_lines > 0;
    let level = foldinfo.fi_level;

    let first_level = (level - fdc - i32::from(closed) + 1).max(1);
    let closedcol = fdc.min(level);

    for i in 0..fdc {
        let mut symbol: Schar;
        if i >= level {
            symbol = schar_from_ascii(b' ');
        } else if i == closedcol - 1 && closed {
            symbol = wp.w_p_fcs_chars.get().foldclosed;
        } else if foldinfo.fi_lnum == lnum && first_level + i >= foldinfo.fi_low_level {
            symbol = wp.w_p_fcs_chars.get().foldopen;
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
        if let Some((ref mut out_vcol, ref mut out_buffer)) = out {
            out_vcol[i as usize] = vcol;
            out_buffer[i as usize] = symbol;
        } else if let Some(ref mut wlv_off) = wlv_off {
            let off = (**wlv_off) as usize;
            g_st().linebuf_vcol.borrow_mut()[off] = vcol;
            g_st().linebuf_attr.borrow_mut()[off] = attr;
            g_st().linebuf_char.borrow_mut()[off] = symbol;

            **wlv_off += 1;
        }
    }
}

fn draw_sign(nrcol: bool, wp: &WinInfoInfo, wlv: &mut Winlinevars, sign_idx: i32) {
    let sattr = wlv.sattrs[sign_idx as usize];
    let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
        HLF_CLS
    } else {
        HLF_SC
    };
    let scl_attr = win_hl_attr(wp, hlf);

    if sattr.text[0] != 0 && wlv.row == wlv.startrow + wlv.filler_lines && wlv.filler_todo <= 0 {
        let fill = if nrcol {
            number_width(wp) + 1
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
        g_st().linebuf_char.borrow_mut()[sign_pos as usize] = sattr.text[0];
        g_st().linebuf_char.borrow_mut()[(sign_pos + 1) as usize] = sattr.text[1];
    } else {
        debug_assert!(!nrcol);
        draw_col_fill(wlv, schar_from_ascii(b' '), SIGN_WIDTH, scl_attr);
    }
}

#[inline]
fn get_line_number_str(wp: &WinInfoInfo, lnum: Linenr, buf: &mut [u8]) {
    let mut num: Linenr;
    let mut fmt = c"%*d ";

    if wp.w_onebuf_opt.wo_nu.get() && !wp.w_onebuf_opt.wo_rnu.get() {
        num = lnum;
    } else {
        num = get_cursor_rel_lnum(wp, lnum).abs();
        if num == 0 && wp.w_onebuf_opt.wo_nu.get() && wp.w_onebuf_opt.wo_rnu.get() {
            num = lnum;
            fmt = c"%-*d ";
        }
    }

    format_to_buf!(buf, fmt, number_width(wp), num);
}

fn use_cursor_line_nr(wp: &WinInfoInfo, wlv: &Winlinevars) -> bool {
    wp.w_onebuf_opt.wo_cul.get()
        && wlv.lnum == wp.w_cursorline.get()
        && (wp.w_p_culopt_flags.get() & K_OPT_CULOPT_FLAG_NUMBER) != 0
        && (wlv.row == wlv.startrow + wlv.filler_lines
            || (wlv.row > wlv.startrow + wlv.filler_lines
                && (wp.w_p_culopt_flags.get() & K_OPT_CULOPT_FLAG_LINE) != 0))
}

fn get_line_number_attr(wp: &WinInfoInfo, wlv: &mut Winlinevars) -> i32 {
    let mut numhl_attr = wlv.sign_num_attr;

    if (wlv.n_virt_lines - wlv.filler_todo) < wlv.n_virt_below {
        if wlv.prev_num_attr == -1 {
            let wp_buf = wp.w_buffer.get();
            decor_redraw_signs(
                wp,
                &wp_buf,
                wlv.lnum - 2,
                None,
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
        return hl_combine_attr(win_hl_attr(wp, HLF_CLN), numhl_attr);
    }

    if wp.w_onebuf_opt.wo_rnu.get() {
        if wlv.lnum < wp.w_cursor.get().lnum {
            return hl_combine_attr(win_hl_attr(wp, HLF_LNA), numhl_attr);
        }
        if wlv.lnum > wp.w_cursor.get().lnum {
            return hl_combine_attr(win_hl_attr(wp, HLF_LNB), numhl_attr);
        }
    }

    hl_combine_attr(win_hl_attr(wp, HLF_N), numhl_attr)
}

fn draw_lnum_col(wp: &WinInfoInfo, wlv: &mut Winlinevars) {
    let has_cpo_n = crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_NUMCOL);

    if (wp.w_onebuf_opt.wo_nu.get() || wp.w_onebuf_opt.wo_rnu.get())
        && (wlv.row == wlv.startrow + wlv.filler_lines || !has_cpo_n)
        && !((has_cpo_n && !wp.w_onebuf_opt.wo_bri.get())
            && wp.w_skipcol.get() > 0
            && wlv.lnum == wp.w_topline.get())
    {
        if wp.w_minscwidth.get() == SCL_NUM
            && wlv.sattrs[0].text[0] != 0
            && wlv.row == wlv.startrow + wlv.filler_lines
            && wlv.filler_todo <= 0
        {
            draw_sign(true, wp, wlv, 0);
        } else {
            let width = number_width(wp) + 1;
            let attr = get_line_number_attr(wp, wlv);
            if wlv.row == wlv.startrow + wlv.filler_lines
                && (wp.w_skipcol.get() == 0
                    || wlv.row > 0
                    || (wp.w_onebuf_opt.wo_nu.get() && wp.w_onebuf_opt.wo_rnu.get()))
            {
                let mut buf = [0u8; 32];
                get_line_number_str(wp, wlv.lnum, &mut buf);
                if wp.w_skipcol.get() > 0 && wlv.startrow == 0 {
                    for b in &mut buf {
                        if *b == b' ' {
                            *b = b'-';
                        } else {
                            break;
                        }
                    }
                }
                if wp.w_onebuf_opt.wo_rl.get() {
                    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    let start = buf[..len]
                        .iter()
                        .position(|&b| b != b' ' && b != b'\t')
                        .unwrap_or(len);
                    let end = start
                        + buf[start..len]
                            .iter()
                            .position(|&b| b == b' ' || b == b'\t')
                            .unwrap_or(len - start);
                    rl_mirror_ascii(&mut buf[start..end]);
                }
                draw_col_buf(wp, wlv, &buf, width as usize, attr, None, false);
            } else {
                draw_col_fill(wlv, schar_from_ascii(b' '), width, attr);
            }
        }
    }
}

fn draw_statuscol(wp: &WinInfoInfo, wlv: &mut Winlinevars, col_rows: i32, stcp: &mut Statuscol) {
    let lnum = wlv.lnum - i32::from((wlv.n_virt_lines - wlv.filler_todo) < wlv.n_virt_below);
    let wp_ptr = crate::globals::WinPtr::from(wp);
    let tick = display_tick_val();

    let reset = dl_st(|st| {
        st.stc_prev_wp.get() != wp_ptr
            || st.stc_prev_tick.get() != tick
            || lnum != st.stc_prev_lnum.get()
    });
    let mut reset_virt = if lnum == wlv.lnum {
        wlv.filler_lines_skip
    } else {
        wlv.virt_below_skip
    };
    if reset && lnum < wp.w_topline.get() {
        let mut virt_below_prev: i32 = 0;
        let virt_lines_prev =
            decor_virt_lines(wp, lnum - 1, lnum, Some(&mut virt_below_prev), None, true);
        let diff_fill_prev = diff_check_fill(wp, lnum - 1);
        reset_virt += virt_lines_prev + diff_fill_prev - virt_below_prev;
    }
    let stc_virtnum = dl_st(|st| {
        let v = (if reset {
            -reset_virt
        } else {
            st.stc_prev_virtnum.get()
        }) - i32::from(wlv.filler_todo > 0);
        st.stc_prev_virtnum.set(v);
        st.stc_prev_tick.set(tick);
        st.stc_prev_lnum.set(lnum);
        st.stc_prev_wp.set(wp_ptr);
        v
    });
    let virtnum = if wlv.filler_todo > 0 {
        stc_virtnum
    } else {
        wlv.row - wlv.startrow - wlv.filler_lines
    };
    let relnum: Linenr = if virtnum == -reset_virt - 1 || virtnum == 0 {
        get_cursor_rel_lnum(wp, lnum).abs()
    } else {
        -1
    };

    let mut buf = [0u8; MAXPATHL];
    if wp.w_statuscol_line_count.get() != wp.w_nrwidth_line_count.get() {
        wp.w_statuscol_line_count.set(wp.w_nrwidth_line_count.get());
        let width = build_statuscol_str(
            wp,
            wp.w_nrwidth_line_count.get(),
            wp.w_nrwidth_line_count.get(),
            0,
            &mut buf,
            stcp,
        );
        if width > stcp.width {
            let addwidth = (width - stcp.width).min(MAX_STCWIDTH - stcp.width);
            wp.w_nrwidth.set(wp.w_nrwidth.get() + (addwidth));
            wp.w_nrwidth_width.set(wp.w_nrwidth.get());
            if col_rows > 0 {
                wp.w_redr_statuscol.set(true);
                dl_st(|st| st.stc_prev_lnum.set(0));
                return;
            }
            stcp.width += addwidth;
            wp.w_valid.set(wp.w_valid.get() & (!VALID_WCOL));
        }
    }

    let width = build_statuscol_str(wp, lnum, relnum, virtnum, &mut buf, stcp);
    if wp.w_onebuf_opt.wo_stc.is_empty() || (width > stcp.width && stcp.width < MAX_STCWIDTH) {
        if wp.w_onebuf_opt.wo_stc.is_empty() {
            wp.w_nrwidth_line_count.set(0);
            let nw = i32::from(wp.w_onebuf_opt.wo_nu.get() || wp.w_onebuf_opt.wo_rnu.get())
                * number_width(wp);
            wp.w_nrwidth.set(nw);
        } else {
            wp.w_nrwidth
                .set(wp.w_nrwidth.get() + ((width - stcp.width).min(MAX_STCWIDTH - stcp.width)));
            wp.w_nrwidth_width.set(wp.w_nrwidth.get());
        }
        wp.w_redr_statuscol.set(true);
        dl_st(|st| st.stc_prev_lnum.set(0));
        return;
    }

    let mut p_off = 0usize;
    let mut transbuf = [0u8; MAXPATHL];
    let mut fold_vcol: Option<&[Colnr]> = None;
    let len = crate::memory::bytes_until_nul(&buf).len();
    let hlf = if use_cursor_line_highlight(wp, wlv.lnum) {
        HLF_CLS
    } else {
        HLF_SC
    };
    let scl_attr = win_hl_attr(wp, hlf);
    let num_attr = get_line_number_attr(wp, wlv);
    let mut cur_attr = num_attr;

    for sp in &stcp.hlrec {
        let sp_off = sp.start_off.unwrap_or(p_off);
        let textlen = sp_off.saturating_sub(p_off).min(len.saturating_sub(p_off));
        let translen = transstr_buf(&buf[p_off..p_off + textlen], &mut transbuf, true);
        draw_col_buf(wp, wlv, &transbuf, translen, cur_attr, fold_vcol, false);
        let attr = if sp.item == i32::from(STL_SIGNCOL) {
            scl_attr
        } else if sp.item == i32::from(STL_FOLDCOL) {
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
        fold_vcol = (sp.item == i32::from(STL_FOLDCOL)).then_some(&stcp.fold_vcol);
        p_off += textlen;
    }
    let rem_len = len.saturating_sub(p_off);
    let translen = transstr_buf(&buf[p_off..p_off + rem_len], &mut transbuf, true);
    draw_col_buf(wp, wlv, &transbuf, translen, cur_attr, fold_vcol, false);
    draw_col_fill(wlv, schar_from_ascii(b' '), stcp.width - width, cur_attr);
}

fn attr_has_line_deco(attr: i32) -> bool {
    let ae = syn_attr2entry(attr);
    let mask: i32 = HL_UNDERLINE_MASK | HL_STRIKETHROUGH | HL_OVERLINE;
    (ae.rgb_ae_attr & mask) != 0 || (ae.cterm_ae_attr & mask) != 0
}

fn handle_breakindent(wp: &WinInfoInfo, wlv: &mut Winlinevars, gap_decor_attr: i32) {
    if wp.w_onebuf_opt.wo_bri.get()
        && (wlv.row > wlv.startrow + wlv.filler_lines || wlv.need_showbreak)
    {
        let mut attr = gap_decor_attr;
        if wlv.diff_hlf != 0 {
            attr = hl_combine_attr(attr, win_hl_attr(wp, wlv.diff_hlf));
        }
        let mut num = if wlv.breakindent < 0 {
            wlv.breakindent = get_breakindent_win(
                wp,
                &crate::memline::ml_get_buf_slice(&wp.buf_mut(), wlv.lnum),
            );
            wlv.breakindent
        } else {
            wlv.breakindent
        };
        if wlv.row == wlv.startrow {
            num -= win_col_off2(wp);
            if wlv.n_extra < 0 {
                num = 0;
            }
        }

        let vcol_before = wlv.vcol;

        for _ in 0..num {
            g_st().linebuf_char.borrow_mut()[wlv.off as usize] = schar_from_ascii(b' ');

            advance_color_col(wlv, wlv.vcol);
            let mut myattr = attr;
            if wlv.color_cols.first().is_some_and(|&cc| cc == wlv.vcol) {
                myattr = hl_combine_attr(win_hl_attr(wp, HLF_MC), myattr);
            }
            g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = myattr;
            g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = wlv.vcol;
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

    if wp.w_skipcol.get() > 0
        && wlv.startrow == 0
        && wp.w_onebuf_opt.wo_wrap.get()
        && wp.w_briopt_sbr.get()
    {
        wlv.need_showbreak = false;
    }
}

fn handle_showbreak_and_filler(wp: &WinInfoInfo, wlv: &mut Winlinevars, gap_decor_attr: i32) {
    let remaining = wp.w_view_width.get() - wlv.off;
    if wlv.filler_todo > wlv.filler_lines - wlv.n_virt_lines {
        draw_col_fill(wlv, schar_from_ascii(b' '), remaining, 0);
    } else if wlv.filler_todo > 0 {
        let c = wp.w_p_fcs_chars.get().diff;
        draw_col_fill(wlv, c, remaining, win_hl_attr(wp, HLF_DED));
    }

    let sbr = get_showbreak_value(wp);
    if !sbr.is_empty() && wlv.need_showbreak {
        let mut attr = hl_combine_attr(wlv.cul_attr, gap_decor_attr);
        attr = hl_combine_attr(attr, win_hl_attr(wp, HLF_AT));
        let vcol_before = wlv.vcol;
        draw_col_buf(
            wp,
            wlv,
            sbr.to_bytes_with_nul(),
            sbr.to_bytes().len(),
            attr,
            None,
            true,
        );

        if wlv.fromcol >= vcol_before && wlv.fromcol < wlv.vcol {
            wlv.fromcol = wlv.vcol;
        }

        if wlv.tocol == vcol_before {
            wlv.tocol = wlv.vcol;
        }
    }

    if wp.w_skipcol.get() == 0
        || wlv.startrow > 0
        || !wp.w_onebuf_opt.wo_wrap.get()
        || !wp.w_briopt_sbr.get()
    {
        wlv.need_showbreak = false;
    }
}

fn apply_cursorline_highlight(wp: &WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.cul_attr = win_hl_attr(wp, HLF_CUL);
    let ae = syn_attr2entry(wlv.cul_attr);
    if ae.rgb_fg_color == -1 && ae.cterm_fg_color == 0 {
        wlv.line_attr_lowprio = wlv.cul_attr;
    } else if (g_st().state_global.get() & MODE_INSERT) == 0
        && bt_quickfix(Some(&wp.buf_mut()))
        && qf_current_entry(wp) == wlv.lnum
    {
        wlv.line_attr = hl_combine_attr(wlv.cul_attr, wlv.line_attr);
    } else {
        wlv.line_attr = wlv.cul_attr;
    }
}

fn set_line_attr_for_diff(wp: &WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.line_attr = win_hl_attr(wp, wlv.diff_hlf);
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
    if wlv.virt_inline_i < wlv.virt_inline.len() {
        return true;
    }

    let count = g_st().decor_state.borrow().ranges_i.len() as i32;
    let cur_end = g_st().decor_state.borrow().current_end;
    let fut_beg = g_st().decor_state.borrow().future_begin;

    let beg_pos = [0, fut_beg];
    let end_pos = [cur_end, count];

    let ds = g_st().decor_state.borrow();
    for pos_i in 0..2 {
        for i in beg_pos[pos_i]..end_pos[pos_i] {
            let slot_idx = ds.ranges_i[i as usize] as usize;
            let item = ds.slots[slot_idx].range();
            if item.start_row != ds.row
                || !item.data.is_vt()
                || item
                    .data
                    .with_vt(|vt| vt.pos != K_VPOS_INLINE || vt.width == 0)
                    .unwrap_or(true)
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

fn handle_inline_virtual_text(wlv: &mut Winlinevars, v: isize, selected: bool) {
    while wlv.n_extra == 0 {
        if wlv.virt_inline_i >= wlv.virt_inline.len() {
            wlv.virt_inline.clear();
            wlv.virt_inline_i = 0;
            let mut ds = g_st().decor_state.borrow_mut();
            let end = ds.current_end;
            let row = ds.row;

            for i in 0..end {
                let slot_idx = ds.ranges_i[i as usize] as usize;
                let item = ds.slots[slot_idx].range_mut();
                if item.draw_col == -3 {
                    decor_init_draw_col(wlv.off, selected, item);
                }
                if item.start_row != row
                    || !item.data.is_vt()
                    || item
                        .data
                        .with_vt(|vt| vt.pos != K_VPOS_INLINE || vt.width == 0)
                        .unwrap_or(true)
                {
                    continue;
                }
                if item.draw_col >= -1 && isize::try_from(item.start_col).unwrap_or(-1) == v {
                    item.data.with_vt(|vt| {
                        wlv.virt_inline = vt.data.virt_text().to_vec();
                        wlv.virt_inline_hl_mode = vt.hl_mode;
                    });

                    item.draw_col = i32::MIN;
                    break;
                }
            }
            drop(ds);
            if wlv.virt_inline.is_empty() {
                break;
            }
        } else {
            let mut attr: i32 = 0;
            let Some(text) =
                next_virt_text_chunk(&wlv.virt_inline, &mut wlv.virt_inline_i, &mut attr)
                    .map(std::ffi::CStr::to_owned)
            else {
                continue;
            };
            wlv.n_extra = text.as_bytes().len() as i32;
            let n_attr = mb_charlen(text.as_bytes());
            wlv.set_p_extra_owned(text);
            if wlv.n_extra == 0 {
                continue;
            }
            wlv.sc_extra = 0;
            wlv.sc_final = 0;
            wlv.extra_attr = attr;
            wlv.n_attr = n_attr;
            if wlv.skip_cells > 0 {
                let virt_text_width =
                    mb_string2cells(wlv.p_extra().map_or(&[][..], std::ffi::CStr::to_bytes)) as i32;
                if virt_text_width > wlv.skip_cells {
                    let mut skip_cells_remaining = wlv.skip_cells;
                    while skip_cells_remaining > 0 {
                        let p_bytes = wlv.p_extra().map_or(&[][..], std::ffi::CStr::to_bytes);
                        let cells = utf_ptr2cells(p_bytes);
                        if cells > skip_cells_remaining {
                            break;
                        }
                        let c_len = utfc_ptr2len(p_bytes);
                        skip_cells_remaining -= cells;
                        wlv.advance_p_extra(c_len as usize);
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

fn win_line_start(wp: &WinInfoInfo, wlv: &mut Winlinevars) {
    wlv.col = 0;
    wlv.off = 0;
    for i in 0..wp.w_view_width.get() as usize {
        g_st().linebuf_char.borrow_mut()[i] = schar_from_ascii(b' ');
        g_st().linebuf_attr.borrow_mut()[i] = 0;
        g_st().linebuf_vcol.borrow_mut()[i] = -1;
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

/// A TAB drawn with tab stops, and not itself concealed, absorbs the cells hidden before it (see
/// CharsizeArg.extconceal_off), but keeps its own cell (see CharSize.hidden).
fn tab_absorb_extconceal(wlv: &mut Winlinevars) {
    wlv.n_extra += wlv.extconceal_off;
    wlv.extconceal_off = wlv.n_extra.min(0);
    wlv.n_extra = wlv.n_extra.max(0);
}

fn get_rightmost_vcol(wp: &WinInfoInfo, color_cols: &[i32]) -> i32 {
    let mut ret: i32 = 0;

    if wp.w_onebuf_opt.wo_cuc.get() {
        ret = wp.w_virtcol.get();
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
        let mut sc_buf = [0u8; MAX_SCHAR_SIZE];
        schar_get(&mut sc_buf, mb_schar);
        debug_assert_eq!(crate::mbyte::ptr2cells(&sc_buf), 1);
    }
    let _ = mb_schar;
    1
}

/// Advance to the first displayed character, retaining partially visible cells. Re-reads `line`,
/// which the `_on_conceal` provider may free (see init_charsize_arg_extconceal()).
fn win_line_skip_columns(
    wp: &WinInfoInfo,
    wlv: &mut Winlinevars,
    draw_text: bool,
    start_vcol: i32,
    leadcol: Colnr,
    in_multispace: &mut bool,
    multispace_pos: &mut usize,
) -> usize {
    let init_line = if draw_text {
        ml_get_buf(&wp.buf_mut(), wlv.lnum)
    } else {
        crate::option_vars::RcCStr::EMPTY
    };
    let mut csarg = CharsizeArg::new();
    let mut spans = ConcealSpans::new();
    let cstype = if wp.w_onebuf_opt.wo_wrap.get() && !init_line.is_empty() {
        init_charsize_arg_extconceal(&mut csarg, wp, wlv.lnum, &mut spans)
    } else {
        init_charsize_arg(&mut csarg, wp, wlv.lnum, init_line)
    };
    csarg.max_head_vcol = start_vcol;
    let line = csarg.line.as_ptr().cast_mut();
    let mut ptr = line.cast_const();
    let mut prev_ptr = ptr;
    let mut cs = CharSize::default();

    let mut vcol = wlv.vcol;
    let line_cstr = csarg.line.clone();
    let mut ci = utf_ptr2_str_char_info(&line_cstr);
    while vcol - csarg.extconceal_off < start_vcol {
        let cli = utf_cluster_info(ci);
        cs = win_charsize(cstype, vcol, ci.cstr(), ci.chr.value, &mut csarg, cli.cells);
        vcol += cs.width;
        prev_ptr = ci.ptr.as_ptr().cast();
        let prev_idx = ci.offset_from(&line_cstr);
        if ci.is_empty() {
            break;
        }
        ci = cli.next;
        if wp.w_onebuf_opt.wo_list.get() {
            let ci_off = ci.offset_from(&line_cstr);
            let line_bytes = line_cstr.to_bytes();
            *in_multispace = line_bytes.get(prev_idx) == Some(&b' ')
                && (ci.first_byte() == b' '
                    || (prev_idx > 0 && line_bytes.get(prev_idx - 1) == Some(&b' ')));
            if !*in_multispace {
                *multispace_pos = 0;
            } else if ci_off >= leadcol as usize && !wp.w_p_lcs_chars.borrow().multispace.is_empty()
            {
                *multispace_pos += 1;
                if *multispace_pos >= wp.w_p_lcs_chars.borrow().multispace.len() {
                    *multispace_pos = 0;
                }
            } else if ci_off < leadcol as usize
                && !wp.w_p_lcs_chars.borrow().leadmultispace.is_empty()
            {
                *multispace_pos += 1;
                if *multispace_pos >= wp.w_p_lcs_chars.borrow().leadmultispace.len() {
                    *multispace_pos = 0;
                }
            }
        }
    }
    wlv.vcol = vcol - csarg.extconceal_off;
    wlv.extconceal_off = csarg.extconceal_off;
    ptr = ci.ptr.as_ptr().cast();
    let charsize = cs.width - cs.hidden;
    let head = cs.head;

    if wlv.vcol < start_vcol
        && (wp.w_onebuf_opt.wo_cuc.get()
            || !wlv.color_cols.is_empty()
            || virtual_active(wp)
            || (g_st().visual_global.borrow().active
                && wp.w_buffer.get() == curwin_mut().w_buffer.get())
            || (wlv.foldinfo.fi_level != 0 && wlv.foldinfo.fi_lines > 0))
    {
        wlv.vcol = start_vcol;
    }

    if wlv.vcol > start_vcol {
        wlv.vcol -= charsize;
        wlv.extconceal_off -= cs.hidden;
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

    if wp.w_onebuf_opt.wo_wrap.get() {
        wlv.need_showbreak = true;
    }
    ((ptr as isize) - (line as isize)).max(0) as usize
}

/// Display line "lnum" of window "wp" on the screen.
pub fn win_line(
    wp: &WinInfoInfo,
    lnum: Linenr,
    startrow: i32,
    endrow: i32,
    col_rows: i32,
    concealed: bool,
    spv: &mut Spellvars,
    foldinfo: Foldinfo,
) -> i32 {
    let view_width = wp.w_view_width.get();
    let view_height = wp.w_view_height.get();

    let in_curline = wp == curwin_ptr() && lnum == curwin_mut().w_cursor.get().lnum;
    let has_fold = foldinfo.fi_level != 0 && foldinfo.fi_lines > 0;
    let has_foldtext = has_fold && wp.w_onebuf_opt.wo_fdt.cb_type() != K_CALLBACK_NONE_CONST;

    let is_wrapped = wp.w_onebuf_opt.wo_wrap.get() && !has_fold;

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
    let mut nextline = [0u8; SPWORDLEN * 2];
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
    let mut conceal_spans = ConcealSpans::new();
    let cursor_vcol = if lnum == wp.w_cursor.get().lnum && !has_fold {
        cursor_disp_vcol(wp)
    } else {
        wp.w_virtcol.get()
    };

    let mut saved_search_attr: i32 = 0;
    let mut saved_area_attr: i32 = 0;
    let mut saved_decor_attr: i32 = 0;
    let mut saved_search_attr_from_match = false;

    let mut win_col_offset: i32 = 0;
    let mut area_active = false;
    let mut decor_need_recheck = false;

    let mut fold_vt: VirtText = Vec::new();

    let mut cul_screenline = false;
    let mut left_curline_col: i32 = 0;
    let mut right_curline_col: i32 = 0;

    let mut match_conc: i32 = 0;
    let mut on_last_col = false;
    let mut syntax_flags: i32 = 0;
    let mut syntax_seqnr: i32 = 0;
    let mut prev_syntax_id: i32 = 0;
    let conceal_attr = win_hl_attr(wp, HLF_CONCEAL);
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
        extconceal_off: 0,
        off: 0,
        cul_attr: 0,
        line_attr: 0,
        line_attr_lowprio: 0,
        sign_num_attr: 0,
        prev_num_attr: -1,
        sign_cul_attr: 0,
        fromcol: -10,
        tocol: MAXCOL,
        need_showbreak: false,
        breakindent: -1,
        char_attr: 0,
        n_extra: 0,
        n_attr: 0,
        p_extra_buf: None,
        p_extra_off: 0,
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
        virt_inline: Vec::new(),
        virt_inline_i: 0,
        virt_inline_hl_mode: 0,
        reset_extra_attr: false,
        skip_cells: 0,
        skipped_cells: 0,
        color_cols: &[],
    };

    let buf = wp.w_buffer.get();
    let draw_text = !concealed && (lnum != buf.b_ml.ml_line_count.get() + 1);

    let mut decor_provider_end_col: i32 = 0;
    let mut check_decor_providers = false;
    let cc_cols_vec: Vec<i32>;

    if col_rows == 0 && draw_text {
        extra_check = wp.w_onebuf_opt.wo_lbr.get();
        let w_s = wp.w_s_ptr();
        if syntax_present(wp) && !w_s.b_syn_error.get() && !w_s.b_syn_slow.get() && !has_foldtext {
            let save_did_emsg = g_st().did_emsg.get();
            g_st().did_emsg.set(0);
            syntax_start(wp, lnum);
            if g_st().did_emsg.get() != 0 {
                w_s.b_syn_error.set(true);
            } else {
                g_st().did_emsg.set(save_did_emsg);
                if !w_s.b_syn_slow.get() {
                    has_syntax = true;
                    extra_check = true;
                }
            }
        }

        check_decor_providers = true;

        wlv.color_cols =
            if !wp.buf_mut().terminal.get().is_null() || wp.w_p_cc_cols.borrow().is_empty() {
                &[]
            } else {
                cc_cols_vec = wp.w_p_cc_cols.borrow().clone();
                cc_cols_vec.as_slice()
            };
        let hlc = vcol_hlc(&wlv);
        advance_color_col(&mut wlv, hlc);

        if g_st().visual_global.borrow().active && wp.w_buffer.get() == curwin_mut().w_buffer.get()
        {
            let (mut top, bot) = if ltoreq(
                curwin_mut().w_cursor.get(),
                g_st().visual_global.borrow().start,
            ) {
                (
                    curwin_mut().w_cursor.get(),
                    g_st().visual_global.borrow().start,
                )
            } else {
                (
                    g_st().visual_global.borrow().start,
                    curwin_mut().w_cursor.get(),
                )
            };

            lnum_in_visual_area = lnum >= top.lnum && lnum <= bot.lnum;

            if g_st().visual_global.borrow().mode == CTRL_V_CONST {
                if lnum_in_visual_area {
                    let fcol = wp.w_old_cursor_fcol.get();
                    let lcol = wp.w_old_cursor_lcol.get();
                    wlv.fromcol = vcol2disp_vcol(wp, lnum, fcol);
                    wlv.tocol = vcol2disp_vcol(wp, lnum, lcol);
                }
            } else {
                if lnum > top.lnum && lnum <= bot.lnum {
                    wlv.fromcol = 0;
                } else if lnum == top.lnum {
                    if g_st().visual_global.borrow().mode == b'V' as i32 {
                        wlv.fromcol = 0;
                    } else {
                        getvvcol(
                            wp,
                            &mut top,
                            Some(&mut wlv.fromcol),
                            None,
                            None,
                            GETVCOL_DISP,
                        );

                        if gchar_pos(&mut top) == i32::from(NUL) {
                            wlv.tocol = wlv.fromcol + 1;
                        }
                    }
                }
                if g_st().visual_global.borrow().mode != b'V' as i32 && lnum == bot.lnum {
                    if opt_vars().p_sel.first_byte() == b'e' && bot.col == 0 && bot.coladd == 0 {
                        wlv.fromcol = -10;
                        wlv.tocol = MAXCOL;
                    } else if bot.col == MAXCOL {
                        wlv.tocol = MAXCOL;
                    } else {
                        let mut pos = bot;
                        if opt_vars().p_sel.first_byte() == b'e' {
                            getvvcol(wp, &mut pos, Some(&mut wlv.tocol), None, None, GETVCOL_DISP);
                        } else {
                            getvvcol(wp, &mut pos, None, None, Some(&mut wlv.tocol), GETVCOL_DISP);
                            wlv.tocol += 1;
                        }
                    }
                }
            }

            if wlv.fromcol >= 0 {
                area_highlighting = true;
                vi_attr = win_hl_attr(wp, HLF_V);
            }
        } else if g_st().search_global.hl_match.get()
            && wp == curwin_ptr()
            && !has_foldtext
            && lnum >= curwin_mut().w_cursor.get().lnum
            && lnum <= curwin_mut().w_cursor.get().lnum + g_st().search_global.match_lines.get()
        {
            if lnum == curwin_mut().w_cursor.get().lnum {
                let mut cursor = curwin_mut().w_cursor.get();
                getvcol(
                    &curwin_mut(),
                    &mut cursor,
                    Some(&mut wlv.fromcol),
                    None,
                    None,
                    GETVCOL_DISP,
                );
                curwin_mut().w_cursor.set(cursor);
            } else {
                wlv.fromcol = 0;
            }
            if lnum == curwin_mut().w_cursor.get().lnum + g_st().search_global.match_lines.get() {
                let mut pos = CursorPos {
                    lnum,
                    col: g_st().search_global.match_endcol.get(),
                    coladd: 0,
                };
                getvcol(
                    &curwin_mut(),
                    &mut pos,
                    Some(&mut wlv.tocol),
                    None,
                    None,
                    GETVCOL_DISP,
                );
            }
            if wlv.fromcol == wlv.tocol && g_st().search_global.match_endcol.get() != 0 {
                wlv.tocol = wlv.fromcol + 1;
            }
            area_highlighting = true;
            vi_attr = win_hl_attr(wp, HLF_I);
        }
    }

    let bg_attr = win_bg_attr(wp);

    let mut linestatus: i32 = 0;
    wlv.filler_lines = diff_check_with_linestatus(wp, lnum, Some(&mut linestatus));
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
        wp,
        lnum - 1,
        lnum,
        Some(&mut wlv.n_virt_below),
        Some(&mut virt_lines),
        true,
    );
    let total_virt_rows = wlv.n_virt_lines;
    wlv.filler_lines += wlv.n_virt_lines;
    if lnum == wp.w_topline.get() {
        wlv.virt_below_skip = wlv.n_virt_below.min(wlv.n_virt_lines - wp.w_topfill.get());
        wlv.n_virt_below -= wlv.virt_below_skip;
        wlv.filler_lines_skip = wlv.filler_lines - wlv.virt_below_skip - wp.w_topfill.get();
        wlv.filler_lines = wp.w_topfill.get();
        wlv.n_virt_lines = wlv.n_virt_lines.min(wlv.filler_lines);
    }
    wlv.filler_todo = wlv.filler_lines;

    if wp.w_onebuf_opt.wo_cul.get()
        && wp.w_p_culopt_flags.get() != K_OPT_CULOPT_FLAG_NUMBER
        && lnum == wp.w_cursorline.get()
        && !(wp == curwin_ptr() && g_st().visual_global.borrow().active)
    {
        cul_screenline =
            is_wrapped && (wp.w_p_culopt_flags.get() & K_OPT_CULOPT_FLAG_SCREENLINE) != 0;
        if !cul_screenline {
            apply_cursorline_highlight(wp, &mut wlv);
        } else {
            margin_columns_win(
                wp,
                cursor_vcol,
                &mut left_curline_col,
                &mut right_curline_col,
            );
        }
        area_highlighting = true;
    }

    let mut sign_line_attr: i32 = 0;
    decor_redraw_signs(
        wp,
        buf.borrow_mut(),
        wlv.lnum - 1,
        Some(&mut wlv.sattrs),
        Some(&mut sign_line_attr),
        Some(&mut wlv.sign_cul_attr),
        Some(&mut wlv.sign_num_attr),
    );

    let mut statuscol = Statuscol::new();
    if !wp.w_onebuf_opt.wo_stc.is_empty() {
        statuscol.draw = true;
        statuscol.sattrs = wlv.sattrs;
        statuscol.lnum = lnum;
        statuscol.foldinfo = foldinfo;
        statuscol.width = win_col_off(wp);
        statuscol.sign_cul_id = if use_cursor_line_highlight(wp, lnum) {
            wlv.sign_cul_attr
        } else {
            0
        };
    } else if wlv.sign_cul_attr > 0 {
        wlv.sign_cul_attr = if use_cursor_line_highlight(wp, lnum) {
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

    if bt_quickfix(wp.buf_opt().as_deref()) && qf_current_entry(wp) == lnum {
        wlv.line_attr = win_hl_attr(wp, HLF_QFL);
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

        nextline[SPWORDLEN] = 0;
        if lnum < wp.buf_mut().b_ml.ml_line_count.get() {
            let line = crate::memline::ml_get_buf_slice(&wp.buf_mut(), lnum + 1);
            spell_cat_line(&mut nextline[SPWORDLEN..SPWORDLEN * 2], &line);
        }
        let line_slice = crate::memline::ml_get_buf_slice(&wp.buf_mut(), lnum);
        let rem_sw = crate::charset::skipwhite_slice(&line_slice);
        if rem_sw.is_empty() {
            spv.spv_cap_col = 0;
            spv.spv_capcol_lnum = lnum + 1;
        } else if spv.spv_cap_col == 0 {
            spv.spv_cap_col = (line_slice.len() - rem_sw.len()) as i32;
        }

        if nextline[SPWORDLEN] == 0 {
            nextlinecol = MAXCOL;
            nextline_idx = 0;
        } else {
            let line_slice = crate::memline::ml_get_buf_slice(&wp.buf_mut(), lnum);
            let line_len = line_slice.len() as i32;
            if (line_len as usize) < SPWORDLEN {
                nextlinecol = 0;
                nextline[..line_len as usize].copy_from_slice(&line_slice);
                let tail_len = crate::memory::bytes_as_cstr(&nextline[SPWORDLEN..])
                    .to_bytes()
                    .len();
                nextline.copy_within(SPWORDLEN..=SPWORDLEN + tail_len, line_len as usize);
                nextline_idx = line_len + 1;
            } else {
                nextlinecol = line_len - SPWORDLEN as i32;
                nextline[..SPWORDLEN].copy_from_slice(
                    &line_slice[nextlinecol as usize..nextlinecol as usize + SPWORDLEN],
                );
                nextline_idx = SPWORDLEN as i32 + 1;
            }
        }
    }

    let mut line_cstr: crate::option_vars::RcCStr = if draw_text {
        ml_get_buf(&wp.buf_mut(), lnum)
    } else {
        crate::option_vars::RcCStr::EMPTY
    };
    let mut line = line_cstr.as_ptr().cast_mut();
    let mut ptr = line.cast_const();
    macro_rules! ptr_cstr {
        ($p:expr) => {{
            let off = (($p as usize).saturating_sub(line as usize)).min(line_cstr.to_bytes().len());
            &line_cstr[off..]
        }};
    }
    macro_rules! ptr_byte {
        ($p:expr) => {{
            let off = (($p as usize).saturating_sub(line as usize)).min(line_cstr.to_bytes().len());
            line_cstr.to_bytes_with_nul()[off]
        }};
    }

    let mut trailcol: Colnr = MAXCOL;
    let mut leadcol: Colnr = 0;

    let mut lcs_eol_todo = true;
    let lcs_eol = wp.w_p_lcs_chars.borrow().eol;
    let mut lcs_prec_todo = wp.w_p_lcs_chars.borrow().prec;

    if wp.w_onebuf_opt.wo_list.get() && !has_foldtext && draw_text {
        if wp.w_p_lcs_chars.borrow().space != 0
            || !wp.w_p_lcs_chars.borrow().multispace.is_empty()
            || !wp.w_p_lcs_chars.borrow().leadmultispace.is_empty()
            || wp.w_p_lcs_chars.borrow().trail != 0
            || wp.w_p_lcs_chars.borrow().lead != 0
            || wp.w_p_lcs_chars.borrow().nbsp != 0
        {
            extra_check = true;
        }
        if wp.w_p_lcs_chars.borrow().trail != 0 {
            let ptr_bytes = ptr_cstr!(ptr).to_bytes();
            trailcol = ml_get_buf_len(&wp.buf_mut(), lnum);
            while trailcol > 0
                && ascii_iswhite(ptr_bytes.get((trailcol - 1) as usize).copied().unwrap_or(0))
            {
                trailcol -= 1;
            }
            trailcol += ((ptr as isize) - (line as isize)) as Colnr;
        }
        if wp.w_p_lcs_chars.borrow().lead != 0
            || !wp.w_p_lcs_chars.borrow().leadmultispace.is_empty()
            || wp.w_p_lcs_chars.borrow().leadtab1 != 0
        {
            let ptr_bytes = ptr_cstr!(ptr).to_bytes();
            let rem = crate::charset::skipwhite_slice(ptr_bytes);
            leadcol = if rem.is_empty() {
                0
            } else {
                (ptr_bytes.len() - rem.len()) as Colnr
                    + ((ptr as isize) - (line as isize)) as Colnr
                    + 1
            };
        }
    }

    let start_vcol = if wp.w_onebuf_opt.wo_wrap.get() {
        if startrow == 0 { wp.w_skipcol.get() } else { 0 }
    } else {
        wp.w_leftcol.get()
    };

    if has_foldtext {
        wlv.vcol = start_vcol;
    } else if start_vcol > 0 && col_rows == 0 {
        let skip_off = win_line_skip_columns(
            wp,
            &mut wlv,
            draw_text,
            start_vcol,
            leadcol,
            &mut in_multispace,
            &mut multispace_pos,
        );
        if draw_text {
            line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
            line = line_cstr.as_ptr().cast_mut();
        }
        ptr = line.wrapping_add(skip_off);
        if spv.spv_has_spell {
            let linecol = ((ptr as isize) - (line as isize)) as Colnr;
            let mut spell_hlf: Hlf = HLF_COUNT;

            let pos = wp.w_cursor.get();
            wp.w_cursor.set_lnum(lnum);
            wp.w_cursor.set_col(linecol);
            let len = spell_move_to(wp, FORWARD, SMT_ALL, true, Some(&mut spell_hlf));

            line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
            line = line_cstr.as_ptr().cast_mut();
            ptr = line.wrapping_add(linecol as usize);

            if len == 0 || wp.w_cursor.get().col > linecol {
                word_end = linecol + spell_to_word_end(ptr_cstr!(ptr).to_bytes(), wp) as i32 + 1;
            } else {
                debug_assert!(len <= i32::MAX as usize);
                word_end = wp.w_cursor.get().col + len as i32 + 1;

                if spell_hlf != HLF_COUNT {
                    spell_attr = g_st().highlight_attr[spell_hlf as usize].get();
                }
            }
            wp.w_cursor.set(pos);

            if has_syntax {
                syntax_start(wp, lnum);
            }
        }
    }

    if check_decor_providers {
        let col = ((ptr as isize) - (line as isize)) as i32;
        decor_provider_end_col =
            decor_providers_setup(endrow - startrow, start_vcol == 0, lnum, col, wp);
        line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
        line = line_cstr.as_ptr().cast_mut();
        ptr = line.wrapping_add(col as usize);
    }

    decor_redraw_line(wp, lnum - 1, &mut g_st().decor_state.borrow_mut());
    if !has_decor && decor_has_more_decorations(&mut g_st().decor_state.borrow_mut(), lnum - 1) {
        has_decor = true;
        extra_check = true;
    }
    if wp.w_onebuf_opt.wo_lbr.get() && line_has_extconceal(wp, lnum) {
        let len = ml_get_buf_len(&wp.buf_mut(), lnum);
        conceal_spans_collect(wp, lnum - 1, len, &mut conceal_spans);
    }

    if wlv.fromcol >= wlv.tocol {
        wlv.fromcol = -1;
    }

    if col_rows == 0 && draw_text && !has_foldtext {
        let v = ((ptr as isize) - (line as isize)) as i32;
        area_highlighting |= prepare_search_hl_line(
            wp,
            lnum,
            v,
            &mut g_st().screen_search_hl.borrow_mut(),
            &mut search_attr,
            &mut search_attr_from_match,
        );
        line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
        line = line_cstr.as_ptr().cast_mut();
        ptr = line.wrapping_add(v as usize);
    }

    if (g_st().state_global.get() & MODE_INSERT) != 0
        && ins_compl_win_active(wp)
        && (in_curline || ins_compl_lnum_in_range(lnum))
    {
        area_highlighting = true;
    }

    win_line_start(wp, &mut wlv);
    let mut draw_cols = true;
    let mut leftcols_width: i32 = 0;

    let mut term_attrs = [0_i32; TERM_ATTRS_MAX];
    if !wp.buf_mut().terminal.get().is_null() {
        terminal_get_line_attributes(
            wp.buf_mut().terminal.get().borrow_mut(),
            lnum,
            &mut term_attrs,
        );

        extra_check = true;
    }

    let may_have_inline_virt = !has_foldtext && buf_meta_total(&wp.buf_mut(), K_MTMETA_INLINE) > 0;
    let conceal_enabled =
        wp.w_onebuf_opt.wo_cole.get() > 0 && !conceal_cursor_reveals_line(wp, lnum);
    let mut has_virt_line = false;
    let mut virt_line_flags: i32 = 0;
    let mut virt_line_index: i32 = 0;
    let mut virt_line_start_row: i32 = 0;
    let mut virt_line_skip_cells: i32 = 0;
    let mut conceal_tab_vcol: Colnr = -1;
    let mut prefix_vcol: Colnr = -1;
    let mut prefix_width: i32 = 0;
    let mut disp_len: i32 = -1;

    loop {
        let mut has_match_conc: i32 = 0;
        let mut decor_conceal: i32 = 0;
        let mut conceal_replacement_extra: i32 = 0;
        let mut conceal_lbr_body: i32 = 0;
        let mut conceal_lbr_tail: i32 = 0;
        let mut conceal_reflow = false;
        let mut extconceal_char: Schar = 0;
        let mut extconceal_attr: i32 = 0;

        let mut did_decrement_ptr = false;
        let mut cell_hidden = false;

        if check_decor_providers
            && (((ptr as isize) - (line as isize)) as i32) >= decor_provider_end_col
        {
            let col = ((ptr as isize) - (line as isize)) as i32;
            decor_provider_end_col = invoke_range_next(wp, lnum, col, 100);
            line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
            line = line_cstr.as_ptr().cast_mut();
            ptr = line.wrapping_add(col as usize);
            if !has_decor
                && decor_has_more_decorations(&mut g_st().decor_state.borrow_mut(), lnum - 1)
            {
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
                        let vl = &virt_lines[virt_line_index as usize];
                        let line_rows = decor_virt_line_rows(wp, vl, 0, None);
                        if target_row < virt_line_start_row + line_rows {
                            has_virt_line = true;

                            virt_line_flags = vl.flags;

                            decor_virt_line_rows(
                                wp,
                                vl,
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
                let v = ((ptr as isize) - (line as isize)) as i32;
                draw_statuscol(wp, &mut wlv, col_rows, &mut statuscol);
                if wp.w_redr_statuscol.get() {
                    break;
                }
                if draw_text {
                    line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
                    line = line_cstr.as_ptr().cast_mut();
                    ptr = line.wrapping_add(v as usize);
                }
            } else {
                draw_foldcolumn(wp, &mut wlv);

                for sign_idx in 0..wp.w_scwidth.get() {
                    draw_sign(false, wp, &mut wlv, sign_idx);
                }

                draw_lnum_col(wp, &mut wlv);
            }

            win_col_offset = wlv.off;

            if col_rows > 0 {
                wlv_put_linebuf(wp, &wlv, wlv.off.min(view_width), false, bg_attr, 0);
                if (wlv.row + 1 - wlv.startrow < col_rows
                    && (statuscol.draw
                        || win_hl_attr(wp, HLF_LNA) != win_hl_attr(wp, HLF_N)
                        || win_hl_attr(wp, HLF_LNB) != win_hl_attr(wp, HLF_N)))
                    || wlv.filler_todo > 0
                {
                    wlv.row += 1;
                    if wlv.row == endrow {
                        break;
                    }
                    wlv.filler_todo -= 1;
                    has_virt_line = false;
                    if wlv.filler_todo == 0 && (wp.w_botfill.get() || !draw_text) {
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
            prefix_vcol = wlv.vcol;
            if !wp.w_briopt_sbr.get() {
                handle_breakindent(wp, &mut wlv, gap_decor_attr);
            }
            handle_showbreak_and_filler(wp, &mut wlv, gap_decor_attr);
            if wp.w_briopt_sbr.get() {
                handle_breakindent(wp, &mut wlv, gap_decor_attr);
            }
            prefix_width = wlv.vcol - prefix_vcol;

            wlv.col = wlv.off;
            draw_cols = false;
            if wlv.filler_todo <= 0 {
                leftcols_width = wlv.off;
            }
            if has_decor && wlv.row == startrow + wlv.filler_lines {
                decor_redraw_col(
                    wp,
                    (((ptr as isize) - (line as isize)) as Colnr) - 1,
                    wlv.off,
                    true,
                    &mut g_st().decor_state.borrow_mut(),
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

            if g_st().dollar_vcol.get() >= 0 && in_curline && wlv.vcol >= cursor_vcol {
                draw_virt_text(wp, buf.borrow_mut(), win_col_offset, &mut wlv.col, wlv.row);

                wlv_put_linebuf(wp, &wlv, wlv.col, false, bg_attr, 0);
                if wp.w_onebuf_opt.wo_cuc.get() {
                    wlv.row = wp.w_cline_row.get() + wp.w_cline_height.get();
                } else {
                    wlv.row = view_height;
                }
                break;
            }

            let draw_folded = has_fold && wlv.row == startrow + wlv.filler_lines;
            if draw_folded && wlv.n_extra == 0 {
                folded_attr = win_hl_attr(wp, HLF_FL);
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
                            && (wlv.n_extra == 0 && utf_ptr2cells(ptr_cstr!(ptr)) > 1))
                    {
                        area_active = true;
                    } else if area_active && wlv.vcol == wlv.tocol {
                        area_active = false;
                    }

                    let selected = area_active;
                    if decor_need_recheck {
                        if !may_have_inline_virt {
                            decor_recheck_draw_col(
                                wlv.off,
                                selected,
                                &mut g_st().decor_state.borrow_mut(),
                            );
                        }
                        decor_need_recheck = false;
                    }
                    extmark_attr = decor_redraw_col(
                        wp,
                        ((ptr as isize) - (line as isize)) as Colnr,
                        if may_have_inline_virt { -3 } else { wlv.off },
                        selected,
                        &mut g_st().decor_state.borrow_mut(),
                        decor_provider_end_col - 1,
                    );
                    if may_have_inline_virt {
                        handle_inline_virtual_text(
                            &mut wlv,
                            (ptr as isize) - (line as isize),
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
                        &mut saved_area_attr
                    } else {
                        &mut area_attr
                    };

                if wlv.vcol == wlv.fromcol
                    || (wlv.vcol + 1 == wlv.fromcol
                        && ((wlv.n_extra == 0 && utf_ptr2cells(ptr_cstr!(ptr)) > 1)
                            || (wlv.n_extra > 0
                                && wlv
                                    .p_extra()
                                    .is_some_and(|s| utf_ptr2cells(s.to_bytes()) > 1))))
                {
                    *area_attr_p = vi_attr;

                    area_active = true;
                } else if *area_attr_p != 0 && wlv.vcol == wlv.tocol {
                    *area_attr_p = 0;

                    area_active = false;
                }

                if !has_foldtext && wlv.n_extra == 0 {
                    let v = ((ptr as isize) - (line as isize)) as i32;
                    search_attr = update_search_hl(
                        wp,
                        lnum,
                        v,
                        &mut g_st().screen_search_hl.borrow_mut(),
                        &mut has_match_conc,
                        &mut match_conc,
                        lcs_eol_todo,
                        &mut on_last_col,
                        &mut search_attr_from_match,
                    );
                    line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
                    line = line_cstr.as_ptr().cast_mut();
                    ptr = line.wrapping_add(v as usize);

                    if ptr_byte!(ptr) == 0 {
                        has_match_conc = 0;
                    }

                    if (g_st().state_global.get() & MODE_INSERT) != 0
                        && ins_compl_win_active(wp)
                        && (in_curline || ins_compl_lnum_in_range(lnum))
                    {
                        let ins_match_attr = ins_compl_col_range_attr(
                            lnum,
                            ((ptr as isize) - (line as isize)) as i32,
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
                        if (((ptr as isize) - (line as isize)) as i32)
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
                        && (((ptr as isize) - (line as isize)) as i32) >= change_start
                        && wlv.n_extra == 0
                    {
                        wlv.diff_hlf = if added { HLF_TXA } else { HLF_TXD };
                    }
                    if (wlv.diff_hlf == HLF_TXD || wlv.diff_hlf == HLF_TXA)
                        && (((((ptr as isize) - (line as isize)) as i32) >= change_end
                            && wlv.n_extra == 0)
                            || (wlv.n_extra > 0 && wlv.extra_for_extmark))
                    {
                        wlv.diff_hlf = HLF_CHD;
                    }
                    set_line_attr_for_diff(wp, &mut wlv);
                }

                if area_attr != 0 {
                    char_attr_pri = hl_combine_attr(wlv.line_attr, area_attr);
                    if !g_st().search_global.hl_match.get() {
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
                let v = ((ptr as isize) - (line as isize)) as i32;
                let lnume = lnum + foldinfo.fi_lines - 1;
                let fdt = get_foldtext(wp, lnum, lnume, foldinfo, &mut fold_vt);
                wlv.n_extra = fdt.to_bytes().len() as i32;
                wlv.set_p_extra_owned(fdt);
                wlv.sc_extra = 0;
                wlv.sc_final = 0;

                line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
                line = line_cstr.as_ptr().cast_mut();
                ptr = line.wrapping_add(v as usize);
            }

            if draw_folded
                && wlv.n_extra == 0
                && wlv.col < view_width
                && (has_foldtext
                    || (ptr_byte!(ptr) == 0
                        && (!wp.w_onebuf_opt.wo_list.get() || !lcs_eol_todo || lcs_eol == 0)))
            {
                wlv.sc_extra = wp.w_p_fcs_chars.get().fold;
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
                    debug_assert!(wlv.p_extra().is_some());
                    let ci = utf_ptr2_str_char_info(wlv.p_extra().unwrap_or(c""));
                    let cli = utf_cluster_info(ci);
                    let mut mb_l = (ci.ptr.len() - cli.next.ptr.len()) as i32;
                    mb_schar = schar_from_cluster(ci, cli);
                    mb_schar_width = cli.cells;
                    if mb_l > wlv.n_extra || mb_l == 0 {
                        mb_l = 1;
                    }

                    if wlv.col >= view_width - 1 && mb_schar_width == 2 {
                        mb_l = 1;
                        mb_schar = schar_from_ascii(b'>');
                        mb_schar_width = 1;
                        multi_attr = win_hl_attr(wp, HLF_AT);

                        if wlv.cul_attr != 0 {
                            multi_attr = if wlv.line_attr_lowprio != 0 {
                                hl_combine_attr(wlv.cul_attr, multi_attr)
                            } else {
                                hl_combine_attr(multi_attr, wlv.cul_attr)
                            };
                        }
                    } else {
                        wlv.n_extra -= mb_l;
                        wlv.advance_p_extra(mb_l as usize);
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
                        wlv.extra_attr = win_hl_attr(wp, HLF_AT);
                    }
                }

                if wlv.n_extra <= 0 {
                    if n_extra_next <= 0 {
                        if search_attr == 0 {
                            search_attr = saved_search_attr;
                            saved_search_attr = 0;
                        }
                        if area_attr == 0 && ptr_byte!(ptr) != 0 {
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
                        debug_assert!(wlv.p_extra().is_some());
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

                if ptr_byte!(ptr) == 0 {
                    wlv.skip_cells = 0;
                }

                let ci = utf_ptr2_str_char_info(ptr_cstr!(ptr));
                let cli = utf_cluster_info(ci);
                let mut mb_l = (ci.ptr.len() - cli.next.ptr.len()) as i32;
                let mb_c = ci.chr.value;
                let mut unprintable_ascii = false;

                if mb_c < 0 || (mb_l >= 1 && mb_c == 0) || (mb_l > 1 && !vim_isprintc(mb_c)) {
                    transchar_hex(
                        &mut wlv.extra,
                        if mb_c < 0 {
                            i32::from(ptr_byte!(ptr))
                        } else {
                            mb_c
                        },
                    );
                    if wp.w_onebuf_opt.wo_rl.get() {
                        rl_mirror_ascii(&mut wlv.extra);
                    }
                    let p_extra_cstr = &crate::memory::bytes_as_cstr(&wlv.extra)[1..];
                    let p_extra_len = p_extra_cstr.to_bytes().len() as i32;
                    let p_extra_owned = p_extra_cstr.to_owned();
                    wlv.set_p_extra_owned(p_extra_owned);
                    mb_schar = schar_from_ascii(wlv.extra[0]);
                    mb_schar_width = 1;
                    wlv.n_extra = p_extra_len;
                    wlv.sc_extra = 0;
                    wlv.sc_final = 0;
                    if area_attr == 0 && search_attr == 0 {
                        wlv.n_attr = wlv.n_extra + 1;
                        wlv.extra_attr = win_hl_attr(wp, HLF_8);
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
                conceal_reflow = is_wrapped
                    && conceal_enabled
                    && g_st().decor_state.borrow().conceal_slot_i >= 0;
                if conceal_reflow {
                    extconceal_char = decor_extconceal_char(
                        wp,
                        &g_st().decor_state.borrow(),
                        ((ptr as isize) - (line as isize)) as i32,
                        Some(&mut extconceal_attr),
                    );
                }
                let extconceal = conceal_enabled
                    && g_st().decor_state.borrow().conceal_slot_i >= 0
                    && (is_wrapped || g_st().decor_state.borrow().conceal != 0);
                let disp_char = if !extconceal {
                    mb_schar
                } else if conceal_reflow {
                    extconceal_char
                } else {
                    decor_conceal_char(wp, &g_st().decor_state.borrow())
                };
                if wlv.col >= view_width - 1 && disp_char != 0 && schar_cells(disp_char) == 2 {
                    if extconceal && mb_c == i32::from(TAB) {
                        conceal_tab_vcol = wlv.vcol;
                    }
                    mb_schar = schar_from_ascii(b'>');
                    mb_schar_width = 1;
                    unprintable_ascii = false;
                    mb_l = 1;
                    multi_attr = win_hl_attr(wp, HLF_AT);
                    ptr = ptr.wrapping_sub(1);
                    did_decrement_ptr = true;
                } else if ptr_byte!(ptr) != 0 {
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
                        wlv.extra_attr = win_hl_attr(wp, HLF_AT);
                        saved_attr2 = wlv.char_attr;
                    }
                }
                ptr = ptr.wrapping_add(1);

                decor_attr = 0;
                syntax_attr = 0;
                if extra_check {
                    let w_s = wp.w_s_ptr();
                    let no_plain_buffer =
                        (w_s.b_p_spo_flags.get() & K_OPT_SPO_FLAG_NOPLAINBUFFER) != 0;
                    let mut can_spell = !no_plain_buffer;

                    let v = ((ptr as isize) - (line as isize)) as i32;
                    let prev_v = (prev_ptr as isize) - (line as isize);
                    if has_syntax && v > 0 {
                        let save_did_emsg = g_st().did_emsg.get();
                        g_st().did_emsg.set(0);

                        decor_attr = get_syntax_attr(
                            v - 1,
                            spv.spv_has_spell.then_some(&mut can_spell),
                            false,
                        );

                        if g_st().did_emsg.get() != 0 {
                            w_s.b_syn_error.set(true);
                            has_syntax = false;
                        } else {
                            g_st().did_emsg.set(save_did_emsg);
                        }

                        if w_s.b_syn_slow.get() {
                            has_syntax = false;
                        }

                        line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
                        line = line_cstr.as_ptr().cast_mut();
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
                        decor_conceal = g_st().decor_state.borrow().conceal;
                        can_spell = tristate_to_bool(g_st().decor_state.borrow().spell, can_spell);
                    }

                    char_attr_base = hl_combine_attr(folded_attr, decor_attr);
                    wlv.char_attr = hl_combine_attr(char_attr_base, char_attr_pri);

                    let mut v1 = ((ptr as isize) - (line as isize)) as i32;
                    if spv.spv_has_spell && v1 >= word_end && v1 > cur_checked_col {
                        spell_attr = 0;
                        if mb_schar != 0
                            && !crate::charset::skipwhite_slice(ptr_cstr!(prev_ptr).to_bytes())
                                .is_empty()
                            && can_spell
                        {
                            let mut spell_hlf: Hlf = HLF_COUNT;
                            v1 -= mb_l - 1;

                            let prev_col = ((prev_ptr as isize) - (line as isize)) as i32;
                            let used_nextline = prev_col - nextlinecol >= 0;
                            let p_off = prev_col - nextlinecol;
                            let p_cstr = if used_nextline {
                                crate::memory::bytes_as_cstr(&nextline[p_off as usize..])
                            } else {
                                ptr_cstr!(prev_ptr)
                            };
                            spv.spv_cap_col -= prev_col;
                            let tmplen = spell_check(
                                wp,
                                p_cstr,
                                &mut spell_hlf,
                                Some(&mut spv.spv_cap_col),
                                spv.spv_unchanged,
                            );
                            debug_assert!(tmplen <= i32::MAX as usize);
                            let len = tmplen as i32;
                            word_end = v1 + len;

                            if spell_hlf != HLF_COUNT
                                && (g_st().state_global.get() & MODE_INSERT) != 0
                                && wp.w_cursor.get().lnum == lnum
                                && wp.w_cursor.get().col >= prev_col
                                && wp.w_cursor.get().col < word_end
                            {
                                spell_hlf = HLF_COUNT;
                                g_st().spell_redraw_lnum.set(lnum);
                            }

                            if spell_hlf == HLF_COUNT && used_nextline && p_off + len > nextline_idx
                            {
                                spv.spv_checked_lnum = lnum + 1;
                                spv.spv_checked_col = p_off + len - nextline_idx;
                            }

                            if spell_hlf != HLF_COUNT {
                                spell_attr = g_st().highlight_attr[spell_hlf as usize].get();
                            }

                            if spv.spv_cap_col > 0 {
                                if used_nextline && p_off + spv.spv_cap_col >= nextline_idx {
                                    spv.spv_capcol_lnum = lnum + 1;
                                    spv.spv_cap_col = p_off + spv.spv_cap_col - nextline_idx;
                                } else {
                                    spv.spv_cap_col += prev_col;
                                }
                            }
                        }
                    }
                    if spell_attr != 0 {
                        char_attr_base = hl_combine_attr(char_attr_base, spell_attr);
                        wlv.char_attr = hl_combine_attr(char_attr_base, char_attr_pri);
                    }

                    if !wp.buf_mut().terminal.get().is_null() {
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
                        hl_combine_attr(syntax_attr, g_st().decor_state.borrow().current_hl_eol)
                    } else {
                        decor_attr
                    };

                    let mut lbr_char = mb_c;
                    if extconceal {
                        lbr_char = if disp_char == 0 {
                            0
                        } else {
                            schar_get_first_codepoint(disp_char)
                        };
                    }
                    if wp.w_onebuf_opt.wo_lbr.get()
                        && lbr_char > 0
                        && lbr_char < 128
                        && vim_isbreak(lbr_char)
                    {
                        let prev_idx = ((ptr as usize) - (line as usize)).saturating_sub(1);
                        let mb_off =
                            crate::mbyte::utf_head_off_slice(line_cstr.to_bytes(), prev_idx);
                        let p = ptr.wrapping_sub((mb_off + 1) as usize);

                        let has_conceal = !conceal_spans.is_empty();
                        if has_conceal || !vim_isbreak(i32::from(ptr_byte!(ptr))) {
                            let mut csarg = CharsizeArg::new();
                            let mut cstype =
                                init_charsize_arg_skip_cur_text(&mut csarg, wp, lnum, &line_cstr);
                            if has_conceal {
                                csarg.extconceal = Some(conceal_spans.clone());
                                csarg.extconceal_off = wlv.extconceal_off;
                                cstype = false;
                            }
                            let lbr_ci = utf_ptr2_str_char_info(ptr_cstr!(p));
                            let lbr_cli = utf_cluster_info(lbr_ci);
                            let lbr_size = win_charsize(
                                cstype,
                                wlv.vcol + csarg.extconceal_off,
                                lbr_ci.cstr(),
                                lbr_ci.chr.value,
                                &mut csarg,
                                lbr_cli.cells,
                            );
                            if lbr_size.linebreak {
                                wlv.n_extra = lbr_size.width - 1;
                                conceal_lbr_body = lbr_size.body;
                                conceal_lbr_tail = lbr_size.tail;

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
                                        wlv.vcol + wlv.extconceal_off,
                                        wp.buf_mut().b_p_ts.get(),
                                        &wp.buf_mut().b_p_vts_array.borrow(),
                                    ) - 1;
                                }
                                wlv.sc_extra = schar_from_ascii(if mb_off > 0 {
                                    MB_FILLER_CHAR
                                } else {
                                    b' '
                                });
                                wlv.sc_final = 0;
                                if mb_c < 128 && ascii_iswhite(mb_c) {
                                    if mb_c == i32::from(TAB) {
                                        fix_for_boguscols(&mut wlv);
                                        if !extconceal && !wp.w_onebuf_opt.wo_list.get() {
                                            tab_absorb_extconceal(&mut wlv);
                                        } else if !extconceal
                                            && wp.w_p_lcs_chars.borrow().tab1 != 0
                                            && wlv.extconceal_off > 0
                                        {
                                            wlv.n_extra += wlv.extconceal_off;
                                        }
                                    }
                                    if !wp.w_onebuf_opt.wo_list.get() {
                                        mb_schar = schar_from_ascii(b' ');
                                        mb_schar_width = 1;
                                        unprintable_ascii = false;
                                    }
                                }
                            }
                        }
                    }

                    if wp.w_onebuf_opt.wo_list.get() {
                        in_multispace = mb_c == b' ' as i32
                            && (ptr_byte!(ptr) == b' '
                                || (prev_ptr > line.cast_const()
                                    && ptr_byte!(prev_ptr.wrapping_sub(1)) == b' '));
                        if !in_multispace {
                            multispace_pos = 0;
                        }
                    }

                    if wp.w_onebuf_opt.wo_list.get()
                        && ((((mb_c == 160 && mb_l == 2) || (mb_c == 0x202f && mb_l == 3))
                            && wp.w_p_lcs_chars.borrow().nbsp != 0)
                            || (mb_c == b' ' as i32
                                && mb_l == 1
                                && (wp.w_p_lcs_chars.borrow().space != 0
                                    || (in_multispace
                                        && !wp.w_p_lcs_chars.borrow().multispace.is_empty()))
                                && (((ptr as isize) - (line as isize)) as Colnr) >= leadcol
                                && (((ptr as isize) - (line as isize)) as Colnr) <= trailcol))
                    {
                        if in_multispace && !wp.w_p_lcs_chars.borrow().multispace.is_empty() {
                            mb_schar =
                                wp.w_p_lcs_chars.borrow().multispace.as_slice()[multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= wp.w_p_lcs_chars.borrow().multispace.len() {
                                multispace_pos = 0;
                            }
                        } else {
                            mb_schar = if mb_c == b' ' as i32 {
                                wp.w_p_lcs_chars.borrow().space
                            } else {
                                wp.w_p_lcs_chars.borrow().nbsp
                            };
                        }
                        wlv.n_attr = 1;
                        wlv.extra_attr = win_hl_attr(wp, HLF_0);
                        saved_attr2 = wlv.char_attr;
                        mb_schar_width = schar_get_single_width(mb_schar);
                    }

                    if mb_c == b' ' as i32
                        && mb_l == 1
                        && ((trailcol != MAXCOL
                            && ptr > line.cast_const().wrapping_add(trailcol as usize))
                            || (leadcol != 0
                                && ptr < line.cast_const().wrapping_add(leadcol as usize)))
                    {
                        if leadcol != 0
                            && in_multispace
                            && ptr < line.cast_const().wrapping_add(leadcol as usize)
                            && !wp.w_p_lcs_chars.borrow().leadmultispace.is_empty()
                        {
                            mb_schar =
                                wp.w_p_lcs_chars.borrow().leadmultispace.as_slice()[multispace_pos];
                            multispace_pos += 1;
                            if multispace_pos >= wp.w_p_lcs_chars.borrow().leadmultispace.len() {
                                multispace_pos = 0;
                            }
                        } else if ptr > line.cast_const().wrapping_add(trailcol as usize)
                            && wp.w_p_lcs_chars.borrow().trail != 0
                        {
                            mb_schar = wp.w_p_lcs_chars.borrow().trail;
                        } else if ptr < line.cast_const().wrapping_add(leadcol as usize)
                            && wp.w_p_lcs_chars.borrow().lead != 0
                        {
                            mb_schar = wp.w_p_lcs_chars.borrow().lead;
                        } else if leadcol != 0 && wp.w_p_lcs_chars.borrow().space != 0 {
                            mb_schar = wp.w_p_lcs_chars.borrow().space;
                        }

                        wlv.n_attr = 1;
                        wlv.extra_attr = win_hl_attr(wp, HLF_0);
                        saved_attr2 = wlv.char_attr;
                        mb_schar_width = schar_get_single_width(mb_schar);
                    }
                }

                if unprintable_ascii {
                    if mb_c == i32::from(TAB)
                        && (!wp.w_onebuf_opt.wo_list.get() || wp.w_p_lcs_chars.borrow().tab1 != 0)
                    {
                        let mut tab_len: i32;
                        let mut vcol_adjusted = wlv.vcol;
                        let mut lcs_tab1 = wp.w_p_lcs_chars.borrow().tab1;
                        let mut lcs_tab2 = wp.w_p_lcs_chars.borrow().tab2;
                        let mut lcs_tab3 = wp.w_p_lcs_chars.borrow().tab3;
                        if wp.w_onebuf_opt.wo_list.get()
                            && wp.w_p_lcs_chars.borrow().leadtab1 != 0
                            && ptr < line.cast_const().wrapping_add(leadcol as usize)
                        {
                            lcs_tab1 = wp.w_p_lcs_chars.borrow().leadtab1;
                            lcs_tab2 = wp.w_p_lcs_chars.borrow().leadtab2;
                            lcs_tab3 = wp.w_p_lcs_chars.borrow().leadtab3;
                        }
                        if wlv.vcol == prefix_vcol + prefix_width && wp.w_onebuf_opt.wo_wrap.get() {
                            vcol_adjusted = prefix_vcol;
                        }
                        if conceal_tab_vcol >= 0 {
                            vcol_adjusted = conceal_tab_vcol;
                            conceal_tab_vcol = -1;
                        }
                        vcol_adjusted += wlv.extconceal_off;
                        tab_len = tabstop_padding(
                            vcol_adjusted,
                            wp.buf_mut().b_p_ts.get(),
                            &wp.buf_mut().b_p_vts_array.borrow(),
                        ) - 1;

                        if !wp.w_onebuf_opt.wo_lbr.get() || !wp.w_onebuf_opt.wo_list.get() {
                            wlv.n_extra = tab_len;
                        } else {
                            let saved_nextra = wlv.n_extra;

                            if wlv.vcol_off_co + wlv.extconceal_off > 0 {
                                tab_len += wlv.vcol_off_co + wlv.extconceal_off;
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
                                    len = len.wrapping_add_signed((wlv.n_extra - tab_len) as isize);
                                }
                                with_extra_buf(len + 1, |extra_slice| {
                                    extra_slice[..len].fill(b' ');
                                    extra_slice[len] = 0;
                                    let mut p_idx = 0usize;

                                    for i in 0..tab_len {
                                        if extra_slice[p_idx] == 0 {
                                            tab_len = i;
                                            break;
                                        }
                                        let mut lcs = lcs_tab2;
                                        if lcs_tab3 != 0 && i == tab_len - 1 {
                                            lcs = lcs_tab3;
                                        }
                                        let slen = schar_get_adv(&mut extra_slice[p_idx..], lcs);
                                        p_idx += slen;
                                        wlv.n_extra +=
                                            slen as i32 - if saved_nextra > 0 { 1 } else { 0 };
                                    }
                                    wlv.set_p_extra(crate::memory::bytes_as_cstr(extra_slice));
                                });

                                if wlv.vcol_off_co + wlv.extconceal_off > 0 {
                                    wlv.n_extra -= wlv.vcol_off_co + wlv.extconceal_off;
                                }
                            }
                        }

                        {
                            let vc_saved = wlv.vcol_off_co + wlv.extconceal_off;
                            fix_for_boguscols(&mut wlv);
                            if !extconceal {
                                tab_absorb_extconceal(&mut wlv);
                            }
                            if wlv.n_extra == tab_len + vc_saved
                                && wp.w_onebuf_opt.wo_list.get()
                                && wp.w_p_lcs_chars.borrow().tab1 != 0
                            {
                                tab_len += vc_saved;
                            }
                        }

                        if wp.w_onebuf_opt.wo_list.get() {
                            mb_schar = if wlv.n_extra == 0 && lcs_tab3 != 0 {
                                lcs_tab3
                            } else {
                                lcs_tab1
                            };
                            if wp.w_onebuf_opt.wo_lbr.get()
                                && wlv.p_extra().is_some_and(|s| !s.to_bytes().is_empty())
                            {
                                wlv.sc_extra = 0;
                            } else {
                                wlv.sc_extra = lcs_tab2;
                            }
                            wlv.sc_final = lcs_tab3;
                            wlv.n_attr = tab_len + 1;
                            wlv.extra_attr = win_hl_attr(wp, HLF_0);
                            saved_attr2 = wlv.char_attr;
                        } else {
                            wlv.sc_final = 0;
                            wlv.sc_extra = schar_from_ascii(b' ');
                            mb_schar = schar_from_ascii(b' ');
                        }
                        mb_schar_width = schar_get_single_width(mb_schar);
                    } else if mb_schar == 0
                        && (wp.w_onebuf_opt.wo_list.get()
                            || (wlv.fromcol >= 0
                                && wlv.tocol > wlv.vcol
                                && g_st().visual_global.borrow().mode != CTRL_V_CONST
                                && wlv.col < view_width))
                        && lcs_eol_todo
                        && lcs_eol != 0
                    {
                        if wlv.diff_hlf == 0 && wlv.line_attr == 0 && wlv.line_attr_lowprio == 0 {
                            if !(area_highlighting
                                && virtual_active(wp)
                                && wlv.tocol != MAXCOL
                                && wlv.vcol < wlv.tocol)
                            {
                                wlv.set_p_extra(c"");
                            }
                            wlv.n_extra = 0;
                        }
                        if wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().eol > 0 {
                            mb_schar = wp.w_p_lcs_chars.borrow().eol;
                        } else {
                            mb_schar = schar_from_ascii(b' ');
                        }
                        lcs_eol_todo = false;
                        ptr = ptr.wrapping_sub(1);
                        wlv.extra_attr = win_hl_attr(wp, HLF_AT);
                        wlv.n_attr = 1;
                        mb_schar_width = schar_get_single_width(mb_schar);
                    } else if mb_schar != 0 {
                        crate::memory::copy_bytes_to_c_buf(
                            &mut wlv.extra,
                            transchar_buf(wp.buf_opt().as_deref(), mb_c).to_bytes(),
                        );

                        if wlv.n_extra == 0 {
                            wlv.n_extra = opt_vars().dy_escape_width.get() - 1;
                        }
                        if (opt_vars().dy_flags.get() & K_OPT_DY_FLAG_UHEX) != 0
                            && wp.w_onebuf_opt.wo_rl.get()
                        {
                            rl_mirror_ascii(&mut wlv.extra);
                        }
                        wlv.sc_extra = 0;
                        wlv.sc_final = 0;

                        mb_schar = schar_from_ascii(wlv.extra[0]);

                        mb_schar_width = 1;
                        if wp.w_onebuf_opt.wo_lbr.get() {
                            with_extra_buf((wlv.n_extra as usize) + 1, |extra_slice| {
                                extra_slice[..wlv.n_extra as usize].fill(b' ');

                                let rem_len = crate::memory::bytes_until_nul(&wlv.extra)
                                    .len()
                                    .saturating_sub(1);
                                extra_slice[..rem_len].copy_from_slice(&wlv.extra[1..1 + rem_len]);
                                extra_slice[wlv.n_extra as usize] = 0;

                                wlv.set_p_extra(crate::memory::bytes_as_cstr(extra_slice));
                            });
                        } else {
                            wlv.n_extra = opt_vars().dy_escape_width.get() - 1;
                            let extra_cstr = &crate::memory::bytes_as_cstr(&wlv.extra)[1..];
                            let extra_owned = extra_cstr.to_owned();
                            wlv.set_p_extra_owned(extra_owned);
                        }
                        wlv.n_attr = wlv.n_extra + 1;
                        wlv.extra_attr = win_hl_attr(wp, HLF_8);
                        saved_attr2 = wlv.char_attr;
                    } else if g_st().visual_global.borrow().active
                        && (g_st().visual_global.borrow().mode == CTRL_V_CONST
                            || g_st().visual_global.borrow().mode == b'v' as i32)
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

                if conceal_enabled
                    && !(extconceal && did_decrement_ptr)
                    && (conceal_reflow
                        || (syntax_flags & HL_CONCEAL) != 0
                        || has_match_conc > 0
                        || decor_conceal > 0)
                {
                    let syntax_conceal = (syntax_flags & HL_CONCEAL) != 0;
                    wlv.char_attr = conceal_attr;
                    let decor_cchar = !(has_match_conc != 0 && match_conc != 0)
                        && decor_conceal != 0
                        && g_st().decor_state.borrow().conceal_char != 0;
                    let cchar = if has_match_conc != 0 && match_conc != 0 {
                        schar_from_char(match_conc)
                    } else if decor_cchar {
                        g_st().decor_state.borrow().conceal_char
                    } else if syntax_conceal && syn_get_sub_char() != i32::from(NUL) {
                        schar_from_char(syn_get_sub_char())
                    } else {
                        0
                    };
                    let is_start = (prev_syntax_id != syntax_seqnr && syntax_conceal)
                        || has_match_conc > 1
                        || decor_conceal > 1;
                    let replacement = if conceal_reflow {
                        extconceal_char
                    } else {
                        conceal_replacement(wp, is_start, cchar)
                    };
                    if replacement != 0 {
                        if extconceal && conceal_lbr_body > 0 {
                            wlv.n_extra = conceal_lbr_body - 1;
                        } else if mb_schar_width > 1 {
                            wlv.n_extra += 1;
                        }

                        mb_schar = replacement;
                        if (decor_cchar
                            || (conceal_reflow && g_st().decor_state.borrow().conceal_char != 0))
                            && g_st().decor_state.borrow().conceal_attr != 0
                        {
                            wlv.char_attr = g_st().decor_state.borrow().conceal_attr;
                        } else if conceal_reflow && extconceal_attr != 0 {
                            wlv.char_attr = extconceal_attr;
                        }

                        mb_schar_width = schar_cells(mb_schar);
                        if conceal_reflow
                            || (decor_conceal > 0
                                && g_st().decor_state.borrow().conceal_slot_i >= 0)
                        {
                            conceal_replacement_extra = mb_schar_width - 1;
                        }

                        prev_syntax_id = syntax_seqnr;

                        if conceal_reflow {
                            wlv.extconceal_off += wlv.n_extra - conceal_replacement_extra;
                        } else {
                            if wlv.n_extra > 0 {
                                wlv.vcol_off_co += wlv.n_extra;
                            }
                            wlv.vcol += wlv.n_extra;
                            if is_wrapped && wlv.n_extra > 0 {
                                wlv.boguscols += wlv.n_extra;
                                wlv.col += wlv.n_extra;
                            }
                        }
                        wlv.n_extra = if extconceal { conceal_lbr_tail } else { 0 };
                        if wlv.n_extra > 0 {
                            wlv.sc_extra = schar_from_ascii(b' ');
                            wlv.sc_final = 0;
                        }
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

            let skip_cells = if conceal_reflow { 0 } else { wlv.skip_cells };
            if !did_wcol
                && wlv.filler_todo <= 0
                && in_curline
                && conceal_cursor_line(wp)
                && (wlv.vcol + skip_cells >= cursor_vcol || mb_schar == 0)
            {
                wp.w_wcol.set(wlv.col - wlv.boguscols);
                wp.w_wcol_conceal_off.set(wlv.vcol_off_co + skip_cells);
                if wlv.vcol + skip_cells < cursor_vcol {
                    wp.w_wcol
                        .set(wp.w_wcol.get() + (cursor_vcol - wlv.vcol - skip_cells));
                }
                wp.w_wrow.set(wlv.row);
                did_wcol = true;
                wp.w_valid
                    .set(wp.w_valid.get() | (VALID_WCOL | VALID_WROW | VALID_VIRTCOL));
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
                && wp.w_onebuf_opt.wo_list.get()
                && (if wp.w_onebuf_opt.wo_wrap.get() {
                    wp.w_skipcol.get() > 0 && wlv.row == 0
                } else {
                    wp.w_leftcol.get() > 0
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
                        debug_assert!(wlv.p_extra().is_some());
                        n_extra_next = wlv.n_extra;
                        extra_attr_next = wlv.extra_attr;
                        wlv.n_attr = (wlv.n_attr + 1).max(2);
                    } else {
                        wlv.n_attr = 2;
                    }
                    wlv.n_extra = 1;
                    wlv.extra_attr = win_hl_attr(wp, HLF_AT);
                }
                mb_schar = wp.w_p_lcs_chars.borrow().prec;
                mb_schar_width = schar_get_single_width(mb_schar);
                saved_attr3 = wlv.char_attr;
                wlv.char_attr = win_hl_attr(wp, HLF_AT);
                n_attr3 = 1;
            }

            if mb_schar == 0 && eol_hl_off == 0 {
                let prevcol_hl_flag = get_prevcol_hl_flag(
                    wp,
                    &mut g_st().screen_search_hl.borrow_mut(),
                    (((ptr as isize) - (line as isize)) as Colnr) - 1,
                );

                if lcs_eol_todo
                    && ((area_attr != 0
                        && wlv.vcol == wlv.fromcol
                        && (g_st().visual_global.borrow().mode != CTRL_V_CONST
                            || lnum == g_st().visual_global.borrow().start.lnum
                            || lnum == curwin_mut().w_cursor.get().lnum))
                        || prevcol_hl_flag)
                {
                    let n = if wlv.col >= view_width { -1 } else { 0 };
                    if n != 0 {
                        wlv.off += n;
                        wlv.col += n;
                    } else {
                        g_st().linebuf_char.borrow_mut()[wlv.off as usize] = schar_from_ascii(b' ');
                    }
                    if area_attr == 0 && !has_fold {
                        get_search_match_hl(
                            wp,
                            &mut g_st().screen_search_hl.borrow_mut(),
                            ((ptr as isize) - (line as isize)) as Colnr,
                            &mut wlv.char_attr,
                        );
                    }

                    let eol_attr = if wlv.cul_attr != 0 {
                        hl_combine_attr(wlv.cul_attr, wlv.char_attr)
                    } else {
                        wlv.char_attr
                    };

                    g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = eol_attr;
                    g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = wlv.vcol;
                    wlv.col += 1;
                    wlv.off += 1;
                    wlv.vcol += 1;
                    eol_hl_off = 1;
                }
            }

            if mb_schar == 0 {
                if in_curline && wlv.col >= view_width {
                    wp.w_wcol.set(wp.w_wcol.get().min(view_width - 1));
                }
                wlv.vcol = wlv.vcol.max(start_vcol + wlv.col - win_col_off(wp));
                wlv.col -= wlv.boguscols;
                wlv.boguscols = 0;

                let hlc = vcol_hlc(&wlv);
                advance_color_col(&mut wlv, hlc);

                let eol_skip = if eol_hl_off == 0 && (lcs_eol_todo || wp.w_onebuf_opt.wo_list.get())
                {
                    1
                } else {
                    0
                };

                if has_decor {
                    decor_redraw_eol(
                        wp,
                        &mut g_st().decor_state.borrow_mut(),
                        &mut wlv.line_attr,
                        wlv.col + eol_skip,
                    );
                }

                for i in wlv.col..view_width {
                    g_st().linebuf_vcol.borrow_mut()[(wlv.off + (i - wlv.col)) as usize] =
                        wlv.vcol + (i - wlv.col);
                }

                if (wp.w_onebuf_opt.wo_cuc.get()
                    && wp.w_virtcol.get() >= vcol_hlc(&wlv) - eol_hl_off
                    && i64::from(wp.w_virtcol.get())
                        < i64::from(view_width) * i64::from(wlv.row - startrow + 1)
                            + i64::from(start_vcol)
                    && lnum != wp.w_cursor.get().lnum)
                    || !wlv.color_cols.is_empty()
                    || wlv.line_attr_lowprio != 0
                    || wlv.line_attr != 0
                    || wlv.diff_hlf != 0
                    || !wp.buf_mut().terminal.get().is_null()
                {
                    let mut rightmost_vcol = get_rightmost_vcol(wp, wlv.color_cols);
                    let cuc_attr = win_hl_attr(wp, HLF_CUC);
                    let mc_attr = win_hl_attr(wp, HLF_MC);

                    if wlv.diff_hlf == HLF_TXD || wlv.diff_hlf == HLF_TXA {
                        wlv.diff_hlf = HLF_CHD;
                        set_line_attr_for_diff(wp, &mut wlv);
                    }

                    let diff_attr = if wlv.diff_hlf != 0 {
                        win_hl_attr(wp, wlv.diff_hlf)
                    } else {
                        0
                    };

                    let base_attr = hl_combine_attr(wlv.line_attr_lowprio, diff_attr);
                    if base_attr != 0
                        || wlv.line_attr != 0
                        || !wp.buf_mut().terminal.get().is_null()
                    {
                        rightmost_vcol = i32::MAX;
                    }

                    while wlv.col < view_width {
                        g_st().linebuf_char.borrow_mut()[wlv.off as usize] = schar_from_ascii(b' ');

                        let hlc = vcol_hlc(&wlv);
                        advance_color_col(&mut wlv, hlc);

                        let mut col_attr = base_attr;

                        if wp.w_onebuf_opt.wo_cuc.get()
                            && vcol_hlc(&wlv) == wp.w_virtcol.get()
                            && lnum != wp.w_cursor.get().lnum
                        {
                            col_attr = hl_combine_attr(col_attr, cuc_attr);
                        } else if wlv
                            .color_cols
                            .first()
                            .is_some_and(|&cc| cc == vcol_hlc(&wlv))
                        {
                            col_attr = hl_combine_attr(col_attr, mc_attr);
                        }

                        if !wp.buf_mut().terminal.get().is_null()
                            && (wlv.vcol as usize) < TERM_ATTRS_MAX
                        {
                            col_attr = hl_combine_attr(col_attr, term_attrs[wlv.vcol as usize]);
                        }

                        col_attr = hl_combine_attr(col_attr, wlv.line_attr);

                        g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = col_attr;
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
                        buf.borrow_mut(),
                        win_col_offset,
                        &fold_vt,
                        K_HL_MODE_COMBINE,
                        view_width,
                        0,
                        0,
                        false,
                    );
                }

                draw_virt_text(wp, buf.borrow_mut(), win_col_offset, &mut wlv.col, wlv.row);

                wlv_put_linebuf(wp, &wlv, wlv.col, true, bg_attr, SLF_INC_VCOL);
                wlv.row += 1;

                if in_curline {
                    curwin_mut().w_cline_row.set(startrow);
                    curwin_mut().w_cline_height.set(wlv.row - startrow);
                    curwin_mut().w_cline_folded.set(has_fold);
                    curwin_mut()
                        .w_valid
                        .set(curwin_mut().w_valid.get() | (VALID_CHEIGHT | VALID_CROW));
                }

                break;
            }

            let lcs_ext = get_lcs_ext(wp);
            if lcs_ext != 0 && wlv.filler_todo <= 0 && wlv.col == view_width - 1 && !has_foldtext {
                if has_decor && ptr_byte!(ptr) == 0 && lcs_eol == 0 && lcs_eol_todo {
                    decor_redraw_col(
                        wp,
                        ((ptr as isize) - (line as isize)) as Colnr,
                        -1,
                        false,
                        &mut g_st().decor_state.borrow_mut(),
                        decor_provider_end_col - 1,
                    );
                }
                if ptr_byte!(ptr) != 0
                    || (lcs_eol > 0 && lcs_eol_todo)
                    || (wlv.n_extra > 0
                        && (wlv.sc_extra != 0
                            || wlv.p_extra().is_some_and(|s| !s.to_bytes().is_empty())))
                    || (may_have_inline_virt
                        && has_more_inline_virt(&wlv, (ptr as isize) - (line as isize)))
                {
                    mb_schar = lcs_ext;
                    mb_schar_width = schar_get_single_width(mb_schar);
                    wlv.char_attr = win_hl_attr(wp, HLF_AT);
                }
            }

            let hlc = vcol_hlc(&wlv);
            advance_color_col(&mut wlv, hlc);

            vcol_save_attr = -1;
            if !lnum_in_visual_area && search_attr == 0 && area_attr == 0 && wlv.filler_todo <= 0 {
                if wp.w_onebuf_opt.wo_cuc.get()
                    && vcol_hlc(&wlv) == wp.w_virtcol.get()
                    && lnum != wp.w_cursor.get().lnum
                {
                    vcol_save_attr = wlv.char_attr;
                    wlv.char_attr = hl_combine_attr(win_hl_attr(wp, HLF_CUC), wlv.char_attr);
                } else if wlv
                    .color_cols
                    .first()
                    .is_some_and(|&cc| cc == vcol_hlc(&wlv))
                {
                    vcol_save_attr = wlv.char_attr;
                    wlv.char_attr = hl_combine_attr(win_hl_attr(wp, HLF_MC), wlv.char_attr);
                }
            }

            if wlv.filler_todo <= 0 {
                let mut low = wlv.line_attr_lowprio;
                let mut high = wlv.char_attr;

                if wlv.line_attr_lowprio != 0 {
                    let line_ae = syn_attr2entry(wlv.line_attr_lowprio);
                    let char_ae = syn_attr2entry(wlv.char_attr);
                    let mut win_normal_bg = g_st().normal_bg.get();
                    let mut win_normal_cterm_bg = g_st().cterm_normal_bg_color.get();

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
                g_st().linebuf_char.borrow_mut()[wlv.off as usize] = mb_schar;
                if multi_attr != 0 {
                    g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = multi_attr;
                    multi_attr = 0;
                } else {
                    g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = wlv.char_attr;
                }

                g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = wlv.vcol;

                if mb_schar_width > 1 {
                    wlv.off += 1;
                    wlv.col += 1;
                    g_st().linebuf_char.borrow_mut()[wlv.off as usize] = 0;
                    let prev_attr = g_st().linebuf_attr.borrow()[(wlv.off - 1) as usize];
                    g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = prev_attr;

                    if conceal_replacement_extra > 0 && !conceal_reflow {
                        g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = wlv.vcol;
                        wlv.vcol_off_co -= conceal_replacement_extra;
                    } else {
                        wlv.vcol += 1;
                        g_st().linebuf_vcol.borrow_mut()[wlv.off as usize] = wlv.vcol;
                    }

                    if wlv.tocol == wlv.vcol {
                        wlv.tocol += 1;
                    }
                }
                wlv.off += 1;
                wlv.col += 1;
            } else if wp.w_onebuf_opt.wo_cole.get() > 0 && is_concealing && conceal_reflow {
                wlv.skip_cells -= 1;
                wlv.extconceal_off += 1 + i32::from(mb_schar_width > 1) + wlv.n_extra.max(0);
                wlv.n_extra = 0;
                wlv.n_attr = 0;
                cell_hidden = true;
            } else if wp.w_onebuf_opt.wo_cole.get() > 0 && is_concealing {
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

            if wlv.filler_todo <= 0 && !cell_hidden {
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
                        wp,
                        ((ptr as isize) - (line as isize)) as Colnr,
                        -3,
                        false,
                        &mut g_st().decor_state.borrow_mut(),
                        decor_provider_end_col - 1,
                    );
                    decor_need_recheck = true;
                    if conceal_enabled && g_st().decor_state.borrow().conceal_slot_i >= 0 {
                        if disp_len < 0 {
                            let col = (ptr as isize) - (line as isize);
                            disp_len = linetabsize_disp_eol(wp, lnum);
                            line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
                            line = line_cstr.as_ptr().cast_mut();
                            ptr = line.wrapping_offset(col);
                        }
                        if wlv.vcol >= disp_len {
                            ptr = ptr.wrapping_add(ptr_cstr!(ptr).to_bytes().len());
                        }
                    }
                } else if !is_wrapped {
                    decor_recheck_draw_col(-1, true, &mut g_st().decor_state.borrow_mut());
                    decor_redraw_col(
                        wp,
                        MAXCOL,
                        -1,
                        true,
                        &mut g_st().decor_state.borrow_mut(),
                        decor_provider_end_col - 1,
                    );
                }
            }
        }

        // end_check:
        if wlv.col >= view_width
            && (!has_foldtext || wlv.filler_todo > 0)
            && (wlv.col <= leftcols_width
                || ptr_byte!(ptr) != 0
                || wlv.filler_todo > 0
                || (wp.w_onebuf_opt.wo_list.get()
                    && wp.w_p_lcs_chars.borrow().eol != 0
                    && lcs_eol_todo)
                || (wlv.n_extra != 0
                    && (wlv.sc_extra != 0
                        || wlv.p_extra().is_some_and(|s| !s.to_bytes().is_empty())))
                || (may_have_inline_virt
                    && has_more_inline_virt(&wlv, (ptr as isize) - (line as isize))))
        {
            let grid_width = wp.w_grid.target.get().borrow().cols.get();
            let wrap = is_wrapped
                && wlv.filler_todo <= 0
                && lcs_eol_todo
                && wlv.row != endrow - 1
                && view_width == grid_width
                && !wp.w_onebuf_opt.wo_rl.get();

            let mut draw_col = wlv.col - wlv.boguscols;

            for i in draw_col..view_width {
                g_st().linebuf_vcol.borrow_mut()[(wlv.off + (i - draw_col)) as usize] =
                    wlv.vcol - 1;
            }

            if wlv.boguscols != 0 && (wlv.line_attr_lowprio != 0 || wlv.line_attr != 0) {
                let attr = hl_combine_attr(wlv.line_attr_lowprio, wlv.line_attr);
                while draw_col < view_width {
                    g_st().linebuf_char.borrow_mut()[wlv.off as usize] =
                        schar_from_char(b' ' as i32);
                    g_st().linebuf_attr.borrow_mut()[wlv.off as usize] = attr;
                    wlv.off += 1;
                    draw_col += 1;
                }
            }

            if has_virt_line {
                let vl = &virt_lines[virt_line_index as usize];
                let overflow = vl.overflow;
                let virt_row_scroll = overflow == K_VLOVERFLOW_SCROLL
                    || (overflow == K_VLOVERFLOW_AUTO && !wp.w_onebuf_opt.wo_wrap.get());
                if virt_row_scroll {
                    virt_line_skip_cells = wp.w_leftcol.get();
                }

                draw_virt_text_item(
                    buf.borrow_mut(),
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
                draw_virt_text(wp, buf.borrow_mut(), win_col_offset, &mut draw_col, wlv.row);
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
                let current_grid = grid_adjust(&wp.w_grid, &mut current_row, &mut dummy_col);

                let next_row_off = current_grid.line_offset.borrow()[(current_row + 1) as usize];
                current_grid.attrs.borrow_mut()[next_row_off] = -1;
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
                    wp.w_view_height.get(),
                    HLF_AT,
                );
                set_empty_rows(wp, wlv.row);
                wlv.row = endrow;
            }

            if wlv.row == endrow {
                wlv.row += 1;
                break;
            }

            win_line_start(wp, &mut wlv);
            draw_cols = true;

            lcs_prec_todo = wp.w_p_lcs_chars.borrow().prec;
            if wlv.filler_todo <= 0 {
                wlv.need_showbreak = true;
            }
            if statuscol.draw
                && crate::strings::vim_haschr(&opt_vars().p_cpo.as_cstr(), K_CPO_NUMCOL)
                && wlv.row > startrow + wlv.filler_lines
            {
                statuscol.draw = false;
            }
            wlv.filler_todo -= 1;
            has_virt_line = false;
            virt_line_flags = 0;
            if wlv.filler_todo == 0 && (wp.w_botfill.get() || !draw_text) {
                break;
            }
        }
    }

    drop(fold_vt);
    drop(virt_lines);
    wlv.row
}

fn wlv_put_linebuf(
    wp: &WinInfoInfo,
    wlv: &Winlinevars,
    mut endcol: i32,
    clear_end: bool,
    bg_attr: i32,
    mut flags: i32,
) {
    let mut startcol: i32 = 0;
    let mut clear_width = if clear_end {
        wp.w_view_width.get()
    } else {
        endcol
    };

    debug_assert!((flags & SLF_RIGHTLEFT) == 0);
    if wp.w_onebuf_opt.wo_rl.get() {
        linebuf_mirror(
            &mut startcol,
            &mut endcol,
            &mut clear_width,
            wp.w_view_width.get(),
        );
        flags |= SLF_RIGHTLEFT;
    }

    if wlv.row == 0
        && wp.w_skipcol.get() > 0
        && get_showbreak_value(wp).is_empty()
        && !(wp.w_onebuf_opt.wo_list.get() && wp.w_p_lcs_chars.borrow().prec != 0)
    {
        let mut off: i32 = 0;
        if wp.w_onebuf_opt.wo_nu.get() && wp.w_onebuf_opt.wo_rnu.get() {
            while off < wp.w_view_width.get()
                && ascii_isdigit(schar_get_ascii(g_st().linebuf_char.borrow()[off as usize]))
            {
                off += 1;
            }
        }

        for _ in 0..3 {
            if off >= wp.w_view_width.get() {
                break;
            }
            if off + 1 < wp.w_view_width.get()
                && g_st().linebuf_char.borrow()[(off + 1) as usize] == 0
            {
                g_st().linebuf_char.borrow_mut()[(off + 1) as usize] = schar_from_ascii(b' ');
            }
            g_st().linebuf_char.borrow_mut()[off as usize] = schar_from_ascii(b'<');
            g_st().linebuf_attr.borrow_mut()[off as usize] = hl_attr(HLF_AT);
            off += 1;
        }
    }

    let mut row = wlv.row;
    let mut coloff: i32 = 0;
    let g = grid_adjust(&wp.w_grid, &mut row, &mut coloff);
    grid_put_linebuf(
        &g,
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

fn decor_providers_setup(
    rows_to_draw: i32,
    draw_from_line_start: bool,
    lnum: Linenr,
    col: Colnr,
    wp: &WinInfoInfo,
) -> i32 {
    let rem_vcols = if wp.w_onebuf_opt.wo_wrap.get() {
        let width = wp.w_view_width.get() - win_col_off(wp);
        let width2 = width + win_col_off2(wp);

        let first_row_width = if draw_from_line_start { width } else { width2 };
        first_row_width + (rows_to_draw - 1) * width2
    } else {
        wp.w_view_width.get() - win_col_off(wp)
    };

    decor_providers_invoke_line(wp, lnum - 1);
    validate_virtcol(wp);

    invoke_range_next(wp, lnum, col, rem_vcols + 1)
}

fn invoke_range_next(wp: &WinInfoInfo, lnum: i32, begin_col: Colnr, mut col_off: Colnr) -> i32 {
    let line_cstr = ml_get_buf(&wp.buf_mut(), lnum);
    let line_bytes = line_cstr.to_bytes();
    let line_len = ml_get_buf_len(&wp.buf_mut(), lnum);
    col_off = col_off.max(1);

    if col_off <= line_len - begin_col {
        let mut end_col = begin_col + col_off;
        end_col += crate::mbyte::mb_off_next(line_bytes, end_col as usize);
        decor_providers_invoke_range(wp, lnum - 1, begin_col, lnum - 1, end_col);
        validate_virtcol(wp);
        end_col
    } else {
        decor_providers_invoke_range(wp, lnum - 1, begin_col, lnum, 0);
        validate_virtcol(wp);
        i32::MAX
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
        assert!(ascii_iswhite(b' '));
        assert!(ascii_iswhite(b'\t'));
        assert!(!ascii_iswhite(b'a'));
        assert!(ascii_isdigit(b'5' as i32));
        assert!(!ascii_isdigit(b'x' as i32));
    }
}
