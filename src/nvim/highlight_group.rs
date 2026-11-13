//! Code for managing highlight groups (`src/nvim/highlight_group.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::has_key;
use crate::api::private::helpers::{
    api_set_error, boolean_obj, cstr_as_obj, cstr_as_string, dict_obj, put_c,
};
use crate::api::private::validate::{Dict, Error, ErrorType, NvimString, Object, ObjectType};
use crate::api::ui::HLATTRS_DICT_SIZE;
use crate::api::win_config::UIExtension;
use crate::autocmd::{EVENT_COLORSCHEME, EVENT_COLORSCHEMEPRE, apply_autocmds};
use crate::buffer::K_UIMESSAGES;
use crate::change::HLF_W;
use crate::charset::{ascii_iswhite, getdigits_int, vim_isprintc, vim_strsize};
use crate::cmdexpand::{EXPAND_HIGHLIGHT, EXPAND_NOTHING, HLF_D};
use crate::cursor_shape::cursor_mode_uses_syn_id;
use crate::decoration::NS;
use crate::decoration_provider::decor_provider_invalidate_hl;
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, HL_UNDERLINE_MASK};
use crate::drawscreen::{HLF_MSG, UPD_NOT_VALID, UPD_SOME_VALID, redraw_all_later};
use crate::errors::*;
use crate::eval::funcs::{
    HL_BLINK, HL_BOLD, HL_CONCEALED, HL_DIM, HL_INVERSE, HL_ITALIC, HL_NOCOMBINE, HL_STANDOUT,
    HL_UNDERCURL, HL_UNDERDASHED, HL_UNDERDOTTED, HL_UNDERDOUBLE, HL_UNDERLINE,
};
use crate::eval::last_set_msg;
use crate::eval::vars::{do_unlet, get_var_value};
use crate::ex_docmd::ends_excmd;
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::globals::*;
use crate::highlight::{
    HL_ALTFONT, HL_DEFAULT, HLATTRS_INIT, HLF_COUNT, HLF_INACTIVE, HLF_NONE, HlAttrs,
    KEYSET_OPTIDX_HIGHLIGHT_BG, KEYSET_OPTIDX_HIGHLIGHT_FG, KEYSET_OPTIDX_HIGHLIGHT_FONT,
    KEYSET_OPTIDX_HIGHLIGHT_SP, KEYSET_OPTIDX_HIGHLIGHT_UPDATE, KeyDict_highlight, OptionalKeys,
    RgbValue, hl_add_font_idx, hl_get_font, hl_get_syn_attr, hl_get_ui_attr, hlattrs2dict,
    ns_get_hl, syn_attr2entry,
};
use crate::lua::converter::Boolean;
use crate::lua::executor::nlua_set_sctx;
use crate::mbyte::{mb_stricmp, mb_strnicmp};
use crate::message::{
    emsg, message_filtered, msg_advance, msg_clr_eos, msg_ext_set_kind, msg_outtrans, msg_putchar,
    msg_puts_hl, msg_source, semsg,
};
use crate::msgpack_rpc::packer::Integer;
use crate::option::{option_was_set, reset_option_was_set, set_option_value_give_err};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::env::ExpandCtx;
use crate::os::time::os_delay;
use crate::runtime::{
    DIP_OPT, DIP_START, Sctx, source_runtime_vim_lua, sourcing_lnum, sourcing_name,
};
use crate::strings::gettext;
use crate::ui::{
    ui_call_hl_group_set, ui_default_colors_set, ui_flush, ui_has, ui_mode_info_set, ui_refresh,
    ui_rgb_attached,
};
use std::ffi::CStr;
use std::io::Write;

const MAX_HL_ID: i32 = 20000;
const MAX_SYN_NAME: usize = 200;

const SG_CTERM: i32 = 2;
const SG_GUI: i32 = 4;
const SG_LINK: i32 = 8;

const K_COLOR_IDX_NONE: i32 = -1;
const K_COLOR_IDX_HEX: i32 = -2;
const K_COLOR_IDX_FG: i32 = -3;
const K_COLOR_IDX_BG: i32 = -4;

const LIST_ATTR: i32 = 1;
const LIST_STRING: i32 = 2;
const LIST_INT: i32 = 3;

const HLF_S: usize = 19;
const HLF_SNC: usize = 20;

const K_UILINEGRID_CONST: UIExtension = 5;

const K_OPT_BACKGROUND: OptIndex = 13;

const KEYSET_OPTIDX_GET_HIGHLIGHT_ID: u64 = 1;
const KEYSET_OPTIDX_GET_HIGHLIGHT_LINK: u64 = 2;
const KEYSET_OPTIDX_GET_HIGHLIGHT_NAME: u64 = 3;
const KEYSET_OPTIDX_GET_HIGHLIGHT_CREATE: u64 = 4;

