//! Statusline, winbar, ruler, tabline, and statuscolumn rendering (`src/nvim/statusline.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::private::helpers::{
    add_c, api_free_array, arena_array, arena_dict, arena_string, cstr_as_string, put_c,
};
use crate::api::private::validate::{Array, NvimString, Object, ObjectData, ObjectType};
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
use crate::map::{map_put_ref_int_StcClick, map_put_ref_int_StcClicks};
use crate::mbyte::{ptr2cells, utf_ptr2char, utfc_ptr2len};
use crate::memline::{ml_find_line_or_offset, ml_get_buf, ml_get_buf_len};
use crate::memory::{
    Arena, arena_finish, arena_mem_free, xcalloc, xfree, xmalloc, xmemdupz, xrealloc, xstrdup,
    xstrlcpy,
};
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
use crate::strings::{atoi, toupper, tr, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::ui::{ui_call_msg_ruler, ui_call_tabline_update, ui_has};
use crate::undo::bufIsChanged;
use crate::window::{global_stl_height, lastwin_nofloating, tabline_height, win_T};
use std::ffi::{CStr, c_char, c_int};
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
const HLF_MSG: hlf_T = 63;
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
const STL_FOLDCOL: c_char = b'C' as c_char;
const STL_SIGNCOL: c_char = b's' as c_char;
const STL_VIM_EXPR: c_char = b'{' as c_char;
const STL_HIGHLIGHT_COMB: c_char = b'$' as c_char;
const STL_TABPAGENR: c_char = b'T' as c_char;
const STL_TABCLOSENR: c_char = b'X' as c_char;
const STL_CLICK_FUNC: c_char = b'@' as c_char;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
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
#[derive(Clone, Copy)]
pub struct StlClickRecord {
    pub def: StlClickDefinition,
    pub start: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct stl_hlrec_t {
    pub start: *mut c_char,
    pub userhl: c_int,
    pub item: StlFlag,
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
#[derive(Clone, Copy)]
pub struct stl_item_t {
    pub start: *mut c_char,
    pub cmd: *mut c_char,
    pub minwid: c_int,
    pub maxwid: c_int,
    pub type_: StlItemType,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct foldinfo_T {
    pub fi_lnum: linenr_T,
    pub fi_level: c_int,
    pub fi_low_level: c_int,
    pub fi_lines: linenr_T,
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
    pub hlrec: *mut stl_hlrec_t,
    pub foldinfo: foldinfo_T,
    pub fold_vcol: [colnr_T; 9],
    pub sattrs: *mut SignTextAttrs,
}

static mut did_show_ext_ruler: bool = false;

/// Redraw the status line of window `wp`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn win_redr_status(wp: *mut win_T) {
    unsafe {
        let is_stl_global = global_stl_height() > 0;
        static mut busy: bool = false;

        if busy || (wild_menu_showing != 0 && !ui_has(kUIWildmenu)) {
            return;
        }

        busy = true;
        (*wp).w_redr_status = false;

        let mut stl_curwin = ctx_saved_curwin();
        if stl_curwin.is_null() {
            stl_curwin = curwin;
        }
        let is_stl_curwin = wp == stl_curwin;

        if (*wp).w_status_height == 0 && !(is_stl_global && is_stl_curwin) {
            redraw_cmdline = true;
        } else if !redrawing() {
            (*wp).w_redr_status = true;
        } else if *(*wp).w_onebuf_opt.wo_stl != NUL
            || !(*wp).w_floating
            || (is_stl_global && is_stl_curwin)
        {
            win_redr_stl_expr(wp, false, false, false);
        }

        let mut group: hlf_T = HLF_C;
        if (*wp).w_vsep_width != 0 && (*wp).w_status_height != 0 && redrawing() {
            let fillchar = if stl_connected(wp) {
                fillchar_status(&raw mut group, wp)
            } else {
                (*wp).w_p_fcs_chars.vert
            };
            let attr = win_hl_attr(wp, group);
            grid_line_start(&raw mut default_gridview, w_endrow(wp));
            grid_line_put_schar(w_endcol(wp), fillchar, attr);
            grid_line_flush();
        }
        busy = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_trans_bufname(buf: *mut buf_T) {
    unsafe {
        let name_buff = (&raw mut NameBuff).cast::<c_char>();
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stl_connected(wp: *mut win_T) -> bool {
    unsafe {
        let mut fr = (*wp).w_frame;
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stl_clear_click_defs(
    click_defs: *mut StlClickDefinition,
    click_defs_size: usize,
) {
    unsafe {
        if !click_defs.is_null() {
            for i in 0..click_defs_size {
                if i == 0 || (*click_defs.add(i)).func != (*click_defs.add(i - 1)).func {
                    xfree((*click_defs.add(i)).func.cast());
                }
            }
            ptr::write_bytes(click_defs, 0, click_defs_size);
        }
    }
}

/// Allocate or resize the click definitions array if needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stl_alloc_click_defs(
    mut cdp: *mut StlClickDefinition,
    width: c_int,
    size: *mut usize,
) -> *mut StlClickDefinition {
    unsafe {
        if *size < width as usize {
            xfree(cdp.cast());
            *size = width as usize;
            cdp = xcalloc(*size, size_of::<StlClickDefinition>()).cast();
        }
        cdp
    }
}

/// Fill the click definitions array if needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stl_fill_click_defs(
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
            func: ptr::null_mut(),
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
                    *click_defs.add(col as usize) = cur_click_def;
                    col += 1;
                }
            } else {
                xfree(cur_click_def.func.cast());
            }
            buf = (*click_recs.add(i)).start;
            cur_click_def = (*click_recs.add(i)).def;
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
                *click_defs.add(col as usize) = cur_click_def;
                col += 1;
            }
        } else {
            xfree(cur_click_def.func.cast());
        }
    }
}

/// Redraws the statusline, winbar, ruler or tabline.
unsafe fn win_redr_stl_expr(wp: *mut win_T, draw_winbar: bool, draw_ruler: bool, ui_event: bool) {
    unsafe {
        static mut entered: bool = false;
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
            &raw mut default_grid
        };

        if !is_stl_global && !wp.is_null() && (*wp).w_config.hide {
            return;
        }

        if entered {
            return;
        }
        entered = true;

        let save_curwin = ctx_saved_curwin();
        let restore_curwin = if !save_curwin.is_null() {
            curwin
        } else {
            ptr::null_mut()
        };
        if !save_curwin.is_null() {
            curwin = save_curwin;
            curbuf = (*curwin).w_buffer;
        }

        if wp.is_null() {
            stl = p_tal;
            row = 0;
            fillchar = schar_from_ascii(b' ');
            group = HLF_TPF;
            attr = hl_attr(group);
            maxwidth = Columns;
            opt_idx = kOptTabline;
            click_defs = tab_page_click_defs;
        } else if draw_winbar {
            opt_idx = kOptWinbar;
            stl = if *(*wp).w_onebuf_opt.wo_wbr != NUL {
                (*wp).w_onebuf_opt.wo_wbr
            } else {
                p_wbr
            };
            opt_scope = if *(*wp).w_onebuf_opt.wo_wbr != NUL {
                OPT_LOCAL
            } else {
                0
            };
            row = -1;
            col = 0;
            grid = grid_adjust(&raw mut (*wp).w_grid, &raw mut row, &raw mut col);

            if row < 0 {
                entered = false;
                if !restore_curwin.is_null() {
                    curwin = restore_curwin;
                    curbuf = (*restore_curwin).w_buffer;
                }
                return;
            }

            fillchar = (*wp).w_p_fcs_chars.wbr;
            group = if wp == curwin { HLF_WBR } else { HLF_WBRNC };
            attr = win_hl_attr(wp, group);
            maxwidth = (*wp).w_view_width;
            stl_clear_click_defs(
                (*wp).w_winbar_click_defs.cast(),
                (*wp).w_winbar_click_defs_size,
            );
            (*wp).w_winbar_click_defs = stl_alloc_click_defs(
                (*wp).w_winbar_click_defs.cast(),
                maxwidth,
                &raw mut (*wp).w_winbar_click_defs_size,
            )
            .cast();
            click_defs = (*wp).w_winbar_click_defs.cast();
        } else if draw_ruler && ui_event {
            stl = p_ruf;
            opt_idx = kOptRulerformat;
            maxwidth = Columns / 2;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else if draw_ruler {
            stl = p_ruf;
            opt_idx = kOptRulerformat;
            if *stl == b'%' as c_char {
                stl = stl.add(1);
                if *stl == b'-' as c_char {
                    stl = stl.add(1);
                }
                if atoi(stl) != 0 {
                    while ascii_isdigit(*stl) {
                        stl = stl.add(1);
                    }
                }
                let ch = *stl;
                stl = stl.add(1);
                if ch != b'(' as c_char {
                    stl = p_ruf;
                }
            }
            row = Rows - 1;
            col = ru_col.max(Columns / 2);
            grid = grid_adjust(&raw mut msg_grid_adj, &raw mut row, &raw mut col);
            maxwidth = Columns - col;
            fillchar = schar_from_ascii(b' ');
            group = HLF_MSG;
            attr = hl_attr(group);
            click_defs = ptr::null_mut();
        } else {
            stl = if *(*wp).w_onebuf_opt.wo_stl != NUL {
                (*wp).w_onebuf_opt.wo_stl
            } else {
                p_stl
            };
            opt_idx = kOptStatusline;
            opt_scope = if *(*wp).w_onebuf_opt.wo_stl != NUL {
                OPT_LOCAL
            } else {
                0
            };
            if is_stl_global {
                row = Rows - (p_ch as c_int) - 1;
                col = 0;
                maxwidth = Columns;
            } else if (*wp).w_status_height == 0 {
                entered = false;
                if !restore_curwin.is_null() {
                    curwin = restore_curwin;
                    curbuf = (*restore_curwin).w_buffer;
                }
                return;
            } else if (*wp).w_floating {
                row = (*wp).w_winrow_off + (*wp).w_view_height;
                col = (*wp).w_wincol_off;
                maxwidth = (*wp).w_view_width;
            } else {
                row = w_endrow(wp);
                col = (*wp).w_wincol;
                maxwidth = (*wp).w_width;
            }
            fillchar = fillchar_status(&raw mut group, wp);
            attr = win_hl_attr(wp, group);
            stl_clear_click_defs(
                (*wp).w_status_click_defs.cast(),
                (*wp).w_status_click_defs_size,
            );
            (*wp).w_status_click_defs = stl_alloc_click_defs(
                (*wp).w_status_click_defs.cast(),
                maxwidth,
                &raw mut (*wp).w_status_click_defs_size,
            )
            .cast();
            click_defs = (*wp).w_status_click_defs.cast();
        }

        if maxwidth > 0 {
            let ewp = if wp.is_null() { curwin } else { wp };
            let p_crb_save = (*ewp).w_onebuf_opt.wo_crb;
            (*ewp).w_onebuf_opt.wo_crb = 0;

            stl = xstrdup(stl);

            let mut tabtab: *mut StlClickRecord = ptr::null_mut();
            build_stl_str_hl(
                ewp,
                CharBuf {
                    data: buf.as_mut_ptr(),
                    size: buf.len(),
                },
                stl,
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

            xfree(stl.cast());
            (*ewp).w_onebuf_opt.wo_crb = p_crb_save;

            let len = CStr::from_ptr(buf.as_ptr()).to_bytes().len();
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
                        data: ObjectData {
                            integer: i64::from(curattr),
                        },
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: NvimString {
                                data: xmemdupz(transbuf.as_ptr().cast(), tsize).cast(),
                                size: tsize,
                            },
                        },
                    });
                    chunk.push(Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: i64::from(curgroup),
                        },
                    });
                    content.push(Object {
                        obj_type: ObjectType::kObjectTypeArray,
                        data: ObjectData { array: chunk },
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
                    let userhl = if !wp.is_null() && wp != curwin && (*wp).w_status_height != 0 {
                        &raw const highlight_stlnc[0]
                    } else {
                        &raw const highlight_user[0]
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

            if p_sc != 0 && find_option(p_sloc) == opt_idx {
                showcmd_update_clear_state();
            }

            if ui_event {
                ui_call_msg_ruler(content);
                did_show_ext_ruler = true;
                api_free_array(content);
            } else {
                let maxcol = start_col + maxwidth;
                grid_line_fill(col, maxcol, fillchar, curattr);
                grid_line_flush();
            }
        }

        entered = false;

        if !restore_curwin.is_null() {
            curwin = restore_curwin;
            curbuf = (*restore_curwin).w_buffer;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn win_redr_winbar(wp: *mut win_T) {
    unsafe {
        static mut entered: bool = false;

        if entered {
            return;
        }
        entered = true;

        if (*wp).w_winbar_height == 0 || !redrawing() {
            // Do nothing.
        } else if *p_wbr != NUL || *(*wp).w_onebuf_opt.wo_wbr != NUL {
            win_redr_stl_expr(wp, true, false, false);
        }
        entered = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn redraw_ruler() {
    unsafe {
        static mut did_show_ruler: bool = false;
        let wp = if !(*curwin).w_config.hide && (*curwin).w_status_height == 0 {
            curwin
        } else {
            lastwin_nofloating(ptr::null_mut())
        };
        let is_stl_global = global_stl_height() > 0;

        let part_of_status = (*wp).w_status_height != 0 || is_stl_global;
        if p_ru == 0 || part_of_status || (p_ch == 0 && !ui_has(kUIMessages)) {
            if did_show_ext_ruler && ui_has(kUIMessages) {
                ui_call_msg_ruler(Array::new());
                did_show_ext_ruler = false;
            } else if did_show_ruler && !ui_has(kUIMessages) {
                msg_clr_eos();
            }
            did_show_ruler = false;
            return;
        }

        if (*wp).w_cursor.lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            return;
        }

        if (*wp).w_status_height == 0 && !is_stl_global && !edit_submode.is_null() {
            return;
        }

        win_redr_stl_expr(wp, false, true, ui_has(kUIMessages));
        did_show_ruler = !ui_has(kUIMessages);
    }
}

/// Get the character to use in a status line. Get its attributes in `*group`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fillchar_status(group: *mut hlf_T, wp: *mut win_T) -> schar_T {
    unsafe {
        if wp == curwin {
            *group = HLF_S;
            (*wp).w_p_fcs_chars.stl
        } else {
            *group = HLF_SNC;
            (*wp).w_p_fcs_chars.stlnc
        }
    }
}

unsafe fn ui_ext_tabline_update() {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let name_buff = (&raw mut NameBuff).cast::<c_char>();

        let mut n_tabs: usize = 0;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            n_tabs += 1;
            tp = (*tp).tp_next;
        }

        let mut tabs = arena_array(&raw mut arena, n_tabs);
        tp = first_tabpage;
        while !tp.is_null() {
            let mut tab_info = arena_dict(&raw mut arena, 2);
            put_c(
                &mut tab_info,
                c"tab".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeTabpage,
                    data: ObjectData {
                        integer: i64::from((*tp).handle),
                    },
                },
            );

            let cwp = if tp == curtab {
                curwin
            } else {
                (*tp).tp_curwin
            };
            get_trans_bufname((*cwp).w_buffer);
            put_c(
                &mut tab_info,
                c"name".as_ptr(),
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: arena_string(&raw mut arena, cstr_as_string(name_buff)),
                    },
                },
            );

            add_c(
                &mut tabs,
                Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData { dict: tab_info },
                },
            );
            tp = (*tp).tp_next;
        }

        let mut n_buffers: usize = 0;
        let mut buf = firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_bl != 0 {
                n_buffers += 1;
            }
            buf = (*buf).b_next;
        }

        let mut buffers = arena_array(&raw mut arena, n_buffers);
        buf = firstbuf;
        while !buf.is_null() {
            if (*buf).b_p_bl != 0 {
                let mut buffer_info = arena_dict(&raw mut arena, 2);
                put_c(
                    &mut buffer_info,
                    c"buffer".as_ptr(),
                    Object {
                        obj_type: ObjectType::kObjectTypeBuffer,
                        data: ObjectData {
                            integer: i64::from((*buf).b_fnum),
                        },
                    },
                );

                get_trans_bufname(buf);
                put_c(
                    &mut buffer_info,
                    c"name".as_ptr(),
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: arena_string(&raw mut arena, cstr_as_string(name_buff)),
                        },
                    },
                );

                add_c(
                    &mut buffers,
                    Object {
                        obj_type: ObjectType::kObjectTypeDict,
                        data: ObjectData { dict: buffer_info },
                    },
                );
            }
            buf = (*buf).b_next;
        }

        ui_call_tabline_update((*curtab).handle, tabs, (*curbuf).b_fnum, buffers);
        arena_mem_free(arena_finish(&raw mut arena));
    }
}

