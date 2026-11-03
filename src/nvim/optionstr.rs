//! String option validation, handling, and completion (`src/nvim/optionstr.c`).

use crate::api::private::helpers::{TryState, api_clear_error, try_enter, try_leave};
use crate::api::private::validate::{Error, ErrorType, NvimString, Object, ObjectData, ObjectType};
use crate::api::win_config::{WIN_CONFIG_INIT, parse_winborder};
use crate::autocmd::{check_ei, get_event_name_no_group};
use crate::buffer::{FileBuf, K_UIMESSAGES, MAXLNUM, STL_IN_ICON, STL_IN_TITLE, modifiable};
use crate::charset::{
    EOL_MAC, K_OPT_DY_FLAG_UHEX, ascii_isdigit, buf_init_isk_chartab, char2cells, check_isopt,
    getdigits_int, hexhex2nr, init_isf_chartab, init_isi_chartab, transchar, transchar_byte,
    try_getdigits,
};
use crate::cmdexpand::{CompleteListItemGetter, expand_generic};
use crate::cursor::{Colnr, CursorPos, coladvance};
use crate::cursor_shape::{SHAPE_CURSOR, parse_shape_opt};
use crate::diff::{K_OPT_DIFFOPT, diffanchors_changed, diffopt_changed};
use crate::digraph::keymap_init;
use crate::drawscreen::{
    UPD_INVERTED, UPD_NOT_VALID, UPD_VALID, comp_col, redraw_all_later, redraw_buf_later,
    redraw_curbuf_later, redraw_later, redraw_winline, status_redraw_buf,
};
use crate::errors::*;
use crate::eval::vars::{do_unlet, get_var_value};
use crate::ex_getln::{B_IMODE_NONE, B_IMODE_USE_INSERT, check_opt_wim};
use crate::fold::{
    fold_update_all, foldmethod_is_diff, foldmethod_is_expr, foldmethod_is_indent,
    foldmethod_is_marker, new_fold_level,
};
use crate::globals::*;
use crate::grid::{Schar, schar_from_char, schar_from_cluster, schar_from_str};
use crate::highlight_group::{get_highlight_name, init_highlight};
use crate::indent::{K_OPT_BREAKINDENTOPT, briopt_check, tabstop_set};
use crate::indent_c::parse_cino;
use crate::insexpand::{LSIZE, set_cpt_callbacks};
use crate::lua::converter::{Boolean, FieldHashfn, KeySetLink, OptKeySet};
use crate::mark::{Fmark, Fmarkv};
use crate::mbyte::{
    enc_canonize, get_encoding_name, utf_cluster_info, utf_ptr2_str_char_info, utfc_ptr2len,
};
use crate::memline::ml_setflags;
use crate::message::{K_OPT_MESSAGESOPT, msg_grid_validate, verbose_open, verbose_stop};
use crate::r#move::validate_virtcol;
use crate::msgpack_rpc::packer::Integer;
use crate::option::{
    K_OPT_BACKUPCOPY, K_OPT_COMMENTS, K_OPT_COMPLETEOPT, K_OPT_EVENTIGNOREWIN, K_OPT_FILEFORMAT,
    K_OPT_FILEFORMATS, K_OPT_FLAG_COMMA, K_OPT_FLAG_NDNAME, K_OPT_FLAG_NFNAME,
    K_OPT_FLAG_ONE_COMMA, K_OPT_LISTCHARS, K_OPT_STATUSLINE, K_OPT_TAGCASE, K_OPT_VARSOFTTABSTOP,
    K_OPT_VIRTUALEDIT, copy_option_part, did_set_title, fill_culopt_flags, get_fileformat,
    get_option, get_option_default, opt_error, option_next_keyval, optval_free_read,
    parse_winhl_opt, redraw_titles, set_iminsert_global, set_imsearch_global, set_option_direct,
    valid_name,
};
use crate::option_vars::*;
use crate::os::env::{ExpandCtx, vim_unsetenv_ext};
use crate::regexp::{Regmatch, Regprog, vim_regexec};
use crate::runtime::Sctx;
use crate::spell::{
    compile_cap_prog, did_set_spell_option, spell_reload, valid_spellfile, valid_spelllang,
};
use crate::spellfile::spell_check_msm;
use crate::spellsuggest::{spell_apply_sps, spell_check_sps};
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::terminal::terminal_notify_theme;
use crate::ui::ui_has;
use crate::window::{FcsChars, LcsChars, WinInfoInfo, check_colorcolumn, global_stl_height};
use crate::winfloat::{win_config_float, win_previewpopup_config};
use std::ffi::{CStr, c_void};
use std::ptr;

pub type OptIndex = i32;
pub type OptScopeFlags = u8;
use crate::highlight::OptionalKeys;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CharsOption {
    Fillchars = 0,
    Listchars = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StlScopeType {
    TopLevel,
    ItemGroup,
    KHighlightScope,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OptSchemaKind {
    OptSchemaFlag = 0,
    OptSchemaNum = 1,
    OptSchemaSNum = 2,
    OptSchemaEnum = 3,
    OptSchemaStr = 4,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharBuf {
    pub data: *mut u8,
    pub size: usize,
}

impl CharBuf {
    #[inline]
    pub fn as_cstr(&self) -> &'static CStr {
        crate::memory::cstr_from_ptr(self.data.cast())
    }
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
    Flags(u32),
    Spellsuggest([i32; 2]),
    Wildmode([u8; 4]),
    Encoding(std::ffi::CString),
    SpellcapProg(Option<Box<Regprog>>),
    Tabstop(Vec<Colnr>),
}

impl OptSetPrep {
    #[inline]
    pub fn as_flags(&self) -> u32 {
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
    pub fn take_spellcap_prog(&mut self) -> Option<Box<Regprog>> {
        match std::mem::replace(self, Self::None) {
            Self::SpellcapProg(p) => p,
            _ => None,
        }
    }

    #[inline]
    pub fn take_tabstop(&mut self) -> Vec<Colnr> {
        match std::mem::replace(self, Self::None) {
            Self::Tabstop(v) => v,
            _ => Vec::new(),
        }
    }
}

#[repr(C)]
pub struct Optset {
    pub os_varp: *mut c_void,
    pub os_idx: OptIndex,
    pub os_flags: i32,
    pub os_oldval: Object,
    pub os_newval: Object,
    pub os_prep: OptSetPrep,
    pub os_value_checked: bool,
    pub os_value_changed: bool,
    pub os_restore_chartab: bool,
    pub os_errbuf: Option<CharBuf>,
    pub os_win: *mut WinInfoInfo,
    pub os_buf: *mut FileBuf,
}

impl Optset {
    pub const fn new() -> Self {
        Self {
            os_varp: ptr::null_mut(),
            os_idx: 0,
            os_flags: 0,
            os_oldval: Object {
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Boolean(false),
            },
            os_newval: Object {
                obj_type: ObjectType::ObjectTypeNil,
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

    #[inline]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        unsafe { &mut *self.os_buf }
    }

    #[inline]
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        unsafe { &mut *self.os_win }
    }

    #[inline]
    pub fn win_opt(&self) -> Option<&'static mut WinInfoInfo> {
        unsafe { self.os_win.as_mut() }
    }

    #[inline]
    pub fn var_cstr_ptr(&self) -> crate::strings::RawCCharPtrMut {
        unsafe { *(self.os_varp as *mut crate::strings::RawCCharPtrMut) }
    }
}

#[repr(C)]
pub struct Optexpand {
    pub oe_varp: *mut u8,
    pub oe_idx: OptIndex,
    pub oe_opt_value: *mut u8,
    pub oe_append: bool,
    pub oe_include_orig_val: bool,
    pub oe_regmatch: *mut Regmatch,
    pub oe_xp: *mut ExpandCtx,
    pub oe_set_arg: *mut u8,
}

impl Optexpand {
    #[inline]
    pub fn regmatch_mut(&self) -> &'static mut Regmatch {
        unsafe { &mut *self.oe_regmatch }
    }

    #[inline]
    pub fn xp_mut(&self) -> &'static mut ExpandCtx {
        unsafe { &mut *self.oe_xp }
    }
}

pub type OptValidateCb = Option<fn(&mut Optset) -> Option<&CStr>>;
pub type OptDidSetCb = Option<fn(&mut Optset) -> Option<&CStr>>;
pub type OptExpandCb = Option<fn(&mut Optexpand, &mut Vec<std::ffi::CString>) -> bool>;

#[repr(C)]
pub struct Vimoption {
    pub fullname: &'static std::ffi::CStr,
    pub shortname: Option<&'static std::ffi::CStr>,
    pub flags: u32,
    pub opt_type: ObjectType,
    pub scope_flags: OptScopeFlags,
    pub var: *mut c_void,
    pub flags_var: *mut u32,
    pub scope_idx: [isize; 4],
    pub immutable: bool,
    pub values: &'static [&'static std::ffi::CStr],
    pub schema: &'static [OptSchemaItem],
    pub opt_validate_cb: OptValidateCb,
    pub opt_did_set_cb: OptDidSetCb,
    pub opt_expand_cb: OptExpandCb,
    pub def_val: Object,
    pub script_ctx: Sctx,
}

use crate::decoration::SCL_NUM;
use crate::ex_cmds::B_IMODE_LMAP;
use crate::ex_session::{K_OPT_SSOP_FLAG_CURDIR, K_OPT_SSOP_FLAG_SESDIR};
use crate::fileio::K_OPT_FILEENCODING;
use crate::globals::IOSIZE;
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::runtime::SID_NONE;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_briopt {
    pub is_set_briopt: OptionalKeys,
    pub shift: Integer,
    pub min: Integer,
    pub sbr: Boolean,
    pub list: Integer,
    pub column: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_dip {
    pub is_set_dip: OptionalKeys,
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
    pub algorithm: [u8; 10],
    pub inline_: [u8; 7],
    pub linematch: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_mopt {
    pub is_set_mopt: OptionalKeys,
    pub hit_enter: Boolean,
    pub history: Integer,
    pub maxheight: Integer,
    pub pager: [u8; 256],
    pub progress: [u8; 2],
    pub timeout: Integer,
    pub wait: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_mousescroll {
    pub is_set_mousescroll: OptionalKeys,
    pub hor: Integer,
    pub ver: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptKeyDict_pvp {
    pub is_set_pvp: OptionalKeys,
    pub height: Integer,
    pub width: Integer,
    pub border: [u8; 8],
}

const _: () = {
    const MOPT_SIZE: usize = std::mem::size_of::<OptKeyDict_mopt>();
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_briopt>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_dip>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_mousescroll>());
    assert!(MOPT_SIZE >= std::mem::size_of::<OptKeyDict_pvp>());
};

const K_OPT_AMBIWIDTH: OptIndex = 2;
const K_OPT_BACKUPEXT: OptIndex = 18;
const K_OPT_BELLOFF: OptIndex = 20;
const K_OPT_CASEMAP: OptIndex = 31;
const K_OPT_CLIPBOARD: OptIndex = 43;
const K_OPT_DISPLAY: OptIndex = 76;
const K_OPT_ENCODING: OptIndex = 80;
const K_OPT_FOLDOPEN: OptIndex = 112;
const K_OPT_JUMPOPTIONS: OptIndex = 158;
const K_OPT_MOUSESCROLL: OptIndex = 203;
const K_OPT_PATCHMODE: OptIndex = 218;
pub(crate) const K_OPT_PREVIEWPOPUP: OptIndex = 222;
const K_OPT_REDRAWDEBUG: OptIndex = 234;
const K_OPT_SESSIONOPTIONS: OptIndex = 257;
const K_OPT_SWITCHBUF: OptIndex = 302;
const K_OPT_TABCLOSE: OptIndex = 305;
const K_OPT_TERMPASTEFILTER: OptIndex = 319;
const K_OPT_VIEWOPTIONS: OptIndex = 346;
const K_OPT_WILDOPTIONS: OptIndex = 357;

pub(crate) const K_OPT_BKC_FLAG_YES: u32 = 0x01;
pub(crate) const K_OPT_BKC_FLAG_AUTO: u32 = 0x02;
const K_OPT_BKC_FLAG_NO: u32 = 0x04;

const KEYSET_OPTIDX_MOUSESCROLL_HOR: u32 = 1;
const KEYSET_OPTIDX_MOUSESCROLL_VER: u32 = 2;
const MOUSESCROLL_VERT_DFLT: i64 = 3;
const MOUSESCROLL_HOR_DFLT: i64 = 6;

const SCL_NO: i32 = -1;

pub(crate) const STL_SEPARATE: u8 = b'=';
pub(crate) const STL_TRUNCMARK: u8 = b'<';
pub(crate) const STL_USER_HL: u8 = b'*';
pub(crate) const STL_HIGHLIGHT: u8 = b'#';

const CPT_ABBR: u32 = 0;
const CPT_KIND: u32 = 1;
const CPT_MENU: u32 = 2;

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
    set_opt_callback_orig_option: *mut u8,
    set_opt_callback_func: CompleteListItemGetter,
    expand_eiw: bool,
    opt_keyset_shared: [u64; std::mem::size_of::<OptKeyDict_mopt>().div_ceil(8)],
}

impl OptionStrState {
    const fn new() -> Self {
        Self {
            set_opt_callback_orig_option: ptr::null_mut(),
            set_opt_callback_func: None,
            expand_eiw: false,
            opt_keyset_shared: [0; std::mem::size_of::<OptKeyDict_mopt>().div_ceil(8)],
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
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"sbr"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, sbr),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"list"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, list),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"shift"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, shift),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"column"),
        ptr_off: std::mem::offset_of!(OptKeyDict_briopt, column),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static DIP_TABLE: [KeySetLink; 20] = [
    KeySetLink {
        str: Some(c"icase"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, icase),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iblank"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iblank),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"filler"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, filler),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"anchor"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, anchor),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"inline"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, inline_),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhite"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhite),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"context"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, context),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"closeoff"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, closeoff),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 8,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"internal"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, internal),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 9,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"vertical"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, vertical),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 10,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"linematch"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, linematch),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 11,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hiddenoff"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, hiddenoff),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 12,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"algorithm"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, algorithm),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 13,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhiteall"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhiteall),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 14,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"iwhiteeol"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, iwhiteeol),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 15,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"foldcolumn"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, foldcolumn),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 16,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"horizontal"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, horizontal),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 17,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"followwrap"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, followwrap),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 18,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"indent-heuristic"),
        ptr_off: std::mem::offset_of!(OptKeyDict_dip, indent_heuristic),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 19,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static MOPT_TABLE: [KeySetLink; 8] = [
    KeySetLink {
        str: Some(c"wait"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, wait),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"pager"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, pager),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"history"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, history),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"timeout"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, timeout),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 4,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"progress"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, progress),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 5,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"hit-enter"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, hit_enter),
        key_type: ObjectType::ObjectTypeBoolean as i32,
        opt_index: 6,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"maxheight"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mopt, maxheight),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 7,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static MOUSESCROLL_TABLE: [KeySetLink; 3] = [
    KeySetLink {
        str: Some(c"hor"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mousescroll, hor),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"ver"),
        ptr_off: std::mem::offset_of!(OptKeyDict_mousescroll, ver),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
        opt_index: -1,
        is_hlgroup: false,
    },
];