#[repr(C)]
#[derive(Clone, Default)]
pub struct KeyDict_get_highlight {
    pub is_set_get_highlight: OptionalKeys,
    pub id: Integer,
    pub name: NvimString,
    pub link: Boolean,
    pub create: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColorNameTable {
    pub name: &'static CStr,
    pub color: RgbValue,
}

#[repr(C)]
#[derive(Clone, PartialEq, Eq)]
struct HlGroup {
    sg_name: std::ffi::CString,
    sg_cleared: bool,
    sg_attr: i32,
    sg_link: i32,
    sg_deflink: i32,
    sg_set: i32,
    sg_deflink_sctx: Sctx,
    sg_script_ctx: Sctx,
    sg_cterm: i32,
    sg_cterm_fg: i32,
    sg_cterm_bg: i32,
    sg_cterm_bold: bool,
    sg_gui: i32,
    sg_rgb_fg: RgbValue,
    sg_rgb_bg: RgbValue,
    sg_rgb_sp: RgbValue,
    sg_rgb_fg_idx: i32,
    sg_rgb_bg_idx: i32,
    sg_rgb_sp_idx: i32,
    sg_blend: i32,
    sg_font: Option<std::ffi::CString>,
    sg_parent: i32,
}

impl HlGroup {
    fn new() -> Self {
        Self {
            sg_name: std::ffi::CString::default(),
            sg_cleared: false,
            sg_attr: 0,
            sg_link: 0,
            sg_deflink: 0,
            sg_set: 0,
            sg_deflink_sctx: Sctx::new(),
            sg_script_ctx: Sctx::new(),
            sg_cterm: 0,
            sg_cterm_fg: 0,
            sg_cterm_bg: 0,
            sg_cterm_bold: false,
            sg_gui: 0,
            sg_rgb_fg: 0,
            sg_rgb_bg: 0,
            sg_rgb_sp: 0,
            sg_rgb_fg_idx: 0,
            sg_rgb_bg_idx: 0,
            sg_rgb_sp_idx: 0,
            sg_blend: 0,
            sg_font: None,
            sg_parent: 0,
        }
    }
}

impl Default for HlGroup {
    fn default() -> Self {
        Self::new()
    }
}

struct HighlightGroupCopyState {
    had_both: std::cell::Cell<bool>,
    load_colors_recursive: std::cell::Cell<bool>,
}

impl HighlightGroupCopyState {
    const fn new() -> Self {
        Self {
            had_both: std::cell::Cell::new(false),
            load_colors_recursive: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static HLG_STATE: HighlightGroupCopyState = const { HighlightGroupCopyState::new() };
    static HIGHLIGHT_GA: std::cell::RefCell<Vec<HlGroup>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static HIGHLIGHT_UNAMES: std::cell::RefCell<crate::memory::FxHashMap<Vec<u8>, i32>> = const {
        std::cell::RefCell::new(crate::memory::fx_hashmap())
    };
}

#[inline]
fn with_hl_group<R>(idx: usize, f: impl FnOnce(&HlGroup) -> R) -> R {
    HIGHLIGHT_GA.with(|cell| f(&cell.borrow()[idx]))
}

#[inline]
fn with_hl_group_mut<R>(idx: usize, f: impl FnOnce(&mut HlGroup) -> R) -> R {
    HIGHLIGHT_GA.with(|cell| f(&mut cell.borrow_mut()[idx]))
}

const HL_NAME_TABLE: [&CStr; 18] = [
    c"bold",
    c"standout",
    c"underline",
    c"undercurl",
    c"underdouble",
    c"underdotted",
    c"underdashed",
    c"italic",
    c"reverse",
    c"inverse",
    c"strikethrough",
    c"altfont",
    c"dim",
    c"blink",
    c"conceal",
    c"overline",
    c"nocombine",
    c"NONE",
];

const HL_ATTR_TABLE: [i32; 18] = [
    HL_BOLD,
    HL_STANDOUT,
    HL_UNDERLINE,
    HL_UNDERCURL,
    HL_UNDERDOUBLE,
    HL_UNDERDOTTED,
    HL_UNDERDASHED,
    HL_ITALIC,
    HL_INVERSE,
    HL_INVERSE,
    HL_STRIKETHROUGH,
    HL_ALTFONT,
    HL_DIM,
    HL_BLINK,
    HL_CONCEALED,
    HL_OVERLINE,
    HL_NOCOMBINE,
    0,
];

const COLOR_NAMES: [&CStr; 28] = [
    c"Black",
    c"DarkBlue",
    c"DarkGreen",
    c"DarkCyan",
    c"DarkRed",
    c"DarkMagenta",
    c"Brown",
    c"DarkYellow",
    c"Gray",
    c"Grey",
    c"LightGray",
    c"LightGrey",
    c"DarkGray",
    c"DarkGrey",
    c"Blue",
    c"LightBlue",
    c"Green",
    c"LightGreen",
    c"Cyan",
    c"LightCyan",
    c"Red",
    c"LightRed",
    c"Magenta",
    c"LightMagenta",
    c"Yellow",
    c"LightYellow",
    c"White",
    c"NONE",
];

const COLOR_NUMBERS_16: [i32; 28] = [
    0, 1, 2, 3, 4, 5, 6, 6, 7, 7, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, -1,
];
const COLOR_NUMBERS_88: [i32; 28] = [
    0, 4, 2, 6, 1, 5, 32, 72, 84, 84, 7, 7, 82, 82, 12, 43, 10, 61, 14, 63, 9, 74, 13, 75, 11, 78,
    15, -1,
];
const COLOR_NUMBERS_256: [i32; 28] = [
    0, 4, 2, 6, 1, 5, 130, 3, 248, 248, 7, 7, 242, 242, 12, 81, 10, 121, 14, 159, 9, 224, 13, 225,
    11, 229, 15, -1,
];
const COLOR_NUMBERS_8: [i32; 28] = [
    0,
    4,
    2,
    6,
    1,
    5,
    3,
    3,
    7,
    7,
    7,
    7,
    8,
    8,
    4 + 8,
    4 + 8,
    2 + 8,
    2 + 8,
    6 + 8,
    6 + 8,
    1 + 8,
    1 + 8,
    5 + 8,
    5 + 8,
    3 + 8,
    3 + 8,
    7 + 8,
    -1,
];

const E_HIGHLIGHT_GROUP_NAME_NOT_FOUND_STR: &CStr = c"E411: Highlight group not found: %s";
const E_GROUP_HAS_SETTINGS_HIGHLIGHT_LINK_IGNORED: &CStr =
    c"E414: Group has settings, highlight link ignored";
const E_UNEXPECTED_EQUAL_SIGN_STR: &CStr = c"E415: Unexpected equal sign: %s";
const E_MISSING_EQUAL_SIGN_STR_2: &CStr = c"E416: Missing equal sign: %s";
const E_MISSING_ARGUMENT_STR: &CStr = c"E417: Missing argument: %s";

#[inline]
fn write_hex6(buf: &mut [u8; 8], val: i32) {
    let mut cursor = std::io::Cursor::new(&mut buf[..7]);
    let _ = write!(cursor, "#{:06x}", (val as u32) & 0xffffff);
    buf[7] = 0;
}

#[inline]
fn write_dec(buf: &mut [u8], val: i32) {
    let cap = buf.len();
    let mut cursor = std::io::Cursor::new(&mut buf[..cap - 1]);
    let _ = write!(cursor, "{val}");
    let pos = cursor.position() as usize;
    buf[pos] = 0;
}

#[inline]
fn hl_len() -> i32 {
    HIGHLIGHT_GA.with(|cell| cell.borrow().len() as i32)
}

#[inline]
const fn rgb(r: RgbValue, g: RgbValue, b: RgbValue) -> RgbValue {
    (r << 16) | (g << 8) | b
}

const HIGHLIGHT_INIT_BOTH: [&CStr; 182] = [
    c"Cursor            guifg=bg      guibg=fg                cterm=reverse",
    c"CursorLineNr      gui=bold      cterm=bold",
    c"Ignore            guifg=bg      ctermfg=0",
    c"PmenuMatch        gui=bold      cterm=bold",
    c"PmenuMatchSel     gui=bold      cterm=bold",
    c"PmenuSel          gui=reverse   cterm=reverse,underline blend=0",
    c"RedrawDebugNormal gui=reverse   cterm=reverse",
    c"TabLineSel        guifg=fg      guibg=bg                gui=bold cterm=nocombine",
    c"TermCursor        gui=reverse   cterm=reverse",
    c"Underlined        gui=underline cterm=underline",
    c"Bold              gui=bold        cterm=bold",
    c"Italic            gui=italic      cterm=italic",
    c"BoldItalic        gui=bold,italic cterm=bold,italic",
    c"lCursor           guifg=bg      guibg=fg                cterm=reverse",
    // UI
    c"default link CursorIM         Cursor",
    c"default link CursorLineFold   FoldColumn",
    c"default link CursorLineSign   SignColumn",
    c"default link DiffTextAdd      DiffText",
    c"default link EndOfBuffer      NonText",
    c"default link FloatBorder      NormalFloat",
    c"default link FloatFooter      FloatTitle",
    c"default link FloatTitle       Title",
    c"default link FoldColumn       SignColumn",
    c"default link IncSearch        CurSearch",
    c"default link LineNrAbove      LineNr",
    c"default link LineNrBelow      LineNr",
    c"default link MsgSeparator     StatusLine",
    c"default link MsgArea          NONE",
    c"default link NormalNC         NONE",
    c"default link PmenuExtra       Pmenu",
    c"default link PmenuExtraSel    PmenuSel",
    c"default link PmenuKind        Pmenu",
    c"default link PmenuKindSel     PmenuSel",
    c"default link PmenuSbar        Pmenu",
    c"default link PmenuBorder        Pmenu",
    c"default link PmenuShadow        FloatShadow",
    c"default link PmenuShadowThrough FloatShadowThrough",
    c"default link PreInsert        Added",
    c"default link ComplMatchIns    NONE",
    c"default link ComplHint        NonText",
    c"default link ComplHintMore    MoreMsg",
    c"default link MCursor          CurSearch",
    c"default link MCursorVisual    Visual",
    c"default link Substitute       Search",
    c"default link StatusLineTerm   StatusLine",
    c"default link StatusLineTermNC StatusLineNC",
    c"default link StatusLineRemote DiffAdd",
    c"default link StderrMsg        ErrorMsg",
    c"default link StdoutMsg        NONE",
    c"default link TabLine          TabLineFill",
    c"default link TabLineFill      StatusLineNC",
    c"default link VertSplit        WinSeparator",
    c"default link VisualNOS        Visual",
    c"default link Whitespace       NonText",
    c"default link WildMenu         PmenuSel",
    c"default link WinSeparator     Normal",
    // Syntax
    c"default link Character      Constant",
    c"default link Number         Constant",
    c"default link Boolean        Constant",
    c"default link Regexp         Constant",
    c"default link Float          Number",
    c"default link Conditional    Statement",
    c"default link Repeat         Statement",
    c"default link Label          Statement",
    c"default link Keyword        Statement",
    c"default link Exception      Statement",
    c"default link Include        PreProc",
    c"default link Define         PreProc",
    c"default link Macro          PreProc",
    c"default link PreCondit      PreProc",
    c"default link StorageClass   Type",
    c"default link Structure      Type",
    c"default link Typedef        Type",
    c"default link Tag            Special",
    c"default link SpecialChar    Special",
    c"default link SpecialComment Special",
    c"default link Debug          Special",
    c"default link SpecialKey     Special",
    c"default link Dimmed         Comment",
    // Built-in LSP
    c"default link LspCodeLens                 NonText",
    c"default link LspCodeLensSeparator        LspCodeLens",
    c"default link LspInlayHint                NonText",
    c"default link LspReferenceRead            LspReferenceText",
    c"default link LspReferenceText            Visual",
    c"default link LspReferenceWrite           LspReferenceText",
    c"default link LspReferenceTarget          LspReferenceText",
    c"default link LspSignatureActiveParameter Visual",
    c"default link SnippetTabstop              Visual",
    c"default link SnippetTabstopActive        SnippetTabstop",
    // Diagnostic
    c"default link DiagnosticFloatingError    DiagnosticError",
    c"default link DiagnosticFloatingWarn     DiagnosticWarn",
    c"default link DiagnosticFloatingInfo     DiagnosticInfo",
    c"default link DiagnosticFloatingHint     DiagnosticHint",
    c"default link DiagnosticFloatingOk       DiagnosticOk",
    c"default link DiagnosticVirtualTextError DiagnosticError",
    c"default link DiagnosticVirtualTextWarn  DiagnosticWarn",
    c"default link DiagnosticVirtualTextInfo  DiagnosticInfo",
    c"default link DiagnosticVirtualTextHint  DiagnosticHint",
    c"default link DiagnosticVirtualTextOk    DiagnosticOk",
    c"default link DiagnosticVirtualLinesError DiagnosticError",
    c"default link DiagnosticVirtualLinesWarn  DiagnosticWarn",
    c"default link DiagnosticVirtualLinesInfo  DiagnosticInfo",
    c"default link DiagnosticVirtualLinesHint  DiagnosticHint",
    c"default link DiagnosticVirtualLinesOk    DiagnosticOk",
    c"default link DiagnosticSignError        DiagnosticError",
    c"default link DiagnosticSignWarn         DiagnosticWarn",
    c"default link DiagnosticSignInfo         DiagnosticInfo",
    c"default link DiagnosticSignHint         DiagnosticHint",
    c"default link DiagnosticSignOk           DiagnosticOk",
    c"default link DiagnosticUnnecessary      Comment",
    // Treesitter standard groups
    c"default link @variable.builtin           Special",
    c"default link @variable.parameter.builtin Special",
    c"default link @constant         Constant",
    c"default link @constant.builtin Special",
    c"default link @module         Structure",
    c"default link @module.builtin Special",
    c"default link @label          Label",
    c"default link @string             String",
    c"default link @string.regexp      @string.special",
    c"default link @string.escape      @string.special",
    c"default link @string.special     SpecialChar",
    c"default link @string.special.url Underlined",
    c"default link @character         Character",
    c"default link @character.special SpecialChar",
    c"default link @boolean      Boolean",
    c"default link @number       Number",
    c"default link @number.float Float",
    c"default link @type         Type",
    c"default link @type.builtin Special",
    c"default link @attribute         Macro",
    c"default link @attribute.builtin Special",
    c"default link @property          Identifier",
    c"default link @function         Function",
    c"default link @function.builtin Special",
    c"default link @constructor Special",
    c"default link @operator    Operator",
    c"default link @keyword Keyword",
    c"default link @punctuation         Delimiter",
    c"default link @punctuation.special Special",
    c"default link @comment Comment",
    c"default link @comment.error   DiagnosticError",
    c"default link @comment.warning DiagnosticWarn",
    c"default link @comment.note    DiagnosticInfo",
    c"default link @comment.todo    Todo",
    c"@markup.strong        gui=bold          cterm=bold",
    c"@markup.italic        gui=italic        cterm=italic",
    c"@markup.strikethrough gui=strikethrough cterm=strikethrough",
    c"@markup.underline     gui=underline     cterm=underline",
    c"default link @markup         Special",
    c"default link @markup.heading Title",
    c"default link @markup.link    Underlined",
    c"default link @diff.plus  Added",
    c"default link @diff.minus Removed",
    c"default link @diff.delta Changed",
    c"default link @tag         Tag",
    c"default link @tag.builtin Special",
    // :help
    c"default @markup.heading.1.delimiter.vimdoc guibg=bg guifg=bg guisp=fg gui=underdouble,nocombine ctermbg=NONE ctermfg=NONE cterm=underdouble,nocombine",
    c"default @markup.heading.2.delimiter.vimdoc guibg=bg guifg=bg guisp=fg gui=underline,nocombine ctermbg=NONE ctermfg=NONE cterm=underline,nocombine",
    // LSP semantic tokens
    c"default link @lsp.type.class         @type",
    c"default link @lsp.type.comment       @comment",
    c"default link @lsp.type.decorator     @attribute",
    c"default link @lsp.type.enum          @type",
    c"default link @lsp.type.enumMember    @constant",
    c"default link @lsp.type.event         @type",
    c"default link @lsp.type.function      @function",
    c"default link @lsp.type.interface     @type",
    c"default link @lsp.type.keyword       @keyword",
    c"default link @lsp.type.macro         @constant.macro",
    c"default link @lsp.type.method        @function.method",
    c"default link @lsp.type.modifier      @type.qualifier",
    c"default link @lsp.type.namespace     @module",
    c"default link @lsp.type.number        @number",
    c"default link @lsp.type.operator      @operator",
    c"default link @lsp.type.parameter     @variable.parameter",
    c"default link @lsp.type.property      @property",
    c"default link @lsp.type.regexp        @string.regexp",
    c"default link @lsp.type.string        @string",
    c"default link @lsp.type.struct        @type",
    c"default link @lsp.type.type          @type",
    c"default link @lsp.type.typeParameter @type.definition",
    c"default link @lsp.type.variable      @variable",
    c"default link @lsp.mod.deprecated DiagnosticDeprecated",
];

const HIGHLIGHT_INIT_LIGHT: [&CStr; 70] = [
    c"Normal guifg=NvimDarkGrey2 guibg=NvimLightGrey2 ctermfg=NONE ctermbg=NONE",
    // UI
    c"Added                guifg=NvimDarkGreen                                  ctermfg=2",
    c"Changed              guifg=NvimDarkCyan                                   ctermfg=6",
    c"ColorColumn                               guibg=NvimLightGrey4            cterm=reverse",
    c"Conceal              guifg=NvimLightGrey4",
    c"CurSearch            guifg=NvimLightGrey1 guibg=NvimDarkYellow            ctermfg=15 ctermbg=3",
    c"CursorColumn                              guibg=NvimLightGrey3",
    c"CursorLine                                guibg=NvimLightGrey3",
    c"DiffAdd              guifg=NvimDarkGrey1  guibg=NvimLightGreen            ctermfg=15 ctermbg=2",
    c"DiffChange           guifg=NvimDarkGrey1  guibg=NvimLightGrey4",
    c"DiffDelete           guifg=NvimDarkRed                          gui=bold  ctermfg=1 cterm=bold",
    c"DiffText             guifg=NvimDarkGrey1  guibg=NvimLightCyan             ctermfg=15 ctermbg=6",
    c"Directory            guifg=NvimDarkCyan                                   ctermfg=6",
    c"ErrorMsg             guifg=NvimDarkRed                                    ctermfg=1",
    c"FloatShadow                               guibg=NvimLightGrey4            ctermbg=0 blend=80",
    c"FloatShadowThrough                        guibg=NvimLightGrey4            ctermbg=0 blend=100",
    c"Folded               guifg=NvimDarkGrey4  guibg=NvimLightGrey1",
    c"LineNr               guifg=NvimLightGrey4",
    c"MatchParen                                guibg=NvimLightGrey4  gui=bold  cterm=bold,underline",
    c"ModeMsg              guifg=NvimDarkGreen                                  ctermfg=2",
    c"MoreMsg              guifg=NvimDarkCyan                                   ctermfg=6",
    c"NonText              guifg=NvimLightGrey4",
    c"NormalFloat                               guibg=NvimLightGrey1",
    c"OkMsg                guifg=NvimDarkGreen                                  ctermfg=2",
    c"Pmenu                                     guibg=NvimLightGrey3            cterm=reverse",
    c"PmenuThumb                                guibg=NvimLightGrey4",
    c"Question             guifg=NvimDarkCyan                                   ctermfg=6",
    c"QuickFixLine         guifg=NvimDarkCyan                                   ctermfg=6",
    c"RedrawDebugClear                          guibg=NvimLightYellow           ctermfg=15 ctermbg=3",
    c"RedrawDebugComposed                       guibg=NvimLightGreen            ctermfg=15 ctermbg=2",
    c"RedrawDebugRecompose                      guibg=NvimLightRed              ctermfg=15 ctermbg=1",
    c"Removed              guifg=NvimDarkRed                                    ctermfg=1",
    c"Search               guifg=NvimDarkGrey1  guibg=NvimLightYellow           ctermfg=15 ctermbg=3",
    c"SignColumn           guifg=NvimLightGrey4",
    c"SpellBad             guisp=NvimDarkRed    gui=undercurl                   cterm=undercurl",
    c"SpellCap             guisp=NvimDarkYellow gui=undercurl                   cterm=undercurl",
    c"SpellLocal           guisp=NvimDarkGreen  gui=undercurl                   cterm=undercurl",
    c"SpellRare            guisp=NvimDarkCyan   gui=undercurl                   cterm=undercurl",
    c"StatusLine           guifg=NvimDarkGrey2  guibg=NvimLightGrey4            cterm=reverse",
    c"StatusLineNC         guifg=NvimDarkGrey3  guibg=NvimLightGrey3            cterm=bold,underline",
    c"Title                guifg=NvimDarkGrey2                        gui=bold  cterm=bold",
    c"Visual                                    guibg=NvimLightGrey4            ctermfg=15 ctermbg=0",
    c"WarningMsg           guifg=NvimDarkYellow                                 ctermfg=3",
    c"WinBar               guifg=NvimDarkGrey4  guibg=NvimLightGrey1  gui=bold  cterm=bold",
    c"WinBarNC             guifg=NvimDarkGrey4  guibg=NvimLightGrey1            cterm=bold",
    // Syntax
    c"Constant   guifg=NvimDarkGrey2",
    c"Operator   guifg=NvimDarkGrey2",
    c"PreProc    guifg=NvimDarkGrey2",
    c"Type       guifg=NvimDarkGrey2",
    c"Delimiter  guifg=NvimDarkGrey2",
    c"Comment    guifg=NvimDarkGrey4",
    c"String     guifg=NvimDarkGreen                    ctermfg=2",
    c"Identifier guifg=NvimDarkBlue                     ctermfg=4",
    c"Function   guifg=NvimDarkCyan                     ctermfg=6",
    c"Statement  guifg=NvimDarkGrey2 gui=bold           cterm=bold",
    c"Special    guifg=NvimDarkCyan                     ctermfg=6",
    c"Error      guifg=NvimDarkGrey1 guibg=NvimLightRed ctermfg=15 ctermbg=1",
    c"Todo       guifg=NvimDarkGrey2 gui=bold           cterm=bold",
    // Diagnostic
    c"DiagnosticError          guifg=NvimDarkRed                      ctermfg=1",
    c"DiagnosticWarn           guifg=NvimDarkYellow                   ctermfg=3",
    c"DiagnosticInfo           guifg=NvimDarkCyan                     ctermfg=6",
    c"DiagnosticHint           guifg=NvimDarkBlue                     ctermfg=4",
    c"DiagnosticOk             guifg=NvimDarkGreen                    ctermfg=2",
    c"DiagnosticUnderlineError guisp=NvimDarkRed    gui=underline     cterm=underline",
    c"DiagnosticUnderlineWarn  guisp=NvimDarkYellow gui=underline     cterm=underline",
    c"DiagnosticUnderlineInfo  guisp=NvimDarkCyan   gui=underline     cterm=underline",
    c"DiagnosticUnderlineHint  guisp=NvimDarkBlue   gui=underline     cterm=underline",
    c"DiagnosticUnderlineOk    guisp=NvimDarkGreen  gui=underline     cterm=underline",
    c"DiagnosticDeprecated     guisp=NvimDarkRed    gui=strikethrough cterm=strikethrough",
    // Treesitter standard groups
    c"@variable guifg=NvimDarkGrey2",
];

const HIGHLIGHT_INIT_DARK: [&CStr; 70] = [
    c"Normal guifg=NvimLightGrey2 guibg=NvimDarkGrey2 ctermfg=NONE ctermbg=NONE",
    // UI
    c"Added                guifg=NvimLightGreen                                ctermfg=10",
    c"Changed              guifg=NvimLightCyan                                 ctermfg=14",
    c"ColorColumn                                guibg=NvimDarkGrey4           cterm=reverse",
    c"Conceal              guifg=NvimDarkGrey4",
    c"CurSearch            guifg=NvimDarkGrey1   guibg=NvimLightYellow         ctermfg=0 ctermbg=11",
    c"CursorColumn                               guibg=NvimDarkGrey3",
    c"CursorLine                                 guibg=NvimDarkGrey3",
    c"DiffAdd              guifg=NvimLightGrey1  guibg=NvimDarkGreen           ctermfg=0 ctermbg=10",
    c"DiffChange           guifg=NvimLightGrey1  guibg=NvimDarkGrey4",
    c"DiffDelete           guifg=NvimLightRed                         gui=bold ctermfg=9 cterm=bold",
    c"DiffText             guifg=NvimLightGrey1  guibg=NvimDarkCyan            ctermfg=0 ctermbg=14",
    c"Directory            guifg=NvimLightCyan                                 ctermfg=14",
    c"ErrorMsg             guifg=NvimLightRed                                  ctermfg=9",
    c"FloatShadow                                guibg=NvimDarkGrey4           ctermbg=0 blend=80",
    c"FloatShadowThrough                         guibg=NvimDarkGrey4           ctermbg=0 blend=100",
    c"Folded               guifg=NvimLightGrey4  guibg=NvimDarkGrey1",
    c"LineNr               guifg=NvimDarkGrey4",
    c"MatchParen                                 guibg=NvimDarkGrey4  gui=bold cterm=bold,underline",
    c"ModeMsg              guifg=NvimLightGreen                                ctermfg=10",
    c"MoreMsg              guifg=NvimLightCyan                                 ctermfg=14",
    c"NonText              guifg=NvimDarkGrey4",
    c"NormalFloat                                guibg=NvimDarkGrey1",
    c"OkMsg                guifg=NvimLightGreen                                ctermfg=10",
    c"Pmenu                                      guibg=NvimDarkGrey3           cterm=reverse",
    c"PmenuThumb                                 guibg=NvimDarkGrey4",
    c"Question             guifg=NvimLightCyan                                 ctermfg=14",
    c"QuickFixLine         guifg=NvimLightCyan                                 ctermfg=14",
    c"RedrawDebugClear                           guibg=NvimDarkYellow          ctermfg=0 ctermbg=11",
    c"RedrawDebugComposed                        guibg=NvimDarkGreen           ctermfg=0 ctermbg=10",
    c"RedrawDebugRecompose                       guibg=NvimDarkRed             ctermfg=0 ctermbg=9",
    c"Removed              guifg=NvimLightRed                                  ctermfg=9",
    c"Search               guifg=NvimLightGrey1  guibg=NvimDarkYellow          ctermfg=0 ctermbg=11",
    c"SignColumn           guifg=NvimDarkGrey4",
    c"SpellBad             guisp=NvimLightRed    gui=undercurl                 cterm=undercurl",
    c"SpellCap             guisp=NvimLightYellow gui=undercurl                 cterm=undercurl",
    c"SpellLocal           guisp=NvimLightGreen  gui=undercurl                 cterm=undercurl",
    c"SpellRare            guisp=NvimLightCyan   gui=undercurl                 cterm=undercurl",
    c"StatusLine           guifg=NvimLightGrey2  guibg=NvimDarkGrey4           cterm=reverse",
    c"StatusLineNC         guifg=NvimLightGrey3  guibg=NvimDarkGrey3           cterm=bold,underline",
    c"Title                guifg=NvimLightGrey2                       gui=bold cterm=bold",
    c"Visual                                     guibg=NvimDarkGrey4           ctermfg=0 ctermbg=15",
    c"WarningMsg           guifg=NvimLightYellow                               ctermfg=11",
    c"WinBar               guifg=NvimLightGrey4  guibg=NvimDarkGrey1  gui=bold cterm=bold",
    c"WinBarNC             guifg=NvimLightGrey4  guibg=NvimDarkGrey1           cterm=bold",
    // Syntax
    c"Constant   guifg=NvimLightGrey2",
    c"Operator   guifg=NvimLightGrey2",
    c"PreProc    guifg=NvimLightGrey2",
    c"Type       guifg=NvimLightGrey2",
    c"Delimiter  guifg=NvimLightGrey2",
    c"Comment    guifg=NvimLightGrey4",
    c"String     guifg=NvimLightGreen                   ctermfg=10",
    c"Identifier guifg=NvimLightBlue                    ctermfg=12",
    c"Function   guifg=NvimLightCyan                    ctermfg=14",
    c"Statement  guifg=NvimLightGrey2 gui=bold          cterm=bold",
    c"Special    guifg=NvimLightCyan                    ctermfg=14",
    c"Error      guifg=NvimLightGrey1 guibg=NvimDarkRed ctermfg=0 ctermbg=9",
    c"Todo       guifg=NvimLightGrey2 gui=bold          cterm=bold",
    // Diagnostic
    c"DiagnosticError          guifg=NvimLightRed                      ctermfg=9",
    c"DiagnosticWarn           guifg=NvimLightYellow                   ctermfg=11",
    c"DiagnosticInfo           guifg=NvimLightCyan                     ctermfg=14",
    c"DiagnosticHint           guifg=NvimLightBlue                     ctermfg=12",
    c"DiagnosticOk             guifg=NvimLightGreen                    ctermfg=10",
    c"DiagnosticUnderlineError guisp=NvimLightRed    gui=underline     cterm=underline",
    c"DiagnosticUnderlineWarn  guisp=NvimLightYellow gui=underline     cterm=underline",
    c"DiagnosticUnderlineInfo  guisp=NvimLightCyan   gui=underline     cterm=underline",
    c"DiagnosticUnderlineHint  guisp=NvimLightBlue   gui=underline     cterm=underline",
    c"DiagnosticUnderlineOk    guisp=NvimDarkGreen   gui=underline     cterm=underline",
    c"DiagnosticDeprecated     guisp=NvimLightRed    gui=strikethrough cterm=strikethrough",
    // Treesitter standard groups
    c"@variable guifg=NvimLightGrey2",
];

pub static HIGHLIGHT_INIT_CMDLINE: [&CStr; 140] = [
    c"NvimInternalError ctermfg=Red ctermbg=Red guifg=Red guibg=Red",
    c"default link NvimAssignment Operator",
    c"default link NvimPlainAssignment NvimAssignment",
    c"default link NvimAugmentedAssignment NvimAssignment",
    c"default link NvimAssignmentWithAddition NvimAugmentedAssignment",
    c"default link NvimAssignmentWithSubtraction NvimAugmentedAssignment",
    c"default link NvimAssignmentWithConcatenation NvimAugmentedAssignment",
    c"default link NvimOperator Operator",
    c"default link NvimUnaryOperator NvimOperator",
    c"default link NvimUnaryPlus NvimUnaryOperator",
    c"default link NvimUnaryMinus NvimUnaryOperator",
    c"default link NvimNot NvimUnaryOperator",
    c"default link NvimBinaryOperator NvimOperator",
    c"default link NvimComparison NvimBinaryOperator",
    c"default link NvimComparisonModifier NvimComparison",
    c"default link NvimBinaryPlus NvimBinaryOperator",
    c"default link NvimBinaryMinus NvimBinaryOperator",
    c"default link NvimConcat NvimBinaryOperator",
    c"default link NvimConcatOrSubscript NvimConcat",
    c"default link NvimOr NvimBinaryOperator",
    c"default link NvimAnd NvimBinaryOperator",
    c"default link NvimMultiplication NvimBinaryOperator",
    c"default link NvimDivision NvimBinaryOperator",
    c"default link NvimMod NvimBinaryOperator",
    c"default link NvimTernary NvimOperator",
    c"default link NvimTernaryColon NvimTernary",
    c"default link NvimParenthesis Delimiter",
    c"default link NvimLambda NvimParenthesis",
    c"default link NvimNestingParenthesis NvimParenthesis",
    c"default link NvimCallingParenthesis NvimParenthesis",
    c"default link NvimSubscript NvimParenthesis",
    c"default link NvimSubscriptBracket NvimSubscript",
    c"default link NvimSubscriptColon NvimSubscript",
    c"default link NvimCurly NvimSubscript",
    c"default link NvimContainer NvimParenthesis",
    c"default link NvimDict NvimContainer",
    c"default link NvimList NvimContainer",
    c"default link NvimIdentifier Identifier",
    c"default link NvimIdentifierScope NvimIdentifier",
    c"default link NvimIdentifierScopeDelimiter NvimIdentifier",
    c"default link NvimIdentifierName NvimIdentifier",
    c"default link NvimIdentifierKey NvimIdentifier",
    c"default link NvimColon Delimiter",
    c"default link NvimComma Delimiter",
    c"default link NvimArrow Delimiter",
    c"default link NvimRegister SpecialChar",
    c"default link NvimNumber Number",
    c"default link NvimFloat NvimNumber",
    c"default link NvimNumberPrefix Type",
    c"default link NvimOptionSigil Type",
    c"default link NvimOptionName NvimIdentifier",
    c"default link NvimOptionScope NvimIdentifierScope",
    c"default link NvimOptionScopeDelimiter NvimIdentifierScopeDelimiter",
    c"default link NvimEnvironmentSigil NvimOptionSigil",
    c"default link NvimEnvironmentName NvimIdentifier",
    c"default link NvimString String",
    c"default link NvimStringBody NvimString",
    c"default link NvimStringQuote NvimString",
    c"default link NvimStringSpecial SpecialChar",
    c"default link NvimSingleQuote NvimStringQuote",
    c"default link NvimSingleQuotedBody NvimStringBody",
    c"default link NvimSingleQuotedQuote NvimStringSpecial",
    c"default link NvimDoubleQuote NvimStringQuote",
    c"default link NvimDoubleQuotedBody NvimStringBody",
    c"default link NvimDoubleQuotedEscape NvimStringSpecial",
    c"default link NvimFigureBrace NvimInternalError",
    c"default link NvimSingleQuotedUnknownEscape NvimInternalError",
    c"default link NvimSpacing Normal",
    c"default link NvimInvalidSingleQuotedUnknownEscape NvimInternalError",
    c"default link NvimInvalid Error",
    c"default link NvimInvalidAssignment NvimInvalid",
    c"default link NvimInvalidPlainAssignment NvimInvalidAssignment",
    c"default link NvimInvalidAugmentedAssignment NvimInvalidAssignment",
    c"default link NvimInvalidAssignmentWithAddition NvimInvalidAugmentedAssignment",
    c"default link NvimInvalidAssignmentWithSubtraction NvimInvalidAugmentedAssignment",
    c"default link NvimInvalidAssignmentWithConcatenation NvimInvalidAugmentedAssignment",
    c"default link NvimInvalidOperator NvimInvalid",
    c"default link NvimInvalidUnaryOperator NvimInvalidOperator",
    c"default link NvimInvalidUnaryPlus NvimInvalidUnaryOperator",
    c"default link NvimInvalidUnaryMinus NvimInvalidUnaryOperator",
    c"default link NvimInvalidNot NvimInvalidUnaryOperator",
    c"default link NvimInvalidBinaryOperator NvimInvalidOperator",
    c"default link NvimInvalidComparison NvimInvalidBinaryOperator",
    c"default link NvimInvalidComparisonModifier NvimInvalidComparison",
    c"default link NvimInvalidBinaryPlus NvimInvalidBinaryOperator",
    c"default link NvimInvalidBinaryMinus NvimInvalidBinaryOperator",
    c"default link NvimInvalidConcat NvimInvalidBinaryOperator",
    c"default link NvimInvalidConcatOrSubscript NvimInvalidConcat",
    c"default link NvimInvalidOr NvimInvalidBinaryOperator",
    c"default link NvimInvalidAnd NvimInvalidBinaryOperator",
    c"default link NvimInvalidMultiplication NvimInvalidBinaryOperator",
    c"default link NvimInvalidDivision NvimInvalidBinaryOperator",
    c"default link NvimInvalidMod NvimInvalidBinaryOperator",
    c"default link NvimInvalidTernary NvimInvalidOperator",
    c"default link NvimInvalidTernaryColon NvimInvalidTernary",
    c"default link NvimInvalidDelimiter NvimInvalid",
    c"default link NvimInvalidParenthesis NvimInvalidDelimiter",
    c"default link NvimInvalidLambda NvimInvalidParenthesis",
    c"default link NvimInvalidNestingParenthesis NvimInvalidParenthesis",
    c"default link NvimInvalidCallingParenthesis NvimInvalidParenthesis",
    c"default link NvimInvalidSubscript NvimInvalidParenthesis",
    c"default link NvimInvalidSubscriptBracket NvimInvalidSubscript",
    c"default link NvimInvalidSubscriptColon NvimInvalidSubscript",
    c"default link NvimInvalidCurly NvimInvalidSubscript",
    c"default link NvimInvalidContainer NvimInvalidParenthesis",
    c"default link NvimInvalidDict NvimInvalidContainer",
    c"default link NvimInvalidList NvimInvalidContainer",
    c"default link NvimInvalidValue NvimInvalid",
    c"default link NvimInvalidIdentifier NvimInvalidValue",
    c"default link NvimInvalidIdentifierScope NvimInvalidIdentifier",
    c"default link NvimInvalidIdentifierScopeDelimiter NvimInvalidIdentifier",
    c"default link NvimInvalidIdentifierName NvimInvalidIdentifier",
    c"default link NvimInvalidIdentifierKey NvimInvalidIdentifier",
    c"default link NvimInvalidColon NvimInvalidDelimiter",
    c"default link NvimInvalidComma NvimInvalidDelimiter",
    c"default link NvimInvalidArrow NvimInvalidDelimiter",
    c"default link NvimInvalidRegister NvimInvalidValue",
    c"default link NvimInvalidNumber NvimInvalidValue",
    c"default link NvimInvalidFloat NvimInvalidNumber",
    c"default link NvimInvalidNumberPrefix NvimInvalidNumber",
    c"default link NvimInvalidOptionSigil NvimInvalidIdentifier",
    c"default link NvimInvalidOptionName NvimInvalidIdentifier",
    c"default link NvimInvalidOptionScope NvimInvalidIdentifierScope",
    c"default link NvimInvalidOptionScopeDelimiter NvimInvalidIdentifierScopeDelimiter",
    c"default link NvimInvalidEnvironmentSigil NvimInvalidOptionSigil",
    c"default link NvimInvalidEnvironmentName NvimInvalidIdentifier",
    c"default link NvimInvalidString NvimInvalidValue",
    c"default link NvimInvalidStringBody NvimStringBody",
    c"default link NvimInvalidStringQuote NvimInvalidString",
    c"default link NvimInvalidStringSpecial NvimStringSpecial",
    c"default link NvimInvalidSingleQuote NvimInvalidStringQuote",
    c"default link NvimInvalidSingleQuotedBody NvimInvalidStringBody",
    c"default link NvimInvalidSingleQuotedQuote NvimInvalidStringSpecial",
    c"default link NvimInvalidDoubleQuote NvimInvalidStringQuote",
    c"default link NvimInvalidDoubleQuotedBody NvimInvalidStringBody",
    c"default link NvimInvalidDoubleQuotedEscape NvimInvalidStringSpecial",
    c"default link NvimInvalidDoubleQuotedUnknownEscape NvimInvalidValue",
    c"default link NvimInvalidFigureBrace NvimInvalidDelimiter",
    c"default link NvimInvalidSpacing ErrorMsg",
    c"default link NvimDoubleQuotedUnknownEscape NvimInvalidValue",
];

pub fn highlight_num_groups() -> i32 {
    hl_len()
}

pub fn highlight_group_name(id: i32) -> std::ffi::CString {
    with_hl_group(id as usize, |g| g.sg_name.clone())
}

pub fn highlight_link_id(id: i32) -> i32 {
    with_hl_group(id as usize, |g| g.sg_link)
}

pub fn syn_init_cmdline_highlight(reset: bool, init: bool) {
    for &cmd in &HIGHLIGHT_INIT_CMDLINE {
        do_highlight(cmd, reset, init);
    }
}

pub fn init_highlight(both: bool, reset: bool) {
    if let Some(p) = get_var_value(c"g:colors_name") {
        let copy_p = p.into_owned();
        let okay = load_colors(&copy_p);
        if okay {
            return;
        }
    }

    if both {
        HLG_STATE.with(|s| s.had_both.set(true));
        for &cmd in &HIGHLIGHT_INIT_BOTH {
            do_highlight(cmd, reset, true);
        }
    } else if !HLG_STATE.with(|s| s.had_both.get()) {
        return;
    }

    let pp = if opt_vars().p_bg.first_byte() == b'l' {
        &HIGHLIGHT_INIT_LIGHT
    } else {
        &HIGHLIGHT_INIT_DARK
    };
    for &cmd in pp {
        do_highlight(cmd, reset, true);
    }

    syn_init_cmdline_highlight(false, false);
}

pub fn load_colors(name: &CStr) -> bool {
    if HLG_STATE.with(|s| s.load_colors_recursive.get()) {
        return true;
    }

    HLG_STATE.with(|s| s.load_colors_recursive.set(true));
    let name_bytes = name.to_bytes();
    let mut buf = Vec::with_capacity(name_bytes.len() + 12);
    apply_autocmds(
        EVENT_COLORSCHEMEPRE,
        Some(name),
        curbuf().fname_opt().as_deref(),
        false,
        curbuf_opt().as_deref(),
    );
    buf.extend_from_slice(b"colors/");
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(b".*\0");
    let retval = source_runtime_vim_lua(crate::memory::bytes_as_cstr(&buf), DIP_START + DIP_OPT);
    if retval {
        apply_autocmds(
            EVENT_COLORSCHEME,
            Some(name),
            curbuf().fname_opt().as_deref(),
            false,
            curbuf_opt().as_deref(),
        );
    }

    HLG_STATE.with(|s| s.load_colors_recursive.set(false));

    retval
}

fn lookup_color(idx: i32, foreground: bool, boldp: &mut TriState) -> i32 {
    let mut color = COLOR_NUMBERS_16[idx as usize];

    if color < 0 {
        return -1;
    }

    if g_st().t_colors.get() == 8 {
        color = COLOR_NUMBERS_8[idx as usize];
        if foreground {
            if (color & 8) != 0 {
                *boldp = K_TRUE;
            } else {
                *boldp = K_FALSE;
            }
        }
        color &= 7;
    } else if g_st().t_colors.get() == 16 {
        color = COLOR_NUMBERS_8[idx as usize];
    } else if g_st().t_colors.get() == 88 {
        color = COLOR_NUMBERS_88[idx as usize];
    } else if g_st().t_colors.get() >= 256 {
        color = COLOR_NUMBERS_256[idx as usize];
    }
    color
}

pub fn set_hl_group(id: i32, attrs: HlAttrs, dict: &KeyDict_highlight, link_id: i32) {
    let idx = (id - 1) as usize;
    let is_default = (attrs.rgb_ae_attr & HL_DEFAULT) != 0;

    if is_default && hl_has_settings(idx as i32, true) && !dict.force {
        return;
    }

    let old_link = with_hl_group(idx, |g| g.sg_link);
    let linked_indices = if old_link > 0 {
        with_hl_group((old_link - 1) as usize, |linked| {
            [
                linked.sg_rgb_fg_idx,
                linked.sg_rgb_bg_idx,
                linked.sg_rgb_sp_idx,
            ]
        })
    } else {
        [K_COLOR_IDX_NONE; 3]
    };

    let update = has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_UPDATE) && dict.update;
    let font_opt = if attrs.font >= 0 {
        hl_get_font(attrs.font)
    } else {
        None
    };
    let syn_attr = hl_get_syn_attr(0, id, attrs);
    let mut sctx = g_st().current_sctx.get();
    sctx.sc_lnum += sourcing_lnum();
    nlua_set_sctx(&mut sctx);

    let (is_normal, sg_cterm_fg, sg_cterm_bg, sg_rgb_fg, sg_rgb_bg, sg_rgb_sp) =
        with_hl_group_mut(idx, |g| {
            g.sg_cleared = false;
            if link_id > 0 {
                g.sg_link = link_id;
                g.sg_script_ctx = sctx;
                g.sg_set |= SG_LINK;
                if is_default {
                    g.sg_deflink = link_id;
                    g.sg_deflink_sctx = sctx;
                }
            } else {
                g.sg_link = 0;
            }

            g.sg_gui = attrs.rgb_ae_attr & !HL_DEFAULT;
            g.sg_rgb_fg = attrs.rgb_fg_color;
            g.sg_rgb_bg = attrs.rgb_bg_color;
            g.sg_rgb_sp = attrs.rgb_sp_color;

            let cinputs: [(RgbValue, Object); 3] = [
                (
                    g.sg_rgb_fg,
                    if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_FG) {
                        dict.fg.clone()
                    } else {
                        dict.foreground.clone()
                    },
                ),
                (
                    g.sg_rgb_bg,
                    if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_BG) {
                        dict.bg.clone()
                    } else {
                        dict.background.clone()
                    },
                ),
                (
                    g.sg_rgb_sp,
                    if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_SP) {
                        dict.sp.clone()
                    } else {
                        dict.special.clone()
                    },
                ),
            ];
            let mut new_indices = [g.sg_rgb_fg_idx, g.sg_rgb_bg_idx, g.sg_rgb_sp_idx];
            for (j, (val, name)) in cinputs.iter().enumerate() {
                let val = *val;
                let dest_ref = &mut new_indices[j];
                if name.obj_type != ObjectType::ObjectTypeNil {
                    if val < 0 {
                        *dest_ref = K_COLOR_IDX_NONE;
                    } else if name.obj_type == ObjectType::ObjectTypeString
                        && name.data.string().size > 0
                    {
                        name_to_color(name.data.as_cstr(), dest_ref);
                    } else {
                        *dest_ref = K_COLOR_IDX_HEX;
                    }
                } else if !update {
                    *dest_ref = K_COLOR_IDX_NONE;
                } else if old_link > 0 && val >= 0 {
                    let linked_idx = linked_indices[j];
                    *dest_ref = if linked_idx != K_COLOR_IDX_NONE {
                        linked_idx
                    } else {
                        K_COLOR_IDX_HEX
                    };
                }
            }
            g.sg_rgb_fg_idx = new_indices[0];
            g.sg_rgb_bg_idx = new_indices[1];
            g.sg_rgb_sp_idx = new_indices[2];

