//! String option validation, handling, and completion (`src/nvim/optionstr.c`).

use crate::api::private::helpers::{TryState, api_clear_error, try_enter, try_leave};
use crate::api::private::validate::{Error, ErrorType, NvimString, Object, ObjectData, ObjectType};
use crate::api::win_config::{WIN_CONFIG_INIT, parse_winborder};
use crate::autocmd::{check_ei, get_event_name_no_group};
use crate::buffer::{MAXLNUM, STL_IN_ICON, STL_IN_TITLE, buf_T, kUIMessages, modifiable};
use crate::charset::{
    EOL_MAC, ascii_isdigit, buf_init_isk_chartab, char2cells, check_isopt, getdigits_int,
    hexhex2nr, init_isf_chartab, init_isi_chartab, kOptDyFlagUhex, transchar, transchar_byte,
    try_getdigits,
};
use crate::cmdexpand::{CompleteListItemGetter, ExpandGeneric};
use crate::cursor::{coladvance, colnr_T, pos_T};
use crate::cursor_shape::{SHAPE_CURSOR, parse_shape_opt};
use crate::diff::{diffanchors_changed, diffopt_changed, kOptDiffopt};
use crate::digraph::keymap_init;
use crate::drawscreen::{
    UPD_INVERTED, UPD_NOT_VALID, UPD_VALID, comp_col, redraw_all_later, redraw_buf_later,
    redraw_curbuf_later, redraw_later, redrawWinline, status_redraw_buf,
};
use crate::errors::*;
use crate::eval::vars::{do_unlet, get_var_value};
use crate::ex_getln::{B_IMODE_NONE, B_IMODE_USE_INSERT, check_opt_wim};
use crate::fold::{
    foldUpdateAll, foldmethodIsDiff, foldmethodIsExpr, foldmethodIsIndent, foldmethodIsMarker,
    newFoldLevel,
};
use crate::globals::*;
use crate::grid::{schar_T, schar_from_char, schar_from_cluster, schar_from_str};
use crate::highlight_group::{get_highlight_name, init_highlight};
use crate::indent::{briopt_check, kOptBreakindentopt, tabstop_set};
use crate::indent_c::parse_cino;
use crate::insexpand::{LSIZE, set_cpt_callbacks};
use crate::lua::converter::{Boolean, FieldHashfn, KeySetLink, OptKeySet};
use crate::mark::{fmark_T, fmarkv_T};
use crate::mbyte::{
    enc_canonize, get_encoding_name, utf_ClusterInfo, utf_ptr2StrCharInfo, utf_ptr2char,
    utfc_ptr2len,
};
use crate::memline::ml_setflags;
use crate::message::{kOptMessagesopt, msg_grid_validate, verbose_open, verbose_stop};
use crate::r#move::validate_virtcol;
use crate::msgpack_rpc::packer::Integer;
use crate::option::{
    copy_option_part, did_set_title, fill_culopt_flags, get_fileformat, get_option,
    get_option_default, kOptBackupcopy, kOptComments, kOptCompleteopt, kOptEventignorewin,
    kOptFileformat, kOptFileformats, kOptFlagComma, kOptFlagNDname, kOptFlagNFname,
    kOptFlagOneComma, kOptListchars, kOptStatusline, kOptTagcase, kOptVarsofttabstop,
    kOptVirtualedit, opt_error, option_next_keyval, option_slice_eq, optval_free_read,
    parse_winhl_opt, redraw_titles, set_iminsert_global, set_imsearch_global, set_option_direct,
    skip_to_option_part, valid_name,
};
use crate::option_vars::*;
use crate::os::env::{expand_T, vim_unsetenv_ext};
use crate::regexp::{regmatch_T, regprog_T, vim_regexec};
use crate::runtime::sctx_T;
use crate::spell::{
    compile_cap_prog, did_set_spell_option, spell_reload, valid_spellfile, valid_spelllang,
};
use crate::spellfile::spell_check_msm;
use crate::spellsuggest::{spell_apply_sps, spell_check_sps};
use crate::strings::{
    gettext, vim_snprintf, vim_strchr,
};
use crate::terminal::terminal_notify_theme;
use crate::ui::ui_has;
use crate::window::{check_colorcolumn, fcs_chars_T, global_stl_height, lcs_chars_T, win_T};
use crate::winfloat::{win_config_float, win_previewpopup_config};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

pub type OptIndex = c_int;
pub type OptScopeFlags = u8;
use crate::highlight::OptionalKeys;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CharsOption {
    kFillchars = 0,
    kListchars = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StlScopeType {
    kTopLevel,
    kItemGroup,
    kHighlightScope,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OptSchemaKind {
    kOptSchemaFlag = 0,
    kOptSchemaNum = 1,
    kOptSchemaSNum = 2,
    kOptSchemaEnum = 3,
    kOptSchemaStr = 4,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharBuf {
    pub data: *mut c_char,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptSchemaItem {
    pub name: &'static std::ffi::CStr,
    pub kind: OptSchemaKind,
    pub enum_values: &'static [&'static std::ffi::CStr],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptDictSchema {
    pub get_field: FieldHashfn,
    pub schema: &'static [OptSchemaItem],
    pub size: usize,
}

pub enum OptSetPrep {
    None,
    Flags(c_uint),
    Spellsuggest([c_int; 2]),
    Wildmode([u8; 4]),
    Encoding(std::ffi::CString),
    SpellcapProg(Option<Box<regprog_T>>),
    Tabstop(Vec<colnr_T>),
}

impl OptSetPrep {
    #[inline]
    pub fn as_flags(&self) -> c_uint {
        match *self {
            Self::Flags(f) => f,
            _ => 0,
        }
    }

    #[inline]
    pub fn take_encoding(&mut self) -> Option<std::ffi::CString> {
        match std::mem::replace(self, Self::None) {
            Self::Encoding(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn take_spellcap_prog(&mut self) -> Option<Box<regprog_T>> {
        match std::mem::replace(self, Self::None) {
            Self::SpellcapProg(p) => p,
            _ => None,
        }
    }

    #[inline]
    pub fn take_tabstop(&mut self) -> Vec<colnr_T> {
        match std::mem::replace(self, Self::None) {
            Self::Tabstop(v) => v,
            _ => Vec::new(),
        }
    }
}

#[repr(C)]
pub struct optset_T {
    pub os_varp: *mut c_void,
    pub os_idx: OptIndex,
    pub os_flags: c_int,
    pub os_oldval: Object,
    pub os_newval: Object,
    pub os_prep: OptSetPrep,
    pub os_value_checked: bool,
    pub os_value_changed: bool,
    pub os_restore_chartab: bool,
    pub os_errbuf: Option<CharBuf>,
    pub os_win: *mut win_T,
    pub os_buf: *mut buf_T,
}

impl optset_T {
    pub const fn new() -> Self {
        Self {
            os_varp: ptr::null_mut(),
            os_idx: 0,
            os_flags: 0,
            os_oldval: Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData::Boolean(false),
            },
            os_newval: Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData::Boolean(false),
            },
            os_prep: OptSetPrep::None,
            os_value_checked: false,
            os_value_changed: false,
            os_restore_chartab: false,
            os_errbuf: None,
            os_win: ptr::null_mut(),
            os_buf: ptr::null_mut(),
        }
    }
}

#[repr(C)]
pub struct optexpand_T {
    pub oe_varp: *mut c_char,
    pub oe_idx: OptIndex,
    pub oe_opt_value: *mut c_char,
    pub oe_append: bool,
    pub oe_include_orig_val: bool,
    pub oe_regmatch: *mut regmatch_T,
    pub oe_xp: *mut expand_T,
    pub oe_set_arg: *mut c_char,
}

pub type opt_validate_cb_T = Option<fn(&mut optset_T) -> *const c_char>;
pub type opt_did_set_cb_T = Option<fn(&mut optset_T) -> *const c_char>;
pub type opt_expand_cb_T = Option<fn(&mut optexpand_T, &mut Vec<std::ffi::CString>) -> bool>;

#[repr(C)]
pub struct vimoption_T {
    pub fullname: &'static std::ffi::CStr,
    pub shortname: Option<&'static std::ffi::CStr>,
    pub flags: u32,
    pub opt_type: ObjectType,
    pub scope_flags: OptScopeFlags,
    pub var: *mut c_void,
    pub flags_var: *mut c_uint,
    pub scope_idx: [isize; 4],
    pub immutable: bool,
    pub values: &'static [&'static std::ffi::CStr],
    pub schema: &'static [OptSchemaItem],
    pub opt_validate_cb: opt_validate_cb_T,
    pub opt_did_set_cb: opt_did_set_cb_T,
    pub opt_expand_cb: opt_expand_cb_T,
    pub def_val: Object,
    pub script_ctx: sctx_T,
}

use crate::charset::NUL;
use crate::decoration::SCL_NUM;
use crate::ex_cmds::B_IMODE_LMAP;
use crate::ex_session::{kOptSsopFlagCurdir, kOptSsopFlagSesdir};
use crate::fileio::kOptFileencoding;
use crate::globals::IOSIZE;
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::runtime::SID_NONE;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_briopt {
    pub is_set__briopt_: OptionalKeys,
    pub shift: Integer,
    pub min: Integer,
    pub sbr: Boolean,
    pub list: Integer,
    pub column: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_dip {
    pub is_set__dip_: OptionalKeys,
    pub filler: Boolean,
    pub anchor: Boolean,
    pub context: Integer,
    pub iblank: Boolean,
    pub icase: Boolean,
    pub iwhite: Boolean,
    pub iwhiteall: Boolean,
    pub iwhiteeol: Boolean,
    pub horizontal: Boolean,
    pub vertical: Boolean,
    pub closeoff: Boolean,
    pub hiddenoff: Boolean,
    pub foldcolumn: Integer,
    pub followwrap: Boolean,
    pub internal: Boolean,
    pub indent_heuristic: Boolean,
    pub algorithm: [c_char; 10],
    pub inline_: [c_char; 7],
    pub linematch: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_mopt {
    pub is_set__mopt_: OptionalKeys,
    pub hit_enter: Boolean,
    pub history: Integer,
    pub maxheight: Integer,
    pub pager: [c_char; 256],
    pub progress: [c_char; 2],
    pub timeout: Integer,
    pub wait: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_mousescroll {
    pub is_set__mousescroll_: OptionalKeys,
    pub hor: Integer,
    pub ver: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_pvp {
    pub is_set__pvp_: OptionalKeys,
    pub height: Integer,
    pub width: Integer,
    pub border: [c_char; 8],
}

const _: () = {
    const MOPT_SIZE: usize = std::mem::size_of::<OptKeyDict_mopt>();
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_briopt>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_dip>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_mousescroll>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_pvp>());
};

const kOptAmbiwidth: OptIndex = 2;
const kOptBackupext: OptIndex = 18;
const kOptBelloff: OptIndex = 20;
const kOptCasemap: OptIndex = 31;
const kOptClipboard: OptIndex = 43;
const kOptDisplay: OptIndex = 76;
const kOptEncoding: OptIndex = 80;
const kOptFoldopen: OptIndex = 112;
const kOptJumpoptions: OptIndex = 158;
const kOptMousescroll: OptIndex = 203;
const kOptPatchmode: OptIndex = 218;
pub(crate) const kOptPreviewpopup: OptIndex = 222;
const kOptRedrawdebug: OptIndex = 234;
const kOptSessionoptions: OptIndex = 257;
const kOptSwitchbuf: OptIndex = 302;
const kOptTabclose: OptIndex = 305;
const kOptTermpastefilter: OptIndex = 319;
const kOptViewoptions: OptIndex = 346;
const kOptWildoptions: OptIndex = 357;

pub(crate) const kOptBkcFlagYes: c_uint = 0x01;
pub(crate) const kOptBkcFlagAuto: c_uint = 0x02;
const kOptBkcFlagNo: c_uint = 0x04;

const KEYSET_OPTIDX_mousescroll__hor: u32 = 1;
const KEYSET_OPTIDX_mousescroll__ver: u32 = 2;
const MOUSESCROLL_VERT_DFLT: i64 = 3;
const MOUSESCROLL_HOR_DFLT: i64 = 6;

const SCL_NO: c_int = -1;

pub(crate) const STL_SEPARATE: c_char = b'=' as c_char;
pub(crate) const STL_TRUNCMARK: c_char = b'<' as c_char;
pub(crate) const STL_USER_HL: c_char = b'*' as c_char;
pub(crate) const STL_HIGHLIGHT: c_char = b'#' as c_char;

const CPT_ABBR: c_uint = 0;
const CPT_KIND: c_uint = 1;
const CPT_MENU: c_uint = 2;

const HIGHLIGHT_INIT: &CStr = c"8:SpecialKey,~:EndOfBuffer,z:TermCursor,@:NonText,d:Directory,e:ErrorMsg,i:IncSearch,l:Search,y:CurSearch,m:MoreMsg,M:ModeMsg,n:LineNr,a:LineNrAbove,b:LineNrBelow,N:CursorLineNr,G:CursorLineSign,O:CursorLineFold,r:Question,s:StatusLine,S:StatusLineNC,c:VertSplit,t:Title,v:Visual,V:VisualNOS,w:WarningMsg,W:WildMenu,f:Folded,F:FoldColumn,A:DiffAdd,C:DiffChange,D:DiffDelete,T:DiffText,E:DiffTextAdd,>:SignColumn,-:Conceal,B:SpellBad,P:SpellCap,R:SpellRare,L:SpellLocal,+:Pmenu,=:PmenuSel,k:PmenuMatch,<:PmenuMatchSel,[:PmenuKind,]:PmenuKindSel,{:PmenuExtra,}:PmenuExtraSel,x:PmenuSbar,X:PmenuThumb,*:TabLine,#:TabLineSel,_:TabLineFill,!:CursorColumn,.:CursorLine,o:ColorColumn,q:QuickFixLine,z:StatusLineTerm,Z:StatusLineTermNC,g:MsgArea,h:ComplMatchIns,0:Whitespace,I:PreInsert";

const COM_ALL: &CStr = c"nbsmexflrO";
const COCU_ALL: &CStr = c"nvic";
const CPO_VI: &CStr = c"aAbBcCdDeEfFiIJKlLmMnoOpPqrRsStuvWxXyZ$!%+>;~_";
const FO_ALL: &CStr = c"tcro/q2vlb1mMBn,aw]jp";
const MOUSE_ALL: &CStr = c"anvichr";
const WW_ALL: &CStr = c"bshl<>[]~";
const WW_ALL_COMMA: &CStr = c"bshl<>[]~,";
pub(crate) const STL_ALL: &CStr = c"fFtcvVlLnkoObBrRhHyYwWmMqpPaNSCs{=<*#$TX@";

const E_ILLEGAL_CHARACTER_AFTER_CHR: &CStr = c"E535: Illegal character after <%c>";
const E_COMMA_REQUIRED: &CStr = c"E536: Comma required";
const E_UNCLOSED_EXPRESSION_SEQUENCE: &CStr = c"E540: Unclosed expression sequence";
const E_UNBALANCED_GROUPS: &CStr = c"E542: Unbalanced groups";
const E_BACKUPEXT_AND_PATCHMODE_ARE_EQUAL: &CStr = c"E589: 'backupext' and 'patchmode' are equal";
const E_SHOWBREAK_CONTAINS_UNPRINTABLE_OR_WIDE_CHARACTER: &CStr =
    c"E595: 'showbreak' contains unprintable or wide character";
const E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR: &CStr =
    c"E1511: Wrong number of characters for field \"%s\"";
const E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR: &CStr =
    c"E1512: Wrong character width for field \"%s\"";
const E_CONFLICTS_WITH_VALUE_OF_LISTCHARS: &CStr = c"E834: Conflicts with value of 'listchars'";
const E_CONFLICTS_WITH_VALUE_OF_FILLCHARS: &CStr = c"E835: Conflicts with value of 'fillchars'";

const SHM_ALL: &CStr = c"rmlwaWtToOsAIcCqFSunfxi";

struct OptionStrState {
    set_opt_callback_orig_option: *mut c_char,
    set_opt_callback_func: CompleteListItemGetter,
    expand_eiw: bool,
    OPT_KEYSET_SHARED: [u64; std::mem::size_of::<OptKeyDict_mopt>().div_ceil(8)],
}

impl OptionStrState {
    const fn new() -> Self {
        Self {
            set_opt_callback_orig_option: ptr::null_mut(),
            set_opt_callback_func: None,
            expand_eiw: false,
            OPT_KEYSET_SHARED: [0; std::mem::size_of::<OptKeyDict_mopt>().div_ceil(8)],
        }
    }
}

static OPTSTR_STATE: GlobalState<OptionStrState> = GlobalState::new(OptionStrState::new());

#[inline]
fn optstr_st() -> &'static mut OptionStrState {
    unsafe { &mut *OPTSTR_STATE.as_ptr() }
}

static BRIOPT_TABLE: [KeySetLink; 6] = [
    KeySetLink {
        str: Some(c"min"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, min),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sbr"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, sbr),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"list"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, list),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"shift"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, shift),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"column"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, column),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::kObjectTypeNil as c_int,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static DIP_TABLE: [KeySetLink; 20] = [
    KeySetLink {
        str: Some(c"icase"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, icase),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iblank"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iblank),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filler"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, filler),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"anchor"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, anchor),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"inline"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, inline_),
        key_type: ObjectType::kObjectTypeString as c_int,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhite"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhite),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"context"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, context),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"closeoff"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, closeoff),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"internal"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, internal),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, vertical),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"linematch"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, linematch),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hiddenoff"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, hiddenoff),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"algorithm"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, algorithm),
        key_type: ObjectType::kObjectTypeString as c_int,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhiteall"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhiteall),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhiteeol"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhiteeol),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"foldcolumn"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, foldcolumn),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"horizontal"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, horizontal),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"followwrap"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, followwrap),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"indent-heuristic"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, indent_heuristic),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::kObjectTypeNil as c_int,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static MOPT_TABLE: [KeySetLink; 8] = [
    KeySetLink {
        str: Some(c"wait"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, wait),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pager"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, pager),
        key_type: ObjectType::kObjectTypeString as c_int,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"history"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, history),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"timeout"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, timeout),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"progress"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, progress),
        key_type: ObjectType::kObjectTypeString as c_int,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hit-enter"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, hit_enter),
        key_type: ObjectType::kObjectTypeBoolean as c_int,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"maxheight"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, maxheight),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::kObjectTypeNil as c_int,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static MOUSESCROLL_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"hor"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mousescroll, hor),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ver"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mousescroll, ver),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::kObjectTypeNil as c_int,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static PVP_TABLE: [KeySetLink; 4] = [
    KeySetLink {
        str: Some(c"width"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, width),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"border"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, border),
        key_type: ObjectType::kObjectTypeString as c_int,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"height"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, height),
        key_type: ObjectType::kObjectTypeInteger as c_int,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::kObjectTypeNil as c_int,
        opt_index: -1,
        is_hlgroup: false,
    },
];