static PVP_TABLE: [KeySetLink; 4] = [
    KeySetLink {
        str: Some(c"width"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, width),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 1,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"border"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, border),
        key_type: ObjectType::ObjectTypeString as i32,
        opt_index: 2,
        is_hlgroup: false,
    },
    KeySetLink {
        str: Some(c"height"),
        ptr_off: std::mem::offset_of!(OptKeyDict_pvp, height),
        key_type: ObjectType::ObjectTypeInteger as i32,
        opt_index: 3,
        is_hlgroup: false,
    },
    KeySetLink {
        str: None,
        ptr_off: 0,
        key_type: ObjectType::ObjectTypeNil as i32,
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

fn opt_key_dict_briopt_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
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

fn opt_key_dict_dip_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
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

fn opt_key_dict_mopt_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
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

fn opt_key_dict_mousescroll_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
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

fn opt_key_dict_pvp_get_field(key: &[u8]) -> Option<&'static KeySetLink> {
    let hash = pvp_hash(key);
    (hash >= 0).then(|| &PVP_TABLE[hash as usize])
}

pub(crate) fn opt_dict_schema(opt_idx: OptIndex) -> Option<&'static OptDictSchema> {
    static INFO_BRIOPT: OptDictSchema = OptDictSchema {
        get_field: Some(opt_key_dict_briopt_get_field),
        schema: &OPT_BRIOPT_SCHEMA,
        size: std::mem::size_of::<OptKeyDict_briopt>(),
    };
    static INFO_DIP: OptDictSchema = OptDictSchema {
        get_field: Some(opt_key_dict_dip_get_field),
        schema: &OPT_DIP_SCHEMA,
        size: std::mem::size_of::<OptKeyDict_dip>(),
    };
    static INFO_MOPT: OptDictSchema = OptDictSchema {
        get_field: Some(opt_key_dict_mopt_get_field),
        schema: &OPT_MOPT_SCHEMA,
        size: std::mem::size_of::<OptKeyDict_mopt>(),
    };
    static INFO_MOUSESCROLL: OptDictSchema = OptDictSchema {
        get_field: Some(opt_key_dict_mousescroll_get_field),
        schema: &OPT_MOUSESCROLL_SCHEMA,
        size: std::mem::size_of::<OptKeyDict_mousescroll>(),
    };
    static INFO_PVP: OptDictSchema = OptDictSchema {
        get_field: Some(opt_key_dict_pvp_get_field),
        schema: &OPT_PVP_SCHEMA,
        size: std::mem::size_of::<OptKeyDict_pvp>(),
    };

    match opt_idx {
        K_OPT_BREAKINDENTOPT => Some(&INFO_BRIOPT),
        K_OPT_DIFFOPT => Some(&INFO_DIP),
        K_OPT_MESSAGESOPT => Some(&INFO_MOPT),
        K_OPT_MOUSESCROLL => Some(&INFO_MOUSESCROLL),
        K_OPT_PREVIEWPOPUP => Some(&INFO_PVP),
        _ => None,
    }
}

/// After setting various option values: recompute variables that depend on option values.
pub fn didset_string_options() {
    check_str_opt(K_OPT_CASEMAP, ptr::null_mut(), None);
    check_str_opt(K_OPT_BACKUPCOPY, ptr::null_mut(), None);
    check_str_opt(K_OPT_BELLOFF, ptr::null_mut(), None);
    check_str_opt(K_OPT_COMPLETEOPT, ptr::null_mut(), None);
    check_str_opt(K_OPT_SESSIONOPTIONS, ptr::null_mut(), None);
    check_str_opt(K_OPT_VIEWOPTIONS, ptr::null_mut(), None);
    check_str_opt(K_OPT_FOLDOPEN, ptr::null_mut(), None);
    check_str_opt(K_OPT_DISPLAY, ptr::null_mut(), None);
    check_str_opt(K_OPT_JUMPOPTIONS, ptr::null_mut(), None);
    check_str_opt(K_OPT_REDRAWDEBUG, ptr::null_mut(), None);
    check_str_opt(K_OPT_TAGCASE, ptr::null_mut(), None);
    check_str_opt(K_OPT_TERMPASTEFILTER, ptr::null_mut(), None);
    check_str_opt(K_OPT_VIRTUALEDIT, ptr::null_mut(), None);
    check_str_opt(K_OPT_SWITCHBUF, ptr::null_mut(), None);
    check_str_opt(K_OPT_TABCLOSE, ptr::null_mut(), None);
    check_str_opt(K_OPT_WILDOPTIONS, ptr::null_mut(), None);
    check_str_opt(K_OPT_CLIPBOARD, ptr::null_mut(), None);
}

fn illegal_char<'a>(errbuf: Option<CharBuf>, c: i32) -> Option<&'a CStr> {
    if errbuf.is_none() {
        return Some(c"");
    }
    opt_error(errbuf, c"E539: Illegal character <%s>", transchar(c))
}

/// Check string options in a buffer for NULL value.
pub fn check_buf_options(buf: &mut FileBuf) {
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

pub fn alloc_string_option<R>(s: impl crate::strings::IntoCCharPtr) -> *mut R {
    let s = s.into_c_char_ptr();
    crate::memory::cstr_to_cstring(s).into_raw().cast::<R>()
}

pub fn alloc_string_option_bytes<R>(bytes: &[u8]) -> *mut R {
    crate::memory::cstr_from_bytes(bytes).into_raw().cast::<R>()
}

/// Free the string allocated for an option.
pub fn free_string_option<T>(p: *mut T) {
    unsafe {
        if !p.is_null() && p.cast::<u8>() != EMPTY_STRING_OPTION.as_ptr().cast::<u8>() {
            drop(std::ffi::CString::from_raw(p.cast()));
        }
    }
}

pub fn clear_string_option<T>(pp: *mut *mut T) {
    unsafe {
        let empty = EMPTY_STRING_OPTION.as_ptr().cast::<T>();
        if !(*pp).is_null() && *pp != empty {
            drop(std::ffi::CString::from_raw((*pp).cast()));
        }
        *pp = empty;
    }
}

pub fn check_string_option<T>(pp: *mut *mut T) {
    unsafe {
        if (*pp).is_null() {
            *pp = EMPTY_STRING_OPTION.as_ptr().cast::<T>();
        }
    }
}

/// Return true if "val" is a valid 'filetype' name.
fn valid_filetype(val: &[u8]) -> bool {
    valid_name(val, b".-_")
}

/// Handle setting 'signcolumn' for value 'val'.
pub fn check_signcolumn(
    scl: impl crate::strings::IntoCCharPtr,
    mut wp: Option<&mut WinInfoInfo>,
) -> bool {
    let scl = scl.into_c_char_ptr();
    let mut val = EMPTY_STRING_OPTION.as_ptr().cast_const().cast();
    if !scl.is_null() {
        val = scl;
    } else if wp.is_some() {
        val = wp.as_deref_mut().unwrap().w_onebuf_opt.wo_scl.cast_const();
    }

    let val_bytes = crate::memory::cstr_as_bytes(val);
    if val_bytes.is_empty() {
        return false;
    }

    if opt_strings_flags(val, &OPT_SCL_VALUES, ptr::null_mut(), false, None).is_none() {
        let Some(wp) = wp else {
            return true;
        };
        if val_bytes.starts_with(b"no") {
            wp.w_minscwidth = SCL_NO;
            wp.w_maxscwidth = SCL_NO;
        } else if val_bytes.starts_with(b"nu")
            && (wp.w_onebuf_opt.wo_nu != 0 || wp.w_onebuf_opt.wo_rnu != 0)
        {
            wp.w_minscwidth = SCL_NUM;
            wp.w_maxscwidth = SCL_NUM;
        } else if let Some(after) = val_bytes.strip_prefix(b"yes:") {
            let n = i32::from(after.first().copied().unwrap_or(0)) - (b'0' as i32);
            wp.w_minscwidth = n;
            wp.w_maxscwidth = n;
        } else if val_bytes.first() == Some(&b'y') {
            wp.w_minscwidth = 1;
            wp.w_maxscwidth = 1;
        } else if let Some(after) = val_bytes.strip_prefix(b"auto:") {
            wp.w_minscwidth = 0;
            wp.w_maxscwidth = i32::from(after.first().copied().unwrap_or(0)) - (b'0' as i32);
        } else {
            wp.w_minscwidth = 0;
            wp.w_maxscwidth = 1;
        }
        let scwidth = if wp.w_minscwidth <= 0 {
            0
        } else {
            wp.w_maxscwidth.min(wp.w_scwidth)
        };
        wp.w_scwidth = wp.w_minscwidth.max(scwidth);
    } else {
        if !val_bytes.starts_with(b"auto:")
            || val_bytes.len() != 8
            || !val_bytes[5].is_ascii_digit()
            || val_bytes[6] != b'-'
            || !val_bytes[7].is_ascii_digit()
        {
            return false;
        }
        let min = i32::from(val_bytes[5] - b'0');
        let max = i32::from(val_bytes[7] - b'0');
        if min < 1 || max < 2 || min > 8 || min >= max {
            return false;
        }
        let Some(wp) = wp else {
            return true;
        };
        wp.w_minscwidth = min;
        wp.w_maxscwidth = max;
        let scwidth = if wp.w_minscwidth <= 0 {
            0
        } else {
            wp.w_maxscwidth.min(wp.w_scwidth)
        };
        wp.w_scwidth = wp.w_minscwidth.max(scwidth);
    }
    true
}

/// Check validity of string with the 'statusline' format until the end of the current scope.
fn check_stl_str_adv_scope<'a>(
    s: &mut &[u8],
    errbuf: Option<CharBuf>,
    scope_type: StlScopeType,
) -> Option<&'a CStr> {
    while !s.is_empty() {
        while !s.is_empty() && s[0] != b'%' {
            *s = &s[1..];
        }
        if s.is_empty() {
            break;
        }
        *s = &s[1..];
        let b = s.first().copied().unwrap_or(0);
        if b == b'%' || b == STL_TRUNCMARK || b == STL_SEPARATE {
            *s = &s[1..];
            continue;
        }
        if b == b')' {
            *s = &s[1..];
            return if scope_type == StlScopeType::ItemGroup {
                None
            } else {
                Some(E_UNBALANCED_GROUPS)
            };
        }
        if b == STL_HIGHLIGHT {
            *s = &s[1..];
            if s.first() == Some(&b'(') {
                *s = &s[1..];
                let err = check_stl_str_adv_scope(s, errbuf, StlScopeType::KHighlightScope);
                if err.is_none() {
                    continue;
                } else {
                    return err;
                }
            } else if s.first() == Some(&b')') {
                *s = &s[1..];
                return if scope_type == StlScopeType::KHighlightScope {
                    None
                } else {
                    Some(E_UNBALANCED_GROUPS)
                };
            }
            while !s.is_empty() && s[0] != STL_HIGHLIGHT {
                *s = &s[1..];
            }
            if !s.is_empty() {
                *s = &s[1..];
            }
            continue;
        }
        if s.first() == Some(&b'-') {
            *s = &s[1..];
        }
        while s.first().is_some_and(|&c| ascii_isdigit(c as i32)) {
            *s = &s[1..];
        }
        if s.first() == Some(&STL_USER_HL) {
            continue;
        }
        if s.first() == Some(&b'.') {
            *s = &s[1..];
            while s.first().is_some_and(|&c| ascii_isdigit(c as i32)) {
                *s = &s[1..];
            }
        }
        if s.first() == Some(&b'(') {
            *s = &s[1..];
            let err = check_stl_str_adv_scope(s, errbuf, StlScopeType::ItemGroup);
            if err.is_none() {
                continue;
            } else {
                return err;
            }
        }
        let c = s.first().copied().unwrap_or(0) as i32;
        if !crate::strings::vim_haschr(STL_ALL, c) {
            return illegal_char(errbuf, c);
        }
        if s.first() == Some(&b'{') {
            *s = &s[1..];
            let reevaluate = s.first() == Some(&b'%');

            if reevaluate {
                *s = &s[1..];
                if s.first() == Some(&b'}') {
                    return illegal_char(errbuf, b'}' as i32);
                }
            }
            let mut prev = b'%';
            while !s.is_empty() && (s[0] != b'}' || (reevaluate && prev != b'%')) {
                prev = s[0];
                *s = &s[1..];
            }
            if s.first() != Some(&b'}') {
                return Some(E_UNCLOSED_EXPRESSION_SEQUENCE);
            }
        }
    }
    if scope_type == StlScopeType::TopLevel {
        None
    } else {
        Some(E_UNBALANCED_GROUPS)
    }
}

