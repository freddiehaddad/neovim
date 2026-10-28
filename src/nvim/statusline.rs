//! Statusline, winbar, ruler, tabline, and statuscolumn rendering (`src/nvim/statusline.c`).

use crate::api::private::helpers::{
    add_c, api_free_array, arena_array, arena_dict, arena_string, cstr_as_string, put_c,
};
use crate::api::private::validate::{Array, Object, ObjectData, ObjectType};
use crate::buffer::{
    append_arg_number, bt_quickfix, buf_T, buf_spname, calc_percentage, get_rel_pos, kUIMessages,
};
use crate::charset::{
    EOL_MAC, ascii_isdigit, getdigits_int, skipdigits, trans_characters, transstr_buf,
    vim_strnsize, vim_strsize,
};
use crate::context::ctx_saved_curwin;
use crate::cursor::{colnr_T, linenr_T};
use crate::digraph::get_keymap_str;
use crate::drawline::{HLF_CLF, HLF_FC, MAX_STCWIDTH, fill_foldcolumn, use_cursor_line_highlight};
use crate::drawscreen::{HLF_C, compute_foldcolumn, redrawing, w_endcol, w_endrow};
use crate::eval::eval_to_string_safe;
use crate::eval::typval::{TypvalVval, VarType, typval_T, varnumber_T};
use crate::eval::vars::{
    VV_LNUM, VV_RELNUM, VV_VIRTNUM, do_unlet, get_vim_var_nr, set_internal_string_var, set_var,
    set_vim_var_nr,
};
use crate::globals::*;
use crate::grid::{
    ScreenGrid, grid_adjust, grid_line_fill, grid_line_flush, grid_line_put_schar, grid_line_puts,
    grid_line_start, schar_T, schar_from_ascii, schar_get, schar_get_adv, schar_len,
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
    StlClickDefinition, kStlClickDisabled, kStlClickFuncRun, kStlClickTabClose, kStlClickTabSwitch,
};
use crate::normal::showcmd_update_clear_state;
use crate::option::{
    find_option, get_fileformat, get_option_default, kOptStatuscolumn, kOptStatusline, kOptWinbar,
    optval_free_read, set_option_direct, was_set_insecurely,
};
use crate::option_vars::*;
use crate::optionstr::{
    CharBuf, OptIndex, STL_ALL, STL_HIGHLIGHT, STL_SEPARATE, STL_TRUNCMARK, STL_USER_HL,
};
use crate::os::env::home_replace;
use crate::path::{path_tail, shorten_dir};
use crate::sign::describe_sign_text;
use crate::strings::{tr, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::ui::{ui_call_msg_ruler, ui_call_tabline_update, ui_has};
use crate::undo::bufIsChanged;
use crate::window::{global_stl_height, lastwin_nofloating, tabline_height, win_T};
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::ex_getln::hlf_T;
pub type StlFlag = c_int;
use crate::api::win_config::{FR_COL, UIExtension};

const TMPLEN: usize = 70;
const MAX_STL_EVAL_DEPTH: c_int = 100;

const kOptRulerformat: OptIndex = 244;
const kOptTabline: OptIndex = 306;

pub(crate) const kUITabline: UIExtension = 2;

const HLF_S: hlf_T = 19;
const HLF_SNC: hlf_T = 20;
const HLF_TP: hlf_T = 52;
const HLF_TPS: hlf_T = 53;
const HLF_TPF: hlf_T = 54;
const HLF_WBR: hlf_T = 65;
const HLF_WBRNC: hlf_T = 66;

const STL_FILEPATH: c_char = b'f' as c_char;
const STL_FULLPATH: c_char = b'F' as c_char;
const STL_FILENAME: c_char = b't' as c_char;
const STL_COLUMN: c_char = b'c' as c_char;
const STL_VIRTCOL: c_char = b'v' as c_char;
const STL_VIRTCOL_ALT: c_char = b'V' as c_char;
const STL_LINE: c_char = b'l' as c_char;
const STL_NUMLINES: c_char = b'L' as c_char;
const STL_BUFNO: c_char = b'n' as c_char;
const STL_KEYMAP: c_char = b'k' as c_char;
const STL_OFFSET: c_char = b'o' as c_char;
const STL_OFFSET_X: c_char = b'O' as c_char;
const STL_BYTEVAL: c_char = b'b' as c_char;
const STL_BYTEVAL_X: c_char = b'B' as c_char;
const STL_ROFLAG: c_char = b'r' as c_char;
const STL_ROFLAG_ALT: c_char = b'R' as c_char;
const STL_HELPFLAG: c_char = b'h' as c_char;
const STL_HELPFLAG_ALT: c_char = b'H' as c_char;
const STL_FILETYPE: c_char = b'y' as c_char;
const STL_FILETYPE_ALT: c_char = b'Y' as c_char;
const STL_PREVIEWFLAG: c_char = b'w' as c_char;
const STL_PREVIEWFLAG_ALT: c_char = b'W' as c_char;
const STL_MODIFIED: c_char = b'm' as c_char;
const STL_MODIFIED_ALT: c_char = b'M' as c_char;
const STL_QUICKFIX: c_char = b'q' as c_char;
const STL_PERCENTAGE: c_char = b'p' as c_char;
const STL_ALTPERCENT: c_char = b'P' as c_char;
const STL_ARGLISTSTAT: c_char = b'a' as c_char;
const STL_PAGENUM: c_char = b'N' as c_char;
const STL_SHOWCMD: c_char = b'S' as c_char;
pub(crate) const STL_FOLDCOL: c_char = b'C' as c_char;
pub(crate) const STL_SIGNCOL: c_char = b's' as c_char;
const STL_VIM_EXPR: c_char = b'{' as c_char;
const STL_HIGHLIGHT_COMB: c_char = b'$' as c_char;
const STL_TABPAGENR: c_char = b'T' as c_char;
const STL_TABCLOSENR: c_char = b'X' as c_char;
const STL_CLICK_FUNC: c_char = b'@' as c_char;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StlPadding {
    kPaddingNone = 0,
    kPaddingLeft = 1,
    kPaddingRight = 2,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumberBase {
    kNumBaseDecimal = 10,
    kNumBaseHexadecimal = 16,
}

#[repr(C)]
#[derive(Clone)]
pub struct StlClickRecord {
    pub def: StlClickDefinition,
    pub start: *const c_char,
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
pub struct stl_hlrec_t {
    pub start: *mut c_char,
    pub userhl: c_int,
    pub item: StlFlag,
}

impl stl_hlrec_t {
    pub const fn new() -> Self {
        Self {
            start: ptr::null_mut(),
            userhl: 0,
            item: 0,
        }
    }
}

impl Default for stl_hlrec_t {
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
pub struct stl_item_t {
    pub start: *mut c_char,
    pub cmd: Option<std::ffi::CString>,
    pub minwid: c_int,
    pub maxwid: c_int,
    pub type_: StlItemType,
}

impl stl_item_t {
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

impl Default for stl_item_t {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct foldinfo_T {
    pub fi_lnum: linenr_T,
    pub fi_level: c_int,
    pub fi_low_level: c_int,
    pub fi_lines: linenr_T,
}

impl foldinfo_T {
    pub const fn new() -> Self {
        Self {
            fi_lnum: 0,
            fi_level: 0,
            fi_low_level: 0,
            fi_lines: 0,
        }
    }
}

impl Default for foldinfo_T {
    fn default() -> Self {
        Self::new()
    }
}

use crate::arglist::ML_EMPTY;
use crate::autocmd::HLF_T;
use crate::charset::NUL;
use crate::cmdexpand::kUIWildmenu;
use crate::decoration::{SCL_NUM, SignTextAttrs};
use crate::keycodes::CAR;
use crate::option::{OPT_LOCAL, kOptInvalid};
use crate::os::fs::MAXPATHL;
use crate::runtime::SID_ERROR;
use crate::state::MODE_INSERT;

#[repr(C)]
pub struct statuscol_T {
    pub width: c_int,
    pub lnum: linenr_T,
    pub sign_cul_id: c_int,
    pub draw: bool,
    pub hlrec: Vec<stl_hlrec_t>,
    pub foldinfo: foldinfo_T,
    pub fold_vcol: [colnr_T; 9],
    pub sattrs: [SignTextAttrs; 9],
}

impl statuscol_T {
    pub const fn new() -> Self {
        Self {
            width: 0,
            lnum: 0,
            sign_cul_id: 0,
            draw: false,
            hlrec: Vec::new(),
            foldinfo: foldinfo_T::new(),
            fold_vcol: [0; 9],
            sattrs: [SignTextAttrs {
                text: [0; 2],
                hl_id: 0,
            }; 9],
        }
    }
}

impl Default for statuscol_T {
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
    stl_items_vec: Vec<stl_item_t>,
    stl_groupitems_vec: Vec<c_int>,
    stl_hltab_vec: Vec<stl_hlrec_t>,
    stl_tabtab_vec: Vec<StlClickRecord>,
    curitem: c_int,
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
pub fn win_redr_status(wp: &mut win_T) {
    unsafe {
        let is_stl_global = global_stl_height() > 0;

        if stl_st().redr_status_busy || (g_st().wild_menu_showing != 0 && !ui_has(kUIWildmenu)) {
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
        } else if *wp.w_onebuf_opt.wo_stl != NUL
            || !wp.w_floating
            || (is_stl_global && is_stl_curwin)
        {
            win_redr_stl_expr(&raw mut *wp, false, false, false);
        }

        let mut group: hlf_T = HLF_C;
        if wp.w_vsep_width != 0 && wp.w_status_height != 0 && redrawing() {
            let fillchar = if stl_connected(wp) {
                fillchar_status(&mut group, &raw mut *wp)
            } else {
                wp.w_p_fcs_chars.vert
            };
            let attr = win_hl_attr(&raw mut *wp, group);
            grid_line_start(&raw mut g_st().default_gridview, w_endrow(&*wp));
            grid_line_put_schar(w_endcol(wp), fillchar, attr);
            grid_line_flush();
        }
        stl_st().redr_status_busy = false;
    }
}

pub fn get_trans_bufname(buf: *mut buf_T) {
    unsafe {
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();
        let sp = buf_spname(buf);
        if !sp.is_null() {
            xstrlcpy(name_buff, sp, MAXPATHL);
        } else {
            home_replace(buf, (*buf).b_fname, name_buff, MAXPATHL, true);
        }
        trans_characters(name_buff, MAXPATHL as c_int);
    }
}

/// Return true if the status line of window `wp` is connected to the status
/// line of the window right of it. If not, then it's a vertical separator.
pub fn stl_connected(wp: &mut win_T) -> bool {
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
    width: c_int,
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
    mut buf: *const c_char,
    width: c_int,
    tabline: bool,
) {
    unsafe {
        if click_defs.is_null() {
            return;
        }

        let mut col: c_int = 0;
        let mut len: c_int = 0;

        let mut cur_click_def = StlClickDefinition {
            type_: kStlClickDisabled,
            tabnr: 0,
            func: None,
        };
        let mut i: usize = 0;
        while !(*click_recs.add(i)).start.is_null() {
            len += vim_strnsize(
                buf.cast_mut(),
                (*click_recs.add(i)).start.offset_from(buf) as c_int,
            );
            debug_assert!(len <= width);
            if col < len {
                while col < len {
                    *click_defs.add(col as usize) = cur_click_def.clone();
                    col += 1;
                }
            }
            buf = (*click_recs.add(i)).start;
            cur_click_def = (*click_recs.add(i)).def.clone();
            if !tabline
                && !(cur_click_def.type_ == kStlClickDisabled
                    || cur_click_def.type_ == kStlClickFuncRun)
            {
                cur_click_def.type_ = kStlClickDisabled;
            }
            i += 1;
        }
        if col < width {
            while col < width {
                *click_defs.add(col as usize) = cur_click_def.clone();
                col += 1;
            }
        }
    }
}

/// Redraws the statusline, winbar, ruler or tabline.
fn win_redr_stl_expr(wp: *mut win_T, draw_winbar: bool, draw_ruler: bool, ui_event: bool) {
    unsafe {
        let mut col: c_int = 0;
        let attr: c_int;
        let mut row: c_int = 0;
        let maxwidth: c_int;
        let mut group: hlf_T = 0;
        let fillchar: schar_T;
        let mut buf: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut transbuf: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut stl: *mut c_char;
        let opt_idx: OptIndex;
        let mut opt_scope: c_int = 0;
        let mut hltab: *mut stl_hlrec_t = ptr::null_mut();
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
            stl = opt_vars().p_tal;
            row = 0;
            fillchar = schar_from_ascii(b' ');
            group = HLF_TPF;
            attr = hl_attr(group);
            maxwidth = g_st().Columns;
            opt_idx = kOptTabline;
            let defs = &raw mut g_st().tab_page_click_defs;
            click_defs = (*defs).as_mut_ptr();
        } else if draw_winbar {
            opt_idx = kOptWinbar;
            stl = if *(*wp).w_onebuf_opt.wo_wbr != NUL {
                (*wp).w_onebuf_opt.wo_wbr
            } else {
                opt_vars().p_wbr
            };
            opt_scope = if *(*wp).w_onebuf_opt.wo_wbr != NUL {
                OPT_LOCAL
            } else {
                0
            };
            row = -1;
            col = 0;
            grid = grid_adjust(&raw mut (*wp).w_grid, &mut row, &mut col);

            if row < 0 {
                stl_st().stl_expr_entered = false;
                if !restore_curwin.is_null() {
                    set_curwin(restore_curwin);
                    g_st().curbuf = (*restore_curwin).w_buffer;
                }
                return;
            }

            fillchar = (*wp).w_p_fcs_chars.wbr;
            group = if wp == curwin_ptr() { HLF_WBR } else { HLF_WBRNC };
            attr = win_hl_attr(wp, group);
            maxwidth = (*wp).w_view_width;
            stl_clear_click_defs(&mut (*wp).w_winbar_click_defs);
            click_defs = stl_alloc_click_defs(&mut (*wp).w_winbar_click_defs, maxwidth);
        } else if draw_ruler && ui_event {
            stl = opt_vars().p_ruf;
            opt_idx = kOptRulerformat;
            maxwidth = g_st().Columns / 2;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else if draw_ruler {
            stl = opt_vars().p_ruf;
            opt_idx = kOptRulerformat;
            if *stl == b'%' as c_char {
                stl = stl.add(1);
                if *stl == b'-' as c_char {
                    stl = stl.add(1);
                }
                if crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(stl)) as c_int != 0 {
                    while ascii_isdigit(*stl) {
                        stl = stl.add(1);
                    }
                }
                let ch = *stl;
                stl = stl.add(1);
                if ch != b'(' as c_char {
                    stl = opt_vars().p_ruf;
                }
            }
            row = g_st().Rows - 1;
            col = g_st().ru_col.max(g_st().Columns / 2);
            grid = grid_adjust(&raw mut g_st().msg_grid_adj, &mut row, &mut col);
            maxwidth = g_st().Columns - col;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else {
            stl = if *(*wp).w_onebuf_opt.wo_stl != NUL {
                (*wp).w_onebuf_opt.wo_stl
            } else {
                opt_vars().p_stl
            };
            opt_idx = kOptStatusline;
            opt_scope = if *(*wp).w_onebuf_opt.wo_stl != NUL {
                OPT_LOCAL
            } else {
                0
            };
            if is_stl_global {
                row = g_st().Rows - (opt_vars().p_ch as c_int) - 1;
                col = 0;
                maxwidth = g_st().Columns;
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
            fillchar = fillchar_status(&mut group, wp);
            attr = win_hl_attr(wp, group);
            stl_clear_click_defs(&mut (*wp).w_status_click_defs);
            click_defs = stl_alloc_click_defs(&mut (*wp).w_status_click_defs, maxwidth);
        }

        if maxwidth > 0 {
            let ewp = if wp.is_null() { curwin_ptr() } else { wp };
            let p_crb_save = (*ewp).w_onebuf_opt.wo_crb;
            (*ewp).w_onebuf_opt.wo_crb = 0;

            let stl_cstr = crate::memory::cstr_to_cstring(stl);

            let mut tabtab: *mut StlClickRecord = ptr::null_mut();
            build_stl_str_hl(
                ewp,
                CharBuf {
                    data: buf.as_mut_ptr(),
                    size: buf.len(),
                },
                stl_cstr.as_ptr().cast_mut(),
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
            stl_fill_click_defs(click_defs, tabtab, buf.as_ptr(), maxwidth, wp.is_null());

            drop(stl_cstr);
            (*ewp).w_onebuf_opt.wo_crb = p_crb_save;

            let len = crate::memory::cstr_len(buf.as_ptr());
            let start_col = col;

            if !ui_event {
                screengrid_line_start(grid, row, 0);
            }

            let mut p = buf.as_mut_ptr();
            let mut curattr = attr;
            let mut curgroup = group;
            let mut content = Array::new();
            let mut sp = hltab;
            loop {
                let textlen = if !(*sp).start.is_null() {
                    (*sp).start.offset_from(p)
                } else {
                    buf.as_mut_ptr().add(len).offset_from(p)
                };
                let src_p = if p >= buf.as_mut_ptr().add(len) {
                    c"".as_ptr()
                } else {
                    p.cast_const()
                };
                let tsize =
                    transstr_buf(src_p, textlen, transbuf.as_mut_ptr(), transbuf.len(), true);
                if !ui_event {
                    col += grid_line_puts(col, transbuf.as_ptr(), tsize as c_int, curattr);
                } else {
                    let mut chunk = Array::new();
                    chunk.push(Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(i64::from(curattr)),
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData::String(crate::api::private::helpers::cbuf_to_string(
                                transbuf.as_ptr(),
                                tsize,
                            )),
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData::Integer(i64::from(curgroup)),
                    });
                    content.push(Object {
                        obj_type: ObjectType::kObjectTypeArray,
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
                    if (*sp).item == c_int::from(STL_HIGHLIGHT_COMB as u8) {
                        curattr = hl_combine_attr(curattr, new_attr);
                    } else {
                        curattr = new_attr;
                    }
                    curgroup = -(*sp).userhl;
                } else {
                    let userhl = if !wp.is_null() && wp != curwin_ptr() && (*wp).w_status_height != 0 {
                        &raw const g_st().highlight_stlnc[0]
                    } else {
                        &raw const g_st().highlight_user[0]
                    };
                    let userbuf: [c_char; 5] = [
                        b'U' as c_char,
                        b's' as c_char,
                        b'e' as c_char,
                        b'r' as c_char,
                        ((*sp).userhl as u8 + b'0') as c_char,
                    ];
                    curattr = *userhl.add(((*sp).userhl - 1) as usize);
                    curgroup = syn_name2id_len(userbuf.as_ptr(), 5);
                }
                if curattr != attr {
                    curattr = hl_combine_attr(attr, curattr);
                }
                sp = sp.add(1);
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

pub fn win_redr_winbar(wp: &mut win_T) {
    unsafe {
        if stl_st().winbar_entered {
            return;
        }
        stl_st().winbar_entered = true;

        if wp.w_winbar_height == 0 || !redrawing() {
            // Do nothing.
        } else if !crate::memory::cstr_is_empty(opt_vars().p_wbr) || *wp.w_onebuf_opt.wo_wbr != NUL {
            win_redr_stl_expr(&raw mut *wp, true, false, false);
        }
        stl_st().winbar_entered = false;
    }
}

pub fn redraw_ruler() {
    unsafe {
        let wp = if !curwin_mut().w_config.hide && curwin_mut().w_status_height == 0 {
            curwin_ptr()
        } else {
            lastwin_nofloating(ptr::null_mut())
        };
        let is_stl_global = global_stl_height() > 0;

        let part_of_status = (*wp).w_status_height != 0 || is_stl_global;
        if opt_vars().p_ru == 0 || part_of_status || (opt_vars().p_ch == 0 && !ui_has(kUIMessages)) {
            if stl_st().did_show_ext_ruler && ui_has(kUIMessages) {
                ui_call_msg_ruler(Array::new());
                stl_st().did_show_ext_ruler = false;
            } else if stl_st().did_show_ruler && !ui_has(kUIMessages) {
                msg_clr_eos();
            }
            stl_st().did_show_ruler = false;
            return;
        }

        if (*wp).w_cursor.lnum > (*wp).buf_mut().b_ml.ml_line_count {
            return;
        }

        if (*wp).w_status_height == 0 && !is_stl_global && !g_st().edit_submode.is_null() {
            return;
        }

        win_redr_stl_expr(wp, false, true, ui_has(kUIMessages));
        stl_st().did_show_ruler = !ui_has(kUIMessages);
    }
}

/// Get the character to use in a status line. Get its attributes in `*group`.
pub fn fillchar_status(group: &mut hlf_T, wp: *mut win_T) -> schar_T {
    unsafe {
        if wp == curwin_ptr() {
            *group = HLF_S;
            (*wp).w_p_fcs_chars.stl
        } else {
            *group = HLF_SNC;
            (*wp).w_p_fcs_chars.stlnc
        }
    }
}

fn ui_ext_tabline_update() {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();

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
                    obj_type: ObjectType::kObjectTypeTabpage,
                    data: ObjectData::Integer(i64::from((*tp).handle)),
                },
            );

            let cwp = if tp == g_st().curtab {
                curwin_ptr()
            } else {
                (*tp).tp_curwin
            };
            get_trans_bufname((*cwp).w_buffer);
            put_c(
                &mut tab_info,
                c"name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(arena_string(&mut arena, cstr_as_string(name_buff))),
                },
            );

            add_c(
                &mut tabs,
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
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
                        obj_type: ObjectType::kObjectTypeBuffer,
                        data: ObjectData::Integer(i64::from((*buf).b_fnum)),
                    },
                );

                get_trans_bufname(buf);
                put_c(
                    &mut buffer_info,
                    c"name",
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData::String(arena_string(&mut arena, cstr_as_string(name_buff))),
                    },
                );

                add_c(
                    &mut buffers,
                    Object {
                        obj_type: ObjectType::kObjectTypeDict,
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
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();

        if !crate::grid::grid_allocated(&g_st().default_grid) {
            return;
        }
        g_st().redraw_tabline = false;

        if ui_has(kUITabline) {
            ui_ext_tabline_update();
            return;
        }

        if tabline_height() < 1 {
            return;
        }

        let defs = &raw mut g_st().tab_page_click_defs;
        debug_assert!((*defs).len() >= g_st().Columns as usize);
        stl_clear_click_defs(&mut *defs);

        if !crate::memory::cstr_is_empty(opt_vars().p_tal) {
            win_redr_stl_expr(ptr::null_mut(), false, false, false);
        } else {
            let mut tabcount: c_int = 0;
            let mut col: c_int = 0;
            grid_line_start(&raw mut g_st().default_gridview, 0);
            let mut tp = g_st().first_tabpage;
            while !tp.is_null() {
                tabcount += 1;
                tp = (*tp).tp_next;
            }

            let tabwidth = (if tabcount > 0 {
                (g_st().Columns - 1 + tabcount / 2) / tabcount
            } else {
                0
            })
            .max(6);

            let mut attr = attr_nosel;
            tabcount = 0;

            tp = g_st().first_tabpage;
            while !tp.is_null() {
                if col >= g_st().Columns - 4 {
                    break;
                }

                let mut scol = col;
                let (cwp, mut wp) = if tp == g_st().curtab {
                    (curwin_ptr(), g_st().firstwin)
                } else {
                    ((*tp).tp_curwin, (*tp).tp_firstwin)
                };

                if (*tp).tp_topframe == g_st().topframe {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(cwp, HLF_TPS));
                }
                if use_sep_chars && col > 0 {
                    grid_line_put_schar(col, schar_from_ascii(b'|'), attr);
                    col += 1;
                }

                if (*tp).tp_topframe != g_st().topframe {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(cwp, HLF_TP));
                }

                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                let mut modified = false;
                let mut wincount: c_int = 0;
                while !wp.is_null() {
                    if !(*wp).w_config.focusable || (*wp).w_config.hide {
                        wincount -= 1;
                    } else if bufIsChanged((*wp).buf_mut()) {
                        modified = true;
                    }
                    wp = (*wp).w_next;
                    wincount += 1;
                }

                if modified || wincount > 1 {
                    if wincount > 1 {
                        let len = vim_snprintf!(name_buff, MAXPATHL, c"%d", wincount);
                        if col + len >= g_st().Columns - 3 {
                            break;
                        }
                        grid_line_puts(
                            col,
                            name_buff,
                            len,
                            hl_combine_attr(attr, win_hl_attr(cwp, HLF_T)),
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
                    get_trans_bufname((*cwp).w_buffer);
                    shorten_dir(name_buff);
                    let mut len = vim_strsize(name_buff);
                    let mut p = name_buff;
                    while len > room {
                        len -= ptr2cells(p);
                        p = p.add(utfc_ptr2len(p) as usize);
                    }
                    let n = g_st().Columns - col - 1;
                    len = len.min(n);

                    grid_line_puts(col, p, -1, attr);
                    col += len;
                }
                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                tabcount += 1;
                while scol < col {
                    (*defs).as_mut_slice()[scol as usize] = StlClickDefinition {
                        type_: kStlClickTabSwitch,
                        tabnr: tabcount,
                        func: None,
                    };
                    scol += 1;
                }

                tp = (*tp).tp_next;
            }

            for scol in col..g_st().Columns {
                (*defs).as_mut_slice()[scol as usize] = StlClickDefinition {
                    type_: kStlClickTabSwitch,
                    tabnr: 0,
                    func: None,
                };
            }

            let c = if use_sep_chars { b'_' } else { b' ' };
            grid_line_fill(col, g_st().Columns, schar_from_ascii(c), attr_fill);

            if opt_vars().p_sc != 0 && crate::memory::cstr_first_byte(opt_vars().p_sloc) == b't' {
                let n = g_st().Columns - col - c_int::from(tabcount > 1) * 3;
                let sc_width = 10.min(n);

                if sc_width > 0 {
                    grid_line_puts(
                        g_st().Columns - sc_width - c_int::from(tabcount > 1) * 2,
                        (&raw const g_st().showcmd_buf).cast(),
                        sc_width,
                        attr_nosel,
                    );
                }
                showcmd_update_clear_state();
            }

            if tabcount > 1 {
                grid_line_put_schar(g_st().Columns - 1, schar_from_ascii(b'X'), attr_nosel);
                (*defs).as_mut_slice()[(g_st().Columns - 1) as usize] = StlClickDefinition {
                    type_: kStlClickTabClose,
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
pub fn build_statuscol_str(wp: &mut win_T,
    lnum: linenr_T,
    relnum: c_int,
    virtnum: c_int,
    buf: CharBuf,
    stcp: &mut statuscol_T,
) -> c_int {
    unsafe {
        if relnum >= 0 {
            set_vim_var_nr(VV_LNUM, varnumber_T::from(lnum));
            set_vim_var_nr(VV_RELNUM, varnumber_T::from(relnum));
        }
        set_vim_var_nr(VV_VIRTNUM, varnumber_T::from(virtnum));

        let mut clickrec: *mut StlClickRecord = ptr::null_mut();
        let mut hlrec_ptr: *mut stl_hlrec_t = ptr::null_mut();
        let buf_data = buf.data;
        let stc = crate::memory::cstr_to_cstring(wp.w_onebuf_opt.wo_stc);
        let width = build_stl_str_hl(
            wp,
            buf,
            stc.as_ptr().cast_mut(),
            kOptStatuscolumn,
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
                sp = sp.add(1);
            }
        }

        if !(*clickrec).start.is_null() {
            let clicks = wp.w_statuscol_click_defs.entry(lnum).or_default();
            let click_defs = clicks.entry(virtnum).or_default();
            stl_clear_click_defs(click_defs);
            let cdp = stl_alloc_click_defs(click_defs, width);
            stl_fill_click_defs(cdp, clickrec, buf_data, width, false);
        }

        width
    }
}

/// Truncates the statusline at the first truncation marker (`%<`) if present.
fn stl_truncate(
    width: &mut c_int,
    mut minwid: c_int,
    maxwid: c_int,
    fillchar: schar_T,
    stl_items: &mut [stl_item_t],
    startitem: c_int,
    curitem: &mut c_int,
    start_p: *mut c_char,
    out_p: *mut *mut c_char,
) {
    unsafe {
        minwid = minwid.min(maxwid);

        let mut item_idx = startitem;
        let mut trunc_p: *mut c_char;

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

        if *width - vim_strsize(trunc_p) >= maxwid {
            trunc_p = start_p;
            *width = 0;
            loop {
                let char_cells = ptr2cells(trunc_p);
                if *width + char_cells >= maxwid {
                    break;
                }
                *width += char_cells;
                trunc_p = trunc_p.add(utfc_ptr2len(trunc_p) as usize);
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

            *trunc_p = b'>' as c_char;
            trunc_p = trunc_p.add(1);
            *out_p = trunc_p;
        } else {
            let mut trunc_len: c_int = 0;
            while *width >= maxwid {
                *width -= ptr2cells(trunc_p.add(trunc_len as usize));
                trunc_len += utfc_ptr2len(trunc_p.add(trunc_len as usize));
            }

            let trunc_end_p = trunc_p.add(trunc_len as usize);
            let move_len = (*out_p).offset_from(trunc_end_p) as usize + 1;
            ptr::copy(trunc_end_p, trunc_p.add(1), move_len);
            *out_p = (*out_p).sub(trunc_end_p.offset_from(trunc_p.add(1)) as usize);

            *trunc_p = b'<' as c_char;

            let item_offset = (trunc_len - 1) as isize;
            for item in &mut stl_items[(item_idx as usize)..(*curitem as usize)] {
                if item.start >= trunc_end_p {
                    item.start = item.start.offset(-item_offset);
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
        **out_p = NUL;
    }
}

/// Expands the statusline at separation markers (`%=`) if present,
/// or otherwise according to padding choice.
fn stl_expand(
    width: &mut c_int,
    target_width: c_int,
    padding: StlPadding,
    remaining_capacity: c_int,
    fillchar: schar_T,
    stl_items: &mut [stl_item_t],
    startitem: c_int,
    curitem: c_int,
    mut start_p: *mut c_char,
    out_p: *mut *mut c_char,
) {
    unsafe {
        let fillchar_bytes = schar_len(fillchar) as c_int;
        let mut added_cells = (target_width - *width).min(remaining_capacity / fillchar_bytes);
        let added_bytes = added_cells * fillchar_bytes;

        let mut num_separators: c_int = 0;
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
                item.start = item.start.offset(cumulated_dislocation);

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
                    let seploc = start.add(dislocation as usize);
                    let len = crate::memory::cstr_len(start) + 1;
                    ptr::copy(start, seploc, len);
                    let mut s = start;
                    while s < seploc {
                        schar_get_adv(&raw mut s, fillchar);
                    }
                }
            }

            *width += added_cells;
            *out_p = (*out_p).add(added_bytes as usize);
        } else if padding == StlPadding::kPaddingRight {
            while added_cells > 0 {
                schar_get_adv(out_p, fillchar);
                added_cells -= 1;
            }
            *width += added_bytes / fillchar_bytes;
        } else if padding == StlPadding::kPaddingLeft {
            ptr::copy(
                start_p,
                start_p.add(added_bytes as usize),
                (*out_p).offset_from(start_p) as usize,
            );

            let orig_added_cells = added_cells;
            while added_cells > 0 {
                schar_get_adv(&raw mut start_p, fillchar);
                added_cells -= 1;
            }

            for item in &mut stl_items[(startitem as usize)..(curitem as usize)] {
                item.start = item.start.add(added_bytes as usize);
            }

            *width += orig_added_cells;
            *out_p = (*out_p).add(added_bytes as usize);
        }
    }
}

/// Build a string from the status line items in `fmt`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn build_stl_str_hl(
    wp: *mut win_T,
    out: CharBuf,
    fmt: *mut c_char,
    opt_idx: OptIndex,
    opt_scope: c_int,
    mut fillchar: schar_T,
    maxwidth: c_int,
    hltab: *mut *mut stl_hlrec_t,
    hltab_len: *mut usize,
    tabtab: *mut *mut StlClickRecord,
    stcp: *mut statuscol_T,
) -> c_int {
    unsafe {


        let mut buf_tmp: [c_char; TMPLEN] = [0; TMPLEN];
        let mut owned_usefmt: Option<std::ffi::CString> = None;
        let mut usefmt = fmt;
        let save_redraw_not_allowed = g_st().redraw_not_allowed;
        let save_KeyTyped = g_st().KeyTyped;
        let did_emsg_before = g_st().did_emsg;
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();

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
            stl_items!().resize(INIT_LEN, stl_item_t::new());
            stl_groupitems!().resize(INIT_LEN, 0);
            stl_hltab!().resize(INIT_LEN + 1, stl_hlrec_t::new());
            stl_tabtab!().resize(INIT_LEN + 1, StlClickRecord::new());
        }

        let use_sandbox = if opt_idx != kOptInvalid {
            was_set_insecurely(wp, opt_idx, opt_scope) != 0
        } else {
            false
        };

        if *fmt == b'%' as c_char && *fmt.add(1) == b'!' as c_char {
            let mut tv = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: 0,
                vval: TypvalVval::Number(varnumber_T::from((*wp).handle)),
            };
            set_var(
                c"g:statusline_winid",
                "g:statusline_winid".len(),
                &mut tv,
                false,
            );

            owned_usefmt = eval_to_string_safe(fmt.add(2), use_sandbox, false);
            usefmt = owned_usefmt.as_ref().map_or(fmt, |s| s.as_ptr().cast_mut());

            do_unlet(
                c"g:statusline_winid",
                "g:statusline_winid".len(),
                true,
            );
        }

        if fillchar == 0 {
            fillchar = schar_from_ascii(b' ');
        }

        let mut lnum = (*wp).w_cursor.lnum;
        if lnum > (*wp).buf_mut().b_ml.ml_line_count {
            lnum = (*wp).buf_mut().b_ml.ml_line_count;
            (*wp).w_cursor.lnum = lnum;
        }

        let line_ptr = ml_get_buf((*wp).w_buffer, lnum);
        let empty_line = *line_ptr == NUL;

        let len = ml_get_buf_len((*wp).w_buffer, lnum);
        let byteval = if (*wp).w_cursor.col > len {
            (*wp).w_cursor.col = len;
            (*wp).w_cursor.coladd = 0;
            0
        } else {
            utf_ptr2char(line_ptr.add((*wp).w_cursor.col as usize))
        };

        let mut groupdepth: usize = 0;
        let mut evaldepth: c_int = 0;
        let mut curr_hl_pos: c_int = -1;
        let evalstart = stl_st().curitem;

        let mut prevchar_isflag = true;
        let mut prevchar_isitem = false;

        let mut out_p = out.data;
        let out_end_p = out.data.add(out.size - 1);

        macro_rules! may_grow_stl_items_len {
            () => {
                if stl_st().curitem as usize == (*items_vec_ptr).len() {
                    let new_len = (*items_vec_ptr).len() * 3 / 2;
                    stl_items!().resize(new_len, stl_item_t::new());
                    stl_groupitems!().resize(new_len, 0);
                    stl_hltab!().resize(new_len + 1, stl_hlrec_t::new());
                    stl_tabtab!().resize(new_len + 1, StlClickRecord::new());
                }
            };
        }

        let mut fmt_p = usefmt;
        while *fmt_p != NUL {
            may_grow_stl_items_len!();

            if *fmt_p != b'%' as c_char {
                prevchar_isflag = false;
                prevchar_isitem = false;
            }

            while *fmt_p != NUL && *fmt_p != b'%' as c_char && out_p < out_end_p {
                *out_p = *fmt_p;
                out_p = out_p.add(1);
                fmt_p = fmt_p.add(1);
            }

            if *fmt_p == NUL || out_p >= out_end_p {
                break;
            }

            fmt_p = fmt_p.add(1);
            if *fmt_p == NUL {
                break;
            }

            if *fmt_p == b'%' as c_char {
                *out_p = *fmt_p;
                out_p = out_p.add(1);
                fmt_p = fmt_p.add(1);
                prevchar_isflag = false;
                prevchar_isitem = false;
                continue;
            }

            if *fmt_p == STL_SEPARATE {
                fmt_p = fmt_p.add(1);
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Separate;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == STL_TRUNCMARK {
                fmt_p = fmt_p.add(1);
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Trunc;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == b')' as c_char {
                fmt_p = fmt_p.add(1);
                if groupdepth < 1 {
                    continue;
                }
                groupdepth -= 1;
                let gidx = stl_groupitems!()[groupdepth];
                if stl_items!()[gidx as usize].type_ != StlItemType::Group {
                    continue;
                }

                let t = stl_items!()[gidx as usize].start;
                *out_p = NUL;
                let mut group_len = vim_strsize(t);

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
                        for item in &mut stl_items!()[((gidx + 1) as usize)..(stl_st().curitem as usize)] {
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
                    StlPadding::kPaddingRight
                } else {
                    StlPadding::kPaddingLeft
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
                        t,
                        &raw mut out_p,
                    );
                } else if minwid > group_len {
                    stl_expand(
                        &mut group_len,
                        minwid,
                        padding,
                        out_end_p.offset_from(out_p) as c_int,
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

            let mut minwid: c_int = 0;
            let mut maxwid: c_int = 9999;
            let mut foldsignitem: c_int = -1;
            let mut left_align_num = false;
            let mut left_align = false;

            let zeropad = *fmt_p == b'0' as c_char;
            if zeropad {
                fmt_p = fmt_p.add(1);
            }

            if *fmt_p == b'-' as c_char {
                fmt_p = fmt_p.add(1);
                left_align = true;
            }

            if ascii_isdigit(*fmt_p) {
                minwid = getdigits_int(&raw mut fmt_p, false, 0);
            }

            if *fmt_p == STL_USER_HL {
                curr_hl_pos = stl_st().curitem;
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].minwid = if minwid > 9 { 1 } else { minwid };
                fmt_p = fmt_p.add(1);
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
                fmt_p = fmt_p.add(1);
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == STL_CLICK_FUNC {
                fmt_p = fmt_p.add(1);
                let t = fmt_p;
                while *fmt_p != STL_CLICK_FUNC && *fmt_p != NUL {
                    fmt_p = fmt_p.add(1);
                }
                if *fmt_p != STL_CLICK_FUNC {
                    break;
                }
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::ClickFunc;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].cmd = if !tabtab.is_null() {
                    let slice =
                        std::slice::from_raw_parts(t.cast::<u8>(), fmt_p.offset_from(t) as usize);
                    Some(std::ffi::CString::new(slice).unwrap())
                } else {
                    None
                };
                stl_items!()[stl_st().curitem as usize].minwid = minwid;
                fmt_p = fmt_p.add(1);
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == b'.' as c_char {
                fmt_p = fmt_p.add(1);
                if ascii_isdigit(*fmt_p) {
                    maxwid = getdigits_int(&raw mut fmt_p, false, 50);
                    if maxwid <= 0 {
                        maxwid = 50;
                    }
                }
            }

            if left_align {
                minwid = -minwid;
            }

            if *fmt_p == b'(' as c_char {
                stl_groupitems!()[groupdepth] = stl_st().curitem;
                groupdepth += 1;
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Group;
                stl_items!()[stl_st().curitem as usize].start = out_p;
                stl_items!()[stl_st().curitem as usize].minwid = minwid;
                stl_items!()[stl_st().curitem as usize].maxwid = maxwid;
                fmt_p = fmt_p.add(1);
                stl_st().curitem += 1;
                continue;
            }

            if *fmt_p == b'}' as c_char && evaldepth > 0 {
                fmt_p = fmt_p.add(1);
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
                if *fmt_p.add(1) == b'(' as c_char {
                    fmt_p = fmt_p.add(2);
                    stl_groupitems!()[groupdepth] = stl_st().curitem;
                    groupdepth += 1;
                    stl_items!()[stl_st().curitem as usize].type_ = StlItemType::HighlightScope;
                    stl_items!()[stl_st().curitem as usize].start = out_p;
                    stl_items!()[stl_st().curitem as usize].minwid = curr_hl_pos;
                    stl_st().curitem += 1;
                    continue;
                } else if *fmt_p.add(1) == b')' as c_char {
                    fmt_p = fmt_p.add(2);
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
                            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::HighlightCombining;
                            stl_items!()[stl_st().curitem as usize].start = out_p;
                            stl_items!()[stl_st().curitem as usize].minwid = mw;
                            stl_st().curitem += 1;
                        }
                    }
                    continue;
                }
            }

            if vim_strchr(STL_ALL, c_int::from(*fmt_p as u8)).is_null() {
                if *fmt_p == NUL {
                    break;
                }
                fmt_p = fmt_p.add(1);
                continue;
            }

            let opt = *fmt_p;
            fmt_p = fmt_p.add(1);

            let mut base = NumberBase::kNumBaseDecimal;
            let mut itemisflag = false;
            let mut fillable = true;
            let mut num: c_int = -1;
            let mut str: *mut c_char = ptr::null_mut();
            let mut owned_str: Option<std::ffi::CString> = None;

            let mut handle_stcsign = false;
            match opt {
                STL_FILEPATH | STL_FULLPATH | STL_FILENAME => {
                    fillable = false;
                    let name = buf_spname((*wp).w_buffer);
                    if !name.is_null() {
                        xstrlcpy(name_buff, name, MAXPATHL);
                    } else {
                        let t = if opt == STL_FULLPATH {
                            (*wp).buf_mut().b_ffname
                        } else {
                            (*wp).buf_mut().b_fname
                        };
                        home_replace((*wp).w_buffer, t, name_buff, MAXPATHL, true);
                    }
                    trans_characters(name_buff, MAXPATHL as c_int);
                    if opt != STL_FILENAME {
                        str = name_buff;
                    } else {
                        str = path_tail(name_buff);
                    }
                }
                STL_VIM_EXPR => {
                    let block_start = fmt_p.sub(1);
                    let reevaluate = *fmt_p == b'%' as c_char;
                    if reevaluate {
                        fmt_p = fmt_p.add(1);
                    }
                    itemisflag = !zeropad;

                    let t = out_p;
                    while (*fmt_p != b'}' as c_char
                        || (reevaluate && *fmt_p.sub(1) != b'%' as c_char))
                        && *fmt_p != NUL
                        && out_p < out_end_p
                    {
                        *out_p = *fmt_p;
                        out_p = out_p.add(1);
                        fmt_p = fmt_p.add(1);
                    }
                    if *fmt_p == b'}' as c_char {
                        fmt_p = fmt_p.add(1);
                        if reevaluate && out_p > out.data {
                            *out_p.sub(1) = NUL;
                        } else {
                            *out_p = NUL;
                        }

                        out_p = t;

                        vim_snprintf!(
                            buf_tmp.as_mut_ptr(),
                            buf_tmp.len(),
                            c"%d",
                            curbuf_mut().b_fnum,
                        );
                        set_internal_string_var(c"g:actual_curbuf", buf_tmp.as_mut_ptr());
                        vim_snprintf!(
                            buf_tmp.as_mut_ptr(),
                            buf_tmp.len(),
                            c"%d",
                            curwin_mut().handle,
                        );
                        set_internal_string_var(c"g:actual_curwin", buf_tmp.as_mut_ptr());

                        let save_curbuf = g_st().curbuf;
                        let save_curwin = curwin_ptr();
                        let save_VIsual_active = g_st().Visual.active;
                        set_curwin(wp);
                        g_st().curbuf = (*wp).w_buffer;
                        if curwin_ptr() != save_curwin {
                            g_st().Visual.active = false;
                        }

                        owned_str = eval_to_string_safe(out_p, use_sandbox, false);
                        str = owned_str
                            .as_ref()
                            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());

                        set_curwin(save_curwin);
                        g_st().curbuf = save_curbuf;
                        g_st().Visual.active = save_VIsual_active;

                        do_unlet(c"g:actual_curbuf", "g:actual_curbuf".len(), true);
                        do_unlet(c"g:actual_curwin", "g:actual_curwin".len(), true);

                        if !zeropad && !str.is_null() && *str != NUL && *skipdigits(str) == NUL {
                            num = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(str)) as c_int;
                            owned_str = None;
                            str = ptr::null_mut();
                            itemisflag = false;
                        }

                        if reevaluate
                            && !str.is_null()
                            && *str != NUL
                            && !vim_strchr(str, b'%' as c_int).is_null()
                            && evaldepth < MAX_STL_EVAL_DEPTH
                        {
                            let parsed_usefmt = block_start.offset_from(usefmt) as usize;
                            let str_bytes = crate::memory::cstr_as_bytes(str);
                            let fmt_bytes = crate::memory::cstr_as_bytes(fmt_p);
                            let mut new_bytes = Vec::with_capacity(
                                parsed_usefmt + str_bytes.len() + fmt_bytes.len() + 2,
                            );
                            new_bytes.extend_from_slice(std::slice::from_raw_parts(
                                usefmt.cast::<u8>(),
                                parsed_usefmt,
                            ));
                            new_bytes.extend_from_slice(str_bytes);
                            new_bytes.extend_from_slice(b"%}");
                            new_bytes.extend_from_slice(fmt_bytes);
                            drop(owned_str);
                            owned_usefmt = Some(std::ffi::CString::new(new_bytes).unwrap());
                            usefmt = owned_usefmt.as_ref().unwrap().as_ptr().cast_mut();
                            fmt_p = usefmt.add(parsed_usefmt);
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
                        && ((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
                        && get_vim_var_nr(VV_VIRTNUM) == 0
                    {
                        if (*wp).w_maxscwidth == SCL_NUM && (*stcp).sattrs[0].text[0] != 0 {
                            handle_stcsign = true;
                        } else {
                            let relnum = get_vim_var_nr(VV_RELNUM) as c_int;
                            num = if (*wp).w_onebuf_opt.wo_rnu == 0
                                || ((*wp).w_onebuf_opt.wo_nu != 0 && relnum == 0)
                            {
                                get_vim_var_nr(VV_LNUM) as c_int
                            } else {
                                relnum
                            };
                            left_align_num = (*wp).w_onebuf_opt.wo_rnu != 0
                                && (*wp).w_onebuf_opt.wo_nu != 0
                                && relnum == 0;
                            if !left_align_num {
                                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Separate;
                                stl_items!()[stl_st().curitem as usize].start = out_p;
                                stl_st().curitem += 1;
                                may_grow_stl_items_len!();
                            }
                        }
                    } else if stcp.is_null() {
                        num = if ((*wp).buf_mut().b_ml.ml_flags & ML_EMPTY) != 0 {
                            0
                        } else {
                            (*wp).w_cursor.lnum
                        };
                    }
                }
                STL_NUMLINES => {
                    num = (*wp).buf_mut().b_ml.ml_line_count;
                }
                STL_COLUMN => {
                    num = if (g_st().State & MODE_INSERT) == 0 && empty_line {
                        0
                    } else {
                        (*wp).w_cursor.col + 1
                    };
                }
                STL_VIRTCOL | STL_VIRTCOL_ALT => {
                    let virtcol = (*wp).w_virtcol + 1;
                    let col_1 = if (g_st().State & MODE_INSERT) == 0 && empty_line {
                        0
                    } else {
                        (*wp).w_cursor.col + 1
                    };
                    if !(opt == STL_VIRTCOL_ALT && virtcol == col_1) {
                        num = virtcol;
                    }
                }
                STL_PERCENTAGE => {
                    num = calc_percentage(
                        i64::from((*wp).w_cursor.lnum),
                        i64::from((*wp).buf_mut().b_ml.ml_line_count),
                    );
                }
                STL_ALTPERCENT => {
                    get_rel_pos(&mut *wp, buf_tmp.as_mut_ptr(), TMPLEN as c_int);
                    str = buf_tmp.as_mut_ptr();
                }
                STL_SHOWCMD => {
                    if opt_vars().p_sc != 0 && (opt_idx == kOptInvalid || find_option(opt_vars().p_sloc) == opt_idx) {
                        str = (&raw mut g_st().showcmd_buf).cast();
                    }
                }
                STL_ARGLISTSTAT => {
                    fillable = false;
                    buf_tmp[0] = NUL;
                    if append_arg_number(&mut *wp, buf_tmp.as_mut_ptr(), buf_tmp.len()) > 0 {
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_KEYMAP => {
                    fillable = false;
                    if get_keymap_str(
                        wp,
                        c"<%s>",
                        buf_tmp.as_mut_ptr(),
                        TMPLEN as c_int,
                    ) > 0
                    {
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
                        base = NumberBase::kNumBaseHexadecimal;
                    }
                    let l = ml_find_line_or_offset(
                        (*wp).buf_mut(),
                        (*wp).w_cursor.lnum,
                        None,
                        false,
                    );
                    num = if ((*wp).buf_mut().b_ml.ml_flags & ML_EMPTY) != 0 || l < 0 {
                        0
                    } else {
                        l + 1
                            + if (g_st().State & MODE_INSERT) == 0 && empty_line {
                                0
                            } else {
                                (*wp).w_cursor.col
                            }
                    };
                }
                STL_BYTEVAL_X | STL_BYTEVAL => {
                    if opt == STL_BYTEVAL_X {
                        base = NumberBase::kNumBaseHexadecimal;
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
                            c",RO".as_ptr().cast_mut()
                        } else {
                            tr(c"[RO]")
                        };
                    }
                }
                STL_HELPFLAG | STL_HELPFLAG_ALT => {
                    itemisflag = true;
                    if (*wp).buf_mut().b_help {
                        str = if opt == STL_HELPFLAG_ALT {
                            c",HLP".as_ptr().cast_mut()
                        } else {
                            tr(c"[Help]")
                        };
                    }
                }
                STL_FOLDCOL | STL_SIGNCOL => {
                    handle_stcsign = true;
                }
                STL_FILETYPE => {
                    let ft = (*wp).buf_mut().b_p_ft;
                    if *ft != NUL && crate::memory::cstr_len(ft) < TMPLEN - 3 {
                        vim_snprintf!(buf_tmp.as_mut_ptr(), buf_tmp.len(), c"[%s]", ft);
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_FILETYPE_ALT => {
                    itemisflag = true;
                    let ft = (*wp).buf_mut().b_p_ft;
                    if *ft != NUL && crate::memory::cstr_len(ft) < TMPLEN - 2 {
                        vim_snprintf!(buf_tmp.as_mut_ptr(), buf_tmp.len(), c",%s", ft);
                        let mut t = buf_tmp.as_mut_ptr();
                        while *t != NUL {
                            *t = (*t as u8).to_ascii_uppercase() as c_char;
                            t = t.add(1);
                        }
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_PREVIEWFLAG | STL_PREVIEWFLAG_ALT => {
                    itemisflag = true;
                    if (*wp).w_onebuf_opt.wo_pvw != 0 {
                        str = if opt == STL_PREVIEWFLAG_ALT {
                            c",PRV".as_ptr().cast_mut()
                        } else {
                            tr(c"[Preview]")
                        };
                    }
                }
                STL_QUICKFIX => {
                    if bt_quickfix((*wp).w_buffer) {
                        str = if !(*wp).w_llist_ref.is_null() {
                            tr(msg_loclist)
                        } else {
                            tr(msg_qflist)
                        };
                    }
                }
                STL_MODIFIED | STL_MODIFIED_ALT => {
                    itemisflag = true;
                    let code = c_int::from(opt == STL_MODIFIED_ALT)
                        + c_int::from(bufIsChanged((*wp).buf_mut())) * 2
                        + c_int::from((*wp).buf_mut().b_p_ma == 0) * 4;
                    str = match code {
                        2 => c"[+]".as_ptr().cast_mut(),
                        3 => c",+".as_ptr().cast_mut(),
                        4 => c"[-]".as_ptr().cast_mut(),
                        5 => c",-".as_ptr().cast_mut(),
                        6 => c"[+-]".as_ptr().cast_mut(),
                        7 => c",+-".as_ptr().cast_mut(),
                        _ => ptr::null_mut(),
                    };
                }
                STL_HIGHLIGHT_COMB | STL_HIGHLIGHT => {
                    let t = fmt_p;
                    while *fmt_p != opt && *fmt_p != NUL {
                        fmt_p = fmt_p.add(1);
                    }
                    if *fmt_p == opt {
                        if opt == STL_HIGHLIGHT {
                            curr_hl_pos = stl_st().curitem;
                            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::Highlight;
                        } else {
                            stl_items!()[stl_st().curitem as usize].type_ = StlItemType::HighlightCombining;
                        }
                        stl_items!()[stl_st().curitem as usize].start = out_p;
                        stl_items!()[stl_st().curitem as usize].minwid =
                            -syn_name2id_len(t, fmt_p.offset_from(t) as usize);
                        stl_st().curitem += 1;
                        fmt_p = fmt_p.add(1);
                    }
                    continue;
                }
                _ => {}
            }

            if handle_stcsign && !stcp.is_null() {
                let fdc = if opt == STL_FOLDCOL {
                    compute_foldcolumn(wp, 0)
                } else {
                    0
                };
                let width = if opt == STL_FOLDCOL {
                    c_int::from(fdc > 0)
                } else if opt == STL_SIGNCOL {
                    (*wp).w_scwidth
                } else {
                    1
                };

                if width > 0 {
                    foldsignitem = stl_st().curitem;
                    lnum = get_vim_var_nr(VV_LNUM) as linenr_T;

                    if fdc > 0 {
                        let mut fold_buf: [schar_T; 9] = [0; 9];
                        fill_foldcolumn(
                            wp,
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
                            buflen += schar_get(buf_tmp.as_mut_ptr().add(buflen), fc);
                        }
                    }

                    let mut signlen: usize = 0;
                    for i in 0..width {
                        stl_items!()[stl_st().curitem as usize].start = out_p.add(signlen);
                        if fdc == 0 {
                            let mut sattr = (*stcp).sattrs[i as usize];
                            if sattr.text[0] != 0 && get_vim_var_nr(VV_VIRTNUM) == 0 {
                                signlen += describe_sign_text(
                                    buf_tmp.as_mut_ptr().add(signlen),
                                    sattr.text.as_mut_ptr(),
                                );
                                stl_items!()[stl_st().curitem as usize].minwid = -if (*stcp).sign_cul_id != 0
                                {
                                    (*stcp).sign_cul_id
                                } else {
                                    sattr.hl_id
                                };
                            } else {
                                buf_tmp[signlen] = b' ' as c_char;
                                signlen += 1;
                                buf_tmp[signlen] = b' ' as c_char;
                                signlen += 1;
                                buf_tmp[signlen] = NUL;
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

            if !str.is_null() && *str != NUL {
                let mut t = str;
                if itemisflag {
                    if *t != NUL
                        && *t.add(1) != NUL
                        && ((!prevchar_isitem && *t == b',' as c_char)
                            || (prevchar_isflag && *t == b' ' as c_char))
                    {
                        t = t.add(1);
                    }
                    prevchar_isflag = true;
                }

                let mut l = vim_strsize(t);
                if l > 0 {
                    prevchar_isitem = true;
                }

                if l > maxwid {
                    while l >= maxwid {
                        l -= ptr2cells(t);
                        t = t.add(utfc_ptr2len(t) as usize);
                    }
                    if out_p >= out_end_p {
                        break;
                    }
                    *out_p = b'<' as c_char;
                    out_p = out_p.add(1);
                }

                if minwid > 0 {
                    while l < minwid && out_p < out_end_p {
                        if l + 1 == minwid
                            && fillchar == schar_from_ascii(b'-')
                            && ascii_isdigit(*t)
                        {
                            *out_p = b' ' as c_char;
                            out_p = out_p.add(1);
                        } else {
                            schar_get_adv(&raw mut out_p, fillchar);
                        }
                        l += 1;
                    }
                    minwid = 0;
                    if foldsignitem >= 0 {
                        let offset = out_p.offset_from(stl_items!()[foldsignitem as usize].start);
                        for item in &mut stl_items!()[(foldsignitem as usize)..(stl_st().curitem as usize)] {
                            item.start = item.start.offset(offset);
                        }
                    }
                } else {
                    minwid = -minwid;
                }

                while *t != NUL && out_p < out_end_p {
                    if fillable
                        && *t == b' ' as c_char
                        && (!ascii_isdigit(*t.add(1)) || fillchar != schar_from_ascii(b'-'))
                    {
                        schar_get_adv(&raw mut out_p, fillchar);
                    } else {
                        *out_p = *t;
                        out_p = out_p.add(1);
                    }
                    t = t.add(1);
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
                if out_p.add(20) > out_end_p {
                    break;
                }
                prevchar_isitem = true;

                let mut nstr: [c_char; 20] = [0; 20];
                let mut t = nstr.as_mut_ptr();
                if opt == STL_VIRTCOL_ALT {
                    *t = b'-' as c_char;
                    t = t.add(1);
                    minwid -= 1;
                }
                *t = b'%' as c_char;
                t = t.add(1);
                if zeropad {
                    *t = b'0' as c_char;
                    t = t.add(1);
                }
                *t = b'*' as c_char;
                t = t.add(1);
                *t = if base == NumberBase::kNumBaseHexadecimal {
                    b'X' as c_char
                } else {
                    b'd' as c_char
                };
                *t.add(1) = NUL;

                let mut num_chars: c_int = 1;
                let mut n_tmp = num;
                while n_tmp >= base as c_int {
                    n_tmp /= base as c_int;
                    num_chars += 1;
                }
                if opt == STL_VIRTCOL_ALT {
                    num_chars += 1;
                }

                let remaining_buf_len = out_end_p.offset_from(out_p) as usize + 1;
                if num_chars > maxwid {
                    num_chars += 2;
                    let n = num_chars - maxwid;
                    while num_chars > maxwid {
                        num /= base as c_int;
                        num_chars -= 1;
                    }
                    let base_ch = *t;
                    t = t.add(1);
                    *t = b'>' as c_char;
                    *t.add(1) = b'%' as c_char;
                    *t.add(2) = base_ch;
                    *t.add(3) = NUL;

                    out_p = out_p.add(vim_snprintf_safelen!(
                        out_p,
                        remaining_buf_len,
                        nstr.as_ptr(),
                        0 as c_int,
                        num,
                        n,
                    ));
                } else {
                    out_p = out_p.add(vim_snprintf_safelen!(
                        out_p,
                        remaining_buf_len,
                        nstr.as_ptr(),
                        minwid,
                        num,
                    ));
                }
            } else {
                stl_items!()[stl_st().curitem as usize].type_ = StlItemType::NormalEmpty;
            }

            if num >= 0 || (!itemisflag && !str.is_null() && *str != NUL) {
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

        *out_p = NUL;
        drop(owned_usefmt);

        let mut width = vim_strsize(out.data);
        if maxwidth > 0 && width > maxwidth && (stcp.is_null() || width > MAX_STCWIDTH) {
            stl_truncate(
                &mut width,
                0,
                maxwidth,
                fillchar,
                stl_items!(),
                evalstart,
                &mut stl_st().curitem,
                out.data,
                &raw mut out_p,
            );
        } else if width < maxwidth {
            stl_expand(
                &mut width,
                maxwidth,
                StlPadding::kPaddingNone,
                out_end_p.offset_from(out_p) as c_int,
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
                        StlItemType::HighlightSign => c_int::from(STL_SIGNCOL as u8),
                        StlItemType::HighlightFold => c_int::from(STL_FOLDCOL as u8),
                        StlItemType::HighlightCombining => c_int::from(STL_HIGHLIGHT_COMB as u8),
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
                        rec.def.type_ = kStlClickDisabled;
                        rec.def.tabnr = 0;
                    } else {
                        let mut tabnr = item.minwid;
                        if item.minwid > 0 {
                            rec.def.type_ = kStlClickTabSwitch;
                        } else {
                            rec.def.type_ = kStlClickTabClose;
                            tabnr = -tabnr;
                        }
                        rec.def.tabnr = tabnr;
                    }
                    rec.def.func = None;
                    tab_idx += 1;
                } else if item.type_ == StlItemType::ClickFunc {
                    let rec = &mut stl_tabtab!()[tab_idx];
                    rec.start = item.start;
                    rec.def.type_ = kStlClickFuncRun;
                    rec.def.tabnr = item.minwid;
                    rec.def.func = item.cmd.take();
                    tab_idx += 1;
                }
            }
            let rec = &mut stl_tabtab!()[tab_idx];
            rec.start = ptr::null();
            rec.def.type_ = kStlClickDisabled;
            rec.def.tabnr = 0;
            rec.def.func = None;
        }

        stl_st().curitem = evalstart;
        g_st().redraw_not_allowed = save_redraw_not_allowed;

        if opt_idx != kOptInvalid && g_st().did_emsg > did_emsg_before {
            let def = get_option_default(opt_idx, opt_scope);
            set_option_direct(opt_idx, def, opt_scope, SID_ERROR);
            optval_free_read(opt_idx, def);
        }

        g_st().KeyTyped = save_KeyTyped;
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
        use std::ffi::{CStr, CString};

        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            crate::main_init::ensure_test_editor_init();
            g_st().emsg_skip += 1;

            let check = |max_cells: c_int,
                         pat: &str,
                         expected: &str,
                         file_name: Option<&str>,
                         fill: Option<&str>,
                         exp_cells: Option<c_int>| {
                if let Some(fname) = file_name {
                    let mut fn_c = CString::new(fname).unwrap().into_bytes_with_nul();
                    setfname(g_st().curbuf, fn_c.as_mut_ptr().cast(), ptr::null_mut(), true);
                } else {
                    setfname(g_st().curbuf, ptr::null_mut(), ptr::null_mut(), true);
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

                let mut out_buf = [b' ' as c_char; 120];
                let mut pat_c = CString::new(pat).unwrap().into_bytes_with_nul();
                let cells = build_stl_str_hl(
                    curwin_ptr(),
                    CharBuf {
                        data: out_buf.as_mut_ptr(),
                        size: out_buf.len(),
                    },
                    pat_c.as_mut_ptr().cast(),
                    kOptInvalid,
                    0,
                    fillchar,
                    max_cells,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                let got = CStr::from_ptr(out_buf.as_ptr()).to_str().unwrap();
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
