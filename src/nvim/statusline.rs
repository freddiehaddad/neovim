//! Statusline, winbar, ruler, tabline, and statuscolumn rendering (`src/nvim/statusline.c`).

use crate::api::private::helpers::{
    add_c, api_free_array, arena_array, arena_dict, arena_string, cstr_as_string, put_c,
};
use crate::api::private::validate::{Array, Object, ObjectData, ObjectType};
use crate::buffer::{
    FileBuf, K_UIMESSAGES, append_arg_number, bt_quickfix, buf_spname, calc_percentage, get_rel_pos,
};
use crate::charset::{
    EOL_MAC, ascii_isdigit, getdigits_int, skipdigits, trans_characters, transstr_buf,
    vim_strnsize, vim_strsize,
};
use crate::context::ctx_saved_curwin;
use crate::cursor::{Colnr, Linenr};
use crate::digraph::get_keymap_str;
use crate::drawline::{HLF_CLF, HLF_FC, MAX_STCWIDTH, fill_foldcolumn, use_cursor_line_highlight};
use crate::drawscreen::{HLF_C, compute_foldcolumn, redrawing, w_endcol, w_endrow};
use crate::eval::eval_to_string_safe;
use crate::eval::typval::{TypVal, TypvalVval, VarType, Varnumber};
use crate::eval::vars::{
    VV_LNUM, VV_RELNUM, VV_VIRTNUM, do_unlet, get_vim_var_nr, set_internal_string_var, set_var,
    set_vim_var_nr,
};
use crate::globals::*;
use crate::grid::{
    Schar, ScreenGrid, grid_adjust, grid_line_fill, grid_line_flush, grid_line_put_schar,
    grid_line_puts, grid_line_start, schar_from_ascii, schar_get, schar_get_adv, schar_len,
    screengrid_line_start,
};
use crate::highlight::{hl_attr, hl_combine_attr, win_hl_attr};
use crate::highlight_group::{syn_id2attr, syn_name2id_len};
use crate::keycodes::NL;
use crate::mbyte::{ptr2cells, utf_ptr2char, utfc_ptr2len};
use crate::memline::{ml_find_line_or_offset, ml_get_buf, ml_get_buf_len};
use crate::memory::{Arena, arena_finish, arena_mem_free, xstrlcpy};
use crate::message::HLF_MSG;
use crate::message::msg_clr_eos;
use crate::mouse::{
    K_STL_CLICK_DISABLED, K_STL_CLICK_FUNC_RUN, K_STL_CLICK_TAB_CLOSE, K_STL_CLICK_TAB_SWITCH,
    StlClickDefinition,
};
use crate::normal::showcmd_update_clear_state;
use crate::option::{
    K_OPT_STATUSCOLUMN, K_OPT_STATUSLINE, K_OPT_WINBAR, find_option, get_fileformat,
    get_option_default, optval_free_read, set_option_direct, was_set_insecurely,
};
use crate::option_vars::*;
use crate::optionstr::{
    CharBuf, OptIndex, STL_ALL, STL_HIGHLIGHT, STL_SEPARATE, STL_TRUNCMARK, STL_USER_HL,
};
use crate::os::env::home_replace;
use crate::path::{path_tail, shorten_dir};
use crate::sign::describe_sign_text;
use crate::strings::{tr, vim_snprintf, vim_snprintf_safelen};
use crate::ui::{ui_call_msg_ruler, ui_call_tabline_update, ui_has};
use crate::undo::buf_is_changed_fn;
use crate::window::{WinInfoInfo, global_stl_height, lastwin_nofloating, tabline_height};
use std::ptr;

use crate::ex_getln::Hlf;
pub type StlFlag = i32;
use crate::api::win_config::{FR_COL, UIExtension};

const TMPLEN: usize = 70;
const MAX_STL_EVAL_DEPTH: i32 = 100;

const K_OPT_RULERFORMAT: OptIndex = 244;
const K_OPT_TABLINE: OptIndex = 306;

pub(crate) const K_UITABLINE: UIExtension = 2;

const HLF_S: Hlf = 19;
const HLF_SNC: Hlf = 20;
const HLF_TP: Hlf = 52;
const HLF_TPS: Hlf = 53;
const HLF_TPF: Hlf = 54;
const HLF_WBR: Hlf = 65;
const HLF_WBRNC: Hlf = 66;

const STL_FILEPATH: u8 = b'f';
const STL_FULLPATH: u8 = b'F';
const STL_FILENAME: u8 = b't';
const STL_COLUMN: u8 = b'c';
const STL_VIRTCOL: u8 = b'v';
const STL_VIRTCOL_ALT: u8 = b'V';
const STL_LINE: u8 = b'l';
const STL_NUMLINES: u8 = b'L';
const STL_BUFNO: u8 = b'n';
const STL_KEYMAP: u8 = b'k';
const STL_OFFSET: u8 = b'o';
const STL_OFFSET_X: u8 = b'O';
const STL_BYTEVAL: u8 = b'b';
const STL_BYTEVAL_X: u8 = b'B';
const STL_ROFLAG: u8 = b'r';
const STL_ROFLAG_ALT: u8 = b'R';
const STL_HELPFLAG: u8 = b'h';
const STL_HELPFLAG_ALT: u8 = b'H';
const STL_FILETYPE: u8 = b'y';
const STL_FILETYPE_ALT: u8 = b'Y';
const STL_PREVIEWFLAG: u8 = b'w';
const STL_PREVIEWFLAG_ALT: u8 = b'W';
const STL_MODIFIED: u8 = b'm';
const STL_MODIFIED_ALT: u8 = b'M';
const STL_QUICKFIX: u8 = b'q';
const STL_PERCENTAGE: u8 = b'p';
const STL_ALTPERCENT: u8 = b'P';
const STL_ARGLISTSTAT: u8 = b'a';
const STL_PAGENUM: u8 = b'N';
const STL_SHOWCMD: u8 = b'S';
pub(crate) const STL_FOLDCOL: u8 = b'C';
pub(crate) const STL_SIGNCOL: u8 = b's';
const STL_VIM_EXPR: u8 = b'{';
const STL_HIGHLIGHT_COMB: u8 = b'$';
const STL_TABPAGENR: u8 = b'T';
const STL_TABCLOSENR: u8 = b'X';
const STL_CLICK_FUNC: u8 = b'@';

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StlPadding {
    PaddingNone = 0,
    PaddingLeft = 1,
    PaddingRight = 2,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumberBase {
    NumBaseDecimal = 10,
    NumBaseHexadecimal = 16,
}

#[repr(C)]
#[derive(Clone)]
pub struct StlClickRecord {
    pub def: StlClickDefinition,
    pub start: *const u8,
}

impl StlClickRecord {
    pub const fn new() -> Self {
        Self {
            def: StlClickDefinition::new(),
            start: ptr::null(),
        }
    }
}

impl Default for StlClickRecord {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StlHlrec {
    pub start: *mut u8,
    pub userhl: i32,
    pub item: StlFlag,
}

impl StlHlrec {
    pub const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            userhl: 0,
            item: 0,
        }
    }
}

impl Default for StlHlrec {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StlItemType {
    Disabled = 0,
    Normal = 1,
    NormalEmpty = 2,
    Group = 3,
    Separate = 4,
    Trunc = 5,
    Expression = 6,
    Highlight = 7,
    HighlightCombining = 8,
    HighlightScope = 9,
    HighlightSign = 10,
    HighlightFold = 11,
    TabPage = 12,
    ClickFunc = 13,
}

#[repr(C)]
#[derive(Clone)]
pub struct StlItem {
    pub start: *mut u8,
    pub cmd: Option<std::ffi::CString>,
    pub minwid: i32,
    pub maxwid: i32,
    pub type_: StlItemType,
}

impl StlItem {
    pub const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            cmd: None,
            minwid: 0,
            maxwid: 0,
            type_: StlItemType::Disabled,
        }
    }
}

impl Default for StlItem {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Foldinfo {
    pub fi_lnum: Linenr,
    pub fi_level: i32,
    pub fi_low_level: i32,
    pub fi_lines: Linenr,
}

impl Foldinfo {
    pub const fn new() -> Self {
        Self {
            fi_lnum: 0,
            fi_level: 0,
            fi_low_level: 0,
            fi_lines: 0,
        }
    }
}

impl Default for Foldinfo {
    fn default() -> Self {
        Self::new()
    }
}

use crate::arglist::ML_EMPTY;
use crate::autocmd::HLF_T;
use crate::charset::NUL;
use crate::cmdexpand::K_UIWILDMENU_CONST;
use crate::decoration::{SCL_NUM, SignTextAttrs};
use crate::keycodes::CAR;
use crate::option::{K_OPT_INVALID, OPT_LOCAL};
use crate::os::fs::MAXPATHL;
use crate::runtime::SID_ERROR;
use crate::state::MODE_INSERT;

#[repr(C)]
pub struct Statuscol {
    pub width: i32,
    pub lnum: Linenr,
    pub sign_cul_id: i32,
    pub draw: bool,
    pub hlrec: Vec<StlHlrec>,
    pub foldinfo: Foldinfo,
    pub fold_vcol: [Colnr; 9],
    pub sattrs: [SignTextAttrs; 9],
}

impl Statuscol {
    pub const fn new() -> Self {
        Self {
            width: 0,
            lnum: 0,
            sign_cul_id: 0,
            draw: false,
            hlrec: Vec::new(),
            foldinfo: Foldinfo::new(),
            fold_vcol: [0; 9],
            sattrs: [SignTextAttrs {
                text: [0; 2],
                hl_id: 0,
            }; 9],
        }
    }
}

impl Default for Statuscol {
    fn default() -> Self {
        Self::new()
    }
}

struct StatuslineState {
    did_show_ext_ruler: bool,
    redr_status_busy: bool,
    stl_expr_entered: bool,
    winbar_entered: bool,
    did_show_ruler: bool,
    stl_items_vec: Vec<StlItem>,
    stl_groupitems_vec: Vec<i32>,
    stl_hltab_vec: Vec<StlHlrec>,
    stl_tabtab_vec: Vec<StlClickRecord>,
    curitem: i32,
}

impl StatuslineState {
    const fn new() -> Self {
        Self {
            did_show_ext_ruler: false,
            redr_status_busy: false,
            stl_expr_entered: false,
            winbar_entered: false,
            did_show_ruler: false,
            stl_items_vec: Vec::new(),
            stl_groupitems_vec: Vec::new(),
            stl_hltab_vec: Vec::new(),
            stl_tabtab_vec: Vec::new(),
            curitem: 0,
        }
    }
}

static STL_STATE: GlobalState<StatuslineState> = GlobalState::new(StatuslineState::new());

#[inline]
fn stl_st() -> &'static mut StatuslineState {
    unsafe { &mut *STL_STATE.as_ptr() }
}

