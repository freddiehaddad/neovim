//! String option validation, handling, and completion (`src/nvim/optionstr.c`).
#![forbid(unsafe_code)]

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
use crate::lua::converter::Boolean;
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
use crate::strings::{format_to_buf, gettext, vim_strchr};
use crate::terminal::terminal_notify_theme;
use crate::ui::ui_has;
use crate::window::{FcsChars, LcsChars, WinInfoInfo, check_colorcolumn, global_stl_height};
use crate::winfloat::{win_config_float, win_previewpopup_config};
use std::ffi::CStr;
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
pub struct OptSchemaItem {
    pub name: &'static std::ffi::CStr,
    pub kind: OptSchemaKind,
    pub enum_values: &'static [&'static std::ffi::CStr],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptDictSchema {
    pub schema: &'static [OptSchemaItem],
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

pub enum OptRef<T: 'static> {
    Global(fn(&OptionVars) -> &T),
    Win(crate::globals::WinPtr, fn(&WinInfoInfo) -> &T),
    WinSyn(crate::globals::WinPtr, fn(&Synblock) -> &T),
    Buf(crate::globals::BufPtr, fn(&FileBuf) -> &T),
}

impl<T: 'static> Clone for OptRef<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for OptRef<T> {}

impl<T: 'static> OptRef<T> {
    #[inline]
    pub fn with<R>(self, cb: impl FnOnce(&T) -> R) -> R {
        match self {
            Self::Global(f) => cb(f(opt_vars())),
            Self::Win(w, f) => cb(f(w.borrow())),
            Self::WinSyn(w, f) => {
                let s = w.borrow().w_s_ptr();
                cb(f(&s))
            }
            Self::Buf(b, f) => cb(f(b.borrow())),
        }
    }

    #[inline]
    pub fn as_ptr(self) -> *const () {
        self.with(|r| std::ptr::from_ref(r).cast::<()>())
    }
}

impl<T: 'static> PartialEq for OptRef<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.as_ptr(), other.as_ptr())
    }
}

impl<T: 'static> Eq for OptRef<T> {}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum OptVar {
    #[default]
    None,
    Bool(OptRef<std::cell::Cell<bool>>),
    I32Bool(OptRef<std::cell::Cell<i32>>),
    OptBool(OptRef<std::cell::Cell<Option<bool>>>),
    Int(OptRef<std::cell::Cell<crate::option::OptInt>>),
    Str(OptRef<OptString>),
    Callback(OptRef<crate::eval::typval::Callback>),
    DefVal(OptIndex),
}

impl OptVar {
    #[inline]
    pub const fn is_some(self) -> bool {
        !matches!(self, Self::None)
    }

    #[inline]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    #[inline]
    pub fn ptr_eq<T>(self, other: &T) -> bool {
        let p = std::ptr::from_ref(other).cast::<()>();
        match self {
            Self::None => false,
            Self::Bool(g) => std::ptr::eq(g.as_ptr(), p),
            Self::I32Bool(g) => std::ptr::eq(g.as_ptr(), p),
            Self::OptBool(g) => std::ptr::eq(g.as_ptr(), p),
            Self::Int(g) => std::ptr::eq(g.as_ptr(), p),
            Self::Str(g) => std::ptr::eq(g.as_ptr(), p),
            Self::Callback(g) => std::ptr::eq(g.as_ptr(), p),
            Self::DefVal(_) => false,
        }
    }

    #[inline]
    pub fn as_cstr(self) -> &'static CStr {
        match self {
            Self::Str(s) => s.with(|v| v.as_cstr()),
            _ => c"",
        }
    }

    #[inline]
    pub fn opt_cstr(self) -> Option<&'static CStr> {
        match self {
            Self::Str(s) => Some(s.with(|v| v.as_cstr())),
            _ => None,
        }
    }

    #[inline]
    pub fn set_cstr(self, val: &CStr) {
        if let Self::Str(s) = self {
            s.with(|v| v.set_option_cstr(val));
        }
    }

    #[inline]
    pub fn set_bytes(self, bytes: &[u8]) {
        if let Self::Str(s) = self {
            s.with(|v| v.set_option_bytes(bytes));
        }
    }

    #[inline]
    pub fn clear_str(self) {
        if let Self::Str(s) = self {
            s.with(|v| v.clear_option_str());
        }
    }

    #[inline]
    pub fn check_str(self) {
        if let Self::Str(s) = self {
            s.with(|v| v.check_option_str());
        }
    }

    #[inline]
    pub fn int_val(self) -> crate::option::OptInt {
        match self {
            Self::Int(p) => p.with(|v| v.get()),
            _ => 0,
        }
    }

    #[inline]
    pub fn set_int(self, val: crate::option::OptInt) {
        if let Self::Int(p) = self {
            p.with(|v| v.set(val));
        }
    }

    #[inline]
    pub fn bool_tristate(self) -> i32 {
        match self {
            Self::Bool(p) => i32::from(p.with(|v| v.get())),
            Self::I32Bool(p) => i32::from(p.with(|v| v.get()) != 0),
            Self::OptBool(p) => match p.with(|v| v.get()) {
                Some(b) => i32::from(b),
                None => crate::extmark::K_NONE,
            },
            _ => 0,
        }
    }

    #[inline]
    pub fn set_bool(self, val: Option<bool>) {
        match self {
            Self::Bool(p) => p.with(|v| v.set(val.unwrap())),
            Self::I32Bool(p) => p.with(|v| v.set(i32::from(val.unwrap()))),
            Self::OptBool(p) => p.with(|v| v.set(val)),
            _ => {}
        }
    }

    #[inline]
    pub fn with_callback<R>(self, f: impl FnOnce(&crate::eval::typval::Callback) -> R) -> R {
        match self {
            Self::Callback(p) => p.with(f),
            _ => unreachable!(),
        }
    }
}

impl From<OptRef<std::cell::Cell<bool>>> for OptVar {
    #[inline]
    fn from(r: OptRef<std::cell::Cell<bool>>) -> Self {
        Self::Bool(r)
    }
}

impl From<OptRef<std::cell::Cell<i32>>> for OptVar {
    #[inline]
    fn from(r: OptRef<std::cell::Cell<i32>>) -> Self {
        Self::I32Bool(r)
    }
}

impl From<OptRef<std::cell::Cell<Option<bool>>>> for OptVar {
    #[inline]
    fn from(r: OptRef<std::cell::Cell<Option<bool>>>) -> Self {
        Self::OptBool(r)
    }
}

impl From<OptRef<std::cell::Cell<crate::option::OptInt>>> for OptVar {
    #[inline]
    fn from(r: OptRef<std::cell::Cell<crate::option::OptInt>>) -> Self {
        Self::Int(r)
    }
}

impl From<OptRef<OptString>> for OptVar {
    #[inline]
    fn from(r: OptRef<OptString>) -> Self {
        Self::Str(r)
    }
}

impl From<OptRef<crate::eval::typval::Callback>> for OptVar {
    #[inline]
    fn from(r: OptRef<crate::eval::typval::Callback>) -> Self {
        Self::Callback(r)
    }
}

#[repr(C)]
pub struct Optset<'a> {
    pub os_varp: OptVar,
    pub os_idx: OptIndex,
    pub os_flags: i32,
    pub os_oldval: Object,
    pub os_newval: Object,
    pub os_prep: OptSetPrep,
    pub os_value_checked: bool,
    pub os_value_changed: bool,
    pub os_restore_chartab: bool,
    pub os_errbuf: Option<&'a mut [u8]>,
    pub os_win: crate::globals::WinPtr,
    pub os_buf: crate::globals::BufPtr,
}

impl<'a> Optset<'a> {
    pub const fn new() -> Self {
        Self {
            os_varp: OptVar::None,
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
            os_win: crate::globals::WinPtr::NULL,
            os_buf: crate::globals::BufPtr::NULL,
        }
    }

    #[inline]
    pub fn buf_mut(&self) -> &'static mut FileBuf {
        self.os_buf.borrow_mut()
    }

    #[inline]
    pub fn win_mut(&self) -> &'static mut WinInfoInfo {
        self.os_win.borrow_mut()
    }

    #[inline]
    pub fn win_opt(&self) -> Option<&'static mut WinInfoInfo> {
        self.os_win.opt_mut()
    }

    #[inline]
    pub fn var_cstr(&self) -> &'static CStr {
        self.os_varp.as_cstr()
    }

    #[inline]
    pub fn set_var_cstr(&mut self, val: &CStr) {
        self.os_varp.set_cstr(val);
    }
}

#[repr(C)]
pub struct Optexpand<'a> {
    pub oe_idx: OptIndex,
    pub oe_opt_value: &'a std::ffi::CStr,
    pub oe_append: bool,
    pub oe_include_orig_val: bool,
    pub oe_regmatch: &'a mut Regmatch,
    pub oe_xp: &'a mut ExpandCtx,
    pub oe_set_arg: crate::strings::OwnedSlice,
}

pub type OptValidateCb = Option<for<'a> fn(&mut Optset<'a>) -> Option<&'a CStr>>;
pub type OptDidSetCb = Option<for<'a> fn(&mut Optset<'a>) -> Option<&'a CStr>>;
pub type OptExpandCb = Option<fn(&mut Optexpand, &mut Vec<std::ffi::CString>) -> bool>;

#[repr(C)]
#[derive(Clone)]
pub struct Vimoption {
    pub fullname: &'static std::ffi::CStr,
    pub shortname: Option<&'static std::ffi::CStr>,
    pub flags: u32,
    pub opt_type: ObjectType,
    pub scope_flags: OptScopeFlags,
    pub var: Option<fn() -> OptVar>,
    pub flags_var: Option<fn() -> &'static std::cell::Cell<u32>>,
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

impl Vimoption {
    #[inline]
    pub fn var(&self) -> OptVar {
        self.var.map_or(OptVar::None, |f| f())
    }

