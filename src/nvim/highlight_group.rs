//! Code for managing highlight groups (`src/nvim/highlight_group.c`).

use crate::api::private::helpers::has_key;
use crate::api::private::helpers::{
    api_set_error, arena_dict, boolean_obj, cstr_as_obj, cstr_as_string, dict_obj, put_c,
};
use crate::api::private::validate::{Dict, Error, ErrorType, NvimString, Object, ObjectType};
use crate::api::ui::HLATTRS_DICT_SIZE;
use crate::api::win_config::UIExtension;
use crate::autocmd::{EVENT_COLORSCHEME, EVENT_COLORSCHEMEPRE, apply_autocmds};
use crate::buffer::K_UIMESSAGES;
use crate::change::HLF_W;
use crate::charset::{
    ascii_iswhite, getdigits_int, skiptowhite, skipwhite, vim_isprintc, vim_strsize,
};
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
use crate::memory::Arena;
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
use std::ptr;

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
#[derive(Clone, Copy, Default)]
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

struct HighlightGroupState {
    highlight_ga: Vec<HlGroup>,
    highlight_unames: std::collections::BTreeMap<Vec<u8>, i32>,
    had_both: bool,
    load_colors_recursive: bool,
    highlight_color_name: [u8; 20],
}

impl HighlightGroupState {
    const fn new() -> Self {
        Self {
            highlight_ga: Vec::new(),
            highlight_unames: std::collections::BTreeMap::new(),
            had_both: false,
            load_colors_recursive: false,
            highlight_color_name: [0; 20],
        }
    }
}

static HLG_STATE: GlobalState<HighlightGroupState> = GlobalState::new(HighlightGroupState::new());

#[inline]
fn hlg_st() -> &'static mut HighlightGroupState {
    unsafe { &mut *HLG_STATE.as_ptr() }
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
fn hl_table_at(idx: usize) -> &'static mut HlGroup {
    &mut hlg_st().highlight_ga[idx]
}

#[inline]
fn hl_len() -> i32 {
    hlg_st().highlight_ga.len() as i32
}

#[inline]
const fn rgb(r: RgbValue, g: RgbValue, b: RgbValue) -> RgbValue {
    (r << 16) | (g << 8) | b
}

const HIGHLIGHT_INIT_BOTH: [&CStr; 181] = [
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

pub fn highlight_group_name(id: i32) -> &'static CStr {
    hl_table_at(id as usize).sg_name.as_c_str()
}

pub fn highlight_link_id(id: i32) -> i32 {
    hl_table_at(id as usize).sg_link
}

pub fn syn_init_cmdline_highlight(reset: bool, init: bool) {
    for &cmd in &HIGHLIGHT_INIT_CMDLINE {
        do_highlight(cmd.as_ptr(), reset, init);
    }
}

pub fn init_highlight(both: bool, reset: bool) {
    let p = get_var_value(c"g:colors_name");
    if !p.is_null() {
        let copy_p = crate::memory::cstr_to_cstring(p);
        let okay = load_colors(copy_p.as_ptr().cast_mut());
        if okay {
            return;
        }
    }

    if both {
        hlg_st().had_both = true;
        for &cmd in &HIGHLIGHT_INIT_BOTH {
            do_highlight(cmd.as_ptr(), reset, true);
        }
    } else if !hlg_st().had_both {
        return;
    }

    let pp = if crate::memory::cstr_first_byte(opt_vars().p_bg) == b'l' {
        &HIGHLIGHT_INIT_LIGHT
    } else {
        &HIGHLIGHT_INIT_DARK
    };
    for &cmd in pp {
        do_highlight(cmd.as_ptr(), reset, true);
    }

    syn_init_cmdline_highlight(false, false);
}

pub fn load_colors(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    {
        if hlg_st().load_colors_recursive {
            return true;
        }

        hlg_st().load_colors_recursive = true;
        let name_bytes = crate::memory::cstr_as_bytes(name);
        let mut buf = Vec::with_capacity(name_bytes.len() + 12);
        apply_autocmds(
            EVENT_COLORSCHEMEPRE,
            name,
            curbuf_mut().b_fname,
            false,
            g_st().curbuf,
        );
        buf.extend_from_slice(b"colors/");
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(b".*\0");
        let retval = source_runtime_vim_lua(buf.as_mut_ptr().cast(), DIP_START + DIP_OPT);
        if retval {
            apply_autocmds(
                EVENT_COLORSCHEME,
                name,
                curbuf_mut().b_fname,
                false,
                g_st().curbuf,
            );
        }

        hlg_st().load_colors_recursive = false;

        retval
    }
}

fn lookup_color(idx: i32, foreground: bool, boldp: &mut TriState) -> i32 {
    let mut color = COLOR_NUMBERS_16[idx as usize];

    if color < 0 {
        return -1;
    }

    if g_st().t_colors == 8 {
        color = COLOR_NUMBERS_8[idx as usize];
        if foreground {
            if (color & 8) != 0 {
                *boldp = K_TRUE;
            } else {
                *boldp = K_FALSE;
            }
        }
        color &= 7;
    } else if g_st().t_colors == 16 {
        color = COLOR_NUMBERS_8[idx as usize];
    } else if g_st().t_colors == 88 {
        color = COLOR_NUMBERS_88[idx as usize];
    } else if g_st().t_colors >= 256 {
        color = COLOR_NUMBERS_256[idx as usize];
    }
    color
}