/// Redraw the status line of window `wp`.
pub fn win_redr_status(wp: &mut WinInfoInfo) {
    let is_stl_global = global_stl_height() > 0;

    if stl_st().redr_status_busy || (g_st().wild_menu_showing != 0 && !ui_has(K_UIWILDMENU_CONST)) {
        return;
    }

    stl_st().redr_status_busy = true;
    wp.w_redr_status = false;

    let mut stl_curwin = ctx_saved_curwin();
    if stl_curwin.is_null() {
        stl_curwin = curwin_ptr();
    }
    let is_stl_curwin = ptr::eq(wp, stl_curwin);

    if wp.w_status_height == 0 && !(is_stl_global && is_stl_curwin) {
        g_st().redraw_cmdline = true;
    } else if !redrawing() {
        wp.w_redr_status = true;
    } else if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_stl)
        || !wp.w_floating
        || (is_stl_global && is_stl_curwin)
    {
        win_redr_stl_expr(&raw mut *wp, false, false, false);
    }

    let mut group: Hlf = HLF_C;
    if wp.w_vsep_width != 0 && wp.w_status_height != 0 && redrawing() {
        let fillchar = if stl_connected(wp) {
            fillchar_status(&mut group, wp)
        } else {
            wp.w_p_fcs_chars.vert
        };
        let attr = win_hl_attr(&mut *wp, group);
        grid_line_start(&raw mut g_st().default_gridview, w_endrow(&*wp));
        grid_line_put_schar(w_endcol(wp), fillchar, attr);
        grid_line_flush();
    }
    stl_st().redr_status_busy = false;
}

pub fn get_trans_bufname(buf: &mut FileBuf) {
    let name_buff = g_st().name_buff.as_mut_ptr();
    if let Some(sp) = buf_spname(&*buf) {
        xstrlcpy(name_buff, sp, MAXPATHL);
    } else {
        home_replace(Some(&*buf), buf.b_fname, name_buff, MAXPATHL, true);
    }
    trans_characters(&mut g_st().name_buff);
}

/// Return true if the status line of window `wp` is connected to the status
/// line of the window right of it. If not, then it's a vertical separator.
pub fn stl_connected(wp: &mut WinInfoInfo) -> bool {
    unsafe {
        let mut fr = wp.w_frame;
        while !(*fr).fr_parent.is_null() {
            if (*(*fr).fr_parent).fr_layout == FR_COL {
                if !(*fr).fr_next.is_null() {
                    break;
                }
            } else if !(*fr).fr_next.is_null() {
                return true;
            }
            fr = (*fr).fr_parent;
        }
        false
    }
}

/// Clear status line, window bar or tab page line click definition table.
pub fn stl_clear_click_defs(defs: &mut [StlClickDefinition]) {
    defs.fill(StlClickDefinition::default());
}

/// Allocate or resize the click definitions array if needed.
pub fn stl_alloc_click_defs(
    defs: &mut Vec<StlClickDefinition>,
    width: i32,
) -> *mut StlClickDefinition {
    if defs.len() < width as usize {
        defs.resize(width as usize, StlClickDefinition::default());
    }
    defs.as_mut_ptr()
}

/// Fill the click definitions array if needed.
pub fn stl_fill_click_defs(
    click_defs: *mut StlClickDefinition,
    click_recs: *mut StlClickRecord,
    buf: impl crate::strings::IntoCCharPtr,
    width: i32,
    tabline: bool,
) {
    let mut buf: *const u8 = buf.into_c_char_ptr().cast();
    unsafe {
        if click_defs.is_null() {
            return;
        }

        let mut col: i32 = 0;
        let mut len: i32 = 0;

        let mut cur_click_def = StlClickDefinition {
            type_: K_STL_CLICK_DISABLED,
            tabnr: 0,
            func: None,
        };
        let mut i: usize = 0;
        while !(*click_recs.wrapping_add(i)).start.is_null() {
            len += vim_strnsize(
                buf.cast_mut().cast(),
                crate::memory::ptr_offset_from((*click_recs.wrapping_add(i)).start, buf) as i32,
            );
            debug_assert!(len <= width);
            if col < len {
                while col < len {
                    *click_defs.wrapping_add(col as usize) = cur_click_def.clone();
                    col += 1;
                }
            }
            buf = (*click_recs.wrapping_add(i)).start;
            cur_click_def = (*click_recs.wrapping_add(i)).def.clone();
            if !tabline
                && !(cur_click_def.type_ == K_STL_CLICK_DISABLED
                    || cur_click_def.type_ == K_STL_CLICK_FUNC_RUN)
            {
                cur_click_def.type_ = K_STL_CLICK_DISABLED;
            }
            i += 1;
        }
        if col < width {
            while col < width {
                *click_defs.wrapping_add(col as usize) = cur_click_def.clone();
                col += 1;
            }
        }
    }
}

/// Redraws the statusline, winbar, ruler or tabline.
fn win_redr_stl_expr(wp: *mut WinInfoInfo, draw_winbar: bool, draw_ruler: bool, ui_event: bool) {
    unsafe {
        let mut col: i32 = 0;
        let attr: i32;
        let mut row: i32 = 0;
        let maxwidth: i32;
        let mut group: Hlf = 0;
        let fillchar: Schar;
        let mut buf: [u8; MAXPATHL] = [0; MAXPATHL];
        let mut transbuf: [u8; MAXPATHL] = [0; MAXPATHL];
        let mut stl: *mut u8;
        let opt_idx: OptIndex;
        let mut opt_scope: i32 = 0;
        let mut hltab: *mut StlHlrec = ptr::null_mut();
        let click_defs: *mut StlClickDefinition;
        let is_stl_global = global_stl_height() > 0;

        let mut grid: *mut ScreenGrid = if !wp.is_null() && (*wp).w_floating && !is_stl_global {
            &raw mut (*wp).w_grid_alloc
        } else {
            &raw mut g_st().default_grid
        };

        if !is_stl_global && !wp.is_null() && (*wp).w_config.hide {
            return;
        }

        if stl_st().stl_expr_entered {
            return;
        }
        stl_st().stl_expr_entered = true;

        let save_curwin = ctx_saved_curwin();
        let restore_curwin = if !save_curwin.is_null() {
            curwin_ptr()
        } else {
            ptr::null_mut()
        };
        if !save_curwin.is_null() {
            set_curwin(save_curwin);
            g_st().curbuf = curwin_mut().w_buffer;
        }

        if wp.is_null() {
            stl = opt_vars().p_tal.cast();
            row = 0;
            fillchar = schar_from_ascii(b' ');
            group = HLF_TPF;
            attr = hl_attr(group);
            maxwidth = g_st().columns_global;
            opt_idx = K_OPT_TABLINE;
            click_defs = g_st().tab_page_click_defs.as_mut_ptr();
        } else if draw_winbar {
            let wp = &mut *wp;
            opt_idx = K_OPT_WINBAR;
            stl = if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr) {
                wp.w_onebuf_opt.wo_wbr.cast()
            } else {
                opt_vars().p_wbr.cast()
            };
            opt_scope = if !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr) {
                OPT_LOCAL
            } else {
                0
            };
            row = -1;
            col = 0;
            grid = grid_adjust(&mut wp.w_grid, &mut row, &mut col);

            if row < 0 {
                stl_st().stl_expr_entered = false;
                if !restore_curwin.is_null() {
                    set_curwin(restore_curwin);
                    g_st().curbuf = (*restore_curwin).w_buffer;
                }
                return;
            }

            fillchar = wp.w_p_fcs_chars.wbr;
            group = if std::ptr::eq(wp, curwin_ptr()) {
                HLF_WBR
            } else {
                HLF_WBRNC
            };
            attr = win_hl_attr(&mut *wp, group);
            maxwidth = wp.w_view_width;
            stl_clear_click_defs(&mut wp.w_winbar_click_defs);
            click_defs = stl_alloc_click_defs(&mut wp.w_winbar_click_defs, maxwidth);
        } else if draw_ruler && ui_event {
            stl = opt_vars().p_ruf.cast();
            opt_idx = K_OPT_RULERFORMAT;
            maxwidth = g_st().columns_global / 2;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else if draw_ruler {
            stl = opt_vars().p_ruf.cast();
            opt_idx = K_OPT_RULERFORMAT;
            if crate::memory::cstr_first_byte(stl.cast()) == b'%' {
                stl = stl.wrapping_add(1);
                if crate::memory::cstr_first_byte(stl.cast()) == b'-' {
                    stl = stl.wrapping_add(1);
                }
                if crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(stl.cast())) as i32
                    != 0
                {
                    while ascii_isdigit(crate::memory::cstr_first_char(stl.cast())) {
                        stl = stl.wrapping_add(1);
                    }
                }
                let ch = *stl;
                stl = stl.wrapping_add(1);
                if ch != b'(' {
                    stl = opt_vars().p_ruf.cast();
                }
            }
            row = g_st().rows_global - 1;
            col = g_st().ru_col.max(g_st().columns_global / 2);
            grid = grid_adjust(&mut g_st().msg_grid_adj, &mut row, &mut col);
            maxwidth = g_st().columns_global - col;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else {
            stl = if !crate::memory::cstr_is_empty((*wp).w_onebuf_opt.wo_stl) {
                (*wp).w_onebuf_opt.wo_stl.cast()
            } else {
                opt_vars().p_stl.cast()
            };
            opt_idx = K_OPT_STATUSLINE;
            opt_scope = if !crate::memory::cstr_is_empty((*wp).w_onebuf_opt.wo_stl) {
                OPT_LOCAL
            } else {
                0
            };
            if is_stl_global {
                row = g_st().rows_global - (opt_vars().p_ch as i32) - 1;
                col = 0;
                maxwidth = g_st().columns_global;
            } else if (*wp).w_status_height == 0 {
                stl_st().stl_expr_entered = false;
                if !restore_curwin.is_null() {
                    set_curwin(restore_curwin);
                    g_st().curbuf = (*restore_curwin).w_buffer;
                }
                return;
            } else if (*wp).w_floating {
                row = (*wp).w_winrow_off + (*wp).w_view_height;
                col = (*wp).w_wincol_off;
                maxwidth = (*wp).w_view_width;
            } else {
                row = w_endrow(&*wp);
                col = (*wp).w_wincol;
                maxwidth = (*wp).w_width;
            }
            fillchar = fillchar_status(&mut group, &mut *wp);
            attr = win_hl_attr(&mut *wp, group);
            stl_clear_click_defs(&mut (*wp).w_status_click_defs);
            click_defs = stl_alloc_click_defs(&mut (*wp).w_status_click_defs, maxwidth);
        }

        if maxwidth > 0 {
            let ewp = if wp.is_null() { curwin_ptr() } else { wp };
            let ewp = &mut *ewp;
            let p_crb_save = ewp.w_onebuf_opt.wo_crb;
            ewp.w_onebuf_opt.wo_crb = 0;

            let stl_cstr = crate::memory::cstr_to_cstring(stl.cast());

            let mut tabtab: *mut StlClickRecord = ptr::null_mut();
            build_stl_str_hl(
                ewp,
                CharBuf {
                    data: buf.as_mut_ptr(),
                    size: buf.len(),
                },
                stl_cstr.as_ptr().cast_mut().cast(),
                opt_idx,
                opt_scope,
                fillchar,
                maxwidth,
                &raw mut hltab,
                ptr::null_mut(),
                if !click_defs.is_null() {
                    &raw mut tabtab
                } else {
                    ptr::null_mut()
                },
                ptr::null_mut(),
            );
            stl_fill_click_defs(
                click_defs,
                tabtab,
                buf.as_ptr().cast(),
                maxwidth,
                wp.is_null(),
            );

            drop(stl_cstr);
            ewp.w_onebuf_opt.wo_crb = p_crb_save;

            let len = crate::memory::cstr_len(buf.as_ptr().cast());
            let start_col = col;

            if !ui_event {
                screengrid_line_start(&mut *grid, row, 0);
            }

            let mut p = buf.as_mut_ptr();
            let mut curattr = attr;
            let mut curgroup = group;
            let mut content = Array::new();
            let mut sp = hltab;
            loop {
                let textlen = if !(*sp).start.is_null() {
                    crate::memory::ptr_offset_from((*sp).start, p)
                } else {
                    crate::memory::ptr_offset_from(buf.as_mut_ptr().wrapping_add(len), p)
                };
                let src_p = if p >= buf.as_mut_ptr().wrapping_add(len) {
                    c"".as_ptr()
                } else {
                    p.cast_const().cast()
                };
                let tsize = transstr_buf(src_p, textlen, &mut transbuf, true);
                if !ui_event {
                    col += grid_line_puts(col, transbuf.as_ptr().cast(), tsize as i32, curattr);
                } else {
                    let mut chunk = Array::new();
                    chunk.push(Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(i64::from(curattr)),
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(crate::api::private::helpers::cbuf_to_string(
                            transbuf.as_ptr().cast(),
                            tsize,
                        )),
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::ObjectTypeInteger,
                        data: ObjectData::Integer(i64::from(curgroup)),
                    });
                    content.push(Object {
                        obj_type: ObjectType::ObjectTypeArray,
                        data: ObjectData::Array(chunk),
                    });
                }
                p = (*sp).start;

                if p.is_null() {
                    break;
                } else if (*sp).userhl == 0 {
                    curattr = attr;
                    curgroup = group;
                } else if (*sp).userhl < 0 {
                    let new_attr = syn_id2attr(-(*sp).userhl);
                    if (*sp).item == i32::from(STL_HIGHLIGHT_COMB) {
                        curattr = hl_combine_attr(curattr, new_attr);
                    } else {
                        curattr = new_attr;
                    }
                    curgroup = -(*sp).userhl;
                } else {
                    let userhl =
                        if !wp.is_null() && wp != curwin_ptr() && (*wp).w_status_height != 0 {
                            &raw const g_st().highlight_stlnc[0]
                        } else {
                            &raw const g_st().highlight_user[0]
                        };
                    let userbuf: [u8; 5] = [b'U', b's', b'e', b'r', (*sp).userhl as u8 + b'0'];
                    curattr = *userhl.wrapping_add(((*sp).userhl - 1) as usize);
                    curgroup = syn_name2id_len(userbuf.as_ptr().cast(), 5);
                }
                if curattr != attr {
                    curattr = hl_combine_attr(attr, curattr);
                }
                sp = sp.wrapping_add(1);
            }

            if opt_vars().p_sc != 0 && find_option(opt_vars().p_sloc) == opt_idx {
                showcmd_update_clear_state();
            }

            if ui_event {
                ui_call_msg_ruler(content);
                stl_st().did_show_ext_ruler = true;
                api_free_array(content);
            } else {
                let maxcol = start_col + maxwidth;
                grid_line_fill(col, maxcol, fillchar, curattr);
                grid_line_flush();
            }
        }

        stl_st().stl_expr_entered = false;

        if !restore_curwin.is_null() {
            set_curwin(restore_curwin);
            g_st().curbuf = (*restore_curwin).w_buffer;
        }
    }
}