            g.sg_cterm = attrs.cterm_ae_attr & !HL_DEFAULT;
            g.sg_cterm_bg = i32::from(attrs.cterm_bg_color);
            g.sg_cterm_fg = i32::from(attrs.cterm_fg_color);

            g.sg_cterm_bold = (g.sg_cterm & HL_BOLD) != 0;
            if attrs.hl_blend != -1 {
                g.sg_blend = attrs.hl_blend;
            } else if !update {
                g.sg_blend = -1;
            }

            if attrs.font >= 0 {
                g.sg_font = font_opt;
            } else if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_FONT) || !update {
                g.sg_font = None;
            }

            g.sg_script_ctx = sctx;
            g.sg_attr = syn_attr;

            (
                g.sg_name.to_bytes().eq_ignore_ascii_case(b"NORMAL"),
                g.sg_cterm_fg,
                g.sg_cterm_bg,
                g.sg_rgb_fg,
                g.sg_rgb_bg,
                g.sg_rgb_sp,
            )
        });

    if is_normal {
        g_st().cterm_normal_fg_color.set(sg_cterm_fg);
        g_st().cterm_normal_bg_color.set(sg_cterm_bg);
        let did_changed = g_st().normal_bg.get() != sg_rgb_bg
            || g_st().normal_fg.get() != sg_rgb_fg
            || g_st().normal_sp.get() != sg_rgb_sp;
        g_st().normal_fg.set(sg_rgb_fg);
        g_st().normal_bg.set(sg_rgb_bg);
        g_st().normal_sp.set(sg_rgb_sp);

        if did_changed {
            highlight_attr_set_all();
        }
        ui_default_colors_set();
    } else if cursor_mode_uses_syn_id(id) {
        ui_mode_info_set();
    }

    if !g_st().updating_screen.get() {
        redraw_all_later(UPD_NOT_VALID);
    }
    g_st().need_highlight_changed.set(true);
}

fn set_gui_color(idx: i32, init: bool, arg: &CStr, which: u8) -> bool {
    with_hl_group_mut(idx as usize, |g| {
        if init && (g.sg_set & SG_GUI) != 0 {
            return false;
        }
        if !init {
            g.sg_set |= SG_GUI;
        }
        let (color, color_idx) = match which {
            b'F' => (&mut g.sg_rgb_fg, &mut g.sg_rgb_fg_idx),
            b'B' => (&mut g.sg_rgb_bg, &mut g.sg_rgb_bg_idx),
            _ => (&mut g.sg_rgb_sp, &mut g.sg_rgb_sp_idx),
        };
        let old_color = *color;
        let old_idx = *color_idx;
        if arg.to_bytes() != b"NONE" {
            *color = name_to_color(arg, color_idx);
        } else {
            *color = -1;
            *color_idx = K_COLOR_IDX_NONE;
        }
        *color != old_color || *color_idx != old_idx
    })
}