pub fn set_hl_group(id: i32, attrs: HlAttrs, dict: &KeyDict_highlight, link_id: i32) {
    unsafe {
        let idx = (id - 1) as usize;
        let is_default = (attrs.rgb_ae_attr & HL_DEFAULT) != 0;

        if is_default && hl_has_settings(idx as i32, true) && !dict.force {
            return;
        }

        let g = &mut *hl_table_at(idx);
        g.sg_cleared = false;
        let old_link = g.sg_link;

        if link_id > 0 {
            g.sg_link = link_id;
            g.sg_script_ctx = g_st().current_sctx;
            g.sg_script_ctx.sc_lnum += sourcing_lnum();
            nlua_set_sctx(&mut g.sg_script_ctx);
            g.sg_set |= SG_LINK;
            if is_default {
                g.sg_deflink = link_id;
                g.sg_deflink_sctx = g_st().current_sctx;
                g.sg_deflink_sctx.sc_lnum += sourcing_lnum();
                nlua_set_sctx(&mut g.sg_deflink_sctx);
            }
        } else {
            g.sg_link = 0;
        }

        let update = has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_UPDATE) && dict.update;
        g.sg_gui = attrs.rgb_ae_attr & !HL_DEFAULT;

        g.sg_rgb_fg = attrs.rgb_fg_color;
        g.sg_rgb_bg = attrs.rgb_bg_color;
        g.sg_rgb_sp = attrs.rgb_sp_color;

        let cattrs: [(*mut i32, RgbValue, Object); 3] = [
            (
                &raw mut g.sg_rgb_fg_idx,
                g.sg_rgb_fg,
                if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_FG) {
                    dict.fg
                } else {
                    dict.foreground
                },
            ),
            (
                &raw mut g.sg_rgb_bg_idx,
                g.sg_rgb_bg,
                if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_BG) {
                    dict.bg
                } else {
                    dict.background
                },
            ),
            (
                &raw mut g.sg_rgb_sp_idx,
                g.sg_rgb_sp,
                if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_SP) {
                    dict.sp
                } else {
                    dict.special
                },
            ),
        ];

        for (j, &(dest, val, name)) in cattrs.iter().enumerate() {
            if name.obj_type != ObjectType::ObjectTypeNil {
                if val < 0 {
                    *dest = K_COLOR_IDX_NONE;
                } else if name.obj_type == ObjectType::ObjectTypeString
                    && name.data.string().size > 0
                {
                    name_to_color(name.data.string().data, dest);
                } else {
                    *dest = K_COLOR_IDX_HEX;
                }
            } else if !update {
                *dest = K_COLOR_IDX_NONE;
            } else if old_link > 0 && val >= 0 {
                let linked = &*hl_table_at((old_link - 1) as usize);
                let linked_idx = if j == 0 {
                    linked.sg_rgb_fg_idx
                } else if j == 1 {
                    linked.sg_rgb_bg_idx
                } else {
                    linked.sg_rgb_sp_idx
                };
                *dest = if linked_idx != K_COLOR_IDX_NONE {
                    linked_idx
                } else {
                    K_COLOR_IDX_HEX
                };
            }
        }

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
            g.sg_font = hl_get_font(attrs.font).map(ToOwned::to_owned);
        } else if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_FONT) || !update {
            g.sg_font = None;
        }

        g.sg_script_ctx = g_st().current_sctx;
        g.sg_script_ctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&mut g.sg_script_ctx);

        g.sg_attr = hl_get_syn_attr(0, id, attrs);

        if g.sg_name.to_bytes().eq_ignore_ascii_case(b"NORMAL") {
            g_st().cterm_normal_fg_color = g.sg_cterm_fg;
            g_st().cterm_normal_bg_color = g.sg_cterm_bg;
            let did_changed = g_st().normal_bg != g.sg_rgb_bg
                || g_st().normal_fg != g.sg_rgb_fg
                || g_st().normal_sp != g.sg_rgb_sp;
            g_st().normal_fg = g.sg_rgb_fg;
            g_st().normal_bg = g.sg_rgb_bg;
            g_st().normal_sp = g.sg_rgb_sp;

            if did_changed {
                highlight_attr_set_all();
            }
            ui_default_colors_set();
        } else if cursor_mode_uses_syn_id(id) {
            ui_mode_info_set();
        }

        if !g_st().updating_screen {
            redraw_all_later(UPD_NOT_VALID);
        }
        g_st().need_highlight_changed = true;
    }
}

fn set_gui_color(
    idx: i32,
    init: bool,
    arg: impl crate::strings::IntoCCharPtr,
    color: &mut RgbValue,
    color_idx: &mut i32,
) -> bool {
    let arg = arg.into_c_char_ptr();
    {
        if init && (hl_table_at(idx as usize).sg_set & SG_GUI) != 0 {
            return false;
        }
        if !init {
            hl_table_at(idx as usize).sg_set |= SG_GUI;
        }
        let old_color = *color;
        let old_idx = *color_idx;
        if crate::memory::cstr_as_bytes(arg) != b"NONE" {
            *color = name_to_color(arg, color_idx);
        } else {
            *color = -1;
            *color_idx = K_COLOR_IDX_NONE;
        }
        *color != old_color || *color_idx != old_idx
    }
}