use crate::mbyte::ptr2cells;

fn briopt_hash(key: &[u8]) -> isize {
    let low: isize = match key.len() {
        3 => match key[0] {
            b'm' => 0,
            b's' => 1,
            _ => -1,
        },
        4 => 2,
        5 => 3,
        6 => 4,
        _ => -1,
    };
    if low < 0 || BRIOPT_TABLE[low as usize].str.unwrap().to_bytes() != key {
        return -1;
    }
    low
}

fn OptKeyDict_briopt_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = briopt_hash(key);
    (hash >= 0).then(|| &BRIOPT_TABLE[hash as usize])
}

fn dip_hash(key: &[u8]) -> isize {
    let (low, high): (usize, usize) = match key.len() {
        5 => (0, 1),
        6 => match key[1] {
            b'b' => (1, 2),
            b'i' => (2, 3),
            b'n' => (3, 5),
            b'w' => (5, 6),
            _ => (0, 0),
        },
        7 => (6, 7),
        8 => match key[0] {
            b'c' => (7, 8),
            b'i' => (8, 9),
            b'v' => (9, 10),
            _ => (0, 0),
        },
        9 => match key[7] {
            b'c' => (10, 11),
            b'f' => (11, 12),
            b'h' => (12, 13),
            b'l' => (13, 14),
            b'o' => (14, 15),
            _ => (0, 0),
        },
        10 => match key[3] {
            b'd' => (15, 16),
            b'i' => (16, 17),
            b'l' => (17, 18),
            _ => (0, 0),
        },
        16 => (18, 19),
        _ => (0, 0),
    };
    for (i, entry) in DIP_TABLE.iter().enumerate().take(high).skip(low) {
        if entry.str.unwrap().to_bytes() == key {
            return i as isize;
        }
    }
    -1
}

fn OptKeyDict_dip_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = dip_hash(key);
    (hash >= 0).then(|| &DIP_TABLE[hash as usize])
}

fn mopt_hash(key: &[u8]) -> isize {
    let low: isize = match key.len() {
        4 => 0,
        5 => 1,
        7 => match key[0] {
            b'h' => 2,
            b't' => 3,
            _ => -1,
        },
        8 => 4,
        9 => match key[0] {
            b'h' => 5,
            b'm' => 6,
            _ => -1,
        },
        _ => -1,
    };
    if low < 0 || MOPT_TABLE[low as usize].str.unwrap().to_bytes() != key {
        return -1;
    }
    low
}

fn OptKeyDict_mopt_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = mopt_hash(key);
    (hash >= 0).then(|| &MOPT_TABLE[hash as usize])
}

fn mousescroll_hash(key: &[u8]) -> isize {
    let low: isize = match key.len() {
        3 => match key[0] {
            b'h' => 0,
            b'v' => 1,
            _ => -1,
        },
        _ => -1,
    };
    if low < 0 || MOUSESCROLL_TABLE[low as usize].str.unwrap().to_bytes() != key {
        return -1;
    }
    low
}

fn OptKeyDict_mousescroll_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = mousescroll_hash(key);
    (hash >= 0).then(|| &MOUSESCROLL_TABLE[hash as usize])
}

fn pvp_hash(key: &[u8]) -> isize {
    let low: isize = match key.len() {
        5 => 0,
        6 => match key[0] {
            b'b' => 1,
            b'h' => 2,
            _ => -1,
        },
        _ => -1,
    };
    if low < 0 || PVP_TABLE[low as usize].str.unwrap().to_bytes() != key {
        return -1;
    }
    low
}

fn OptKeyDict_pvp_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = pvp_hash(key);
    (hash >= 0).then(|| &PVP_TABLE[hash as usize])
}

pub(crate) fn opt_dict_schema(opt_idx: OptIndex) -> Option<&'static OptDictSchema> {
    static INFO_BRIOPT: OptDictSchema = OptDictSchema {
        get_field: Some(OptKeyDict_briopt_get_field),
        schema: &opt_briopt_schema,
        size: std::mem::size_of::<OptKeyDict_briopt>(),
    };
    static INFO_DIP: OptDictSchema = OptDictSchema {
        get_field: Some(OptKeyDict_dip_get_field),
        schema: &opt_dip_schema,
        size: std::mem::size_of::<OptKeyDict_dip>(),
    };
    static INFO_MOPT: OptDictSchema = OptDictSchema {
        get_field: Some(OptKeyDict_mopt_get_field),
        schema: &opt_mopt_schema,
        size: std::mem::size_of::<OptKeyDict_mopt>(),
    };
    static INFO_MOUSESCROLL: OptDictSchema = OptDictSchema {
        get_field: Some(OptKeyDict_mousescroll_get_field),
        schema: &opt_mousescroll_schema,
        size: std::mem::size_of::<OptKeyDict_mousescroll>(),
    };
    static INFO_PVP: OptDictSchema = OptDictSchema {
        get_field: Some(OptKeyDict_pvp_get_field),
        schema: &opt_pvp_schema,
        size: std::mem::size_of::<OptKeyDict_pvp>(),
    };

    match opt_idx {
        kOptBreakindentopt => Some(&INFO_BRIOPT),
        kOptDiffopt => Some(&INFO_DIP),
        kOptMessagesopt => Some(&INFO_MOPT),
        kOptMousescroll => Some(&INFO_MOUSESCROLL),
        kOptPreviewpopup => Some(&INFO_PVP),
        _ => None,
    }
}

/// After setting various option values: recompute variables that depend on option values.
pub fn didset_string_options() {
    check_str_opt(kOptCasemap, ptr::null_mut(), None);
        check_str_opt(kOptBackupcopy, ptr::null_mut(), None);
        check_str_opt(kOptBelloff, ptr::null_mut(), None);
        check_str_opt(kOptCompleteopt, ptr::null_mut(), None);
        check_str_opt(kOptSessionoptions, ptr::null_mut(), None);
        check_str_opt(kOptViewoptions, ptr::null_mut(), None);
        check_str_opt(kOptFoldopen, ptr::null_mut(), None);
        check_str_opt(kOptDisplay, ptr::null_mut(), None);
        check_str_opt(kOptJumpoptions, ptr::null_mut(), None);
        check_str_opt(kOptRedrawdebug, ptr::null_mut(), None);
        check_str_opt(kOptTagcase, ptr::null_mut(), None);
        check_str_opt(kOptTermpastefilter, ptr::null_mut(), None);
        check_str_opt(kOptVirtualedit, ptr::null_mut(), None);
        check_str_opt(kOptSwitchbuf, ptr::null_mut(), None);
        check_str_opt(kOptTabclose, ptr::null_mut(), None);
        check_str_opt(kOptWildoptions, ptr::null_mut(), None);
        check_str_opt(kOptClipboard, ptr::null_mut(), None);
}

fn illegal_char(errbuf: Option<CharBuf>, c: c_int) -> *const c_char {
    if errbuf.is_none() {
        return c"".as_ptr();
    }
    opt_error(
            errbuf,
            c"E539: Illegal character <%s>".as_ptr(),
            transchar(c),
        )
}

/// Check string options in a buffer for NULL value.
pub fn check_buf_options(buf: &mut buf_T) {
    check_string_option(&raw mut buf.b_p_bh);
        check_string_option(&raw mut buf.b_p_bt);
        check_string_option(&raw mut buf.b_p_fenc);
        check_string_option(&raw mut buf.b_p_ff);
        check_string_option(&raw mut buf.b_p_def);
        check_string_option(&raw mut buf.b_p_inc);
        check_string_option(&raw mut buf.b_p_indk);
        check_string_option(&raw mut buf.b_p_fp);
        check_string_option(&raw mut buf.b_p_kp);
        check_string_option(&raw mut buf.b_p_mps);
        check_string_option(&raw mut buf.b_p_fo);
        check_string_option(&raw mut buf.b_p_flp);
        check_string_option(&raw mut buf.b_p_isk);
        check_string_option(&raw mut buf.b_p_com);
        check_string_option(&raw mut buf.b_p_cms);
        check_string_option(&raw mut buf.b_p_nf);
        check_string_option(&raw mut buf.b_p_qe);
        check_string_option(&raw mut buf.b_p_syn);
        buf.b_s.b_syn_isk = None;
        check_string_option(&raw mut buf.b_s.b_p_spc);
        check_string_option(&raw mut buf.b_s.b_p_spf);
        check_string_option(&raw mut buf.b_s.b_p_spl);
        check_string_option(&raw mut buf.b_s.b_p_spo);
        check_string_option(&raw mut buf.b_p_sua);
        check_string_option(&raw mut buf.b_p_cink);
        check_string_option(&raw mut buf.b_p_cino);
        parse_cino(buf);
        check_string_option(&raw mut buf.b_p_lop);
        check_string_option(&raw mut buf.b_p_ft);
        check_string_option(&raw mut buf.b_p_cinw);
        check_string_option(&raw mut buf.b_p_cinsd);
        check_string_option(&raw mut buf.b_p_cot);
        check_string_option(&raw mut buf.b_p_cpt);
        check_string_option(&raw mut buf.b_p_keymap);
        check_string_option(&raw mut buf.b_p_gefm);
        check_string_option(&raw mut buf.b_p_gp);
        check_string_option(&raw mut buf.b_p_mp);
        check_string_option(&raw mut buf.b_p_efm);
        check_string_option(&raw mut buf.b_p_ep);
        check_string_option(&raw mut buf.b_p_path);
        check_string_option(&raw mut buf.b_p_tags);
        check_string_option(&raw mut buf.b_p_tc);
        check_string_option(&raw mut buf.b_p_dict);
        check_string_option(&raw mut buf.b_p_dia);
        check_string_option(&raw mut buf.b_p_tsr);
        check_string_option(&raw mut buf.b_p_lw);
        check_string_option(&raw mut buf.b_p_bkc);
        check_string_option(&raw mut buf.b_p_menc);
        check_string_option(&raw mut buf.b_p_vsts);
        check_string_option(&raw mut buf.b_p_vts);
}

pub fn alloc_string_option(s: *const c_char) -> *mut c_char {
     crate::memory::cstr_to_cstring(s).into_raw()
}

pub fn alloc_string_option_bytes(bytes: &[u8]) -> *mut c_char {
    crate::memory::cstr_from_bytes(bytes).into_raw()
}

/// Free the string allocated for an option.
pub fn free_string_option(p: *mut c_char) {
    unsafe {
        if !p.is_null() && p != empty_string_option.as_ptr().cast::<c_char>() {
            drop(std::ffi::CString::from_raw(p));
        }
    }
}

pub fn clear_string_option(pp: *mut *mut c_char) {
    unsafe {
        let empty = empty_string_option.as_ptr().cast::<c_char>();
        if !(*pp).is_null() && *pp != empty {
            drop(std::ffi::CString::from_raw(*pp));
        }
        *pp = empty;
    }
}

pub fn check_string_option(pp: *mut *mut c_char) {
    unsafe {
        if (*pp).is_null() {
            *pp = empty_string_option.as_ptr().cast::<c_char>();
        }
    }
}