pub fn win_redr_winbar(wp: &mut WinInfoInfo) {
    if stl_st().winbar_entered {
        return;
    }
    stl_st().winbar_entered = true;

    if wp.w_winbar_height == 0 || !redrawing() {
        // Do nothing.
    } else if !crate::memory::cstr_is_empty(opt_vars().p_wbr)
        || !crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_wbr)
    {
        win_redr_stl_expr(&raw mut *wp, true, false, false);
    }
    stl_st().winbar_entered = false;
}

pub fn redraw_ruler() {
    let wp = if !curwin_mut().w_config.hide && curwin_mut().w_status_height == 0 {
        curwin_ptr()
    } else {
        lastwin_nofloating(ptr::null_mut())
    };
    let wp = unsafe { &mut *wp };
    let is_stl_global = global_stl_height() > 0;

    let part_of_status = wp.w_status_height != 0 || is_stl_global;
    if opt_vars().p_ru == 0 || part_of_status || (opt_vars().p_ch == 0 && !ui_has(K_UIMESSAGES)) {
        if stl_st().did_show_ext_ruler && ui_has(K_UIMESSAGES) {
            ui_call_msg_ruler(Array::new());
            stl_st().did_show_ext_ruler = false;
        } else if stl_st().did_show_ruler && !ui_has(K_UIMESSAGES) {
            msg_clr_eos();
        }
        stl_st().did_show_ruler = false;
        return;
    }

    if wp.w_cursor.lnum > (*wp).buf_mut().b_ml.ml_line_count {
        return;
    }

    if wp.w_status_height == 0 && !is_stl_global && g_st().edit_submode.is_some() {
        return;
    }

    win_redr_stl_expr(wp, false, true, ui_has(K_UIMESSAGES));
    stl_st().did_show_ruler = !ui_has(K_UIMESSAGES);
}

/// Get the character to use in a status line. Get its attributes in `*group`.
pub fn fillchar_status(group: &mut Hlf, wp: &mut WinInfoInfo) -> Schar {
    if (&raw mut *wp) == curwin_ptr() {
        *group = HLF_S;
        wp.w_p_fcs_chars.stl
    } else {
        *group = HLF_SNC;
        wp.w_p_fcs_chars.stlnc
    }
}

fn ui_ext_tabline_update() {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let name_buff = g_st().name_buff.as_mut_ptr();

        let mut n_tabs: usize = 0;
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            n_tabs += 1;
            tp = (*tp).tp_next;
        }

        let mut tabs = arena_array(&mut arena, n_tabs);
        tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut tab_info = arena_dict(&mut arena, 2);
            put_c(
                &mut tab_info,
                c"tab",
                Object {
                    obj_type: ObjectType::ObjectTypeTabpage,
                    data: ObjectData::Integer(i64::from((*tp).handle)),
                },
            );

            let cwp = if tp == g_st().curtab {
                curwin_ptr()
            } else {
                (*tp).tp_curwin
            };
            get_trans_bufname((*cwp).buf_mut());
            put_c(
                &mut tab_info,
                c"name",
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(arena_string(&mut arena, cstr_as_string(name_buff))),
                },
            );

            add_c(
                &mut tabs,
                Object {
                    obj_type: ObjectType::ObjectTypeDict,
                    data: ObjectData::Dict(tab_info),
                },
            );
            tp = (*tp).tp_next;
        }

        let mut n_buffers: usize = 0;
        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_bl != 0 {
                n_buffers += 1;
            }
            buf = (*buf).b_next;
        }

        let mut buffers = arena_array(&mut arena, n_buffers);
        buf = g_st().firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_bl != 0 {
                let mut buffer_info = arena_dict(&mut arena, 2);
                put_c(
                    &mut buffer_info,
                    c"buffer",
                    Object {
                        obj_type: ObjectType::ObjectTypeBuffer,
                        data: ObjectData::Integer(i64::from((*buf).b_fnum)),
                    },
                );

                get_trans_bufname(&mut *buf);
                put_c(
                    &mut buffer_info,
                    c"name",
                    Object {
                        obj_type: ObjectType::ObjectTypeString,
                        data: ObjectData::String(arena_string(
                            &mut arena,
                            cstr_as_string(name_buff),
                        )),
                    },
                );

                add_c(
                    &mut buffers,
                    Object {
                        obj_type: ObjectType::ObjectTypeDict,
                        data: ObjectData::Dict(buffer_info),
                    },
                );
            }
            buf = (*buf).b_next;
        }

        ui_call_tabline_update(curtab_mut().handle, tabs, curbuf_mut().b_fnum, buffers);
        arena_mem_free(arena_finish(&mut arena));
    }
}

/// Draw the tab pages line at the top of the Vim window.
pub fn draw_tabline() {
    unsafe {
        let attr_fill = hl_attr(HLF_TPF);
        let attr_nosel = hl_combine_attr(attr_fill, hl_attr(HLF_TP));
        let use_sep_chars = g_st().t_colors < 8;
        let name_buff = g_st().name_buff.as_mut_ptr();

        if !crate::grid::grid_allocated(&g_st().default_grid) {
            return;
        }
        g_st().redraw_tabline = false;

        if ui_has(K_UITABLINE) {
            ui_ext_tabline_update();
            return;
        }

        if tabline_height() < 1 {
            return;
        }

        debug_assert!(g_st().tab_page_click_defs.len() >= g_st().columns_global as usize);
        stl_clear_click_defs(&mut g_st().tab_page_click_defs);

        if !crate::memory::cstr_is_empty(opt_vars().p_tal) {
            win_redr_stl_expr(ptr::null_mut(), false, false, false);
        } else {
            let mut tabcount: i32 = 0;
            let mut col: i32 = 0;
            grid_line_start(&raw mut g_st().default_gridview, 0);
            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                tabcount += 1;
                tp = (*tp).tp_next;
            }

            let tabwidth = (if tabcount > 0 {
                (g_st().columns_global - 1 + tabcount / 2) / tabcount
            } else {
                0
            })
            .max(6);

            let mut attr = attr_nosel;
            tabcount = 0;

            tp = g_st().first_tabpage;
            while !tp.is_null() {
                if col >= g_st().columns_global - 4 {
                    break;
                }

                let mut scol = col;
                let (cwp, mut wp) = if tp == g_st().curtab {
                    (curwin_ptr(), g_st().firstwin)
                } else {
                    ((*tp).tp_curwin, (*tp).tp_firstwin)
                };

                if (*tp).tp_topframe == g_st().topframe {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(&mut *cwp, HLF_TPS));
                }
                if use_sep_chars && col > 0 {
                    grid_line_put_schar(col, schar_from_ascii(b'|'), attr);
                    col += 1;
                }

                if (*tp).tp_topframe != g_st().topframe {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(&mut *cwp, HLF_TP));
                }

                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                let mut modified = false;
                let mut wincount: i32 = 0;
                while !wp.is_null() {
                    if !(*wp).w_config.focusable || (*wp).w_config.hide {
                        wincount -= 1;
                    } else if buf_is_changed_fn((*wp).buf_mut()) {
                        modified = true;
                    }
                    wp = (*wp).w_next;
                    wincount += 1;
                }

                if modified || wincount > 1 {
                    if wincount > 1 {
                        let len = vim_snprintf!(name_buff, MAXPATHL, c"%d", wincount);
                        if col + len >= g_st().columns_global - 3 {
                            break;
                        }
                        grid_line_puts(
                            col,
                            name_buff,
                            len,
                            hl_combine_attr(attr, win_hl_attr(&mut *cwp, HLF_T)),
                        );
                        col += len;
                    }
                    if modified {
                        grid_line_put_schar(col, schar_from_ascii(b'+'), attr);
                        col += 1;
                    }
                    grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                    col += 1;
                }

                let room = scol - col + tabwidth - 1;
                if room > 0 {
                    get_trans_bufname((*cwp).buf_mut());
                    shorten_dir(name_buff);
                    let mut len = vim_strsize(name_buff);
                    let mut p = name_buff;
                    while len > room {
                        len -= ptr2cells(p);
                        p = p.wrapping_add(utfc_ptr2len(p) as usize);
                    }
                    let n = g_st().columns_global - col - 1;
                    len = len.min(n);

                    grid_line_puts(col, p, -1, attr);
                    col += len;
                }
                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                tabcount += 1;
                while scol < col {
                    g_st().tab_page_click_defs.as_mut_slice()[scol as usize] = StlClickDefinition {
                        type_: K_STL_CLICK_TAB_SWITCH,
                        tabnr: tabcount,
                        func: None,
                    };
                    scol += 1;
                }

                tp = (*tp).tp_next;
            }

            for scol in col..g_st().columns_global {
                g_st().tab_page_click_defs.as_mut_slice()[scol as usize] = StlClickDefinition {
                    type_: K_STL_CLICK_TAB_SWITCH,
                    tabnr: 0,
                    func: None,
                };
            }

            let c = if use_sep_chars { b'_' } else { b' ' };
            grid_line_fill(col, g_st().columns_global, schar_from_ascii(c), attr_fill);

            if opt_vars().p_sc != 0 && crate::memory::cstr_first_byte(opt_vars().p_sloc) == b't' {
                let n = g_st().columns_global - col - i32::from(tabcount > 1) * 3;
                let sc_width = 10.min(n);

                if sc_width > 0 {
                    grid_line_puts(
                        g_st().columns_global - sc_width - i32::from(tabcount > 1) * 2,
                        (&raw const g_st().showcmd_buf).cast(),
                        sc_width,
                        attr_nosel,
                    );
                }
                showcmd_update_clear_state();
            }

            if tabcount > 1 {
                grid_line_put_schar(
                    g_st().columns_global - 1,
                    schar_from_ascii(b'X'),
                    attr_nosel,
                );
                g_st().tab_page_click_defs.as_mut_slice()[(g_st().columns_global - 1) as usize] =
                    StlClickDefinition {
                        type_: K_STL_CLICK_TAB_CLOSE,
                        tabnr: 999,
                        func: None,
                    };
            }

            grid_line_flush();
        }

        g_st().redraw_tabline = false;
    }
}