pub fn do_highlight(line: impl crate::strings::IntoCCharPtr, forceit: bool, init: bool) {
    let mut line = line.into_c_char_ptr();
    unsafe {
        if !init && ends_excmd(i32::from(crate::memory::cstr_first_byte(line))) != 0 {
            msg_ext_set_kind(c"list_cmd");
            let mut i = 1;
            while i <= hl_len() && !g_st().got_int {
                highlight_list_one(i);
                i += 1;
            }
            return;
        }

        let mut dodefault = false;

        let mut name_end = skiptowhite(line).cast_const();
        let mut linep = skipwhite(name_end).cast_const();

        if crate::memory::cstr_prefix_bytes(line, name_end as usize - line as usize)
            == crate::memory::cstr_prefix_bytes(c"default", name_end as usize - line as usize)
        {
            dodefault = true;
            line = linep;
            name_end = skiptowhite(line).cast_const();
            linep = skipwhite(name_end).cast_const();
        }

        let mut doclear = false;
        let mut dolink = false;

        let word_len = name_end as usize - line as usize;
        if crate::memory::cstr_prefix_bytes(line, word_len)
            == crate::memory::cstr_prefix_bytes(c"clear", word_len)
        {
            doclear = true;
        } else if crate::memory::cstr_prefix_bytes(line, word_len)
            == crate::memory::cstr_prefix_bytes(c"link", word_len)
        {
            dolink = true;
        }

        if !doclear && !dolink && ends_excmd(i32::from(crate::memory::cstr_first_byte(linep))) != 0
        {
            let id = syn_name2id_len(line, word_len);
            if id == 0 {
                semsg!(gettext(E_HIGHLIGHT_GROUP_NAME_NOT_FOUND_STR), line);
            } else {
                msg_ext_set_kind(c"list_cmd");
                highlight_list_one(id);
            }
            return;
        }

        if dolink {
            let from_start = linep;
            let from_end = skiptowhite(from_start).cast_const();
            let to_start = skipwhite(from_end).cast_const();
            let to_end = skiptowhite(to_start).cast_const();

            if ends_excmd(i32::from(crate::memory::cstr_first_byte(from_start))) != 0
                || ends_excmd(i32::from(crate::memory::cstr_first_byte(to_start))) != 0
            {
                semsg!(
                    gettext(c"E412: Not enough arguments: \":highlight link %s\""),
                    from_start,
                );
                return;
            }

            if ends_excmd(i32::from(*skipwhite(to_end) as u8)) == 0 {
                semsg!(
                    gettext(c"E413: Too many arguments: \":highlight link %s\""),
                    from_start,
                );
                return;
            }

            let from_id = syn_check_group(from_start, from_end as usize - from_start as usize);
            let to_id = if crate::memory::cstr_prefix_bytes(to_start, 4) == &b"NONE"[..4] {
                0
            } else {
                syn_check_group(to_start, to_end as usize - to_start as usize)
            };

            let mut hlgroup: *mut HlGroup = ptr::null_mut();
            if from_id > 0 {
                hlgroup = hl_table_at((from_id - 1) as usize);
                if dodefault && (forceit || (*hlgroup).sg_deflink == 0) {
                    (*hlgroup).sg_deflink = to_id;
                    (*hlgroup).sg_deflink_sctx = g_st().current_sctx;
                    (*hlgroup).sg_deflink_sctx.sc_lnum += sourcing_lnum();
                    nlua_set_sctx(&mut (*hlgroup).sg_deflink_sctx);
                }
            }

            if from_id > 0 && (!init || (*hlgroup).sg_set == 0) {
                if to_id > 0 && !forceit && !init && hl_has_settings(from_id - 1, dodefault) {
                    if sourcing_name().is_null() && !dodefault {
                        emsg(gettext(E_GROUP_HAS_SETTINGS_HIGHLIGHT_LINK_IGNORED));
                    }
                } else if (*hlgroup).sg_link != to_id
                    || (*hlgroup).sg_script_ctx.sc_sid != g_st().current_sctx.sc_sid
                    || (*hlgroup).sg_cleared
                {
                    if !init {
                        (*hlgroup).sg_set |= SG_LINK;
                    }
                    (*hlgroup).sg_link = to_id;
                    (*hlgroup).sg_script_ctx = g_st().current_sctx;
                    (*hlgroup).sg_script_ctx.sc_lnum += sourcing_lnum();
                    nlua_set_sctx(&mut (*hlgroup).sg_script_ctx);
                    (*hlgroup).sg_cleared = false;
                    redraw_all_later(UPD_SOME_VALID);
                    g_st().need_highlight_changed = true;
                }
            }

            return;
        }

        if doclear {
            line = linep;
            if ends_excmd(i32::from(crate::memory::cstr_first_byte(line))) != 0 {
                do_unlet(c"g:colors_name", 13, true);
                restore_cterm_colors();

                for j in 0..hl_len() {
                    highlight_clear(j);
                }
                init_highlight(true, true);
                highlight_changed();
                redraw_all_later(UPD_NOT_VALID);
                return;
            }
            name_end = skiptowhite(line);
            linep = skipwhite(name_end);
        }

        let id = syn_check_group(line, name_end as usize - line as usize);
        if id == 0 {
            return;
        }
        let idx = id - 1;

        if dodefault && hl_has_settings(idx, true) {
            return;
        }

        let item_before = (*hl_table_at(idx as usize)).clone();
        let is_normal_group = hl_table_at(idx as usize)
            .sg_name
            .to_bytes()
            .eq_ignore_ascii_case(b"NORMAL");

        if doclear || (forceit && init) {
            highlight_clear(idx);
            if !doclear {
                hl_table_at(idx as usize).sg_set = 0;
            }
        }

        let mut did_change = false;
        let mut error = false;

        let mut key = [0u8; 64];
        let mut arg = [0u8; 512];
        if !doclear {
            while ends_excmd(i32::from(crate::memory::cstr_first_byte(linep))) == 0 {
                let key_start = linep;
                if crate::memory::cstr_first_byte(linep) == b'=' {
                    semsg!(gettext(E_UNEXPECTED_EQUAL_SIGN_STR), key_start);
                    error = true;
                    break;
                }

                while *linep != 0
                    && !ascii_iswhite(crate::memory::cstr_first_byte(linep))
                    && crate::memory::cstr_first_byte(linep) != b'='
                {
                    linep = linep.wrapping_add(1);
                }
                let key_len = linep as usize - key_start as usize;
                if key_len > key.len() - 1 {
                    emsg(gettext(c"E423: Illegal argument"));
                    error = true;
                    break;
                }
                for (i, dst) in key.iter_mut().take(key_len).enumerate() {
                    *dst = (crate::memory::cstr_byte_at(key_start, i)).to_ascii_uppercase();
                }
                key[key_len] = 0;
                linep = skipwhite(linep);

                let key_bytes = &key[..key_len];
                if key_bytes == b"NONE" {
                    if !init || hl_table_at(idx as usize).sg_set == 0 {
                        if !init {
                            hl_table_at(idx as usize).sg_set |= SG_CTERM + SG_GUI;
                        }
                        highlight_clear(idx);
                    }
                    continue;
                }

                if crate::memory::cstr_first_byte(linep) != b'=' {
                    semsg!(gettext(E_MISSING_EQUAL_SIGN_STR_2), key_start);
                    error = true;
                    break;
                }
                linep = linep.wrapping_add(1);

                linep = skipwhite(linep);
                let arg_start;
                if crate::memory::cstr_first_byte(linep) == b'\'' {
                    linep = linep.wrapping_add(1);
                    arg_start = linep;
                    linep = crate::memory::cstr_as_bytes(linep)
                        .iter()
                        .position(|&b| b == b'\'')
                        .map_or(ptr::null(), |i| linep.wrapping_add(i));
                    if linep.is_null() {
                        semsg!(gettext(E_INVARG2), key_start);
                        error = true;
                        break;
                    }
                } else {
                    arg_start = linep;
                    linep = skiptowhite(linep);
                }
                if ptr::eq(linep, arg_start) {
                    semsg!(gettext(E_MISSING_ARGUMENT_STR), key_start);
                    error = true;
                    break;
                }
                let arg_len = linep as usize - arg_start as usize;
                if arg_len > arg.len() - 1 {
                    emsg(gettext(c"E423: Illegal argument"));
                    error = true;
                    break;
                }
                ptr::copy_nonoverlapping(arg_start.cast::<u8>(), arg.as_mut_ptr(), arg_len);
                arg[arg_len] = 0;

                if crate::memory::cstr_first_byte(linep) == b'\'' {
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
                            if mb_strnicmp(arg[off..].as_ptr().cast(), hname.as_ptr(), len) == 0 {
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
                            semsg!(gettext(c"E418: Illegal value: %s"), arg.as_ptr());
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
                        if !init || (hl_table_at(idx as usize).sg_set & SG_CTERM) == 0 {
                            if !init {
                                hl_table_at(idx as usize).sg_set |= SG_CTERM;
                            }
                            hl_table_at(idx as usize).sg_cterm = attr;
                            hl_table_at(idx as usize).sg_cterm_bold = false;
                        }
                    } else if key_bytes[0] == b'G'
                        && (!init || (hl_table_at(idx as usize).sg_set & SG_GUI) == 0)
                    {
                        if !init {
                            hl_table_at(idx as usize).sg_set |= SG_GUI;
                        }
                        hl_table_at(idx as usize).sg_gui = attr;
                    }
                } else if key_bytes == b"FONT"
                    && (!init || (hl_table_at(idx as usize).sg_set & SG_GUI) == 0)
                {
                    if !init {
                        hl_table_at(idx as usize).sg_set |= SG_GUI;
                    }
                    hl_table_at(idx as usize).sg_font = None;
                    if &arg[..arg_len] != b"NONE" {
                        hl_table_at(idx as usize).sg_font =
                            Some(crate::memory::cstr_to_cstring(arg.as_ptr().cast()));
                    }
                } else if key_bytes == b"CTERMFG" || key_bytes == b"CTERMBG" {
                    if !init || (hl_table_at(idx as usize).sg_set & SG_CTERM) == 0 {
                        if !init {
                            hl_table_at(idx as usize).sg_set |= SG_CTERM;
                        }

                        if key_bytes[5] == b'F' && hl_table_at(idx as usize).sg_cterm_bold {
                            hl_table_at(idx as usize).sg_cterm &= !HL_BOLD;
                            hl_table_at(idx as usize).sg_cterm_bold = false;
                        }

                        let color: i32;
                        if arg[0].is_ascii_digit() {
                            color = crate::charset::parse_i64_prefix(&arg[..arg_len]) as i32;
                        } else if mb_stricmp(arg.as_ptr().cast(), c"fg") == 0 {
                            if g_st().cterm_normal_fg_color != 0 {
                                color = g_st().cterm_normal_fg_color - 1;
                            } else {
                                emsg(gettext(c"E419: FG color unknown"));
                                error = true;
                                break;
                            }
                        } else if mb_stricmp(arg.as_ptr().cast(), c"bg") == 0 {
                            if g_st().cterm_normal_bg_color > 0 {
                                color = g_st().cterm_normal_bg_color - 1;
                            } else {
                                emsg(gettext(c"E420: BG color unknown"));
                                error = true;
                                break;
                            }
                        } else {
                            let off = arg[0].to_ascii_uppercase();
                            let mut i = COLOR_NAMES.len() as isize - 1;
                            while i >= 0 {
                                let cname = COLOR_NAMES[i as usize].as_ptr();
                                if off == crate::memory::cstr_first_byte(cname)
                                    && mb_stricmp(
                                        arg[1..].as_ptr().cast(),
                                        cname.wrapping_add(1),
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

                            if bold == K_TRUE {
                                hl_table_at(idx as usize).sg_cterm |= HL_BOLD;
                                hl_table_at(idx as usize).sg_cterm_bold = true;
                            } else if bold == K_FALSE {
                                hl_table_at(idx as usize).sg_cterm &= !HL_BOLD;
                            }
                        }

                        if key_bytes[5] == b'F' {
                            hl_table_at(idx as usize).sg_cterm_fg = color + 1;
                            if is_normal_group {
                                g_st().cterm_normal_fg_color = color + 1;
                            }
                        } else {
                            hl_table_at(idx as usize).sg_cterm_bg = color + 1;
                            if is_normal_group {
                                g_st().cterm_normal_bg_color = color + 1;
                                if !ui_rgb_attached() && color >= 0 {
                                    let dark = if g_st().t_colors < 16 {
                                        i32::from(color == 0 || color == 4)
                                    } else if color < 16 {
                                        i32::from(color < 7 || color == 8)
                                    } else {
                                        -1
                                    };
                                    if dark != -1
                                        && (dark != 0)
                                            != (crate::memory::cstr_first_byte(opt_vars().p_bg)
                                                == b'd')
                                        && !option_was_set(K_OPT_BACKGROUND)
                                    {
                                        set_option_value_give_err(
                                            K_OPT_BACKGROUND,
                                            cstr_as_obj(if dark != 0 {
                                                c"dark".as_ptr()
                                            } else {
                                                c"light".as_ptr()
                                            }),
                                            0,
                                        );
                                        reset_option_was_set(K_OPT_BACKGROUND);
                                    }
                                }
                            }
                        }
                    }
                } else if key_bytes == b"GUIFG" {
                    let g = hl_table_at(idx as usize);
                    did_change = set_gui_color(
                        idx,
                        init,
                        arg.as_ptr().cast(),
                        &mut g.sg_rgb_fg,
                        &mut g.sg_rgb_fg_idx,
                    );
                    if is_normal_group {
                        g_st().normal_fg = hl_table_at(idx as usize).sg_rgb_fg;
                    }
                } else if key_bytes == b"GUIBG" {
                    let g = hl_table_at(idx as usize);
                    did_change = set_gui_color(
                        idx,
                        init,
                        arg.as_ptr().cast(),
                        &mut g.sg_rgb_bg,
                        &mut g.sg_rgb_bg_idx,
                    );
                    if is_normal_group {
                        g_st().normal_bg = hl_table_at(idx as usize).sg_rgb_bg;
                    }
                } else if key_bytes == b"GUISP" {
                    let g = hl_table_at(idx as usize);
                    did_change = set_gui_color(
                        idx,
                        init,
                        arg.as_ptr().cast(),
                        &mut g.sg_rgb_sp,
                        &mut g.sg_rgb_sp_idx,
                    );
                    if is_normal_group {
                        g_st().normal_sp = hl_table_at(idx as usize).sg_rgb_sp;
                    }
                } else if key_bytes == b"START" || key_bytes == b"STOP" {
                    // Ignored for now
                } else if key_bytes == b"BLEND" {
                    if &arg[..arg_len] != b"NONE" {
                        let mut arg_end = arg.as_mut_ptr().cast();
                        hl_table_at(idx as usize).sg_blend =
                            getdigits_int(&raw mut arg_end, false, 0);
                    } else {
                        hl_table_at(idx as usize).sg_blend = -1;
                    }
                } else {
                    semsg!(gettext(c"E423: Illegal argument: %s"), key_start);
                    error = true;
                    break;
                }
                hl_table_at(idx as usize).sg_cleared = false;

                if !init || (hl_table_at(idx as usize).sg_set & SG_LINK) == 0 {
                    hl_table_at(idx as usize).sg_link = 0;
                }

                linep = skipwhite(linep);
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
        let g = &mut *hl_table_at(idx as usize);
        g.sg_script_ctx = g_st().current_sctx;
        g.sg_script_ctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&mut g.sg_script_ctx);

        if (did_change || *hl_table_at(idx as usize) != item_before) && !did_highlight_changed {
            if !g_st().updating_screen {
                redraw_all_later(UPD_NOT_VALID);
            }
            g_st().need_highlight_changed = true;
        }
    }
}

pub fn free_highlight() {
    hlg_st().highlight_ga = Vec::new();
    hlg_st().highlight_unames = std::collections::BTreeMap::new();
}

pub fn restore_cterm_colors() {
    g_st().normal_fg = -1;
    g_st().normal_bg = -1;
    g_st().normal_sp = -1;
    g_st().cterm_normal_fg_color = 0;
    g_st().cterm_normal_bg_color = 0;
}

fn hl_has_settings(idx: i32, check_link: bool) -> bool {
    let g = &*hl_table_at(idx as usize);
    !g.sg_cleared
        && (g.sg_attr != 0
            || g.sg_cterm_fg != 0
            || g.sg_cterm_bg != 0
            || g.sg_rgb_fg_idx != K_COLOR_IDX_NONE
            || g.sg_rgb_bg_idx != K_COLOR_IDX_NONE
            || g.sg_rgb_sp_idx != K_COLOR_IDX_NONE
            || (check_link && (g.sg_set & SG_LINK) != 0))
}

fn highlight_clear(idx: i32) {
    let g = &mut *hl_table_at(idx as usize);
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
}

fn highlight_list_one(id: i32) {
    let sgp = &*hl_table_at((id - 1) as usize);
    let mut didh = false;

    if message_filtered(sgp.sg_name.as_ptr()) {
        return;
    }

    if sgp.sg_parent != 0 && sgp.sg_cleared {
        return;
    }

    didh = highlight_list_arg(id, didh, LIST_ATTR, sgp.sg_cterm, ptr::null(), c"cterm");
    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_cterm_fg, ptr::null(), c"ctermfg");
    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_cterm_bg, ptr::null(), c"ctermbg");

    didh = highlight_list_arg(id, didh, LIST_ATTR, sgp.sg_gui, ptr::null(), c"gui");
    let mut hexbuf = [0u8; 8];
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_fg_idx, sgp.sg_rgb_fg, &mut hexbuf)
            .map_or(ptr::null(), CStr::as_ptr),
        c"guifg",
    );
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_bg_idx, sgp.sg_rgb_bg, &mut hexbuf)
            .map_or(ptr::null(), CStr::as_ptr),
        c"guibg",
    );
    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        coloridx_to_name(sgp.sg_rgb_sp_idx, sgp.sg_rgb_sp, &mut hexbuf)
            .map_or(ptr::null(), CStr::as_ptr),
        c"guisp",
    );

    didh = highlight_list_arg(id, didh, LIST_INT, sgp.sg_blend + 1, ptr::null(), c"blend");

    didh = highlight_list_arg(
        id,
        didh,
        LIST_STRING,
        0,
        sgp.sg_font.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        c"font",
    );

    if sgp.sg_link != 0 && !g_st().got_int {
        syn_list_header(didh, 0, id, true);
        didh = true;
        msg_puts_hl(c"links to", HLF_D, false);
        msg_putchar(i32::from(b' '));
        let link_id = hl_table_at((id - 1) as usize).sg_link;
        msg_outtrans(
            hl_table_at((link_id - 1) as usize).sg_name.as_ptr(),
            0,
            false,
        );
    }

    if !didh {
        highlight_list_arg(id, didh, LIST_STRING, 0, c"cleared", c"");
    }
    if opt_vars().p_verbose > 0 {
        last_set_msg(sgp.sg_script_ctx);
    }
}