    #[inline]
    pub fn flags_var_mut(&self) -> Option<&'static std::cell::Cell<u32>> {
        self.flags_var.map(|f| f())
    }
}

use crate::decoration::SCL_NUM;
use crate::ex_cmds::B_IMODE_LMAP;
use crate::ex_session::{K_OPT_SSOP_FLAG_CURDIR, K_OPT_SSOP_FLAG_SESDIR};
use crate::fileio::K_OPT_FILEENCODING;
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::runtime::SID_NONE;

pub trait OptKeyDict: Default {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>);
}

#[inline]
fn parse_opt_int(v: Option<&[u8]>) -> Integer {
    let mut rem = v.unwrap_or(&[]);
    Integer::from(getdigits_int(&mut rem, false, 0))
}

#[inline]
fn copy_opt_str<const N: usize>(dst: &mut [u8; N], v: Option<&[u8]>) {
    let src = v.unwrap_or(&[]);
    let len = src.len().min(N - 1);
    dst[..len].copy_from_slice(&src[..len]);
    dst[len] = 0;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OptKeyDict_briopt {
    pub is_set_briopt: OptionalKeys,
    pub shift: Integer,
    pub min: Integer,
    pub sbr: Boolean,
    pub list: Integer,
    pub column: Integer,
}

impl OptKeyDict for OptKeyDict_briopt {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>) {
        let idx = match key {
            b"min" => {
                self.min = parse_opt_int(v);
                1
            }
            b"sbr" => {
                self.sbr = true;
                2
            }
            b"list" => {
                self.list = parse_opt_int(v);
                3
            }
            b"shift" => {
                self.shift = parse_opt_int(v);
                4
            }
            b"column" => {
                self.column = parse_opt_int(v);
                5
            }
            _ => return,
        };
        self.is_set_briopt |= 1u64 << idx;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
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

impl OptKeyDict for OptKeyDict_dip {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>) {
        let idx = match key {
            b"icase" => {
                self.icase = true;
                1
            }
            b"iblank" => {
                self.iblank = true;
                2
            }
            b"filler" => {
                self.filler = true;
                3
            }
            b"anchor" => {
                self.anchor = true;
                4
            }
            b"inline" => {
                copy_opt_str(&mut self.inline_, v);
                5
            }
            b"iwhite" => {
                self.iwhite = true;
                6
            }
            b"context" => {
                self.context = parse_opt_int(v);
                7
            }
            b"closeoff" => {
                self.closeoff = true;
                8
            }
            b"internal" => {
                self.internal = true;
                9
            }
            b"vertical" => {
                self.vertical = true;
                10
            }
            b"linematch" => {
                self.linematch = parse_opt_int(v);
                11
            }
            b"hiddenoff" => {
                self.hiddenoff = true;
                12
            }
            b"algorithm" => {
                copy_opt_str(&mut self.algorithm, v);
                13
            }
            b"iwhiteall" => {
                self.iwhiteall = true;
                14
            }
            b"iwhiteeol" => {
                self.iwhiteeol = true;
                15
            }
            b"foldcolumn" => {
                self.foldcolumn = parse_opt_int(v);
                16
            }
            b"horizontal" => {
                self.horizontal = true;
                17
            }
            b"followwrap" => {
                self.followwrap = true;
                18
            }
            b"indent-heuristic" => {
                self.indent_heuristic = true;
                19
            }
            _ => return,
        };
        self.is_set_dip |= 1u64 << idx;
    }
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

impl Default for OptKeyDict_mopt {
    fn default() -> Self {
        Self {
            is_set_mopt: 0,
            hit_enter: false,
            history: 0,
            maxheight: 0,
            pager: [0; 256],
            progress: [0; 2],
            timeout: 0,
            wait: 0,
        }
    }
}

impl OptKeyDict for OptKeyDict_mopt {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>) {
        let idx = match key {
            b"wait" => {
                self.wait = parse_opt_int(v);
                1
            }
            b"pager" => {
                copy_opt_str(&mut self.pager, v);
                2
            }
            b"history" => {
                self.history = parse_opt_int(v);
                3
            }
            b"timeout" => {
                self.timeout = parse_opt_int(v);
                4
            }
            b"progress" => {
                copy_opt_str(&mut self.progress, v);
                5
            }
            b"hit-enter" => {
                self.hit_enter = true;
                6
            }
            b"maxheight" => {
                self.maxheight = parse_opt_int(v);
                7
            }
            _ => return,
        };
        self.is_set_mopt |= 1u64 << idx;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OptKeyDict_mousescroll {
    pub is_set_mousescroll: OptionalKeys,
    pub hor: Integer,
    pub ver: Integer,
}

impl OptKeyDict for OptKeyDict_mousescroll {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>) {
        let idx = match key {
            b"hor" => {
                self.hor = parse_opt_int(v);
                1
            }
            b"ver" => {
                self.ver = parse_opt_int(v);
                2
            }
            _ => return,
        };
        self.is_set_mousescroll |= 1u64 << idx;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OptKeyDict_pvp {
    pub is_set_pvp: OptionalKeys,
    pub height: Integer,
    pub width: Integer,
    pub border: [u8; 8],
}

impl OptKeyDict for OptKeyDict_pvp {
    fn set_field(&mut self, key: &[u8], v: Option<&[u8]>) {
        let idx = match key {
            b"width" => {
                self.width = parse_opt_int(v);
                1
            }
            b"border" => {
                copy_opt_str(&mut self.border, v);
                2
            }
            b"height" => {
                self.height = parse_opt_int(v);
                3
            }
            _ => return,
        };
        self.is_set_pvp |= 1u64 << idx;
    }
}

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
    set_opt_callback_func: std::cell::Cell<CompleteListItemGetter>,
    expand_eiw: std::cell::Cell<bool>,
}

impl OptionStrState {
    const fn new() -> Self {
        Self {
            set_opt_callback_func: std::cell::Cell::new(None),
            expand_eiw: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static OPTSTR_STATE: OptionStrState = const { OptionStrState::new() };
    static SET_OPT_CALLBACK_ORIG: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
}

use crate::mbyte::ptr2cells;

pub(crate) fn opt_dict_schema(opt_idx: OptIndex) -> Option<&'static OptDictSchema> {
    static INFO_BRIOPT: OptDictSchema = OptDictSchema {
        schema: &OPT_BRIOPT_SCHEMA,
    };
    static INFO_DIP: OptDictSchema = OptDictSchema {
        schema: &OPT_DIP_SCHEMA,
    };
    static INFO_MOPT: OptDictSchema = OptDictSchema {
        schema: &OPT_MOPT_SCHEMA,
    };
    static INFO_MOUSESCROLL: OptDictSchema = OptDictSchema {
        schema: &OPT_MOUSESCROLL_SCHEMA,
    };
    static INFO_PVP: OptDictSchema = OptDictSchema {
        schema: &OPT_PVP_SCHEMA,
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
    check_str_opt(K_OPT_CASEMAP, None, None);
    check_str_opt(K_OPT_BACKUPCOPY, None, None);
    check_str_opt(K_OPT_BELLOFF, None, None);
    check_str_opt(K_OPT_COMPLETEOPT, None, None);
    check_str_opt(K_OPT_SESSIONOPTIONS, None, None);
    check_str_opt(K_OPT_VIEWOPTIONS, None, None);
    check_str_opt(K_OPT_FOLDOPEN, None, None);
    check_str_opt(K_OPT_DISPLAY, None, None);
    check_str_opt(K_OPT_JUMPOPTIONS, None, None);
    check_str_opt(K_OPT_REDRAWDEBUG, None, None);
    check_str_opt(K_OPT_TAGCASE, None, None);
    check_str_opt(K_OPT_TERMPASTEFILTER, None, None);
    check_str_opt(K_OPT_VIRTUALEDIT, None, None);
    check_str_opt(K_OPT_SWITCHBUF, None, None);
    check_str_opt(K_OPT_TABCLOSE, None, None);
    check_str_opt(K_OPT_WILDOPTIONS, None, None);
    check_str_opt(K_OPT_CLIPBOARD, None, None);
}

fn illegal_char(errbuf: Option<&mut [u8]>, c: i32) -> Option<&CStr> {
    if errbuf.is_none() {
        return Some(c"");
    }
    opt_error(errbuf, c"E539: Illegal character <%s>", &transchar(c))
}

/// Check string options in a buffer for NULL value.
pub fn check_buf_options(buf: &mut FileBuf) {
    buf.b_p_bh.check_option_str();
    buf.b_p_bt.check_option_str();
    buf.b_p_fenc.check_option_str();
    buf.b_p_ff.check_option_str();
    buf.b_p_def.check_option_str();
    buf.b_p_inc.check_option_str();
    buf.b_p_indk.check_option_str();
    buf.b_p_fp.check_option_str();
    buf.b_p_kp.check_option_str();
    buf.b_p_mps.check_option_str();
    buf.b_p_fo.check_option_str();
    buf.b_p_flp.check_option_str();
    buf.b_p_isk.check_option_str();
    buf.b_p_com.check_option_str();
    buf.b_p_cms.check_option_str();
    buf.b_p_nf.check_option_str();
    buf.b_p_qe.check_option_str();
    buf.b_p_syn.check_option_str();
    buf.b_s.b_syn_isk.set(None);
    buf.b_s.b_p_spc.check_option_str();
    buf.b_s.b_p_spf.check_option_str();
    buf.b_s.b_p_spl.check_option_str();
    buf.b_s.b_p_spo.check_option_str();
    buf.b_p_sua.check_option_str();
    buf.b_p_cink.check_option_str();
    buf.b_p_cino.check_option_str();
    parse_cino(buf);
    buf.b_p_lop.check_option_str();
    buf.b_p_ft.check_option_str();
    buf.b_p_cinw.check_option_str();
    buf.b_p_cinsd.check_option_str();
    buf.b_p_cot.check_option_str();
    buf.b_p_cpt.check_option_str();
    buf.b_p_keymap.check_option_str();
    buf.b_p_gefm.check_option_str();
    buf.b_p_gp.check_option_str();
    buf.b_p_mp.check_option_str();
    buf.b_p_efm.check_option_str();
    buf.b_p_ep.check_option_str();
    buf.b_p_path.check_option_str();
    buf.b_p_tags.check_option_str();
    buf.b_p_tc.check_option_str();
    buf.b_p_dict.check_option_str();
    buf.b_p_dia.check_option_str();
    buf.b_p_tsr.check_option_str();
    buf.b_p_lw.check_option_str();
    buf.b_p_bkc.check_option_str();
    buf.b_p_menc.check_option_str();
    buf.b_p_vsts.check_option_str();
    buf.b_p_vts.check_option_str();
}

/// Return true if "val" is a valid 'filetype' name.
fn valid_filetype(val: &[u8]) -> bool {
    valid_name(val, b".-_")
}

/// Handle setting 'signcolumn' for value 'val'.
pub fn check_signcolumn(scl: Option<&CStr>, wp: Option<&mut WinInfoInfo>) -> bool {
    let scl_owned = if scl.is_none()
        && let Some(ref w) = wp
    {
        Some(w.w_onebuf_opt.wo_scl.to_owned())
    } else {
        None
    };
    let val_cstr = scl.or(scl_owned.as_deref()).unwrap_or(c"");

    let val_bytes = val_cstr.to_bytes();
    if val_bytes.is_empty() {
        return false;
    }

    if opt_strings_flags(val_cstr, &OPT_SCL_VALUES, None, false, None).is_none() {
        let Some(wp) = wp else {
            return true;
        };
        if val_bytes.starts_with(b"no") {
            wp.w_minscwidth.set(SCL_NO);
            wp.w_maxscwidth.set(SCL_NO);
        } else if val_bytes.starts_with(b"nu")
            && (wp.w_onebuf_opt.wo_nu.get() || wp.w_onebuf_opt.wo_rnu.get())
        {
            wp.w_minscwidth.set(SCL_NUM);
            wp.w_maxscwidth.set(SCL_NUM);
        } else if let Some(after) = val_bytes.strip_prefix(b"yes:") {
            let n = i32::from(after.first().copied().unwrap_or(0)) - (b'0' as i32);
            wp.w_minscwidth.set(n);
            wp.w_maxscwidth.set(n);
        } else if val_bytes.first() == Some(&b'y') {
            wp.w_minscwidth.set(1);
            wp.w_maxscwidth.set(1);
        } else if let Some(after) = val_bytes.strip_prefix(b"auto:") {
            wp.w_minscwidth.set(0);
            wp.w_maxscwidth
                .set(i32::from(after.first().copied().unwrap_or(0)) - (b'0' as i32));
        } else {
            wp.w_minscwidth.set(0);
            wp.w_maxscwidth.set(1);
        }
        let scwidth = if wp.w_minscwidth.get() <= 0 {
            0
        } else {
            wp.w_maxscwidth.get().min(wp.w_scwidth.get())
        };
        wp.w_scwidth.set(wp.w_minscwidth.get().max(scwidth));
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
        wp.w_minscwidth.set(min);
        wp.w_maxscwidth.set(max);
        let scwidth = if wp.w_minscwidth.get() <= 0 {
            0
        } else {
            wp.w_maxscwidth.get().min(wp.w_scwidth.get())
        };
        wp.w_scwidth.set(wp.w_minscwidth.get().max(scwidth));
    }
    true
}

/// Check validity of string with the 'statusline' format until the end of the current scope.
fn check_stl_str_adv_scope<'a>(
    s: &mut &[u8],
    errbuf: &mut Option<&'a mut [u8]>,
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
            return illegal_char(errbuf.take(), c);
        }
        if s.first() == Some(&b'{') {
            *s = &s[1..];
            let reevaluate = s.first() == Some(&b'%');

            if reevaluate {
                *s = &s[1..];
                if s.first() == Some(&b'}') {
                    return illegal_char(errbuf.take(), b'}' as i32);
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
pub fn check_stl_option<'a>(s: &[u8], mut errbuf: Option<&'a mut [u8]>) -> Option<&'a CStr> {
    let mut bytes = crate::memory::bytes_until_nul(s);
    check_stl_str_adv_scope(&mut bytes, &mut errbuf, StlScopeType::TopLevel)
}

/// Check for a "normal" directory or file name in some options.
pub fn check_illegal_path_names(val_bytes: &[u8], flags: u32) -> bool {
    let nfname_illegal: &[u8] = if g_st().secure.get() != 0 {
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
    var_cstr: Option<&CStr>,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let opt = get_option(idx);
    let val_cstr = var_cstr.unwrap_or_else(|| opt.var().as_cstr());
    let list = (opt.flags & (K_OPT_FLAG_COMMA | K_OPT_FLAG_ONE_COMMA)) != 0;
    let values = opt_values(idx);
    let mut new_flags = 0u32;
    let err = opt_strings_flags(
        val_cstr,
        values,
        opt.flags_var_mut().map(|_| &mut new_flags),
        list,
        errbuf,
    );
    if err.is_none()
        && let Some(slot) = opt.flags_var_mut()
    {
        slot.set(new_flags);
    }
    err
}

pub fn expand_set_str_generic(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let values = opt_values(args.oe_idx);
    expand_set_opt_string(args, values, matches)
}

/// An option that accepts a list of flags is changed.
pub fn did_set_str_generic<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_str_opt(args.os_idx, Some(args.var_cstr()), args.os_errbuf.take())
}

/// Validate named string values without updating derived flags.
pub fn validate_str_generic<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let opt = get_option(args.os_idx);
    let value = args.os_newval.data.as_cstr();
    opt_strings_flags(
        value,
        opt_values(args.os_idx),
        None,
        (opt.flags & K_OPT_FLAG_COMMA) != 0,
        args.os_errbuf.take(),
    )
}

/// Validate a list of character flags against "flags".
fn check_option_listflag<'a>(
    val: &[u8],
    flags: &CStr,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    for &s_byte in crate::memory::bytes_until_nul(val) {
        if !crate::strings::vim_haschr(flags, s_byte as i32) {
            return illegal_char(errbuf, s_byte as i32);
        }
    }
    None
}

/// Expand an option that accepts a list of string values.
fn expand_set_opt_string(
    args: &mut Optexpand,
    values: &[&CStr],
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let regmatch = &mut *args.oe_regmatch;
    let include_orig_val = args.oe_include_orig_val;
    let option_val = args.oe_opt_value;

    matches.clear();

    if include_orig_val && !option_val.to_bytes().is_empty() {
        matches.push(option_val.to_owned());
    }

    for &val in values {
        if val.to_bytes().is_empty()
            || (include_orig_val
                && !option_val.to_bytes().is_empty()
                && val.to_bytes() == option_val.to_bytes())
        {
            continue;
        }
        if vim_regexec(regmatch, val, 0) {
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
        let orig = SET_OPT_CALLBACK_ORIG.with(|cell| cell.borrow().clone());
        return Some(match orig {
            Some(cstr) => xp.set_item(cstr),
            None => c"",
        });
    }
    let func = OPTSTR_STATE.with(|s| s.set_opt_callback_func.get());
    func?(xp, idx - 1)
}

/// Expand an option with a callback that iterates through a list of possible names.
fn expand_set_opt_generic(
    args: &mut Optexpand,
    func: CompleteListItemGetter,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    SET_OPT_CALLBACK_ORIG.with(|cell| {
        *cell.borrow_mut() = args
            .oe_include_orig_val
            .then(|| args.oe_opt_value.to_owned());
    });
    OPTSTR_STATE.with(|s| s.set_opt_callback_func.set(func));

    expand_generic(
        c"",
        args.oe_xp,
        args.oe_regmatch,
        matches,
        Some(expand_set_opt_callback),
        false,
    );

    SET_OPT_CALLBACK_ORIG.with(|cell| {
        *cell.borrow_mut() = None;
    });
    OPTSTR_STATE.with(|s| s.set_opt_callback_func.set(None));
    true
}

/// Expand an option which is a list of flags.
fn expand_set_opt_listflag(
    args: &mut Optexpand,
    flags: &CStr,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    let flags_bytes = flags.to_bytes();
    let option_val = args.oe_opt_value;
    let option_bytes = option_val.to_bytes();
    let cmdline_bytes = args.oe_set_arg.as_bytes().to_vec();
    let append = args.oe_append;
    let include_orig_val = args.oe_include_orig_val && !option_bytes.is_empty();

    matches.clear();

    if include_orig_val {
        matches.push(option_val.to_owned());
    }

    for &flag_b in flags_bytes {
        if append && option_bytes.contains(&flag_b) {
            continue;
        }

        if !cmdline_bytes.contains(&flag_b) {
            if include_orig_val && option_bytes == [flag_b] {
                continue;
            }
            matches.push(crate::memory::bytes_to_cstring(&[flag_b]));
        }
    }

    !matches.is_empty()
}

fn completing_value_for_subopt(args: &mut Optexpand, name_suffix: &std::ffi::CStr) -> bool {
    let pat_off = args.oe_xp.xp_pattern.offset_from(&args.oe_set_arg);
    let suffix_bytes = name_suffix.to_bytes();
    let len = suffix_bytes.len();

    pat_off > len && args.oe_set_arg.wrapping_add(pat_off - 1 - len).slice(len) == suffix_bytes
}

/// The 'ambiwidth' option is changed.
pub fn did_set_ambiwidth<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_chars_options()
}

/// The 'emoji' option is changed.
pub fn did_set_emoji<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let errmsg = check_str_opt(K_OPT_AMBIWIDTH, None, args.os_errbuf.take());
    if errmsg.is_some() {
        return errmsg;
    }
    check_chars_options()
}

/// The 'background' option is changed.
pub fn did_set_background<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_oldval.data.string().first_byte() == opt_vars().p_bg.first_byte() {
        return None;
    }

    let dark = opt_vars().p_bg.first_byte() == b'd';

    init_highlight(false, false);

    if dark != (opt_vars().p_bg.first_byte() == b'd') && get_var_value(c"g:colors_name").is_some() {
        do_unlet(b"g:colors_name", true);
        opt_vars()
            .p_bg
            .set_option_cstr(if dark { c"dark" } else { c"light" });
        opt_vars().p_bg.check_option_str();
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
pub fn validate_backspace<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let value = args.os_newval.data.string();
    if ascii_isdigit(value.first_byte()) {
        return if value.first_byte() == b'2' {
            None
        } else {
            Some(E_INVARG)
        };
    }
    validate_str_generic(args)
}

pub fn validate_backupcopy<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut flags: u32 = 0;
    let errmsg = opt_strings_flags(
        args.os_newval.data.string().as_cstr(),
        &OPT_BKC_VALUES,
        Some(&mut flags),
        true,
        args.os_errbuf.take(),
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
pub fn did_set_backupcopy<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;
    let p_bkc = opt_vars().p_bkc.to_owned();
    let mut bkc = p_bkc.as_c_str();
    let is_local = (opt_flags & OPT_LOCAL) != 0;

    if is_local {
        bkc = buf.b_p_bkc.as_cstr();
    } else if (opt_flags & OPT_GLOBAL) == 0 {
        buf.b_bkc_flags.set(0);
    }

    if is_local && bkc.to_bytes().is_empty() {
        buf.b_bkc_flags.set(0);
    } else {
        let mut new_flags = 0u32;
        let errmsg = opt_strings_flags(
            bkc,
            &OPT_BKC_VALUES,
            Some(&mut new_flags),
            true,
            args.os_errbuf.take(),
        );
        if errmsg.is_some() {
            return errmsg;
        }
        if is_local {
            buf.b_bkc_flags.set(new_flags);
        } else {
            opt_vars().bkc_flags.set(new_flags);
        }
    }

    None
}

/// Validate the 'backupext' or 'patchmode' option.
pub fn validate_backupext_or_patchmode<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let ov = opt_vars();
    let bex_bytes = if args.os_idx == K_OPT_BACKUPEXT {
        newval_str.as_bytes()
    } else {
        ov.p_bex.as_bytes()
    };
    let pm_bytes = if args.os_idx == K_OPT_PATCHMODE {
        newval_str.as_bytes()
    } else {
        ov.p_pm.as_bytes()
    };
    let bex_s = bex_bytes.strip_prefix(b".").unwrap_or(bex_bytes);
    let pm_s = pm_bytes.strip_prefix(b".").unwrap_or(pm_bytes);
    if bex_s == pm_s {
        return Some(E_BACKUPEXT_AND_PATCHMODE_ARE_EQUAL);
    }

    None
}

/// The 'breakat' option is changed.
pub fn did_set_breakat<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    for b in &opt_vars().breakat_flags {
        b.set(false);
    }

    if !opt_vars().p_breakat.is_null() {
        for &p_byte in opt_vars().p_breakat.as_bytes() {
            opt_vars().breakat_flags[p_byte as usize].set(true);
        }
    }

    None
}