/// Return true if "val" is a valid 'filetype' name.
fn valid_filetype(val: *const c_char) -> bool {
    valid_name(val, c".-_")
}

/// Handle setting 'signcolumn' for value 'val'.
pub fn check_signcolumn(scl: *mut c_char, wp: *mut win_T) -> bool {
    unsafe {
        let mut val = empty_string_option.as_ptr().cast::<c_char>();
        if !scl.is_null() {
            val = scl;
        } else if !wp.is_null() {
            val = (*wp).w_onebuf_opt.wo_scl;
        }

        if *val == NUL {
            return false;
        }

        if opt_strings_flags(
            val,
            &opt_scl_values,
            ptr::null_mut(),
            false,
            None,
        )
        .is_null()
        {
            if wp.is_null() {
                return true;
            }
            if crate::memory::cstr_prefix_bytes(val, 2) == &b"no"[..2] {
                (*wp).w_minscwidth = SCL_NO;
                (*wp).w_maxscwidth = SCL_NO;
            } else if (crate::memory::cstr_prefix_bytes(val, 2) == &b"nu"[..2])
                && ((*wp).w_onebuf_opt.wo_nu != 0 || (*wp).w_onebuf_opt.wo_rnu != 0)
            {
                (*wp).w_minscwidth = SCL_NUM;
                (*wp).w_maxscwidth = SCL_NUM;
            } else if crate::memory::cstr_prefix_bytes(val, 4) == &b"yes:"[..4] {
                let n = (*val.add(4) as c_int) - (b'0' as c_int);
                (*wp).w_minscwidth = n;
                (*wp).w_maxscwidth = n;
            } else if *val == b'y' as c_char {
                (*wp).w_minscwidth = 1;
                (*wp).w_maxscwidth = 1;
            } else if crate::memory::cstr_prefix_bytes(val, 5) == &b"auto:"[..5] {
                (*wp).w_minscwidth = 0;
                (*wp).w_maxscwidth = (*val.add(5) as c_int) - (b'0' as c_int);
            } else {
                (*wp).w_minscwidth = 0;
                (*wp).w_maxscwidth = 1;
            }
        } else {
            if !(crate::memory::cstr_prefix_bytes(val, 5) == &b"auto:"[..5])
                || crate::memory::cstr_len(val) != 8
                || !ascii_isdigit(*val.add(5) as c_int)
                || *val.add(6) != b'-' as c_char
                || !ascii_isdigit(*val.add(7) as c_int)
            {
                return false;
            }
            let min = (*val.add(5) as c_int) - (b'0' as c_int);
            let max = (*val.add(7) as c_int) - (b'0' as c_int);
            if min < 1 || max < 2 || min > 8 || min >= max {
                return false;
            }
            if wp.is_null() {
                return true;
            }
            (*wp).w_minscwidth = min;
            (*wp).w_maxscwidth = max;
        }

        let scwidth = if (*wp).w_minscwidth <= 0 {
            0
        } else {
            (*wp).w_maxscwidth.min((*wp).w_scwidth)
        };
        (*wp).w_scwidth = (*wp).w_minscwidth.max(scwidth);
        true
    }
}

/// Check validity of string with the 'statusline' format until the end of the current scope.
fn check_stl_str_adv_scope(
    s: *mut *mut c_char,
    errbuf: Option<CharBuf>,
    scope_type: StlScopeType,
) -> *const c_char {
    unsafe {
        while **s != NUL {
            while **s != NUL && **s != b'%' as c_char {
                *s = (*s).add(1);
            }
            if **s == NUL {
                break;
            }
            *s = (*s).add(1);
            if **s == b'%' as c_char || **s == STL_TRUNCMARK || **s == STL_SEPARATE {
                *s = (*s).add(1);
                continue;
            }
            if **s == b')' as c_char {
                *s = (*s).add(1);
                return if scope_type == StlScopeType::kItemGroup {
                    ptr::null()
                } else {
                    E_UNBALANCED_GROUPS.as_ptr()
                };
            }
            if **s == STL_HIGHLIGHT {
                *s = (*s).add(1);
                if **s == b'(' as c_char {
                    *s = (*s).add(1);
                    let err = check_stl_str_adv_scope(s, errbuf, StlScopeType::kHighlightScope);
                    if err.is_null() {
                        continue;
                    } else {
                        return err;
                    }
                } else if **s == b')' as c_char {
                    *s = (*s).add(1);
                    return if scope_type == StlScopeType::kHighlightScope {
                        ptr::null()
                    } else {
                        E_UNBALANCED_GROUPS.as_ptr()
                    };
                }
                while **s != NUL && **s != STL_HIGHLIGHT {
                    *s = (*s).add(1);
                }
                if **s != NUL {
                    *s = (*s).add(1);
                }
                continue;
            }
            if **s == b'-' as c_char {
                *s = (*s).add(1);
            }
            while ascii_isdigit(**s as c_int) {
                *s = (*s).add(1);
            }
            if **s == STL_USER_HL {
                continue;
            }
            if **s == b'.' as c_char {
                *s = (*s).add(1);
                while **s != NUL && ascii_isdigit(**s as c_int) {
                    *s = (*s).add(1);
                }
            }
            if **s == b'(' as c_char {
                *s = (*s).add(1);
                let err = check_stl_str_adv_scope(s, errbuf, StlScopeType::kItemGroup);
                if err.is_null() {
                    continue;
                } else {
                    return err;
                }
            }
            if vim_strchr(STL_ALL, **s as u8 as c_int).is_null() {
                return illegal_char(errbuf, **s as u8 as c_int);
            }
            if **s == b'{' as c_char {
                *s = (*s).add(1);
                let reevaluate = **s == b'%' as c_char;

                if reevaluate {
                    *s = (*s).add(1);
                    if **s == b'}' as c_char {
                        return illegal_char(errbuf, b'}' as c_int);
                    }
                }
                while (**s != b'}' as c_char || (reevaluate && *(*s).sub(1) != b'%' as c_char))
                    && **s != NUL
                {
                    *s = (*s).add(1);
                }
                if **s != b'}' as c_char {
                    return E_UNCLOSED_EXPRESSION_SEQUENCE.as_ptr();
                }
            }
        }
        if scope_type == StlScopeType::kTopLevel {
            ptr::null()
        } else {
            E_UNBALANCED_GROUPS.as_ptr()
        }
    }
}

/// Check validity of options with the 'statusline' format.
pub fn check_stl_option(mut s: *mut c_char, errbuf: Option<CharBuf>) -> *const c_char {
    check_stl_str_adv_scope(&raw mut s, errbuf, StlScopeType::kTopLevel)
}

/// Check for a "normal" directory or file name in some options.
pub fn check_illegal_path_names(val: *mut c_char, flags: u32) -> bool {
     {
        let nfname_illegal = if g_st().secure != 0 {
            c"/\\*?[|;&<>\r\n".as_ptr()
        } else {
            c"/\\*?[<>\r\n".as_ptr()
        };
        ((flags & kOptFlagNFname) != 0 && crate::memory::cstr_as_bytes(val).iter().any(|b| crate::memory::cstr_as_bytes(nfname_illegal).contains(b)))
            || ((flags & kOptFlagNDname) != 0 && crate::memory::cstr_as_bytes(val).iter().any(|b| b"*?[|;&<>\r\n".contains(b)))
    }
}

fn opt_values(idx: OptIndex) -> &'static [&'static CStr] {
    unsafe {
        let idx1 = if idx == kOptViewoptions {
            kOptSessionoptions
        } else if idx == kOptFileformats {
            kOptFileformat
        } else {
            idx
        };

        (*get_option(idx1)).values
    }
}

fn check_str_opt(
    idx: OptIndex,
    mut varp: *mut *mut c_char,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let opt = get_option(idx);
        if varp.is_null() {
            varp = (*opt).var.cast::<*mut c_char>();
        }
        let list = ((*opt).flags & (kOptFlagComma | kOptFlagOneComma)) != 0;
        let values = opt_values(idx);
        opt_strings_flags(*varp, values, (*opt).flags_var, list, errbuf)
    }
}

pub fn expand_set_str_generic(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let values = opt_values(args.oe_idx);
        expand_set_opt_string(args, values, matches)
}

/// An option that accepts a list of flags is changed.
pub fn did_set_str_generic(args: &mut optset_T) -> *const c_char {
    check_str_opt(
            args.os_idx,
            args.os_varp.cast::<*mut c_char>(),
            args.os_errbuf,
        )
}

/// Validate named string values without updating derived flags.
pub fn validate_str_generic(args: &mut optset_T) -> *const c_char {
    unsafe {
        let opt = get_option(args.os_idx);
        let value = args.os_newval.data.string().data;
        opt_strings_flags(
            value,
            opt_values(args.os_idx),
            ptr::null_mut(),
            ((*opt).flags & kOptFlagComma) != 0,
            args.os_errbuf,
        )
    }
}

/// Validate a list of character flags against "flags".
fn check_option_listflag(
    val: *const c_char,
    flags: *const c_char,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let mut s = val;
        while *s != NUL {
            if vim_strchr(flags, *s as u8 as c_int).is_null() {
                return illegal_char(errbuf, *s as u8 as c_int);
            }
            s = s.add(1);
        }
        ptr::null()
    }
}

/// Expand an option that accepts a list of string values.
fn expand_set_opt_string(
    args: &mut optexpand_T,
    values: &[&CStr],
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        let regmatch = args.oe_regmatch;
        let include_orig_val = args.oe_include_orig_val;
        let option_val = args.oe_opt_value;

        matches.clear();

        if include_orig_val && *option_val != NUL {
            matches.push(crate::memory::cstr_to_cstring(option_val));
        }

        for &val in values {
            if val.to_bytes().is_empty()
                || (include_orig_val && *option_val != NUL && (crate::memory::cstr_as_bytes(val.as_ptr()) == crate::memory::cstr_as_bytes(option_val)))
            {
                continue;
            }
            if vim_regexec(&mut *regmatch, val.as_ptr(), 0) {
                matches.push(val.to_owned());
            }
        }
        if matches.is_empty() {
            return false;
        }
        true
    }
}

/// Callback used by expand_set_opt_generic to also include the original value.
fn expand_set_opt_callback(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
            if !optstr_st().set_opt_callback_orig_option.is_null() {
                return optstr_st().set_opt_callback_orig_option;
            } else {
                return c"".as_ptr().cast_mut();
            }
        }
        if let Some(f) = optstr_st().set_opt_callback_func {
            f(xp, idx - 1)
        } else {
            ptr::null_mut()
        }
}

/// Expand an option with a callback that iterates through a list of possible names.
fn expand_set_opt_generic(
    args: &mut optexpand_T,
    func: CompleteListItemGetter,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        optstr_st().set_opt_callback_orig_option = if args.oe_include_orig_val {
            args.oe_opt_value
        } else {
            ptr::null_mut()
        };
        optstr_st().set_opt_callback_func = func;

        ExpandGeneric(
            c"".as_ptr(),
            &mut *args.oe_xp,
            &mut *args.oe_regmatch,
            matches,
            Some(expand_set_opt_callback),
            false,
        );

        optstr_st().set_opt_callback_orig_option = ptr::null_mut();
        optstr_st().set_opt_callback_func = None;
        true
    }
}

/// Expand an option which is a list of flags.
fn expand_set_opt_listflag(
    args: &mut optexpand_T,
    flags: *const c_char,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        let option_val = args.oe_opt_value;
        let cmdline_val = args.oe_set_arg;
        let append = args.oe_append;
        let include_orig_val = args.oe_include_orig_val && (*option_val != NUL);

        matches.clear();

        if include_orig_val {
            matches.push(crate::memory::cstr_to_cstring(option_val));
        }

        let mut flag = flags;
        while *flag != NUL {
            if append && !vim_strchr(option_val, *flag as u8 as c_int).is_null() {
                flag = flag.add(1);
                continue;
            }

            if vim_strchr(cmdline_val, *flag as u8 as c_int).is_null() {
                if include_orig_val && *option_val.add(1) == NUL && *flag == *option_val {
                    flag = flag.add(1);
                    continue;
                }
                matches.push(crate::memory::cstr_from_bytes(&[*flag as u8]));
            }
            flag = flag.add(1);
        }

        if matches.is_empty() {
            return false;
        }
        true
    }
}

fn completing_value_for_subopt(args: &mut optexpand_T, name_suffix: &std::ffi::CStr) -> bool {
    unsafe {
        let colon = (*args.oe_xp).xp_pattern.sub(1);
        let len = name_suffix.to_bytes().len();

        colon.offset_from(args.oe_set_arg) >= (len as isize)
            && (crate::memory::cstr_prefix_bytes(colon.sub(len), len) == crate::memory::cstr_prefix_bytes(name_suffix.as_ptr(), len))
    }
}

/// The 'ambiwidth' option is changed.
pub fn did_set_ambiwidth(_args: &mut optset_T) -> *const c_char {
    check_chars_options()
}

/// The 'emoji' option is changed.
pub fn did_set_emoji(args: &mut optset_T) -> *const c_char {
    let errmsg = check_str_opt(kOptAmbiwidth, ptr::null_mut(), args.os_errbuf);
        if !errmsg.is_null() {
            return errmsg;
        }
        check_chars_options()
}

/// The 'background' option is changed.
pub fn did_set_background(args: &mut optset_T) -> *const c_char {
    unsafe {
        if *args.os_oldval.data.string().data == *opt_vars().p_bg {
            return ptr::null();
        }

        let dark = *opt_vars().p_bg == b'd' as c_char;

        init_highlight(false, false);

        if dark != (*opt_vars().p_bg == b'd' as c_char) && !get_var_value(c"g:colors_name").is_null()
        {
            do_unlet(c"g:colors_name".as_ptr(), 13, true);
            free_string_option(opt_vars().p_bg);
            opt_vars().p_bg = alloc_string_option(if dark {
                c"dark".as_ptr()
            } else {
                c"light".as_ptr()
            });
            check_string_option(&raw mut opt_vars().p_bg);
            init_highlight(false, false);
        }

        let mut buf = g_st().firstbuf;
        while !buf.is_null() {
            if !(*buf).terminal.is_null() {
                terminal_notify_theme(&mut *(*buf).terminal.cast(), dark);
            }
            buf = (*buf).b_next;
        }

        ptr::null()
    }
}

/// Validate the 'backspace' option.
pub fn validate_backspace(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        if ascii_isdigit(*value as c_int) {
            return if *value == b'2' as c_char {
                ptr::null()
            } else {
                e_invarg.as_ptr()
            };
        }
        validate_str_generic(args)
    }
}