fn hlgroup2dict(hl: &mut Dict, ns_id: NS, hl_id: i32, arena: &mut Arena) -> bool {
    let sgp = &*hl_table_at((hl_id - 1) as usize);
    let mut ns = ns_id;
    let link = if ns_id == 0 {
        sgp.sg_link
    } else {
        ns_get_hl(&mut ns, hl_id, true, sgp.sg_set != 0)
    };
    if link == -1 {
        return false;
    }
    if ns_id == 0 && sgp.sg_cleared && sgp.sg_set == 0 {
        return false;
    }
    ns = ns_id;
    let attr_id = if ns_id == 0 {
        sgp.sg_attr
    } else {
        ns_get_hl(&mut ns, hl_id, false, sgp.sg_set != 0)
    };
    let attr = syn_attr2entry(attr_id);
    *hl = arena_dict(arena, HLATTRS_DICT_SIZE + 1);
    if (attr.rgb_ae_attr & HL_DEFAULT) != 0 {
        put_c(hl, c"default", boolean_obj(true));
    }
    if link > 0 {
        assert!(1 <= link && link <= hl_len());
        put_c(
            hl,
            c"link",
            cstr_as_obj(hl_table_at((link - 1) as usize).sg_name.as_ptr()),
        );
    }
    let mut hl_cterm = arena_dict(arena, HLATTRS_DICT_SIZE);
    hlattrs2dict(hl, None, attr, true, true);
    hlattrs2dict(hl, Some(&mut hl_cterm), attr, false, true);
    if hl_cterm.size > 0 {
        put_c(hl, c"cterm", dict_obj(hl_cterm));
    }
    true
}