/// The 'breakindentopt' option is changed.
pub fn did_set_breakindentopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let is_local = args.os_varp.ptr_eq(&win.w_onebuf_opt.wo_briopt);
    briopt_check(is_local.then_some(&mut *win));

    if is_local && win.w_briopt_list.get() != 0 {
        redraw_all_later(UPD_NOT_VALID);
    }

    None
}

/// Validate the 'buftype' option.
pub fn validate_buftype<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let os_flags = args.os_flags;
    let newval_data = args.os_newval.data.string();
    let errmsg = validate_str_generic(args);
    if errmsg.is_some() {
        return errmsg;
    }
    if (os_flags & OPT_GLOBAL) == 0
        && !buf.terminal.get().is_null() != (newval_data.first_byte() == b't')
    {
        return Some(E_INVARG);
    }
    None
}

/// The 'buftype' option is changed.
pub fn did_set_buftype<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let win = args.win_mut();
    if buf.b_p_bt.first_byte() == b'p' {
        set_option_direct(
            K_OPT_COMMENTS,
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(NvimString::empty_cstr()),
            },
            OPT_LOCAL,
            SID_NONE,
        );
        let next_prompt = CursorPos {
            lnum: buf.b_ml.ml_line_count.get(),
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
    if win.w_status_height.get() != 0 || global_stl_height() != 0 {
        win.w_redr_status.set(true);
        redraw_later(Some(win), UPD_VALID);
    }
    buf.b_help.set(buf.b_p_bt.first_byte() == b'h');
    redraw_titles();
    None
}