pub fn validate_backupcopy(args: &mut optset_T) -> *const c_char {
    let mut flags: c_uint = 0;
        let errmsg = opt_strings_flags(
            args.os_newval.data.string().data,
            &opt_bkc_values,
            &raw mut flags,
            true,
            args.os_errbuf,
        );
        if !errmsg.is_null() {
            return errmsg;
        }
        let cnt = u32::from((flags & kOptBkcFlagAuto) != 0)
            + u32::from((flags & kOptBkcFlagYes) != 0)
            + u32::from((flags & kOptBkcFlagNo) != 0);
        if cnt == 1 {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// The 'backupcopy' option is changed.
pub fn did_set_backupcopy(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let opt_flags = args.os_flags;
        let mut bkc = opt_vars().p_bkc;
        let mut flags = &raw mut opt_vars().bkc_flags;

        if (opt_flags & OPT_LOCAL) != 0 {
            bkc = (*buf).b_p_bkc;
            flags = &raw mut (*buf).b_bkc_flags;
        } else if (opt_flags & OPT_GLOBAL) == 0 {
            (*buf).b_bkc_flags = 0;
        }

        if (opt_flags & OPT_LOCAL) != 0 && *bkc == NUL {
            *flags = 0;
        } else {
            let errmsg = opt_strings_flags(
                bkc,
                &opt_bkc_values,
                flags,
                true,
                args.os_errbuf,
            );
            if !errmsg.is_null() {
                return errmsg;
            }
        }

        ptr::null()
    }
}

/// Validate the 'backupext' or 'patchmode' option.
pub fn validate_backupext_or_patchmode(args: &mut optset_T) -> *const c_char {
    unsafe {
        let bex = if args.os_idx == kOptBackupext {
            args.os_newval.data.string().data
        } else {
            opt_vars().p_bex
        };
        let pm = if args.os_idx == kOptPatchmode {
            args.os_newval.data.string().data
        } else {
            opt_vars().p_pm
        };
        let bex_s = if *bex == b'.' as c_char {
            bex.add(1)
        } else {
            bex
        };
        let pm_s = if *pm == b'.' as c_char { pm.add(1) } else { pm };
        if crate::memory::cstr_as_bytes(bex_s) == crate::memory::cstr_as_bytes(pm_s) {
            return E_BACKUPEXT_AND_PATCHMODE_ARE_EQUAL.as_ptr();
        }

        ptr::null()
    }
}

/// The 'breakat' option is changed.
pub fn did_set_breakat(_args: &mut optset_T) -> *const c_char {
    unsafe {
        let flags_ptr = (&raw mut opt_vars().breakat_flags).cast::<c_char>();
        for i in 0..256 {
            *flags_ptr.add(i) = 0;
        }

        if !opt_vars().p_breakat.is_null() {
            let mut p = opt_vars().p_breakat;
            while *p != NUL {
                *flags_ptr.add(*p as u8 as usize) = 1;
                p = p.add(1);
            }
        }

        ptr::null()
    }
}

/// The 'breakindentopt' option is changed.
pub fn did_set_breakindentopt(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let is_local = ptr::eq(
            args.os_varp,
            (&raw mut (*win).w_onebuf_opt.wo_briopt).cast(),
        );
        briopt_check(if is_local { win } else { ptr::null_mut() });

        if is_local && (*win).w_briopt_list != 0 {
            redraw_all_later(UPD_NOT_VALID);
        }

        ptr::null()
    }
}

/// Validate the 'buftype' option.
pub fn validate_buftype(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let errmsg = validate_str_generic(args);
        if !errmsg.is_null() {
            return errmsg;
        }
        if (args.os_flags & OPT_GLOBAL) == 0
            && !(*buf).terminal.is_null() != (*args.os_newval.data.string().data == b't' as c_char)
        {
            return e_invarg.as_ptr();
        }
        ptr::null()
    }
}

/// The 'buftype' option is changed.
pub fn did_set_buftype(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let win = args.os_win;
        if *(*buf).b_p_bt == b'p' as c_char {
            set_option_direct(
                kOptComments,
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(NvimString {
                            data: c"".as_ptr().cast_mut(),
                            size: 0,
                        }),
                },
                OPT_LOCAL,
                SID_NONE,
            );
            let next_prompt = pos_T {
                lnum: (*buf).b_ml.ml_line_count,
                col: (*buf).b_prompt_start.mark.col,
                coladd: 0,
            };
            (*buf).b_prompt_start = fmark_T {
                mark: next_prompt,
                fnum: 0,
                timestamp: 0,
                view: fmarkv_T {
                    topline_offset: MAXLNUM,
                    skipcol: 0,
                },
                additional_data: None,
            };
        }
        if (*win).w_status_height != 0 || global_stl_height() != 0 {
            (*win).w_redr_status = true;
            redraw_later(win, UPD_VALID);
        }
        (*buf).b_help = *(*buf).b_p_bt == b'h' as c_char;
        redraw_titles();
        ptr::null()
    }
}

pub fn validate_chars_option(args: &mut optset_T) -> *const c_char {
    set_chars_option(
            args.os_win,
            args.os_newval.data.string().data,
            if args.os_idx == kOptListchars {
                CharsOption::kListchars
            } else {
                CharsOption::kFillchars
            },
            false,
            args.os_errbuf,
        )
}

/// The global 'listchars' or 'fillchars' option is changed.
fn did_set_global_chars_option(
    win: &mut win_T,
    val: *const c_char,
    what: CharsOption,
    opt_flags: c_int,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let local_ptr = if what == CharsOption::kListchars {
            &raw mut win.w_onebuf_opt.wo_lcs
        } else {
            &raw mut win.w_onebuf_opt.wo_fcs
        };

        let errmsg = set_chars_option(
            &raw mut *win,
            val,
            what,
            **local_ptr == NUL || (opt_flags & OPT_GLOBAL) == 0,
            errbuf,
        );
        if !errmsg.is_null() {
            return errmsg;
        }

        if (opt_flags & OPT_GLOBAL) == 0 {
            clear_string_option(local_ptr);
        }

        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                let opt = if what == CharsOption::kListchars {
                    (*wp).w_onebuf_opt.wo_lcs
                } else {
                    (*wp).w_onebuf_opt.wo_fcs
                };
                if *opt == NUL {
                    set_chars_option(wp, opt, what, true, errbuf);
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        redraw_all_later(UPD_NOT_VALID);

        ptr::null()
    }
}

/// The 'fillchars' option or the 'listchars' option is changed.
pub fn did_set_chars_option(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let varp = args.os_varp.cast::<*mut c_char>();
        let mut errmsg: *const c_char = ptr::null();

        if ptr::eq(varp, &raw mut opt_vars().p_lcs) {
            errmsg = did_set_global_chars_option(
                &mut *win,
                *varp,
                CharsOption::kListchars,
                args.os_flags,
                args.os_errbuf,
            );
        } else if ptr::eq(varp, &raw mut opt_vars().p_fcs) {
            errmsg = did_set_global_chars_option(
                &mut *win,
                *varp,
                CharsOption::kFillchars,
                args.os_flags,
                args.os_errbuf,
            );
        } else if ptr::eq(varp, &raw mut (*win).w_onebuf_opt.wo_lcs) {
            errmsg = set_chars_option(win, *varp, CharsOption::kListchars, true, args.os_errbuf);
        } else if ptr::eq(varp, &raw mut (*win).w_onebuf_opt.wo_fcs) {
            errmsg = set_chars_option(win, *varp, CharsOption::kFillchars, true, args.os_errbuf);
        }

        errmsg
    }
}

/// Expand 'fillchars' or 'listchars' option value.
pub fn expand_set_chars_option(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        let varp = args.oe_varp.cast::<*mut c_char>();
        let is_lcs =
            ptr::eq(varp, &raw mut opt_vars().p_lcs) || ptr::eq(varp, &raw mut (**curwin.as_ptr()).w_onebuf_opt.wo_lcs);
        expand_set_opt_generic(
            args,
            Some(if is_lcs {
                get_listchars_name
            } else {
                get_fillchars_name
            }),
            matches,
        )
    }
}

/// The 'cinoptions' option is changed.
pub fn did_set_cinoptions(args: &mut optset_T) -> *const c_char {
    unsafe {
        parse_cino(&mut *args.os_buf);
        ptr::null()
    }
}

pub fn validate_colorcolumn(args: &mut optset_T) -> *const c_char {
    check_colorcolumn(args.os_newval.data.string().data, ptr::null_mut())
}

/// The 'colorcolumn' option is changed.
pub fn did_set_colorcolumn(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let varp = args.os_varp.cast::<*mut c_char>();
        check_colorcolumn(
            *varp,
            if ptr::eq(varp, &raw mut (*win).w_onebuf_opt.wo_cc) {
                win
            } else {
                ptr::null_mut()
            },
        )
    }
}

/// Validate the 'comments' option.
pub fn validate_comments(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut errmsg: *const c_char = ptr::null();
        let mut s = args.os_newval.data.string().data;
        while *s != NUL {
            while *s != NUL && *s != b':' as c_char {
                if vim_strchr(COM_ALL, *s as u8 as c_int).is_null()
                    && !ascii_isdigit(*s as c_int)
                    && *s != b'-' as c_char
                {
                    errmsg = illegal_char(args.os_errbuf, *s as u8 as c_int);
                    break;
                }
                s = s.add(1);
            }
            let c = *s;
            s = s.add(1);
            if c == NUL {
                errmsg = c"E524: Missing colon".as_ptr();
            } else if *s == b',' as c_char || *s == NUL {
                errmsg = c"E525: Zero length string".as_ptr();
            }
            if !errmsg.is_null() {
                break;
            }
            while *s != NUL && *s != b',' as c_char {
                if *s == b'\\' as c_char && *s.add(1) != NUL {
                    s = s.add(1);
                }
                s = s.add(1);
            }
            s = skip_to_option_part(s);
        }
        errmsg
    }
}

/// Validate the 'commentstring' option.
pub fn validate_commentstring(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        if *value == NUL || crate::memory::cstr_as_bytes(value).windows(2).any(|w| w == b"%s") {
            ptr::null()
        } else {
            c"E537: 'commentstring' must be empty or contain %s".as_ptr()
        }
    }
}

/// Validate the 'complete' option.
pub fn validate_complete(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut buffer = [0 as c_char; LSIZE];
        let mut char_before: u8 = 0;

        let mut p = args.os_newval.data.string().data;
        while *p != NUL {
            buffer.fill(0);
            let mut buf_idx: usize = 0;
            let mut escape = false;

            while *p != NUL && (*p != b',' as c_char || escape) && buf_idx < LSIZE - 1 {
                if *p == b'\\' as c_char && *p.add(1) == b',' as c_char {
                    escape = true;
                    p = p.add(1);
                } else {
                    escape = false;
                    buffer[buf_idx] = *p;
                    buf_idx += 1;
                }
                p = p.add(1);
            }
            buffer[buf_idx] = NUL;

            if vim_strchr(c".wbuksid]tUfFo", buffer[0] as u8 as c_int).is_null() {
                return illegal_char(args.os_errbuf, buffer[0] as u8 as c_int);
            }

            if vim_strchr(c"ksF", buffer[0] as u8 as c_int).is_null()
                && buffer[1] != NUL
                && buffer[1] != b'^' as c_char
            {
                char_before = buffer[0] as u8;
            } else {
                let mut t = vim_strchr(buffer.as_ptr(), b'^' as c_int);
                if !t.is_null() {
                    *t = NUL;
                    t = t.add(1);
                    if *t == NUL {
                        char_before = b'^';
                    } else {
                        while *t != NUL {
                            if !ascii_isdigit(*t as c_int) {
                                char_before = b'^';
                                break;
                            }
                            t = t.add(1);
                        }
                    }
                }
            }
            if char_before != 0 {
                return crate::option::opt_error_int(
                    args.os_errbuf,
                    E_ILLEGAL_CHARACTER_AFTER_CHR.as_ptr(),
                    c_int::from(char_before),
                );
            }
            while *p == b',' as c_char || *p == b' ' as c_char {
                p = p.add(1);
            }
        }

        ptr::null()
    }
}

/// The 'complete' option is changed.
pub fn did_set_complete(args: &mut optset_T) -> *const c_char {
    if !set_cpt_callbacks(args) {
            return crate::option::opt_error_int(
                args.os_errbuf,
                E_ILLEGAL_CHARACTER_AFTER_CHR.as_ptr(),
                b'F' as c_int,
            );
        }
        ptr::null()
}

/// Validate 'completeitemalign' and prepare its column order.
pub fn validate_completeitemalign(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut p = args.os_newval.data.string().data;
        let mut new_cia_flags: c_uint = 0;
        let mut seen = [false; 3];
        let mut count = 0;
        let mut buf = [0 as c_char; 10];
        while *p != NUL {
            copy_option_part(&raw mut p, buf.as_mut_ptr(), buf.len(), c",");
            let item = if crate::memory::cstr_as_bytes(buf.as_ptr()) == b"abbr" {
                CPT_ABBR
            } else if crate::memory::cstr_as_bytes(buf.as_ptr()) == b"kind" {
                CPT_KIND
            } else if crate::memory::cstr_as_bytes(buf.as_ptr()) == b"menu" {
                CPT_MENU
            } else {
                return e_invarg.as_ptr();
            };
            if seen[item as usize] {
                return e_invarg.as_ptr();
            }
            new_cia_flags = new_cia_flags * 10 + item;
            seen[item as usize] = true;
            count += 1;
        }
        if count != 3 {
            return e_invarg.as_ptr();
        }
        args.os_prep = OptSetPrep::Flags(new_cia_flags);
        ptr::null()
    }
}

/// The 'completeitemalign' option is changed.
pub fn did_set_completeitemalign(args: &mut optset_T) -> *const c_char {
    opt_vars().cia_flags = args.os_prep.as_flags();
        ptr::null()
}

/// The 'completeopt' option is changed.
pub fn did_set_completeopt(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let mut cot = opt_vars().p_cot;
        let mut flags = &raw mut opt_vars().cot_flags;

        if (args.os_flags & OPT_LOCAL) != 0 {
            cot = (*buf).b_p_cot;
            flags = &raw mut (*buf).b_cot_flags;
        } else if (args.os_flags & OPT_GLOBAL) == 0 {
            (*buf).b_cot_flags = 0;
        }

        opt_strings_flags(
            cot,
            &opt_cot_values,
            flags,
            true,
            args.os_errbuf,
        )
    }
}

#[cfg(windows)]
pub fn did_set_completeslash(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let errmsg = opt_strings_flags(
            opt_vars().p_csl,
            &opt_csl_values,
            ptr::null_mut(),
            false,
            args.os_errbuf,
        );
        if !errmsg.is_null() {
            return errmsg;
        }
        opt_strings_flags(
            (*buf).b_p_csl,
            &opt_csl_values,
            ptr::null_mut(),
            false,
            args.os_errbuf,
        )
    }
}

/// Validate the 'concealcursor' option.
pub fn validate_concealcursor(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            COCU_ALL.as_ptr(),
            args.os_errbuf,
        )
}

pub fn expand_set_concealcursor(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, COCU_ALL.as_ptr(), matches)
}

/// Validate the 'cpoptions' option.
pub fn validate_cpoptions(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            CPO_VI.as_ptr(),
            args.os_errbuf,
        )
}