pub fn ns_get_hl_defs(
    ns_id: NS,
    opts: &mut KeyDict_get_highlight,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
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
        id = if create {
            syn_check_group(opts.name.data, opts.name.size)
        } else {
            syn_name2id_len(opts.name.data, opts.name.size)
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
        hlgroup2dict(&mut attrs, ns_id, target_id, arena);
        return attrs;
    }
    if err.err_type != ErrorType::ErrorTypeNone {
        return Dict::new();
    }

    let len = hl_len();
    let mut rv = arena_dict(arena, len as usize);
    for i in 1..=len {
        let mut attrs = Dict::new();
        if !hlgroup2dict(&mut attrs, ns_id, i, arena) {
            continue;
        }
        let target_id = if link { i } else { syn_get_final_id(i) };
        put_c(
            &mut rv,
            hl_table_at((target_id - 1) as usize).sg_name.as_ptr(),
            dict_obj(attrs),
        );
    }

    rv
}

fn highlight_list_arg(
    id: i32,
    mut didh: bool,
    type_: i32,
    mut iarg: i32,
    sarg: impl crate::strings::IntoCCharPtr,
    name: &std::ffi::CStr,
) -> bool {
    let sarg = sarg.into_c_char_ptr();
    {
        if g_st().got_int {
            return false;
        }

        if if type_ == LIST_STRING {
            sarg.is_null()
        } else {
            iarg == 0
        } {
            return didh;
        }

        let mut buf = [0u8; 100];
        let mut ts = buf.as_ptr().cast();
        if type_ == LIST_INT {
            write_dec(&mut buf, iarg - 1);
        } else if type_ == LIST_STRING {
            ts = sarg;
        } else {
            buf[0] = 0;
            let mut i = 0;
            while HL_ATTR_TABLE[i] != 0 {
                let flag = HL_ATTR_TABLE[i];
                if ((flag & HL_UNDERLINE_MASK) != 0 && ((iarg & HL_UNDERLINE_MASK) == flag))
                    || ((flag & HL_UNDERLINE_MASK) == 0 && (iarg & flag) != 0)
                {
                    let dst: *mut _ = buf.as_mut_ptr().cast();
                    if buf[0] != 0 {
                        let dlen = crate::memory::cstr_len(dst);
                        crate::memory::xstrlcpy(dst.wrapping_add(dlen), c",", 100 - dlen);
                    }
                    let dlen = crate::memory::cstr_len(dst);
                    crate::memory::xstrlcpy(
                        dst.wrapping_add(dlen),
                        HL_NAME_TABLE[i].as_ptr(),
                        100 - dlen,
                    );
                    if (flag & HL_UNDERLINE_MASK) == 0 {
                        iarg &= !flag;
                    }
                }
                i += 1;
            }
        }

        syn_list_header(
            didh,
            vim_strsize(ts) + name.to_bytes().len() as i32 + 1,
            id,
            false,
        );
        didh = true;
        if !g_st().got_int {
            if !crate::memory::cstr_is_empty(name.as_ptr()) {
                msg_puts_hl(name.as_ptr(), HLF_D, false);
                msg_puts_hl(c"=", HLF_D, false);
            }
            msg_outtrans(ts, 0, false);
        }
        didh
    }
}

pub fn highlight_has_attr(id: i32, flag: i32, modec: i32) -> Option<&'static CStr> {
    if id <= 0 || id > hl_len() {
        return None;
    }

    let attr = if modec == i32::from(b'g') {
        hl_table_at((id - 1) as usize).sg_gui
    } else {
        hl_table_at((id - 1) as usize).sg_cterm
    };

    if (flag & HL_UNDERLINE_MASK) != 0 {
        let ul = attr & HL_UNDERLINE_MASK;
        (ul == flag).then_some(c"1")
    } else {
        ((attr & flag) != 0).then_some(c"1")
    }
}