pub fn do_highlight(line_in: &CStr, forceit: bool, init: bool) {
    let mut line = crate::strings::OwnedSlice::from_cstr(line_in);
    {
        if !init && ends_excmd(i32::from(line.first_byte())) != 0 {
            msg_ext_set_kind(c"list_cmd");
            let mut i = 1;
            while i <= hl_len() && !g_st().got_int.get() {
                highlight_list_one(i);
                i += 1;
            }
            return;
        }

        let mut dodefault = false;

        let mut name_end = line.skiptowhite();
        let mut linep = name_end.skipwhite();

        let def_len = name_end.offset_from(&line);
        if line.slice(def_len) == &b"default"[..def_len.min(7)] {
            dodefault = true;
            line = linep;
            name_end = line.skiptowhite();
            linep = name_end.skipwhite();
        }

        let mut doclear = false;
        let mut dolink = false;

        let word_len = name_end.offset_from(&line);
        let word_slice = line.slice(word_len);
        if word_slice == &b"clear"[..word_len.min(5)] {
            doclear = true;
        } else if word_slice == &b"link"[..word_len.min(4)] {
            dolink = true;
        }

        if !doclear && !dolink && ends_excmd(i32::from(linep.first_byte())) != 0 {
            let id = syn_name2id_len(word_slice);
            if id == 0 {
                semsg!(gettext(E_HIGHLIGHT_GROUP_NAME_NOT_FOUND_STR), &line,);
            } else {
                msg_ext_set_kind(c"list_cmd");
                highlight_list_one(id);
            }
            return;
        }

        if dolink {
            let from_start = linep;
            let from_end = from_start.skiptowhite();
            let to_start = from_end.skipwhite();
            let to_end = to_start.skiptowhite();

            if ends_excmd(i32::from(from_start.first_byte())) != 0
                || ends_excmd(i32::from(to_start.first_byte())) != 0
            {
                semsg!(
                    gettext(c"E412: Not enough arguments: \":highlight link %s\""),
                    &from_start,
                );
                return;
            }

            if ends_excmd(i32::from(to_end.skipwhite().first_byte())) == 0 {
                semsg!(
                    gettext(c"E413: Too many arguments: \":highlight link %s\""),
                    &from_start,
                );
                return;
            }

            let from_id = syn_check_group(from_start.slice(from_end.offset_from(&from_start)));
            let to_id = if to_start.as_bytes().starts_with(b"NONE") {
                0
            } else {
                syn_check_group(to_start.slice(to_end.offset_from(&to_start)))
            };

            if from_id > 0 {
                let sg_set = with_hl_group_mut((from_id - 1) as usize, |hlg| {
                    if dodefault && (forceit || hlg.sg_deflink == 0) {
                        hlg.sg_deflink = to_id;
                        hlg.sg_deflink_sctx = g_st().current_sctx.get();
                        hlg.sg_deflink_sctx.sc_lnum += sourcing_lnum();
                        nlua_set_sctx(&mut hlg.sg_deflink_sctx);
                    }
                    hlg.sg_set
                });

                if !init || sg_set == 0 {
                    if to_id > 0 && !forceit && !init && hl_has_settings(from_id - 1, dodefault) {
                        if sourcing_name().is_none() && !dodefault {
                            emsg(gettext(E_GROUP_HAS_SETTINGS_HIGHLIGHT_LINK_IGNORED));
                        }
                    } else {
                        with_hl_group_mut((from_id - 1) as usize, |hlg| {
                            if hlg.sg_link != to_id
                                || hlg.sg_script_ctx.sc_sid != g_st().current_sctx.get().sc_sid
                                || hlg.sg_cleared
                            {
                                if !init {
                                    hlg.sg_set |= SG_LINK;
                                }
                                hlg.sg_link = to_id;
                                hlg.sg_script_ctx = g_st().current_sctx.get();
                                hlg.sg_script_ctx.sc_lnum += sourcing_lnum();
                                nlua_set_sctx(&mut hlg.sg_script_ctx);
                                hlg.sg_cleared = false;
                                redraw_all_later(UPD_SOME_VALID);
                                g_st().need_highlight_changed.set(true);
                            }
                        });
                    }
                }
            }

            return;
        }

        if doclear {
            line = linep;
            if ends_excmd(i32::from(line.first_byte())) != 0 {
                do_unlet(b"g:colors_name", true);
                restore_cterm_colors();

                for j in 0..hl_len() {
                    highlight_clear(j);
                }
                init_highlight(true, true);
                highlight_changed();
                redraw_all_later(UPD_NOT_VALID);
                return;
            }
            name_end = line.skiptowhite();
            linep = name_end.skipwhite();
        }

        let id = syn_check_group(line.slice(name_end.offset_from(&line)));
        if id == 0 {
            return;
        }
        let idx = id - 1;

        if dodefault && hl_has_settings(idx, true) {
            return;
        }

        let (item_before, is_normal_group) = with_hl_group(idx as usize, |g| {
            (
                g.clone(),
                g.sg_name.to_bytes().eq_ignore_ascii_case(b"NORMAL"),
            )
        });

        if doclear || (forceit && init) {
            highlight_clear(idx);
            if !doclear {
                with_hl_group_mut(idx as usize, |g| g.sg_set = 0);
            }
        }

        let mut did_change = false;
        let mut error = false;

        let mut key = [0u8; 64];
        let mut arg = [0u8; 512];
        if !doclear {
            while ends_excmd(i32::from(linep.first_byte())) == 0 {
                let key_start = linep.clone();
                let linep_bytes = linep.as_bytes().to_vec();
                if linep_bytes.first() == Some(&b'=') {
                    semsg!(gettext(E_UNEXPECTED_EQUAL_SIGN_STR), &key_start);
                    error = true;
                    break;
                }

                let key_len = linep_bytes
                    .iter()
                    .take_while(|&&b| !ascii_iswhite(b) && b != b'=')
                    .count();
                linep = linep.wrapping_add(key_len);
                if key_len > key.len() - 1 {
                    emsg(gettext(c"E423: Illegal argument"));
                    error = true;
                    break;
                }
                for (i, dst) in key.iter_mut().take(key_len).enumerate() {
                    *dst = linep_bytes[i].to_ascii_uppercase();
                }
                key[key_len] = 0;
                linep = linep.skipwhite();

                let key_bytes = &key[..key_len];
                if key_bytes == b"NONE" {
                    let do_clear = with_hl_group_mut(idx as usize, |g| {
                        if !init || g.sg_set == 0 {
                            if !init {
                                g.sg_set |= SG_CTERM + SG_GUI;
                            }
                            true
                        } else {
                            false
                        }
                    });
                    if do_clear {
                        highlight_clear(idx);
                    }
                    continue;
                }

                if linep.first_byte() != b'=' {
                    semsg!(gettext(E_MISSING_EQUAL_SIGN_STR_2), &key_start);
                    error = true;
                    break;
                }
                linep = linep.wrapping_add(1).skipwhite();
                let arg_start;
                if linep.first_byte() == b'\'' {
                    linep = linep.wrapping_add(1);
                    arg_start = linep.clone();
                    linep = linep
                        .as_bytes()
                        .iter()
                        .position(|&b| b == b'\'')
                        .map_or(crate::strings::OwnedSlice::NULL, |i| linep.wrapping_add(i));
                    if linep.is_null() {
                        semsg!(gettext(E_INVARG2), &key_start);
                        error = true;
                        break;
                    }
                } else {
                    arg_start = linep.clone();
                    linep = linep.skiptowhite();
                }
                if linep.addr() == arg_start.addr() {
                    semsg!(gettext(E_MISSING_ARGUMENT_STR), &key_start);
                    error = true;
                    break;
                }
                let arg_len = linep.offset_from(&arg_start);
                if arg_len > arg.len() - 1 {
                    emsg(gettext(c"E423: Illegal argument"));
                    error = true;
                    break;
                }
                arg[..arg_len].copy_from_slice(arg_start.slice(arg_len));
                arg[arg_len] = 0;

                if linep.first_byte() == b'\'' {
                    linep = linep.wrapping_add(1);
                }

                if key_bytes == b"TERM" || key_bytes == b"CTERM" || key_bytes == b"GUI" {
                    let mut attr: i32 = 0;
                    let mut off: usize = 0;
                    while arg[off] != 0 {
                        let mut i = HL_ATTR_TABLE.len() as isize - 1;
                        while i >= 0 {
                            let hname = HL_NAME_TABLE[i as usize];
                            let len = hname.to_bytes().len();
                            if mb_strnicmp(
                                crate::memory::bytes_until_nul(&arg[off..]),
                                hname.to_bytes(),
                                len,
                            ) == 0
                            {
                                if (HL_ATTR_TABLE[i as usize] & HL_UNDERLINE_MASK) != 0 {
                                    attr &= !HL_UNDERLINE_MASK;
                                }
                                attr |= HL_ATTR_TABLE[i as usize];
                                off += len;
                                break;
                            }
                            i -= 1;
                        }
                        if i < 0 {
                            semsg!(gettext(c"E418: Illegal value: %s"), &arg);
                            error = true;
                            break;
                        }
                        if arg[off] == b',' {
                            off += 1;
                        }
                    }
                    if error {
                        break;
                    }
                    if key_bytes[0] == b'C' {
                        with_hl_group_mut(idx as usize, |g| {
                            if !init || (g.sg_set & SG_CTERM) == 0 {
                                if !init {
                                    g.sg_set |= SG_CTERM;
                                }
                                g.sg_cterm = attr;
                                g.sg_cterm_bold = false;
                            }
                        });
                    } else if key_bytes[0] == b'G' {
                        with_hl_group_mut(idx as usize, |g| {
                            if !init || (g.sg_set & SG_GUI) == 0 {
                                if !init {
                                    g.sg_set |= SG_GUI;
                                }
                                g.sg_gui = attr;
                            }
                        });
                    }
                } else if key_bytes == b"FONT"
                    && (!init || with_hl_group(idx as usize, |g| (g.sg_set & SG_GUI) == 0))
                {
                    with_hl_group_mut(idx as usize, |g| {
                        if !init {
                            g.sg_set |= SG_GUI;
                        }
                        g.sg_font = (&arg[..arg_len] != b"NONE")
                            .then(|| crate::memory::cstring_until_nul(&arg));
                    });
                } else if key_bytes == b"CTERMFG" || key_bytes == b"CTERMBG" {
                    if !init || with_hl_group(idx as usize, |g| (g.sg_set & SG_CTERM) == 0) {
                        with_hl_group_mut(idx as usize, |g| {
                            if !init {
                                g.sg_set |= SG_CTERM;
                            }
                            if key_bytes[5] == b'F' && g.sg_cterm_bold {
                                g.sg_cterm &= !HL_BOLD;
                                g.sg_cterm_bold = false;
                            }
                        });

                        let color: i32;
                        if arg[0].is_ascii_digit() {
                            color = crate::charset::parse_i64_prefix(&arg[..arg_len]) as i32;
                        } else if mb_stricmp(crate::memory::bytes_as_cstr(&arg), c"fg") == 0 {
                            if g_st().cterm_normal_fg_color.get() != 0 {
                                color = g_st().cterm_normal_fg_color.get() - 1;
                            } else {
                                emsg(gettext(c"E419: FG color unknown"));
                                error = true;
                                break;
                            }
                        } else if mb_stricmp(crate::memory::bytes_as_cstr(&arg), c"bg") == 0 {
                            if g_st().cterm_normal_bg_color.get() > 0 {
                                color = g_st().cterm_normal_bg_color.get() - 1;
                            } else {
                                emsg(gettext(c"E420: BG color unknown"));
                                error = true;
                                break;
                            }
                        } else {
                            let off = arg[0].to_ascii_uppercase();
                            let mut i = COLOR_NAMES.len() as isize - 1;
                            while i >= 0 {
                                let cname = COLOR_NAMES[i as usize];
                                if off == cname.to_bytes()[0]
                                    && mb_stricmp(
                                        crate::memory::bytes_as_cstr(&arg[1..]),
                                        crate::memory::CStrSubslice::advance(cname, 1),
                                    ) == 0
                                {
                                    break;
                                }
                                i -= 1;
                            }
                            if i < 0 {
                                semsg!(
                                    gettext(c"E421: Color name or number not recognized: %s",),
                                    key_start,
                                );
                                error = true;
                                break;
                            }

                            let mut bold = K_NONE;
                            color = lookup_color(i as i32, key_bytes[5] == b'F', &mut bold);

                            with_hl_group_mut(idx as usize, |g| {
                                if bold == K_TRUE {
                                    g.sg_cterm |= HL_BOLD;
                                    g.sg_cterm_bold = true;
                                } else if bold == K_FALSE {
                                    g.sg_cterm &= !HL_BOLD;
                                }
                            });
                        }

                        if key_bytes[5] == b'F' {
                            with_hl_group_mut(idx as usize, |g| g.sg_cterm_fg = color + 1);
                            if is_normal_group {
                                g_st().cterm_normal_fg_color.set(color + 1);
                            }
                        } else {
                            with_hl_group_mut(idx as usize, |g| g.sg_cterm_bg = color + 1);
                            if is_normal_group {
                                g_st().cterm_normal_bg_color.set(color + 1);
                                if !ui_rgb_attached() && color >= 0 {
                                    let dark = if g_st().t_colors.get() < 16 {
                                        i32::from(color == 0 || color == 4)
                                    } else if color < 16 {
                                        i32::from(color < 7 || color == 8)
                                    } else {
                                        -1
                                    };
                                    if dark != -1
                                        && (dark != 0) != (opt_vars().p_bg.first_byte() == b'd')
                                        && !option_was_set(K_OPT_BACKGROUND)
                                    {
                                        set_option_value_give_err(
                                            K_OPT_BACKGROUND,
                                            cstr_as_obj(Some(if dark != 0 {
                                                c"dark"
                                            } else {
                                                c"light"
                                            })),
                                            0,
                                        );
                                        reset_option_was_set(K_OPT_BACKGROUND);
                                    }
                                }
                            }
                        }
                    }
                } else if key_bytes == b"GUIFG" {
                    did_change = set_gui_color(idx, init, crate::memory::bytes_as_cstr(&arg), b'F');
                    if is_normal_group {
                        g_st()
                            .normal_fg
                            .set(with_hl_group(idx as usize, |g| g.sg_rgb_fg));
                    }
                } else if key_bytes == b"GUIBG" {
                    did_change = set_gui_color(idx, init, crate::memory::bytes_as_cstr(&arg), b'B');
                    if is_normal_group {
                        g_st()
                            .normal_bg
                            .set(with_hl_group(idx as usize, |g| g.sg_rgb_bg));
                    }
                } else if key_bytes == b"GUISP" {
                    did_change = set_gui_color(idx, init, crate::memory::bytes_as_cstr(&arg), b'S');
                    if is_normal_group {
                        g_st()
                            .normal_sp
                            .set(with_hl_group(idx as usize, |g| g.sg_rgb_sp));
                    }
                } else if key_bytes == b"START" || key_bytes == b"STOP" {
                    // Ignored for now
                } else if key_bytes == b"BLEND" {
                    let blend = if &arg[..arg_len] != b"NONE" {
                        let mut rem = &arg[..arg_len];
                        getdigits_int(&mut rem, false, 0)
                    } else {
                        -1
                    };
                    with_hl_group_mut(idx as usize, |g| g.sg_blend = blend);
                } else {
                    semsg!(gettext(c"E423: Illegal argument: %s"), key_start);
                    error = true;
                    break;
                }
                with_hl_group_mut(idx as usize, |g| {
                    g.sg_cleared = false;
                    if !init || (g.sg_set & SG_LINK) == 0 {
                        g.sg_link = 0;
                    }
                });

                linep = linep.skipwhite();
            }
        }

        let mut did_highlight_changed = false;

        if !error && is_normal_group {
            highlight_attr_set_all();

            if !ui_has(K_UILINEGRID_CONST) && starting_val() == 0 {
                ui_refresh();
            } else {
                ui_default_colors_set();
            }
            did_highlight_changed = true;
            redraw_all_later(UPD_NOT_VALID);
        } else {
            set_hl_attr(idx);
        }
        let mut sctx = g_st().current_sctx.get();
        sctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&mut sctx);
        let item_changed = with_hl_group_mut(idx as usize, |g| {
            g.sg_script_ctx = sctx;
            *g != item_before
        });

        if (did_change || item_changed) && !did_highlight_changed {
            if !g_st().updating_screen.get() {
                redraw_all_later(UPD_NOT_VALID);
            }
            g_st().need_highlight_changed.set(true);
        }
    }
}

pub fn free_highlight() {
    HIGHLIGHT_GA.with(|cell| cell.borrow_mut().clear());
    HIGHLIGHT_UNAMES.with(|cell| cell.borrow_mut().clear());
}

pub fn restore_cterm_colors() {
    g_st().normal_fg.set(-1);
    g_st().normal_bg.set(-1);
    g_st().normal_sp.set(-1);
    g_st().cterm_normal_fg_color.set(0);
    g_st().cterm_normal_bg_color.set(0);
}

fn hl_has_settings(idx: i32, check_link: bool) -> bool {
    with_hl_group(idx as usize, |g| {
        !g.sg_cleared
            && (g.sg_attr != 0
                || g.sg_cterm_fg != 0
                || g.sg_cterm_bg != 0
                || g.sg_rgb_fg_idx != K_COLOR_IDX_NONE
                || g.sg_rgb_bg_idx != K_COLOR_IDX_NONE
                || g.sg_rgb_sp_idx != K_COLOR_IDX_NONE
                || (check_link && (g.sg_set & SG_LINK) != 0))
    })
}

fn highlight_clear(idx: i32) {
    with_hl_group_mut(idx as usize, |g| {
        g.sg_cleared = true;
        g.sg_attr = 0;
        g.sg_cterm = 0;
        g.sg_cterm_bold = false;
        g.sg_cterm_fg = 0;
        g.sg_cterm_bg = 0;
        g.sg_gui = 0;
        g.sg_rgb_fg = -1;
        g.sg_rgb_bg = -1;
        g.sg_rgb_sp = -1;
        g.sg_rgb_fg_idx = K_COLOR_IDX_NONE;
        g.sg_rgb_bg_idx = K_COLOR_IDX_NONE;
        g.sg_rgb_sp_idx = K_COLOR_IDX_NONE;
        g.sg_blend = -1;
        g.sg_font = None;
        g.sg_link = g.sg_deflink;
        g.sg_script_ctx = g.sg_deflink_sctx;
    });
}

fn highlight_list_one(id: i32) {
    let sgp = with_hl_group((id - 1) as usize, HlGroup::clone);
    let mut didh = false;

    if message_filtered(&sgp.sg_name) {
        return;
    }

    if sgp.sg_parent != 0 && sgp.sg_cleared {
        return;
    }

    didh = highlight_list_arg(id, didh, LIST_ATTR, sgp.sg_cterm, None, c"cterm");
    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_cterm_fg, None, c"ctermfg");
    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_cterm_bg, None, c"ctermbg");

    didh = highlight_list_arg(id, didh, LIST_ATTR, sgp.sg_gui, None, c"gui");
    let mut hexbuf = [0u8; 8];
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_fg_idx, sgp.sg_rgb_fg, &mut hexbuf),
        c"guifg",
    );
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_bg_idx, sgp.sg_rgb_bg, &mut hexbuf),
        c"guibg",
    );
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_sp_idx, sgp.sg_rgb_sp, &mut hexbuf),
        c"guisp",
    );

    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_blend + 1, None, c"blend");

    didh = highlight_list_arg(id, didh, LIST_STRING, 0, sgp.sg_font.as_deref(), c"font");

    if sgp.sg_link != 0 && !g_st().got_int.get() {
        syn_list_header(didh, 0, id, true);
        didh = true;
        msg_puts_hl(c"links to", HLF_D, false);
        msg_putchar(i32::from(b' '));
        let link_id = with_hl_group((id - 1) as usize, |g| g.sg_link);
        let link_name = with_hl_group((link_id - 1) as usize, |g| g.sg_name.clone());
        msg_outtrans(&link_name, 0, false);
    }

    if !didh {
        highlight_list_arg(id, didh, LIST_STRING, 0, Some(c"cleared"), c"");
    }
    if opt_vars().p_verbose.get() > 0 {
        last_set_msg(sgp.sg_script_ctx);
    }
}

fn hlgroup2dict(hl: &mut Dict, ns_id: NS, hl_id: i32) -> bool {
    let (sg_link, sg_set, sg_cleared, sg_attr) = with_hl_group((hl_id - 1) as usize, |sgp| {
        (sgp.sg_link, sgp.sg_set, sgp.sg_cleared, sgp.sg_attr)
    });
    let mut ns = ns_id;
    let link = if ns_id == 0 {
        sg_link
    } else {
        ns_get_hl(&mut ns, hl_id, true, sg_set != 0)
    };
    if link == -1 {
        return false;
    }
    if ns_id == 0 && sg_cleared && sg_set == 0 {
        return false;
    }
    ns = ns_id;
    let attr_id = if ns_id == 0 {
        sg_attr
    } else {
        ns_get_hl(&mut ns, hl_id, false, sg_set != 0)
    };
    let attr = syn_attr2entry(attr_id);
    *hl = Dict::with_capacity(HLATTRS_DICT_SIZE + 1);
    if (attr.rgb_ae_attr & HL_DEFAULT) != 0 {
        put_c(hl, c"default", boolean_obj(true));
    }
    if link > 0 {
        assert!(1 <= link && link <= hl_len());
        let link_name = with_hl_group((link - 1) as usize, |g| g.sg_name.clone());
        put_c(
            hl,
            c"link",
            crate::api::private::helpers::cstr_to_nonnull_obj(Some(&link_name)),
        );
    }
    let mut hl_cterm = Dict::with_capacity(HLATTRS_DICT_SIZE);
    hlattrs2dict(hl, None, attr, true, true);
    hlattrs2dict(hl, Some(&mut hl_cterm), attr, false, true);
    if hl_cterm.size > 0 {
        put_c(hl, c"cterm", dict_obj(hl_cterm));
    }
    true
}

pub fn ns_get_hl_defs(ns_id: NS, opts: &mut KeyDict_get_highlight, err: &mut Error) -> Dict {
    let link = if has_key(opts.is_set_get_highlight, KEYSET_OPTIDX_GET_HIGHLIGHT_LINK) {
        opts.link
    } else {
        true
    };
    let mut id: i32 = -1;
    if has_key(opts.is_set_get_highlight, KEYSET_OPTIDX_GET_HIGHLIGHT_NAME) {
        let create = if has_key(
            opts.is_set_get_highlight,
            KEYSET_OPTIDX_GET_HIGHLIGHT_CREATE,
        ) {
            opts.create
        } else {
            true
        };
        let name_slice = opts.name.as_bytes();
        id = if create {
            syn_check_group(name_slice)
        } else {
            syn_name2id_len(name_slice)
        };
        if id == 0 && !create {
            return Dict::new();
        }
    } else if has_key(opts.is_set_get_highlight, KEYSET_OPTIDX_GET_HIGHLIGHT_ID) {
        id = opts.id as i32;
    }

    if id != -1 {
        if !(1 <= id && id <= hl_len()) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"Highlight id out of bounds",
            );
            return Dict::new();
        }
        let mut attrs = Dict::new();
        let target_id = if link { id } else { syn_get_final_id(id) };
        hlgroup2dict(&mut attrs, ns_id, target_id);
        return attrs;
    }
    if err.err_type != ErrorType::ErrorTypeNone {
        return Dict::new();
    }

    let len = hl_len();
    let mut rv = Dict::with_capacity(len as usize);
    for i in 1..=len {
        let mut attrs = Dict::new();
        if !hlgroup2dict(&mut attrs, ns_id, i) {
            continue;
        }
        let target_id = if link { i } else { syn_get_final_id(i) };
        let target_name = with_hl_group((target_id - 1) as usize, |g| {
            NvimString::from_bytes(g.sg_name.as_bytes())
        });
        put_c(&mut rv, target_name, dict_obj(attrs));
    }

    rv
}

fn highlight_list_arg(
    id: i32,
    mut didh: bool,
    type_: i32,
    mut iarg: i32,
    sarg: Option<&CStr>,
    name: &std::ffi::CStr,
) -> bool {
    if g_st().got_int.get() {
        return false;
    }

    if if type_ == LIST_STRING {
        sarg.is_none()
    } else {
        iarg == 0
    } {
        return didh;
    }

    let mut buf = [0u8; 100];
    if type_ == LIST_INT {
        write_dec(&mut buf, iarg - 1);
    } else if type_ != LIST_STRING {
        buf[0] = 0;
        let mut i = 0;
        while HL_ATTR_TABLE[i] != 0 {
            let flag = HL_ATTR_TABLE[i];
            if ((flag & HL_UNDERLINE_MASK) != 0 && ((iarg & HL_UNDERLINE_MASK) == flag))
                || ((flag & HL_UNDERLINE_MASK) == 0 && (iarg & flag) != 0)
            {
                if buf[0] != 0 {
                    crate::memory::append_bytes_to_c_buf(&mut buf, b",");
                }
                crate::memory::append_bytes_to_c_buf(&mut buf, HL_NAME_TABLE[i].to_bytes());
                if (flag & HL_UNDERLINE_MASK) == 0 {
                    iarg &= !flag;
                }
            }
            i += 1;
        }
    }
    let ts_cstr = if type_ == LIST_STRING {
        sarg.unwrap_or(c"")
    } else {
        crate::memory::bytes_as_cstr(&buf)
    };

    syn_list_header(
        didh,
        vim_strsize(ts_cstr) + name.to_bytes().len() as i32 + 1,
        id,
        false,
    );
    didh = true;
    if !g_st().got_int.get() {
        if !name.to_bytes().is_empty() {
            msg_puts_hl(name, HLF_D, false);
            msg_puts_hl(c"=", HLF_D, false);
        }
        msg_outtrans(ts_cstr, 0, false);
    }
    didh
}

pub fn highlight_has_attr(id: i32, flag: i32, modec: i32) -> Option<&'static CStr> {
    if id <= 0 || id > hl_len() {
        return None;
    }

    let attr = with_hl_group((id - 1) as usize, |g| {
        if modec == i32::from(b'g') {
            g.sg_gui
        } else {
            g.sg_cterm
        }
    });

    if (flag & HL_UNDERLINE_MASK) != 0 {
        let ul = attr & HL_UNDERLINE_MASK;
        (ul == flag).then_some(c"1")
    } else {
        ((attr & flag) != 0).then_some(c"1")
    }
}