pub fn validate_chars_option<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    set_chars_option(
        args.win_mut(),
        args.os_newval.data.string().as_cstr(),
        if args.os_idx == K_OPT_LISTCHARS {
            CharsOption::Listchars
        } else {
            CharsOption::Fillchars
        },
        false,
        args.os_errbuf.take(),
    )
}

/// The global 'listchars' or 'fillchars' option is changed.
fn did_set_global_chars_option<'a>(
    win: &mut WinInfoInfo,
    val: &CStr,
    what: CharsOption,
    opt_flags: i32,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let local_empty = if what == CharsOption::Listchars {
        win.w_onebuf_opt.wo_lcs.is_empty()
    } else {
        win.w_onebuf_opt.wo_fcs.is_empty()
    };

    let errmsg = set_chars_option(
        &mut *win,
        val,
        what,
        local_empty || (opt_flags & OPT_GLOBAL) == 0,
        errbuf,
    );
    if errmsg.is_some() {
        return errmsg;
    }

    if (opt_flags & OPT_GLOBAL) == 0 {
        if what == CharsOption::Listchars {
            win.w_onebuf_opt.wo_lcs.clear_option_str();
        } else {
            win.w_onebuf_opt.wo_fcs.clear_option_str();
        }
    }

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            let opt = if what == CharsOption::Listchars {
                wp.w_onebuf_opt.wo_lcs.to_owned()
            } else {
                wp.w_onebuf_opt.wo_fcs.to_owned()
            };
            if opt.to_bytes().is_empty() {
                set_chars_option(&mut *wp, &opt, what, true, None);
            }
        }
    }

    redraw_all_later(UPD_NOT_VALID);

    None
}

/// The 'fillchars' option or the 'listchars' option is changed.
pub fn did_set_chars_option<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let varp = args.os_varp;
    let val = args.var_cstr();
    let mut errmsg: Option<&CStr> = None;

    if varp.ptr_eq(&opt_vars().p_lcs) {
        errmsg = did_set_global_chars_option(
            win,
            val,
            CharsOption::Listchars,
            args.os_flags,
            args.os_errbuf.take(),
        );
    } else if varp.ptr_eq(&opt_vars().p_fcs) {
        errmsg = did_set_global_chars_option(
            win,
            val,
            CharsOption::Fillchars,
            args.os_flags,
            args.os_errbuf.take(),
        );
    } else if varp.ptr_eq(&win.w_onebuf_opt.wo_lcs) {
        errmsg = set_chars_option(
            win,
            val,
            CharsOption::Listchars,
            true,
            args.os_errbuf.take(),
        );
    } else if varp.ptr_eq(&win.w_onebuf_opt.wo_fcs) {
        errmsg = set_chars_option(
            win,
            val,
            CharsOption::Fillchars,
            true,
            args.os_errbuf.take(),
        );
    }

    errmsg
}

/// Expand 'fillchars' or 'listchars' option value.
pub fn expand_set_chars_option(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let is_lcs = args.oe_idx == K_OPT_LISTCHARS;
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
pub fn did_set_cinoptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    parse_cino(args.buf_mut());
    None
}

pub fn validate_colorcolumn<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_colorcolumn(args.os_newval.data.string().as_opt_cstr(), None)
}

/// The 'colorcolumn' option is changed.
pub fn did_set_colorcolumn<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let is_local = args.os_varp.ptr_eq(&win.w_onebuf_opt.wo_cc);
    check_colorcolumn(Some(args.var_cstr()), is_local.then_some(win))
}

/// Validate the 'comments' option.
pub fn validate_comments<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
                errmsg = illegal_char(args.os_errbuf.take(), i32::from(b));
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
pub fn validate_commentstring<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let bytes = newval_str.as_bytes();
    if bytes.is_empty() || bytes.windows(2).any(|w| w == b"%s") {
        None
    } else {
        Some(c"E537: 'commentstring' must be empty or contain %s")
    }
}

/// Validate the 'complete' option.
pub fn validate_complete<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
            return illegal_char(args.os_errbuf.take(), i32::from(buffer[0]));
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
                args.os_errbuf.take(),
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
pub fn did_set_complete<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !set_cpt_callbacks(args) {
        return crate::option::opt_error_int(
            args.os_errbuf.take(),
            E_ILLEGAL_CHARACTER_AFTER_CHR,
            b'F' as i32,
        );
    }
    None
}