pub fn expand_set_cpoptions(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, CPO_VI.as_ptr(), matches)
}

pub fn validate_cursorlineopt(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        let mut flags: c_uint = 0;
        if *value == NUL || !fill_culopt_flags(value, &mut flags) {
            return e_invarg.as_ptr();
        }
        args.os_prep = OptSetPrep::Flags(flags);
        ptr::null()
    }
}

/// The 'cursorlineopt' option is changed.
pub fn did_set_cursorlineopt(args: &mut optset_T) -> *const c_char {
    unsafe {
        if (args.os_flags & OPT_GLOBAL) != 0 {
            return ptr::null();
        }
        let win = args.os_win;
        (*win).w_p_culopt_flags = args.os_prep.as_flags() as u8;
        ptr::null()
    }
}

/// The 'diffanchors' option is changed.
pub fn did_set_diffanchors(args: &mut optset_T) -> *const c_char {
    if !diffanchors_changed((args.os_flags & OPT_LOCAL) != 0) {
            return e_invarg.as_ptr();
        }
        ptr::null()
}

/// The 'diffopt' option is changed.
pub fn did_set_diffopt(_args: &mut optset_T) -> *const c_char {
    if !diffopt_changed() {
            e_invarg.as_ptr()
        } else {
            ptr::null()
        }
}

pub fn expand_set_diffopt(args: &mut optexpand_T, matches: &mut Vec<std::ffi::CString>) -> bool {
    unsafe {
        let xp = args.oe_xp;

        if ((*xp).xp_pattern as usize) > (args.oe_set_arg as usize)
            && *(*xp).xp_pattern.sub(1) == b':' as c_char
        {
            if completing_value_for_subopt(args, c"algorithm") {
                return expand_set_opt_string(
                    args,
                    &opt_dip_algorithm_values,
                    matches,
                );
            }
            if completing_value_for_subopt(args, c"inline") {
                return expand_set_opt_string(
                    args,
                    &opt_dip_inline_values,
                    matches,
                );
            }
            return false;
        }

        expand_set_str_generic(args, matches)
    }
}

/// The 'display' option is changed.
pub fn did_set_display(args: &mut optset_T) -> *const c_char {
    let errmsg = did_set_str_generic(args);
        if !errmsg.is_null() {
            return errmsg;
        }
        opt_vars().dy_escape_width = if (opt_vars().dy_flags & kOptDyFlagUhex) != 0 {
            4
        } else {
            2
        };
        msg_grid_validate();
        ptr::null()
}

/// Validate and canonize 'encoding', 'fileencoding' or 'makeencoding'.
pub fn validate_encoding(args: &mut optset_T) -> *const c_char {
    unsafe {
        if args.os_idx == kOptFileencoding {
            let buf = args.os_buf;
            if !modifiable(&*buf) && args.os_flags != OPT_GLOBAL {
                return e_modifiable.as_ptr();
            }

            if !vim_strchr(args.os_newval.data.string().data, b',' as c_int).is_null() {
                return e_invarg.as_ptr();
            }
        }

        let value_cstr = enc_canonize(args.os_newval.data.string().data);
        let is_not_utf8 = value_cstr.as_bytes() != b"utf-8";
        args.os_prep = OptSetPrep::Encoding(value_cstr);
        if args.os_idx == kOptEncoding && is_not_utf8 {
            return e_unsupportedoption.as_ptr();
        }
        ptr::null()
    }
}

/// One of the 'encoding', 'fileencoding' or 'makeencoding' options is changed.
pub fn did_set_encoding(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let varp = args.os_varp.cast::<*mut c_char>();
        if args.os_idx == kOptFileencoding {
            redraw_titles();
            ml_setflags(buf);
        }

        if let Some(prep_cstr) = args.os_prep.take_encoding() {
            free_string_option(*varp);
            *varp = alloc_string_option(prep_cstr.as_ptr());
        }
        if ptr::eq(varp, &raw mut opt_vars().p_enc) {
            spell_reload();
        }
        ptr::null()
    }
}

pub fn expand_set_encoding(args: &mut optexpand_T, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_generic(args, Some(get_encoding_name), matches)
}

/// Validate the 'eventignore'/'eventignorewin' option.
pub fn validate_eventignore(args: &mut optset_T) -> *const c_char {
    if !check_ei(
            args.os_newval.data.string().data,
            args.os_idx == kOptEventignorewin,
        )
        {
            return e_invarg.as_ptr();
        }
        ptr::null()
}

fn get_eventignore_name(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let subtract = *xp.xp_pattern == b'-' as c_char;
        if !subtract && idx == 0 {
            return c"all".as_ptr().cast_mut();
        }

        let name = get_event_name_no_group(xp, idx - 1 + c_int::from(subtract), optstr_st().expand_eiw);
        if name.is_null() {
            return ptr::null_mut();
        }

        let io_buff = (&raw mut g_st().IObuff).cast::<c_char>();
        vim_snprintf!(
            io_buff,
            IOSIZE,
            c"%s%s",
            if subtract {
                c"-".as_ptr()
            } else {
                c"".as_ptr()
            },
            name,
        );
        io_buff
    }
}

pub fn expand_set_eventignore(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    optstr_st().expand_eiw = !ptr::eq(args.oe_varp, (&raw mut opt_vars().p_ei).cast::<c_char>());
        expand_set_opt_generic(args, Some(get_eventignore_name), matches)
}

pub fn validate_fileformat(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        if (args.os_flags & OPT_GLOBAL) == 0 && !modifiable(&*buf) {
            return e_modifiable.as_ptr();
        }
        validate_str_generic(args)
    }
}

/// The 'fileformat' option is changed.
pub fn did_set_fileformat(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let oldval = args.os_oldval.data.string().data;

        redraw_titles();
        ml_setflags(buf);
        if get_fileformat(&*buf) == EOL_MAC || *oldval == b'm' as c_char {
            redraw_buf_later(buf, UPD_NOT_VALID);
        }
        ptr::null()
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of fileformat options.
pub fn get_fileformat_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 {
        return ptr::null_mut();
    }
    opt_ff_values
        .get(idx as usize)
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
}

/// Validate the 'filetype', 'syntax' or 'keymap' option.
pub fn validate_filetype(args: &mut optset_T) -> *const c_char {
    if valid_filetype(args.os_newval.data.string().data) {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// The 'filetype' or the 'syntax' option is changed.
pub fn did_set_filetype_or_syntax(args: &mut optset_T) -> *const c_char {
    unsafe {
        let varp = args.os_varp.cast::<*mut c_char>();
        args.os_value_changed = crate::memory::cstr_as_bytes(args.os_oldval.data.string().data) != crate::memory::cstr_as_bytes(*varp);
        args.os_value_checked = true;
        ptr::null()
    }
}

/// The 'foldexpr' option is changed.
pub fn did_set_foldexpr(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if foldmethodIsExpr(&mut *win) {
            foldUpdateAll(&mut *win);
        }
        ptr::null()
    }
}

/// The 'foldignore' option is changed.
pub fn did_set_foldignore(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if foldmethodIsIndent(&mut *win) {
            foldUpdateAll(&mut *win);
        }
        ptr::null()
    }
}

/// Validate the 'foldmarker' option.
pub fn validate_foldmarker(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        let p = vim_strchr(value, b',' as c_int);
        if p.is_null() {
            return E_COMMA_REQUIRED.as_ptr();
        }
        if p == value || *p.add(1) == NUL {
            e_invarg.as_ptr()
        } else {
            ptr::null()
        }
    }
}

/// The 'foldmarker' option is changed.
pub fn did_set_foldmarker(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if foldmethodIsMarker(&mut *win) {
            foldUpdateAll(&mut *win);
        }
        ptr::null()
    }
}

/// The 'foldmethod' option is changed.
pub fn did_set_foldmethod(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        foldUpdateAll(&mut *win);
        if foldmethodIsDiff(&mut *win) {
            newFoldLevel();
        }
        ptr::null()
    }
}

/// Validate the 'formatoptions' option.
pub fn validate_formatoptions(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            FO_ALL.as_ptr(),
            args.os_errbuf,
        )
}

pub fn expand_set_formatoptions(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, FO_ALL.as_ptr(), matches)
}

/// The 'guicursor' option is changed.
pub fn did_set_guicursor(_args: &mut optset_T) -> *const c_char {
    unsafe {
        let errmsg = parse_shape_opt(SHAPE_CURSOR);
        if !errmsg.is_null() {
            return errmsg;
        }
        if g_st().Visual.active {
            redrawWinline(*curwin.as_ptr(), (**curwin.as_ptr()).w_cursor.lnum);
        }
        ptr::null()
    }
}

/// The 'helpfile' option is changed.
pub fn did_set_helpfile(_args: &mut optset_T) -> *const c_char {
    if g_st().didset_vim {
            vim_unsetenv_ext(c"VIM".as_ptr());
        }
        if g_st().didset_vimruntime {
            vim_unsetenv_ext(c"VIMRUNTIME".as_ptr());
        }
        ptr::null()
}

/// Validate the 'helplang' option.
pub fn validate_helplang(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut s = args.os_newval.data.string().data;
        while *s != NUL {
            if *s.add(1) == NUL
                || ((*s.add(2) != b',' as c_char || *s.add(3) == NUL) && *s.add(2) != NUL)
            {
                return e_invarg.as_ptr();
            }
            if *s.add(2) == NUL {
                break;
            }
            s = s.add(3);
        }
        ptr::null()
    }
}

/// Validate the 'highlight' option.
pub fn validate_highlight(args: &mut optset_T) -> *const c_char {
     {
        if crate::memory::cstr_as_bytes(args.os_newval.data.string().data) != crate::memory::cstr_as_bytes(HIGHLIGHT_INIT.as_ptr()) {
            return e_unsupportedoption.as_ptr();
        }
        ptr::null()
    }
}

/// The 'iconstring' option is changed.
pub fn did_set_iconstring(args: &mut optset_T) -> *const c_char {
    did_set_titleiconstring(args, STL_IN_ICON)
}

/// The 'inccommand' option is changed.
pub fn did_set_inccommand(_args: &mut optset_T) -> *const c_char {
    if g_st().cmdpreview {
            return e_invarg.as_ptr();
        }
        ptr::null()
}

/// Validate the 'isident', 'iskeyword' or 'isfname' option.
pub fn validate_isopt(args: &mut optset_T) -> *const c_char {
    if !check_isopt(args.os_newval.data.string().data) {
            e_invarg.as_ptr()
        } else {
            ptr::null()
        }
}

/// The 'iskeyword' option is changed.
pub fn did_set_iskeyword(args: &mut optset_T) -> *const c_char {
    unsafe {
        let varp = args.os_varp.cast::<*mut c_char>();
        if !ptr::eq(varp, &raw mut opt_vars().p_isk) {
            let buf = args.os_buf;
            if !buf_init_isk_chartab(&mut *buf) {
                return e_invarg.as_ptr();
            }
        }
        ptr::null()
    }
}

/// The 'isident', 'iskeyword', 'isprint' or 'isfname' option is changed.
pub fn did_set_isopt(args: &mut optset_T) -> *const c_char {
    let res = if ptr::eq(args.os_varp, (&raw mut opt_vars().p_isf).cast()) {
            init_isf_chartab()
        } else {
            init_isi_chartab()
        };
        if !res {
            e_invarg.as_ptr()
        } else {
            ptr::null()
        }
}

/// The 'keymap' option has changed.
pub fn did_set_keymap(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let opt_flags = args.os_flags;

        let secure_save = g_st().secure;
        g_st().secure = 0;

        let errmsg = keymap_init();

        g_st().secure = secure_save;

        args.os_value_checked = true;

        if errmsg.is_null() {
            if *(*buf).b_p_keymap != NUL {
                (*buf).b_p_iminsert = B_IMODE_LMAP;
                if (*buf).b_p_imsearch != B_IMODE_USE_INSERT {
                    (*buf).b_p_imsearch = B_IMODE_LMAP;
                }
            } else {
                if (*buf).b_p_iminsert == B_IMODE_LMAP {
                    (*buf).b_p_iminsert = B_IMODE_NONE;
                }
                if (*buf).b_p_imsearch == B_IMODE_LMAP {
                    (*buf).b_p_imsearch = B_IMODE_USE_INSERT;
                }
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                set_iminsert_global(&mut *buf);
                set_imsearch_global(&mut *buf);
            }
            status_redraw_buf(buf);
        }

        errmsg
    }
}

/// The 'keymodel' option is changed.
pub fn did_set_keymodel(_args: &mut optset_T) -> *const c_char {
    g_st().km_stopsel = !vim_strchr(opt_vars().p_km, b'o' as c_int).is_null();
        g_st().km_startsel = !vim_strchr(opt_vars().p_km, b'a' as c_int).is_null();
        ptr::null()
}

/// Validate the 'lispoptions' option.
pub fn validate_lispoptions(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        if *value == NUL
            || crate::memory::cstr_as_bytes(value) == b"expr:0"
            || crate::memory::cstr_as_bytes(value) == b"expr:1"
        {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
    }
}

/// Validate the 'matchpairs' option.
pub fn validate_matchpairs(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut p = args.os_newval.data.string().data;
        while *p != NUL {
            let mut x2: c_int = -1;
            let mut x3: c_int = -1;

            p = p.add(utfc_ptr2len(p) as usize);
            if *p != NUL {
                x2 = *p as u8 as c_int;
                p = p.add(1);
            }
            if *p != NUL {
                x3 = utf_ptr2char(p);
                p = p.add(utfc_ptr2len(p) as usize);
            }
            if x2 != b':' as c_int || x3 == -1 || (*p != NUL && *p != b',' as c_char) {
                return e_invarg.as_ptr();
            }
            if *p == NUL {
                break;
            }
            p = p.add(1);
        }
        ptr::null()
    }
}

/// Validate the 'mkspellmem' option.
pub fn validate_mkspellmem(args: &mut optset_T) -> *const c_char {
    if spell_check_msm(args.os_newval.data.string().data, false) {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// Validate the 'mouse' option.
pub fn validate_mouse(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            MOUSE_ALL.as_ptr(),
            args.os_errbuf,
        )
}

pub fn expand_set_mouse(args: &mut optexpand_T, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, MOUSE_ALL.as_ptr(), matches)
}