pub fn highlight_color(id: i32, what_bytes: &[u8], modec: i32) -> Option<std::ffi::CString> {
    let mut fg = false;
    let mut sp = false;
    let mut font = false;

    if id <= 0 || id > hl_len() {
        return None;
    }

    let w0 = what_bytes.first().map_or(0, u8::to_ascii_lowercase);
    let w1 = what_bytes.get(1).map_or(0, u8::to_ascii_lowercase);
    if w0 == b'f' && w1 == b'g' {
        fg = true;
    } else if w0 == b'f'
        && w1 == b'o'
        && what_bytes.len() >= 4
        && what_bytes[2..4].eq_ignore_ascii_case(b"nt")
    {
        font = true;
    } else if w0 == b's' && w1 == b'p' {
        sp = true;
    } else if !(w0 == b'b' && w1 == b'g') {
        return None;
    }

    let (
        sg_rgb_fg,
        sg_rgb_bg,
        sg_rgb_sp,
        sg_rgb_fg_idx,
        sg_rgb_bg_idx,
        sg_rgb_sp_idx,
        sg_cterm_fg,
        sg_cterm_bg,
    ) = with_hl_group((id - 1) as usize, |g| {
        (
            g.sg_rgb_fg,
            g.sg_rgb_bg,
            g.sg_rgb_sp,
            g.sg_rgb_fg_idx,
            g.sg_rgb_bg_idx,
            g.sg_rgb_sp_idx,
            g.sg_cterm_fg,
            g.sg_cterm_bg,
        )
    });

    let mut color_name = [0u8; 20];
    if modec == i32::from(b'g') {
        if what_bytes.get(2) == Some(&b'#') && ui_rgb_attached() {
            let n = if fg {
                sg_rgb_fg
            } else if sp {
                sg_rgb_sp
            } else {
                sg_rgb_bg
            };
            if !(0..=0xffffff).contains(&n) {
                return None;
            }
            let hex_arr: &mut [u8; 8] = (&mut color_name[..8]).try_into().unwrap();
            write_hex6(hex_arr, n);
            return CStr::from_bytes_until_nul(&color_name)
                .ok()
                .map(CStr::to_owned);
        }
        let hex_arr: &mut [u8; 8] = (&mut color_name[..8]).try_into().unwrap();
        let res = if fg {
            coloridx_to_name(sg_rgb_fg_idx, sg_rgb_fg, hex_arr)
        } else if sp {
            coloridx_to_name(sg_rgb_sp_idx, sg_rgb_sp, hex_arr)
        } else {
            coloridx_to_name(sg_rgb_bg_idx, sg_rgb_bg, hex_arr)
        };
        return res.map(CStr::to_owned);
    }
    if font || sp {
        return None;
    }
    if modec == i32::from(b'c') {
        let n = if fg { sg_cterm_fg - 1 } else { sg_cterm_bg - 1 };
        if n < 0 {
            return None;
        }
        write_dec(&mut color_name, n);
        return CStr::from_bytes_until_nul(&color_name)
            .ok()
            .map(CStr::to_owned);
    }
    None
}

pub fn syn_list_header(did_header: bool, outlen: i32, id: i32, force_newline: bool) -> bool {
    let mut endcol: i32 = 19;
    let mut newline = true;
    let mut name_col: i32 = 0;
    let mut adjust = true;

    if !did_header {
        if !ui_has(K_UIMESSAGES) || g_st().msg_col.get() > 0 {
            msg_putchar(i32::from(b'\n'));
        }
        if g_st().got_int.get() {
            return true;
        }
        let name = with_hl_group((id - 1) as usize, |g| g.sg_name.clone());
        name_col = msg_outtrans(&name, 0, false);
        g_st().msg_col.set(name_col);
        endcol = 15;
    } else if (ui_has(K_UIMESSAGES) || g_st().msg_silent.get() != 0) && !force_newline {
        msg_putchar(i32::from(b' '));
        adjust = false;
    } else if g_st().msg_col.get() + outlen + 1 >= g_st().columns_global.get() || force_newline {
        msg_putchar(i32::from(b'\n'));
        if g_st().got_int.get() {
            return true;
        }
    } else if g_st().msg_col.get() >= endcol {
        newline = false;
    }

    if adjust {
        if g_st().msg_col.get() >= endcol {
            endcol = g_st().msg_col.get() + 1;
        }
        msg_advance(endcol);
    }

    if !did_header {
        if endcol == g_st().columns_global.get() - 1 && endcol <= name_col {
            msg_putchar(i32::from(b' '));
        }
        msg_puts_hl(c"xxx", id, false);
        msg_putchar(i32::from(b' '));
    }

    newline
}

fn set_hl_attr(idx: i32) {
    let mut at_en = HLATTRS_INIT;
    let font_bytes = with_hl_group(idx as usize, |sgp| {
        at_en.cterm_ae_attr = sgp.sg_cterm;
        at_en.cterm_fg_color = sgp.sg_cterm_fg as i16;
        at_en.cterm_bg_color = sgp.sg_cterm_bg as i16;
        at_en.rgb_ae_attr = sgp.sg_gui;
        at_en.rgb_fg_color = if sgp.sg_rgb_fg_idx != K_COLOR_IDX_NONE {
            sgp.sg_rgb_fg
        } else {
            -1
        };
        at_en.rgb_bg_color = if sgp.sg_rgb_bg_idx != K_COLOR_IDX_NONE {
            sgp.sg_rgb_bg
        } else {
            -1
        };
        at_en.rgb_sp_color = if sgp.sg_rgb_sp_idx != K_COLOR_IDX_NONE {
            sgp.sg_rgb_sp
        } else {
            -1
        };
        at_en.hl_blend = sgp.sg_blend;
        sgp.sg_font.as_ref().map(|f| f.as_bytes().to_vec())
    });
    if let Some(ref font) = font_bytes {
        at_en.font = hl_add_font_idx(Some(font));
    }

    let syn_attr = hl_get_syn_attr(0, idx + 1, at_en);
    with_hl_group_mut(idx as usize, |sgp| sgp.sg_attr = syn_attr);

    if cursor_mode_uses_syn_id(idx + 1) {
        ui_mode_info_set();
    }
}

pub fn syn_name2id(name: &CStr) -> i32 {
    let bytes = name.to_bytes();
    if bytes.first() == Some(&b'@') {
        return syn_check_group(bytes);
    }
    syn_name2id_len(bytes)
}

pub fn syn_name2id_len(name_slice: &[u8]) -> i32 {
    let len = name_slice.len();
    if len == 0 || len > MAX_SYN_NAME {
        return 0;
    }

    let mut name_u = [0u8; MAX_SYN_NAME + 1];
    for (dst, &b) in name_u.iter_mut().zip(name_slice) {
        *dst = b.to_ascii_uppercase();
    }
    name_u[len] = 0;

    HIGHLIGHT_UNAMES.with(|cell| cell.borrow().get(&name_u[..len]).copied().unwrap_or(0))
}

pub fn syn_name2attr(name: &CStr) -> i32 {
    let id = syn_name2id(name);
    if id != 0 {
        return syn_id2attr(id);
    }
    0
}

pub fn highlight_exists(name: &CStr) -> i32 {
    i32::from(syn_name2id(name) > 0)
}

pub fn syn_id2name(id: i32) -> std::ffi::CString {
    if id <= 0 || id > hl_len() {
        return std::ffi::CString::default();
    }
    with_hl_group((id - 1) as usize, |g| g.sg_name.clone())
}

pub fn syn_check_group(name: &[u8]) -> i32 {
    if name.len() > MAX_SYN_NAME {
        emsg(gettext(E_HIGHLIGHT_GROUP_NAME_TOO_LONG));
        return 0;
    }
    let id = syn_name2id_len(name);
    if id == 0 {
        return syn_add_group(name);
    }
    id
}

fn syn_add_group(name_slice: &[u8]) -> i32 {
    let len = name_slice.len();
    for &c in name_slice {
        if !vim_isprintc(i32::from(c)) {
            emsg(gettext(c"E669: Unprintable character in group name"));
            return 0;
        } else if !c.is_ascii_alphanumeric() && c != b'_' && c != b'.' && c != b'@' && c != b'-' {
            msg_source(HLF_W);
            emsg(gettext(E_HIGHLIGHT_GROUP_NAME_INVALID_CHAR));
            return 0;
        }
    }

    let mut scoped_parent: i32 = 0;
    if len > 1
        && name_slice.first() == Some(&b'@')
        && let Some(delim_idx) = name_slice.iter().rposition(|&b| b == b'.')
    {
        scoped_parent = syn_check_group(&name_slice[..delim_idx]);
    }

    HIGHLIGHT_GA.with(|cell| {
        let mut ga = cell.borrow_mut();
        if ga.capacity() == 0 {
            ga.reserve(300);
        }
    });

    if hl_len() >= MAX_HL_ID {
        emsg(gettext(c"E849: Too many highlight and syntax groups"));
        return 0;
    }

    let name_u_bytes: Vec<u8> = name_slice.iter().map(u8::to_ascii_uppercase).collect();
    let mut hlg = HlGroup::new();
    hlg.sg_name = std::ffi::CString::new(name_slice).unwrap();
    hlg.sg_rgb_bg = -1;
    hlg.sg_rgb_fg = -1;
    hlg.sg_rgb_sp = -1;
    hlg.sg_rgb_bg_idx = K_COLOR_IDX_NONE;
    hlg.sg_rgb_fg_idx = K_COLOR_IDX_NONE;
    hlg.sg_rgb_sp_idx = K_COLOR_IDX_NONE;
    hlg.sg_blend = -1;
    hlg.sg_parent = scoped_parent;
    hlg.sg_cleared = true;
    HIGHLIGHT_GA.with(|cell| cell.borrow_mut().push(hlg));

    let id = hl_len();
    HIGHLIGHT_UNAMES.with(|cell| {
        cell.borrow_mut().insert(name_u_bytes, id);
    });

    id
}

pub fn syn_id2attr(hl_id: i32) -> i32 {
    let mut optional = false;
    syn_ns_id2attr(-1, hl_id, &mut optional)
}

pub fn syn_ns_id2attr(mut ns_id: i32, mut hl_id: i32, optional: &mut bool) -> i32 {
    if syn_ns_get_final_id(&mut ns_id, &mut hl_id) {
        *optional = false;
    }
    let (sg_set, sg_attr) = with_hl_group((hl_id - 1) as usize, |sgp| (sgp.sg_set, sgp.sg_attr));

    let attr = ns_get_hl(&mut ns_id, hl_id, false, sg_set != 0);

    if attr >= 0 || (*optional && ns_id > 0) {
        return attr;
    }
    sg_attr
}

pub fn syn_get_final_id(mut hl_id: i32) -> i32 {
    let mut ns_id = curwin().w_ns_hl_active.get();
    syn_ns_get_final_id(&mut ns_id, &mut hl_id);
    hl_id
}

pub fn syn_ns_get_final_id(ns_id: &mut i32, hl_idp: &mut i32) -> bool {
    let mut hl_id = *hl_idp;
    let mut used = false;

    if hl_id > hl_len() || hl_id < 1 {
        *hl_idp = 0;
        return false;
    }

    let mut count = 100;
    while count > 0 {
        count -= 1;
        let sg_set = with_hl_group((hl_id - 1) as usize, |sgp| sgp.sg_set);

        let check = ns_get_hl(ns_id, hl_id, true, sg_set != 0);
        if check == 0 {
            *hl_idp = hl_id;
            return true;
        } else if check > 0 {
            used = true;
            hl_id = check;
            continue;
        }

        let (sg_link, sg_cleared, sg_parent) = with_hl_group((hl_id - 1) as usize, |sgp| {
            (sgp.sg_link, sgp.sg_cleared, sgp.sg_parent)
        });
        if sg_link > 0 && sg_link <= hl_len() {
            hl_id = sg_link;
        } else if sg_cleared && sg_parent > 0 {
            hl_id = sg_parent;
        } else {
            break;
        }
    }

    *hl_idp = hl_id;
    used
}

pub fn highlight_attr_set_all() {
    for idx in 0..hl_len() {
        with_hl_group_mut(idx as usize, |sgp| {
            if sgp.sg_rgb_bg_idx == K_COLOR_IDX_FG {
                sgp.sg_rgb_bg = g_st().normal_fg.get();
            } else if sgp.sg_rgb_bg_idx == K_COLOR_IDX_BG {
                sgp.sg_rgb_bg = g_st().normal_bg.get();
            }
            if sgp.sg_rgb_fg_idx == K_COLOR_IDX_FG {
                sgp.sg_rgb_fg = g_st().normal_fg.get();
            } else if sgp.sg_rgb_fg_idx == K_COLOR_IDX_BG {
                sgp.sg_rgb_fg = g_st().normal_bg.get();
            }
            if sgp.sg_rgb_sp_idx == K_COLOR_IDX_FG {
                sgp.sg_rgb_sp = g_st().normal_fg.get();
            } else if sgp.sg_rgb_sp_idx == K_COLOR_IDX_BG {
                sgp.sg_rgb_sp = g_st().normal_bg.get();
            }
        });
        set_hl_attr(idx);
    }
}

fn combine_stl_hlt(
    id: i32,
    id_s: i32,
    id_alt: i32,
    hlcnt: i32,
    i: i32,
    hlf: usize,
    table: &[std::cell::Cell<i32>],
) {
    let dst = (hlcnt + i) as usize;
    let u_idx = (id - 1) as usize;
    let s_idx = (id_s - 1) as usize;

    HIGHLIGHT_GA.with(|cell| {
        let mut ga = cell.borrow_mut();
        if id_alt == 0 {
            ga[dst] = HlGroup::new();
            ga[dst].sg_cterm = g_st().highlight_attr[hlf].get();
            ga[dst].sg_gui = g_st().highlight_attr[hlf].get();
        } else {
            ga[dst] = ga[(id_alt - 1) as usize].clone();
        }
        ga[dst].sg_link = 0;

        ga[dst].sg_cterm ^= ga[u_idx].sg_cterm ^ ga[s_idx].sg_cterm;
        if ga[u_idx].sg_cterm_fg != ga[s_idx].sg_cterm_fg {
            ga[dst].sg_cterm_fg = ga[u_idx].sg_cterm_fg;
        }
        if ga[u_idx].sg_cterm_bg != ga[s_idx].sg_cterm_bg {
            ga[dst].sg_cterm_bg = ga[u_idx].sg_cterm_bg;
        }
        ga[dst].sg_gui ^= ga[u_idx].sg_gui ^ ga[s_idx].sg_gui;
        if ga[u_idx].sg_rgb_fg != ga[s_idx].sg_rgb_fg {
            ga[dst].sg_rgb_fg = ga[u_idx].sg_rgb_fg;
        }
        if ga[u_idx].sg_rgb_bg != ga[s_idx].sg_rgb_bg {
            ga[dst].sg_rgb_bg = ga[u_idx].sg_rgb_bg;
        }
        if ga[u_idx].sg_rgb_sp != ga[s_idx].sg_rgb_sp {
            ga[dst].sg_rgb_sp = ga[u_idx].sg_rgb_sp;
        }
    });
    set_hl_attr(hlcnt + i);

    table[i as usize].set(syn_id2attr(hlcnt + i + 1));
}

pub fn highlight_changed() {
    let mut userhl = [0u8; 30];
    let mut id_s: i32 = -1;
    let mut id_snc: i32 = 0;

    g_st().need_highlight_changed.set(false);

    g_st().highlight_attr[HLF_NONE].set(0);

    for (hlf, name_opt) in HLF_NAMES.iter().enumerate().take(HLF_COUNT).skip(1) {
        let name_cstr = name_opt.unwrap();
        let id = syn_check_group(name_cstr.to_bytes());
        if id == 0 {
            std::process::abort();
        }
        let mut ns_id: i32 = 0;
        let mut final_id = id;
        syn_ns_get_final_id(&mut ns_id, &mut final_id);
        if hlf == HLF_SNC {
            id_snc = final_id;
        } else if hlf == HLF_S {
            id_s = final_id;
        }

        g_st().highlight_attr[hlf].set(hl_get_ui_attr(
            ns_id,
            hlf as i32,
            final_id,
            hlf == HLF_INACTIVE,
        ));

        if g_st().highlight_attr[hlf].get() != g_st().highlight_attr_last[hlf].get() {
            if hlf == HLF_MSG {
                g_st().clear_cmdline.set(true);
                let attrs = syn_attr2entry(g_st().highlight_attr[hlf].get());
                g_st().msg_grid.blending.set(attrs.hl_blend > -1);
            }
            ui_call_hl_group_set(
                cstr_as_string(Some(name_cstr)),
                Integer::from(g_st().highlight_attr[hlf].get()),
            );
            g_st().highlight_attr_last[hlf].set(g_st().highlight_attr[hlf].get());
        }
    }

    let hlcnt = hl_len();
    HIGHLIGHT_GA.with(|cell| {
        let mut ga = cell.borrow_mut();
        ga.resize((hlcnt + 10) as usize, HlGroup::new());
        if id_s == -1 {
            ga[(hlcnt + 9) as usize] = HlGroup::new();
        }
    });
    if id_s == -1 {
        id_s = hlcnt + 10;
    }
    for i in 0..9 {
        userhl[0] = b'U';
        userhl[1] = b's';
        userhl[2] = b'e';
        userhl[3] = b'r';
        userhl[4] = b'1' + i as u8;
        userhl[5] = 0;
        let id = syn_name2id(crate::memory::bytes_as_cstr(&userhl));
        if id == 0 {
            g_st().highlight_user[i].set(0);
            g_st().highlight_stlnc[i].set(0);
        } else {
            g_st().highlight_user[i].set(syn_id2attr(id));
            combine_stl_hlt(
                id,
                id_s,
                id_snc,
                hlcnt,
                i as i32,
                HLF_SNC,
                &g_st().highlight_stlnc,
            );
        }
    }
    HIGHLIGHT_GA.with(|cell| cell.borrow_mut().truncate(hlcnt as usize));

    decor_provider_invalidate_hl();
}

pub fn set_context_in_highlight_cmd(xp: &mut ExpandCtx, arg_in: &CStr) {
    let mut arg = crate::strings::OwnedSlice::from_cstr(arg_in);
    {
        xp.xp_context = EXPAND_HIGHLIGHT;
        xp.xp_pattern = arg.clone();
        g_st().include_link.set(2);
        g_st().include_default.set(1);

        if arg.is_empty() {
            return;
        }

        let mut p = arg.skiptowhite();
        if p.is_empty() {
            return;
        }

        g_st().include_default.set(0);
        let dlen = p.offset_from(&arg);
        if arg.slice(dlen) == &b"default"[..dlen.min(7)] {
            arg = p.skipwhite();
            xp.xp_pattern = arg.clone();
            p = arg.skiptowhite();
        }
        if p.is_empty() {
            return;
        }

        g_st().include_link.set(0);
        if arg.as_bytes().starts_with(b"Ni") {
            highlight_list();
        }
        let len = p.offset_from(&arg);
        let arg_word = arg.slice(len);
        if arg_word == &b"link"[..len.min(4)] || arg_word == &b"clear"[..len.min(5)] {
            xp.xp_pattern = p.skipwhite();
            p = xp.xp_pattern.skiptowhite();
            if !p.is_empty() {
                xp.xp_pattern = p.skipwhite();
                p = xp.xp_pattern.skiptowhite();
            }
        }
        if !p.is_empty() {
            xp.xp_context = EXPAND_NOTHING;
        }
    }
}

fn highlight_list() {
    let mut i = 9;
    while i >= 0 {
        highlight_list_two(i, HLF_D);
        i -= 1;
    }
    let mut j = 39;
    while j >= 0 {
        highlight_list_two(99, 0);
        j -= 1;
    }
}

fn highlight_list_two(cnt: i32, id: i32) {
    const MSG: &CStr = c"N \x08I \x08!  \x08";
    msg_puts_hl(
        crate::memory::CStrSubslice::advance(MSG, (cnt / 11) as usize),
        id,
        false,
    );
    msg_clr_eos();
    ui_flush();
    os_delay(if cnt == 99 { 40 } else { (cnt as u64) * 50 }, false);
}

pub fn get_highlight_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    let name = get_highlight_name_ext(idx, true)?;
    Some(xp.set_item(name))
}

pub fn get_highlight_name_ext(idx: i32, skip_cleared: bool) -> Option<std::ffi::CString> {
    if idx < 0 {
        return None;
    }

    let len = hl_len();
    if skip_cleared && idx < len && with_hl_group(idx as usize, |g| g.sg_cleared) {
        return Some(std::ffi::CString::default());
    }

    if idx == len && g_st().include_none.get() != 0 {
        return Some(c"none".to_owned());
    } else if idx == len + g_st().include_none.get() && g_st().include_default.get() != 0 {
        return Some(c"default".to_owned());
    } else if idx == len + g_st().include_none.get() + g_st().include_default.get()
        && g_st().include_link.get() != 0
    {
        return Some(c"link".to_owned());
    } else if idx == len + g_st().include_none.get() + g_st().include_default.get() + 1
        && g_st().include_link.get() != 0
    {
        return Some(c"clear".to_owned());
    } else if idx >= len {
        return None;
    }
    Some(with_hl_group(idx as usize, |g| g.sg_name.clone()))
}