/// Validate 'completeitemalign' and prepare its column order.
pub fn validate_completeitemalign<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let mut p = newval_str.as_bytes();
    let mut new_cia_flags: u32 = 0;
    let mut seen = [false; 3];
    let mut count = 0;
    let mut buf = [0u8; 10];
    while !p.is_empty() {
        copy_option_part(&mut p, &mut buf, c",");
        let item_bytes = crate::memory::bytes_until_nul(&buf);
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
pub fn did_set_completeitemalign<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    opt_vars().cia_flags.set(args.os_prep.as_flags());
    None
}

/// The 'completeopt' option is changed.
pub fn did_set_completeopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let p_cot = opt_vars().p_cot.to_owned();
    let mut cot = p_cot.as_c_str();
    let is_local = (args.os_flags & OPT_LOCAL) != 0;

    if is_local {
        cot = buf.b_p_cot.as_cstr();
    } else if opt_flags_is_both(args.os_flags) {
        buf.b_cot_flags.set(0);
    }

    let mut new_flags = 0u32;
    let err = opt_strings_flags(
        cot,
        &OPT_COT_VALUES,
        Some(&mut new_flags),
        true,
        args.os_errbuf.take(),
    );
    if err.is_none() {
        if is_local {
            buf.b_cot_flags.set(new_flags);
        } else {
            opt_vars().cot_flags.set(new_flags);
        }
    }
    err
}

#[inline]
fn opt_flags_is_both(opt_flags: i32) -> bool {
    (opt_flags & OPT_GLOBAL) == 0
}

#[cfg(windows)]
pub fn did_set_completeslash<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_strings_flags(
        opt_vars().p_csl.as_cstr(),
        &OPT_CSL_VALUES,
        None,
        false,
        None,
    )
    .is_some()
    {
        return opt_strings_flags(
            opt_vars().p_csl.as_cstr(),
            &OPT_CSL_VALUES,
            None,
            false,
            args.os_errbuf.take(),
        );
    }
    opt_strings_flags(
        args.buf_mut().b_p_csl.as_cstr(),
        &OPT_CSL_VALUES,
        None,
        false,
        args.os_errbuf.take(),
    )
}

/// Validate the 'concealcursor' option.
pub fn validate_concealcursor<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().as_bytes(),
        COCU_ALL,
        args.os_errbuf.take(),
    )
}

pub fn expand_set_concealcursor(
    args: &mut Optexpand,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, COCU_ALL, matches)
}

/// Validate the 'cpoptions' option.
pub fn validate_cpoptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().as_bytes(),
        CPO_VI,
        args.os_errbuf.take(),
    )
}

pub fn expand_set_cpoptions(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, CPO_VI, matches)
}

pub fn validate_cursorlineopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let value = newval_str.as_bytes();
    let mut flags: u32 = 0;
    if value.is_empty() || !fill_culopt_flags(value, &mut flags) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Flags(flags);
    None
}

/// The 'cursorlineopt' option is changed.
pub fn did_set_cursorlineopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if (args.os_flags & OPT_GLOBAL) != 0 {
        return None;
    }
    args.win_mut()
        .w_p_culopt_flags
        .set(args.os_prep.as_flags() as u8);
    None
}

/// The 'diffanchors' option is changed.
pub fn did_set_diffanchors<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !diffanchors_changed((args.os_flags & OPT_LOCAL) != 0) {
        return Some(E_INVARG);
    }
    None
}

/// The 'diffopt' option is changed.
pub fn did_set_diffopt<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !diffopt_changed() {
        Some(E_INVARG)
    } else {
        None
    }
}

pub fn expand_set_diffopt(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let pat_off = args.oe_xp.xp_pattern.offset_from(&args.oe_set_arg);

    if pat_off > 0 && args.oe_set_arg.byte_at(pat_off - 1) == b':' {
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
pub fn did_set_display<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let errmsg = did_set_str_generic(args);
    if errmsg.is_some() {
        return errmsg;
    }
    opt_vars()
        .dy_escape_width
        .set(if (opt_vars().dy_flags.get() & K_OPT_DY_FLAG_UHEX) != 0 {
            4
        } else {
            2
        });
    msg_grid_validate();
    None
}

/// Validate and canonize 'encoding', 'fileencoding' or 'makeencoding'.
pub fn validate_encoding<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_idx == K_OPT_FILEENCODING {
        if !modifiable(args.buf_mut()) && args.os_flags != OPT_GLOBAL {
            return Some(E_MODIFIABLE);
        }

        if crate::strings::vim_haschr(args.os_newval.data.string().as_cstr(), b',' as i32) {
            return Some(E_INVARG);
        }
    }

    let value_cstr = enc_canonize(args.os_newval.data.string().as_cstr());
    let is_not_utf8 = value_cstr.as_bytes() != b"utf-8";
    args.os_prep = OptSetPrep::Encoding(value_cstr);
    if args.os_idx == K_OPT_ENCODING && is_not_utf8 {
        return Some(E_UNSUPPORTEDOPTION);
    }
    None
}

/// One of the 'encoding', 'fileencoding' or 'makeencoding' options is changed.
pub fn did_set_encoding<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_idx == K_OPT_FILEENCODING {
        redraw_titles();
        ml_setflags(args.buf_mut());
    }

    if let Some(prep_cstr) = args.os_prep.take_encoding() {
        args.set_var_cstr(prep_cstr.as_c_str());
    }
    if args.os_varp.ptr_eq(&opt_vars().p_enc) {
        spell_reload();
    }
    None
}

pub fn expand_set_encoding(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_generic(args, Some(get_encoding_name), matches)
}

/// Validate the 'eventignore'/'eventignorewin' option.
pub fn validate_eventignore<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !check_ei(
        args.os_newval.data.string().as_cstr(),
        args.os_idx == K_OPT_EVENTIGNOREWIN,
    ) {
        return Some(E_INVARG);
    }
    None
}

fn get_eventignore_name(xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    let subtract = xp.xp_pattern.first_byte() == b'-';
    if !subtract && idx == 0 {
        return Some(c"all");
    }

    let name = get_event_name_no_group(
        xp,
        idx - 1 + i32::from(subtract),
        OPTSTR_STATE.with(|s| s.expand_eiw.get()),
    )?;

    format_to_buf!(
        &mut *g_st().iobuff.borrow_mut(),
        c"%s%s",
        if subtract { c"-" } else { c"" },
        name,
    );
    Some(g_st().iobuff_cstr())
}

pub fn expand_set_eventignore(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    OPTSTR_STATE.with(|s| s.expand_eiw.set(args.oe_idx == K_OPT_EVENTIGNOREWIN));
    expand_set_opt_generic(args, Some(get_eventignore_name), matches)
}

pub fn validate_fileformat<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if (args.os_flags & OPT_GLOBAL) == 0 && !modifiable(args.buf_mut()) {
        return Some(E_MODIFIABLE);
    }
    validate_str_generic(args)
}

/// The 'fileformat' option is changed.
pub fn did_set_fileformat<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let oldval = args.os_oldval.data.string();

    redraw_titles();
    ml_setflags(buf);
    if get_fileformat(buf) == EOL_MAC || oldval.first_byte() == b'm' {
        redraw_buf_later(args.os_buf.borrow(), UPD_NOT_VALID);
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
pub fn validate_filetype<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if valid_filetype(args.os_newval.data.string().as_bytes()) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'filetype' or the 'syntax' option is changed.
pub fn did_set_filetype_or_syntax<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    args.os_value_changed = args.os_oldval.data.string().cstr_bytes() != args.var_cstr().to_bytes();
    args.os_value_checked = true;
    None
}

/// The 'foldexpr' option is changed.
pub fn did_set_foldexpr<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if foldmethod_is_expr(win) {
        fold_update_all(win);
    }
    None
}

/// The 'foldignore' option is changed.
pub fn did_set_foldignore<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if foldmethod_is_indent(win) {
        fold_update_all(win);
    }
    None
}

/// Validate the 'foldmarker' option.
pub fn validate_foldmarker<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let value = args.os_newval.data.as_cstr();
    let Some(p) = vim_strchr(value, b',' as i32) else {
        return Some(E_COMMA_REQUIRED);
    };
    if ptr::eq(value.as_ptr(), p.as_ptr()) || p.to_bytes().len() <= 1 {
        Some(E_INVARG)
    } else {
        None
    }
}

/// The 'foldmarker' option is changed.
pub fn did_set_foldmarker<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if foldmethod_is_marker(win) {
        fold_update_all(win);
    }
    None
}

/// The 'foldmethod' option is changed.
pub fn did_set_foldmethod<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    fold_update_all(win);
    if foldmethod_is_diff(win) {
        new_fold_level();
    }
    None
}

/// Validate the 'formatoptions' option.
pub fn validate_formatoptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().cstr_bytes(),
        FO_ALL,
        args.os_errbuf.take(),
    )
}

pub fn expand_set_formatoptions(
    args: &mut Optexpand,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    expand_set_opt_listflag(args, FO_ALL, matches)
}

/// The 'guicursor' option is changed.
pub fn did_set_guicursor<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if let Some(errmsg) = parse_shape_opt(SHAPE_CURSOR) {
        return Some(errmsg);
    }
    if g_st().visual_global.borrow().active {
        redraw_winline(
            crate::globals::curwin_mut(),
            curwin_mut().w_cursor.get().lnum,
        );
    }
    None
}

/// The 'helpfile' option is changed.
pub fn did_set_helpfile<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if g_st().didset_vim.get() {
        vim_unsetenv_ext(c"VIM");
    }
    if g_st().didset_vimruntime.get() {
        vim_unsetenv_ext(c"VIMRUNTIME");
    }
    None
}

/// Validate the 'helplang' option.
pub fn validate_helplang<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
pub fn validate_highlight<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_newval.data.string().cstr_bytes() != HIGHLIGHT_INIT.to_bytes() {
        return Some(E_UNSUPPORTEDOPTION);
    }
    None
}