/// Validate the 'mousescroll' option.
pub fn validate_mousescroll(args: &mut optset_T) -> *const c_char {
    unsafe {
        let v = opt_keyset(
            args.os_newval.data.string().data,
            kOptMousescroll,
            ptr::null_mut(),
        )
        .cast::<OptKeyDict_mousescroll>();
        let has_hor = ((*v).is_set__mousescroll_ & (1 << KEYSET_OPTIDX_mousescroll__hor)) != 0;
        let has_ver = ((*v).is_set__mousescroll_ & (1 << KEYSET_OPTIDX_mousescroll__ver)) != 0;
        if has_hor || has_ver {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
    }
}

/// Handle setting 'mousescroll'.
pub fn did_set_mousescroll(_args: &mut optset_T) -> *const c_char {
    unsafe {
        let v = opt_keyset(opt_vars().p_mousescroll, kOptMousescroll, ptr::null_mut())
            .cast::<OptKeyDict_mousescroll>();
        let has_hor = ((*v).is_set__mousescroll_ & (1 << KEYSET_OPTIDX_mousescroll__hor)) != 0;
        let has_ver = ((*v).is_set__mousescroll_ & (1 << KEYSET_OPTIDX_mousescroll__ver)) != 0;
        opt_vars().p_mousescroll_hor = if has_hor {
            i64::from((*v).hor as c_int)
        } else {
            MOUSESCROLL_HOR_DFLT
        };
        opt_vars().p_mousescroll_vert = if has_ver {
            i64::from((*v).ver as c_int)
        } else {
            MOUSESCROLL_VERT_DFLT
        };
        ptr::null()
    }
}

/// The 'rulerformat' option is changed.
pub fn did_set_rulerformat(args: &mut optset_T) -> *const c_char {
    did_set_statustabline_rulerformat(args, true, false)
}

/// The 'selection' option is changed.
pub fn did_set_selection(_args: &mut optset_T) -> *const c_char {
    if g_st().Visual.active {
            redraw_curbuf_later(UPD_INVERTED);
        }
        ptr::null()
}

/// Validate the 'sessionoptions' option.
pub fn validate_sessionoptions(args: &mut optset_T) -> *const c_char {
    let mut flags: c_uint = 0;
        let errmsg = opt_strings_flags(
            args.os_newval.data.string().data,
            &opt_ssop_values,
            &raw mut flags,
            true,
            args.os_errbuf,
        );
        if !errmsg.is_null() {
            return errmsg;
        }
        if (flags & kOptSsopFlagCurdir) != 0 && (flags & kOptSsopFlagSesdir) != 0 {
            e_invarg.as_ptr()
        } else {
            ptr::null()
        }
}

/// Validate the 'shada' option.
pub fn validate_shada(args: &mut optset_T) -> *const c_char {
    unsafe {
        let errbuf = args.os_errbuf;
        let value = args.os_newval.data.string().data;
        let mut marks: *mut c_char = ptr::null_mut();

        let mut s = value;
        while *s != NUL {
            if *s == b'\'' as c_char && marks.is_null() {
                marks = s.add(1);
            }
            if vim_strchr(c"!\"%'/:<@cfhnrs", *s as u8 as c_int).is_null() {
                return illegal_char(errbuf, *s as u8 as c_int);
            }
            if *s == b'n' as c_char {
                break;
            } else if *s == b'r' as c_char {
                loop {
                    s = s.add(1);
                    if *s == NUL || *s == b',' as c_char {
                        break;
                    }
                }
            } else if *s == b'%' as c_char {
                loop {
                    s = s.add(1);
                    if !ascii_isdigit(*s as c_int) {
                        break;
                    }
                }
            } else if *s == b'!' as c_char || *s == b'h' as c_char || *s == b'c' as c_char {
                s = s.add(1);
            } else {
                loop {
                    s = s.add(1);
                    if !ascii_isdigit(*s as c_int) {
                        break;
                    }
                }

                if !ascii_isdigit(*s.sub(1) as c_int) {
                    if errbuf.is_some() {
                        return opt_error(
                            errbuf,
                            c"E526: Missing number after <%s>".as_ptr(),
                            transchar_byte(c_int::from(*s.sub(1) as u8)),
                        );
                    } else {
                        return c"".as_ptr();
                    }
                }
            }
            if *s == b',' as c_char {
                s = s.add(1);
            } else if *s != NUL {
                if errbuf.is_some() {
                    return c"E527: Missing comma".as_ptr();
                } else {
                    return c"".as_ptr();
                }
            }
        }
        if *value != NUL && (marks.is_null() || (crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(marks)) as c_int) < 0) {
            return c"E528: Must specify a ' value".as_ptr();
        }
        ptr::null()
    }
}

/// Validate 'shellpipe'/'shellredir' option.
pub fn validate_shellpipe_redir(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut seen = false;

        let mut p = args.os_newval.data.string().data;
        while *p != NUL {
            if *p != b'%' as c_char {
                p = p.add(1);
                continue;
            }
            if *p.add(1) == NUL {
                return e_invalid_format_string_single_percent_s.as_ptr();
            }
            if *p.add(1) == b'%' as c_char {
                p = p.add(2);
                continue;
            }
            if *p.add(1) == b's' as c_char {
                if seen {
                    return e_invalid_format_string_single_percent_s.as_ptr();
                }
                seen = true;
                p = p.add(2);
                continue;
            }
            return e_invalid_format_string_single_percent_s.as_ptr();
        }
        ptr::null()
    }
}

pub fn expand_set_shortmess(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, SHM_ALL.as_ptr(), matches)
}

/// Validate the 'shortmess' option.
pub fn validate_shortmess(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            SHM_ALL.as_ptr(),
            args.os_errbuf,
        )
}

/// Validate the 'showbreak' option.
pub fn validate_showbreak(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut s = args.os_newval.data.string().data;
        while *s != NUL {
            if ptr2cells(s) != 1 {
                return E_SHOWBREAK_CONTAINS_UNPRINTABLE_OR_WIDE_CHARACTER.as_ptr();
            }
            s = s.add(utfc_ptr2len(s) as usize);
        }
        ptr::null()
    }
}

/// The 'showcmdloc' option is changed.
pub fn did_set_showcmdloc(_args: &mut optset_T) -> *const c_char {
    comp_col();
        ptr::null()
}

pub fn validate_signcolumn(args: &mut optset_T) -> *const c_char {
    if check_signcolumn(args.os_newval.data.string().data, ptr::null_mut()) {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// The 'signcolumn' option is changed.
pub fn did_set_signcolumn(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let varp = args.os_varp.cast::<*mut c_char>();
        let oldval = args.os_oldval.data.string().data;
        if !check_signcolumn(
            *varp,
            if ptr::eq(varp, &raw mut (*win).w_onebuf_opt.wo_scl) {
                win
            } else {
                ptr::null_mut()
            },
        )
        {
            return e_invarg.as_ptr();
        }
        if (*oldval == b'n' as c_char && *oldval.add(1) == b'u' as c_char)
            || (*win).w_minscwidth == SCL_NUM
        {
            (*win).w_nrwidth_line_count = 0;
        }
        ptr::null()
    }
}

/// Validate 'spellcapcheck' and prepare its regexp program.
pub fn validate_spellcapcheck(args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut prog: Option<Box<regprog_T>> = None;
        let mut err = Error::new();
        let save_emsg_silent = g_st().emsg_silent;
        let save_suppress_errthrow = g_st().suppress_errthrow;
        g_st().emsg_silent = 0;
        g_st().suppress_errthrow = false;

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let mut errmsg = compile_cap_prog(args.os_newval.data.string().data, &mut prog);
        try_leave(&tstate, &mut err);

        g_st().emsg_silent = save_emsg_silent;
        g_st().suppress_errthrow = save_suppress_errthrow;
        args.os_prep = OptSetPrep::SpellcapProg(prog);
        if err.err_type != ErrorType::kErrorTypeNone {
            let msg = if crate::memory::cstr_prefix_bytes(err.msg_ptr(), 4) == &b"Vim:"[..4] {
                err.msg_ptr().add(4)
            } else {
                err.msg_ptr()
            };
            errmsg = opt_error(args.os_errbuf, c"%s".as_ptr(), msg);
            api_clear_error(&mut err);
            return errmsg;
        }
        errmsg
    }
}

/// The 'spellcapcheck' option is changed.
pub fn did_set_spellcapcheck(args: &mut optset_T) -> *const c_char {
    unsafe {
        if ptr::eq(args.os_varp, (&raw mut opt_vars().p_spc).cast()) {
            return ptr::null();
        }
        let win = args.os_win;
        let ws = (*win).w_s_ptr();
        (*ws).b_cap_prog = args.os_prep.take_spellcap_prog();
        ptr::null()
    }
}

/// Validate the 'spellfile' option.
pub fn validate_spellfile(args: &mut optset_T) -> *const c_char {
    if valid_spellfile(args.os_newval.data.string().data) {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// The 'spellfile' option is changed.
pub fn did_set_spellfile(_args: &mut optset_T) -> *const c_char {
    did_set_spell_option()
}

/// Validate the 'spelllang' option.
pub fn validate_spelllang(args: &mut optset_T) -> *const c_char {
    if valid_spelllang(args.os_newval.data.string().data) {
            ptr::null()
        } else {
            e_invarg.as_ptr()
        }
}

/// The 'spelllang' option is changed.
pub fn did_set_spelllang(_args: &mut optset_T) -> *const c_char {
    did_set_spell_option()
}

/// The 'spelloptions' option is changed.
pub fn did_set_spelloptions(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let opt_flags = args.os_flags;
        let val = args.os_newval.data.string().data;

        if (opt_flags & OPT_LOCAL) == 0 {
            let errmsg = opt_strings_flags(
                val,
                &opt_spo_values,
                &raw mut opt_vars().spo_flags,
                true,
                args.os_errbuf,
            );
            if !errmsg.is_null() {
                return errmsg;
            }
        }
        if (opt_flags & OPT_GLOBAL) == 0 {
            let ws = (*win).w_s_ptr();
            let errmsg = opt_strings_flags(
                val,
                &opt_spo_values,
                &raw mut (*ws).b_p_spo_flags,
                true,
                args.os_errbuf,
            );
            if !errmsg.is_null() {
                return errmsg;
            }
        }
        ptr::null()
    }
}

/// Validate the 'spellsuggest' option.
pub fn validate_spellsuggest(args: &mut optset_T) -> *const c_char {
    let mut sps: [c_int; 2] = [0, 0];
    let [s0, s1] = &mut sps;
    if !spell_check_sps(
        args.os_newval.data.string().data,
        s0,
        s1,
    )
    {
        return e_invarg.as_ptr();
    }
    args.os_prep = OptSetPrep::Spellsuggest(sps);
    ptr::null()
}

/// The 'spellsuggest' option is changed.
pub fn did_set_spellsuggest(args: &mut optset_T) -> *const c_char {
    if let OptSetPrep::Spellsuggest(sps) = args.os_prep {
            spell_apply_sps(sps[0], sps[1]);
        }
        ptr::null()
}

pub fn did_set_splitkeep(_args: &mut optset_T) -> *const c_char {
    unsafe {
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                (*wp).w_prev_height = (*wp).w_height;
                (*wp).w_prev_winrow = (*wp).w_winrow;
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        ptr::null()
    }
}

pub fn validate_statustabline_rulerformat(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        if *value == b'%' as c_char && *value.add(1) == b'!' as c_char {
            ptr::null()
        } else {
            check_stl_option(value, args.os_errbuf)
        }
    }
}

/// The 'statuscolumn' option is changed.
pub fn did_set_statuscolumn(args: &mut optset_T) -> *const c_char {
    did_set_statustabline_rulerformat(args, false, true)
}

/// The 'statusline' option is changed.
pub fn did_set_statusline(args: &mut optset_T) -> *const c_char {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'statusline', 'winbar', 'tabline', 'rulerformat' or 'statuscolumn' option is changed.
fn did_set_statustabline_rulerformat(
    args: &mut optset_T,
    rulerformat: bool,
    statuscolumn: bool,
) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let varp = args.os_varp.cast::<*mut c_char>();
        if rulerformat {
            g_st().ru_wid = 0;
        } else if statuscolumn {
            (*win).w_nrwidth_line_count = 0;
        }
        let mut s = *varp;
        let is_stl = args.os_idx == kOptStatusline;

        if (is_stl || rulerformat)
            && ((args.os_flags & OPT_GLOBAL) != 0 || (args.os_flags & OPT_LOCAL) == 0)
            && *s == NUL
        {
            free_string_option(*varp);
            let def = get_option_default(args.os_idx, args.os_flags);
            *varp = alloc_string_option(def.data.string().data);
            optval_free_read(args.os_idx, def);
            s = *varp;
        }

        if is_stl && !win.is_null() && (*win).w_floating {
            win_config_float(&mut *win, (*win).w_config.clone());
        }

        if rulerformat && !ui_has(kUIMessages) && *s == b'%' as c_char {
            s = s.add(1);
            if *s == b'-' as c_char {
                s = s.add(1);
            }
            let wid = getdigits_int(&raw mut s, true, 0);
            if wid != 0 && *s == b'(' as c_char {
                g_st().ru_wid = wid;
            }
        }
        if rulerformat {
            comp_col();
        }
        ptr::null()
    }
}

/// The 'tabline' option is changed.
pub fn did_set_tabline(args: &mut optset_T) -> *const c_char {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'tagcase' option is changed.
pub fn did_set_tagcase(args: &mut optset_T) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let opt_flags = args.os_flags;

        let (p, flags) = if (opt_flags & OPT_LOCAL) != 0 {
            ((*buf).b_p_tc, &raw mut (*buf).b_tc_flags)
        } else {
            (opt_vars().p_tc, &raw mut opt_vars().tc_flags)
        };

        if (opt_flags & OPT_LOCAL) != 0 && *p == NUL {
            *flags = 0;
        } else {
            let errmsg = opt_strings_flags(
                p,
                &opt_tc_values,
                flags,
                false,
                args.os_errbuf,
            );
            if !errmsg.is_null() {
                return errmsg;
            }
        }
        ptr::null()
    }
}

/// The 'titlestring' or the 'iconstring' option is changed.
fn did_set_titleiconstring(args: &mut optset_T, flagval: c_int) -> *const c_char {
    unsafe {
        let varp = args.os_varp.cast::<*mut c_char>();

        if !vim_strchr(*varp, b'%' as c_int).is_null()
            && check_stl_option(*varp, None).is_null()
        {
            g_st().stl_syntax |= flagval;
        } else {
            g_st().stl_syntax &= !flagval;
        }
        did_set_title();

        ptr::null()
    }
}

/// The 'titlestring' option is changed.
pub fn did_set_titlestring(args: &mut optset_T) -> *const c_char {
    did_set_titleiconstring(args, STL_IN_TITLE)
}

/// Validate 'vartabstop' or 'varsofttabstop' and prepare its tab widths.
pub fn validate_vartabstop(args: &mut optset_T) -> *const c_char {
    unsafe {
        let value = args.os_newval.data.string().data;
        let mut cp = value;
        while *cp != NUL {
            if ascii_isdigit(*cp as c_int) {
                cp = cp.add(1);
                continue;
            }
            if *cp == b',' as c_char
                && (cp as usize) > (value as usize)
                && *cp.sub(1) != b',' as c_char
            {
                cp = cp.add(1);
                continue;
            }
            return e_invarg.as_ptr();
        }

        let mut array: Vec<colnr_T> = Vec::new();
        let errmsg = tabstop_set(value, &mut array, args.os_errbuf);
        args.os_prep = OptSetPrep::Tabstop(array);
        errmsg
    }
}