pub fn name_to_color(name: &CStr, idx: &mut i32) -> RgbValue {
    let name_bytes = name.to_bytes();
    if name_bytes.len() == 7
        && name_bytes[0] == b'#'
        && name_bytes[1..].iter().all(u8::is_ascii_hexdigit)
    {
        *idx = K_COLOR_IDX_HEX;
        let hex_str = std::str::from_utf8(&name_bytes[1..]).unwrap_or("");
        return RgbValue::from_str_radix(hex_str, 16).unwrap_or(0);
    } else if mb_stricmp(name, c"bg") == 0 || mb_stricmp(name, c"background") == 0 {
        *idx = K_COLOR_IDX_BG;
        return g_st().normal_bg.get();
    } else if mb_stricmp(name, c"fg") == 0 || mb_stricmp(name, c"foreground") == 0 {
        *idx = K_COLOR_IDX_FG;
        return g_st().normal_fg.get();
    }

    let mut lo: usize = 0;
    let mut hi: usize = COLOR_NAME_TABLE.len();
    while lo < hi {
        let m = (lo + hi) / 2;
        let cmp = mb_stricmp(name, COLOR_NAME_TABLE[m].name);
        if cmp < 0 {
            hi = m;
        } else if cmp > 0 {
            lo = m + 1;
        } else {
            *idx = m as i32;
            return COLOR_NAME_TABLE[m].color;
        }
    }

    *idx = K_COLOR_IDX_NONE;
    -1
}

pub fn coloridx_to_name(idx: i32, val: i32, hexbuf: &mut [u8; 8]) -> Option<&CStr> {
    if idx >= 0 {
        return Some(COLOR_NAME_TABLE[idx as usize].name);
    }
    match idx {
        K_COLOR_IDX_NONE => None,
        K_COLOR_IDX_FG => Some(c"fg"),
        K_COLOR_IDX_BG => Some(c"bg"),
        K_COLOR_IDX_HEX => {
            write_hex6(hexbuf, val);
            CStr::from_bytes_until_nul(hexbuf).ok()
        }
        _ => std::process::abort(),
    }
}

pub fn name_to_ctermcolor(name: &CStr) -> i32 {
    let name_bytes = name.to_bytes();
    let Some(&first) = name_bytes.first() else {
        return -1;
    };
    let off = first.to_ascii_uppercase();
    let mut i = COLOR_NAMES.len() as isize - 1;
    while i >= 0 {
        let cname = COLOR_NAMES[i as usize];
        if off == cname.to_bytes()[0]
            && mb_stricmp(&name[1..], crate::memory::CStrSubslice::advance(cname, 1)) == 0
        {
            break;
        }
        i -= 1;
    }
    if i < 0 {
        return -1;
    }
    let mut bold = K_NONE;
    lookup_color(i as i32, false, &mut bold)
}

macro_rules! cname {
    ($name:literal, $r:expr, $g:expr, $b:expr) => {
        ColorNameTable {
            name: $name,
            color: rgb($r, $g, $b),
        }
    };
}