pub fn highlight_color(
    id: i32,
    what: impl crate::strings::IntoCCharPtr,
    modec: i32,
) -> Option<&'static CStr> {
    let what = what.into_c_char_ptr();
    {
        let mut fg = false;
        let mut sp = false;
        let mut font = false;

        if id <= 0 || id > hl_len() {
            return None;
        }

        let w0 = (crate::memory::cstr_first_byte(what)).to_ascii_lowercase();
        let w1 = if w0 != 0 {
            (crate::memory::cstr_byte_at(what, 1)).to_ascii_lowercase()
        } else {
            0
        };
        if w0 == b'f' && w1 == b'g' {
            fg = true;
        } else if w0 == b'f'
            && w1 == b'o'
            && (crate::memory::cstr_byte_at(what, 2)).eq_ignore_ascii_case(&b'n')
            && (crate::memory::cstr_byte_at(what, 3)).eq_ignore_ascii_case(&b't')
        {
            font = true;
        } else if w0 == b's' && w1 == b'p' {
            sp = true;
        } else if !(w0 == b'b' && w1 == b'g') {
            return None;
        }

        let g = &*hl_table_at((id - 1) as usize);

        if modec == i32::from(b'g') {
            if crate::memory::cstr_byte_at(what, 2) == b'#' && ui_rgb_attached() {
                let n = if fg {
                    g.sg_rgb_fg
                } else if sp {
                    g.sg_rgb_sp
                } else {
                    g.sg_rgb_bg
                };
                if !(0..=0xffffff).contains(&n) {
                    return None;
                }
                let hex_arr: &mut [u8; 8] =
                    (&mut hlg_st().highlight_color_name[..8]).try_into().unwrap();
                write_hex6(hex_arr, n);
                return CStr::from_bytes_until_nul(&hlg_st().highlight_color_name).ok();
            }
            let hex_arr: &mut [u8; 8] =
                (&mut hlg_st().highlight_color_name[..8]).try_into().unwrap();
            if fg {
                return coloridx_to_name(g.sg_rgb_fg_idx, g.sg_rgb_fg, hex_arr);
            } else if sp {
                return coloridx_to_name(g.sg_rgb_sp_idx, g.sg_rgb_sp, hex_arr);
            } else {
                return coloridx_to_name(g.sg_rgb_bg_idx, g.sg_rgb_bg, hex_arr);
            }
        }
        if font || sp {
            return None;
        }
        if modec == i32::from(b'c') {
            let n = if fg {
                g.sg_cterm_fg - 1
            } else {
                g.sg_cterm_bg - 1
            };
            if n < 0 {
                return None;
            }
            write_dec(&mut hlg_st().highlight_color_name, n);
            return CStr::from_bytes_until_nul(&hlg_st().highlight_color_name).ok();
        }
        None
    }
}

pub fn syn_list_header(did_header: bool, outlen: i32, id: i32, force_newline: bool) -> bool {
    let mut endcol: i32 = 19;
    let mut newline = true;
    let mut name_col: i32 = 0;
    let mut adjust = true;

    if !did_header {
        if !ui_has(K_UIMESSAGES) || g_st().msg_col > 0 {
            msg_putchar(i32::from(b'\n'));
        }
        if g_st().got_int {
            return true;
        }
        name_col = msg_outtrans(hl_table_at((id - 1) as usize).sg_name.as_ptr(), 0, false);
        g_st().msg_col = name_col;
        endcol = 15;
    } else if (ui_has(K_UIMESSAGES) || g_st().msg_silent != 0) && !force_newline {
        msg_putchar(i32::from(b' '));
        adjust = false;
    } else if g_st().msg_col + outlen + 1 >= g_st().columns_global || force_newline {
        msg_putchar(i32::from(b'\n'));
        if g_st().got_int {
            return true;
        }
    } else if g_st().msg_col >= endcol {
        newline = false;
    }

    if adjust {
        if g_st().msg_col >= endcol {
            endcol = g_st().msg_col + 1;
        }
        msg_advance(endcol);
    }

    if !did_header {
        if endcol == g_st().columns_global - 1 && endcol <= name_col {
            msg_putchar(i32::from(b' '));
        }
        msg_puts_hl(c"xxx", id, false);
        msg_putchar(i32::from(b' '));
    }

    newline
}

fn set_hl_attr(idx: i32) {
    let mut at_en = HLATTRS_INIT;
    let sgp = &mut *hl_table_at(idx as usize);

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
    if let Some(ref font) = sgp.sg_font {
        at_en.font = hl_add_font_idx(Some(font.as_bytes()));
    }

    sgp.sg_attr = hl_get_syn_attr(0, idx + 1, at_en);

    if cursor_mode_uses_syn_id(idx + 1) {
        ui_mode_info_set();
    }
}

pub fn syn_name2id(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(name) == b'@' {
            return syn_check_group(name, crate::memory::cstr_len(name));
        }
        syn_name2id_len(name, crate::memory::cstr_len(name))
    }
}

pub fn syn_name2id_len(name: impl crate::strings::IntoCCharPtr, len: usize) -> i32 {
    let name = name.into_c_char_ptr();
    {
        if len == 0 || len > MAX_SYN_NAME {
            return 0;
        }

        let mut name_u = [0u8; MAX_SYN_NAME + 1];
        for (i, dst) in name_u.iter_mut().take(len).enumerate() {
            *dst = (crate::memory::cstr_byte_at(name, i)).to_ascii_uppercase();
        }
        name_u[len] = 0;

        hlg_st()
            .highlight_unames
            .get(&name_u[..len])
            .copied()
            .unwrap_or(0)
    }
}

pub fn syn_name2attr(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    let id = syn_name2id(name);
    if id != 0 {
        return syn_id2attr(id);
    }
    0
}

pub fn highlight_exists(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    i32::from(syn_name2id(name) > 0)
}

pub fn syn_id2name(id: i32) -> &'static CStr {
    if id <= 0 || id > hl_len() {
        return c"";
    }
    hl_table_at((id - 1) as usize).sg_name.as_c_str()
}

pub fn syn_check_group<S: crate::strings::IntoCCharPtr>(name: S, len: usize) -> i32 {
    let name = name.into_c_char_ptr();
    if len > MAX_SYN_NAME {
        emsg(gettext(E_HIGHLIGHT_GROUP_NAME_TOO_LONG));
        return 0;
    }
    let id = syn_name2id_len(name, len);
    if id == 0 {
        return syn_add_group(name, len);
    }
    id
}