/// The 'vartabstop' or 'varsofttabstop' option is changed.
pub fn did_set_vartabstop(args: &mut optset_T) -> *const c_char {
    unsafe {
        if ptr::eq(args.os_varp, (&raw mut opt_vars().p_vts).cast())
            || ptr::eq(args.os_varp, (&raw mut opt_vars().p_vsts).cast())
        {
            return ptr::null();
        }
        let buf = args.os_buf;
        let win = args.os_win;
        let soft = args.os_idx == kOptVarsofttabstop;
        let array = if soft {
            &mut (*buf).b_p_vsts_array
        } else {
            &mut (*buf).b_p_vts_array
        };
        *array = args.os_prep.take_tabstop();
        if !soft && foldmethodIsIndent(&mut *win) {
            foldUpdateAll(&mut *win);
        }
        ptr::null()
    }
}

/// The 'verbosefile' option is changed.
pub fn did_set_verbosefile(_args: &mut optset_T) -> *const c_char {
    unsafe {
        verbose_stop();
        if *opt_vars().p_vfile != NUL && !verbose_open() {
            return e_invarg.as_ptr();
        }
        ptr::null()
    }
}

/// The 'virtualedit' option is changed.
pub fn did_set_virtualedit(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;

        let mut ve = opt_vars().p_ve;
        let mut flags = &raw mut opt_vars().ve_flags;

        if (args.os_flags & OPT_LOCAL) != 0 {
            ve = (*win).w_onebuf_opt.wo_ve;
            flags = &raw mut (*win).w_onebuf_opt.wo_ve_flags;
        }

        if (args.os_flags & OPT_LOCAL) != 0 && *ve == NUL {
            *flags = 0;
        } else {
            let errmsg = opt_strings_flags(
                ve,
                &opt_ve_values,
                flags,
                true,
                args.os_errbuf,
            );
            if !errmsg.is_null() {
                return errmsg;
            } else if crate::memory::cstr_as_bytes(ve) != crate::memory::cstr_as_bytes(args.os_oldval.data.string().data) {
                validate_virtcol(win);
                coladvance(&mut *win, (*win).w_virtcol);
            }
        }
        ptr::null()
    }
}

/// Validate the 'whichwrap' option.
pub fn validate_whichwrap(args: &mut optset_T) -> *const c_char {
    check_option_listflag(
            args.os_newval.data.string().data,
            WW_ALL_COMMA.as_ptr(),
            args.os_errbuf,
        )
}

pub fn expand_set_whichwrap(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, WW_ALL.as_ptr(), matches)
}

pub fn validate_wildmode(args: &mut optset_T) -> *const c_char {
    let mut flags = [0u8; 4];
        if !check_opt_wim(args.os_newval.data.string().data, flags.as_mut_ptr()) {
            return e_invarg.as_ptr();
        }
        args.os_prep = OptSetPrep::Wildmode(flags);
        ptr::null()
}

/// The 'wildmode' option is changed.
pub fn did_set_wildmode(args: &mut optset_T) -> *const c_char {
    if let OptSetPrep::Wildmode(flags) = args.os_prep {
            g_st().wim_flags = flags;
        }
        ptr::null()
}

/// The 'winbar' option is changed.
pub fn did_set_winbar(args: &mut optset_T) -> *const c_char {
    did_set_statustabline_rulerformat(args, false, false)
}

/// Validate the 'winborder' or 'pumborder' option.
pub fn validate_border(args: &mut optset_T) -> *const c_char {
    let mut fconfig = WIN_CONFIG_INIT;
        let mut err = Error::new();
        let ok = parse_winborder(
            Some(&mut fconfig),
            args.os_newval.data.string().data,
            &mut err,
        );
        api_clear_error(&mut err);
        if ok { ptr::null() } else { e_invarg.as_ptr() }
}

/// The 'winhighlight' option is changed.
pub fn did_set_winhighlight(args: &mut optset_T) -> *const c_char {
    unsafe {
        let win = args.os_win;
        let varp = args.os_varp.cast::<*mut c_char>();
        if !parse_winhl_opt(
            *varp,
            if ptr::eq(varp, &raw mut (*win).w_onebuf_opt.wo_winhl) {
                win
            } else {
                ptr::null_mut()
            },
        ) {
            return e_invarg.as_ptr();
        }
        ptr::null()
    }
}

pub fn expand_set_winhighlight(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_generic(args, Some(get_highlight_name), matches)
}

/// Format an error naming an item (at most 63 bytes) and listing its valid values.
fn opt_values_err(
    errbuf: Option<CharBuf>,
    fmt: &std::ffi::CStr,
    item: *const c_char,
    itemlen: usize,
    values: &[&CStr],
) -> *const c_char {
    unsafe {
        let Some(errbuf) = errbuf else {
            return e_invarg.as_ptr();
        };
        let mut itembuf = [0 as c_char; 64];
        crate::memory::cstr_ncpy(itembuf.as_mut_ptr(), item, itemlen.min(itembuf.len() - 1));
        let mut n = vim_snprintf!(
            errbuf.data,
            errbuf.size,
            gettext(fmt.as_ptr()),
            itembuf.as_ptr(),
        );
        for (j, &v) in values.iter().enumerate() {
            if n <= 0 || (n as usize) >= errbuf.size {
                break;
            }
            n += vim_snprintf!(
                errbuf.data.add(n as usize),
                errbuf.size - (n as usize),
                c"%s%s",
                if j == 0 {
                    c" ".as_ptr()
                } else {
                    c", ".as_ptr()
                },
                v.as_ptr(),
            );
        }
        errbuf.data
    }
}

/// Handle an option that can be a range of string values.
fn opt_strings_flags(
    mut val: *const c_char,
    values: &[&CStr],
    flagp: *mut c_uint,
    list: bool,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let mut new_flags: c_uint = 0;
        let iter_one = (*val == NUL) && !list;

        while *val != NUL || iter_one {
            let mut matched = false;
            for (i, &v) in values.iter().enumerate() {
                let len = v.to_bytes().len();
                if (crate::memory::cstr_prefix_bytes(v.as_ptr(), len) == crate::memory::cstr_prefix_bytes(val, len))
                    && ((list && *val.add(len) == b',' as c_char) || *val.add(len) == NUL)
                {
                    val = val.add(len + usize::from(*val.add(len) == b',' as c_char));
                    debug_assert!(i < std::mem::size_of::<c_uint>() * 8);
                    new_flags |= 1u32 << i;
                    matched = true;
                    break;
                }
            }
            if !matched {
                return opt_values_err(
                    errbuf,
                    c"E474: Invalid value '%s', expected one of:",
                    val,
                    crate::memory::cstr_as_bytes(val).iter().position(|&b| b == b',').unwrap_or_else(|| crate::memory::cstr_len(val)),
                    values,
                );
            }
            if iter_one {
                break;
            }
        }
        if !flagp.is_null() {
            *flagp = new_flags;
        }

        ptr::null()
    }
}

/// Format a schema-validation error naming the offending key.
fn opt_schema_err(
    errbuf: Option<CharBuf>,
    fmt: &std::ffi::CStr,
    key: *const c_char,
    keylen: usize,
) -> *const c_char {
     {
        if errbuf.is_none() {
            return e_invarg.as_ptr();
        }
        let mut keybuf = [0 as c_char; 64];
        crate::memory::cstr_ncpy(keybuf.as_mut_ptr(), key, keylen.min(keybuf.len() - 1));
        opt_error(errbuf, fmt.as_ptr(), keybuf.as_ptr())
    }
}

/// Validates a dict option against its schema (options.lua).
pub fn opt_strings_check(
    val: *const c_char,
    schema: &[OptSchemaItem],
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let mut key: *const c_char = ptr::null();
        let mut v: *const c_char = ptr::null();
        let mut keylen: usize = 0;
        let mut vlen: usize = 0;
        let mut p = val;
        while option_next_keyval(
            &raw mut p,
            &raw mut key,
            &mut keylen,
            &raw mut v,
            &mut vlen,
        ) {
            if keylen == 0 && v.is_null() {
                continue;
            }
            let Some(it) = schema
                .iter()
                .find(|item| option_slice_eq(key, keylen, item.name))
            else {
                return opt_schema_err(errbuf, c"E474: Unknown item '%s'", key, keylen);
            };
            match it.kind {
                OptSchemaKind::kOptSchemaFlag => {
                    if !v.is_null() {
                        return opt_schema_err(
                            errbuf,
                            c"E474: '%s' does not take a value",
                            key,
                            keylen,
                        );
                    }
                }
                OptSchemaKind::kOptSchemaNum | OptSchemaKind::kOptSchemaSNum => {
                    let mut i: usize = if !v.is_null()
                        && it.kind == OptSchemaKind::kOptSchemaSNum
                        && *v == b'-' as c_char
                    {
                        1
                    } else {
                        0
                    };
                    let mut ok = !v.is_null() && vlen > i;
                    while ok && i < vlen {
                        ok = ascii_isdigit(*v.add(i) as u8 as c_int);
                        i += 1;
                    }
                    if !ok {
                        return opt_schema_err(
                            errbuf,
                            c"E474: '%s' requires a number",
                            key,
                            keylen,
                        );
                    }
                    let mut end = v.cast_mut();
                    let mut n: i64 = 0;
                    if !try_getdigits(&raw mut end, &mut n)
                        || n < i64::from(c_int::MIN)
                        || n > i64::from(c_int::MAX)
                    {
                        return opt_schema_err(
                            errbuf,
                            c"E474: '%s' number is out of range",
                            key,
                            keylen,
                        );
                    }
                }
                OptSchemaKind::kOptSchemaEnum => {
                    if !v.is_null()
                        && it
                            .enum_values
                            .iter()
                            .any(|&ev| option_slice_eq(v, vlen, ev))
                    {
                        continue;
                    }
                    return opt_values_err(
                        errbuf,
                        c"E474: '%s' must be one of:",
                        key,
                        keylen,
                        it.enum_values,
                    );
                }
                OptSchemaKind::kOptSchemaStr => {
                    if v.is_null() {
                        return opt_schema_err(
                            errbuf,
                            c"E474: '%s' requires a value",
                            key,
                            keylen,
                        );
                    }
                    if vlen >= 256 {
                        return opt_schema_err(
                            errbuf,
                            c"E474: '%s' value is too long",
                            key,
                            keylen,
                        );
                    }
                }
            }
        }
        ptr::null()
    }
}

/// Parses a validated dict option into `out`.
pub fn opt_fill(value: *const c_char, get_field: FieldHashfn, out: *mut OptKeySet) {
    unsafe {
        let Some(get_field_fn) = get_field else {
            return;
        };
        let mut key: *const c_char = ptr::null();
        let mut v: *const c_char = ptr::null();
        let mut keylen: usize = 0;
        let mut vlen: usize = 0;
        let mut p = value;
        while option_next_keyval(
            &raw mut p,
            &raw mut key,
            &mut keylen,
            &raw mut v,
            &mut vlen,
        ) {
            let key_bytes = if key.is_null() || keylen == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(key.cast::<u8>(), keylen)
            };
            let Some(f) = get_field_fn(key_bytes) else {
                continue;
            };
            let field = out.cast::<u8>().add(f.ptr_off);
            if f.key_type == ObjectType::kObjectTypeBoolean as c_int {
                *field.cast::<Boolean>() = true;
            } else if f.key_type == ObjectType::kObjectTypeInteger as c_int {
                let mut end = v.cast_mut();
                *field.cast::<Integer>() = Integer::from(getdigits_int(&raw mut end, false, 0));
            } else if f.key_type == ObjectType::kObjectTypeString as c_int {
                ptr::copy_nonoverlapping(v.cast::<u8>(), field, vlen);
                *field.add(vlen) = 0;
            }
            (*out).is_set_ |= 1u64 << (f.opt_index as u32);
        }
    }
}

/// Reify a dict-option value to a keyset (`OptKeyDict_…`).
pub fn opt_keyset(
    value: *const c_char,
    opt_idx: OptIndex,
    mut keyset: *mut OptKeySet,
) -> *mut OptKeySet {
    unsafe {
        if keyset.is_null() {
            keyset = (&raw mut optstr_st().OPT_KEYSET_SHARED).cast::<OptKeySet>();
        }
        let si = opt_dict_schema(opt_idx).unwrap();
        ptr::write_bytes(keyset.cast::<u8>(), 0, si.size);
        if !value.is_null() {
            opt_fill(value, si.get_field, keyset);
        }
        keyset
    }
}

/// Return OK if "p" is a valid fileformat name, FAIL otherwise.
pub fn check_ff_value(p: *mut c_char) -> bool {
    opt_strings_flags(
            p,
            &opt_ff_values,
            ptr::null_mut(),
            false,
            None,
        )
        .is_null()
}