/// The 'iconstring' option is changed.
pub fn did_set_iconstring<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_titleiconstring(args, STL_IN_ICON)
}

/// The 'inccommand' option is changed.
pub fn did_set_inccommand<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if g_st().cmdpreview.get() {
        return Some(E_INVARG);
    }
    None
}

/// Validate the 'isident', 'iskeyword' or 'isfname' option.
pub fn validate_isopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !check_isopt(args.os_newval.data.string().as_bytes()) {
        Some(E_INVARG)
    } else {
        None
    }
}

/// The 'iskeyword' option is changed.
pub fn did_set_iskeyword<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !args.os_varp.ptr_eq(&opt_vars().p_isk) && !buf_init_isk_chartab(args.buf_mut()) {
        return Some(E_INVARG);
    }
    None
}

/// The 'isident', 'iskeyword', 'isprint' or 'isfname' option is changed.
pub fn did_set_isopt<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let res = if args.os_varp.ptr_eq(&opt_vars().p_isf) {
        init_isf_chartab()
    } else {
        init_isi_chartab()
    };
    if !res { Some(E_INVARG) } else { None }
}

/// The 'keymap' option has changed.
pub fn did_set_keymap<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;

    let secure_save = g_st().secure.get();
    g_st().secure.set(0);

    let errmsg = keymap_init();

    g_st().secure.set(secure_save);

    args.os_value_checked = true;

    if errmsg.is_none() {
        if !buf.b_p_keymap.is_empty() {
            buf.b_p_iminsert.set(B_IMODE_LMAP);
            if buf.b_p_imsearch.get() != B_IMODE_USE_INSERT {
                buf.b_p_imsearch.set(B_IMODE_LMAP);
            }
        } else {
            if buf.b_p_iminsert.get() == B_IMODE_LMAP {
                buf.b_p_iminsert.set(B_IMODE_NONE);
            }
            if buf.b_p_imsearch.get() == B_IMODE_LMAP {
                buf.b_p_imsearch.set(B_IMODE_USE_INSERT);
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
pub fn did_set_keymodel<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    g_st().km_stopsel.set(crate::strings::vim_haschr(
        opt_vars().p_km.as_cstr(),
        b'o' as i32,
    ));
    g_st().km_startsel.set(crate::strings::vim_haschr(
        opt_vars().p_km.as_cstr(),
        b'a' as i32,
    ));
    None
}

/// Validate the 'lispoptions' option.
pub fn validate_lispoptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let bytes = newval_str.as_bytes();
    if bytes.is_empty() || bytes == b"expr:0" || bytes == b"expr:1" {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Validate the 'matchpairs' option.
pub fn validate_matchpairs<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
pub fn validate_mkspellmem<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if spell_check_msm(args.os_newval.data.string().as_bytes(), false) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Validate the 'mouse' option.
pub fn validate_mouse<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().as_bytes(),
        MOUSE_ALL,
        args.os_errbuf.take(),
    )
}

pub fn expand_set_mouse(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, MOUSE_ALL, matches)
}

/// Validate the 'mousescroll' option.
pub fn validate_mousescroll<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let v: OptKeyDict_mousescroll =
        opt_keyset(args.os_newval.data.string().cstr_bytes(), K_OPT_MOUSESCROLL);
    let has_hor = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_HOR)) != 0;
    let has_ver = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_VER)) != 0;
    if has_hor || has_ver {
        None
    } else {
        Some(E_INVARG)
    }
}

/// Handle setting 'mousescroll'.
pub fn did_set_mousescroll<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    let v: OptKeyDict_mousescroll =
        opt_keyset(opt_vars().p_mousescroll.as_bytes(), K_OPT_MOUSESCROLL);
    let has_hor = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_HOR)) != 0;
    let has_ver = (v.is_set_mousescroll & (1 << KEYSET_OPTIDX_MOUSESCROLL_VER)) != 0;
    opt_vars().p_mousescroll_hor.set(if has_hor {
        i64::from(v.hor as i32)
    } else {
        MOUSESCROLL_HOR_DFLT
    });
    opt_vars().p_mousescroll_vert.set(if has_ver {
        i64::from(v.ver as i32)
    } else {
        MOUSESCROLL_VERT_DFLT
    });
    None
}

/// The 'rulerformat' option is changed.
pub fn did_set_rulerformat<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_statustabline_rulerformat(args, true, false)
}

/// The 'selection' option is changed.
pub fn did_set_selection<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if g_st().visual_global.borrow().active {
        redraw_curbuf_later(UPD_INVERTED);
    }
    None
}

/// Validate the 'sessionoptions' option.
pub fn validate_sessionoptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut flags: u32 = 0;
    let errmsg = opt_strings_flags(
        args.os_newval.data.string().as_cstr(),
        &OPT_SSOP_VALUES,
        Some(&mut flags),
        true,
        args.os_errbuf.take(),
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
pub fn validate_shada<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let errbuf = args.os_errbuf.take();
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
                        &transchar_byte(i32::from(b0)),
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
pub fn validate_shellpipe_redir<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
pub fn validate_shortmess<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().cstr_bytes(),
        SHM_ALL,
        args.os_errbuf.take(),
    )
}

/// Validate the 'showbreak' option.
pub fn validate_showbreak<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut s = args.os_newval.data.cstr_bytes();
    while !s.is_empty() {
        if ptr2cells(s) != 1 {
            return Some(E_SHOWBREAK_CONTAINS_UNPRINTABLE_OR_WIDE_CHARACTER);
        }
        let step = (utfc_ptr2len(s) as usize).max(1).min(s.len());
        s = &s[step..];
    }
    None
}

/// The 'showcmdloc' option is changed.
pub fn did_set_showcmdloc<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    comp_col();
    None
}

pub fn validate_signcolumn<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if check_signcolumn(args.os_newval.data.string().as_opt_cstr(), None) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'signcolumn' option is changed.
pub fn did_set_signcolumn<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let oldval = args.os_oldval.data.string();
    let is_local = args.os_varp.ptr_eq(&win.w_onebuf_opt.wo_scl);
    if !check_signcolumn(Some(args.var_cstr()), is_local.then_some(&mut *win)) {
        return Some(E_INVARG);
    }
    if oldval.cstr_bytes().starts_with(b"nu") || win.w_minscwidth.get() == SCL_NUM {
        win.w_nrwidth_line_count.set(0);
    }
    None
}

/// Validate 'spellcapcheck' and prepare its regexp program.
pub fn validate_spellcapcheck<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut prog: Option<Box<Regprog>> = None;
    let mut err = Error::new();
    let save_emsg_silent = g_st().emsg_silent.get();
    let save_suppress_errthrow = g_st().suppress_errthrow.get();
    g_st().emsg_silent.set(0);
    g_st().suppress_errthrow.set(false);

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let errmsg = compile_cap_prog(args.os_newval.data.string().as_cstr(), &mut prog);
    try_leave(&tstate, &mut err);

    g_st().emsg_silent.set(save_emsg_silent);
    g_st().suppress_errthrow.set(save_suppress_errthrow);
    args.os_prep = OptSetPrep::SpellcapProg(prog);
    if err.err_type != ErrorType::ErrorTypeNone {
        let msg_cstr = err.msg_cstr().unwrap_or(c"");
        let msg = if msg_cstr.to_bytes().starts_with(b"Vim:") {
            crate::memory::CStrSubslice::advance(msg_cstr, 4)
        } else {
            msg_cstr
        };
        let err_out = opt_error(args.os_errbuf.take(), c"%s", msg);
        api_clear_error(&mut err);
        return err_out;
    }
    errmsg
}

/// The 'spellcapcheck' option is changed.
pub fn did_set_spellcapcheck<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_varp.ptr_eq(&opt_vars().p_spc) {
        return None;
    }
    let ws = args.win_mut().w_s_ptr();
    *ws.b_cap_prog.borrow_mut() = args.os_prep.take_spellcap_prog();
    None
}

/// Validate the 'spellfile' option.
pub fn validate_spellfile<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if valid_spellfile(args.os_newval.data.string().as_cstr()) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'spellfile' option is changed.
pub fn did_set_spellfile<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_spell_option()
}

/// Validate the 'spelllang' option.
pub fn validate_spelllang<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if valid_spelllang(args.os_newval.data.string().as_cstr()) {
        None
    } else {
        Some(E_INVARG)
    }
}

/// The 'spelllang' option is changed.
pub fn did_set_spelllang<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_spell_option()
}

/// The 'spelloptions' option is changed.
pub fn did_set_spelloptions<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let opt_flags = args.os_flags;
    let val = args.os_newval.data.as_cstr();

    if (opt_flags & OPT_LOCAL) == 0 {
        let mut new_flags = 0u32;
        if opt_strings_flags(val, &OPT_SPO_VALUES, Some(&mut new_flags), true, None).is_some() {
            return opt_strings_flags(val, &OPT_SPO_VALUES, None, true, args.os_errbuf.take());
        }
        opt_vars().spo_flags.set(new_flags);
    }
    if (opt_flags & OPT_GLOBAL) == 0 {
        let ws = args.win_mut().w_s_ptr();
        let errmsg = opt_strings_flags(
            val,
            &OPT_SPO_VALUES,
            Some(&mut ws.b_p_spo_flags.get()),
            true,
            args.os_errbuf.take(),
        );
        if errmsg.is_some() {
            return errmsg;
        }
    }
    None
}

/// Validate the 'spellsuggest' option.
pub fn validate_spellsuggest<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut sps: [i32; 2] = [0, 0];
    let [s0, s1] = &mut sps;
    if !spell_check_sps(args.os_newval.data.string().as_cstr(), s0, s1) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Spellsuggest(sps);
    None
}