/// Build the `'statuscolumn'` string for line `lnum`.
pub fn build_statuscol_str(
    wp: &mut WinInfoInfo,
    lnum: Linenr,
    relnum: i32,
    virtnum: i32,
    buf: CharBuf,
    stcp: &mut Statuscol,
) -> i32 {
    unsafe {
        if relnum >= 0 {
            set_vim_var_nr(VV_LNUM, Varnumber::from(lnum));
            set_vim_var_nr(VV_RELNUM, Varnumber::from(relnum));
        }
        set_vim_var_nr(VV_VIRTNUM, Varnumber::from(virtnum));

        let mut clickrec: *mut StlClickRecord = ptr::null_mut();
        let mut hlrec_ptr: *mut StlHlrec = ptr::null_mut();
        let buf_data = buf.data;
        let stc = crate::memory::cstr_to_cstring(wp.w_onebuf_opt.wo_stc);
        let width = build_stl_str_hl(
            wp,
            buf,
            stc.as_ptr().cast_mut().cast(),
            K_OPT_STATUSCOLUMN,
            OPT_LOCAL,
            0,
            stcp.width,
            &raw mut hlrec_ptr,
            ptr::null_mut(),
            &raw mut clickrec,
            stcp,
        );
        drop(stc);

        stcp.hlrec.clear();
        if !hlrec_ptr.is_null() {
            let mut sp = hlrec_ptr;
            while !(*sp).start.is_null() {
                stcp.hlrec.push(*sp);
                sp = sp.wrapping_add(1);
            }
        }

        if !(*clickrec).start.is_null() {
            let clicks = wp.w_statuscol_click_defs.entry(lnum).or_default();
            let click_defs = clicks.entry(virtnum).or_default();
            stl_clear_click_defs(click_defs);
            let cdp = stl_alloc_click_defs(click_defs, width);
            stl_fill_click_defs(cdp, clickrec, buf_data.cast(), width, false);
        }

        width
    }
}

/// Truncates the statusline at the first truncation marker (`%<`) if present.
fn stl_truncate(
    width: &mut i32,
    mut minwid: i32,
    maxwid: i32,
    fillchar: Schar,
    stl_items: &mut [StlItem],
    startitem: i32,
    curitem: &mut i32,
    start_p: impl crate::strings::IntoCCharPtr,
    out_p: *mut *mut u8,
) {
    let start_p: *mut u8 = start_p.into_c_char_ptr().cast_mut().cast();
    unsafe {
        minwid = minwid.min(maxwid);

        let mut item_idx = startitem;
        let mut trunc_p: *mut u8;

        if startitem == *curitem {
            trunc_p = start_p;
        } else {
            trunc_p = stl_items[item_idx as usize].start;

            for i in startitem..*curitem {
                if stl_items[i as usize].type_ == StlItemType::Trunc {
                    trunc_p = stl_items[i as usize].start;
                    item_idx = i;
                    break;
                }
            }
        }

        if *width - vim_strsize(trunc_p.cast()) >= maxwid {
            trunc_p = start_p;
            *width = 0;
            loop {
                let char_cells = ptr2cells(trunc_p.cast());
                if *width + char_cells >= maxwid {
                    break;
                }
                *width += char_cells;
                trunc_p = trunc_p.wrapping_add(utfc_ptr2len(trunc_p.cast()) as usize);
            }

            for i in startitem..*curitem {
                if stl_items[i as usize].start > trunc_p {
                    for j in i..*curitem {
                        if stl_items[j as usize].type_ == StlItemType::ClickFunc {
                            stl_items[j as usize].cmd = None;
                        }
                    }
                    *curitem = i;
                    break;
                }
            }

            *trunc_p = b'>';
            trunc_p = trunc_p.wrapping_add(1);
            *out_p = trunc_p;
        } else {
            let mut trunc_len: i32 = 0;
            while *width >= maxwid {
                *width -= ptr2cells(trunc_p.wrapping_add(trunc_len as usize).cast());
                trunc_len += utfc_ptr2len(trunc_p.wrapping_add(trunc_len as usize).cast());
            }

            let trunc_end_p = trunc_p.wrapping_add(trunc_len as usize);
            let move_len = crate::memory::ptr_offset_from(*out_p, trunc_end_p) as usize + 1;
            ptr::copy(trunc_end_p, trunc_p.wrapping_add(1), move_len);
            *out_p = (*out_p)
                .wrapping_sub(
                    crate::memory::ptr_offset_from(trunc_end_p, trunc_p.wrapping_add(1)) as usize,
                );

            *trunc_p = b'<';

            let item_offset = (trunc_len - 1) as isize;
            for item in &mut stl_items[(item_idx as usize)..(*curitem as usize)] {
                if item.start >= trunc_end_p {
                    item.start = item.start.wrapping_offset(-item_offset);
                } else {
                    item.start = trunc_p;
                }
            }
        }

        loop {
            *width += 1;
            if *width >= minwid {
                break;
            }
            schar_get_adv(out_p, fillchar);
        }
        **out_p = 0;
    }
}

/// Expands the statusline at separation markers (`%=`) if present,
/// or otherwise according to padding choice.
fn stl_expand(
    width: &mut i32,
    target_width: i32,
    padding: StlPadding,
    remaining_capacity: i32,
    fillchar: Schar,
    stl_items: &mut [StlItem],
    startitem: i32,
    curitem: i32,
    mut start_p: *mut u8,
    out_p: *mut *mut u8,
) {
    unsafe {
        let fillchar_bytes = schar_len(fillchar) as i32;
        let mut added_cells = (target_width - *width).min(remaining_capacity / fillchar_bytes);
        let added_bytes = added_cells * fillchar_bytes;

        let mut num_separators: i32 = 0;
        for item in &stl_items[(startitem as usize)..(curitem as usize)] {
            if item.type_ == StlItemType::Separate {
                num_separators += 1;
            }
        }
        if num_separators > 0 {
            let standard_spaces = added_cells / num_separators;
            let final_spaces = added_cells - standard_spaces * (num_separators - 1);
            let mut cumulated_dislocation: isize = 0;

            for item in &mut stl_items[(startitem as usize)..(curitem as usize)] {
                item.start = item.start.wrapping_offset(cumulated_dislocation);

                if item.type_ == StlItemType::Separate {
                    num_separators -= 1;
                    let mut dislocation = if num_separators > 0 {
                        standard_spaces
                    } else {
                        final_spaces
                    };
                    dislocation *= fillchar_bytes;
                    cumulated_dislocation += dislocation as isize;

                    let start = item.start;
                    let seploc = start.wrapping_add(dislocation as usize);
                    let len = crate::memory::cstr_len(start.cast()) + 1;
                    ptr::copy(start, seploc, len);
                    let mut s = start;
                    while s < seploc {
                        schar_get_adv(&raw mut s, fillchar);
                    }
                }
            }

            *width += added_cells;
            *out_p = (*out_p).wrapping_add(added_bytes as usize);
        } else if padding == StlPadding::PaddingRight {
            while added_cells > 0 {
                schar_get_adv(out_p, fillchar);
                added_cells -= 1;
            }
            *width += added_bytes / fillchar_bytes;
        } else if padding == StlPadding::PaddingLeft {
            ptr::copy(
                start_p,
                start_p.wrapping_add(added_bytes as usize),
                crate::memory::ptr_offset_from(*out_p, start_p) as usize,
            );

            let orig_added_cells = added_cells;
            while added_cells > 0 {
                schar_get_adv(&raw mut start_p, fillchar);
                added_cells -= 1;
            }

            for item in &mut stl_items[(startitem as usize)..(curitem as usize)] {
                item.start = item.start.wrapping_add(added_bytes as usize);
            }

            *width += orig_added_cells;
            *out_p = (*out_p).wrapping_add(added_bytes as usize);
        }
    }
}