/// Draw the tab pages line at the top of the Vim window.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn draw_tabline() {
    unsafe {
        let attr_fill = hl_attr(HLF_TPF);
        let attr_nosel = hl_combine_attr(attr_fill, hl_attr(HLF_TP));
        let use_sep_chars = t_colors < 8;
        let name_buff = (&raw mut NameBuff).cast::<c_char>();

        if default_grid.chars.is_null() {
            return;
        }
        redraw_tabline = false;

        if ui_has(kUITabline) {
            ui_ext_tabline_update();
            return;
        }

        if tabline_height() < 1 {
            return;
        }

        debug_assert!(tab_page_click_defs_size >= Columns as usize);
        stl_clear_click_defs(tab_page_click_defs, tab_page_click_defs_size);

        if *p_tal != NUL {
            win_redr_stl_expr(ptr::null_mut(), false, false, false);
        } else {
            let mut tabcount: c_int = 0;
            let mut col: c_int = 0;
            grid_line_start(&raw mut default_gridview, 0);
            let mut tp = first_tabpage;
            while !tp.is_null() {
                tabcount += 1;
                tp = (*tp).tp_next;
            }

            let tabwidth = (if tabcount > 0 {
                (Columns - 1 + tabcount / 2) / tabcount
            } else {
                0
            })
            .max(6);

            let mut attr = attr_nosel;
            tabcount = 0;

            tp = first_tabpage;
            while !tp.is_null() {
                if col >= Columns - 4 {
                    break;
                }

                let mut scol = col;
                let (cwp, mut wp) = if tp == curtab {
                    (curwin, firstwin)
                } else {
                    ((*tp).tp_curwin, (*tp).tp_firstwin)
                };

                if (*tp).tp_topframe == topframe.cast() {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(cwp, HLF_TPS));
                }
                if use_sep_chars && col > 0 {
                    grid_line_put_schar(col, schar_from_ascii(b'|'), attr);
                    col += 1;
                }

                if (*tp).tp_topframe != topframe.cast() {
                    attr = hl_combine_attr(attr_fill, win_hl_attr(cwp, HLF_TP));
                }

                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                let mut modified = false;
                let mut wincount: c_int = 0;
                while !wp.is_null() {
                    if !(*wp).w_config.focusable || (*wp).w_config.hide {
                        wincount -= 1;
                    } else if bufIsChanged((*wp).w_buffer) {
                        modified = true;
                    }
                    wp = (*wp).w_next;
                    wincount += 1;
                }

                if modified || wincount > 1 {
                    if wincount > 1 {
                        let len = vim_snprintf!(name_buff, MAXPATHL, c"%d".as_ptr(), wincount);
                        if col + len >= Columns - 3 {
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
                    let n = Columns - col - 1;
                    len = len.min(n);

                    grid_line_puts(col, p, -1, attr);
                    col += len;
                }
                grid_line_put_schar(col, schar_from_ascii(b' '), attr);
                col += 1;

                tabcount += 1;
                while scol < col {
                    *tab_page_click_defs.add(scol as usize) = StlClickDefinition {
                        type_: kStlClickTabSwitch,
                        tabnr: tabcount,
                        func: ptr::null_mut(),
                    };
                    scol += 1;
                }

                tp = (*tp).tp_next;
            }

            for scol in col..Columns {
                *tab_page_click_defs.add(scol as usize) = StlClickDefinition {
                    type_: kStlClickTabSwitch,
                    tabnr: 0,
                    func: ptr::null_mut(),
                };
            }

            let c = if use_sep_chars { b'_' } else { b' ' };
            grid_line_fill(col, Columns, schar_from_ascii(c), attr_fill);

            if p_sc != 0 && *p_sloc == b't' as c_char {
                let n = Columns - col - c_int::from(tabcount > 1) * 3;
                let sc_width = 10.min(n);

                if sc_width > 0 {
                    grid_line_puts(
                        Columns - sc_width - c_int::from(tabcount > 1) * 2,
                        (&raw const showcmd_buf).cast(),
                        sc_width,
                        attr_nosel,
                    );
                }
                showcmd_update_clear_state();
            }

            if tabcount > 1 {
                grid_line_put_schar(Columns - 1, schar_from_ascii(b'X'), attr_nosel);
                *tab_page_click_defs.add((Columns - 1) as usize) = StlClickDefinition {
                    type_: kStlClickTabClose,
                    tabnr: 999,
                    func: ptr::null_mut(),
                };
            }

            grid_line_flush();
        }

        redraw_tabline = false;
    }
}