/// Check validity of options with the 'statusline' format.
pub fn check_stl_option<'a>(
    s: impl crate::strings::IntoCCharPtr,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let mut bytes = crate::memory::cstr_as_bytes(s.into_c_char_ptr());
    check_stl_str_adv_scope(&mut bytes, errbuf, StlScopeType::TopLevel)
}

/// Check for a "normal" directory or file name in some options.
pub fn check_illegal_path_names(val_bytes: &[u8], flags: u32) -> bool {
    let nfname_illegal: &[u8] = if g_st().secure != 0 {
        b"/\\*?[|;&<>\r\n"
    } else {
        b"/\\*?[<>\r\n"
    };
    ((flags & K_OPT_FLAG_NFNAME) != 0 && val_bytes.iter().any(|b| nfname_illegal.contains(b)))
        || ((flags & K_OPT_FLAG_NDNAME) != 0
            && val_bytes.iter().any(|b| b"*?[|;&<>\r\n".contains(b)))
}

fn opt_values(idx: OptIndex) -> &'static [&'static CStr] {
    let idx1 = if idx == K_OPT_VIEWOPTIONS {
        K_OPT_SESSIONOPTIONS
    } else if idx == K_OPT_FILEFORMATS {
        K_OPT_FILEFORMAT
    } else {
        idx
    };

    get_option(idx1).values
}

fn check_str_opt<'a>(
    idx: OptIndex,
    mut varp: *mut *mut u8,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let opt = get_option(idx);
    if varp.is_null() {
        varp = opt.var.cast::<*mut u8>();
    }
    let list = (opt.flags & (K_OPT_FLAG_COMMA | K_OPT_FLAG_ONE_COMMA)) != 0;
    let values = opt_values(idx);
    opt_strings_flags(
        unsafe { (*varp).cast() },
        values,
        opt.flags_var,
        list,
        errbuf,
    )
}

pub fn expand_set_str_generic(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let values = opt_values(args.oe_idx);
    expand_set_opt_string(args, values, matches)
}

/// An option that accepts a list of flags is changed.
pub fn did_set_str_generic(args: &mut Optset) -> Option<&CStr> {
    check_str_opt(args.os_idx, args.os_varp.cast::<*mut u8>(), args.os_errbuf)
}

/// Validate named string values without updating derived flags.
pub fn validate_str_generic(args: &mut Optset) -> Option<&CStr> {
    let opt = get_option(args.os_idx);
    let value = args.os_newval.data.string().data;
    opt_strings_flags(
        value,
        opt_values(args.os_idx),
        ptr::null_mut(),
        (opt.flags & K_OPT_FLAG_COMMA) != 0,
        args.os_errbuf,
    )
}

/// Validate a list of character flags against "flags".
fn check_option_listflag<'a>(
    val: impl crate::strings::IntoCCharPtr,
    flags: impl crate::strings::IntoCCharPtr,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let val = val.into_c_char_ptr();
    let flags = flags.into_c_char_ptr();
    {
        let s = val;
        for &s_byte in crate::memory::cstr_as_bytes(s) {
            if !crate::strings::vim_haschr(flags, s_byte as i32) {
                return illegal_char(errbuf, s_byte as i32);
            }
        }
        None
    }
}

/// Expand an option that accepts a list of string values.
fn expand_set_opt_string(
    args: &mut Optexpand,
    values: &[&CStr],
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let regmatch = args.regmatch_mut();
    let include_orig_val = args.oe_include_orig_val;
    let option_val = args.oe_opt_value.cast();

    matches.clear();

    if include_orig_val && !crate::memory::cstr_is_empty(option_val) {
        matches.push(crate::memory::cstr_to_cstring(option_val));
    }

    for &val in values {
        if val.to_bytes().is_empty()
            || (include_orig_val
                && !crate::memory::cstr_is_empty(option_val)
                && (crate::memory::cstr_as_bytes(val.as_ptr())
                    == crate::memory::cstr_as_bytes(option_val)))
        {
            continue;
        }
        if vim_regexec(regmatch, val.as_ptr(), 0) {
            matches.push(val.to_owned());
        }
    }
    if matches.is_empty() {
        return false;
    }
    true
}

/// Callback used by expand_set_opt_generic to also include the original value.
fn expand_set_opt_callback(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if idx == 0 {
        if !optstr_st().set_opt_callback_orig_option.is_null() {
            return Some(crate::memory::cstr_from_ptr(
                optstr_st().set_opt_callback_orig_option.cast(),
            ));
        } else {
            return Some(c"");
        }
    }
    optstr_st().set_opt_callback_func?(xp, idx - 1)
}

/// Expand an option with a callback that iterates through a list of possible names.
fn expand_set_opt_generic(
    args: &mut Optexpand,
    func: CompleteListItemGetter,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    optstr_st().set_opt_callback_orig_option = if args.oe_include_orig_val {
        args.oe_opt_value
    } else {
        ptr::null_mut()
    };
    optstr_st().set_opt_callback_func = func;

    expand_generic(
        c"",
        args.xp_mut(),
        args.regmatch_mut(),
        matches,
        Some(expand_set_opt_callback),
        false,
    );

    optstr_st().set_opt_callback_orig_option = ptr::null_mut();
    optstr_st().set_opt_callback_func = None;
    true
}

/// Expand an option which is a list of flags.
fn expand_set_opt_listflag(
    args: &mut Optexpand,
    flags: impl crate::strings::IntoCCharPtr,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let flags_bytes = crate::memory::cstr_as_bytes(flags.into_c_char_ptr());
    let option_val = args.oe_opt_value.cast();
    let option_bytes = crate::memory::cstr_as_bytes(option_val);
    let cmdline_bytes = crate::memory::cstr_as_bytes(args.oe_set_arg.cast());
    let append = args.oe_append;
    let include_orig_val = args.oe_include_orig_val && !option_bytes.is_empty();

    matches.clear();

    if include_orig_val {
        matches.push(crate::memory::cstr_to_cstring(option_val));
    }

    for &flag_b in flags_bytes {
        if append && option_bytes.contains(&flag_b) {
            continue;
        }

        if !cmdline_bytes.contains(&flag_b) {
            if include_orig_val && option_bytes == [flag_b] {
                continue;
            }
            matches.push(crate::memory::cstr_from_bytes(&[flag_b]));
        }
    }

    !matches.is_empty()
}

fn completing_value_for_subopt(args: &mut Optexpand, name_suffix: &std::ffi::CStr) -> bool {
    let colon = args.xp_mut().xp_pattern.wrapping_sub(1);
    let suffix_bytes = name_suffix.to_bytes();
    let len = suffix_bytes.len();

    crate::memory::ptr_offset_from(colon, args.oe_set_arg.cast()) >= (len as isize)
        && crate::memory::cstr_slice(colon.wrapping_sub(len).cast(), len) == suffix_bytes
}

/// The 'ambiwidth' option is changed.
pub fn did_set_ambiwidth(_args: &mut Optset) -> Option<&CStr> {
    check_chars_options()
}

/// The 'emoji' option is changed.
pub fn did_set_emoji(args: &mut Optset) -> Option<&CStr> {
    let errmsg = check_str_opt(K_OPT_AMBIWIDTH, ptr::null_mut(), args.os_errbuf);
    if errmsg.is_some() {
        return errmsg;
    }
    check_chars_options()
}

/// The 'background' option is changed.
pub fn did_set_background(args: &mut Optset) -> Option<&CStr> {
    if crate::memory::cstr_first_byte(args.os_oldval.data.string().data)
        == crate::memory::cstr_first_byte(opt_vars().p_bg)
    {
        return None;
    }

    let dark = crate::memory::cstr_first_byte(opt_vars().p_bg) == b'd';

    init_highlight(false, false);

    if dark != (crate::memory::cstr_first_byte(opt_vars().p_bg) == b'd')
        && get_var_value(c"g:colors_name").is_some()
    {
        do_unlet(b"g:colors_name", true);
        free_string_option(opt_vars().p_bg);
        opt_vars().p_bg = alloc_string_option(if dark {
            c"dark".as_ptr()
        } else {
            c"light".as_ptr()
        });
        check_string_option(&raw mut opt_vars().p_bg);
        init_highlight(false, false);
    }

    for buf in for_all_buffers_mut() {
        if let Some(term) = buf.terminal_opt() {
            terminal_notify_theme(term, dark);
        }
    }

    None
}

/// Validate the 'backspace' option.
pub fn validate_backspace(args: &mut Optset) -> Option<&CStr> {
    let value = args.os_newval.data.string().data;
    if ascii_isdigit(crate::memory::cstr_first_byte(value)) {
        return if crate::memory::cstr_first_byte(value) == b'2' {
            None
        } else {
            Some(E_INVARG)
        };
    }
    validate_str_generic(args)
}

pub fn validate_backupcopy(args: &mut Optset) -> Option<&CStr> {
    let mut flags: u32 = 0;
    let errmsg = opt_strings_flags(
        args.os_newval.data.string().data,
        &OPT_BKC_VALUES,
        &raw mut flags,
        true,
        args.os_errbuf,
    );
    if errmsg.is_some() {
        return errmsg;
    }
    let cnt = u32::from((flags & K_OPT_BKC_FLAG_AUTO) != 0)
        + u32::from((flags & K_OPT_BKC_FLAG_YES) != 0)
        + u32::from((flags & K_OPT_BKC_FLAG_NO) != 0);
    if cnt == 1 { None } else { Some(E_INVARG) }
}

/// The 'backupcopy' option is changed.
pub fn did_set_backupcopy(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;
    let mut bkc = opt_vars().p_bkc;
    let mut flags: &mut u32 = &mut opt_vars().bkc_flags;

    if (opt_flags & OPT_LOCAL) != 0 {
        bkc = buf.b_p_bkc;
        flags = &mut buf.b_bkc_flags;
    } else if (opt_flags & OPT_GLOBAL) == 0 {
        buf.b_bkc_flags = 0;
    }

    if (opt_flags & OPT_LOCAL) != 0 && crate::memory::cstr_is_empty(bkc) {
        *flags = 0;
    } else {
        let errmsg = opt_strings_flags(bkc, &OPT_BKC_VALUES, flags, true, args.os_errbuf);
        if errmsg.is_some() {
            return errmsg;
        }
    }

    None
}

/// Validate the 'backupext' or 'patchmode' option.
pub fn validate_backupext_or_patchmode(args: &mut Optset) -> Option<&CStr> {
    let bex = if args.os_idx == K_OPT_BACKUPEXT {
        args.os_newval.data.string().data
    } else {
        opt_vars().p_bex
    };
    let pm = if args.os_idx == K_OPT_PATCHMODE {
        args.os_newval.data.string().data
    } else {
        opt_vars().p_pm
    };
    let bex_s = if crate::memory::cstr_first_byte(bex) == b'.' {
        bex.wrapping_add(1)
    } else {
        bex
    };
    let pm_s = if crate::memory::cstr_first_byte(pm) == b'.' {
        pm.wrapping_add(1)
    } else {
        pm
    };
    if crate::memory::cstr_as_bytes(bex_s) == crate::memory::cstr_as_bytes(pm_s) {
        return Some(E_BACKUPEXT_AND_PATCHMODE_ARE_EQUAL);
    }

    None
}