fn syn_add_group(name: impl crate::strings::IntoCCharPtr, len: usize) -> i32 {
    let name = name.into_c_char_ptr();
    {
        for i in 0..len {
            let c = crate::memory::cstr_byte_at(name, i);
            if !vim_isprintc(i32::from(c)) {
                emsg(gettext(c"E669: Unprintable character in group name"));
                return 0;
            } else if !c.is_ascii_alphanumeric() && c != b'_' && c != b'.' && c != b'@' && c != b'-'
            {
                msg_source(HLF_W);
                emsg(gettext(E_HIGHLIGHT_GROUP_NAME_INVALID_CHAR));
                return 0;
            }
        }

        let mut scoped_parent: i32 = 0;
        if len > 1 && crate::memory::cstr_first_byte(name) == b'@' {
            let slice = unsafe { std::slice::from_raw_parts(name.cast::<u8>(), len) };
            if let Some(delim_idx) = slice.iter().rposition(|&b| b == b'.') {
                scoped_parent = syn_check_group(name, delim_idx);
            }
        }

        if hlg_st().highlight_ga.capacity() == 0 {
            hlg_st().highlight_ga.reserve(300);
        }

        if hl_len() >= MAX_HL_ID {
            emsg(gettext(c"E849: Too many highlight and syntax groups"));
            return 0;
        }

        let name_slice = crate::memory::cstr_slice(name, len);
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
        hlg_st().highlight_ga.push(hlg);

        let id = hl_len();
        hlg_st().highlight_unames.insert(name_u_bytes, id);

        id
    }
}

pub fn syn_id2attr(hl_id: i32) -> i32 {
    let mut optional = false;
    syn_ns_id2attr(-1, hl_id, &mut optional)
}

pub fn syn_ns_id2attr(mut ns_id: i32, mut hl_id: i32, optional: &mut bool) -> i32 {
    if syn_ns_get_final_id(&mut ns_id, &mut hl_id) {
        *optional = false;
    }
    let sgp = &*hl_table_at((hl_id - 1) as usize);

    let attr = ns_get_hl(&mut ns_id, hl_id, false, sgp.sg_set != 0);

    if attr >= 0 || (*optional && ns_id > 0) {
        return attr;
    }
    sgp.sg_attr
}

pub fn syn_get_final_id(mut hl_id: i32) -> i32 {
    let mut ns_id = curwin_mut().w_ns_hl_active;
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
        let sgp = &*hl_table_at((hl_id - 1) as usize);

        let check = ns_get_hl(ns_id, hl_id, true, sgp.sg_set != 0);
        if check == 0 {
            *hl_idp = hl_id;
            return true;
        } else if check > 0 {
            used = true;
            hl_id = check;
            continue;
        }

        if sgp.sg_link > 0 && sgp.sg_link <= hl_len() {
            hl_id = sgp.sg_link;
        } else if sgp.sg_cleared && sgp.sg_parent > 0 {
            hl_id = sgp.sg_parent;
        } else {
            break;
        }
    }

    *hl_idp = hl_id;
    used
}