/// Calls mb_cptr2char_adv(p) and returns the character.
fn get_encoded_char_adv(p: *mut *const c_char) -> schar_T {
    unsafe {
        let s = *p;

        if *s == b'\\' as c_char
            && (*s.add(1) == b'x' as c_char
                || *s.add(1) == b'u' as c_char
                || *s.add(1) == b'U' as c_char)
        {
            let mut num: i64 = 0;
            let mut bytes = if *s.add(1) == b'x' as c_char {
                1
            } else if *s.add(1) == b'u' as c_char {
                2
            } else {
                4
            };
            while bytes > 0 {
                *p = (*p).add(2);
                let n = hexhex2nr(*p);
                if n < 0 {
                    return 0;
                }
                num = num * 256 + i64::from(n);
                bytes -= 1;
            }
            *p = (*p).add(2);
            return if char2cells(num as c_int) > 1 {
                0
            } else {
                schar_from_char(num as c_int)
            };
        }

        let ci = utf_ptr2StrCharInfo(s);
        let cli = utf_ClusterInfo(ci);
        *p = cli.next.ptr;
        if cli.cells > 1 {
            0
        } else {
            schar_from_cluster(ci, cli)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharsTarget {
    FcsStl,
    FcsStlnc,
    FcsWbr,
    FcsHoriz,
    FcsHorizup,
    FcsHorizdown,
    FcsVert,
    FcsVertleft,
    FcsVertright,
    FcsVerthoriz,
    FcsFold,
    FcsFoldopen,
    FcsFoldclosed,
    FcsFoldsep,
    FcsFoldinner,
    FcsDiff,
    FcsMsgsep,
    FcsEob,
    FcsLastline,
    FcsTrunc,
    FcsTruncrl,
    LcsEol,
    LcsExt,
    LcsNbsp,
    LcsPrec,
    LcsSpace,
    LcsTab,
    LcsLeadtab,
    LcsLead,
    LcsTrail,
    LcsConceal,
    LcsMultispace,
    LcsLeadmultispace,
}

impl CharsTarget {
    fn slot_mut<'a>(
        self,
        fcs: &'a mut fcs_chars_T,
        lcs: &'a mut lcs_chars_T,
    ) -> Option<&'a mut schar_T> {
        match self {
            Self::FcsStl => Some(&mut fcs.stl),
            Self::FcsStlnc => Some(&mut fcs.stlnc),
            Self::FcsWbr => Some(&mut fcs.wbr),
            Self::FcsHoriz => Some(&mut fcs.horiz),
            Self::FcsHorizup => Some(&mut fcs.horizup),
            Self::FcsHorizdown => Some(&mut fcs.horizdown),
            Self::FcsVert => Some(&mut fcs.vert),
            Self::FcsVertleft => Some(&mut fcs.vertleft),
            Self::FcsVertright => Some(&mut fcs.vertright),
            Self::FcsVerthoriz => Some(&mut fcs.verthoriz),
            Self::FcsFold => Some(&mut fcs.fold),
            Self::FcsFoldopen => Some(&mut fcs.foldopen),
            Self::FcsFoldclosed => Some(&mut fcs.foldclosed),
            Self::FcsFoldsep => Some(&mut fcs.foldsep),
            Self::FcsFoldinner => Some(&mut fcs.foldinner),
            Self::FcsDiff => Some(&mut fcs.diff),
            Self::FcsMsgsep => Some(&mut fcs.msgsep),
            Self::FcsEob => Some(&mut fcs.eob),
            Self::FcsLastline => Some(&mut fcs.lastline),
            Self::FcsTrunc => Some(&mut fcs.trunc),
            Self::FcsTruncrl => Some(&mut fcs.truncrl),
            Self::LcsEol => Some(&mut lcs.eol),
            Self::LcsExt => Some(&mut lcs.ext),
            Self::LcsNbsp => Some(&mut lcs.nbsp),
            Self::LcsPrec => Some(&mut lcs.prec),
            Self::LcsSpace => Some(&mut lcs.space),
            Self::LcsTab => Some(&mut lcs.tab2),
            Self::LcsLeadtab => Some(&mut lcs.leadtab2),
            Self::LcsLead => Some(&mut lcs.lead),
            Self::LcsTrail => Some(&mut lcs.trail),
            Self::LcsConceal => Some(&mut lcs.conceal),
            Self::LcsMultispace | Self::LcsLeadmultispace => None,
        }
    }
}

struct CharsTabEntry {
    target: CharsTarget,
    name: &'static CStr,
    def: Option<&'static CStr>,
    fallback: Option<&'static CStr>,
}

const FCS_TAB: [CharsTabEntry; 21] = [
    CharsTabEntry {
        target: CharsTarget::FcsStl,
        name: c"stl",
        def: Some(c" "),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsStlnc,
        name: c"stlnc",
        def: Some(c" "),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsWbr,
        name: c"wbr",
        def: Some(c" "),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsHoriz,
        name: c"horiz",
        def: Some(c"─"),
        fallback: Some(c"-"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsHorizup,
        name: c"horizup",
        def: Some(c"┴"),
        fallback: Some(c"-"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsHorizdown,
        name: c"horizdown",
        def: Some(c"┬"),
        fallback: Some(c"-"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsVert,
        name: c"vert",
        def: Some(c"│"),
        fallback: Some(c"|"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsVertleft,
        name: c"vertleft",
        def: Some(c"┤"),
        fallback: Some(c"|"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsVertright,
        name: c"vertright",
        def: Some(c"├"),
        fallback: Some(c"|"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsVerthoriz,
        name: c"verthoriz",
        def: Some(c"┼"),
        fallback: Some(c"+"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsFold,
        name: c"fold",
        def: Some(c"·"),
        fallback: Some(c"-"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsFoldopen,
        name: c"foldopen",
        def: Some(c"-"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsFoldclosed,
        name: c"foldclose",
        def: Some(c"+"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsFoldsep,
        name: c"foldsep",
        def: Some(c"│"),
        fallback: Some(c"|"),
    },
    CharsTabEntry {
        target: CharsTarget::FcsFoldinner,
        name: c"foldinner",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsDiff,
        name: c"diff",
        def: Some(c"-"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsMsgsep,
        name: c"msgsep",
        def: Some(c" "),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsEob,
        name: c"eob",
        def: Some(c"~"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsLastline,
        name: c"lastline",
        def: Some(c"@"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsTrunc,
        name: c"trunc",
        def: Some(c">"),
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::FcsTruncrl,
        name: c"truncrl",
        def: Some(c"<"),
        fallback: None,
    },
];

const LCS_TAB: [CharsTabEntry; 12] = [
    CharsTabEntry {
        target: CharsTarget::LcsEol,
        name: c"eol",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsExt,
        name: c"extends",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsNbsp,
        name: c"nbsp",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsPrec,
        name: c"precedes",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsSpace,
        name: c"space",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsTab,
        name: c"tab",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsLeadtab,
        name: c"leadtab",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsLead,
        name: c"lead",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsTrail,
        name: c"trail",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsConceal,
        name: c"conceal",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsMultispace,
        name: c"multispace",
        def: None,
        fallback: None,
    },
    CharsTabEntry {
        target: CharsTarget::LcsLeadmultispace,
        name: c"leadmultispace",
        def: None,
        fallback: None,
    },
];

/// Handle setting 'listchars' or 'fillchars'.
pub fn set_chars_option(
    wp: *mut win_T,
    mut value: *const c_char,
    what: CharsOption,
    apply: bool,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let mut last_multispace: *const c_char = ptr::null();
        let mut last_lmultispace: *const c_char = ptr::null();
        let mut multispace_len: usize = 0;
        let mut lead_multispace_len: usize = 0;
        let mut fcs_chars = fcs_chars_T::default();
        let mut lcs_chars = lcs_chars_T::new();

        let tab: &[CharsTabEntry] = if what == CharsOption::kListchars {
            if apply && *(*wp).w_onebuf_opt.wo_lcs == NUL {
                value = opt_vars().p_lcs;
            }
            &LCS_TAB
        } else {
            if apply && *(*wp).w_onebuf_opt.wo_fcs == NUL {
                value = opt_vars().p_fcs;
            }
            &FCS_TAB
        };
        let entries = tab.len();

        let max_round = if apply { 1 } else { 0 };
        for round in 0..=max_round {
            let mut has_tab = false;
            let mut has_leadtab = false;

            if round > 0 {
                for entry in tab {
                    if let Some(slot) = entry.target.slot_mut(&mut fcs_chars, &mut lcs_chars) {
                        let s_ptr = match entry.def {
                            Some(d) if ptr2cells(d.as_ptr()) == 1 => d.as_ptr(),
                            _ => entry.fallback.map_or(ptr::null(), |f| f.as_ptr()),
                        };
                        *slot = schar_from_str(s_ptr);
                    }
                }

                if what == CharsOption::kListchars {
                    lcs_chars.tab1 = 0;
                    lcs_chars.tab3 = 0;
                    lcs_chars.leadtab1 = 0;
                    lcs_chars.leadtab3 = 0;
                    lcs_chars.multispace = vec![0; multispace_len];
                    lcs_chars.leadmultispace = vec![0; lead_multispace_len];
                }
            }

            let mut p = value;
            while *p != NUL {
                let mut i: usize = 0;
                while i < entries {
                    let name_ptr = tab[i].name.as_ptr();
                    let name_len = tab[i].name.to_bytes().len();
                    if !((crate::memory::cstr_prefix_bytes(p, name_len) == crate::memory::cstr_prefix_bytes(name_ptr, name_len)) && *p.add(name_len) == b':' as c_char)
                    {
                        i += 1;
                        continue;
                    }

                    let mut s = p.add(name_len + 1);

                    if tab[i].target == CharsTarget::LcsMultispace {
                        if round == 0 {
                            last_multispace = p;
                            multispace_len = 0;
                            while *s != NUL && *s != b',' as c_char {
                                let c1 = get_encoded_char_adv(&raw mut s);
                                if c1 == 0 {
                                    return opt_error(
                                        errbuf,
                                        E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR.as_ptr(),
                                        name_ptr,
                                    );
                                }
                                multispace_len += 1;
                            }
                            if multispace_len == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR.as_ptr(),
                                    name_ptr,
                                );
                            }
                        } else {
                            let mut multispace_pos: usize = 0;
                            while *s != NUL && *s != b',' as c_char {
                                let c1 = get_encoded_char_adv(&raw mut s);
                                if p == last_multispace {
                                    lcs_chars.multispace[multispace_pos] = c1;
                                    multispace_pos += 1;
                                }
                            }
                        }
                        p = s;
                        break;
                    }

                    if tab[i].target == CharsTarget::LcsLeadmultispace {
                        if round == 0 {
                            last_lmultispace = p;
                            lead_multispace_len = 0;
                            while *s != NUL && *s != b',' as c_char {
                                let c1 = get_encoded_char_adv(&raw mut s);
                                if c1 == 0 {
                                    return opt_error(
                                        errbuf,
                                        E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR.as_ptr(),
                                        name_ptr,
                                    );
                                }
                                lead_multispace_len += 1;
                            }
                            if lead_multispace_len == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR.as_ptr(),
                                    name_ptr,
                                );
                            }
                        } else {
                            let mut multispace_pos: usize = 0;
                            while *s != NUL && *s != b',' as c_char {
                                let c1 = get_encoded_char_adv(&raw mut s);
                                if p == last_lmultispace {
                                    lcs_chars.leadmultispace[multispace_pos] = c1;
                                    multispace_pos += 1;
                                }
                            }
                        }
                        p = s;
                        break;
                    }

                    if *s == NUL {
                        return opt_error(
                            errbuf,
                            E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR.as_ptr(),
                            name_ptr,
                        );
                    }
                    let c1 = get_encoded_char_adv(&raw mut s);
                    if c1 == 0 {
                        return opt_error(
                            errbuf,
                            E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR.as_ptr(),
                            name_ptr,
                        );
                    }
                    let mut c2: schar_T = 0;
                    let mut c3: schar_T = 0;
                    if matches!(tab[i].target, CharsTarget::LcsTab | CharsTarget::LcsLeadtab) {
                        if *s == NUL {
                            return opt_error(
                                errbuf,
                                E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR.as_ptr(),
                                name_ptr,
                            );
                        }
                        c2 = get_encoded_char_adv(&raw mut s);
                        if c2 == 0 {
                            return opt_error(
                                errbuf,
                                E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR.as_ptr(),
                                name_ptr,
                            );
                        }
                        if !(*s == b',' as c_char || *s == NUL) {
                            c3 = get_encoded_char_adv(&raw mut s);
                            if c3 == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR.as_ptr(),
                                    name_ptr,
                                );
                            }
                        }
                        if tab[i].target == CharsTarget::LcsTab {
                            has_tab = true;
                        } else {
                            has_leadtab = true;
                        }
                    }

                    if *s == b',' as c_char || *s == NUL {
                        if round > 0 {
                            if tab[i].target == CharsTarget::LcsTab {
                                lcs_chars.tab1 = c1;
                                lcs_chars.tab2 = c2;
                                lcs_chars.tab3 = c3;
                            } else if tab[i].target == CharsTarget::LcsLeadtab {
                                lcs_chars.leadtab1 = c1;
                                lcs_chars.leadtab2 = c2;
                                lcs_chars.leadtab3 = c3;
                            } else if let Some(slot) =
                                tab[i].target.slot_mut(&mut fcs_chars, &mut lcs_chars)
                            {
                                *slot = c1;
                            }
                        }
                        p = s;
                        break;
                    } else {
                        return opt_error(
                            errbuf,
                            E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR.as_ptr(),
                            name_ptr,
                        );
                    }
                }

                if i == entries {
                    return e_invarg.as_ptr();
                }

                if *p == b',' as c_char {
                    p = p.add(1);
                }
            }

            if what == CharsOption::kListchars && has_leadtab && !has_tab {
                return e_leadtab_requires_tab.as_ptr();
            }
        }

        if apply {
            if what == CharsOption::kListchars {
                (*wp).w_p_lcs_chars = lcs_chars;
            } else {
                (*wp).w_p_fcs_chars = fcs_chars;
            }
        }

        ptr::null()
    }
}

/// Function given to ExpandGeneric() to obtain possible arguments of the 'fillchars' option.
pub fn get_fillchars_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= FCS_TAB.len() {
        return ptr::null_mut();
    }
    FCS_TAB[idx as usize].name.as_ptr().cast_mut()
}

/// Function given to ExpandGeneric() to obtain possible arguments of the 'listchars' option.
pub fn get_listchars_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= LCS_TAB.len() {
        return ptr::null_mut();
    }
    LCS_TAB[idx as usize].name.as_ptr().cast_mut()
}

/// Check all global and local values of 'listchars' and 'fillchars'.
pub fn check_chars_options() -> *const c_char {
    unsafe {
        if !set_chars_option(*curwin.as_ptr(), opt_vars().p_lcs, CharsOption::kListchars, false, None).is_null() {
            return E_CONFLICTS_WITH_VALUE_OF_LISTCHARS.as_ptr();
        }
        if !set_chars_option(*curwin.as_ptr(), opt_vars().p_fcs, CharsOption::kFillchars, false, None).is_null() {
            return E_CONFLICTS_WITH_VALUE_OF_FILLCHARS.as_ptr();
        }
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == g_st().curtab {
                g_st().firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if !set_chars_option(
                    wp,
                    (*wp).w_onebuf_opt.wo_lcs,
                    CharsOption::kListchars,
                    true,
                    None,
                )
                .is_null()
                {
                    return E_CONFLICTS_WITH_VALUE_OF_LISTCHARS.as_ptr();
                }
                if !set_chars_option(
                    wp,
                    (*wp).w_onebuf_opt.wo_fcs,
                    CharsOption::kFillchars,
                    true,
                    None,
                )
                .is_null()
                {
                    return E_CONFLICTS_WITH_VALUE_OF_FILLCHARS.as_ptr();
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        ptr::null()
    }
}

pub fn validate_previewpopup(args: &mut optset_T) -> *const c_char {
    let mut fconfig = WIN_CONFIG_INIT;
        if !win_previewpopup_config(args.os_newval.data.string().data, &mut fconfig) {
            return e_invarg.as_ptr();
        }
        ptr::null()
}

pub fn expand_set_popupoption(
    args: &mut optexpand_T,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        let xp = args.oe_xp;

        if ((*xp).xp_pattern as usize) > (args.oe_set_arg as usize)
            && *(*xp).xp_pattern.sub(1) == b':' as c_char
        {
            if completing_value_for_subopt(args, c"border") {
                return expand_set_opt_string(
                    args,
                    &opt_pvp_border_values,
                    matches,
                );
            }
            return false;
        }

        expand_set_opt_string(args, &opt_pvp_values, matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ff_value_accepts_valid_and_rejects_invalid() {
        assert!(check_ff_value(c"unix".as_ptr().cast_mut()));
            assert!(check_ff_value(c"dos".as_ptr().cast_mut()));
            assert!(check_ff_value(c"mac".as_ptr().cast_mut()));
            assert!(!check_ff_value(c"".as_ptr().cast_mut()));
            assert!(!check_ff_value(c"foo".as_ptr().cast_mut()));
    }
}