/// The 'breakat' option is changed.
pub fn did_set_breakat(_args: &mut Optset) -> Option<&CStr> {
    opt_vars().breakat_flags = [false; 256];

    if !opt_vars().p_breakat.is_null() {
        let p = opt_vars().p_breakat;
        for &p_byte in crate::memory::cstr_as_bytes(p) {
            opt_vars().breakat_flags[p_byte as usize] = true;
        }
    }

    None
}

/// The 'breakindentopt' option is changed.
pub fn did_set_breakindentopt(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    let is_local = ptr::eq(args.os_varp, (&raw mut win.w_onebuf_opt.wo_briopt).cast());
    briopt_check(is_local.then_some(&mut *win));

    if is_local && win.w_briopt_list != 0 {
        redraw_all_later(UPD_NOT_VALID);
    }

    None
}

/// Validate the 'buftype' option.
pub fn validate_buftype(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let os_flags = args.os_flags;
    let newval_data = args.os_newval.data.string().data;
    let errmsg = validate_str_generic(args);
    if errmsg.is_some() {
        return errmsg;
    }
    if (os_flags & OPT_GLOBAL) == 0
        && !buf.terminal.is_null() != (crate::memory::cstr_first_byte(newval_data) == b't')
    {
        return Some(E_INVARG);
    }
    None
}

/// The 'buftype' option is changed.
pub fn did_set_buftype(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let win = args.win_mut();
    if crate::memory::cstr_first_byte(buf.b_p_bt) == b'p' {
        set_option_direct(
            K_OPT_COMMENTS,
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(NvimString {
                    data: c"".as_ptr().cast_mut(),
                    size: 0,
                }),
            },
            OPT_LOCAL,
            SID_NONE,
        );
        let next_prompt = CursorPos {
            lnum: buf.b_ml.ml_line_count,
            col: buf.b_prompt_start.mark.col,
            coladd: 0,
        };
        buf.b_prompt_start = Fmark {
            mark: next_prompt,
            fnum: 0,
            timestamp: 0,
            view: Fmarkv {
                topline_offset: MAXLNUM,
                skipcol: 0,
            },
            additional_data: None,
        };
    }
    if win.w_status_height != 0 || global_stl_height() != 0 {
        win.w_redr_status = true;
        redraw_later(Some(win), UPD_VALID);
    }
    buf.b_help = crate::memory::cstr_first_byte(buf.b_p_bt) == b'h';
    redraw_titles();
    None
}

pub fn validate_chars_option(args: &mut Optset) -> Option<&CStr> {
    set_chars_option(
        args.win_mut(),
        args.os_newval.data.string().data,
        if args.os_idx == K_OPT_LISTCHARS {
            CharsOption::Listchars
        } else {
            CharsOption::Fillchars
        },
        false,
        args.os_errbuf,
    )
}

/// The global 'listchars' or 'fillchars' option is changed.
fn did_set_global_chars_option<'a>(
    win: &mut WinInfoInfo,
    val: impl crate::strings::IntoCCharPtr,
    what: CharsOption,
    opt_flags: i32,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let val = val.into_c_char_ptr();
    {
        let local_str = if what == CharsOption::Listchars {
            win.w_onebuf_opt.wo_lcs
        } else {
            win.w_onebuf_opt.wo_fcs
        };
        let local_ptr = if what == CharsOption::Listchars {
            &raw mut win.w_onebuf_opt.wo_lcs
        } else {
            &raw mut win.w_onebuf_opt.wo_fcs
        };

        let errmsg = set_chars_option(
            &mut *win,
            val,
            what,
            crate::memory::cstr_is_empty(local_str) || (opt_flags & OPT_GLOBAL) == 0,
            errbuf,
        );
        if errmsg.is_some() {
            return errmsg;
        }

        if (opt_flags & OPT_GLOBAL) == 0 {
            clear_string_option(local_ptr);
        }

        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_ptr(&*tp) {
                let opt = if what == CharsOption::Listchars {
                    wp.w_onebuf_opt.wo_lcs
                } else {
                    wp.w_onebuf_opt.wo_fcs
                };
                if crate::memory::cstr_is_empty(opt) {
                    set_chars_option(&mut *wp, opt, what, true, errbuf);
                }
            }
        }

        redraw_all_later(UPD_NOT_VALID);

        None
    }
}

/// The 'fillchars' option or the 'listchars' option is changed.
pub fn did_set_chars_option(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    let varp = args.os_varp.cast::<*mut u8>();
    let val = args.var_cstr_ptr();
    let mut errmsg: Option<&CStr> = None;

    if ptr::eq(varp, (&raw mut opt_vars().p_lcs).cast()) {
        errmsg = did_set_global_chars_option(
            win,
            val,
            CharsOption::Listchars,
            args.os_flags,
            args.os_errbuf,
        );
    } else if ptr::eq(varp, (&raw mut opt_vars().p_fcs).cast()) {
        errmsg = did_set_global_chars_option(
            win,
            val,
            CharsOption::Fillchars,
            args.os_flags,
            args.os_errbuf,
        );
    } else if ptr::eq(varp, (&raw mut win.w_onebuf_opt.wo_lcs).cast()) {
        errmsg = set_chars_option(win, val, CharsOption::Listchars, true, args.os_errbuf);
    } else if ptr::eq(varp, (&raw mut win.w_onebuf_opt.wo_fcs).cast()) {
        errmsg = set_chars_option(win, val, CharsOption::Fillchars, true, args.os_errbuf);
    }

    errmsg
}

/// Expand 'fillchars' or 'listchars' option value.
pub fn expand_set_chars_option(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let varp = args.oe_varp;
    let is_lcs = ptr::eq(varp, (&raw mut opt_vars().p_lcs).cast())
        || ptr::eq(varp, (&raw mut curwin_mut().w_onebuf_opt.wo_lcs).cast());
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

/// The 'cinoptions' option is changed.
pub fn did_set_cinoptions(args: &mut Optset) -> Option<&CStr> {
    parse_cino(args.buf_mut());
    None
}

pub fn validate_colorcolumn(args: &mut Optset) -> Option<&CStr> {
    check_colorcolumn(args.os_newval.data.string().data, None)
}

/// The 'colorcolumn' option is changed.
pub fn did_set_colorcolumn(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    let varp = args.os_varp.cast::<*mut u8>();
    let is_local = ptr::eq(varp, (&raw mut win.w_onebuf_opt.wo_cc).cast());
    check_colorcolumn(args.var_cstr_ptr(), is_local.then_some(win))
}

/// Validate the 'comments' option.
pub fn validate_comments(args: &mut Optset) -> Option<&CStr> {
    let mut errmsg: Option<&CStr> = None;
    let newval_str = args.os_newval.data.string();
    let mut rem = newval_str.as_bytes();
    let com_all_bytes = COM_ALL.to_bytes();
    while !rem.is_empty() {
        while let Some(&b) = rem.first() {
            if b == b':' {
                break;
            }
            if !com_all_bytes.contains(&b) && !b.is_ascii_digit() && b != b'-' {
                errmsg = illegal_char(args.os_errbuf, i32::from(b));
                break;
            }
            rem = &rem[1..];
        }
        let c = rem.first().copied().unwrap_or(0);
        if !rem.is_empty() {
            rem = &rem[1..];
        }
        if c == 0 {
            errmsg = Some(c"E524: Missing colon");
        } else if rem.first().is_none_or(|&b| b == b',') {
            errmsg = Some(c"E525: Zero length string");
        }
        if errmsg.is_some() {
            break;
        }
        while !rem.is_empty() && rem[0] != b',' {
            if rem[0] == b'\\' && rem.len() >= 2 {
                rem = &rem[1..];
            }
            rem = &rem[1..];
        }
        rem = crate::option::skip_to_option_part_slice(rem);
    }
    errmsg
}

/// Validate the 'commentstring' option.
pub fn validate_commentstring(args: &mut Optset) -> Option<&CStr> {
    let newval_str = args.os_newval.data.string();
    let bytes = newval_str.as_bytes();
    if bytes.is_empty() || bytes.windows(2).any(|w| w == b"%s") {
        None
    } else {
        Some(c"E537: 'commentstring' must be empty or contain %s")
    }
}

/// Validate the 'complete' option.
pub fn validate_complete(args: &mut Optset) -> Option<&CStr> {
    let mut buffer = [0u8; LSIZE];
    let mut char_before: u8 = 0;

    let newval_str = args.os_newval.data.string();
    let mut rem = newval_str.as_bytes();
    while !rem.is_empty() {
        buffer.fill(0);
        let mut buf_idx: usize = 0;
        let mut escape = false;
        while !rem.is_empty() && (rem[0] != b',' || escape) && buf_idx < LSIZE - 1 {
            if rem.starts_with(b"\\,") {
                escape = true;
                rem = &rem[1..];
            } else {
                escape = false;
                buffer[buf_idx] = rem[0];
                buf_idx += 1;
                rem = &rem[1..];
            }
        }
        buffer[buf_idx] = 0;

        if !b".wbuksid]tUfFo".contains(&buffer[0]) {
            return illegal_char(args.os_errbuf, i32::from(buffer[0]));
        }

        if !b"ksF".contains(&buffer[0]) && buffer[1] != 0 && buffer[1] != b'^' {
            char_before = buffer[0];
        } else if let Some(caret_pos) = buffer[..buf_idx].iter().position(|&b| b == b'^') {
            let tail = &buffer[caret_pos + 1..buf_idx];
            if tail.is_empty() || !tail.iter().all(u8::is_ascii_digit) {
                char_before = b'^';
            }
        }
        if char_before != 0 {
            return crate::option::opt_error_int(
                args.os_errbuf,
                E_ILLEGAL_CHARACTER_AFTER_CHR,
                i32::from(char_before),
            );
        }
        while rem.first().is_some_and(|&b| matches!(b, b',' | b' ')) {
            rem = &rem[1..];
        }
    }

    None
}

/// The 'complete' option is changed.
pub fn did_set_complete(args: &mut Optset) -> Option<&CStr> {
    if !set_cpt_callbacks(args) {
        return crate::option::opt_error_int(
            args.os_errbuf,
            E_ILLEGAL_CHARACTER_AFTER_CHR,
            b'F' as i32,
        );
    }
    None
}

/// Validate 'completeitemalign' and prepare its column order.
pub fn validate_completeitemalign(args: &mut Optset) -> Option<&CStr> {
    let mut p = args.os_newval.data.string().data;
    let mut new_cia_flags: u32 = 0;
    let mut seen = [false; 3];
    let mut count = 0;
    let mut buf = [0u8; 10];
    while !crate::memory::cstr_is_empty(p) {
        copy_option_part(&raw mut p, buf.as_mut_ptr().cast(), buf.len(), c",");
        let item_bytes = crate::memory::cstr_until_nul(&buf).to_bytes();
        let item = if item_bytes == b"abbr" {
            CPT_ABBR
        } else if item_bytes == b"kind" {
            CPT_KIND
        } else if item_bytes == b"menu" {
            CPT_MENU
        } else {
            return Some(E_INVARG);
        };
        if seen[item as usize] {
            return Some(E_INVARG);
        }
        new_cia_flags = new_cia_flags * 10 + item;
        seen[item as usize] = true;
        count += 1;
    }
    if count != 3 {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Flags(new_cia_flags);
    None
}

/// The 'completeitemalign' option is changed.
pub fn did_set_completeitemalign(args: &mut Optset) -> Option<&CStr> {
    opt_vars().cia_flags = args.os_prep.as_flags();
    None
}

/// The 'completeopt' option is changed.
pub fn did_set_completeopt(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let mut cot = opt_vars().p_cot;
    let mut flags: &mut u32 = &mut opt_vars().cot_flags;

    if (args.os_flags & OPT_LOCAL) != 0 {
        cot = buf.b_p_cot;
        flags = &mut buf.b_cot_flags;
    } else if opt_flags_is_both(args.os_flags) {
        buf.b_cot_flags = 0;
    }

    opt_strings_flags(cot, &OPT_COT_VALUES, flags, true, args.os_errbuf)
}

#[inline]
fn opt_flags_is_both(opt_flags: i32) -> bool {
    (opt_flags & OPT_GLOBAL) == 0
}

#[cfg(windows)]
pub fn did_set_completeslash(args: &mut Optset) -> Option<&CStr> {
    let errmsg = opt_strings_flags(
        opt_vars().p_csl,
        &OPT_CSL_VALUES,
        ptr::null_mut(),
        false,
        args.os_errbuf,
    );
    if errmsg.is_some() {
        return errmsg;
    }
    opt_strings_flags(
        args.buf_mut().b_p_csl,
        &OPT_CSL_VALUES,
        ptr::null_mut(),
        false,
        args.os_errbuf,
    )
}

/// Validate the 'concealcursor' option.
pub fn validate_concealcursor(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(args.os_newval.data.string().data, COCU_ALL, args.os_errbuf)
}

pub fn expand_set_concealcursor(
    args: &mut Optexpand,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, COCU_ALL, matches)
}

/// Validate the 'cpoptions' option.
pub fn validate_cpoptions(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(args.os_newval.data.string().data, CPO_VI, args.os_errbuf)
}

pub fn expand_set_cpoptions(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, CPO_VI, matches)
}