/// Build a string from the status line items in `fmt`.
#[unsafe(no_mangle)]
pub extern "C" fn build_stl_str_hl(
    wp: *mut WinInfoInfo,
    out: CharBuf,
    fmt: *mut u8,
    opt_idx: OptIndex,
    opt_scope: i32,
    mut fillchar: Schar,
    maxwidth: i32,
    hltab: *mut *mut StlHlrec,
    hltab_len: *mut usize,
    tabtab: *mut *mut StlClickRecord,
    stcp: *mut Statuscol,
) -> i32 {
    unsafe {
        let wp = &mut *wp;
        let mut buf_tmp: [u8; TMPLEN] = [0; TMPLEN];
        let mut owned_usefmt: Option<std::ffi::CString> = None;
        let mut usefmt = fmt;
        let save_redraw_not_allowed = g_st().redraw_not_allowed;
        #[allow(non_snake_case)]
        let save_KeyTyped = g_st().key_typed;
        let did_emsg_before = g_st().did_emsg;
        let name_buff = g_st().name_buff.as_mut_ptr();

        if g_st().updating_screen {
            g_st().redraw_not_allowed = true;
        }

        let items_vec_ptr = &raw mut stl_st().stl_items_vec;
        let groupitems_vec_ptr = &raw mut stl_st().stl_groupitems_vec;
        let hltab_vec_ptr = &raw mut stl_st().stl_hltab_vec;
        let tabtab_vec_ptr = &raw mut stl_st().stl_tabtab_vec;
        macro_rules! stl_items {
            () => {
                (&mut *items_vec_ptr)
            };
        }
        macro_rules! stl_groupitems {
            () => {
                (&mut *groupitems_vec_ptr)
            };
        }
        macro_rules! stl_hltab {
            () => {
                (&mut *hltab_vec_ptr)
            };
        }
        macro_rules! stl_tabtab {
            () => {
                (&mut *tabtab_vec_ptr)
            };
        }
        if (*items_vec_ptr).is_empty() {
            const INIT_LEN: usize = 20;
            stl_items!().resize(INIT_LEN, StlItem::new());
            stl_groupitems!().resize(INIT_LEN, 0);
            stl_hltab!().resize(INIT_LEN + 1, StlHlrec::new());
            stl_tabtab!().resize(INIT_LEN + 1, StlClickRecord::new());
        }

        let use_sandbox = if opt_idx != K_OPT_INVALID {
            was_set_insecurely(Some(&mut *wp), opt_idx, opt_scope) != 0
        } else {
            false
        };

        if crate::memory::cstr_first_byte(fmt.cast()) == b'%'
            && crate::memory::cstr_byte_at(fmt.cast(), 1) == b'!'
        {
            let mut tv = TypVal {
                v_type: VarType::VarNumber,
                v_lock: 0,
                vval: TypvalVval::Number(Varnumber::from(wp.handle)),
            };
            set_var(
                c"g:statusline_winid",
                "g:statusline_winid".len(),
                &mut tv,
                false,
            );

            owned_usefmt = eval_to_string_safe(fmt.wrapping_add(2).cast(), use_sandbox, false);
            usefmt = owned_usefmt
                .as_ref()
                .map_or(fmt, |s| s.as_ptr().cast_mut().cast());

            do_unlet(c"g:statusline_winid", "g:statusline_winid".len(), true);
        }

        if fillchar == 0 {
            fillchar = schar_from_ascii(b' ');
        }

        let mut lnum = wp.w_cursor.lnum;
        if lnum > (*wp).buf_mut().b_ml.ml_line_count {
            lnum = (*wp).buf_mut().b_ml.ml_line_count;
            wp.w_cursor.lnum = lnum;
        }

        let line_ptr = ml_get_buf((*wp).buf_mut(), lnum);
        let empty_line = crate::memory::cstr_is_empty(line_ptr);

        let len = ml_get_buf_len((*wp).buf_mut(), lnum);
        let byteval = if wp.w_cursor.col > len {
            wp.w_cursor.col = len;
            wp.w_cursor.coladd = 0;
            0
        } else {
            utf_ptr2char(line_ptr.wrapping_add(wp.w_cursor.col as usize))
        };

        let mut groupdepth: usize = 0;
        let mut evaldepth: i32 = 0;
        let mut curr_hl_pos: i32 = -1;
        let evalstart = stl_st().curitem;

        let mut prevchar_isflag = true;
        let mut prevchar_isitem = false;

        let mut out_p = out.data;
        let out_end_p = out_p.wrapping_add(out.size - 1);

        macro_rules! may_grow_stl_items_len {
            () => {
                if stl_st().curitem as usize == (*items_vec_ptr).len() {
                    let new_len = (*items_vec_ptr).len() * 3 / 2;
                    stl_items!().resize(new_len, StlItem::new());
                    stl_groupitems!().resize(new_len, 0);
                    stl_hltab!().resize(new_len + 1, StlHlrec::new());
                    stl_tabtab!().resize(new_len + 1, StlClickRecord::new());
                }
            };
        }

        let mut fmt_p = usefmt;
        while !crate::memory::cstr_is_empty(fmt_p.cast()) {
            may_grow_stl_items_len!();

            if crate::memory::cstr_first_byte(fmt_p.cast()) != b'%' {
                prevchar_isflag = false;
                prevchar_isitem = false;
            }

            while !crate::memory::cstr_is_empty(fmt_p.cast())
                && crate::memory::cstr_first_byte(fmt_p.cast()) != b'%'
                && out_p < out_end_p
            {
                *out_p = *fmt_p;
                out_p = out_p.wrapping_add(1);
                fmt_p = fmt_p.wrapping_add(1);
            }

            if crate::memory::cstr_is_empty(fmt_p.cast()) || out_p >= out_end_p {
                break;
            }

            fmt_p = fmt_p.wrapping_add(1);
            if crate::memory::cstr_is_empty(fmt_p.cast()) {
                break;
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b'%' {
                *out_p = *fmt_p;
                out_p = out_p.wrapping_add(1);
                fmt_p = fmt_p.wrapping_add(1);
                prevchar_isflag = false;
                prevchar_isitem = false;
                continue;
            }

            if *fmt_p == STL_SEPARATE {
                fmt_p = fmt_p.wrapping_add(1);
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Separate;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == STL_TRUNCMARK {
                fmt_p = fmt_p.wrapping_add(1);
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Trunc;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_st().curitem += 1;
                continue;
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b')' {
                fmt_p = fmt_p.wrapping_add(1);
                if groupdepth < 1 {
                    continue;
                }
                groupdepth -= 1;
                let gidx = stl_groupitems!()[groupdepth];
                if stl_items!()[gidx as usize].type_ != StlItemType::Group {
                    continue;
                }

                let t = stl_items!()[gidx as usize].start;
                *out_p = 0;
                let mut group_len = vim_strsize(t.cast());

                if stl_st().curitem > gidx + 1 && stl_items!()[gidx as usize].minwid == 0 {
                    let mut group_start_userhl = 0;
                    let mut group_end_userhl = 0;
                    let mut n = gidx - 1;
                    while n >= 0 {
                        let it = stl_items!()[n as usize].type_;
                        if it == StlItemType::Highlight || it == StlItemType::HighlightCombining {
                            group_start_userhl = stl_items!()[n as usize].minwid;
                            group_end_userhl = group_start_userhl;
                            break;
                        }
                        n -= 1;
                    }
                    n = gidx + 1;
                    while n < stl_st().curitem {
                        let it = stl_items!()[n as usize].type_;
                        if it == StlItemType::Normal {
                            break;
                        }
                        if it == StlItemType::Highlight || it == StlItemType::HighlightCombining {
                            group_end_userhl = stl_items!()[n as usize].minwid;
                        }
                        n += 1;
                    }
                    if n == stl_st().curitem && group_start_userhl == group_end_userhl {
                        out_p = t;
                        group_len = 0;
                        for item in
                            &mut stl_items!()[((gidx + 1) as usize)..(stl_st().curitem as usize)]
                        {
                            if item.type_ == StlItemType::Highlight
                                || item.type_ == StlItemType::HighlightCombining
                            {
                                item.type_ = StlItemType::Disabled;
                            }
                            if item.type_ == StlItemType::TabPage {
                                item.start = out_p;
                            }
                        }
                    }
                }

                let maxwid = stl_items!()[gidx as usize].maxwid;
                let mut minwid = stl_items!()[gidx as usize].minwid;
                let padding = if minwid < 0 {
                    StlPadding::PaddingRight
                } else {
                    StlPadding::PaddingLeft
                };
                minwid = minwid.abs();

                if group_len > maxwid
                    && stl_items!()[gidx as usize].type_ != StlItemType::HighlightFold
                {
                    stl_truncate(
                        &mut group_len,
                        minwid,
                        maxwid,
                        fillchar,
                        stl_items!(),
                        gidx + 1,
                        &mut stl_st().curitem,
                        t.cast(),
                        &raw mut out_p,
                    );
                } else if minwid > group_len {
                    stl_expand(
                        &mut group_len,
                        minwid,
                        padding,
                        crate::memory::ptr_offset_from(out_end_p, out_p) as i32,
                        fillchar,
                        stl_items!(),
                        gidx + 1,
                        stl_st().curitem,
                        t,
                        &raw mut out_p,
                    );
                }

                for item in &mut stl_items!()[((gidx + 1) as usize)..(stl_st().curitem as usize)] {
                    if item.type_ == StlItemType::Separate || item.type_ == StlItemType::Trunc {
                        item.type_ = StlItemType::Disabled;
                    }
                }

                continue;
            }

            let mut minwid: i32 = 0;
            let mut maxwid: i32 = 9999;
            let mut foldsignitem: i32 = -1;
            let mut left_align_num = false;
            let mut left_align = false;

            let zeropad = crate::memory::cstr_first_byte(fmt_p.cast()) == b'0';
            if zeropad {
                fmt_p = fmt_p.wrapping_add(1);
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b'-' {
                fmt_p = fmt_p.wrapping_add(1);
                left_align = true;
            }

            if ascii_isdigit(crate::memory::cstr_first_char(fmt_p.cast())) {
                let mut fp = fmt_p.cast();
                minwid = getdigits_int(&raw mut fp, false, 0);
                fmt_p = fp.cast();
            }

            if *fmt_p == STL_USER_HL {
                curr_hl_pos = stl_st().curitem;
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].minwid =
                    if minwid > 9 { 1 } else { minwid };
                fmt_p = fmt_p.wrapping_add(1);
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == STL_TABPAGENR || *fmt_p == STL_TABCLOSENR {
                if *fmt_p == STL_TABCLOSENR {
                    if minwid == 0 {
                        let mut n = stl_st().curitem - 1;
                        while n >= 0 {
                            if stl_items!()[n as usize].type_ == StlItemType::TabPage
                                && stl_items!()[n as usize].minwid >= 0
                            {
                                minwid = stl_items!()[n as usize].minwid;
                                break;
                            }
                            n -= 1;
                        }
                    } else {
                        minwid = -minwid;
                    }
                }
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::TabPage;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].minwid = minwid;
                fmt_p = fmt_p.wrapping_add(1);
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == STL_CLICK_FUNC {
                fmt_p = fmt_p.wrapping_add(1);
                let t = fmt_p;
                while *fmt_p != STL_CLICK_FUNC && !crate::memory::cstr_is_empty(fmt_p.cast()) {
                    fmt_p = fmt_p.wrapping_add(1);
                }
                if *fmt_p != STL_CLICK_FUNC {
                    break;
                }
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::ClickFunc;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].cmd = if !tabtab.is_null() {
                    let slice = crate::memory::cstr_slice(
                        t.cast(),
                        crate::memory::ptr_offset_from(fmt_p, t) as usize,
                    );
                    Some(std::ffi::CString::new(slice).unwrap())
                } else {
                    None
                };
                stl_items!()[stl_st().curitem as usize].minwid = minwid;
                fmt_p = fmt_p.wrapping_add(1);
                stl_st().curitem += 1;
                continue;
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b'.' {
                fmt_p = fmt_p.wrapping_add(1);
                if ascii_isdigit(crate::memory::cstr_first_char(fmt_p.cast())) {
                    let mut fp = fmt_p.cast();
                    maxwid = getdigits_int(&raw mut fp, false, 50);
                    fmt_p = fp.cast();
                    if maxwid <= 0 {
                        maxwid = 50;
                    }
                }
            }

            if left_align {
                minwid = -minwid;
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b'(' {
                stl_groupitems!()[groupdepth] = stl_st().curitem;
                groupdepth += 1;
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Group;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].minwid = minwid;
                stl_items!()[stl_st().curitem as usize].maxwid = maxwid;
                fmt_p = fmt_p.wrapping_add(1);
                stl_st().curitem += 1;
                continue;
            }

            if crate::memory::cstr_first_byte(fmt_p.cast()) == b'}' && evaldepth > 0 {
                fmt_p = fmt_p.wrapping_add(1);
                evaldepth -= 1;
                if groupdepth > 0 {
                    let mut normal_items = false;
                    let mut i = stl_st().curitem - 1;
                    while i >= evalstart
                        && stl_items!()[i as usize].type_ != StlItemType::Expression
                    {
                        let it = stl_items!()[i as usize].type_;
                        if it == StlItemType::Normal || it == StlItemType::NormalEmpty {
                            normal_items = true;
                            break;
                        }
                        i -= 1;
                    }
                    if !normal_items {
                        stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Normal;
                        stl_items!()[stl_st().curitem as usize].start = out_p;
                        stl_st().curitem += 1;
                    }
                }
                continue;
            }

            if *fmt_p == STL_HIGHLIGHT {
                if crate::memory::cstr_byte_at(fmt_p.cast(), 1) == b'(' {
                    fmt_p = fmt_p.wrapping_add(2);
                    stl_groupitems!()[groupdepth] = stl_st().curitem;
                    groupdepth += 1;
                    stl_items!()[stl_st().curitem as usize].type_ = StlItemType::HighlightScope;
                    stl_items!()[stl_st().curitem as usize].start = out_p;
                    stl_items!()[stl_st().curitem as usize].minwid = curr_hl_pos;
                    stl_st().curitem += 1;
                    continue;
                } else if crate::memory::cstr_byte_at(fmt_p.cast(), 1) == b')' {
                    fmt_p = fmt_p.wrapping_add(2);
                    if groupdepth < 1 {
                        continue;
                    }
                    groupdepth -= 1;
                    let scope_start = stl_groupitems!()[groupdepth];
                    if stl_items!()[scope_start as usize].type_ != StlItemType::HighlightScope {
                        continue;
                    }
                    let hl_pos = stl_items!()[scope_start as usize].minwid;
                    curr_hl_pos = stl_st().curitem;
                    stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                    stl_items!()[stl_st().curitem as usize].start = out_p;
                    stl_items!()[stl_st().curitem as usize].minwid = if hl_pos == -1 {
                        0
                    } else {
                        stl_items!()[hl_pos as usize].minwid
                    };
                    stl_st().curitem += 1;
                    let start_i = if hl_pos == -1 { evalstart } else { hl_pos + 1 };
                    for i in start_i..scope_start {
                        may_grow_stl_items_len!();
                        if stl_items!()[i as usize].type_ == StlItemType::HighlightCombining {
                            let mw = stl_items!()[i as usize].minwid;
                            stl_items!()[stl_st().curitem as usize].type_ =
                                StlItemType::HighlightCombining;
                            stl_items!()[stl_st().curitem as usize].start = out_p;
                            stl_items!()[stl_st().curitem as usize].minwid = mw;
                            stl_st().curitem += 1;
                        }
                    }
                    continue;
                }
            }

            if !crate::strings::vim_haschr(
                STL_ALL,
                i32::from(crate::memory::cstr_first_byte(fmt_p.cast())),
            ) {
                if crate::memory::cstr_is_empty(fmt_p.cast()) {
                    break;
                }
                fmt_p = fmt_p.wrapping_add(1);
                continue;
            }

            let opt = *fmt_p;
            fmt_p = fmt_p.wrapping_add(1);

            let mut base = NumberBase::NumBaseDecimal;
            let mut itemisflag = false;
            let mut fillable = true;
            let mut num: i32 = -1;
            let mut str: *mut u8 = ptr::null_mut();
            let mut owned_str: Option<std::ffi::CString> = None;

            let mut handle_stcsign = false;
            match opt {
                STL_FILEPATH | STL_FULLPATH | STL_FILENAME => {
                    fillable = false;
                    if let Some(name) = buf_spname(&*(wp.w_buffer)) {
                        xstrlcpy(name_buff, name, MAXPATHL);
                    } else {
                        let t = if opt == STL_FULLPATH {
                            (*wp).buf_mut().b_ffname
                        } else {
                            (*wp).buf_mut().b_fname
                        };
                        home_replace((wp.w_buffer).as_ref(), t, name_buff, MAXPATHL, true);
                    }
                    trans_characters(&mut g_st().name_buff);
                    if opt != STL_FILENAME {
                        str = name_buff.cast();
                    } else {
                        str = path_tail(name_buff).cast();
                    }
                }
                STL_VIM_EXPR => {
                    let block_start = fmt_p.wrapping_sub(1);
                    let reevaluate = crate::memory::cstr_first_byte(fmt_p.cast()) == b'%';
                    if reevaluate {
                        fmt_p = fmt_p.wrapping_add(1);
                    }
                    itemisflag = !zeropad;

                    let t = out_p;
                    while (crate::memory::cstr_first_byte(fmt_p.cast()) != b'}'
                        || (reevaluate && *fmt_p.wrapping_sub(1) != b'%'))
                        && !crate::memory::cstr_is_empty(fmt_p.cast())
                        && out_p < out_end_p
                    {
                        *out_p = *fmt_p;
                        out_p = out_p.wrapping_add(1);
                        fmt_p = fmt_p.wrapping_add(1);
                    }
                    if crate::memory::cstr_first_byte(fmt_p.cast()) == b'}' {
                        fmt_p = fmt_p.wrapping_add(1);
                        if reevaluate && out_p > out.data {
                            *out_p.wrapping_sub(1) = 0;
                        } else {
                            *out_p = 0;
                        }

                        out_p = t;

                        vim_snprintf!(
                            buf_tmp.as_mut_ptr().cast(),
                            buf_tmp.len(),
                            c"%d",
                            curbuf_mut().b_fnum,
                        );
                        set_internal_string_var(c"g:actual_curbuf", buf_tmp.as_mut_ptr().cast());
                        vim_snprintf!(
                            buf_tmp.as_mut_ptr().cast(),
                            buf_tmp.len(),
                            c"%d",
                            curwin_mut().handle,
                        );
                        set_internal_string_var(c"g:actual_curwin", buf_tmp.as_mut_ptr().cast());

                        let save_curbuf = g_st().curbuf;
                        let save_curwin = curwin_ptr();
                        #[allow(non_snake_case)]
                        let save_VIsual_active = g_st().visual_global.active;
                        set_curwin(wp);
                        g_st().curbuf = wp.w_buffer;
                        if curwin_ptr() != save_curwin {
                            g_st().visual_global.active = false;
                        }

                        owned_str = eval_to_string_safe(out_p.cast(), use_sandbox, false);
                        str = owned_str
                            .as_ref()
                            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut().cast());

                        set_curwin(save_curwin);
                        g_st().curbuf = save_curbuf;
                        g_st().visual_global.active = save_VIsual_active;

                        do_unlet(c"g:actual_curbuf", "g:actual_curbuf".len(), true);
                        do_unlet(c"g:actual_curwin", "g:actual_curwin".len(), true);

                        if !zeropad
                            && !crate::memory::cstr_is_empty(str.cast())
                            && *skipdigits(str.cast()) == NUL
                        {
                            num = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(
                                str.cast(),
                            )) as i32;
                            owned_str = None;
                            str = ptr::null_mut();
                            itemisflag = false;
                        }

                        if reevaluate
                            && !crate::memory::cstr_is_empty(str.cast())
                            && crate::strings::vim_haschr(str.cast(), b'%' as i32)
                            && evaldepth < MAX_STL_EVAL_DEPTH
                        {
                            let parsed_usefmt =
                                crate::memory::ptr_offset_from(block_start, usefmt) as usize;
                            let str_bytes = crate::memory::cstr_as_bytes(str.cast());
                            let fmt_bytes = crate::memory::cstr_as_bytes(fmt_p.cast());
                            let mut new_bytes = Vec::with_capacity(
                                parsed_usefmt + str_bytes.len() + fmt_bytes.len() + 2,
                            );
                            new_bytes.extend_from_slice(std::slice::from_raw_parts(
                                usefmt,
                                parsed_usefmt,
                            ));
                            new_bytes.extend_from_slice(str_bytes);
                            new_bytes.extend_from_slice(b"%}");
                            new_bytes.extend_from_slice(fmt_bytes);
                            drop(owned_str);
                            owned_usefmt = Some(std::ffi::CString::new(new_bytes).unwrap());
                            usefmt = owned_usefmt.as_ref().unwrap().as_ptr().cast_mut().cast();
                            fmt_p = usefmt.wrapping_add(parsed_usefmt);
                            evaldepth += 1;
                            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Expression;
                            stl_items!()[stl_st().curitem as usize].start = out_p;
                            stl_st().curitem += 1;
                            continue;
                        }
                    }
                }
                STL_LINE => {
                    if !stcp.is_null()
                        && (wp.w_onebuf_opt.wo_nu != 0 || wp.w_onebuf_opt.wo_rnu != 0)
                        && get_vim_var_nr(VV_VIRTNUM) == 0
                    {
                        if wp.w_maxscwidth == SCL_NUM && (*stcp).sattrs[0].text[0] != 0 {
                            handle_stcsign = true;
                        } else {
                            let relnum = get_vim_var_nr(VV_RELNUM) as i32;
                            num = if wp.w_onebuf_opt.wo_rnu == 0
                                || (wp.w_onebuf_opt.wo_nu != 0 && relnum == 0)
                            {
                                get_vim_var_nr(VV_LNUM) as i32
                            } else {
                                relnum
                            };
                            left_align_num = wp.w_onebuf_opt.wo_rnu != 0
                                && wp.w_onebuf_opt.wo_nu != 0
                                && relnum == 0;
                            if !left_align_num {
                                stl_items!()[stl_st().curitem as usize].type_ =
                                    StlItemType::Separate;
                                stl_items!()[stl_st().curitem as usize].start = out_p;
                                stl_st().curitem += 1;
                                may_grow_stl_items_len!();
                            }
                        }
                    } else if stcp.is_null() {
                        num = if ((*wp).buf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
                            0
                        } else {
                            wp.w_cursor.lnum
                        };
                    }
                }
                STL_NUMLINES => {
                    num = (*wp).buf_mut().b_ml.ml_line_count;
                }
                STL_COLUMN => {
                    num = if (g_st().state_global & MODE_INSERT) == 0 && empty_line {
                        0
                    } else {
                        wp.w_cursor.col + 1
                    };
                }
                STL_VIRTCOL | STL_VIRTCOL_ALT => {
                    let virtcol = wp.w_virtcol + 1;
                    let col_1 = if (g_st().state_global & MODE_INSERT) == 0 && empty_line {
                        0
                    } else {
                        wp.w_cursor.col + 1
                    };
                    if !(opt == STL_VIRTCOL_ALT && virtcol == col_1) {
                        num = virtcol;
                    }
                }
                STL_PERCENTAGE => {
                    num = calc_percentage(
                        i64::from(wp.w_cursor.lnum),
                        i64::from((*wp).buf_mut().b_ml.ml_line_count),
                    );
                }
                STL_ALTPERCENT => {
                    get_rel_pos(&mut *wp, &mut buf_tmp);
                    str = buf_tmp.as_mut_ptr();
                }
                STL_SHOWCMD => {
                    if opt_vars().p_sc != 0
                        && (opt_idx == K_OPT_INVALID || find_option(opt_vars().p_sloc) == opt_idx)
                    {
                        str = (&raw mut g_st().showcmd_buf).cast();
                    }
                }
                STL_ARGLISTSTAT => {
                    fillable = false;
                    buf_tmp[0] = 0;
                    if append_arg_number(&mut *wp, &mut buf_tmp) > 0 {
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_KEYMAP => {
                    fillable = false;
                    if get_keymap_str(&mut *wp, c"<%s>", &mut buf_tmp) > 0 {
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_PAGENUM => {
                    num = 0;
                }
                STL_BUFNO => {
                    num = (*wp).buf_mut().b_fnum;
                }
                STL_OFFSET_X | STL_OFFSET => {
                    if opt == STL_OFFSET_X {
                        base = NumberBase::NumBaseHexadecimal;
                    }
                    let l = ml_find_line_or_offset((*wp).buf_mut(), wp.w_cursor.lnum, None, false);
                    num = if ((*wp).buf_mut().b_ml.ml_flags & ML_EMPTY) != 0 || l < 0 {
                        0
                    } else {
                        l + 1
                            + if (g_st().state_global & MODE_INSERT) == 0 && empty_line {
                                0
                            } else {
                                wp.w_cursor.col
                            }
                    };
                }
                STL_BYTEVAL_X | STL_BYTEVAL => {
                    if opt == STL_BYTEVAL_X {
                        base = NumberBase::NumBaseHexadecimal;
                    }
                    num = byteval;
                    if num == NL {
                        num = 0;
                    } else if num == CAR && get_fileformat((*wp).buf_mut()) == EOL_MAC {
                        num = NL;
                    }
                }
                STL_ROFLAG | STL_ROFLAG_ALT => {
                    itemisflag = true;
                    if (*wp).buf_mut().b_p_ro != 0 {
                        str = if opt == STL_ROFLAG_ALT {
                            c",RO".as_ptr().cast_mut().cast()
                        } else {
                            tr(c"[RO]").as_ptr().cast_mut().cast()
                        };
                    }
                }
                STL_HELPFLAG | STL_HELPFLAG_ALT => {
                    itemisflag = true;
                    if (*wp).buf_mut().b_help {
                        str = if opt == STL_HELPFLAG_ALT {
                            c",HLP".as_ptr().cast_mut().cast()
                        } else {
                            tr(c"[Help]").as_ptr().cast_mut().cast()
                        };
                    }
                }
                STL_FOLDCOL | STL_SIGNCOL => {
                    handle_stcsign = true;
                }
                STL_FILETYPE => {
                    let ft = (*wp).buf_mut().b_p_ft;
                    if !crate::memory::cstr_is_empty(ft) && crate::memory::cstr_len(ft) < TMPLEN - 3
                    {
                        vim_snprintf!(buf_tmp.as_mut_ptr().cast(), buf_tmp.len(), c"[%s]", ft);
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_FILETYPE_ALT => {
                    itemisflag = true;
                    let ft = (*wp).buf_mut().b_p_ft;
                    if !crate::memory::cstr_is_empty(ft) && crate::memory::cstr_len(ft) < TMPLEN - 2
                    {
                        vim_snprintf!(buf_tmp.as_mut_ptr().cast(), buf_tmp.len(), c",%s", ft);
                        let mut t = buf_tmp.as_mut_ptr();
                        while *t != 0 {
                            *t = (*t).to_ascii_uppercase();
                            t = t.wrapping_add(1);
                        }
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_PREVIEWFLAG | STL_PREVIEWFLAG_ALT => {
                    itemisflag = true;
                    if wp.w_onebuf_opt.wo_pvw != 0 {
                        str = if opt == STL_PREVIEWFLAG_ALT {
                            c",PRV".as_ptr().cast_mut().cast()
                        } else {
                            tr(c"[Preview]").as_ptr().cast_mut().cast()
                        };
                    }
                }
                STL_QUICKFIX => {
                    if bt_quickfix((wp.w_buffer).as_ref()) {
                        str = if !wp.w_llist_ref.is_null() {
                            tr(MSG_LOCLIST).as_ptr().cast_mut().cast()
                        } else {
                            tr(MSG_QFLIST).as_ptr().cast_mut().cast()
                        };
                    }
                }
                STL_MODIFIED | STL_MODIFIED_ALT => {
                    itemisflag = true;
                    let code = i32::from(opt == STL_MODIFIED_ALT)
                        + i32::from(buf_is_changed_fn((*wp).buf_mut())) * 2
                        + i32::from((*wp).buf_mut().b_p_ma == 0) * 4;
                    str = match code {
                        2 => c"[+]".as_ptr().cast_mut().cast(),
                        3 => c",+".as_ptr().cast_mut().cast(),
                        4 => c"[-]".as_ptr().cast_mut().cast(),
                        5 => c",-".as_ptr().cast_mut().cast(),
                        6 => c"[+-]".as_ptr().cast_mut().cast(),
                        7 => c",+-".as_ptr().cast_mut().cast(),
                        _ => ptr::null_mut(),
                    };
                }
                STL_HIGHLIGHT_COMB | STL_HIGHLIGHT => {
                    let t = fmt_p;
                    while *fmt_p != opt && !crate::memory::cstr_is_empty(fmt_p.cast()) {
                        fmt_p = fmt_p.wrapping_add(1);
                    }
                    if *fmt_p == opt {
                        if opt == STL_HIGHLIGHT {
                            curr_hl_pos = stl_st().curitem;
                            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                        } else {
                            stl_items!()[stl_st().curitem as usize].type_ =
                                StlItemType::HighlightCombining;
                        }
                        stl_items!()[stl_st().curitem as usize].start = out_p;
                        stl_items!()[stl_st().curitem as usize].minwid = -syn_name2id_len(
                            t.cast(),
                            crate::memory::ptr_offset_from(fmt_p, t) as usize,
                        );
                        stl_st().curitem += 1;
                        fmt_p = fmt_p.wrapping_add(1);
                    }
                    continue;
                }
                _ => {}
            }

            if handle_stcsign && !stcp.is_null() {
                let fdc = if opt == STL_FOLDCOL {
                    compute_foldcolumn(&mut *wp, 0)
                } else {
                    0
                };
                let width = if opt == STL_FOLDCOL {
                    i32::from(fdc > 0)
                } else if opt == STL_SIGNCOL {
                    wp.w_scwidth
                } else {
                    1
                };

                if width > 0 {
                    foldsignitem = stl_st().curitem;
                    lnum = get_vim_var_nr(VV_LNUM) as Linenr;

                    if fdc > 0 {
                        let mut fold_buf: [Schar; 9] = [0; 9];
                        fill_foldcolumn(
                            &mut *wp,
                            (*stcp).foldinfo,
                            (*stcp).lnum,
                            0,
                            fdc,
                            get_vim_var_nr(VV_VIRTNUM) < 0,
                            ptr::null_mut(),
                            (*stcp).fold_vcol.as_mut_ptr(),
                            fold_buf.as_mut_ptr(),
                        );
                        stl_items!()[stl_st().curitem as usize].minwid =
                            -if use_cursor_line_highlight(&mut *wp, lnum) {
                                HLF_CLF
                            } else {
                                HLF_FC
                            };
                        let mut buflen: usize = 0;
                        for &fc in fold_buf.iter().take(fdc as usize) {
                            buflen += schar_get(buf_tmp.as_mut_ptr().wrapping_add(buflen), fc);
                        }
                    }

                    let mut signlen: usize = 0;
                    for i in 0..width {
                        stl_items!()[stl_st().curitem as usize].start = out_p.wrapping_add(signlen);
                        if fdc == 0 {
                            let sattr = (*stcp).sattrs[i as usize];
                            if sattr.text[0] != 0 && get_vim_var_nr(VV_VIRTNUM) == 0 {
                                signlen += describe_sign_text(&mut buf_tmp[signlen..], &sattr.text);
                                stl_items!()[stl_st().curitem as usize].minwid =
                                    -if (*stcp).sign_cul_id != 0 {
                                        (*stcp).sign_cul_id
                                    } else {
                                        sattr.hl_id
                                    };
                            } else {
                                buf_tmp[signlen] = b' ';
                                signlen += 1;
                                buf_tmp[signlen] = b' ';
                                signlen += 1;
                                buf_tmp[signlen] = 0;
                                stl_items!()[stl_st().curitem as usize].minwid = 0;
                            }
                        }
                        stl_items!()[stl_st().curitem as usize].type_ = if fdc > 0 {
                            StlItemType::HighlightFold
                        } else {
                            StlItemType::HighlightSign
                        };
                        stl_st().curitem += 1;
                        may_grow_stl_items_len!();
                    }
                    str = buf_tmp.as_mut_ptr();
                }
            }

            debug_assert!((stl_st().curitem as usize) < (*items_vec_ptr).len());
            stl_items!()[stl_st().curitem as usize].start = out_p;
            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Normal;

            if !crate::memory::cstr_is_empty(str.cast()) {
                let mut t = str;
                if itemisflag {
                    if !crate::memory::cstr_is_empty(t.cast())
                        && crate::memory::cstr_byte_at(t.cast(), 1) != 0
                        && ((!prevchar_isitem && crate::memory::cstr_first_byte(t.cast()) == b',')
                            || (prevchar_isflag
                                && crate::memory::cstr_first_byte(t.cast()) == b' '))
                    {
                        t = t.wrapping_add(1);
                    }
                    prevchar_isflag = true;
                }

                let mut l = vim_strsize(t.cast());
                if l > 0 {
                    prevchar_isitem = true;
                }

                if l > maxwid {
                    while l >= maxwid {
                        l -= ptr2cells(t.cast());
                        t = t.wrapping_add(utfc_ptr2len(t.cast()) as usize);
                    }
                    if out_p >= out_end_p {
                        break;
                    }
                    *out_p = b'<';
                    out_p = out_p.wrapping_add(1);
                }

                if minwid > 0 {
                    while l < minwid && out_p < out_end_p {
                        if l + 1 == minwid
                            && fillchar == schar_from_ascii(b'-')
                            && ascii_isdigit(crate::memory::cstr_first_char(t.cast()))
                        {
                            *out_p = b' ';
                            out_p = out_p.wrapping_add(1);
                        } else {
                            schar_get_adv(&raw mut out_p, fillchar);
                        }
                        l += 1;
                    }
                    minwid = 0;
                    if foldsignitem >= 0 {
                        let offset = crate::memory::ptr_offset_from(
                            out_p,
                            stl_items!()[foldsignitem as usize].start,
                        );
                        for item in
                            &mut stl_items!()[(foldsignitem as usize)..(stl_st().curitem as usize)]
                        {
                            item.start = item.start.wrapping_offset(offset);
                        }
                    }
                } else {
                    minwid = -minwid;
                }

                while !crate::memory::cstr_is_empty(t.cast()) && out_p < out_end_p {
                    if fillable
                        && crate::memory::cstr_first_byte(t.cast()) == b' '
                        && (!ascii_isdigit(crate::memory::cstr_char_at(t.cast(), 1))
                            || fillchar != schar_from_ascii(b'-'))
                    {
                        schar_get_adv(&raw mut out_p, fillchar);
                    } else {
                        *out_p = *t;
                        out_p = out_p.wrapping_add(1);
                    }
                    t = t.wrapping_add(1);
                }

                if foldsignitem >= 0 {
                    stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                    stl_items!()[stl_st().curitem as usize].start = out_p;
                    stl_items!()[stl_st().curitem as usize].minwid = 0;
                }

                while l < minwid && out_p < out_end_p {
                    schar_get_adv(&raw mut out_p, fillchar);
                    l += 1;
                }
            } else if num >= 0 {
                if out_p.wrapping_add(20) > out_end_p {
                    break;
                }
                prevchar_isitem = true;

                let mut nstr: [u8; 20] = [0; 20];
                let mut t = nstr.as_mut_ptr();
                if opt == STL_VIRTCOL_ALT {
                    *t = b'-';
                    t = t.wrapping_add(1);
                    minwid -= 1;
                }
                *t = b'%';
                t = t.wrapping_add(1);
                if zeropad {
                    *t = b'0';
                    t = t.wrapping_add(1);
                }
                *t = b'*';
                t = t.wrapping_add(1);
                *t = if base == NumberBase::NumBaseHexadecimal {
                    b'X'
                } else {
                    b'd'
                };
                *t.wrapping_add(1) = 0;

                let mut num_chars: i32 = 1;
                let mut n_tmp = num;
                while n_tmp >= base as i32 {
                    n_tmp /= base as i32;
                    num_chars += 1;
                }
                if opt == STL_VIRTCOL_ALT {
                    num_chars += 1;
                }

                let remaining_buf_len =
                    crate::memory::ptr_offset_from(out_end_p, out_p) as usize + 1;
                if num_chars > maxwid {
                    num_chars += 2;
                    let n = num_chars - maxwid;
                    while num_chars > maxwid {
                        num /= base as i32;
                        num_chars -= 1;
                    }
                    let base_ch = *t;
                    t = t.wrapping_add(1);
                    *t = b'>';
                    *t.wrapping_add(1) = b'%';
                    *t.wrapping_add(2) = base_ch;
                    *t.wrapping_add(3) = 0;

                    out_p = out_p.wrapping_add(vim_snprintf_safelen!(
                        out_p.cast(),
                        remaining_buf_len,
                        nstr.as_ptr().cast(),
                        0_i32,
                        num,
                        n,
                    ));
                } else {
                    out_p = out_p.wrapping_add(vim_snprintf_safelen!(
                        out_p.cast(),
                        remaining_buf_len,
                        nstr.as_ptr().cast(),
                        minwid,
                        num,
                    ));
                }
            } else {
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::NormalEmpty;
            }

            if num >= 0 || (!itemisflag && !crate::memory::cstr_is_empty(str.cast())) {
                prevchar_isflag = false;
            }

            drop(owned_str);

            stl_st().curitem += 1;
            if left_align_num {
                may_grow_stl_items_len!();
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Separate;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_st().curitem += 1;
            }
        }

        *out_p = 0;
        drop(owned_usefmt);

        let mut width = vim_strsize(out.data.cast());
        if maxwidth > 0 && width > maxwidth && (stcp.is_null() || width > MAX_STCWIDTH) {
            stl_truncate(
                &mut width,
                0,
                maxwidth,
                fillchar,
                stl_items!(),
                evalstart,
                &mut stl_st().curitem,
                out.data.cast(),
                &raw mut out_p,
            );
        } else if width < maxwidth {
            stl_expand(
                &mut width,
                maxwidth,
                StlPadding::PaddingNone,
                crate::memory::ptr_offset_from(out_end_p, out_p) as i32,
                fillchar,
                stl_items!(),
                evalstart,
                stl_st().curitem,
                out.data,
                &raw mut out_p,
            );
        }

        if !hltab.is_null() {
            *hltab = stl_hltab!().as_mut_ptr();
            let mut sp_idx: usize = 0;
            for item in &stl_items!()[(evalstart as usize)..(stl_st().curitem as usize)] {
                if matches!(
                    item.type_,
                    StlItemType::Highlight
                        | StlItemType::HighlightCombining
                        | StlItemType::HighlightFold
                        | StlItemType::HighlightSign
                ) {
                    stl_hltab!()[sp_idx].start = item.start;
                    stl_hltab!()[sp_idx].userhl = item.minwid;
                    stl_hltab!()[sp_idx].item = match item.type_ {
                        StlItemType::HighlightSign => i32::from(STL_SIGNCOL),
                        StlItemType::HighlightFold => i32::from(STL_FOLDCOL),
                        StlItemType::HighlightCombining => i32::from(STL_HIGHLIGHT_COMB),
                        _ => 0,
                    };
                    sp_idx += 1;
                }
            }
            stl_hltab!()[sp_idx].start = ptr::null_mut();
            stl_hltab!()[sp_idx].userhl = 0;
        }
        if !hltab_len.is_null() {
            *hltab_len = (stl_st().curitem - evalstart) as usize;
        }

        if !tabtab.is_null() {
            *tabtab = stl_tabtab!().as_mut_ptr();
            let mut tab_idx: usize = 0;
            for item in &mut stl_items!()[(evalstart as usize)..(stl_st().curitem as usize)] {
                if item.type_ == StlItemType::TabPage {
                    let rec = &mut stl_tabtab!()[tab_idx];
                    rec.start = item.start;
                    if item.minwid == 0 {
                        rec.def.type_ = K_STL_CLICK_DISABLED;
                        rec.def.tabnr = 0;
                    } else {
                        let mut tabnr = item.minwid;
                        if item.minwid > 0 {
                            rec.def.type_ = K_STL_CLICK_TAB_SWITCH;
                        } else {
                            rec.def.type_ = K_STL_CLICK_TAB_CLOSE;
                            tabnr = -tabnr;
                        }
                        rec.def.tabnr = tabnr;
                    }
                    rec.def.func = None;
                    tab_idx += 1;
                } else if item.type_ == StlItemType::ClickFunc {
                    let rec = &mut stl_tabtab!()[tab_idx];
                    rec.start = item.start;
                    rec.def.type_ = K_STL_CLICK_FUNC_RUN;
                    rec.def.tabnr = item.minwid;
                    rec.def.func = item.cmd.take();
                    tab_idx += 1;
                }
            }
            let rec = &mut stl_tabtab!()[tab_idx];
            rec.start = ptr::null();
            rec.def.type_ = K_STL_CLICK_DISABLED;
            rec.def.tabnr = 0;
            rec.def.func = None;
        }

        stl_st().curitem = evalstart;
        g_st().redraw_not_allowed = save_redraw_not_allowed;

        if opt_idx != K_OPT_INVALID && g_st().did_emsg > did_emsg_before {
            let def = get_option_default(opt_idx, opt_scope);
            set_option_direct(opt_idx, def, opt_scope, SID_ERROR);
            optval_free_read(opt_idx, def);
        }

        g_st().key_typed = save_KeyTyped;
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stl_alloc_and_clear_click_defs_manage_buffer() {
        let mut defs: Vec<StlClickDefinition> = Vec::new();
        let cdp = stl_alloc_click_defs(&mut defs, 4);
        assert!(!cdp.is_null());
        assert_eq!(defs.len(), 4);
        defs[0].func = Some(c"MyClick".to_owned());
        defs[1].func = Some(c"MyClick".to_owned());
        stl_clear_click_defs(&mut defs);
        assert!(defs[0].func.is_none());
    }

    #[test]
    fn build_stl_str_hl_formats_aligns_truncates_and_groups() {
        use crate::buffer::setfname;
        use crate::grid::schar_from_str;
        use std::ffi::CString;

        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            crate::main_init::ensure_test_editor_init();
            g_st().emsg_skip += 1;

            let check = |max_cells: i32,
                         pat: &str,
                         expected: &str,
                         file_name: Option<&str>,
                         fill: Option<&str>,
                         exp_cells: Option<i32>| {
                if let Some(fname) = file_name {
                    let mut fn_c = CString::new(fname).unwrap().into_bytes_with_nul();
                    setfname(
                        curbuf_mut(),
                        fn_c.as_mut_ptr().cast(),
                        ptr::null_mut(),
                        true,
                    );
                } else {
                    setfname(curbuf_mut(), ptr::null_mut(), ptr::null_mut(), true);
                }
                curwin_mut().w_cursor.lnum = 0;

                let fillchar = match fill {
                    None => schar_from_str(c" "),
                    Some("") => 0,
                    Some(s) => {
                        let c = CString::new(s).unwrap();
                        schar_from_str(c.as_ptr())
                    }
                };

                let mut out_buf = [b' '; 120];
                let mut pat_c = CString::new(pat).unwrap().into_bytes_with_nul();
                let cells = build_stl_str_hl(
                    curwin_ptr(),
                    CharBuf {
                        data: out_buf.as_mut_ptr(),
                        size: out_buf.len(),
                    },
                    pat_c.as_mut_ptr(),
                    K_OPT_INVALID,
                    0,
                    fillchar,
                    max_cells,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                let got = crate::memory::cstr_from_ptr(out_buf.as_ptr().cast())
                    .to_str()
                    .unwrap();
                assert_eq!(got, expected, "pat={pat:?}");
                assert_eq!(cells, exp_cells.unwrap_or(max_cells), "pat={pat:?}");
            };

            // Expression, file name, fillchar, plain text, line numbers
            check(2, "%!expand(20+1)", "21", None, None, None);
            check(11, "%!expand(20+1", "expand(20+1", None, None, None);
            check(10, "%f", "[No Name]", None, None, Some(9));
            check(
                30,
                "%f",
                "test/unit/buffer_spec.lua",
                Some("test/unit/buffer_spec.lua"),
                None,
                Some(25),
            );
            check(
                80,
                "%t",
                "buffer_spec.lua",
                Some("test/unit/buffer_spec.lua"),
                None,
                Some(15),
            );
            check(10, "abcde%=", "abcde!!!!!", None, Some("!"), None);
            check(10, "%=abcde", "~~~~~abcde", None, Some("~"), None);
            check(10, "abc%=def", "abc━━━━def", None, Some("━"), None);
            check(10, "abcde%=", "abcde     ", None, Some(""), None);
            check(80, "this is a test", "this is a test", None, None, Some(14));
            check(80, "%n", "1", None, None, Some(1));
            check(80, "%l", "0", None, None, Some(1));
            check(80, "%L", "1", None, None, Some(1));

            // Truncation & alignment
            check(10, "0123456789abcde", "<6789abcde", None, None, None);
            check(10, "abcdef%=ghijkl", "abcdef<jkl", None, None, None);
            check(10, "abcde%=gone%=fghij", "abcde<ghij", None, None, None);
            check(10, "abc%<defghijkl", "abc<ghijkl", None, None, None);
            check(10, "abc%<def%=ghijklmno", "abc<jklmno", None, None, None);
            check(10, "abcdef%=ghijkl%<mno", "abcdefghi>", None, None, None);
            check(3, "abc%", "abc", None, None, None);
            check(20, "neo%=vim", "neo              vim", None, None, None);
            check(
                20,
                "abc%=neovim%=def",
                "abc!!!!neovim!!!!def",
                None,
                Some("!"),
                None,
            );
            check(
                21,
                "abc%=neovim%=def",
                "abc━━━━neovim━━━━━def",
                None,
                Some("━"),
                None,
            );
            check(
                50,
                "aaaa%=b%=c%=d%=e%=fg%=hi%=jk%=lmnop%=qrstuv%=wxyz",
                "aaaa  b  c  d  e  fg  hi  jk  lmnop  qrstuv   wxyz",
                None,
                None,
                None,
            );

            // Item groups & auto-hiding
            check(
                30,
                "%5(%l%), %5.(%l%), %5.5(%l%), %5.10(%l%)",
                "    0,     0,     0,     0",
                None,
                None,
                Some(26),
            );
            check(
                30,
                "%-5(%l%), %-5.(%l%), %-5.5(%l%), %-5.10(%l%)",
                "0    , 0    , 0    , 0    ",
                None,
                None,
                Some(26),
            );
            check(
                29,
                "%21(foo%=%7(x%=y%=z%)%=bar%)%=%5(a%=b%=c%)",
                "foo    x  y  z    bar   a b c",
                None,
                None,
                None,
            );
            check(
                40,
                "%.24(trim both ends: %<%.17(%f%<%)%), path: %<%f",
                "trim both ends: <buffer>, path: <pec.lua",
                Some("test/unit/buffer_spec.lua"),
                None,
                None,
            );
            check(
                29,
                "visible: %(pre-%h%f-post%), hidden: %(pre-%h%w-post%)",
                "visible: pre-X-post, hidden: ",
                Some("X"),
                None,
                None,
            );
            check(
                29,
                "visible: %(pre-%{'X'}-post%), hidden: %(pre-%{''}-post%)",
                "visible: pre-X-post, hidden: ",
                None,
                None,
                None,
            );

            // Multi-cell truncation & escaping %
            check(5, "12🙂345", "<345", None, None, Some(4));
            check(5, "123🙂45%<", "123>", None, None, Some(4));
            check(4, "abc%%", "abc%", None, None, None);
            check(3, "abc%%abcabc", "<bc", None, None, None);
            check(1, "%%", "%", None, None, None);

            g_st().emsg_skip -= 1;
        }
    }
}