pub fn highlight_attr_set_all() {
    for idx in 0..hl_len() {
        let sgp = &mut *hl_table_at(idx as usize);
        if sgp.sg_rgb_bg_idx == K_COLOR_IDX_FG {
            sgp.sg_rgb_bg = g_st().normal_fg;
        } else if sgp.sg_rgb_bg_idx == K_COLOR_IDX_BG {
            sgp.sg_rgb_bg = g_st().normal_bg;
        }
        if sgp.sg_rgb_fg_idx == K_COLOR_IDX_FG {
            sgp.sg_rgb_fg = g_st().normal_fg;
        } else if sgp.sg_rgb_fg_idx == K_COLOR_IDX_BG {
            sgp.sg_rgb_fg = g_st().normal_bg;
        }
        if sgp.sg_rgb_sp_idx == K_COLOR_IDX_FG {
            sgp.sg_rgb_sp = g_st().normal_fg;
        } else if sgp.sg_rgb_sp_idx == K_COLOR_IDX_BG {
            sgp.sg_rgb_sp = g_st().normal_bg;
        }
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
    table: *mut i32,
) {
    let dst = (hlcnt + i) as usize;
    let u_idx = (id - 1) as usize;
    let s_idx = (id_s - 1) as usize;

    if id_alt == 0 {
        *hl_table_at(dst) = HlGroup::new();
        hl_table_at(dst).sg_cterm = g_st().highlight_attr[hlf];
        hl_table_at(dst).sg_gui = g_st().highlight_attr[hlf];
    } else {
        *hl_table_at(dst) = hl_table_at((id_alt - 1) as usize).clone();
    }
    hl_table_at(dst).sg_link = 0;

    hl_table_at(dst).sg_cterm ^= hl_table_at(u_idx).sg_cterm ^ hl_table_at(s_idx).sg_cterm;
    if hl_table_at(u_idx).sg_cterm_fg != hl_table_at(s_idx).sg_cterm_fg {
        hl_table_at(dst).sg_cterm_fg = hl_table_at(u_idx).sg_cterm_fg;
    }
    if hl_table_at(u_idx).sg_cterm_bg != hl_table_at(s_idx).sg_cterm_bg {
        hl_table_at(dst).sg_cterm_bg = hl_table_at(u_idx).sg_cterm_bg;
    }
    hl_table_at(dst).sg_gui ^= hl_table_at(u_idx).sg_gui ^ hl_table_at(s_idx).sg_gui;
    if hl_table_at(u_idx).sg_rgb_fg != hl_table_at(s_idx).sg_rgb_fg {
        hl_table_at(dst).sg_rgb_fg = hl_table_at(u_idx).sg_rgb_fg;
    }
    if hl_table_at(u_idx).sg_rgb_bg != hl_table_at(s_idx).sg_rgb_bg {
        hl_table_at(dst).sg_rgb_bg = hl_table_at(u_idx).sg_rgb_bg;
    }
    if hl_table_at(u_idx).sg_rgb_sp != hl_table_at(s_idx).sg_rgb_sp {
        hl_table_at(dst).sg_rgb_sp = hl_table_at(u_idx).sg_rgb_sp;
    }
    set_hl_attr(hlcnt + i);
    unsafe {
        *table.wrapping_add(i as usize) = syn_id2attr(hlcnt + i + 1);
    }
}

pub fn highlight_changed() {
    let mut userhl = [0u8; 30];
    let mut id_s: i32 = -1;
    let mut id_snc: i32 = 0;

    g_st().need_highlight_changed = false;

    g_st().highlight_attr[HLF_NONE] = 0;

    for (hlf, name_opt) in HLF_NAMES.iter().enumerate().take(HLF_COUNT).skip(1) {
        let name_cstr = name_opt.unwrap();
        let id = syn_check_group(name_cstr.as_ptr(), name_cstr.to_bytes().len());
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

        g_st().highlight_attr[hlf] =
            hl_get_ui_attr(ns_id, hlf as i32, final_id, hlf == HLF_INACTIVE);

        if g_st().highlight_attr[hlf] != g_st().highlight_attr_last[hlf] {
            if hlf == HLF_MSG {
                g_st().clear_cmdline = true;
                let attrs = syn_attr2entry(g_st().highlight_attr[hlf]);
                g_st().msg_grid.blending = attrs.hl_blend > -1;
            }
            ui_call_hl_group_set(
                cstr_as_string(name_cstr.as_ptr()),
                Integer::from(g_st().highlight_attr[hlf]),
            );
            g_st().highlight_attr_last[hlf] = g_st().highlight_attr[hlf];
        }
    }

    let hlcnt = hl_len();
    hlg_st()
        .highlight_ga
        .resize((hlcnt + 10) as usize, HlGroup::new());
    if id_s == -1 {
        *hl_table_at((hlcnt + 9) as usize) = HlGroup::new();
        id_s = hlcnt + 10;
    }
    for i in 0..9 {
        userhl[0] = b'U';
        userhl[1] = b's';
        userhl[2] = b'e';
        userhl[3] = b'r';
        userhl[4] = b'1' + i as u8;
        userhl[5] = 0;
        let id = syn_name2id(userhl.as_ptr().cast());
        if id == 0 {
            g_st().highlight_user[i] = 0;
            g_st().highlight_stlnc[i] = 0;
        } else {
            g_st().highlight_user[i] = syn_id2attr(id);
            combine_stl_hlt(
                id,
                id_s,
                id_snc,
                hlcnt,
                i as i32,
                HLF_SNC,
                (&raw mut g_st().highlight_stlnc).cast::<i32>(),
            );
        }
    }
    hlg_st().highlight_ga.truncate(hlcnt as usize);

    decor_provider_invalidate_hl();
}

pub fn set_context_in_highlight_cmd(xp: &mut ExpandCtx, arg: impl crate::strings::IntoCCharPtr) {
    let mut arg = arg.into_c_char_ptr();
    {
        xp.xp_context = EXPAND_HIGHLIGHT;
        xp.xp_pattern = arg.cast_mut();
        g_st().include_link = 2;
        g_st().include_default = 1;

        if crate::memory::cstr_is_empty(arg) {
            return;
        }

        let mut p = skiptowhite(arg).cast_const();
        if crate::memory::cstr_is_empty(p) {
            return;
        }

        g_st().include_default = 0;
        if crate::memory::cstr_prefix_bytes(c"default", p as usize - arg as usize)
            == crate::memory::cstr_prefix_bytes(arg, p as usize - arg as usize)
        {
            arg = skipwhite(p);
            xp.xp_pattern = arg.cast_mut();
            p = skiptowhite(arg).cast_const();
        }
        if crate::memory::cstr_is_empty(p) {
            return;
        }

        g_st().include_link = 0;
        if crate::memory::cstr_first_byte(arg) == b'N'
            && crate::memory::cstr_byte_at(arg, 1) == b'i'
        {
            highlight_list();
        }
        let len = p as usize - arg as usize;
        if (crate::memory::cstr_prefix_bytes(c"link", len)
            == crate::memory::cstr_prefix_bytes(arg, len))
            || (crate::memory::cstr_prefix_bytes(c"clear", len)
                == crate::memory::cstr_prefix_bytes(arg, len))
        {
            xp.xp_pattern = skipwhite(p);
            p = skiptowhite(xp.xp_pattern).cast_const();
            if !crate::memory::cstr_is_empty(p) {
                xp.xp_pattern = skipwhite(p);
                p = skiptowhite(xp.xp_pattern).cast_const();
            }
        }
        if !crate::memory::cstr_is_empty(p) {
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
    msg_puts_hl(MSG.as_ptr().wrapping_add((cnt / 11) as usize), id, false);
    msg_clr_eos();
    ui_flush();
    os_delay(if cnt == 99 { 40 } else { (cnt as u64) * 50 }, false);
}

pub fn get_highlight_name(xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    get_highlight_name_ext(xp, idx, true)
}

pub fn get_highlight_name_ext(
    _xp: &mut ExpandCtx,
    idx: i32,
    skip_cleared: bool,
) -> Option<&'static CStr> {
    if idx < 0 {
        return None;
    }

    let len = hl_len();
    if skip_cleared && idx < len && hl_table_at(idx as usize).sg_cleared {
        return Some(c"");
    }

    if idx == len && g_st().include_none != 0 {
        return Some(c"none");
    } else if idx == len + g_st().include_none && g_st().include_default != 0 {
        return Some(c"default");
    } else if idx == len + g_st().include_none + g_st().include_default && g_st().include_link != 0
    {
        return Some(c"link");
    } else if idx == len + g_st().include_none + g_st().include_default + 1
        && g_st().include_link != 0
    {
        return Some(c"clear");
    } else if idx >= len {
        return None;
    }
    Some(hl_table_at(idx as usize).sg_name.as_c_str())
}

pub fn name_to_color(name: impl crate::strings::IntoCCharPtr, idx: *mut i32) -> RgbValue {
    let name = name.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(name) == b'#'
            && (crate::memory::cstr_byte_at(name, 1)).is_ascii_hexdigit()
            && (crate::memory::cstr_byte_at(name, 2)).is_ascii_hexdigit()
            && (crate::memory::cstr_byte_at(name, 3)).is_ascii_hexdigit()
            && (crate::memory::cstr_byte_at(name, 4)).is_ascii_hexdigit()
            && (crate::memory::cstr_byte_at(name, 5)).is_ascii_hexdigit()
            && (crate::memory::cstr_byte_at(name, 6)).is_ascii_hexdigit()
            && crate::memory::cstr_byte_at(name, 7) == 0
        {
            unsafe {
                *idx = K_COLOR_IDX_HEX;
            }
            let hex_str = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    name.wrapping_add(1).cast::<u8>(),
                    6,
                ))
            };
            return RgbValue::from_str_radix(hex_str, 16).unwrap_or(0);
        } else if mb_stricmp(name, c"bg") == 0 || mb_stricmp(name, c"background") == 0 {
            unsafe {
                *idx = K_COLOR_IDX_BG;
            }
            return g_st().normal_bg;
        } else if mb_stricmp(name, c"fg") == 0 || mb_stricmp(name, c"foreground") == 0 {
            unsafe {
                *idx = K_COLOR_IDX_FG;
            }
            return g_st().normal_fg;
        }

        let mut lo: usize = 0;
        let mut hi: usize = COLOR_NAME_TABLE.len();
        while lo < hi {
            let m = (lo + hi) / 2;
            let cmp = mb_stricmp(name, COLOR_NAME_TABLE[m].name.as_ptr());
            if cmp < 0 {
                hi = m;
            } else if cmp > 0 {
                lo = m + 1;
            } else {
                unsafe {
                    *idx = m as i32;
                }
                return COLOR_NAME_TABLE[m].color;
            }
        }

        unsafe {
            *idx = K_COLOR_IDX_NONE;
        }
        -1
    }
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

pub fn name_to_ctermcolor(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        let off = (crate::memory::cstr_first_byte(name)).to_ascii_uppercase();
        let mut i = COLOR_NAMES.len() as isize - 1;
        while i >= 0 {
            let cname = COLOR_NAMES[i as usize].as_ptr();
            if off == crate::memory::cstr_first_byte(cname)
                && mb_stricmp(name.wrapping_add(1), cname.wrapping_add(1)) == 0
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
            assert_eq!(coloridx_to_name(K_COLOR_IDX_FG, 0, &mut hexbuf), Some(c"fg"));
            assert_eq!(coloridx_to_name(K_COLOR_IDX_BG, 0, &mut hexbuf), Some(c"bg"));
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