pub fn validate_cursorlineopt(args: &mut Optset) -> Option<&CStr> {
    let value = args.os_newval.data.string().data;
    let mut flags: u32 = 0;
    if crate::memory::cstr_is_empty(value) || !fill_culopt_flags(value, &mut flags) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Flags(flags);
    None
}

/// The 'cursorlineopt' option is changed.
pub fn did_set_cursorlineopt(args: &mut Optset) -> Option<&CStr> {
    if (args.os_flags & OPT_GLOBAL) != 0 {
        return None;
    }
    args.win_mut().w_p_culopt_flags = args.os_prep.as_flags() as u8;
    None
}

/// The 'diffanchors' option is changed.
pub fn did_set_diffanchors(args: &mut Optset) -> Option<&CStr> {
    if !diffanchors_changed((args.os_flags & OPT_LOCAL) != 0) {
        return Some(E_INVARG);
    }
    None
}

/// The 'diffopt' option is changed.
pub fn did_set_diffopt(_args: &mut Optset) -> Option<&CStr> {
    if !diffopt_changed() {
        Some(E_INVARG)
    } else {
        None
    }
}

pub fn expand_set_diffopt(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let xp_pat = args.xp_mut().xp_pattern;

    if (xp_pat as usize) > (args.oe_set_arg as usize)
        && crate::memory::cstr_first_byte(xp_pat.wrapping_sub(1).cast()) == b':'
    {
        if completing_value_for_subopt(args, c"algorithm") {
            return expand_set_opt_string(args, &OPT_DIP_ALGORITHM_VALUES, matches);
        }
        if completing_value_for_subopt(args, c"inline") {
            return expand_set_opt_string(args, &OPT_DIP_INLINE_VALUES, matches);
        }
        return false;
    }

    expand_set_str_generic(args, matches)
}

/// The 'display' option is changed.
pub fn did_set_display(args: &mut Optset) -> Option<&CStr> {
    let errmsg = did_set_str_generic(args);
    if errmsg.is_some() {
        return errmsg;
    }
    opt_vars().dy_escape_width = if (opt_vars().dy_flags & K_OPT_DY_FLAG_UHEX) != 0 {
        4
    } else {
        2
    };
    msg_grid_validate();
    None
}

/// Validate and canonize 'encoding', 'fileencoding' or 'makeencoding'.
pub fn validate_encoding(args: &mut Optset) -> Option<&CStr> {
    if args.os_idx == K_OPT_FILEENCODING {
        if !modifiable(args.buf_mut()) && args.os_flags != OPT_GLOBAL {
            return Some(E_MODIFIABLE);
        }

        if crate::strings::vim_haschr(args.os_newval.data.string().data, b',' as i32) {
            return Some(E_INVARG);
        }
    }

    let value_cstr = enc_canonize(args.os_newval.data.string().data);
    let is_not_utf8 = value_cstr.as_bytes() != b"utf-8";
    args.os_prep = OptSetPrep::Encoding(value_cstr);
    if args.os_idx == K_OPT_ENCODING && is_not_utf8 {
        return Some(E_UNSUPPORTEDOPTION);
    }
    None
}

/// One of the 'encoding', 'fileencoding' or 'makeencoding' options is changed.
pub fn did_set_encoding(args: &mut Optset) -> Option<&CStr> {
    let varp = args.os_varp.cast::<*mut u8>();
    if args.os_idx == K_OPT_FILEENCODING {
        redraw_titles();
        ml_setflags(args.buf_mut());
    }

    if let Some(prep_cstr) = args.os_prep.take_encoding() {
        free_string_option(args.var_cstr_ptr());
        unsafe {
            *varp = alloc_string_option(prep_cstr.as_ptr());
        }
    }
    if ptr::eq(varp, (&raw mut opt_vars().p_enc).cast()) {
        spell_reload();
    }
    None
}

pub fn expand_set_encoding(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_generic(args, Some(get_encoding_name), matches)
}

/// Validate the 'eventignore'/'eventignorewin' option.
pub fn validate_eventignore(args: &mut Optset) -> Option<&CStr> {
    if !check_ei(
        args.os_newval.data.string().data,
        args.os_idx == K_OPT_EVENTIGNOREWIN,
    ) {
        return Some(E_INVARG);
    }
    None
}

fn get_eventignore_name(xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    let subtract = crate::memory::cstr_first_byte(xp.xp_pattern.cast()) == b'-';
    if !subtract && idx == 0 {
        return Some(c"all");
    }

    let name = get_event_name_no_group(xp, idx - 1 + i32::from(subtract), optstr_st().expand_eiw)?;

    let io_buff = g_st().iobuff.as_mut_ptr();
    vim_snprintf!(
        io_buff,
        IOSIZE,
        c"%s%s",
        if subtract {
            c"-".as_ptr()
        } else {
            c"".as_ptr()
        },
        name.as_ptr(),
    );
    Some(crate::memory::cstr_from_ptr(io_buff))
}

pub fn expand_set_eventignore(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    optstr_st().expand_eiw = !ptr::eq(args.oe_varp, (&raw mut opt_vars().p_ei).cast::<u8>());
    expand_set_opt_generic(args, Some(get_eventignore_name), matches)
}

pub fn validate_fileformat(args: &mut Optset) -> Option<&CStr> {
    if (args.os_flags & OPT_GLOBAL) == 0 && !modifiable(args.buf_mut()) {
        return Some(E_MODIFIABLE);
    }
    validate_str_generic(args)
}

/// The 'fileformat' option is changed.
pub fn did_set_fileformat(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let oldval = args.os_oldval.data.string().data;

    redraw_titles();
    ml_setflags(buf);
    if get_fileformat(buf) == EOL_MAC || crate::memory::cstr_first_byte(oldval) == b'm' {
        redraw_buf_later(args.os_buf, UPD_NOT_VALID);
    }
    None
}

/// Function given to ExpandGeneric() to obtain the possible arguments of fileformat options.
pub fn get_fileformat_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx >= 0)
        .then(|| OPT_FF_VALUES.get(idx as usize).copied())
        .flatten()
}

/// Validate the 'filetype', 'syntax' or 'keymap' option.
pub fn validate_filetype(args: &mut Optset) -> Option<&CStr> {
    if valid_filetype(args.os_newval.data.string().as_bytes()) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'filetype' or the 'syntax' option is changed.
pub fn did_set_filetype_or_syntax(args: &mut Optset) -> Option<&CStr> {
    args.os_value_changed = crate::memory::cstr_as_bytes(args.os_oldval.data.string().data)
        != crate::memory::cstr_as_bytes(args.var_cstr_ptr());
    args.os_value_checked = true;
    None
}

/// The 'foldexpr' option is changed.
pub fn did_set_foldexpr(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    if foldmethod_is_expr(win) {
        fold_update_all(win);
    }
    None
}

/// The 'foldignore' option is changed.
pub fn did_set_foldignore(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    if foldmethod_is_indent(win) {
        fold_update_all(win);
    }
    None
}

/// Validate the 'foldmarker' option.
pub fn validate_foldmarker(args: &mut Optset) -> Option<&CStr> {
    let value = args.os_newval.data.string().data;
    let p = vim_strchr(value, b',' as i32);
    if p.is_null() {
        return Some(E_COMMA_REQUIRED);
    }
    if p == value || crate::memory::cstr_len(p) <= 1 {
        Some(E_INVARG)
    } else {
        None
    }
}

/// The 'foldmarker' option is changed.
pub fn did_set_foldmarker(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    if foldmethod_is_marker(win) {
        fold_update_all(win);
    }
    None
}

/// The 'foldmethod' option is changed.
pub fn did_set_foldmethod(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    fold_update_all(win);
    if foldmethod_is_diff(win) {
        new_fold_level();
    }
    None
}

/// Validate the 'formatoptions' option.
pub fn validate_formatoptions(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(args.os_newval.data.string().data, FO_ALL, args.os_errbuf)
}

pub fn expand_set_formatoptions(
    args: &mut Optexpand,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, FO_ALL, matches)
}

/// The 'guicursor' option is changed.
pub fn did_set_guicursor(_args: &mut Optset) -> Option<&CStr> {
    if let Some(errmsg) = parse_shape_opt(SHAPE_CURSOR) {
        return Some(errmsg);
    }
    if g_st().visual_global.active {
        redraw_winline(crate::globals::curwin_mut(), curwin_mut().w_cursor.lnum);
    }
    None
}

/// The 'helpfile' option is changed.
pub fn did_set_helpfile(_args: &mut Optset) -> Option<&CStr> {
    if g_st().didset_vim {
        vim_unsetenv_ext(c"VIM");
    }
    if g_st().didset_vimruntime {
        vim_unsetenv_ext(c"VIMRUNTIME");
    }
    None
}

/// Validate the 'helplang' option.
pub fn validate_helplang(args: &mut Optset) -> Option<&CStr> {
    let newval_str = args.os_newval.data.string();
    let mut rem = newval_str.as_bytes();
    while !rem.is_empty() {
        if rem.len() < 2 || (rem.len() > 2 && (rem[2] != b',' || rem.len() == 3)) {
            return Some(E_INVARG);
        }
        if rem.len() == 2 {
            break;
        }
        rem = &rem[3..];
    }
    None
}

/// Validate the 'highlight' option.
pub fn validate_highlight(args: &mut Optset) -> Option<&CStr> {
    if crate::memory::cstr_as_bytes(args.os_newval.data.string().data)
        != crate::memory::cstr_as_bytes(HIGHLIGHT_INIT.as_ptr())
    {
        return Some(E_UNSUPPORTEDOPTION);
    }
    None
}

/// The 'iconstring' option is changed.
pub fn did_set_iconstring(args: &mut Optset) -> Option<&CStr> {
    did_set_titleiconstring(args, STL_IN_ICON)
}

/// The 'inccommand' option is changed.
pub fn did_set_inccommand(_args: &mut Optset) -> Option<&CStr> {
    if g_st().cmdpreview {
        return Some(E_INVARG);
    }
    None
}

/// Validate the 'isident', 'iskeyword' or 'isfname' option.
pub fn validate_isopt(args: &mut Optset) -> Option<&CStr> {
    if !check_isopt(args.os_newval.data.string().as_bytes()) {
        Some(E_INVARG)
    } else {
        None
    }
}

/// The 'iskeyword' option is changed.
pub fn did_set_iskeyword(args: &mut Optset) -> Option<&CStr> {
    let varp = args.os_varp.cast::<*mut u8>();
    if !ptr::eq(varp, (&raw mut opt_vars().p_isk).cast()) && !buf_init_isk_chartab(args.buf_mut()) {
        return Some(E_INVARG);
    }
    None
}

/// The 'isident', 'iskeyword', 'isprint' or 'isfname' option is changed.
pub fn did_set_isopt(args: &mut Optset) -> Option<&CStr> {
    let res = if ptr::eq(args.os_varp, (&raw mut opt_vars().p_isf).cast()) {
        init_isf_chartab()
    } else {
        init_isi_chartab()
    };
    if !res { Some(E_INVARG) } else { None }
}