/// The 'spellsuggest' option is changed.
pub fn did_set_spellsuggest<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if let OptSetPrep::Spellsuggest(sps) = args.os_prep {
        spell_apply_sps(sps[0], sps[1]);
    }
    None
}

pub fn did_set_splitkeep<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&tp) {
            wp.w_prev_height.set(wp.w_height.get());
            wp.w_prev_winrow.set(wp.w_winrow.get());
        }
    }
    None
}

pub fn validate_statustabline_rulerformat<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let newval_str = args.os_newval.data.string();
    let value = newval_str.as_bytes();
    if value.starts_with(b"%!") {
        None
    } else {
        check_stl_option(value, args.os_errbuf.take())
    }
}

/// The 'statuscolumn' option is changed.
pub fn did_set_statuscolumn<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_statustabline_rulerformat(args, false, true)
}

/// The 'statusline' option is changed.
pub fn did_set_statusline<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'statusline', 'winbar', 'tabline', 'rulerformat' or 'statuscolumn' option is changed.
fn did_set_statustabline_rulerformat<'a>(
    args: &mut Optset<'a>,
    rulerformat: bool,
    statuscolumn: bool,
) -> Option<&'a CStr> {
    if rulerformat {
        g_st().ru_wid.set(0);
    } else if statuscolumn {
        args.win_mut().w_nrwidth_line_count.set(0);
    }
    let is_stl = args.os_idx == K_OPT_STATUSLINE;

    if (is_stl || rulerformat)
        && ((args.os_flags & OPT_GLOBAL) != 0 || (args.os_flags & OPT_LOCAL) == 0)
        && args.var_cstr().to_bytes().is_empty()
    {
        let def = get_option_default(args.os_idx, args.os_flags);
        args.set_var_cstr(def.data.string().as_cstr());
        optval_free_read(args.os_idx, def);
    }
    let s_bytes = args.var_cstr().to_bytes();

    if is_stl
        && let Some(win) = args.win_opt()
        && win.w_floating.get()
    {
        let cfg = win.w_config.borrow().clone();
        win_config_float(win, cfg);
    }

    if rulerformat && !ui_has(K_UIMESSAGES) && s_bytes.first() == Some(&b'%') {
        let mut rem = &s_bytes[1..];
        if let Some(after_dash) = rem.strip_prefix(b"-") {
            rem = after_dash;
        }
        let wid = getdigits_int(&mut rem, true, 0);
        if wid != 0 && rem.first() == Some(&b'(') {
            g_st().ru_wid.set(wid);
        }
    }
    if rulerformat {
        comp_col();
    }
    None
}

/// The 'tabline' option is changed.
pub fn did_set_tabline<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// The 'tagcase' option is changed.
pub fn did_set_tagcase<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let opt_flags = args.os_flags;
    let p_tc = opt_vars().p_tc.to_owned();
    let is_local = (opt_flags & OPT_LOCAL) != 0;

    let p: &CStr = if is_local {
        buf.b_p_tc.as_cstr()
    } else {
        p_tc.as_c_str()
    };

    if is_local && p.to_bytes().is_empty() {
        buf.b_tc_flags.set(0);
    } else {
        let mut new_flags = 0u32;
        let errmsg = opt_strings_flags(
            p,
            &OPT_TC_VALUES,
            Some(&mut new_flags),
            false,
            args.os_errbuf.take(),
        );
        if errmsg.is_some() {
            return errmsg;
        }
        if is_local {
            buf.b_tc_flags.set(new_flags);
        } else {
            opt_vars().tc_flags.set(new_flags);
        }
    }
    None
}

/// The 'titlestring' or the 'iconstring' option is changed.
fn did_set_titleiconstring<'a>(args: &mut Optset<'a>, flagval: i32) -> Option<&'a CStr> {
    let val = args.var_cstr();

    if crate::strings::vim_haschr(val, b'%' as i32)
        && check_stl_option(val.to_bytes(), None).is_none()
    {
        g_st().stl_syntax.set(g_st().stl_syntax.get() | (flagval));
    } else {
        g_st().stl_syntax.set(g_st().stl_syntax.get() & (!flagval));
    }
    did_set_title();

    None
}

/// The 'titlestring' option is changed.
pub fn did_set_titlestring<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_titleiconstring(args, STL_IN_TITLE)
}

/// Validate 'vartabstop' or 'varsofttabstop' and prepare its tab widths.
pub fn validate_vartabstop<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let val_bytes = args.os_newval.data.cstr_bytes();
    let mut prev_comma = false;
    for (idx, &b) in val_bytes.iter().enumerate() {
        if ascii_isdigit(b) {
            prev_comma = false;
            continue;
        }
        if b == b',' && idx > 0 && !prev_comma {
            prev_comma = true;
            continue;
        }
        return Some(E_INVARG);
    }

    let mut array: Vec<Colnr> = Vec::new();
    let errmsg = tabstop_set(val_bytes, &mut array, args.os_errbuf.take());
    args.os_prep = OptSetPrep::Tabstop(array);
    errmsg
}

/// The 'vartabstop' or 'varsofttabstop' option is changed.
pub fn did_set_vartabstop<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_varp.ptr_eq(&opt_vars().p_vts) || args.os_varp.ptr_eq(&opt_vars().p_vsts) {
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
pub fn did_set_verbosefile<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    verbose_stop();
    if !opt_vars().p_vfile.is_empty() && !verbose_open() {
        return Some(E_INVARG);
    }
    None
}

/// The 'virtualedit' option is changed.
pub fn did_set_virtualedit<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();

    let p_ve = opt_vars().p_ve.to_owned();
    let mut ve = p_ve.as_c_str();
    let is_local = (args.os_flags & OPT_LOCAL) != 0;

    if is_local {
        ve = win.w_onebuf_opt.wo_ve.as_cstr();
    }

    if is_local && ve.to_bytes().is_empty() {
        win.w_onebuf_opt.wo_ve_flags.set(0);
    } else {
        let mut new_flags = 0u32;
        let errmsg = opt_strings_flags(
            ve,
            &OPT_VE_VALUES,
            Some(&mut new_flags),
            true,
            args.os_errbuf.take(),
        );
        if errmsg.is_some() {
            return errmsg;
        }
        if is_local {
            win.w_onebuf_opt.wo_ve_flags.set(new_flags);
        } else {
            opt_vars().ve_flags.set(new_flags);
        }
        if ve.to_bytes() != args.os_oldval.data.string().as_bytes() {
            validate_virtcol(win);
            let vc = win.w_virtcol.get();
            coladvance(win, vc);
        }
    }
    None
}

/// Validate the 'whichwrap' option.
pub fn validate_whichwrap<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    check_option_listflag(
        args.os_newval.data.string().as_bytes(),
        WW_ALL_COMMA,
        args.os_errbuf.take(),
    )
}

pub fn expand_set_whichwrap(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_listflag(args, WW_ALL, matches)
}

pub fn validate_wildmode<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut flags = [0u8; 4];
    if !check_opt_wim(args.os_newval.data.string().as_bytes(), &mut flags) {
        return Some(E_INVARG);
    }
    args.os_prep = OptSetPrep::Wildmode(flags);
    None
}

/// The 'wildmode' option is changed.
pub fn did_set_wildmode<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if let OptSetPrep::Wildmode(flags) = args.os_prep {
        for (dst, src) in g_st().wim_flags.iter().zip(flags) {
            dst.set(src);
        }
    }
    None
}

/// The 'winbar' option is changed.
pub fn did_set_winbar<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_statustabline_rulerformat(args, false, false)
}

/// Validate the 'winborder' or 'pumborder' option.
pub fn validate_border<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
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
pub fn did_set_winhighlight<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let is_local = args.os_varp.ptr_eq(&win.w_onebuf_opt.wo_winhl);
    if !parse_winhl_opt(Some(args.var_cstr()), is_local.then_some(win)) {
        return Some(E_INVARG);
    }
    None
}

pub fn expand_set_winhighlight(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    expand_set_opt_generic(args, Some(get_highlight_name), matches)
}

/// Format an error naming an item (at most 63 bytes) and listing its valid values.
fn opt_values_err<'a>(
    errbuf: Option<&'a mut [u8]>,
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
    let mut n = format_to_buf!(errbuf, gettext(fmt), &itembuf);
    for (j, &v) in values.iter().enumerate() {
        if n <= 0 || (n as usize) >= errbuf.len() {
            break;
        }
        n += format_to_buf!(
            &mut errbuf[n as usize..],
            c"%s%s",
            if j == 0 { c" " } else { c", " },
            v,
        );
    }
    Some(crate::memory::bytes_as_cstr(errbuf))
}

/// Handle an option that can be a range of string values.
fn opt_strings_flags<'a>(
    val: &CStr,
    values: &[&CStr],
    flagp: Option<&mut u32>,
    list: bool,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let val_bytes = val.to_bytes();
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
    if let Some(flagp) = flagp {
        *flagp = new_flags;
    }

    None
}

/// Format a schema-validation error naming the offending key.
fn opt_schema_err<'a>(
    errbuf: Option<&'a mut [u8]>,
    fmt: &std::ffi::CStr,
    key: &[u8],
) -> Option<&'a CStr> {
    let errbuf = errbuf?;
    let mut keybuf = [0u8; 64];
    let copy_len = key.len().min(keybuf.len() - 1);
    keybuf[..copy_len].copy_from_slice(&key[..copy_len]);
    opt_error(Some(errbuf), fmt, crate::memory::bytes_as_cstr(&keybuf))
}