pub static COLOR_NAME_TABLE: [ColorNameTable; 707] = [
    cname!(c"AliceBlue", 0xf0, 0xf8, 0xff),
    cname!(c"AntiqueWhite", 0xfa, 0xeb, 0xd7),
    cname!(c"AntiqueWhite1", 0xff, 0xef, 0xdb),
    cname!(c"AntiqueWhite2", 0xee, 0xdf, 0xcc),
    cname!(c"AntiqueWhite3", 0xcd, 0xc0, 0xb0),
    cname!(c"AntiqueWhite4", 0x8b, 0x83, 0x78),
    cname!(c"Aqua", 0x00, 0xff, 0xff),
    cname!(c"Aquamarine", 0x7f, 0xff, 0xd4),
    cname!(c"Aquamarine1", 0x7f, 0xff, 0xd4),
    cname!(c"Aquamarine2", 0x76, 0xee, 0xc6),
    cname!(c"Aquamarine3", 0x66, 0xcd, 0xaa),
    cname!(c"Aquamarine4", 0x45, 0x8b, 0x74),
    cname!(c"Azure", 0xf0, 0xff, 0xff),
    cname!(c"Azure1", 0xf0, 0xff, 0xff),
    cname!(c"Azure2", 0xe0, 0xee, 0xee),
    cname!(c"Azure3", 0xc1, 0xcd, 0xcd),
    cname!(c"Azure4", 0x83, 0x8b, 0x8b),
    cname!(c"Beige", 0xf5, 0xf5, 0xdc),
    cname!(c"Bisque", 0xff, 0xe4, 0xc4),
    cname!(c"Bisque1", 0xff, 0xe4, 0xc4),
    cname!(c"Bisque2", 0xee, 0xd5, 0xb7),
    cname!(c"Bisque3", 0xcd, 0xb7, 0x9e),
    cname!(c"Bisque4", 0x8b, 0x7d, 0x6b),
    cname!(c"Black", 0x00, 0x00, 0x00),
    cname!(c"BlanchedAlmond", 0xff, 0xeb, 0xcd),
    cname!(c"Blue", 0x00, 0x00, 0xff),
    cname!(c"Blue1", 0x0, 0x0, 0xff),
    cname!(c"Blue2", 0x0, 0x0, 0xee),
    cname!(c"Blue3", 0x0, 0x0, 0xcd),
    cname!(c"Blue4", 0x0, 0x0, 0x8b),
    cname!(c"BlueViolet", 0x8a, 0x2b, 0xe2),
    cname!(c"Brown", 0xa5, 0x2a, 0x2a),
    cname!(c"Brown1", 0xff, 0x40, 0x40),
    cname!(c"Brown2", 0xee, 0x3b, 0x3b),
    cname!(c"Brown3", 0xcd, 0x33, 0x33),
    cname!(c"Brown4", 0x8b, 0x23, 0x23),
    cname!(c"BurlyWood", 0xde, 0xb8, 0x87),
    cname!(c"Burlywood1", 0xff, 0xd3, 0x9b),
    cname!(c"Burlywood2", 0xee, 0xc5, 0x91),
    cname!(c"Burlywood3", 0xcd, 0xaa, 0x7d),
    cname!(c"Burlywood4", 0x8b, 0x73, 0x55),
    cname!(c"CadetBlue", 0x5f, 0x9e, 0xa0),
    cname!(c"CadetBlue1", 0x98, 0xf5, 0xff),
    cname!(c"CadetBlue2", 0x8e, 0xe5, 0xee),
    cname!(c"CadetBlue3", 0x7a, 0xc5, 0xcd),
    cname!(c"CadetBlue4", 0x53, 0x86, 0x8b),
    cname!(c"ChartReuse", 0x7f, 0xff, 0x00),
    cname!(c"Chartreuse1", 0x7f, 0xff, 0x0),
    cname!(c"Chartreuse2", 0x76, 0xee, 0x0),
    cname!(c"Chartreuse3", 0x66, 0xcd, 0x0),
    cname!(c"Chartreuse4", 0x45, 0x8b, 0x0),
    cname!(c"Chocolate", 0xd2, 0x69, 0x1e),
    cname!(c"Chocolate1", 0xff, 0x7f, 0x24),
    cname!(c"Chocolate2", 0xee, 0x76, 0x21),
    cname!(c"Chocolate3", 0xcd, 0x66, 0x1d),
    cname!(c"Chocolate4", 0x8b, 0x45, 0x13),
    cname!(c"Coral", 0xff, 0x7f, 0x50),
    cname!(c"Coral1", 0xff, 0x72, 0x56),
    cname!(c"Coral2", 0xee, 0x6a, 0x50),
    cname!(c"Coral3", 0xcd, 0x5b, 0x45),
    cname!(c"Coral4", 0x8b, 0x3e, 0x2f),
    cname!(c"CornFlowerBlue", 0x64, 0x95, 0xed),
    cname!(c"Cornsilk", 0xff, 0xf8, 0xdc),
    cname!(c"Cornsilk1", 0xff, 0xf8, 0xdc),
    cname!(c"Cornsilk2", 0xee, 0xe8, 0xcd),
    cname!(c"Cornsilk3", 0xcd, 0xc8, 0xb1),
    cname!(c"Cornsilk4", 0x8b, 0x88, 0x78),
    cname!(c"Crimson", 0xdc, 0x14, 0x3c),
    cname!(c"Cyan", 0x00, 0xff, 0xff),
    cname!(c"Cyan1", 0x0, 0xff, 0xff),
    cname!(c"Cyan2", 0x0, 0xee, 0xee),
    cname!(c"Cyan3", 0x0, 0xcd, 0xcd),
    cname!(c"Cyan4", 0x0, 0x8b, 0x8b),
    cname!(c"DarkBlue", 0x00, 0x00, 0x8b),
    cname!(c"DarkCyan", 0x00, 0x8b, 0x8b),
    cname!(c"DarkGoldenrod", 0xb8, 0x86, 0x0b),
    cname!(c"DarkGoldenrod1", 0xff, 0xb9, 0xf),
    cname!(c"DarkGoldenrod2", 0xee, 0xad, 0xe),
    cname!(c"DarkGoldenrod3", 0xcd, 0x95, 0xc),
    cname!(c"DarkGoldenrod4", 0x8b, 0x65, 0x8),
    cname!(c"DarkGray", 0xa9, 0xa9, 0xa9),
    cname!(c"DarkGreen", 0x00, 0x64, 0x00),
    cname!(c"DarkGrey", 0xa9, 0xa9, 0xa9),
    cname!(c"DarkKhaki", 0xbd, 0xb7, 0x6b),
    cname!(c"DarkMagenta", 0x8b, 0x00, 0x8b),
    cname!(c"DarkOliveGreen", 0x55, 0x6b, 0x2f),
    cname!(c"DarkOliveGreen1", 0xca, 0xff, 0x70),
    cname!(c"DarkOliveGreen2", 0xbc, 0xee, 0x68),
    cname!(c"DarkOliveGreen3", 0xa2, 0xcd, 0x5a),
    cname!(c"DarkOliveGreen4", 0x6e, 0x8b, 0x3d),
    cname!(c"DarkOrange", 0xff, 0x8c, 0x00),
    cname!(c"DarkOrange1", 0xff, 0x7f, 0x0),
    cname!(c"DarkOrange2", 0xee, 0x76, 0x0),
    cname!(c"DarkOrange3", 0xcd, 0x66, 0x0),
    cname!(c"DarkOrange4", 0x8b, 0x45, 0x0),
    cname!(c"DarkOrchid", 0x99, 0x32, 0xcc),
    cname!(c"DarkOrchid1", 0xbf, 0x3e, 0xff),
    cname!(c"DarkOrchid2", 0xb2, 0x3a, 0xee),
    cname!(c"DarkOrchid3", 0x9a, 0x32, 0xcd),
    cname!(c"DarkOrchid4", 0x68, 0x22, 0x8b),
    cname!(c"DarkRed", 0x8b, 0x00, 0x00),
    cname!(c"DarkSalmon", 0xe9, 0x96, 0x7a),
    cname!(c"DarkSeaGreen", 0x8f, 0xbc, 0x8f),
    cname!(c"DarkSeaGreen1", 0xc1, 0xff, 0xc1),
    cname!(c"DarkSeaGreen2", 0xb4, 0xee, 0xb4),
    cname!(c"DarkSeaGreen3", 0x9b, 0xcd, 0x9b),
    cname!(c"DarkSeaGreen4", 0x69, 0x8b, 0x69),
    cname!(c"DarkSlateBlue", 0x48, 0x3d, 0x8b),
    cname!(c"DarkSlateGray", 0x2f, 0x4f, 0x4f),
    cname!(c"DarkSlateGray1", 0x97, 0xff, 0xff),
    cname!(c"DarkSlateGray2", 0x8d, 0xee, 0xee),
    cname!(c"DarkSlateGray3", 0x79, 0xcd, 0xcd),
    cname!(c"DarkSlateGray4", 0x52, 0x8b, 0x8b),
    cname!(c"DarkSlateGrey", 0x2f, 0x4f, 0x4f),
    cname!(c"DarkTurquoise", 0x00, 0xce, 0xd1),
    cname!(c"DarkViolet", 0x94, 0x00, 0xd3),
    cname!(c"DarkYellow", 0xbb, 0xbb, 0x00),
    cname!(c"DeepPink", 0xff, 0x14, 0x93),
    cname!(c"DeepPink1", 0xff, 0x14, 0x93),
    cname!(c"DeepPink2", 0xee, 0x12, 0x89),
    cname!(c"DeepPink3", 0xcd, 0x10, 0x76),
    cname!(c"DeepPink4", 0x8b, 0xa, 0x50),
    cname!(c"DeepSkyBlue", 0x00, 0xbf, 0xff),
    cname!(c"DeepSkyBlue1", 0x0, 0xbf, 0xff),
    cname!(c"DeepSkyBlue2", 0x0, 0xb2, 0xee),
    cname!(c"DeepSkyBlue3", 0x0, 0x9a, 0xcd),
    cname!(c"DeepSkyBlue4", 0x0, 0x68, 0x8b),
    cname!(c"DimGray", 0x69, 0x69, 0x69),
    cname!(c"DimGrey", 0x69, 0x69, 0x69),
    cname!(c"DodgerBlue", 0x1e, 0x90, 0xff),
    cname!(c"DodgerBlue1", 0x1e, 0x90, 0xff),
    cname!(c"DodgerBlue2", 0x1c, 0x86, 0xee),
    cname!(c"DodgerBlue3", 0x18, 0x74, 0xcd),
    cname!(c"DodgerBlue4", 0x10, 0x4e, 0x8b),
    cname!(c"Firebrick", 0xb2, 0x22, 0x22),
    cname!(c"Firebrick1", 0xff, 0x30, 0x30),
    cname!(c"Firebrick2", 0xee, 0x2c, 0x2c),
    cname!(c"Firebrick3", 0xcd, 0x26, 0x26),
    cname!(c"Firebrick4", 0x8b, 0x1a, 0x1a),
    cname!(c"FloralWhite", 0xff, 0xfa, 0xf0),
    cname!(c"ForestGreen", 0x22, 0x8b, 0x22),
    cname!(c"Fuchsia", 0xff, 0x00, 0xff),
    cname!(c"Gainsboro", 0xdc, 0xdc, 0xdc),
    cname!(c"GhostWhite", 0xf8, 0xf8, 0xff),
    cname!(c"Gold", 0xff, 0xd7, 0x00),
    cname!(c"Gold1", 0xff, 0xd7, 0x0),
    cname!(c"Gold2", 0xee, 0xc9, 0x0),
    cname!(c"Gold3", 0xcd, 0xad, 0x0),
    cname!(c"Gold4", 0x8b, 0x75, 0x0),
    cname!(c"Goldenrod", 0xda, 0xa5, 0x20),
    cname!(c"Goldenrod1", 0xff, 0xc1, 0x25),
    cname!(c"Goldenrod2", 0xee, 0xb4, 0x22),
    cname!(c"Goldenrod3", 0xcd, 0x9b, 0x1d),
    cname!(c"Goldenrod4", 0x8b, 0x69, 0x14),
    cname!(c"Gray", 0x80, 0x80, 0x80),
    cname!(c"Gray0", 0x0, 0x0, 0x0),
    cname!(c"Gray1", 0x3, 0x3, 0x3),
    cname!(c"Gray10", 0x1a, 0x1a, 0x1a),
    cname!(c"Gray100", 0xff, 0xff, 0xff),
    cname!(c"Gray11", 0x1c, 0x1c, 0x1c),
    cname!(c"Gray12", 0x1f, 0x1f, 0x1f),
    cname!(c"Gray13", 0x21, 0x21, 0x21),
    cname!(c"Gray14", 0x24, 0x24, 0x24),
    cname!(c"Gray15", 0x26, 0x26, 0x26),
    cname!(c"Gray16", 0x29, 0x29, 0x29),
    cname!(c"Gray17", 0x2b, 0x2b, 0x2b),
    cname!(c"Gray18", 0x2e, 0x2e, 0x2e),
    cname!(c"Gray19", 0x30, 0x30, 0x30),
    cname!(c"Gray2", 0x5, 0x5, 0x5),
    cname!(c"Gray20", 0x33, 0x33, 0x33),
    cname!(c"Gray21", 0x36, 0x36, 0x36),
    cname!(c"Gray22", 0x38, 0x38, 0x38),
    cname!(c"Gray23", 0x3b, 0x3b, 0x3b),
    cname!(c"Gray24", 0x3d, 0x3d, 0x3d),
    cname!(c"Gray25", 0x40, 0x40, 0x40),
    cname!(c"Gray26", 0x42, 0x42, 0x42),
    cname!(c"Gray27", 0x45, 0x45, 0x45),
    cname!(c"Gray28", 0x47, 0x47, 0x47),
    cname!(c"Gray29", 0x4a, 0x4a, 0x4a),
    cname!(c"Gray3", 0x8, 0x8, 0x8),
    cname!(c"Gray30", 0x4d, 0x4d, 0x4d),
    cname!(c"Gray31", 0x4f, 0x4f, 0x4f),
    cname!(c"Gray32", 0x52, 0x52, 0x52),
    cname!(c"Gray33", 0x54, 0x54, 0x54),
    cname!(c"Gray34", 0x57, 0x57, 0x57),
    cname!(c"Gray35", 0x59, 0x59, 0x59),
    cname!(c"Gray36", 0x5c, 0x5c, 0x5c),
    cname!(c"Gray37", 0x5e, 0x5e, 0x5e),
    cname!(c"Gray38", 0x61, 0x61, 0x61),
    cname!(c"Gray39", 0x63, 0x63, 0x63),
    cname!(c"Gray4", 0xa, 0xa, 0xa),
    cname!(c"Gray40", 0x66, 0x66, 0x66),
    cname!(c"Gray41", 0x69, 0x69, 0x69),
    cname!(c"Gray42", 0x6b, 0x6b, 0x6b),
    cname!(c"Gray43", 0x6e, 0x6e, 0x6e),
    cname!(c"Gray44", 0x70, 0x70, 0x70),
    cname!(c"Gray45", 0x73, 0x73, 0x73),
    cname!(c"Gray46", 0x75, 0x75, 0x75),
    cname!(c"Gray47", 0x78, 0x78, 0x78),
    cname!(c"Gray48", 0x7a, 0x7a, 0x7a),
    cname!(c"Gray49", 0x7d, 0x7d, 0x7d),
    cname!(c"Gray5", 0xd, 0xd, 0xd),
    cname!(c"Gray50", 0x7f, 0x7f, 0x7f),
    cname!(c"Gray51", 0x82, 0x82, 0x82),
    cname!(c"Gray52", 0x85, 0x85, 0x85),
    cname!(c"Gray53", 0x87, 0x87, 0x87),
    cname!(c"Gray54", 0x8a, 0x8a, 0x8a),
    cname!(c"Gray55", 0x8c, 0x8c, 0x8c),
    cname!(c"Gray56", 0x8f, 0x8f, 0x8f),
    cname!(c"Gray57", 0x91, 0x91, 0x91),
    cname!(c"Gray58", 0x94, 0x94, 0x94),
    cname!(c"Gray59", 0x96, 0x96, 0x96),
    cname!(c"Gray6", 0xf, 0xf, 0xf),
    cname!(c"Gray60", 0x99, 0x99, 0x99),
    cname!(c"Gray61", 0x9c, 0x9c, 0x9c),
    cname!(c"Gray62", 0x9e, 0x9e, 0x9e),
    cname!(c"Gray63", 0xa1, 0xa1, 0xa1),
    cname!(c"Gray64", 0xa3, 0xa3, 0xa3),
    cname!(c"Gray65", 0xa6, 0xa6, 0xa6),
    cname!(c"Gray66", 0xa8, 0xa8, 0xa8),
    cname!(c"Gray67", 0xab, 0xab, 0xab),
    cname!(c"Gray68", 0xad, 0xad, 0xad),
    cname!(c"Gray69", 0xb0, 0xb0, 0xb0),
    cname!(c"Gray7", 0x12, 0x12, 0x12),
    cname!(c"Gray70", 0xb3, 0xb3, 0xb3),
    cname!(c"Gray71", 0xb5, 0xb5, 0xb5),
    cname!(c"Gray72", 0xb8, 0xb8, 0xb8),
    cname!(c"Gray73", 0xba, 0xba, 0xba),
    cname!(c"Gray74", 0xbd, 0xbd, 0xbd),
    cname!(c"Gray75", 0xbf, 0xbf, 0xbf),
    cname!(c"Gray76", 0xc2, 0xc2, 0xc2),
    cname!(c"Gray77", 0xc4, 0xc4, 0xc4),
    cname!(c"Gray78", 0xc7, 0xc7, 0xc7),
    cname!(c"Gray79", 0xc9, 0xc9, 0xc9),
    cname!(c"Gray8", 0x14, 0x14, 0x14),
    cname!(c"Gray80", 0xcc, 0xcc, 0xcc),
    cname!(c"Gray81", 0xcf, 0xcf, 0xcf),
    cname!(c"Gray82", 0xd1, 0xd1, 0xd1),
    cname!(c"Gray83", 0xd4, 0xd4, 0xd4),
    cname!(c"Gray84", 0xd6, 0xd6, 0xd6),
    cname!(c"Gray85", 0xd9, 0xd9, 0xd9),
    cname!(c"Gray86", 0xdb, 0xdb, 0xdb),
    cname!(c"Gray87", 0xde, 0xde, 0xde),
    cname!(c"Gray88", 0xe0, 0xe0, 0xe0),
    cname!(c"Gray89", 0xe3, 0xe3, 0xe3),
    cname!(c"Gray9", 0x17, 0x17, 0x17),
    cname!(c"Gray90", 0xe5, 0xe5, 0xe5),
    cname!(c"Gray91", 0xe8, 0xe8, 0xe8),
    cname!(c"Gray92", 0xeb, 0xeb, 0xeb),
    cname!(c"Gray93", 0xed, 0xed, 0xed),
    cname!(c"Gray94", 0xf0, 0xf0, 0xf0),
    cname!(c"Gray95", 0xf2, 0xf2, 0xf2),
    cname!(c"Gray96", 0xf5, 0xf5, 0xf5),
    cname!(c"Gray97", 0xf7, 0xf7, 0xf7),
    cname!(c"Gray98", 0xfa, 0xfa, 0xfa),
    cname!(c"Gray99", 0xfc, 0xfc, 0xfc),
    cname!(c"Green", 0x00, 0x80, 0x00),
    cname!(c"Green1", 0x0, 0xff, 0x0),
    cname!(c"Green2", 0x0, 0xee, 0x0),
    cname!(c"Green3", 0x0, 0xcd, 0x0),
    cname!(c"Green4", 0x0, 0x8b, 0x0),
    cname!(c"GreenYellow", 0xad, 0xff, 0x2f),
    cname!(c"Grey", 0x80, 0x80, 0x80),
    cname!(c"Grey0", 0x0, 0x0, 0x0),
    cname!(c"Grey1", 0x3, 0x3, 0x3),
    cname!(c"Grey10", 0x1a, 0x1a, 0x1a),
    cname!(c"Grey100", 0xff, 0xff, 0xff),
    cname!(c"Grey11", 0x1c, 0x1c, 0x1c),
    cname!(c"Grey12", 0x1f, 0x1f, 0x1f),
    cname!(c"Grey13", 0x21, 0x21, 0x21),
    cname!(c"Grey14", 0x24, 0x24, 0x24),
    cname!(c"Grey15", 0x26, 0x26, 0x26),
    cname!(c"Grey16", 0x29, 0x29, 0x29),
    cname!(c"Grey17", 0x2b, 0x2b, 0x2b),
    cname!(c"Grey18", 0x2e, 0x2e, 0x2e),
    cname!(c"Grey19", 0x30, 0x30, 0x30),
    cname!(c"Grey2", 0x5, 0x5, 0x5),
    cname!(c"Grey20", 0x33, 0x33, 0x33),
    cname!(c"Grey21", 0x36, 0x36, 0x36),
    cname!(c"Grey22", 0x38, 0x38, 0x38),
    cname!(c"Grey23", 0x3b, 0x3b, 0x3b),
    cname!(c"Grey24", 0x3d, 0x3d, 0x3d),
    cname!(c"Grey25", 0x40, 0x40, 0x40),
    cname!(c"Grey26", 0x42, 0x42, 0x42),
    cname!(c"Grey27", 0x45, 0x45, 0x45),
    cname!(c"Grey28", 0x47, 0x47, 0x47),
    cname!(c"Grey29", 0x4a, 0x4a, 0x4a),
    cname!(c"Grey3", 0x8, 0x8, 0x8),
    cname!(c"Grey30", 0x4d, 0x4d, 0x4d),
    cname!(c"Grey31", 0x4f, 0x4f, 0x4f),
    cname!(c"Grey32", 0x52, 0x52, 0x52),
    cname!(c"Grey33", 0x54, 0x54, 0x54),
    cname!(c"Grey34", 0x57, 0x57, 0x57),
    cname!(c"Grey35", 0x59, 0x59, 0x59),
    cname!(c"Grey36", 0x5c, 0x5c, 0x5c),
    cname!(c"Grey37", 0x5e, 0x5e, 0x5e),
    cname!(c"Grey38", 0x61, 0x61, 0x61),
    cname!(c"Grey39", 0x63, 0x63, 0x63),
    cname!(c"Grey4", 0xa, 0xa, 0xa),
    cname!(c"Grey40", 0x66, 0x66, 0x66),
    cname!(c"Grey41", 0x69, 0x69, 0x69),
    cname!(c"Grey42", 0x6b, 0x6b, 0x6b),
    cname!(c"Grey43", 0x6e, 0x6e, 0x6e),
    cname!(c"Grey44", 0x70, 0x70, 0x70),
    cname!(c"Grey45", 0x73, 0x73, 0x73),
    cname!(c"Grey46", 0x75, 0x75, 0x75),
    cname!(c"Grey47", 0x78, 0x78, 0x78),
    cname!(c"Grey48", 0x7a, 0x7a, 0x7a),
    cname!(c"Grey49", 0x7d, 0x7d, 0x7d),
    cname!(c"Grey5", 0xd, 0xd, 0xd),
    cname!(c"Grey50", 0x7f, 0x7f, 0x7f),
    cname!(c"Grey51", 0x82, 0x82, 0x82),
    cname!(c"Grey52", 0x85, 0x85, 0x85),
    cname!(c"Grey53", 0x87, 0x87, 0x87),
    cname!(c"Grey54", 0x8a, 0x8a, 0x8a),
    cname!(c"Grey55", 0x8c, 0x8c, 0x8c),
    cname!(c"Grey56", 0x8f, 0x8f, 0x8f),
    cname!(c"Grey57", 0x91, 0x91, 0x91),
    cname!(c"Grey58", 0x94, 0x94, 0x94),
    cname!(c"Grey59", 0x96, 0x96, 0x96),
    cname!(c"Grey6", 0xf, 0xf, 0xf),
    cname!(c"Grey60", 0x99, 0x99, 0x99),
    cname!(c"Grey61", 0x9c, 0x9c, 0x9c),
    cname!(c"Grey62", 0x9e, 0x9e, 0x9e),
    cname!(c"Grey63", 0xa1, 0xa1, 0xa1),
    cname!(c"Grey64", 0xa3, 0xa3, 0xa3),
    cname!(c"Grey65", 0xa6, 0xa6, 0xa6),
    cname!(c"Grey66", 0xa8, 0xa8, 0xa8),
    cname!(c"Grey67", 0xab, 0xab, 0xab),
    cname!(c"Grey68", 0xad, 0xad, 0xad),
    cname!(c"Grey69", 0xb0, 0xb0, 0xb0),
    cname!(c"Grey7", 0x12, 0x12, 0x12),
    cname!(c"Grey70", 0xb3, 0xb3, 0xb3),
    cname!(c"Grey71", 0xb5, 0xb5, 0xb5),
    cname!(c"Grey72", 0xb8, 0xb8, 0xb8),
    cname!(c"Grey73", 0xba, 0xba, 0xba),
    cname!(c"Grey74", 0xbd, 0xbd, 0xbd),
    cname!(c"Grey75", 0xbf, 0xbf, 0xbf),
    cname!(c"Grey76", 0xc2, 0xc2, 0xc2),
    cname!(c"Grey77", 0xc4, 0xc4, 0xc4),
    cname!(c"Grey78", 0xc7, 0xc7, 0xc7),
    cname!(c"Grey79", 0xc9, 0xc9, 0xc9),
    cname!(c"Grey8", 0x14, 0x14, 0x14),
    cname!(c"Grey80", 0xcc, 0xcc, 0xcc),
    cname!(c"Grey81", 0xcf, 0xcf, 0xcf),
    cname!(c"Grey82", 0xd1, 0xd1, 0xd1),
    cname!(c"Grey83", 0xd4, 0xd4, 0xd4),
    cname!(c"Grey84", 0xd6, 0xd6, 0xd6),
    cname!(c"Grey85", 0xd9, 0xd9, 0xd9),
    cname!(c"Grey86", 0xdb, 0xdb, 0xdb),
    cname!(c"Grey87", 0xde, 0xde, 0xde),
    cname!(c"Grey88", 0xe0, 0xe0, 0xe0),
    cname!(c"Grey89", 0xe3, 0xe3, 0xe3),
    cname!(c"Grey9", 0x17, 0x17, 0x17),
    cname!(c"Grey90", 0xe5, 0xe5, 0xe5),
    cname!(c"Grey91", 0xe8, 0xe8, 0xe8),
    cname!(c"Grey92", 0xeb, 0xeb, 0xeb),
    cname!(c"Grey93", 0xed, 0xed, 0xed),
    cname!(c"Grey94", 0xf0, 0xf0, 0xf0),
    cname!(c"Grey95", 0xf2, 0xf2, 0xf2),
    cname!(c"Grey96", 0xf5, 0xf5, 0xf5),
    cname!(c"Grey97", 0xf7, 0xf7, 0xf7),
    cname!(c"Grey98", 0xfa, 0xfa, 0xfa),
    cname!(c"Grey99", 0xfc, 0xfc, 0xfc),
    cname!(c"Honeydew", 0xf0, 0xff, 0xf0),
    cname!(c"Honeydew1", 0xf0, 0xff, 0xf0),
    cname!(c"Honeydew2", 0xe0, 0xee, 0xe0),
    cname!(c"Honeydew3", 0xc1, 0xcd, 0xc1),
    cname!(c"Honeydew4", 0x83, 0x8b, 0x83),
    cname!(c"HotPink", 0xff, 0x69, 0xb4),
    cname!(c"HotPink1", 0xff, 0x6e, 0xb4),
    cname!(c"HotPink2", 0xee, 0x6a, 0xa7),
    cname!(c"HotPink3", 0xcd, 0x60, 0x90),
    cname!(c"HotPink4", 0x8b, 0x3a, 0x62),
    cname!(c"IndianRed", 0xcd, 0x5c, 0x5c),
    cname!(c"IndianRed1", 0xff, 0x6a, 0x6a),
    cname!(c"IndianRed2", 0xee, 0x63, 0x63),
    cname!(c"IndianRed3", 0xcd, 0x55, 0x55),
    cname!(c"IndianRed4", 0x8b, 0x3a, 0x3a),
    cname!(c"Indigo", 0x4b, 0x00, 0x82),
    cname!(c"Ivory", 0xff, 0xff, 0xf0),
    cname!(c"Ivory1", 0xff, 0xff, 0xf0),
    cname!(c"Ivory2", 0xee, 0xee, 0xe0),
    cname!(c"Ivory3", 0xcd, 0xcd, 0xc1),
    cname!(c"Ivory4", 0x8b, 0x8b, 0x83),
    cname!(c"Khaki", 0xf0, 0xe6, 0x8c),
    cname!(c"Khaki1", 0xff, 0xf6, 0x8f),
    cname!(c"Khaki2", 0xee, 0xe6, 0x85),
    cname!(c"Khaki3", 0xcd, 0xc6, 0x73),
    cname!(c"Khaki4", 0x8b, 0x86, 0x4e),
    cname!(c"Lavender", 0xe6, 0xe6, 0xfa),
    cname!(c"LavenderBlush", 0xff, 0xf0, 0xf5),
    cname!(c"LavenderBlush1", 0xff, 0xf0, 0xf5),
    cname!(c"LavenderBlush2", 0xee, 0xe0, 0xe5),
    cname!(c"LavenderBlush3", 0xcd, 0xc1, 0xc5),
    cname!(c"LavenderBlush4", 0x8b, 0x83, 0x86),
    cname!(c"LawnGreen", 0x7c, 0xfc, 0x00),
    cname!(c"LemonChiffon", 0xff, 0xfa, 0xcd),
    cname!(c"LemonChiffon1", 0xff, 0xfa, 0xcd),
    cname!(c"LemonChiffon2", 0xee, 0xe9, 0xbf),
    cname!(c"LemonChiffon3", 0xcd, 0xc9, 0xa5),
    cname!(c"LemonChiffon4", 0x8b, 0x89, 0x70),
    cname!(c"LightBlue", 0xad, 0xd8, 0xe6),
    cname!(c"LightBlue1", 0xbf, 0xef, 0xff),
    cname!(c"LightBlue2", 0xb2, 0xdf, 0xee),
    cname!(c"LightBlue3", 0x9a, 0xc0, 0xcd),
    cname!(c"LightBlue4", 0x68, 0x83, 0x8b),
    cname!(c"LightCoral", 0xf0, 0x80, 0x80),
    cname!(c"LightCyan", 0xe0, 0xff, 0xff),
    cname!(c"LightCyan1", 0xe0, 0xff, 0xff),
    cname!(c"LightCyan2", 0xd1, 0xee, 0xee),
    cname!(c"LightCyan3", 0xb4, 0xcd, 0xcd),
    cname!(c"LightCyan4", 0x7a, 0x8b, 0x8b),
    cname!(c"LightGoldenrod", 0xee, 0xdd, 0x82),
    cname!(c"LightGoldenrod1", 0xff, 0xec, 0x8b),
    cname!(c"LightGoldenrod2", 0xee, 0xdc, 0x82),
    cname!(c"LightGoldenrod3", 0xcd, 0xbe, 0x70),
    cname!(c"LightGoldenrod4", 0x8b, 0x81, 0x4c),
    cname!(c"LightGoldenrodYellow", 0xfa, 0xfa, 0xd2),
    cname!(c"LightGray", 0xd3, 0xd3, 0xd3),
    cname!(c"LightGreen", 0x90, 0xee, 0x90),
    cname!(c"LightGrey", 0xd3, 0xd3, 0xd3),
    cname!(c"LightMagenta", 0xff, 0xbb, 0xff),
    cname!(c"LightPink", 0xff, 0xb6, 0xc1),
    cname!(c"LightPink1", 0xff, 0xae, 0xb9),
    cname!(c"LightPink2", 0xee, 0xa2, 0xad),
    cname!(c"LightPink3", 0xcd, 0x8c, 0x95),
    cname!(c"LightPink4", 0x8b, 0x5f, 0x65),
    cname!(c"LightRed", 0xff, 0xbb, 0xbb),
    cname!(c"LightSalmon", 0xff, 0xa0, 0x7a),
    cname!(c"LightSalmon1", 0xff, 0xa0, 0x7a),
    cname!(c"LightSalmon2", 0xee, 0x95, 0x72),
    cname!(c"LightSalmon3", 0xcd, 0x81, 0x62),
    cname!(c"LightSalmon4", 0x8b, 0x57, 0x42),
    cname!(c"LightSeaGreen", 0x20, 0xb2, 0xaa),
    cname!(c"LightSkyBlue", 0x87, 0xce, 0xfa),
    cname!(c"LightSkyBlue1", 0xb0, 0xe2, 0xff),
    cname!(c"LightSkyBlue2", 0xa4, 0xd3, 0xee),
    cname!(c"LightSkyBlue3", 0x8d, 0xb6, 0xcd),
    cname!(c"LightSkyBlue4", 0x60, 0x7b, 0x8b),
    cname!(c"LightSlateBlue", 0x84, 0x70, 0xff),
    cname!(c"LightSlateGray", 0x77, 0x88, 0x99),
    cname!(c"LightSlateGrey", 0x77, 0x88, 0x99),
    cname!(c"LightSteelBlue", 0xb0, 0xc4, 0xde),
    cname!(c"LightSteelBlue1", 0xca, 0xe1, 0xff),
    cname!(c"LightSteelBlue2", 0xbc, 0xd2, 0xee),
    cname!(c"LightSteelBlue3", 0xa2, 0xb5, 0xcd),
    cname!(c"LightSteelBlue4", 0x6e, 0x7b, 0x8b),
    cname!(c"LightYellow", 0xff, 0xff, 0xe0),
    cname!(c"LightYellow1", 0xff, 0xff, 0xe0),
    cname!(c"LightYellow2", 0xee, 0xee, 0xd1),
    cname!(c"LightYellow3", 0xcd, 0xcd, 0xb4),
    cname!(c"LightYellow4", 0x8b, 0x8b, 0x7a),
    cname!(c"Lime", 0x00, 0xff, 0x00),
    cname!(c"LimeGreen", 0x32, 0xcd, 0x32),
    cname!(c"Linen", 0xfa, 0xf0, 0xe6),
    cname!(c"Magenta", 0xff, 0x00, 0xff),
    cname!(c"Magenta1", 0xff, 0x0, 0xff),
    cname!(c"Magenta2", 0xee, 0x0, 0xee),
    cname!(c"Magenta3", 0xcd, 0x0, 0xcd),
    cname!(c"Magenta4", 0x8b, 0x0, 0x8b),
    cname!(c"Maroon", 0x80, 0x00, 0x00),
    cname!(c"Maroon1", 0xff, 0x34, 0xb3),
    cname!(c"Maroon2", 0xee, 0x30, 0xa7),
    cname!(c"Maroon3", 0xcd, 0x29, 0x90),
    cname!(c"Maroon4", 0x8b, 0x1c, 0x62),
    cname!(c"MediumAquamarine", 0x66, 0xcd, 0xaa),
    cname!(c"MediumBlue", 0x00, 0x00, 0xcd),
    cname!(c"MediumOrchid", 0xba, 0x55, 0xd3),
    cname!(c"MediumOrchid1", 0xe0, 0x66, 0xff),
    cname!(c"MediumOrchid2", 0xd1, 0x5f, 0xee),
    cname!(c"MediumOrchid3", 0xb4, 0x52, 0xcd),
    cname!(c"MediumOrchid4", 0x7a, 0x37, 0x8b),
    cname!(c"MediumPurple", 0x93, 0x70, 0xdb),
    cname!(c"MediumPurple1", 0xab, 0x82, 0xff),
    cname!(c"MediumPurple2", 0x9f, 0x79, 0xee),
    cname!(c"MediumPurple3", 0x89, 0x68, 0xcd),
    cname!(c"MediumPurple4", 0x5d, 0x47, 0x8b),
    cname!(c"MediumSeaGreen", 0x3c, 0xb3, 0x71),
    cname!(c"MediumSlateBlue", 0x7b, 0x68, 0xee),
    cname!(c"MediumSpringGreen", 0x00, 0xfa, 0x9a),
    cname!(c"MediumTurquoise", 0x48, 0xd1, 0xcc),
    cname!(c"MediumVioletRed", 0xc7, 0x15, 0x85),
    cname!(c"MidnightBlue", 0x19, 0x19, 0x70),
    cname!(c"MintCream", 0xf5, 0xff, 0xfa),
    cname!(c"MistyRose", 0xff, 0xe4, 0xe1),
    cname!(c"MistyRose1", 0xff, 0xe4, 0xe1),
    cname!(c"MistyRose2", 0xee, 0xd5, 0xd2),
    cname!(c"MistyRose3", 0xcd, 0xb7, 0xb5),
    cname!(c"MistyRose4", 0x8b, 0x7d, 0x7b),
    cname!(c"Moccasin", 0xff, 0xe4, 0xb5),
    cname!(c"NavajoWhite", 0xff, 0xde, 0xad),
    cname!(c"NavajoWhite1", 0xff, 0xde, 0xad),
    cname!(c"NavajoWhite2", 0xee, 0xcf, 0xa1),
    cname!(c"NavajoWhite3", 0xcd, 0xb3, 0x8b),
    cname!(c"NavajoWhite4", 0x8b, 0x79, 0x5e),
    cname!(c"Navy", 0x00, 0x00, 0x80),
    cname!(c"NavyBlue", 0x0, 0x0, 0x80),
    cname!(c"NvimDarkBlue", 0x00, 0x4c, 0x73),
    cname!(c"NvimDarkCyan", 0x00, 0x73, 0x73),
    cname!(c"NvimDarkGray1", 0x07, 0x08, 0x0d),
    cname!(c"NvimDarkGray2", 0x14, 0x16, 0x1b),
    cname!(c"NvimDarkGray3", 0x2c, 0x2e, 0x33),
    cname!(c"NvimDarkGray4", 0x4f, 0x52, 0x58),
    cname!(c"NvimDarkGreen", 0x00, 0x55, 0x23),
    cname!(c"NvimDarkGrey1", 0x07, 0x08, 0x0d),
    cname!(c"NvimDarkGrey2", 0x14, 0x16, 0x1b),
    cname!(c"NvimDarkGrey3", 0x2c, 0x2e, 0x33),
    cname!(c"NvimDarkGrey4", 0x4f, 0x52, 0x58),
    cname!(c"NvimDarkMagenta", 0x47, 0x00, 0x45),
    cname!(c"NvimDarkRed", 0x59, 0x00, 0x08),
    cname!(c"NvimDarkYellow", 0x6b, 0x53, 0x00),
    cname!(c"NvimLightBlue", 0xa6, 0xdb, 0xff),
    cname!(c"NvimLightCyan", 0x8c, 0xf8, 0xf7),
    cname!(c"NvimLightGray1", 0xee, 0xf1, 0xf8),
    cname!(c"NvimLightGray2", 0xe0, 0xe2, 0xea),
    cname!(c"NvimLightGray3", 0xc4, 0xc6, 0xcd),
    cname!(c"NvimLightGray4", 0x9b, 0x9e, 0xa4),
    cname!(c"NvimLightGreen", 0xb3, 0xf6, 0xc0),
    cname!(c"NvimLightGrey1", 0xee, 0xf1, 0xf8),
    cname!(c"NvimLightGrey2", 0xe0, 0xe2, 0xea),
    cname!(c"NvimLightGrey3", 0xc4, 0xc6, 0xcd),
    cname!(c"NvimLightGrey4", 0x9b, 0x9e, 0xa4),
    cname!(c"NvimLightMagenta", 0xff, 0xca, 0xff),
    cname!(c"NvimLightRed", 0xff, 0xc0, 0xb9),
    cname!(c"NvimLightYellow", 0xfc, 0xe0, 0x94),
    cname!(c"OldLace", 0xfd, 0xf5, 0xe6),
    cname!(c"Olive", 0x80, 0x80, 0x00),
    cname!(c"OliveDrab", 0x6b, 0x8e, 0x23),
    cname!(c"OliveDrab1", 0xc0, 0xff, 0x3e),
    cname!(c"OliveDrab2", 0xb3, 0xee, 0x3a),
    cname!(c"OliveDrab3", 0x9a, 0xcd, 0x32),
    cname!(c"OliveDrab4", 0x69, 0x8b, 0x22),
    cname!(c"Orange", 0xff, 0xa5, 0x00),
    cname!(c"Orange1", 0xff, 0xa5, 0x0),
    cname!(c"Orange2", 0xee, 0x9a, 0x0),
    cname!(c"Orange3", 0xcd, 0x85, 0x0),
    cname!(c"Orange4", 0x8b, 0x5a, 0x0),
    cname!(c"OrangeRed", 0xff, 0x45, 0x00),
    cname!(c"OrangeRed1", 0xff, 0x45, 0x0),
    cname!(c"OrangeRed2", 0xee, 0x40, 0x0),
    cname!(c"OrangeRed3", 0xcd, 0x37, 0x0),
    cname!(c"OrangeRed4", 0x8b, 0x25, 0x0),
    cname!(c"Orchid", 0xda, 0x70, 0xd6),
    cname!(c"Orchid1", 0xff, 0x83, 0xfa),
    cname!(c"Orchid2", 0xee, 0x7a, 0xe9),
    cname!(c"Orchid3", 0xcd, 0x69, 0xc9),
    cname!(c"Orchid4", 0x8b, 0x47, 0x89),
    cname!(c"PaleGoldenrod", 0xee, 0xe8, 0xaa),
    cname!(c"PaleGreen", 0x98, 0xfb, 0x98),
    cname!(c"PaleGreen1", 0x9a, 0xff, 0x9a),
    cname!(c"PaleGreen2", 0x90, 0xee, 0x90),
    cname!(c"PaleGreen3", 0x7c, 0xcd, 0x7c),
    cname!(c"PaleGreen4", 0x54, 0x8b, 0x54),
    cname!(c"PaleTurquoise", 0xaf, 0xee, 0xee),
    cname!(c"PaleTurquoise1", 0xbb, 0xff, 0xff),
    cname!(c"PaleTurquoise2", 0xae, 0xee, 0xee),
    cname!(c"PaleTurquoise3", 0x96, 0xcd, 0xcd),
    cname!(c"PaleTurquoise4", 0x66, 0x8b, 0x8b),
    cname!(c"PaleVioletRed", 0xdb, 0x70, 0x93),
    cname!(c"PaleVioletRed1", 0xff, 0x82, 0xab),
    cname!(c"PaleVioletRed2", 0xee, 0x79, 0x9f),
    cname!(c"PaleVioletRed3", 0xcd, 0x68, 0x89),
    cname!(c"PaleVioletRed4", 0x8b, 0x47, 0x5d),
    cname!(c"PapayaWhip", 0xff, 0xef, 0xd5),
    cname!(c"PeachPuff", 0xff, 0xda, 0xb9),
    cname!(c"PeachPuff1", 0xff, 0xda, 0xb9),
    cname!(c"PeachPuff2", 0xee, 0xcb, 0xad),
    cname!(c"PeachPuff3", 0xcd, 0xaf, 0x95),
    cname!(c"PeachPuff4", 0x8b, 0x77, 0x65),
    cname!(c"Peru", 0xcd, 0x85, 0x3f),
    cname!(c"Pink", 0xff, 0xc0, 0xcb),
    cname!(c"Pink1", 0xff, 0xb5, 0xc5),
    cname!(c"Pink2", 0xee, 0xa9, 0xb8),
    cname!(c"Pink3", 0xcd, 0x91, 0x9e),
    cname!(c"Pink4", 0x8b, 0x63, 0x6c),
    cname!(c"Plum", 0xdd, 0xa0, 0xdd),
    cname!(c"Plum1", 0xff, 0xbb, 0xff),
    cname!(c"Plum2", 0xee, 0xae, 0xee),
    cname!(c"Plum3", 0xcd, 0x96, 0xcd),
    cname!(c"Plum4", 0x8b, 0x66, 0x8b),
    cname!(c"PowderBlue", 0xb0, 0xe0, 0xe6),
    cname!(c"Purple", 0x80, 0x00, 0x80),
    cname!(c"Purple1", 0x9b, 0x30, 0xff),
    cname!(c"Purple2", 0x91, 0x2c, 0xee),
    cname!(c"Purple3", 0x7d, 0x26, 0xcd),
    cname!(c"Purple4", 0x55, 0x1a, 0x8b),
    cname!(c"RebeccaPurple", 0x66, 0x33, 0x99),
    cname!(c"Red", 0xff, 0x00, 0x00),
    cname!(c"Red1", 0xff, 0x0, 0x0),
    cname!(c"Red2", 0xee, 0x0, 0x0),
    cname!(c"Red3", 0xcd, 0x0, 0x0),
    cname!(c"Red4", 0x8b, 0x0, 0x0),
    cname!(c"RosyBrown", 0xbc, 0x8f, 0x8f),
    cname!(c"RosyBrown1", 0xff, 0xc1, 0xc1),
    cname!(c"RosyBrown2", 0xee, 0xb4, 0xb4),
    cname!(c"RosyBrown3", 0xcd, 0x9b, 0x9b),
    cname!(c"RosyBrown4", 0x8b, 0x69, 0x69),
    cname!(c"RoyalBlue", 0x41, 0x69, 0xe1),
    cname!(c"RoyalBlue1", 0x48, 0x76, 0xff),
    cname!(c"RoyalBlue2", 0x43, 0x6e, 0xee),
    cname!(c"RoyalBlue3", 0x3a, 0x5f, 0xcd),
    cname!(c"RoyalBlue4", 0x27, 0x40, 0x8b),
    cname!(c"SaddleBrown", 0x8b, 0x45, 0x13),
    cname!(c"Salmon", 0xfa, 0x80, 0x72),
    cname!(c"Salmon1", 0xff, 0x8c, 0x69),
    cname!(c"Salmon2", 0xee, 0x82, 0x62),
    cname!(c"Salmon3", 0xcd, 0x70, 0x54),
    cname!(c"Salmon4", 0x8b, 0x4c, 0x39),
    cname!(c"SandyBrown", 0xf4, 0xa4, 0x60),
    cname!(c"SeaGreen", 0x2e, 0x8b, 0x57),
    cname!(c"SeaGreen1", 0x54, 0xff, 0x9f),
    cname!(c"SeaGreen2", 0x4e, 0xee, 0x94),
    cname!(c"SeaGreen3", 0x43, 0xcd, 0x80),
    cname!(c"SeaGreen4", 0x2e, 0x8b, 0x57),
    cname!(c"SeaShell", 0xff, 0xf5, 0xee),
    cname!(c"Seashell1", 0xff, 0xf5, 0xee),
    cname!(c"Seashell2", 0xee, 0xe5, 0xde),
    cname!(c"Seashell3", 0xcd, 0xc5, 0xbf),
    cname!(c"Seashell4", 0x8b, 0x86, 0x82),
    cname!(c"Sienna", 0xa0, 0x52, 0x2d),
    cname!(c"Sienna1", 0xff, 0x82, 0x47),
    cname!(c"Sienna2", 0xee, 0x79, 0x42),
    cname!(c"Sienna3", 0xcd, 0x68, 0x39),
    cname!(c"Sienna4", 0x8b, 0x47, 0x26),
    cname!(c"Silver", 0xc0, 0xc0, 0xc0),
    cname!(c"SkyBlue", 0x87, 0xce, 0xeb),
    cname!(c"SkyBlue1", 0x87, 0xce, 0xff),
    cname!(c"SkyBlue2", 0x7e, 0xc0, 0xee),
    cname!(c"SkyBlue3", 0x6c, 0xa6, 0xcd),
    cname!(c"SkyBlue4", 0x4a, 0x70, 0x8b),
    cname!(c"SlateBlue", 0x6a, 0x5a, 0xcd),
    cname!(c"SlateBlue1", 0x83, 0x6f, 0xff),
    cname!(c"SlateBlue2", 0x7a, 0x67, 0xee),
    cname!(c"SlateBlue3", 0x69, 0x59, 0xcd),
    cname!(c"SlateBlue4", 0x47, 0x3c, 0x8b),
    cname!(c"SlateGray", 0x70, 0x80, 0x90),
    cname!(c"SlateGray1", 0xc6, 0xe2, 0xff),
    cname!(c"SlateGray2", 0xb9, 0xd3, 0xee),
    cname!(c"SlateGray3", 0x9f, 0xb6, 0xcd),
    cname!(c"SlateGray4", 0x6c, 0x7b, 0x8b),
    cname!(c"SlateGrey", 0x70, 0x80, 0x90),
    cname!(c"Snow", 0xff, 0xfa, 0xfa),
    cname!(c"Snow1", 0xff, 0xfa, 0xfa),
    cname!(c"Snow2", 0xee, 0xe9, 0xe9),
    cname!(c"Snow3", 0xcd, 0xc9, 0xc9),
    cname!(c"Snow4", 0x8b, 0x89, 0x89),
    cname!(c"SpringGreen", 0x00, 0xff, 0x7f),
    cname!(c"SpringGreen1", 0x0, 0xff, 0x7f),
    cname!(c"SpringGreen2", 0x0, 0xee, 0x76),
    cname!(c"SpringGreen3", 0x0, 0xcd, 0x66),
    cname!(c"SpringGreen4", 0x0, 0x8b, 0x45),
    cname!(c"SteelBlue", 0x46, 0x82, 0xb4),
    cname!(c"SteelBlue1", 0x63, 0xb8, 0xff),
    cname!(c"SteelBlue2", 0x5c, 0xac, 0xee),
    cname!(c"SteelBlue3", 0x4f, 0x94, 0xcd),
    cname!(c"SteelBlue4", 0x36, 0x64, 0x8b),
    cname!(c"Tan", 0xd2, 0xb4, 0x8c),
    cname!(c"Tan1", 0xff, 0xa5, 0x4f),
    cname!(c"Tan2", 0xee, 0x9a, 0x49),
    cname!(c"Tan3", 0xcd, 0x85, 0x3f),
    cname!(c"Tan4", 0x8b, 0x5a, 0x2b),
    cname!(c"Teal", 0x00, 0x80, 0x80),
    cname!(c"Thistle", 0xd8, 0xbf, 0xd8),
    cname!(c"Thistle1", 0xff, 0xe1, 0xff),
    cname!(c"Thistle2", 0xee, 0xd2, 0xee),
    cname!(c"Thistle3", 0xcd, 0xb5, 0xcd),
    cname!(c"Thistle4", 0x8b, 0x7b, 0x8b),
    cname!(c"Tomato", 0xff, 0x63, 0x47),
    cname!(c"Tomato1", 0xff, 0x63, 0x47),
    cname!(c"Tomato2", 0xee, 0x5c, 0x42),
    cname!(c"Tomato3", 0xcd, 0x4f, 0x39),
    cname!(c"Tomato4", 0x8b, 0x36, 0x26),
    cname!(c"Turquoise", 0x40, 0xe0, 0xd0),
    cname!(c"Turquoise1", 0x0, 0xf5, 0xff),
    cname!(c"Turquoise2", 0x0, 0xe5, 0xee),
    cname!(c"Turquoise3", 0x0, 0xc5, 0xcd),
    cname!(c"Turquoise4", 0x0, 0x86, 0x8b),
    cname!(c"Violet", 0xee, 0x82, 0xee),
    cname!(c"VioletRed", 0xd0, 0x20, 0x90),
    cname!(c"VioletRed1", 0xff, 0x3e, 0x96),
    cname!(c"VioletRed2", 0xee, 0x3a, 0x8c),
    cname!(c"VioletRed3", 0xcd, 0x32, 0x78),
    cname!(c"VioletRed4", 0x8b, 0x22, 0x52),
    cname!(c"WebGray", 0x80, 0x80, 0x80),
    cname!(c"WebGreen", 0x0, 0x80, 0x0),
    cname!(c"WebGrey", 0x80, 0x80, 0x80),
    cname!(c"WebMaroon", 0x80, 0x0, 0x0),
    cname!(c"WebPurple", 0x80, 0x0, 0x80),
    cname!(c"Wheat", 0xf5, 0xde, 0xb3),
    cname!(c"Wheat1", 0xff, 0xe7, 0xba),
    cname!(c"Wheat2", 0xee, 0xd8, 0xae),
    cname!(c"Wheat3", 0xcd, 0xba, 0x96),
    cname!(c"Wheat4", 0x8b, 0x7e, 0x66),
    cname!(c"White", 0xff, 0xff, 0xff),
    cname!(c"WhiteSmoke", 0xf5, 0xf5, 0xf5),
    cname!(c"X11Gray", 0xbe, 0xbe, 0xbe),
    cname!(c"X11Green", 0x0, 0xff, 0x0),
    cname!(c"X11Grey", 0xbe, 0xbe, 0xbe),
    cname!(c"X11Maroon", 0xb0, 0x30, 0x60),
    cname!(c"X11Purple", 0xa0, 0x20, 0xf0),
    cname!(c"Yellow", 0xff, 0xff, 0x00),
    cname!(c"Yellow1", 0xff, 0xff, 0x0),
    cname!(c"Yellow2", 0xee, 0xee, 0x0),
    cname!(c"Yellow3", 0xcd, 0xcd, 0x0),
    cname!(c"Yellow4", 0x8b, 0x8b, 0x0),
    cname!(c"YellowGreen", 0x9a, 0xcd, 0x32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_name_table_is_sorted_and_terminated() {
        for (i, pair) in COLOR_NAME_TABLE.windows(2).enumerate() {
            let a = pair[0].name.to_bytes();
            let b = pair[1].name.to_bytes();
            assert!(
                a.to_ascii_lowercase() < b.to_ascii_lowercase(),
                "Out of order at {i}: {:?} >= {:?}",
                std::str::from_utf8(a),
                std::str::from_utf8(b),
            );
        }
    }

    #[test]
    fn coloridx_to_name_handles_special_indices() {
        let mut hexbuf = [0u8; 8];
        {
            assert!(coloridx_to_name(K_COLOR_IDX_NONE, 0, &mut hexbuf).is_none());
            assert_eq!(
                coloridx_to_name(K_COLOR_IDX_FG, 0, &mut hexbuf),
                Some(c"fg")
            );
            assert_eq!(
                coloridx_to_name(K_COLOR_IDX_BG, 0, &mut hexbuf),
                Some(c"bg")
            );
            assert_eq!(
                coloridx_to_name(K_COLOR_IDX_HEX, 0x12abef, &mut hexbuf),
                Some(c"#12abef")
            );
        }
    }

    #[test]
    fn cmdline_highlight_definitions_link_only_to_previously_defined_groups() {
        use std::collections::HashSet;
        let predefined: HashSet<&str> = [
            "Conceal",
            "Cursor",
            "lCursor",
            "DiffText",
            "ErrorMsg",
            "IncSearch",
            "ModeMsg",
            "NonText",
            "PmenuSbar",
            "StatusLine",
            "StatusLineNC",
            "TabLineFill",
            "TabLineSel",
            "TermCursor",
            "VertSplit",
            "WildMenu",
            "WinSeparator",
            "EndOfBuffer",
            "QuickFixLine",
            "Substitute",
            "Whitespace",
            "Error",
            "Todo",
            "String",
            "Character",
            "Number",
            "Boolean",
            "Float",
            "Function",
            "Conditional",
            "Repeat",
            "Label",
            "Operator",
            "Keyword",
            "Exception",
            "Include",
            "Define",
            "Macro",
            "PreCondit",
            "StorageClass",
            "Structure",
            "Typedef",
            "Tag",
            "SpecialChar",
            "Delimiter",
            "SpecialComment",
            "Debug",
            "Dimmed",
            "ColorColumn",
            "CursorColumn",
            "CursorLine",
            "CursorLineNr",
            "DiffAdd",
            "DiffChange",
            "DiffDelete",
            "Directory",
            "FoldColumn",
            "Folded",
            "LineNr",
            "MatchParen",
            "MoreMsg",
            "Pmenu",
            "PmenuSel",
            "PmenuThumb",
            "Question",
            "Search",
            "SignColumn",
            "SpecialKey",
            "SpellBad",
            "SpellCap",
            "SpellLocal",
            "SpellRare",
            "TabLine",
            "Title",
            "Visual",
            "WarningMsg",
            "Normal",
            "Comment",
            "Constant",
            "Special",
            "Identifier",
            "Statement",
            "PreProc",
            "Type",
            "Underlined",
            "Ignore",
        ]
        .into_iter()
        .collect();

        let mut defined = HashSet::new();
        for &cmd in &HIGHLIGHT_INIT_CMDLINE {
            let s = cmd.to_str().unwrap();
            if let Some(rest) = s.strip_prefix("default link ") {
                let mut parts = rest.split_whitespace();
                let new_grp = parts.next().unwrap();
                let grp_link = parts.next().unwrap();
                assert!(parts.next().is_none());
                assert!(
                    defined.contains(grp_link) || predefined.contains(grp_link),
                    "target group {grp_link} not yet defined for {new_grp}"
                );
                assert!(!defined.contains(new_grp) && !predefined.contains(new_grp));
                assert!(new_grp.starts_with("Nvim"));
                defined.insert(new_grp);
            } else {
                let (new_grp, _args) = s.split_once(' ').unwrap();
                assert!(!defined.contains(new_grp) && !predefined.contains(new_grp));
                assert!(new_grp.starts_with("Nvim"));
                defined.insert(new_grp);
            }
        }
    }
}