/// The 'keymap' option has changed.
pub fn did_set_keymap(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;

    let secure_save = g_st().secure;
    g_st().secure = 0;

    let errmsg = keymap_init();

    g_st().secure = secure_save;

    args.os_value_checked = true;

    if errmsg.is_none() {
        if !crate::memory::cstr_is_empty(buf.b_p_keymap) {
            buf.b_p_iminsert = B_IMODE_LMAP;
            if buf.b_p_imsearch != B_IMODE_USE_INSERT {
                buf.b_p_imsearch = B_IMODE_LMAP;
            }
        } else {
            if buf.b_p_iminsert == B_IMODE_LMAP {
                buf.b_p_iminsert = B_IMODE_NONE;
            }
            if buf.b_p_imsearch == B_IMODE_LMAP {
                buf.b_p_imsearch = B_IMODE_USE_INSERT;
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

/// The 'keymodel' option is changed.
pub fn did_set_keymodel(_args: &mut Optset) -> Option<&CStr> {
    g_st().km_stopsel = crate::strings::vim_haschr(opt_vars().p_km, b'o' as i32);
    g_st().km_startsel = crate::strings::vim_haschr(opt_vars().p_km, b'a' as i32);
    None
}

/// Validate the 'lispoptions' option.
pub fn validate_lispoptions(args: &mut Optset) -> Option<&CStr> {
    let newval_str = args.os_newval.data.string();
    let bytes = newval_str.as_bytes();
    if bytes.is_empty() || bytes == b"expr:0" || bytes == b"expr:1" {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Validate the 'matchpairs' option.
pub fn validate_matchpairs(args: &mut Optset) -> Option<&CStr> {
    let newval_str = args.os_newval.data.string();
    let mut rem = newval_str.as_bytes();
    while !rem.is_empty() {
        let mut x2: i32 = -1;
        let mut x3: i32 = -1;

        let l1 = crate::mbyte::utfc_slice2len(rem).min(rem.len());
        rem = &rem[l1..];
        if let Some((&b, rest)) = rem.split_first() {
            x2 = i32::from(b);
            rem = rest;
        }
        if !rem.is_empty() {
            x3 = crate::mbyte::utf_slice2char(rem);
            let l2 = crate::mbyte::utfc_slice2len(rem).min(rem.len());
            rem = &rem[l2..];
        }
        if x2 != b':' as i32 || x3 == -1 || rem.first().is_some_and(|&b| b != b',') {
            return Some(E_INVARG);
        }
        if rem.is_empty() {
            break;
        }
        rem = &rem[1..];
    }
    None
}

/// Validate the 'mkspellmem' option.
pub fn validate_mkspellmem(args: &mut Optset) -> Option<&CStr> {
    if spell_check_msm(args.os_newval.data.string().data, false) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Validate the 'mouse' option.
pub fn validate_mouse(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(args.os_newval.data.string().data, MOUSE_ALL, args.os_errbuf)
}

pub fn expand_set_mouse(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, MOUSE_ALL, matches)
}

/// Validate the 'mousescroll' option.
pub fn validate_mousescroll(args: &mut Optset) -> Option<&CStr> {
    let v = opt_keyset(
        args.os_newval.data.string().data,
        K_OPT_MOUSESCROLL,
        ptr::null_mut(),
    )
    .cast::<OptKeyDict_mousescroll>();
    let has_hor = (unsafe { (*v).is_set_mousescroll } & (1 << KEYSET_OPTIDX_MOUSESCROLL_HOR)) != 0;
    let has_ver = (unsafe { (*v).is_set_mousescroll } & (1 << KEYSET_OPTIDX_MOUSESCROLL_VER)) != 0;
    if has_hor || has_ver {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Handle setting 'mousescroll'.
pub fn did_set_mousescroll(_args: &mut Optset) -> Option<&CStr> {
    let v = opt_keyset(opt_vars().p_mousescroll, K_OPT_MOUSESCROLL, ptr::null_mut())
        .cast::<OptKeyDict_mousescroll>();
    let v = unsafe { &mut *v };
    let has_hor = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_HOR)) != 0;
    let has_ver = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_VER)) != 0;
    opt_vars().p_mousescroll_hor = if has_hor {
        i64::from(v.hor as i32)
    } else {
        MOUSESCROLL_HOR_DFLT
    };
    opt_vars().p_mousescroll_vert = if has_ver {
        i64::from(v.ver as i32)
    } else {
        MOUSESCROLL_VERT_DFLT
    };
    None
}

/// The 'rulerformat' option is changed.
pub fn did_set_rulerformat(args: &mut Optset) -> Option<&CStr> {
    did_set_statustabline_rulerformat(args, true, false)
}

/// The 'selection' option is changed.
pub fn did_set_selection(_args: &mut Optset) -> Option<&CStr> {
    if g_st().visual_global.active {
        redraw_curbuf_later(UPD_INVERTED);
    }
    None
}

/// Validate the 'sessionoptions' option.
pub fn validate_sessionoptions(args: &mut Optset) -> Option<&CStr> {
    let mut flags: u32 = 0;
    let errmsg = opt_strings_flags(
        args.os_newval.data.string().data,
        &OPT_SSOP_VALUES,
        &raw mut flags,
        true,
        args.os_errbuf,
    );
    if errmsg.is_some() {
        return errmsg;
    }
    if (flags & K_OPT_SSOP_FLAG_CURDIR) != 0 && (flags & K_OPT_SSOP_FLAG_SESDIR) != 0 {
        Some(E_INVARG)
    } else {
        None
    }
}

/// Validate the 'shada' option.
pub fn validate_shada(args: &mut Optset) -> Option<&CStr> {
    let errbuf = args.os_errbuf;
    let newval_str = args.os_newval.data.string();
    let val_bytes = newval_str.as_bytes();
    let mut marks: Option<&[u8]> = None;

    let mut rem = val_bytes;
    while let Some((&b0, rest)) = rem.split_first() {
        if b0 == b'\'' && marks.is_none() {
            marks = Some(rest);
        }
        if !b"!\"%'/:<@cfhnrs".contains(&b0) {
            return illegal_char(errbuf, i32::from(b0));
        }
        if b0 == b'n' {
            break;
        } else if b0 == b'r' {
            let pos = rest.iter().position(|&b| b == b',').unwrap_or(rest.len());
            rem = &rest[pos..];
        } else if b0 == b'%' {
            let digits = rest.iter().take_while(|&&b| b.is_ascii_digit()).count();
            rem = &rest[digits..];
        } else if matches!(b0, b'!' | b'h' | b'c') {
            rem = rest;
        } else {
            let digits = rest.iter().take_while(|&&b| b.is_ascii_digit()).count();
            if digits == 0 {
                if errbuf.is_some() {
                    return opt_error(
                        errbuf,
                        c"E526: Missing number after <%s>",
                        transchar_byte(i32::from(b0)),
                    );
                } else {
                    return Some(c"");
                }
            }
            rem = &rest[digits..];
        }
        if let Some(after_comma) = rem.strip_prefix(b",") {
            rem = after_comma;
        } else if !rem.is_empty() {
            if errbuf.is_some() {
                return Some(c"E527: Missing comma");
            } else {
                return Some(c"");
            }
        }
    }
    if !val_bytes.is_empty()
        && marks.is_none_or(|m| (crate::charset::parse_i64_prefix(m) as i32) < 0)
    {
        return Some(c"E528: Must specify a ' value");
    }
    None
}

/// Validate 'shellpipe'/'shellredir' option.
pub fn validate_shellpipe_redir(args: &mut Optset) -> Option<&CStr> {
    let mut seen = false;
    let newval_str = args.os_newval.data.string();
    let mut rem = newval_str.as_bytes();
    while let Some((&b0, rest)) = rem.split_first() {
        if b0 != b'%' {
            rem = rest;
            continue;
        }
        let Some((&b1, rest2)) = rest.split_first() else {
            return Some(E_INVALID_FORMAT_STRING_SINGLE_PERCENT_S);
        };
        if b1 == b'%' {
            rem = rest2;
            continue;
        }
        if b1 == b's' {
            if seen {
                return Some(E_INVALID_FORMAT_STRING_SINGLE_PERCENT_S);
            }
            seen = true;
            rem = rest2;
            continue;
        }
        return Some(E_INVALID_FORMAT_STRING_SINGLE_PERCENT_S);
    }
    None
}

pub fn expand_set_shortmess(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, SHM_ALL, matches)
}

/// Validate the 'shortmess' option.
pub fn validate_shortmess(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(args.os_newval.data.string().data, SHM_ALL, args.os_errbuf)
}

/// Validate the 'showbreak' option.
pub fn validate_showbreak(args: &mut Optset) -> Option<&CStr> {
    let mut s = args.os_newval.data.string().data;
    while !crate::memory::cstr_is_empty(s) {
        if ptr2cells(s) != 1 {
            return Some(E_SHOWBREAK_CONTAINS_UNPRINTABLE_OR_WIDE_CHARACTER);
        }
        s = s.wrapping_add(utfc_ptr2len(s) as usize);
    }
    None
}

/// The 'showcmdloc' option is changed.
pub fn did_set_showcmdloc(_args: &mut Optset) -> Option<&CStr> {
    comp_col();
    None
}

pub fn validate_signcolumn(args: &mut Optset) -> Option<&CStr> {
    if check_signcolumn(args.os_newval.data.string().data, None) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'signcolumn' option is changed.
pub fn did_set_signcolumn(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    let varp = args.os_varp.cast::<*mut u8>();
    let oldval = args.os_oldval.data.string().data;
    let is_local = ptr::eq(varp, (&raw mut win.w_onebuf_opt.wo_scl).cast());
    if !check_signcolumn(args.var_cstr_ptr(), is_local.then_some(&mut *win)) {
        return Some(E_INVARG);
    }
    if crate::memory::cstr_as_bytes(oldval).starts_with(b"nu") || win.w_minscwidth == SCL_NUM {
        win.w_nrwidth_line_count = 0;
    }
    None
}

/// Validate 'spellcapcheck' and prepare its regexp program.
pub fn validate_spellcapcheck(args: &mut Optset) -> Option<&CStr> {
    let mut prog: Option<Box<Regprog>> = None;
    let mut err = Error::new();
    let save_emsg_silent = g_st().emsg_silent;
    let save_suppress_errthrow = g_st().suppress_errthrow;
    g_st().emsg_silent = 0;
    g_st().suppress_errthrow = false;

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let errmsg = compile_cap_prog(args.os_newval.data.string().data, &mut prog);
    try_leave(&tstate, &mut err);

    g_st().emsg_silent = save_emsg_silent;
    g_st().suppress_errthrow = save_suppress_errthrow;
    args.os_prep = OptSetPrep::SpellcapProg(prog);
    if err.err_type != ErrorType::ErrorTypeNone {
        let msg_cstr = err.msg_cstr().unwrap_or(c"");
        let msg = if msg_cstr.to_bytes().starts_with(b"Vim:") {
            msg_cstr.as_ptr().wrapping_add(4)
        } else {
            msg_cstr.as_ptr()
        };
        let err_out = opt_error(args.os_errbuf, c"%s", msg);
        api_clear_error(&mut err);
        return err_out;
    }
    errmsg
}

/// The 'spellcapcheck' option is changed.
pub fn did_set_spellcapcheck(args: &mut Optset) -> Option<&CStr> {
    if ptr::eq(args.os_varp, (&raw mut opt_vars().p_spc).cast()) {
        return None;
    }
    let ws = args.win_mut().w_s_ptr();
    ws.b_cap_prog = args.os_prep.take_spellcap_prog();
    None
}

/// Validate the 'spellfile' option.
pub fn validate_spellfile(args: &mut Optset) -> Option<&CStr> {
    if valid_spellfile(args.os_newval.data.string().data) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'spellfile' option is changed.
pub fn did_set_spellfile(_args: &mut Optset) -> Option<&CStr> {
    did_set_spell_option()
}

/// Validate the 'spelllang' option.
pub fn validate_spelllang(args: &mut Optset) -> Option<&CStr> {
    if valid_spelllang(args.os_newval.data.string().data) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'spelllang' option is changed.
pub fn did_set_spelllang(_args: &mut Optset) -> Option<&CStr> {
    did_set_spell_option()
}

/// The 'spelloptions' option is changed.
pub fn did_set_spelloptions(args: &mut Optset) -> Option<&CStr> {
    let opt_flags = args.os_flags;
    let val = args.os_newval.data.string().data;

    if (opt_flags & OPT_LOCAL) == 0 {
        let errmsg = opt_strings_flags(
            val,
            &OPT_SPO_VALUES,
            &raw mut opt_vars().spo_flags,
            true,
            args.os_errbuf,
        );
        if errmsg.is_some() {
            return errmsg;
        }
    }
    if (opt_flags & OPT_GLOBAL) == 0 {
        let ws = args.win_mut().w_s_ptr();
        let errmsg = opt_strings_flags(
            val,
            &OPT_SPO_VALUES,
            &raw mut ws.b_p_spo_flags,
            true,
            args.os_errbuf,
        );
        if errmsg.is_some() {
            return errmsg;
        }
    }
    None
}

/// Validate the 'spellsuggest' option.
pub fn validate_spellsuggest(args: &mut Optset) -> Option<&CStr> {
    let mut sps: [i32; 2] = [0, 0];
    let [s0, s1] = &mut sps;
    if !spell_check_sps(args.os_newval.data.string().data, s0, s1) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Spellsuggest(sps);
    None
}

/// The 'spellsuggest' option is changed.
pub fn did_set_spellsuggest(args: &mut Optset) -> Option<&CStr> {
    if let OptSetPrep::Spellsuggest(sps) = args.os_prep {
        spell_apply_sps(sps[0], sps[1]);
    }
    None
}

pub fn did_set_splitkeep(_args: &mut Optset) -> Option<&CStr> {
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&*tp) {
            wp.w_prev_height = wp.w_height;
            wp.w_prev_winrow = wp.w_winrow;
        }
    }
    None
}

pub fn validate_statustabline_rulerformat(args: &mut Optset) -> Option<&CStr> {
    let value = args.os_newval.data.string().data;
    if crate::memory::cstr_as_bytes(value).starts_with(b"%!") {
        None
    } else {
        check_stl_option(value, args.os_errbuf)
    }
}

/// The 'statuscolumn' option is changed.
pub fn did_set_statuscolumn(args: &mut Optset) -> Option<&CStr> {
    did_set_statustabline_rulerformat(args, false, true)
}

/// The 'statusline' option is changed.
pub fn did_set_statusline(args: &mut Optset) -> Option<&CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'statusline', 'winbar', 'tabline', 'rulerformat' or 'statuscolumn' option is changed.
fn did_set_statustabline_rulerformat<'a>(
    args: &mut Optset,
    rulerformat: bool,
    statuscolumn: bool,
) -> Option<&'a CStr> {
    let varp = args.os_varp.cast::<*mut u8>();
    if rulerformat {
        g_st().ru_wid = 0;
    } else if statuscolumn {
        args.win_mut().w_nrwidth_line_count = 0;
    }
    let mut s = args.var_cstr_ptr();
    let is_stl = args.os_idx == K_OPT_STATUSLINE;

    if (is_stl || rulerformat)
        && ((args.os_flags & OPT_GLOBAL) != 0 || (args.os_flags & OPT_LOCAL) == 0)
        && crate::memory::cstr_is_empty(s)
    {
        free_string_option(s);
        let def = get_option_default(args.os_idx, args.os_flags);
        let new_ptr = alloc_string_option(def.data.string().data);
        unsafe {
            *varp = new_ptr;
        }
        optval_free_read(args.os_idx, def);
        s = new_ptr.cast();
    }

    if is_stl
        && let Some(win) = args.win_opt()
        && win.w_floating
    {
        let cfg = win.w_config.clone();
        win_config_float(win, cfg);
    }

    if rulerformat && !ui_has(K_UIMESSAGES) && crate::memory::cstr_first_byte(s) == b'%' {
        s = s.wrapping_add(1);
        if crate::memory::cstr_first_byte(s) == b'-' {
            s = s.wrapping_add(1);
        }
        let mut rem = crate::memory::cstr_as_bytes(s);
        let wid = getdigits_int(&mut rem, true, 0);
        if wid != 0 && rem.first() == Some(&b'(') {
            g_st().ru_wid = wid;
        }
    }
    if rulerformat {
        comp_col();
    }
    None
}

/// The 'tabline' option is changed.
pub fn did_set_tabline(args: &mut Optset) -> Option<&CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'tagcase' option is changed.
pub fn did_set_tagcase(args: &mut Optset) -> Option<&CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;

    let (p, flags): (_, &mut u32) = if (opt_flags & OPT_LOCAL) != 0 {
        (buf.b_p_tc, &mut buf.b_tc_flags)
    } else {
        (opt_vars().p_tc, &mut opt_vars().tc_flags)
    };

    if (opt_flags & OPT_LOCAL) != 0 && crate::memory::cstr_is_empty(p) {
        *flags = 0;
    } else {
        let errmsg = opt_strings_flags(p, &OPT_TC_VALUES, flags, false, args.os_errbuf);
        if errmsg.is_some() {
            return errmsg;
        }
    }
    None
}

/// The 'titlestring' or the 'iconstring' option is changed.
fn did_set_titleiconstring<'a>(args: &mut Optset, flagval: i32) -> Option<&'a CStr> {
    let val = args.var_cstr_ptr();

    if crate::strings::vim_haschr(val, b'%' as i32) && check_stl_option(val, None).is_none() {
        g_st().stl_syntax |= flagval;
    } else {
        g_st().stl_syntax &= !flagval;
    }
    did_set_title();

    None
}

/// The 'titlestring' option is changed.
pub fn did_set_titlestring(args: &mut Optset) -> Option<&CStr> {
    did_set_titleiconstring(args, STL_IN_TITLE)
}

/// Validate 'vartabstop' or 'varsofttabstop' and prepare its tab widths.
pub fn validate_vartabstop(args: &mut Optset) -> Option<&CStr> {
    let value = args.os_newval.data.string().data;
    let mut cp = value;
    while !crate::memory::cstr_is_empty(cp) {
        if ascii_isdigit(crate::memory::cstr_first_byte(cp)) {
            cp = cp.wrapping_add(1);
            continue;
        }
        if crate::memory::cstr_first_byte(cp) == b','
            && (cp as usize) > (value as usize)
            && crate::memory::cstr_first_byte(cp.wrapping_sub(1)) != b','
        {
            cp = cp.wrapping_add(1);
            continue;
        }
        return Some(E_INVARG);
    }

    let mut array: Vec<Colnr> = Vec::new();
    let errmsg = tabstop_set(value, &mut array, args.os_errbuf);
    args.os_prep = OptSetPrep::Tabstop(array);
    errmsg
}

/// The 'vartabstop' or 'varsofttabstop' option is changed.
pub fn did_set_vartabstop(args: &mut Optset) -> Option<&CStr> {
    if ptr::eq(args.os_varp, (&raw mut opt_vars().p_vts).cast())
        || ptr::eq(args.os_varp, (&raw mut opt_vars().p_vsts).cast())
    {
        return None;
    }
    let buf = args.buf_mut();
    let win = args.win_mut();
    let soft = args.os_idx == K_OPT_VARSOFTTABSTOP;
    let array = if soft {
        &mut buf.b_p_vsts_array
    } else {
        &mut buf.b_p_vts_array
    };
    *array = args.os_prep.take_tabstop();
    if !soft && foldmethod_is_indent(win) {
        fold_update_all(win);
    }
    None
}

/// The 'verbosefile' option is changed.
pub fn did_set_verbosefile(_args: &mut Optset) -> Option<&CStr> {
    verbose_stop();
    if !crate::memory::cstr_is_empty(opt_vars().p_vfile) && !verbose_open() {
        return Some(E_INVARG);
    }
    None
}

/// The 'virtualedit' option is changed.
pub fn did_set_virtualedit(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();

    let mut ve = opt_vars().p_ve;
    let mut flags: &mut u32 = &mut opt_vars().ve_flags;

    if (args.os_flags & OPT_LOCAL) != 0 {
        ve = win.w_onebuf_opt.wo_ve;
        flags = &mut win.w_onebuf_opt.wo_ve_flags;
    }

    if (args.os_flags & OPT_LOCAL) != 0 && crate::memory::cstr_is_empty(ve) {
        *flags = 0;
    } else {
        let errmsg = opt_strings_flags(ve, &OPT_VE_VALUES, flags, true, args.os_errbuf);
        if errmsg.is_some() {
            return errmsg;
        } else if crate::memory::cstr_as_bytes(ve)
            != crate::memory::cstr_as_bytes(args.os_oldval.data.string().data)
        {
            validate_virtcol(win);
            let vc = win.w_virtcol;
            coladvance(win, vc);
        }
    }
    None
}

/// Validate the 'whichwrap' option.
pub fn validate_whichwrap(args: &mut Optset) -> Option<&CStr> {
    check_option_listflag(
        args.os_newval.data.string().data,
        WW_ALL_COMMA,
        args.os_errbuf,
    )
}

pub fn expand_set_whichwrap(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, WW_ALL, matches)
}