/// Validates a dict option against its schema (options.lua).
pub fn opt_strings_check<'a>(
    val: &[u8],
    schema: &[OptSchemaItem],
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let mut p = crate::memory::bytes_until_nul(val);
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
                let digits = v.map(|v_bytes| {
                    if it.kind == OptSchemaKind::OptSchemaSNum {
                        v_bytes.strip_prefix(b"-").unwrap_or(v_bytes)
                    } else {
                        v_bytes
                    }
                });
                let Some(v_bytes) = v.filter(|_| {
                    digits.is_some_and(|d| !d.is_empty() && d.iter().all(u8::is_ascii_digit))
                }) else {
                    return opt_schema_err(errbuf, c"E474: '%s' requires a number", key);
                };
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

/// Reify a dict-option value to a keyset (`OptKeyDict_…`).
pub fn opt_keyset<T: OptKeyDict>(value: &[u8], _opt_idx: OptIndex) -> T {
    let mut out = T::default();
    if !value.is_empty() {
        let mut p = value;
        while let Some((key_bytes, v)) = option_next_keyval(&mut p) {
            out.set_field(key_bytes, v);
        }
    }
    out
}

/// Return OK if "p" is a valid fileformat name, FAIL otherwise.
pub fn check_ff_value(p: &CStr) -> bool {
    opt_strings_flags(p, &OPT_FF_VALUES, None, false, None).is_none()
}

/// Calls mb_cptr2char_adv(p) and returns the character.
fn get_encoded_char_adv(p: &mut &CStr) -> Schar {
    let s = *p;
    let s_bytes = s.to_bytes();
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
        let mut cur = s;
        while bytes > 0 {
            cur = crate::memory::CStrSubslice::advance(cur, 2);
            let n = hexhex2nr(cur.to_bytes());
            if n < 0 {
                *p = cur;
                return 0;
            }
            num = num * 256 + i64::from(n);
            bytes -= 1;
        }
        cur = crate::memory::CStrSubslice::advance(cur, 2);
        *p = cur;
        return if char2cells(num as i32) > 1 {
            0
        } else {
            schar_from_char(num as i32)
        };
    }

    let ci = utf_ptr2_str_char_info(s);
    let cli = utf_cluster_info(ci);
    let adv = (cli.next.ptr.as_ptr() as usize).saturating_sub(s.as_ptr() as usize);
    *p = crate::memory::CStrSubslice::advance(s, adv);
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
    value: &CStr,
    what: CharsOption,
    apply: bool,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let mut value = value;
    let mut last_multispace = ptr::null();
    let mut last_lmultispace = ptr::null();
    let mut multispace_len: usize = 0;
    let mut lead_multispace_len: usize = 0;
    let mut fcs_chars = FcsChars::default();
    let mut lcs_chars = LcsChars::new();

    let p_lcs = opt_vars().p_lcs.to_owned();
    let p_fcs = opt_vars().p_fcs.to_owned();
    let tab: &[CharsTabEntry] = if what == CharsOption::Listchars {
        if apply && wp.w_onebuf_opt.wo_lcs.is_empty() {
            value = p_lcs.as_c_str();
        }
        &LCS_TAB
    } else {
        if apply && wp.w_onebuf_opt.wo_fcs.is_empty() {
            value = p_fcs.as_c_str();
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
                    let s_cstr = match entry.def {
                        Some(d) if ptr2cells(d) == 1 => Some(d),
                        _ => entry.fallback,
                    };
                    *slot = schar_from_str(s_cstr);
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
        while !p.is_empty() {
            let p_bytes = p.to_bytes();
            let mut i: usize = 0;
            while i < entries {
                let name_cstr = tab[i].name;
                let name_bytes = name_cstr.to_bytes();
                let name_len = name_bytes.len();
                if !(p_bytes.starts_with(name_bytes) && p_bytes.get(name_len) == Some(&b':')) {
                    i += 1;
                    continue;
                }

                let mut s_cstr = crate::memory::CStrSubslice::advance(p, name_len + 1);

                if tab[i].target == CharsTarget::LcsMultispace {
                    if round == 0 {
                        last_multispace = p.as_ptr();
                        multispace_len = 0;
                        while !s_cstr.to_bytes().is_empty()
                            && s_cstr.to_bytes().first() != Some(&b',')
                        {
                            let c1 = get_encoded_char_adv(&mut s_cstr);
                            if c1 == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                    name_cstr,
                                );
                            }
                            multispace_len += 1;
                        }
                        if multispace_len == 0 {
                            return opt_error(
                                errbuf,
                                E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                                name_cstr,
                            );
                        }
                    } else {
                        let mut multispace_pos: usize = 0;
                        while !s_cstr.to_bytes().is_empty()
                            && s_cstr.to_bytes().first() != Some(&b',')
                        {
                            let c1 = get_encoded_char_adv(&mut s_cstr);
                            if p.as_ptr() == last_multispace {
                                lcs_chars.multispace[multispace_pos] = c1;
                                multispace_pos += 1;
                            }
                        }
                    }
                    p = s_cstr;
                    break;
                }

                if tab[i].target == CharsTarget::LcsLeadmultispace {
                    if round == 0 {
                        last_lmultispace = p.as_ptr();
                        lead_multispace_len = 0;
                        while !s_cstr.to_bytes().is_empty()
                            && s_cstr.to_bytes().first() != Some(&b',')
                        {
                            let c1 = get_encoded_char_adv(&mut s_cstr);
                            if c1 == 0 {
                                return opt_error(
                                    errbuf,
                                    E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                    name_cstr,
                                );
                            }
                            lead_multispace_len += 1;
                        }
                        if lead_multispace_len == 0 {
                            return opt_error(
                                errbuf,
                                E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                                name_cstr,
                            );
                        }
                    } else {
                        let mut multispace_pos: usize = 0;
                        while !s_cstr.to_bytes().is_empty()
                            && s_cstr.to_bytes().first() != Some(&b',')
                        {
                            let c1 = get_encoded_char_adv(&mut s_cstr);
                            if p.as_ptr() == last_lmultispace {
                                lcs_chars.leadmultispace[multispace_pos] = c1;
                                multispace_pos += 1;
                            }
                        }
                    }
                    p = s_cstr;
                    break;
                }

                if s_cstr.to_bytes().is_empty() {
                    return opt_error(
                        errbuf,
                        E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                        name_cstr,
                    );
                }
                let c1 = get_encoded_char_adv(&mut s_cstr);
                if c1 == 0 {
                    return opt_error(errbuf, E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR, name_cstr);
                }
                let mut c2: Schar = 0;
                let mut c3: Schar = 0;
                if matches!(tab[i].target, CharsTarget::LcsTab | CharsTarget::LcsLeadtab) {
                    if s_cstr.to_bytes().is_empty() {
                        return opt_error(
                            errbuf,
                            E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                            name_cstr,
                        );
                    }
                    c2 = get_encoded_char_adv(&mut s_cstr);
                    if c2 == 0 {
                        return opt_error(errbuf, E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR, name_cstr);
                    }
                    if !(s_cstr.to_bytes().first() == Some(&b',') || s_cstr.to_bytes().is_empty()) {
                        c3 = get_encoded_char_adv(&mut s_cstr);
                        if c3 == 0 {
                            return opt_error(
                                errbuf,
                                E_WRONG_CHARACTER_WIDTH_FOR_FIELD_STR,
                                name_cstr,
                            );
                        }
                    }
                    if tab[i].target == CharsTarget::LcsTab {
                        has_tab = true;
                    } else {
                        has_leadtab = true;
                    }
                }

                if s_cstr.to_bytes().first() == Some(&b',') || s_cstr.to_bytes().is_empty() {
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
                    p = s_cstr;
                    break;
                } else {
                    return opt_error(
                        errbuf,
                        E_WRONG_NUMBER_OF_CHARACTERS_FOR_FIELD_STR,
                        name_cstr,
                    );
                }
            }

            if i == entries {
                return Some(E_INVARG);
            }

            if p.to_bytes().first() == Some(&b',') {
                p = crate::memory::CStrSubslice::advance(p, 1);
            }
        }

        if what == CharsOption::Listchars && has_leadtab && !has_tab {
            return Some(E_LEADTAB_REQUIRES_TAB);
        }
    }

    if apply {
        if what == CharsOption::Listchars {
            *wp.w_p_lcs_chars.borrow_mut() = lcs_chars;
        } else {
            wp.w_p_fcs_chars.set(fcs_chars);
        }
    }

    None
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
        opt_vars().p_lcs.as_cstr(),
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
        opt_vars().p_fcs.as_cstr(),
        CharsOption::Fillchars,
        false,
        None,
    )
    .is_some()
    {
        return Some(E_CONFLICTS_WITH_VALUE_OF_FILLCHARS);
    }
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            let wo_lcs = wp.w_onebuf_opt.wo_lcs.to_owned();
            if set_chars_option(wp, &wo_lcs, CharsOption::Listchars, true, None).is_some() {
                return Some(E_CONFLICTS_WITH_VALUE_OF_LISTCHARS);
            }
            let wo_fcs = wp.w_onebuf_opt.wo_fcs.to_owned();
            if set_chars_option(wp, &wo_fcs, CharsOption::Fillchars, true, None).is_some() {
                return Some(E_CONFLICTS_WITH_VALUE_OF_FILLCHARS);
            }
        }
    }
    None
}

pub fn validate_previewpopup<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let mut fconfig = WIN_CONFIG_INIT;
    let cstr = args.os_newval.data.as_cstr();
    if !win_previewpopup_config(cstr, &mut fconfig) {
        return Some(E_INVARG);
    }
    None
}

pub fn expand_set_popupoption(args: &mut Optexpand, matches: &mut Vec<std::ffi::CString>) -> bool {
    let pat_off = args.oe_xp.xp_pattern.offset_from(&args.oe_set_arg);

    if pat_off > 0 && args.oe_set_arg.byte_at(pat_off - 1) == b':' {
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