/// Build the `'statuscolumn'` string for line `lnum`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn build_statuscol_str(
    wp: *mut win_T,
    lnum: linenr_T,
    relnum: c_int,
    virtnum: c_int,
    buf: CharBuf,
    stcp: *mut statuscol_T,
) -> c_int {
    unsafe {
        if relnum >= 0 {
            set_vim_var_nr(VV_LNUM, varnumber_T::from(lnum));
            set_vim_var_nr(VV_RELNUM, varnumber_T::from(relnum));
        }
        set_vim_var_nr(VV_VIRTNUM, varnumber_T::from(virtnum));

        let mut clickrec: *mut StlClickRecord = ptr::null_mut();
        let buf_data = buf.data;
        let stc = xstrdup((*wp).w_onebuf_opt.wo_stc);
        let width = build_stl_str_hl(
            wp,
            buf,
            stc,
            kOptStatuscolumn,
            OPT_LOCAL,
            0,
            (*stcp).width,
            &raw mut (*stcp).hlrec,
            ptr::null_mut(),
            &raw mut clickrec,
            stcp,
        );
        xfree(stc.cast());

        if !(*clickrec).start.is_null() {
            let clicks = map_put_ref_int_StcClicks(
                (*wp).w_statuscol_click_defs.as_mut_ptr(),
                lnum,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            let click_defs =
                map_put_ref_int_StcClick(clicks, virtnum, ptr::null_mut(), ptr::null_mut());
            stl_clear_click_defs((*click_defs).def.cast(), (*click_defs).size);
            (*click_defs).def =
                stl_alloc_click_defs((*click_defs).def.cast(), width, &raw mut (*click_defs).size)
                    .cast();
            stl_fill_click_defs((*click_defs).def.cast(), clickrec, buf_data, width, false);
        }

        width
    }
}

/// Truncates the statusline at the first truncation marker (`%<`) if present.
unsafe fn stl_truncate(
    width: *mut c_int,
    mut minwid: c_int,
    maxwid: c_int,
    fillchar: schar_T,
    stl_items: *mut stl_item_t,
    startitem: c_int,
    curitem: *mut c_int,
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
            trunc_p = (*stl_items.add(item_idx as usize)).start;

            for i in startitem..*curitem {
                if (*stl_items.add(i as usize)).type_ == StlItemType::Trunc {
                    trunc_p = (*stl_items.add(i as usize)).start;
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
                if (*stl_items.add(i as usize)).start > trunc_p {
                    for j in i..*curitem {
                        if (*stl_items.add(j as usize)).type_ == StlItemType::ClickFunc {
                            xfree((*stl_items.add(j as usize)).cmd.cast());
                            (*stl_items.add(j as usize)).cmd = ptr::null_mut();
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
            for i in item_idx..*curitem {
                let item = stl_items.add(i as usize);
                if (*item).start >= trunc_end_p {
                    (*item).start = (*item).start.offset(-item_offset);
                } else {
                    (*item).start = trunc_p;
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
unsafe fn stl_expand(
    width: *mut c_int,
    target_width: c_int,
    padding: StlPadding,
    remaining_capacity: c_int,
    fillchar: schar_T,
    stl_items: *mut stl_item_t,
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
        for i in startitem..curitem {
            if (*stl_items.add(i as usize)).type_ == StlItemType::Separate {
                num_separators += 1;
            }
        }
        if num_separators > 0 {
            let standard_spaces = added_cells / num_separators;
            let final_spaces = added_cells - standard_spaces * (num_separators - 1);
            let mut cumulated_dislocation: isize = 0;

            for i in startitem..curitem {
                let item = stl_items.add(i as usize);
                (*item).start = (*item).start.offset(cumulated_dislocation);

                if (*item).type_ == StlItemType::Separate {
                    num_separators -= 1;
                    let mut dislocation = if num_separators > 0 {
                        standard_spaces
                    } else {
                        final_spaces
                    };
                    dislocation *= fillchar_bytes;
                    cumulated_dislocation += dislocation as isize;

                    let start = (*item).start;
                    let seploc = start.add(dislocation as usize);
                    let len = CStr::from_ptr(start).to_bytes().len() + 1;
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

            for n in startitem..curitem {
                (*stl_items.add(n as usize)).start =
                    (*stl_items.add(n as usize)).start.add(added_bytes as usize);
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
        static mut stl_items_len: usize = 20;
        static mut stl_items: *mut stl_item_t = ptr::null_mut();
        static mut stl_groupitems: *mut c_int = ptr::null_mut();
        static mut stl_hltab: *mut stl_hlrec_t = ptr::null_mut();
        static mut stl_tabtab: *mut StlClickRecord = ptr::null_mut();
        static mut curitem: c_int = 0;

        let mut buf_tmp: [c_char; TMPLEN] = [0; TMPLEN];
        let mut usefmt = fmt;
        let save_redraw_not_allowed = redraw_not_allowed;
        let save_KeyTyped = KeyTyped;
        let did_emsg_before = did_emsg;
        let name_buff = (&raw mut NameBuff).cast::<c_char>();

        if updating_screen {
            redraw_not_allowed = true;
        }

        if stl_items.is_null() {
            stl_items = xmalloc(size_of::<stl_item_t>() * stl_items_len).cast();
            stl_groupitems = xmalloc(size_of::<c_int>() * stl_items_len).cast();
            stl_hltab = xmalloc(size_of::<stl_hlrec_t>() * (stl_items_len + 1)).cast();
            stl_tabtab = xmalloc(size_of::<StlClickRecord>() * (stl_items_len + 1)).cast();
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
                vval: TypvalVval {
                    v_number: varnumber_T::from((*wp).handle),
                },
            };
            set_var(
                c"g:statusline_winid".as_ptr(),
                "g:statusline_winid".len(),
                &raw mut tv,
                false,
            );

            usefmt = eval_to_string_safe(fmt.add(2), use_sandbox, false);
            if usefmt.is_null() {
                usefmt = fmt;
            }

            do_unlet(
                c"g:statusline_winid".as_ptr(),
                "g:statusline_winid".len(),
                true,
            );
        }

        if fillchar == 0 {
            fillchar = schar_from_ascii(b' ');
        }

        let mut lnum = (*wp).w_cursor.lnum;
        if lnum > (*(*wp).w_buffer).b_ml.ml_line_count {
            lnum = (*(*wp).w_buffer).b_ml.ml_line_count;
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
        let evalstart = curitem;

        let mut prevchar_isflag = true;
        let mut prevchar_isitem = false;

        let mut out_p = out.data;
        let out_end_p = out.data.add(out.size - 1);

        macro_rules! may_grow_stl_items_len {
            () => {
                if curitem == stl_items_len as c_int {
                    let new_len = stl_items_len * 3 / 2;
                    stl_items =
                        xrealloc(stl_items.cast(), size_of::<stl_item_t>() * new_len).cast();
                    stl_groupitems =
                        xrealloc(stl_groupitems.cast(), size_of::<c_int>() * new_len).cast();
                    stl_hltab =
                        xrealloc(stl_hltab.cast(), size_of::<stl_hlrec_t>() * (new_len + 1)).cast();
                    stl_tabtab = xrealloc(
                        stl_tabtab.cast(),
                        size_of::<StlClickRecord>() * (new_len + 1),
                    )
                    .cast();
                    stl_items_len = new_len;
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
                (*stl_items.add(curitem as usize)).type_ = StlItemType::Separate;
                (*stl_items.add(curitem as usize)).start = out_p;
                curitem += 1;
                continue;
            }

            if *fmt_p == STL_TRUNCMARK {
                fmt_p = fmt_p.add(1);
                (*stl_items.add(curitem as usize)).type_ = StlItemType::Trunc;
                (*stl_items.add(curitem as usize)).start = out_p;
                curitem += 1;
                continue;
            }

            if *fmt_p == b')' as c_char {
                fmt_p = fmt_p.add(1);
                if groupdepth < 1 {
                    continue;
                }
                groupdepth -= 1;
                let gidx = *stl_groupitems.add(groupdepth);
                if (*stl_items.add(gidx as usize)).type_ != StlItemType::Group {
                    continue;
                }

                let t = (*stl_items.add(gidx as usize)).start;
                *out_p = NUL;
                let mut group_len = vim_strsize(t);

                if curitem > gidx + 1 && (*stl_items.add(gidx as usize)).minwid == 0 {
                    let mut group_start_userhl = 0;
                    let mut group_end_userhl = 0;
                    let mut n = gidx - 1;
                    while n >= 0 {
                        let it = (*stl_items.add(n as usize)).type_;
                        if it == StlItemType::Highlight || it == StlItemType::HighlightCombining {
                            group_start_userhl = (*stl_items.add(n as usize)).minwid;
                            group_end_userhl = group_start_userhl;
                            break;
                        }
                        n -= 1;
                    }
                    n = gidx + 1;
                    while n < curitem {
                        let it = (*stl_items.add(n as usize)).type_;
                        if it == StlItemType::Normal {
                            break;
                        }
                        if it == StlItemType::Highlight || it == StlItemType::HighlightCombining {
                            group_end_userhl = (*stl_items.add(n as usize)).minwid;
                        }
                        n += 1;
                    }
                    if n == curitem && group_start_userhl == group_end_userhl {
                        out_p = t;
                        group_len = 0;
                        for idx in (gidx + 1)..curitem {
                            let item = stl_items.add(idx as usize);
                            if (*item).type_ == StlItemType::Highlight
                                || (*item).type_ == StlItemType::HighlightCombining
                            {
                                (*item).type_ = StlItemType::Disabled;
                            }
                            if (*item).type_ == StlItemType::TabPage {
                                (*item).start = out_p;
                            }
                        }
                    }
                }

                let maxwid = (*stl_items.add(gidx as usize)).maxwid;
                let mut minwid = (*stl_items.add(gidx as usize)).minwid;
                let padding = if minwid < 0 {
                    StlPadding::kPaddingRight
                } else {
                    StlPadding::kPaddingLeft
                };
                minwid = minwid.abs();

                if group_len > maxwid
                    && (*stl_items.add(gidx as usize)).type_ != StlItemType::HighlightFold
                {
                    stl_truncate(
                        &raw mut group_len,
                        minwid,
                        maxwid,
                        fillchar,
                        stl_items,
                        gidx + 1,
                        &raw mut curitem,
                        t,
                        &raw mut out_p,
                    );
                } else if minwid > group_len {
                    stl_expand(
                        &raw mut group_len,
                        minwid,
                        padding,
                        out_end_p.offset_from(out_p) as c_int,
                        fillchar,
                        stl_items,
                        gidx + 1,
                        curitem,
                        t,
                        &raw mut out_p,
                    );
                }

                for idx in (gidx + 1)..curitem {
                    let item = stl_items.add(idx as usize);
                    if (*item).type_ == StlItemType::Separate || (*item).type_ == StlItemType::Trunc
                    {
                        (*item).type_ = StlItemType::Disabled;
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
                curr_hl_pos = curitem;
                (*stl_items.add(curitem as usize)).type_ = StlItemType::Highlight;
                (*stl_items.add(curitem as usize)).start = out_p;
                (*stl_items.add(curitem as usize)).minwid = if minwid > 9 { 1 } else { minwid };
                fmt_p = fmt_p.add(1);
                curitem += 1;
                continue;
            }

            if *fmt_p == STL_TABPAGENR || *fmt_p == STL_TABCLOSENR {
                if *fmt_p == STL_TABCLOSENR {
                    if minwid == 0 {
                        let mut n = curitem - 1;
                        while n >= 0 {
                            if (*stl_items.add(n as usize)).type_ == StlItemType::TabPage
                                && (*stl_items.add(n as usize)).minwid >= 0
                            {
                                minwid = (*stl_items.add(n as usize)).minwid;
                                break;
                            }
                            n -= 1;
                        }
                    } else {
                        minwid = -minwid;
                    }
                }
                (*stl_items.add(curitem as usize)).type_ = StlItemType::TabPage;
                (*stl_items.add(curitem as usize)).start = out_p;
                (*stl_items.add(curitem as usize)).minwid = minwid;
                fmt_p = fmt_p.add(1);
                curitem += 1;
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
                (*stl_items.add(curitem as usize)).type_ = StlItemType::ClickFunc;
                (*stl_items.add(curitem as usize)).start = out_p;
                (*stl_items.add(curitem as usize)).cmd = if !tabtab.is_null() {
                    xmemdupz(t.cast(), fmt_p.offset_from(t) as usize).cast()
                } else {
                    ptr::null_mut()
                };
                (*stl_items.add(curitem as usize)).minwid = minwid;
                fmt_p = fmt_p.add(1);
                curitem += 1;
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
                *stl_groupitems.add(groupdepth) = curitem;
                groupdepth += 1;
                (*stl_items.add(curitem as usize)).type_ = StlItemType::Group;
                (*stl_items.add(curitem as usize)).start = out_p;
                (*stl_items.add(curitem as usize)).minwid = minwid;
                (*stl_items.add(curitem as usize)).maxwid = maxwid;
                fmt_p = fmt_p.add(1);
                curitem += 1;
                continue;
            }

            if *fmt_p == b'}' as c_char && evaldepth > 0 {
                fmt_p = fmt_p.add(1);
                evaldepth -= 1;
                if groupdepth > 0 {
                    let mut normal_items = false;
                    let mut i = curitem - 1;
                    while i >= evalstart
                        && (*stl_items.add(i as usize)).type_ != StlItemType::Expression
                    {
                        let it = (*stl_items.add(i as usize)).type_;
                        if it == StlItemType::Normal || it == StlItemType::NormalEmpty {
                            normal_items = true;
                            break;
                        }
                        i -= 1;
                    }
                    if !normal_items {
                        (*stl_items.add(curitem as usize)).type_ = StlItemType::Normal;
                        (*stl_items.add(curitem as usize)).start = out_p;
                        curitem += 1;
                    }
                }
                continue;
            }

            if *fmt_p == STL_HIGHLIGHT {
                if *fmt_p.add(1) == b'(' as c_char {
                    fmt_p = fmt_p.add(2);
                    *stl_groupitems.add(groupdepth) = curitem;
                    groupdepth += 1;
                    (*stl_items.add(curitem as usize)).type_ = StlItemType::HighlightScope;
                    (*stl_items.add(curitem as usize)).start = out_p;
                    (*stl_items.add(curitem as usize)).minwid = curr_hl_pos;
                    curitem += 1;
                    continue;
                } else if *fmt_p.add(1) == b')' as c_char {
                    fmt_p = fmt_p.add(2);
                    if groupdepth < 1 {
                        continue;
                    }
                    groupdepth -= 1;
                    let scope_start = *stl_groupitems.add(groupdepth);
                    if (*stl_items.add(scope_start as usize)).type_ != StlItemType::HighlightScope {
                        continue;
                    }
                    let hl_pos = (*stl_items.add(scope_start as usize)).minwid;
                    curr_hl_pos = curitem;
                    (*stl_items.add(curitem as usize)).type_ = StlItemType::Highlight;
                    (*stl_items.add(curitem as usize)).start = out_p;
                    (*stl_items.add(curitem as usize)).minwid = if hl_pos == -1 {
                        0
                    } else {
                        (*stl_items.add(hl_pos as usize)).minwid
                    };
                    curitem += 1;
                    let start_i = if hl_pos == -1 { evalstart } else { hl_pos + 1 };
                    for i in start_i..scope_start {
                        may_grow_stl_items_len!();
                        if (*stl_items.add(i as usize)).type_ == StlItemType::HighlightCombining {
                            (*stl_items.add(curitem as usize)).type_ =
                                StlItemType::HighlightCombining;
                            (*stl_items.add(curitem as usize)).start = out_p;
                            (*stl_items.add(curitem as usize)).minwid =
                                (*stl_items.add(i as usize)).minwid;
                            curitem += 1;
                        }
                    }
                    continue;
                }
            }

            if vim_strchr(STL_ALL.as_ptr(), c_int::from(*fmt_p as u8)).is_null() {
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

            let mut handle_stcsign = false;
            match opt {
                STL_FILEPATH | STL_FULLPATH | STL_FILENAME => {
                    fillable = false;
                    let name = buf_spname((*wp).w_buffer);
                    if !name.is_null() {
                        xstrlcpy(name_buff, name, MAXPATHL);
                    } else {
                        let t = if opt == STL_FULLPATH {
                            (*(*wp).w_buffer).b_ffname
                        } else {
                            (*(*wp).w_buffer).b_fname
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
                            c"%d".as_ptr(),
                            (*curbuf).b_fnum,
                        );
                        set_internal_string_var(c"g:actual_curbuf".as_ptr(), buf_tmp.as_mut_ptr());
                        vim_snprintf!(
                            buf_tmp.as_mut_ptr(),
                            buf_tmp.len(),
                            c"%d".as_ptr(),
                            (*curwin).handle,
                        );
                        set_internal_string_var(c"g:actual_curwin".as_ptr(), buf_tmp.as_mut_ptr());

                        let save_curbuf = curbuf;
                        let save_curwin = curwin;
                        let save_VIsual_active = Visual.active;
                        curwin = wp;
                        curbuf = (*wp).w_buffer;
                        if curwin != save_curwin {
                            Visual.active = false;
                        }

                        str = eval_to_string_safe(out_p, use_sandbox, false);

                        curwin = save_curwin;
                        curbuf = save_curbuf;
                        Visual.active = save_VIsual_active;

                        do_unlet(c"g:actual_curbuf".as_ptr(), "g:actual_curbuf".len(), true);
                        do_unlet(c"g:actual_curwin".as_ptr(), "g:actual_curwin".len(), true);

                        if !zeropad && !str.is_null() && *str != NUL && *skipdigits(str) == NUL {
                            num = atoi(str);
                            xfree(str.cast());
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
                            let str_length = CStr::from_ptr(str).to_bytes().len();
                            let fmt_length = CStr::from_ptr(fmt_p).to_bytes().len();
                            let new_fmt_len = parsed_usefmt + str_length + fmt_length + 3;
                            let new_fmt: *mut c_char = xmalloc(new_fmt_len).cast();
                            let mut new_fmt_p = new_fmt;

                            ptr::copy_nonoverlapping(usefmt, new_fmt_p, parsed_usefmt);
                            new_fmt_p = new_fmt_p.add(parsed_usefmt);
                            ptr::copy_nonoverlapping(str, new_fmt_p, str_length);
                            new_fmt_p = new_fmt_p.add(str_length);
                            *new_fmt_p = b'%' as c_char;
                            *new_fmt_p.add(1) = b'}' as c_char;
                            new_fmt_p = new_fmt_p.add(2);
                            ptr::copy_nonoverlapping(fmt_p, new_fmt_p, fmt_length);
                            new_fmt_p = new_fmt_p.add(fmt_length);
                            *new_fmt_p = NUL;

                            if usefmt != fmt {
                                xfree(usefmt.cast());
                            }
                            xfree(str.cast());
                            usefmt = new_fmt;
                            fmt_p = usefmt.add(parsed_usefmt);
                            evaldepth += 1;
                            (*stl_items.add(curitem as usize)).type_ = StlItemType::Expression;
                            (*stl_items.add(curitem as usize)).start = out_p;
                            curitem += 1;
                            continue;
                        }
                    }
                }
                STL_LINE => {
                    if !stcp.is_null()
                        && ((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
                        && get_vim_var_nr(VV_VIRTNUM) == 0
                    {
                        if (*wp).w_maxscwidth == SCL_NUM && (*(*stcp).sattrs).text[0] != 0 {
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
                                (*stl_items.add(curitem as usize)).type_ = StlItemType::Separate;
                                (*stl_items.add(curitem as usize)).start = out_p;
                                curitem += 1;
                                may_grow_stl_items_len!();
                            }
                        }
                    } else if stcp.is_null() {
                        num = if ((*(*wp).w_buffer).b_ml.ml_flags & ML_EMPTY) != 0 {
                            0
                        } else {
                            (*wp).w_cursor.lnum
                        };
                    }
                }
                STL_NUMLINES => {
                    num = (*(*wp).w_buffer).b_ml.ml_line_count;
                }
                STL_COLUMN => {
                    num = if (State & MODE_INSERT) == 0 && empty_line {
                        0
                    } else {
                        (*wp).w_cursor.col + 1
                    };
                }
                STL_VIRTCOL | STL_VIRTCOL_ALT => {
                    let virtcol = (*wp).w_virtcol + 1;
                    let col_1 = if (State & MODE_INSERT) == 0 && empty_line {
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
                        i64::from((*(*wp).w_buffer).b_ml.ml_line_count),
                    );
                }
                STL_ALTPERCENT => {
                    get_rel_pos(wp, buf_tmp.as_mut_ptr(), TMPLEN as c_int);
                    str = buf_tmp.as_mut_ptr();
                }
                STL_SHOWCMD => {
                    if p_sc != 0 && (opt_idx == kOptInvalid || find_option(p_sloc) == opt_idx) {
                        str = (&raw mut showcmd_buf).cast();
                    }
                }
                STL_ARGLISTSTAT => {
                    fillable = false;
                    buf_tmp[0] = NUL;
                    if append_arg_number(wp, buf_tmp.as_mut_ptr(), buf_tmp.len()) > 0 {
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_KEYMAP => {
                    fillable = false;
                    if get_keymap_str(
                        wp,
                        c"<%s>".as_ptr().cast_mut(),
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
                    num = (*(*wp).w_buffer).b_fnum;
                }
                STL_OFFSET_X | STL_OFFSET => {
                    if opt == STL_OFFSET_X {
                        base = NumberBase::kNumBaseHexadecimal;
                    }
                    let l = ml_find_line_or_offset(
                        (*wp).w_buffer,
                        (*wp).w_cursor.lnum,
                        ptr::null_mut(),
                        false,
                    );
                    num = if ((*(*wp).w_buffer).b_ml.ml_flags & ML_EMPTY) != 0 || l < 0 {
                        0
                    } else {
                        l + 1
                            + if (State & MODE_INSERT) == 0 && empty_line {
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
                    } else if num == CAR && get_fileformat((*wp).w_buffer) == EOL_MAC {
                        num = NL;
                    }
                }
                STL_ROFLAG | STL_ROFLAG_ALT => {
                    itemisflag = true;
                    if (*(*wp).w_buffer).b_p_ro != 0 {
                        str = if opt == STL_ROFLAG_ALT {
                            c",RO".as_ptr().cast_mut()
                        } else {
                            tr(c"[RO]".as_ptr())
                        };
                    }
                }
                STL_HELPFLAG | STL_HELPFLAG_ALT => {
                    itemisflag = true;
                    if (*(*wp).w_buffer).b_help {
                        str = if opt == STL_HELPFLAG_ALT {
                            c",HLP".as_ptr().cast_mut()
                        } else {
                            tr(c"[Help]".as_ptr())
                        };
                    }
                }
                STL_FOLDCOL | STL_SIGNCOL => {
                    handle_stcsign = true;
                }
                STL_FILETYPE => {
                    let ft = (*(*wp).w_buffer).b_p_ft;
                    if *ft != NUL && CStr::from_ptr(ft).to_bytes().len() < TMPLEN - 3 {
                        vim_snprintf!(buf_tmp.as_mut_ptr(), buf_tmp.len(), c"[%s]".as_ptr(), ft);
                        str = buf_tmp.as_mut_ptr();
                    }
                }
                STL_FILETYPE_ALT => {
                    itemisflag = true;
                    let ft = (*(*wp).w_buffer).b_p_ft;
                    if *ft != NUL && CStr::from_ptr(ft).to_bytes().len() < TMPLEN - 2 {
                        vim_snprintf!(buf_tmp.as_mut_ptr(), buf_tmp.len(), c",%s".as_ptr(), ft);
                        let mut t = buf_tmp.as_mut_ptr();
                        while *t != NUL {
                            *t = toupper(c_int::from(*t as u8)) as c_char;
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
                            tr(c"[Preview]".as_ptr())
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
                        + c_int::from(bufIsChanged((*wp).w_buffer)) * 2
                        + c_int::from((*(*wp).w_buffer).b_p_ma == 0) * 4;
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
                            curr_hl_pos = curitem;
                            (*stl_items.add(curitem as usize)).type_ = StlItemType::Highlight;
                        } else {
                            (*stl_items.add(curitem as usize)).type_ =
                                StlItemType::HighlightCombining;
                        }
                        (*stl_items.add(curitem as usize)).start = out_p;
                        (*stl_items.add(curitem as usize)).minwid =
                            -syn_name2id_len(t, fmt_p.offset_from(t) as usize);
                        curitem += 1;
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
                    foldsignitem = curitem;
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
                        (*stl_items.add(curitem as usize)).minwid =
                            -if use_cursor_line_highlight(wp, lnum) {
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
                        (*stl_items.add(curitem as usize)).start = out_p.add(signlen);
                        if fdc == 0 {
                            let mut sattr = *(*stcp).sattrs.add(i as usize);
                            if sattr.text[0] != 0 && get_vim_var_nr(VV_VIRTNUM) == 0 {
                                signlen += describe_sign_text(
                                    buf_tmp.as_mut_ptr().add(signlen),
                                    sattr.text.as_mut_ptr(),
                                );
                                (*stl_items.add(curitem as usize)).minwid =
                                    -if (*stcp).sign_cul_id != 0 {
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
                                (*stl_items.add(curitem as usize)).minwid = 0;
                            }
                        }
                        (*stl_items.add(curitem as usize)).type_ = if fdc > 0 {
                            StlItemType::HighlightFold
                        } else {
                            StlItemType::HighlightSign
                        };
                        curitem += 1;
                        may_grow_stl_items_len!();
                    }
                    str = buf_tmp.as_mut_ptr();
                }
            }

            debug_assert!(curitem < stl_items_len as c_int);
            (*stl_items.add(curitem as usize)).start = out_p;
            (*stl_items.add(curitem as usize)).type_ = StlItemType::Normal;

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
                        let offset =
                            out_p.offset_from((*stl_items.add(foldsignitem as usize)).start);
                        for i in foldsignitem..curitem {
                            (*stl_items.add(i as usize)).start =
                                (*stl_items.add(i as usize)).start.offset(offset);
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
                    (*stl_items.add(curitem as usize)).type_ = StlItemType::Highlight;
                    (*stl_items.add(curitem as usize)).start = out_p;
                    (*stl_items.add(curitem as usize)).minwid = 0;
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
                (*stl_items.add(curitem as usize)).type_ = StlItemType::NormalEmpty;
            }

            if num >= 0 || (!itemisflag && !str.is_null() && *str != NUL) {
                prevchar_isflag = false;
            }

            if opt == STL_VIM_EXPR {
                xfree(str.cast());
            }

            curitem += 1;
            if left_align_num {
                may_grow_stl_items_len!();
                (*stl_items.add(curitem as usize)).type_ = StlItemType::Separate;
                (*stl_items.add(curitem as usize)).start = out_p;
                curitem += 1;
            }
        }

        *out_p = NUL;

        if usefmt != fmt {
            xfree(usefmt.cast());
        }

        let mut width = vim_strsize(out.data);
        if maxwidth > 0 && width > maxwidth && (stcp.is_null() || width > MAX_STCWIDTH) {
            stl_truncate(
                &raw mut width,
                0,
                maxwidth,
                fillchar,
                stl_items,
                evalstart,
                &raw mut curitem,
                out.data,
                &raw mut out_p,
            );
        } else if width < maxwidth {
            stl_expand(
                &raw mut width,
                maxwidth,
                StlPadding::kPaddingNone,
                out_end_p.offset_from(out_p) as c_int,
                fillchar,
                stl_items,
                evalstart,
                curitem,
                out.data,
                &raw mut out_p,
            );
        }

        if !hltab.is_null() {
            *hltab = stl_hltab;
            let mut sp = stl_hltab;
            for l in evalstart..curitem {
                let item = *stl_items.add(l as usize);
                if matches!(
                    item.type_,
                    StlItemType::Highlight
                        | StlItemType::HighlightCombining
                        | StlItemType::HighlightFold
                        | StlItemType::HighlightSign
                ) {
                    (*sp).start = item.start;
                    (*sp).userhl = item.minwid;
                    (*sp).item = match item.type_ {
                        StlItemType::HighlightSign => c_int::from(STL_SIGNCOL as u8),
                        StlItemType::HighlightFold => c_int::from(STL_FOLDCOL as u8),
                        StlItemType::HighlightCombining => c_int::from(STL_HIGHLIGHT_COMB as u8),
                        _ => 0,
                    };
                    sp = sp.add(1);
                }
            }
            (*sp).start = ptr::null_mut();
            (*sp).userhl = 0;
        }
        if !hltab_len.is_null() {
            *hltab_len = (curitem - evalstart) as usize;
        }

        if !tabtab.is_null() {
            *tabtab = stl_tabtab;
            let mut cur_tab_rec = stl_tabtab;
            for l in evalstart..curitem {
                let item = *stl_items.add(l as usize);
                if item.type_ == StlItemType::TabPage {
                    (*cur_tab_rec).start = item.start;
                    if item.minwid == 0 {
                        (*cur_tab_rec).def.type_ = kStlClickDisabled;
                        (*cur_tab_rec).def.tabnr = 0;
                    } else {
                        let mut tabnr = item.minwid;
                        if item.minwid > 0 {
                            (*cur_tab_rec).def.type_ = kStlClickTabSwitch;
                        } else {
                            (*cur_tab_rec).def.type_ = kStlClickTabClose;
                            tabnr = -tabnr;
                        }
                        (*cur_tab_rec).def.tabnr = tabnr;
                    }
                    (*cur_tab_rec).def.func = ptr::null_mut();
                    cur_tab_rec = cur_tab_rec.add(1);
                } else if item.type_ == StlItemType::ClickFunc {
                    (*cur_tab_rec).start = item.start;
                    (*cur_tab_rec).def.type_ = kStlClickFuncRun;
                    (*cur_tab_rec).def.tabnr = item.minwid;
                    (*cur_tab_rec).def.func = item.cmd;
                    cur_tab_rec = cur_tab_rec.add(1);
                }
            }
            (*cur_tab_rec).start = ptr::null();
            (*cur_tab_rec).def.type_ = kStlClickDisabled;
            (*cur_tab_rec).def.tabnr = 0;
            (*cur_tab_rec).def.func = ptr::null_mut();
        }

        curitem = evalstart;
        redraw_not_allowed = save_redraw_not_allowed;

        if opt_idx != kOptInvalid && did_emsg > did_emsg_before {
            let def = get_option_default(opt_idx, opt_scope);
            set_option_direct(opt_idx, def, opt_scope, SID_ERROR);
            optval_free_read(opt_idx, def);
        }

        KeyTyped = save_KeyTyped;
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stl_alloc_and_clear_click_defs_manage_buffer() {
        unsafe {
            let mut size: usize = 0;
            let cdp = stl_alloc_click_defs(ptr::null_mut(), 4, &raw mut size);
            assert!(!cdp.is_null());
            assert_eq!(size, 4);
            let fn_str = xstrdup(c"MyClick".as_ptr());
            (*cdp.add(0)).func = fn_str;
            (*cdp.add(1)).func = fn_str;
            stl_clear_click_defs(cdp, size);
            assert!((*cdp.add(0)).func.is_null());
            xfree(cdp.cast());
        }
    }
}