pub fn validate_wildmode(args: &mut Optset) -> Option<&CStr> {
    let mut flags = [0u8; 4];
    if !check_opt_wim(args.os_newval.data.string().as_bytes(), &mut flags) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Wildmode(flags);
    None
}

/// The 'wildmode' option is changed.
pub fn did_set_wildmode(args: &mut Optset) -> Option<&CStr> {
    if let OptSetPrep::Wildmode(flags) = args.os_prep {
        g_st().wim_flags = flags;
    }
    None
}

/// The 'winbar' option is changed.
pub fn did_set_winbar(args: &mut Optset) -> Option<&CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// Validate the 'winborder' or 'pumborder' option.
pub fn validate_border(args: &mut Optset) -> Option<&CStr> {
    let mut fconfig = WIN_CONFIG_INIT;
    let mut err = Error::new();
    let ok = parse_winborder(
        Some(&mut fconfig),
        args.os_newval.data.string().cstr_bytes(),
        &mut err,
    );
    api_clear_error(&mut err);
    if ok { None } else { Some(E_INVARG) }
}

/// The 'winhighlight' option is changed.
pub fn did_set_winhighlight(args: &mut Optset) -> Option<&CStr> {
    let win = args.win_mut();
    let varp = args.os_varp.cast::<*mut u8>();
    let is_local = ptr::eq(varp, (&raw mut win.w_onebuf_opt.wo_winhl).cast());
    if !parse_winhl_opt(args.var_cstr_ptr(), is_local.then_some(win)) {
        return Some(E_INVARG);
    }
    None
}

pub fn expand_set_winhighlight(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_generic(args, Some(get_highlight_name), matches)
}

/// Format an error naming an item (at most 63 bytes) and listing its valid values.
fn opt_values_err<'a>(
    errbuf: Option<CharBuf>,
    fmt: &std::ffi::CStr,
    item: &[u8],
    values: &[&CStr],
) -> Option<&'a CStr> {
    let Some(errbuf) = errbuf else {
        return Some(E_INVARG);
    };
    let mut itembuf = [0u8; 64];
    let copy_len = item.len().min(itembuf.len() - 1);
    itembuf[..copy_len].copy_from_slice(&item[..copy_len]);
    let mut n = vim_snprintf!(
        errbuf.data.cast(),
        errbuf.size,
        gettext(fmt.as_ptr()),
        itembuf.as_ptr(),
    );
    for (j, &v) in values.iter().enumerate() {
        if n <= 0 || (n as usize) >= errbuf.size {
            break;
        }
        n += vim_snprintf!(
            errbuf.data.wrapping_add(n as usize).cast(),
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
    Some(errbuf.as_cstr())
}

/// Handle an option that can be a range of string values.
fn opt_strings_flags<'a>(
    val: impl crate::strings::IntoCCharPtr,
    values: &[&CStr],
    flagp: *mut u32,
    list: bool,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let val_start = val.into_c_char_ptr();
    let val_bytes = crate::memory::cstr_as_bytes(val_start);
    let mut rem = val_bytes;
    let mut new_flags: u32 = 0;
    let iter_one = rem.is_empty() && !list;

    while !rem.is_empty() || iter_one {
        let mut matched = false;
        for (i, &v) in values.iter().enumerate() {
            let v_bytes = v.to_bytes();
            let len = v_bytes.len();
            if rem.starts_with(v_bytes)
                && ((list && rem.get(len) == Some(&b',')) || rem.len() == len)
            {
                let step = len + usize::from(rem.get(len) == Some(&b','));
                rem = &rem[step..];
                debug_assert!(i < std::mem::size_of::<u32>() * 8);
                new_flags |= 1u32 << i;
                matched = true;
                break;
            }
        }
        if !matched {
            let item_len = rem.iter().position(|&b| b == b',').unwrap_or(rem.len());
            return opt_values_err(
                errbuf,
                c"E474: Invalid value '%s', expected one of:",
                &rem[..item_len],
                values,
            );
        }
        if iter_one {
            break;
        }
    }
    if !flagp.is_null() {
        unsafe {
            *flagp = new_flags;
        }
    }

    None
}

/// Format a schema-validation error naming the offending key.
fn opt_schema_err<'a>(
    errbuf: Option<CharBuf>,
    fmt: &std::ffi::CStr,
    key: &[u8],
) -> Option<&'a CStr> {
    errbuf?;
    let mut keybuf = [0u8; 64];
    let copy_len = key.len().min(keybuf.len() - 1);
    keybuf[..copy_len].copy_from_slice(&key[..copy_len]);
    opt_error(errbuf, fmt.as_ptr(), crate::memory::cstr_until_nul(&keybuf))
}

/// Validates a dict option against its schema (options.lua).
pub fn opt_strings_check<'a>(
    val: impl crate::strings::IntoCCharPtr,
    schema: &[OptSchemaItem],
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let mut p = crate::memory::cstr_as_bytes(val);
    while let Some((key, v)) = option_next_keyval(&mut p) {
        if key.is_empty() && v.is_none() {
            continue;
        }
        let Some(it) = schema.iter().find(|item| item.name.to_bytes() == key) else {
            return opt_schema_err(errbuf, c"E474: Unknown item '%s'", key);
        };
        match it.kind {
            OptSchemaKind::OptSchemaFlag => {
                if v.is_some() {
                    return opt_schema_err(errbuf, c"E474: '%s' does not take a value", key);
                }
            }
            OptSchemaKind::OptSchemaNum | OptSchemaKind::OptSchemaSNum => {
                let Some(v_bytes) = v else {
                    return opt_schema_err(errbuf, c"E474: '%s' requires a number", key);
                };
                let digits = if it.kind == OptSchemaKind::OptSchemaSNum {
                    v_bytes.strip_prefix(b"-").unwrap_or(v_bytes)
                } else {
                    v_bytes
                };
                if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                    return opt_schema_err(errbuf, c"E474: '%s' requires a number", key);
                }
                let mut rem = v_bytes;
                let mut n: i64 = 0;
                if !try_getdigits(&mut rem, &mut n)
                    || n < i64::from(i32::MIN)
                    || n > i64::from(i32::MAX)
                {
                    return opt_schema_err(errbuf, c"E474: '%s' number is out of range", key);
                }
            }
            OptSchemaKind::OptSchemaEnum => {
                if let Some(v_bytes) = v
                    && it.enum_values.iter().any(|&ev| ev.to_bytes() == v_bytes)
                {
                    continue;
                }
                return opt_values_err(errbuf, c"E474: '%s' must be one of:", key, it.enum_values);
            }
            OptSchemaKind::OptSchemaStr => {
                let Some(v_bytes) = v else {
                    return opt_schema_err(errbuf, c"E474: '%s' requires a value", key);
                };
                if v_bytes.len() >= 256 {
                    return opt_schema_err(errbuf, c"E474: '%s' value is too long", key);
                }
            }
        }
    }
    None
}

/// Parses a validated dict option into `out`.
pub fn opt_fill(
    value: impl crate::strings::IntoCCharPtr,
    get_field: FieldHashfn,
    out: *mut OptKeySet,
) {
    let Some(get_field_fn) = get_field else {
        return;
    };
    let mut p = crate::memory::cstr_as_bytes(value);
    while let Some((key_bytes, v)) = option_next_keyval(&mut p) {
        let Some(f) = get_field_fn(key_bytes) else {
            continue;
        };
        let field = out.cast::<u8>().wrapping_add(f.ptr_off);
        if f.key_type == ObjectType::ObjectTypeBoolean as i32 {
            unsafe {
                *field.cast::<Boolean>() = true;
            }
        } else if f.key_type == ObjectType::ObjectTypeInteger as i32 {
            let mut rem = v.unwrap_or(&[]);
            unsafe {
                *field.cast::<Integer>() = Integer::from(getdigits_int(&mut rem, false, 0));
            }
        } else if f.key_type == ObjectType::ObjectTypeString as i32 {
            let v_bytes = v.unwrap_or(&[]);
            unsafe { ptr::copy_nonoverlapping(v_bytes.as_ptr(), field, v_bytes.len()) };
            unsafe {
                *field.wrapping_add(v_bytes.len()) = 0;
            }
        }
        unsafe {
            (*out).is_set_ |= 1u64 << (f.opt_index as u32);
        }
    }
}

/// Reify a dict-option value to a keyset (`OptKeyDict_…`).
pub fn opt_keyset(
    value: impl crate::strings::IntoCCharPtr,
    opt_idx: OptIndex,
    mut keyset: *mut OptKeySet,
) -> *mut OptKeySet {
    let value = value.into_c_char_ptr();
    {
        if keyset.is_null() {
            keyset = (&raw mut optstr_st().opt_keyset_shared).cast::<OptKeySet>();
        }
        let si = opt_dict_schema(opt_idx).unwrap();
        unsafe { ptr::write_bytes(keyset.cast::<u8>(), 0, si.size) };
        if !value.is_null() {
            opt_fill(value, si.get_field, keyset);
        }
        keyset
    }
}

/// Return OK if "p" is a valid fileformat name, FAIL otherwise.
pub fn check_ff_value(p: impl crate::strings::IntoCCharPtr) -> bool {
    let p = p.into_c_char_ptr().cast_mut();
    opt_strings_flags(p, &OPT_FF_VALUES, ptr::null_mut(), false, None).is_none()
}

/// Calls mb_cptr2char_adv(p) and returns the character.
fn get_encoded_char_adv(p: &mut crate::strings::RawCCharPtr) -> Schar {
    let s = *p;

    let s_bytes = crate::memory::cstr_as_bytes(s);
    if s_bytes.first() == Some(&b'\\')
        && let Some(&b1 @ (b'x' | b'u' | b'U')) = s_bytes.get(1)
    {
        let mut num: i64 = 0;
        let mut bytes = if b1 == b'x' {
            1
        } else if b1 == b'u' {
            2
        } else {
            4
        };
        while bytes > 0 {
            *p = p.wrapping_add(2);
            let n = hexhex2nr(crate::memory::cstr_as_bytes(*p));
            if n < 0 {
                return 0;
            }
            num = num * 256 + i64::from(n);
            bytes -= 1;
        }
        *p = p.wrapping_add(2);
        return if char2cells(num as i32) > 1 {
            0
        } else {
            schar_from_char(num as i32)
        };
    }

    let ci = utf_ptr2_str_char_info(s);
    let cli = utf_cluster_info(ci);
    *p = cli.next.ptr;
    if cli.cells > 1 {
        0
    } else {
        schar_from_cluster(ci, cli)
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
    fn slot_mut<'a>(self, fcs: &'a mut FcsChars, lcs: &'a mut LcsChars) -> Option<&'a mut Schar> {
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
pub fn set_chars_option<'a>(
    wp: &mut WinInfoInfo,
    value: impl crate::strings::IntoCCharPtr,
    what: CharsOption,
    apply: bool,
    errbuf: Option<CharBuf>,
) -> Option<&'a CStr> {
    let mut value = value.into_c_char_ptr();
    {
        let mut last_multispace = ptr::null();
        let mut last_lmultispace = ptr::null();
        let mut multispace_len: usize = 0;
        let mut lead_multispace_len: usize = 0;
        let mut fcs_chars = FcsChars::default();
        let mut lcs_chars = LcsChars::new();

        let tab: &[CharsTabEntry] = if what == CharsOption::Listchars {
            if apply && crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_lcs) {
                value = opt_vars().p_lcs;
            }
            &LCS_TAB
        } else {
            if apply && crate::memory::cstr_is_empty(wp.w_onebuf_opt.wo_fcs) {
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

                if what == CharsOption::Listchars {
                    lcs_chars.tab1 = 0;
                    lcs_chars.tab3 = 0;
                    lcs_chars.leadtab1 = 0;
                    lcs_chars.leadtab3 = 0;
                    lcs_chars.multispace = vec![0; multispace_len];
                    lcs_chars.leadmultispace = vec![0; lead_multispace_len];
                }
            }

            let mut p = value;
            while !crate::memory::cstr_is_empty(p) {
                let p_bytes = crate::memory::cstr_as_bytes(p);
                let mut i: usize = 0;
                while i < entries {
                    let name_ptr = tab[i].name.as_ptr();
                    let name_bytes = tab[i].name.to_bytes();
                    let name_len = name_bytes.len();
                    if !(p_bytes.starts_with(name_bytes) && p_bytes.get(name_len) == Some(&b':')) {
                        i += 1;
                        continue;
                    }

                    let mut s = p.wrapping_add(name_len + 1);

                    if tab[i].target == CharsTarget::LcsMultispace {
                        if round == 0 {
                            last_multispace = p;
                            multispace_len = 0;
                            while !crate::memory::cstr_is_empty(s)
                                && crate::memory::cstr_first_byte(s) != b','
                            {
                                let c1 = get_encoded_char_adv(&mut s);
                                if c1 == 0 {
                                    return opt_error(
                                        errbuf,
                                        E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                        name_ptr,
                                    );
                                }
                                multispace_len += 1;
                            }
                            if multispace_len == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                                    name_ptr,
                                );
                            }
                        } else {
                            let mut multispace_pos: usize = 0;
                            while !crate::memory::cstr_is_empty(s)
                                && crate::memory::cstr_first_byte(s) != b','
                            {
                                let c1 = get_encoded_char_adv(&mut s);
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
                            while !crate::memory::cstr_is_empty(s)
                                && crate::memory::cstr_first_byte(s) != b','
                            {
                                let c1 = get_encoded_char_adv(&mut s);
                                if c1 == 0 {
                                    return opt_error(
                                        errbuf,
                                        E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                        name_ptr,
                                    );
                                }
                                lead_multispace_len += 1;
                            }
                            if lead_multispace_len == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                                    name_ptr,
                                );
                            }
                        } else {
                            let mut multispace_pos: usize = 0;
                            while !crate::memory::cstr_is_empty(s)
                                && crate::memory::cstr_first_byte(s) != b','
                            {
                                let c1 = get_encoded_char_adv(&mut s);
                                if p == last_lmultispace {
                                    lcs_chars.leadmultispace[multispace_pos] = c1;
                                    multispace_pos += 1;
                                }
                            }
                        }
                        p = s;
                        break;
                    }

                    if crate::memory::cstr_is_empty(s) {
                        return opt_error(
                            errbuf,
                            E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                            name_ptr,
                        );
                    }
                    let c1 = get_encoded_char_adv(&mut s);
                    if c1 == 0 {
                        return opt_error(errbuf, E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR, name_ptr);
                    }
                    let mut c2: Schar = 0;
                    let mut c3: Schar = 0;
                    if matches!(tab[i].target, CharsTarget::LcsTab | CharsTarget::LcsLeadtab) {
                        if crate::memory::cstr_is_empty(s) {
                            return opt_error(
                                errbuf,
                                E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                                name_ptr,
                            );
                        }
                        c2 = get_encoded_char_adv(&mut s);
                        if c2 == 0 {
                            return opt_error(
                                errbuf,
                                E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                name_ptr,
                            );
                        }
                        if !(crate::memory::cstr_first_byte(s) == b','
                            || crate::memory::cstr_is_empty(s))
                        {
                            c3 = get_encoded_char_adv(&mut s);
                            if c3 == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
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

                    if crate::memory::cstr_first_byte(s) == b',' || crate::memory::cstr_is_empty(s)
                    {
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
                            E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                            name_ptr,
                        );
                    }
                }

                if i == entries {
                    return Some(E_INVARG);
                }

                if crate::memory::cstr_first_byte(p) == b',' {
                    p = p.wrapping_add(1);
                }
            }

            if what == CharsOption::Listchars && has_leadtab && !has_tab {
                return Some(E_LEADTAB_REQUIRES_TAB);
            }
        }

        if apply {
            if what == CharsOption::Listchars {
                wp.w_p_lcs_chars = lcs_chars;
            } else {
                wp.w_p_fcs_chars = fcs_chars;
            }
        }

        None
    }
}

/// Function given to ExpandGeneric() to obtain possible arguments of the 'fillchars' option.
pub fn get_fillchars_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx >= 0)
        .then(|| FCS_TAB.get(idx as usize).map(|e| e.name))
        .flatten()
}

/// Function given to ExpandGeneric() to obtain possible arguments of the 'listchars' option.
pub fn get_listchars_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx >= 0)
        .then(|| LCS_TAB.get(idx as usize).map(|e| e.name))
        .flatten()
}

/// Check all global and local values of 'listchars' and 'fillchars'.
pub fn check_chars_options() -> Option<&'static CStr> {
    if set_chars_option(
        curwin_mut(),
        opt_vars().p_lcs,
        CharsOption::Listchars,
        false,
        None,
    )
    .is_some()
    {
        return Some(E_CONFLICTS_WITH_VALUE_OF_LISTCHARS);
    }
    if set_chars_option(
        curwin_mut(),
        opt_vars().p_fcs,
        CharsOption::Fillchars,
        false,
        None,
    )
    .is_some()
    {
        return Some(E_CONFLICTS_WITH_VALUE_OF_FILLCHARS);
    }
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&*tp) {
            if set_chars_option(
                wp,
                wp.w_onebuf_opt.wo_lcs,
                CharsOption::Listchars,
                true,
                None,
            )
            .is_some()
            {
                return Some(E_CONFLICTS_WITH_VALUE_OF_LISTCHARS);
            }
            if set_chars_option(
                wp,
                wp.w_onebuf_opt.wo_fcs,
                CharsOption::Fillchars,
                true,
                None,
            )
            .is_some()
            {
                return Some(E_CONFLICTS_WITH_VALUE_OF_FILLCHARS);
            }
        }
    }
    None
}

pub fn validate_previewpopup(args: &mut Optset) -> Option<&CStr> {
    let mut fconfig = WIN_CONFIG_INIT;
    let cstr = crate::memory::cstr_from_ptr(args.os_newval.data.string().data);
    if !win_previewpopup_config(cstr, &mut fconfig) {
        return Some(E_INVARG);
    }
    None
}

pub fn expand_set_popupoption(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let xp_pat = args.xp_mut().xp_pattern;

    if (xp_pat as usize) > (args.oe_set_arg as usize)
        && crate::memory::cstr_first_byte(xp_pat.wrapping_sub(1).cast()) == b':'
    {
        if completing_value_for_subopt(args, c"border") {
            return expand_set_opt_string(args, &OPT_PVP_BORDER_VALUES, matches);
        }
        return false;
    }

    expand_set_opt_string(args, &OPT_PVP_VALUES, matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ff_value_accepts_valid_and_rejects_invalid() {
        assert!(check_ff_value(c"unix"));
        assert!(check_ff_value(c"dos"));
        assert!(check_ff_value(c"mac"));
        assert!(!check_ff_value(c""));
        assert!(!check_ff_value(c"foo"));
    }
}
