//! Low-level code for UI and syntax highlighting (`src/nvim/highlight.c`).

use crate::api::private::helpers::{
    add_c, api_dict_to_keydict, api_set_error, arena_array, arena_dict, boolean_obj, cstr_as_obj,
    cstr_as_string, dict_obj, integer_obj, put_c, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectData, ObjectType, api_err_exp,
    api_err_invalid,
};
use crate::api::ui::{HLATTRS_DICT_SIZE, remote_ui_hl_attr_define, remote_ui_hl_group_set};
use crate::decoration::{HLGroupID, NS};
use crate::decoration_provider::get_decor_provider;
use crate::drawscreen::screen_invalidate_highlights;
use crate::globals::*;
use crate::highlight_group::{
    highlight_attr_set_all, highlight_changed, name_to_color, name_to_ctermcolor, set_hl_group,
    syn_check_group, syn_id2name, syn_ns_id2attr,
};
use crate::lua::converter::Boolean;
use crate::lua::executor::{kRetObject, nlua_call_ref};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::message::emsg;
use crate::msgpack_rpc::packer::Integer;
use crate::option::check_blending;
use crate::option_vars::*;
use crate::popupmenu::pum_drawn;
use crate::strings::gettext;
use crate::ui::ui_call_hl_attr_define;
use crate::window::win_T;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type RgbValue = i32;
pub type OptionalKeys = u64;
use crate::msgpack_rpc::channel::RemoteUI;
pub type HlKind = c_int;

const MAX_TYPENR: u32 = 65535;

pub(crate) const HLF_NONE: usize = 0;
pub(crate) const HLF_PNI: c_int = 41;
pub(crate) const HLF_PST: c_int = 50;
pub(crate) const HLF_INACTIVE: usize = 60;
const HLF_NFLOAT: usize = 62;
const HLF_BORDER: usize = 64;
pub(crate) const HLF_COUNT: usize = 76;

const kHlUnknown: HlKind = 0;
const kHlUI: HlKind = 1;
const kHlSyntax: HlKind = 2;
const kHlTerminal: HlKind = 3;
const kHlCombine: HlKind = 4;
const kHlBlend: HlKind = 5;
const kHlBlendThrough: HlKind = 6;
const kHlInvalid: HlKind = 7;

pub(crate) const HL_ALTFONT: i32 = 0x0100;
pub(crate) const HL_BG_INDEXED: i32 = 0x0800;
pub(crate) const HL_FG_INDEXED: i32 = 0x1000;
pub(crate) const HL_DEFAULT: i32 = 0x2000;
pub(crate) const HL_GLOBAL: i32 = 0x4000;

pub(crate) const KEYSET_OPTIDX_highlight__bg: u64 = 1;
pub(crate) const KEYSET_OPTIDX_highlight__fg: u64 = 2;
pub(crate) const KEYSET_OPTIDX_highlight__sp: u64 = 3;
const KEYSET_OPTIDX_highlight__dim: u64 = 4;
pub(crate) const KEYSET_OPTIDX_highlight__url: u64 = 5;
const KEYSET_OPTIDX_highlight__bold: u64 = 6;
pub(crate) const KEYSET_OPTIDX_highlight__font: u64 = 7;
const KEYSET_OPTIDX_highlight__link: u64 = 8;
const KEYSET_OPTIDX_highlight__blend: u64 = 9;
const KEYSET_OPTIDX_highlight__blink: u64 = 11;
const KEYSET_OPTIDX_highlight__cterm: u64 = 12;
const KEYSET_OPTIDX_highlight__italic: u64 = 13;
pub(crate) const KEYSET_OPTIDX_highlight__update: u64 = 14;
const KEYSET_OPTIDX_highlight__reverse: u64 = 15;
const KEYSET_OPTIDX_highlight__default: u64 = 16;
const KEYSET_OPTIDX_highlight__altfont: u64 = 17;
const KEYSET_OPTIDX_highlight__conceal: u64 = 18;
const KEYSET_OPTIDX_highlight__special: u64 = 19;
const KEYSET_OPTIDX_highlight__ctermfg: u64 = 20;
const KEYSET_OPTIDX_highlight__ctermbg: u64 = 21;
const KEYSET_OPTIDX_highlight__fallback: u64 = 22;
const KEYSET_OPTIDX_highlight__overline: u64 = 23;
const KEYSET_OPTIDX_highlight__standout: u64 = 24;
const KEYSET_OPTIDX_highlight__nocombine: u64 = 25;
const KEYSET_OPTIDX_highlight__undercurl: u64 = 26;
const KEYSET_OPTIDX_highlight__underline: u64 = 27;
const KEYSET_OPTIDX_highlight__background: u64 = 28;
const KEYSET_OPTIDX_highlight__bg_indexed: u64 = 29;
const KEYSET_OPTIDX_highlight__foreground: u64 = 30;
const KEYSET_OPTIDX_highlight__fg_indexed: u64 = 31;
const KEYSET_OPTIDX_highlight__link_global: u64 = 32;
const KEYSET_OPTIDX_highlight__underdashed: u64 = 33;
const KEYSET_OPTIDX_highlight__underdotted: u64 = 34;
const KEYSET_OPTIDX_highlight__underdouble: u64 = 35;
const KEYSET_OPTIDX_highlight__strikethrough: u64 = 36;

pub const HLATTRS_INIT: HlAttrs = HlAttrs {
    rgb_ae_attr: 0,
    cterm_ae_attr: 0,
    rgb_fg_color: -1,
    rgb_bg_color: -1,
    rgb_sp_color: -1,
    cterm_fg_color: 0,
    cterm_bg_color: 0,
    hl_blend: -1,
    url: -1,
    font: -1,
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_highlight {
    pub is_set__highlight_: OptionalKeys,
    pub altfont: Boolean,
    pub blink: Boolean,
    pub bold: Boolean,
    pub conceal: Boolean,
    pub dim: Boolean,
    pub italic: Boolean,
    pub nocombine: Boolean,
    pub overline: Boolean,
    pub reverse: Boolean,
    pub standout: Boolean,
    pub strikethrough: Boolean,
    pub undercurl: Boolean,
    pub underdashed: Boolean,
    pub underdotted: Boolean,
    pub underdouble: Boolean,
    pub underline: Boolean,
    pub default_: Boolean,
    pub cterm: Dict,
    pub foreground: Object,
    pub fg: Object,
    pub background: Object,
    pub bg: Object,
    pub ctermfg: Object,
    pub ctermbg: Object,
    pub special: Object,
    pub sp: Object,
    pub link: HLGroupID,
    pub link_global: HLGroupID,
    pub fallback: Boolean,
    pub blend: Integer,
    pub fg_indexed: Boolean,
    pub bg_indexed: Boolean,
    pub force: Boolean,
    pub update: Boolean,
    pub url: NvimString,
    pub font: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_highlight_cterm {
    pub bold: Boolean,
    pub standout: Boolean,
    pub strikethrough: Boolean,
    pub underline: Boolean,
    pub undercurl: Boolean,
    pub underdouble: Boolean,
    pub underdotted: Boolean,
    pub underdashed: Boolean,
    pub italic: Boolean,
    pub reverse: Boolean,
    pub altfont: Boolean,
    pub dim: Boolean,
    pub blink: Boolean,
    pub conceal: Boolean,
    pub overline: Boolean,
    pub nocombine: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HlAttrs {
    pub rgb_ae_attr: i32,
    pub cterm_ae_attr: i32,
    pub rgb_fg_color: i32,
    pub rgb_bg_color: i32,
    pub rgb_sp_color: i32,
    pub cterm_fg_color: i16,
    pub cterm_bg_color: i16,
    pub hl_blend: i32,
    pub url: i32,
    pub font: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HlEntry {
    pub attr: HlAttrs,
    pub kind: c_int,
    pub id1: c_int,
    pub id2: c_int,
    pub winid: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColorKey {
    pub ns_id: c_int,
    pub syn_id: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColorItem {
    pub attr_id: c_int,
    pub link_id: c_int,
    pub version: c_int,
    pub is_default: bool,
    pub link_global: bool,
}

type NSHlAttr = [c_int; HLF_COUNT];

use crate::api::private::helpers::has_key;

static mut hlstate_active: bool = false;
static mut attr_entries: Vec<HlEntry> = Vec::new();
static mut attr_entry_ids: std::collections::BTreeMap<HlEntry, u32> =
    std::collections::BTreeMap::new();
static mut combine_attr_entries: std::collections::BTreeMap<u64, c_int> =
    std::collections::BTreeMap::new();
static mut blend_attr_entries: std::collections::BTreeMap<u64, c_int> =
    std::collections::BTreeMap::new();
static mut blendthrough_attr_entries: std::collections::BTreeMap<u64, c_int> =
    std::collections::BTreeMap::new();
static mut urls: Vec<std::ffi::CString> = Vec::new();
static mut fonts: Vec<std::ffi::CString> = Vec::new();
static mut ns_hls: std::collections::BTreeMap<ColorKey, ColorItem> =
    std::collections::BTreeMap::new();
static mut ns_hl_attr: std::collections::BTreeMap<c_int, *mut NSHlAttr> =
    std::collections::BTreeMap::new();

static mut get_attr_entry_recursive: bool = false;
static mut ns_get_hl_recursive: c_int = 0;

#[inline]
unsafe fn is_none_str(s: *const c_char) -> bool {
    !s.is_null() && unsafe { CStr::from_ptr(s).to_bytes().eq_ignore_ascii_case(b"NONE") }
}

use crate::api::private::dispatch::{
    KeyDict_highlight_cterm_get_field, KeyDict_highlight_get_field,
};
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, HL_UNDERLINE_MASK};
use crate::eval::funcs::{
    HL_BLINK, HL_BOLD, HL_CONCEALED, HL_DIM, HL_INVERSE, HL_ITALIC, HL_NOCOMBINE, HL_STANDOUT,
    HL_UNDERCURL, HL_UNDERDASHED, HL_UNDERDOTTED, HL_UNDERDOUBLE, HL_UNDERLINE,
};
use crate::lua::converter::LUA_NOREF;

#[inline]
pub unsafe fn win_hl_attr(wp: *mut win_T, hlf: c_int) -> c_int {
    unsafe {
        let tbl = if !(*wp).w_ns_hl_attr.is_null() && ns_hl_fast < 0 {
            (*wp).w_ns_hl_attr
        } else {
            hl_attr_active
        };
        *tbl.add(hlf as usize)
    }
}

#[inline]
pub unsafe fn hl_attr(n: c_int) -> c_int {
    unsafe { *hl_attr_active.add(n as usize) }
}

#[inline]
const fn hl_attr_key(a: c_int, b: c_int) -> u64 {
    ((a as u32 as u64) << 32) | (b as u32 as u64)
}

#[inline]
unsafe fn map_get_u64_int(
    map: *const std::collections::BTreeMap<u64, c_int>,
    key: u64,
) -> c_int {
    unsafe { (*map).get(&key).copied().unwrap_or(0) }
}

#[inline]
unsafe fn map_put_u64_int(
    map: *mut std::collections::BTreeMap<u64, c_int>,
    key: u64,
    val: c_int,
) {
    unsafe {
        (*map).insert(key, val);
    }
}

#[inline]
unsafe fn map_get_color(
    map: *const std::collections::BTreeMap<ColorKey, ColorItem>,
    key: ColorKey,
) -> ColorItem {
    unsafe {
        (*map).get(&key).copied().unwrap_or(ColorItem {
            attr_id: -1,
            link_id: -1,
            version: -1,
            is_default: false,
            link_global: false,
        })
    }
}

#[inline]
unsafe fn map_put_color(
    map: *mut std::collections::BTreeMap<ColorKey, ColorItem>,
    key: ColorKey,
    val: ColorItem,
) {
    unsafe {
        (*map).insert(key, val);
    }
}

pub unsafe fn highlight_init() {
    unsafe {
        let entries_ptr = &raw mut attr_entries;
        let ids_ptr = &raw mut attr_entry_ids;
        if (*entries_ptr).is_empty() {
            let entry = HlEntry {
                attr: HLATTRS_INIT,
                kind: kHlInvalid,
                id1: 0,
                id2: 0,
                winid: 0,
            };
            (*entries_ptr).push(entry);
            (*ids_ptr).insert(entry, 0);
        }
    }
}

pub unsafe fn highlight_use_hlstate() -> bool {
    unsafe {
        if hlstate_active {
            return false;
        }
        hlstate_active = true;
        clear_hl_tables(true);
        true
    }
}

unsafe fn get_attr_entry(mut entry: HlEntry) -> c_int {
    unsafe {
        let mut retried = false;
        if !hlstate_active {
            entry.kind = kHlUnknown;
            entry.id1 = 0;
            entry.id2 = 0;
        }

        loop {
            let entries_ptr = &raw mut attr_entries;
            let ids_ptr = &raw mut attr_entry_ids;
            if let Some(&k) = (*ids_ptr).get(&entry) {
                return k as c_int;
            }

            if (*entries_ptr).len() as u32 >= MAX_TYPENR {
                if get_attr_entry_recursive || retried {
                    emsg(gettext(
                        c"E424: Too many different highlighting attributes in use".as_ptr(),
                    ));
                    return 0;
                }
                get_attr_entry_recursive = true;
                clear_hl_tables(true);
                get_attr_entry_recursive = false;

                if entry.kind == kHlCombine {
                    return 0;
                }
                retried = true;
                continue;
            }

            let k = (*entries_ptr).len() as u32;
            (*entries_ptr).push(entry);
            (*ids_ptr).insert(entry, k);

            let id = k as c_int;
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let inspect = hl_inspect(id, &raw mut arena);
            ui_call_hl_attr_define(Integer::from(id), entry.attr, entry.attr, inspect);
            arena_mem_free(arena_finish(&raw mut arena));
            return id;
        }
    }
}

pub unsafe fn ui_send_all_hls(ui: *mut RemoteUI) {
    unsafe {
        let entries_ptr = &raw const attr_entries;
        let len = (*entries_ptr).len();
        for i in 1..len {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let inspect = hl_inspect(i as c_int, &raw mut arena);
            let attr = (*entries_ptr).as_slice()[i].attr;
            remote_ui_hl_attr_define(ui, i as Integer, attr, attr, inspect);
            arena_mem_free(arena_finish(&raw mut arena));
        }
        for hlf in 0..HLF_COUNT {
            remote_ui_hl_group_set(
                ui,
                cstr_as_string(hlf_names[hlf]),
                Integer::from(highlight_attr[hlf]),
            );
        }
    }
}

pub unsafe fn hl_get_syn_attr(ns_id: c_int, idx: c_int, at_en: HlAttrs) -> c_int {
    unsafe {
        if at_en.cterm_fg_color != 0
            || at_en.cterm_bg_color != 0
            || at_en.rgb_fg_color != -1
            || at_en.rgb_bg_color != -1
            || at_en.rgb_sp_color != -1
            || at_en.cterm_ae_attr != 0
            || at_en.rgb_ae_attr != 0
            || at_en.font >= 0
            || ns_id != 0
        {
            return get_attr_entry(HlEntry {
                attr: at_en,
                kind: kHlSyntax,
                id1: idx,
                id2: ns_id,
                winid: 0,
            });
        }
        0
    }
}

pub unsafe fn ns_hl_def(
    ns_id: NS,
    hl_id: c_int,
    attrs: HlAttrs,
    link_id: c_int,
    dict: *mut KeyDict_highlight,
) {
    unsafe {
        if ns_id == 0 {
            assert!(!dict.is_null());
            set_hl_group(hl_id, attrs, dict, link_id);
            return;
        }
        let key = ColorKey {
            ns_id,
            syn_id: hl_id,
        };
        let ns_hls_ptr = &raw const ns_hls;
        if (attrs.rgb_ae_attr & HL_DEFAULT) != 0 && (*ns_hls_ptr).contains_key(&key) {
            return;
        }
        let p = get_decor_provider(ns_id, true);
        let attr_id = if link_id > 0 {
            -1
        } else {
            hl_get_syn_attr(ns_id, hl_id, attrs)
        };
        let it = ColorItem {
            attr_id,
            link_id,
            version: (*p).hl_valid,
            is_default: (attrs.rgb_ae_attr & HL_DEFAULT) != 0,
            link_global: (attrs.rgb_ae_attr & HL_GLOBAL) != 0,
        };
        map_put_color(&raw mut ns_hls, key, it);
        (*p).hl_cached = false;
    }
}

pub unsafe fn ns_get_hl(ns_hl: *mut NS, hl_id: c_int, link: bool, nodefault: bool) -> c_int {
    unsafe {
        if *ns_hl == 0 {
            return -1;
        }

        if *ns_hl < 0 {
            if ns_hl_active <= 0 {
                return -1;
            }
            *ns_hl = ns_hl_active;
        }

        let ns_id = *ns_hl;
        let p = get_decor_provider(ns_id, true);
        let key = ColorKey {
            ns_id,
            syn_id: hl_id,
        };
        let mut it = map_get_color(&raw mut ns_hls, key);
        let mut valid_item = it.version >= (*p).hl_valid;

        if !valid_item && (*p).hl_def != LUA_NOREF && ns_get_hl_recursive == 0 {
            let mut args_storage = [Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData { integer: 0 },
            }; 3];
            let mut args = Array {
                size: 0,
                capacity: 3,
                items: args_storage.as_mut_ptr(),
            };
            add_c(&mut args, integer_obj(Integer::from(ns_id)));
            add_c(&mut args, cstr_as_obj(syn_id2name(hl_id)));
            add_c(&mut args, boolean_obj(link));

            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            ns_get_hl_recursive += 1;
            let ret = nlua_call_ref(
                (*p).hl_def,
                c"hl_def".as_ptr(),
                args,
                kRetObject,
                ptr::null_mut(),
                &raw mut err,
            );
            ns_get_hl_recursive -= 1;

            let mut fallback = true;
            let mut tmp = false;
            let mut attrs = HLATTRS_INIT;
            if ret.obj_type == ObjectType::kObjectTypeDict {
                fallback = false;
                let mut dict: KeyDict_highlight = std::mem::zeroed();
                if api_dict_to_keydict(
                    (&raw mut dict).cast(),
                    Some(KeyDict_highlight_get_field),
                    ret.data.dict,
                    &raw mut err,
                ) {
                    attrs = dict2hlattrs(
                        &raw mut dict,
                        true,
                        &raw mut it.link_id,
                        ptr::null_mut(),
                        &raw mut err,
                    );
                    fallback =
                        if has_key(dict.is_set__highlight_, KEYSET_OPTIDX_highlight__fallback) {
                            dict.fallback
                        } else {
                            true
                        };
                    tmp = dict.fallback;
                    if it.link_id >= 0 {
                        fallback = true;
                    }
                }
            }

            it.attr_id = if fallback {
                -1
            } else {
                hl_get_syn_attr(ns_id, hl_id, attrs)
            };
            it.version = (*p).hl_valid - c_int::from(tmp);
            it.is_default = (attrs.rgb_ae_attr & HL_DEFAULT) != 0;
            it.link_global = (attrs.rgb_ae_attr & HL_GLOBAL) != 0;
            map_put_color(&raw mut ns_hls, key, it);
            valid_item = true;
        }

        if (it.is_default && nodefault) || !valid_item {
            return -1;
        }

        if link {
            if it.attr_id >= 0 {
                return 0;
            }
            if it.link_global {
                *ns_hl = 0;
            }
            it.link_id
        } else {
            it.attr_id
        }
    }
}

pub unsafe fn hl_check_ns() -> bool {
    unsafe {
        let ns = if ns_hl_fast > 0 {
            ns_hl_fast
        } else if ns_hl_win >= 0 {
            ns_hl_win
        } else {
            ns_hl_global
        };
        if ns_hl_active == ns {
            return false;
        }

        ns_hl_active = ns;
        hl_attr_active = (&raw mut highlight_attr).cast::<c_int>();
        if ns > 0 {
            update_ns_hl(ns);
            let ns_hl_attr_ptr = &raw const ns_hl_attr;
            if let Some(&hl_def) = (*ns_hl_attr_ptr).get(&ns)
                && !hl_def.is_null()
            {
                hl_attr_active = (*hl_def).as_mut_ptr();
            }
        }
        true
    }
}

pub unsafe fn win_check_ns_hl(wp: *mut win_T) -> bool {
    unsafe {
        ns_hl_win = if !wp.is_null() { (*wp).w_ns_hl } else { -1 };
        hl_check_ns()
    }
}

pub unsafe fn hl_ns_get_attrs(
    ns_id: c_int,
    hl_id: c_int,
    optional: *mut bool,
    attrs: *mut HlAttrs,
) -> bool {
    unsafe {
        let mut opt = if !optional.is_null() { *optional } else { true };
        let syn_attr = syn_ns_id2attr(ns_id, hl_id, &raw mut opt);
        if !optional.is_null() {
            *optional = opt;
        }
        if syn_attr <= 0 {
            return false;
        }
        *attrs = syn_attr2entry(syn_attr);
        true
    }
}

pub unsafe fn hl_get_ui_attr(
    ns_id: c_int,
    idx: c_int,
    final_id: c_int,
    mut optional: bool,
) -> c_int {
    unsafe {
        let mut attrs = HLATTRS_INIT;
        let mut available = false;

        if final_id > 0 {
            available = hl_ns_get_attrs(ns_id, final_id, &raw mut optional, &raw mut attrs);
        }

        if (HLF_PNI..=HLF_PST).contains(&idx) {
            if attrs.hl_blend == -1 && p_pb > 0 {
                attrs.hl_blend = p_pb as i32;
            }
            if pum_drawn() {
                must_redraw_pum = true;
            }
        }

        if optional && !available {
            return 0;
        }
        get_attr_entry(HlEntry {
            attr: attrs,
            kind: kHlUI,
            id1: idx,
            id2: final_id,
            winid: 0,
        })
    }
}

pub unsafe fn hl_apply_winblend(winbl: c_int, mut attr: c_int) -> c_int {
    unsafe {
        let entries_ptr = &raw const attr_entries;
        let mut entry = (*entries_ptr).as_slice()[attr as usize];
        if entry.attr.hl_blend == -1 && winbl > 0 {
            entry.attr.hl_blend = winbl;
            attr = get_attr_entry(entry);
        }
        attr
    }
}

pub unsafe fn update_window_hl(wp: *mut win_T, invalid: bool) {
    unsafe {
        let ns_id = (*wp).w_ns_hl;

        update_ns_hl(ns_id);
        if ns_id != (*wp).w_ns_hl_active {
            (*wp).w_ns_hl_active = ns_id;

            let ns_hl_attr_ptr = &raw const ns_hl_attr;
            let hl_def_ptr = (*ns_hl_attr_ptr)
                .get(&ns_id)
                .copied()
                .unwrap_or(ptr::null_mut());
            if !hl_def_ptr.is_null() {
                (*wp).w_ns_hl_attr = (*hl_def_ptr).as_mut_ptr();
            } else {
                (*wp).w_ns_hl_attr = ptr::null_mut();
            }
        }

        let hl_def = if !(*wp).w_ns_hl_attr.is_null() {
            (*wp).w_ns_hl_attr
        } else {
            hl_attr_active
        };

        if (*wp).w_hl_needs_update == 0 && !invalid {
            return;
        }
        (*wp).w_hl_needs_update = 0;

        let float_win = (*wp).w_floating && !(*wp).w_config.external;
        if float_win && *hl_def.add(HLF_NFLOAT) != 0 && ns_id > 0 {
            (*wp).w_hl_attr_normal = *hl_def.add(HLF_NFLOAT);
        } else if *hl_def.add(HLF_NONE) > 0 {
            (*wp).w_hl_attr_normal = *hl_def.add(HLF_NONE);
        } else if float_win {
            (*wp).w_hl_attr_normal = if *hl_attr_active.add(HLF_NFLOAT) > 0 {
                *hl_attr_active.add(HLF_NFLOAT)
            } else {
                highlight_attr[HLF_NFLOAT]
            };
        } else {
            (*wp).w_hl_attr_normal = 0;
        }

        if (*wp).w_floating {
            (*wp).w_hl_attr_normal =
                hl_apply_winblend((*wp).w_onebuf_opt.wo_winbl as c_int, (*wp).w_hl_attr_normal);
        }

        (*wp).w_config.shadow = false;
        if (*wp).w_floating && (*wp).w_config.border {
            for i in 0..8 {
                let mut attr = *hl_def.add(HLF_BORDER);
                if (*wp).w_config.border_hl_ids[i] != 0 {
                    attr = hl_get_ui_attr(
                        ns_id,
                        HLF_BORDER as c_int,
                        (*wp).w_config.border_hl_ids[i],
                        false,
                    );
                }
                attr = hl_apply_winblend((*wp).w_onebuf_opt.wo_winbl as c_int, attr);
                if syn_attr2entry(attr).hl_blend > 0 {
                    (*wp).w_config.shadow = true;
                }
                (*wp).w_config.border_attr[i] = attr;
            }
        }

        check_blending(wp);

        if *hl_def.add(HLF_INACTIVE) == 0 {
            (*wp).w_hl_attr_normalnc =
                hl_combine_attr(*hl_attr_active.add(HLF_INACTIVE), (*wp).w_hl_attr_normal);
        } else {
            (*wp).w_hl_attr_normalnc = *hl_def.add(HLF_INACTIVE);
        }

        if (*wp).w_floating {
            (*wp).w_hl_attr_normalnc = hl_apply_winblend(
                (*wp).w_onebuf_opt.wo_winbl as c_int,
                (*wp).w_hl_attr_normalnc,
            );
        }
    }
}

pub unsafe fn update_ns_hl(ns_id: c_int) {
    unsafe {
        if ns_id <= 0 {
            return;
        }
        let mut p = get_decor_provider(ns_id, true);
        if (*p).hl_cached {
            return;
        }

        let ns_hl_attr_ptr = &raw mut ns_hl_attr;
        let alloc = (*ns_hl_attr_ptr)
            .entry(ns_id)
            .or_insert_with(|| Box::into_raw(Box::new([0; HLF_COUNT])));
        let hl_attrs = (**alloc).as_mut_ptr();

        for hlf in 1..HLF_COUNT {
            let name = *(&raw const hlf_names).cast::<*const c_char>().add(hlf);
            let id = syn_check_group(name, CStr::from_ptr(name).to_bytes().len());
            let optional = hlf == HLF_INACTIVE || hlf == HLF_NFLOAT;
            *hl_attrs.add(hlf) = hl_get_ui_attr(ns_id, hlf as c_int, id, optional);
        }

        let normality = syn_check_group(c"Normal".as_ptr(), 6);
        *hl_attrs.add(HLF_NONE) = hl_get_ui_attr(ns_id, -1, normality, true);

        p = get_decor_provider(ns_id, true);
        (*p).hl_cached = true;
    }
}

pub unsafe fn win_bg_attr(wp: *mut win_T) -> c_int {
    unsafe {
        if ns_hl_fast < 0 {
            let local = if ptr::eq(wp, curwin) {
                (*wp).w_hl_attr_normal
            } else {
                (*wp).w_hl_attr_normalnc
            };
            if local != 0 {
                return local;
            }
        }

        if ptr::eq(wp, curwin) || *hl_attr_active.add(HLF_INACTIVE) == 0 {
            *hl_attr_active.add(HLF_NONE)
        } else {
            *hl_attr_active.add(HLF_INACTIVE)
        }
    }
}

pub unsafe fn hl_get_underline() -> c_int {
    unsafe {
        let mut attrs = HLATTRS_INIT;
        attrs.cterm_ae_attr = HL_UNDERLINE;
        attrs.rgb_ae_attr = HL_UNDERLINE;
        get_attr_entry(HlEntry {
            attr: attrs,
            kind: kHlUI,
            id1: 0,
            id2: 0,
            winid: 0,
        })
    }
}

pub unsafe fn hl_add_url(attr: c_int, url: *const c_char) -> c_int {
    unsafe {
        let mut attrs = HLATTRS_INIT;
        let url_cstr = CStr::from_ptr(url);
        let urls_ptr = &raw mut urls;
        let k = if let Some(pos) = (*urls_ptr).iter().position(|u| u.as_c_str() == url_cstr) {
            pos
        } else {
            let idx = (*urls_ptr).len();
            (*urls_ptr).push(url_cstr.to_owned());
            idx
        };
        attrs.url = k as i32;

        let new = get_attr_entry(HlEntry {
            attr: attrs,
            kind: kHlUI,
            id1: 0,
            id2: 0,
            winid: 0,
        });
        hl_combine_attr(attr, new)
    }
}

pub unsafe fn hl_get_url(index: u32) -> *const c_char {
    unsafe {
        let urls_ptr = &raw const urls;
        (*urls_ptr).as_slice()[index as usize].as_ptr()
    }
}

pub unsafe fn hl_add_font_idx(font_name: *const c_char) -> i32 {
    unsafe {
        if font_name.is_null() || *font_name == 0 {
            return -1;
        }
        let font_cstr = CStr::from_ptr(font_name);
        let fonts_ptr = &raw mut fonts;
        if let Some(pos) = (*fonts_ptr).iter().position(|f| f.as_c_str() == font_cstr) {
            return pos as i32;
        }
        let idx = (*fonts_ptr).len() as i32;
        (*fonts_ptr).push(font_cstr.to_owned());
        idx
    }
}

pub unsafe fn hl_get_font(index: i32) -> *const c_char {
    unsafe {
        let fonts_ptr = &raw const fonts;
        if index < 0 || (index as usize) >= (*fonts_ptr).len() {
            return ptr::null();
        }
        (*fonts_ptr).as_slice()[index as usize].as_ptr()
    }
}

pub unsafe fn hl_get_term_attr(aep: *mut HlAttrs) -> c_int {
    unsafe {
        get_attr_entry(HlEntry {
            attr: *aep,
            kind: kHlTerminal,
            id1: 0,
            id2: 0,
            winid: 0,
        })
    }
}

pub unsafe fn clear_hl_tables(reinit: bool) {
    unsafe {
        let entries_ptr = &raw mut attr_entries;
        let ids_ptr = &raw mut attr_entry_ids;
        let combine_ptr = &raw mut combine_attr_entries;
        let blend_ptr = &raw mut blend_attr_entries;
        let blendthrough_ptr = &raw mut blendthrough_attr_entries;
        let urls_ptr = &raw mut urls;
        if reinit {
            (*entries_ptr).clear();
            (*ids_ptr).clear();
            highlight_init();
            (*combine_ptr).clear();
            (*blend_ptr).clear();
            (*blendthrough_ptr).clear();
            (*urls_ptr).clear();
            ptr::write_bytes(
                (&raw mut highlight_attr_last).cast::<u8>(),
                0xFF,
                size_of::<[c_int; HLF_COUNT]>(),
            );
            highlight_attr_set_all();
            highlight_changed();
            screen_invalidate_highlights();
        } else {
            *entries_ptr = Vec::new();
            *ids_ptr = std::collections::BTreeMap::new();
            *combine_ptr = std::collections::BTreeMap::new();
            *blend_ptr = std::collections::BTreeMap::new();
            *blendthrough_ptr = std::collections::BTreeMap::new();
            let ns_hls_ptr = &raw mut ns_hls;
            *ns_hls_ptr = std::collections::BTreeMap::new();
            let ns_hl_attr_ptr = &raw mut ns_hl_attr;
            for (_, ptr) in std::mem::take(&mut *ns_hl_attr_ptr) {
                drop(Box::from_raw(ptr));
            }
            *urls_ptr = Vec::new();
            let fonts_ptr = &raw mut fonts;
            *fonts_ptr = Vec::new();
        }
    }
}

pub unsafe fn hl_invalidate_blends() {
    unsafe {
        let blend_ptr = &raw mut blend_attr_entries;
        (*blend_ptr).clear();
        let blendthrough_ptr = &raw mut blendthrough_attr_entries;
        (*blendthrough_ptr).clear();
        highlight_changed();
        update_window_hl(curwin, true);
    }
}

#[inline]
const fn hl_combine_ae(char_ae: i32, prim_ae: i32) -> i32 {
    let char_ul = char_ae & HL_UNDERLINE_MASK;
    let prim_ul = prim_ae & HL_UNDERLINE_MASK;
    let new_ul = if prim_ul != 0 { prim_ul } else { char_ul };
    (char_ae & !HL_UNDERLINE_MASK) | (prim_ae & !HL_UNDERLINE_MASK) | new_ul
}

pub unsafe fn hl_combine_attr(char_attr: c_int, prim_attr: c_int) -> c_int {
    unsafe {
        if char_attr == 0 {
            return prim_attr;
        } else if prim_attr == 0 {
            return char_attr;
        }

        let combine_tag = hl_attr_key(char_attr, prim_attr);
        let mut id = map_get_u64_int(&raw mut combine_attr_entries, combine_tag);
        if id > 0 {
            return id;
        }

        let char_aep = syn_attr2entry(char_attr);
        let prim_aep = syn_attr2entry(prim_attr);

        let mut new_en = char_aep;

        if (prim_aep.cterm_ae_attr & HL_NOCOMBINE) != 0 {
            new_en.cterm_ae_attr = prim_aep.cterm_ae_attr;
        } else {
            new_en.cterm_ae_attr = hl_combine_ae(new_en.cterm_ae_attr, prim_aep.cterm_ae_attr);
        }
        if (prim_aep.rgb_ae_attr & HL_NOCOMBINE) != 0 {
            new_en.rgb_ae_attr = prim_aep.rgb_ae_attr;
        } else {
            new_en.rgb_ae_attr = hl_combine_ae(new_en.rgb_ae_attr, prim_aep.rgb_ae_attr);
        }

        if prim_aep.cterm_fg_color > 0 {
            new_en.cterm_fg_color = prim_aep.cterm_fg_color;
            new_en.rgb_ae_attr &= !HL_FG_INDEXED | (prim_aep.rgb_ae_attr & HL_FG_INDEXED);
        }

        if prim_aep.cterm_bg_color > 0 {
            new_en.cterm_bg_color = prim_aep.cterm_bg_color;
            new_en.rgb_ae_attr &= !HL_BG_INDEXED | (prim_aep.rgb_ae_attr & HL_BG_INDEXED);
        }

        if prim_aep.rgb_fg_color >= 0 {
            new_en.rgb_fg_color = prim_aep.rgb_fg_color;
            new_en.rgb_ae_attr &= !HL_FG_INDEXED | (prim_aep.rgb_ae_attr & HL_FG_INDEXED);
        }

        if prim_aep.rgb_bg_color >= 0 {
            new_en.rgb_bg_color = prim_aep.rgb_bg_color;
            new_en.rgb_ae_attr &= !HL_BG_INDEXED | (prim_aep.rgb_ae_attr & HL_BG_INDEXED);
        }

        if prim_aep.rgb_sp_color >= 0 {
            new_en.rgb_sp_color = prim_aep.rgb_sp_color;
        }

        if prim_aep.hl_blend >= 0 {
            new_en.hl_blend = prim_aep.hl_blend;
        }

        if new_en.url == -1 && prim_aep.url >= 0 {
            new_en.url = prim_aep.url;
        }

        if prim_aep.font >= 0 {
            new_en.font = prim_aep.font;
        }

        id = get_attr_entry(HlEntry {
            attr: new_en,
            kind: kHlCombine,
            id1: char_attr,
            id2: prim_attr,
            winid: 0,
        });
        if id > 0 {
            map_put_u64_int(&raw mut combine_attr_entries, combine_tag, id);
        }

        id
    }
}

unsafe fn get_colors_force(mut attrs: HlAttrs) -> HlAttrs {
    unsafe {
        if attrs.rgb_bg_color == -1 {
            attrs.rgb_bg_color = normal_bg;
        }
        if attrs.rgb_fg_color == -1 {
            attrs.rgb_fg_color = normal_fg;
        }
        if attrs.rgb_sp_color == -1 {
            attrs.rgb_sp_color = normal_sp;
        }
        let dark = *p_bg == b'd' as c_char;
        if attrs.rgb_fg_color == -1 {
            attrs.rgb_fg_color = if dark { 0xFFFFFF } else { 0x000000 };
        }
        if attrs.rgb_bg_color == -1 {
            attrs.rgb_bg_color = if dark { 0x000000 } else { 0xFFFFFF };
        }
        if attrs.rgb_sp_color == -1 {
            attrs.rgb_sp_color = 0xFF0000;
        }

        if (attrs.rgb_ae_attr & HL_INVERSE) != 0 {
            std::mem::swap(&mut attrs.rgb_bg_color, &mut attrs.rgb_fg_color);
            attrs.rgb_ae_attr &= !HL_INVERSE;
        }

        attrs
    }
}

pub unsafe fn hl_blend_attrs(back_attr: c_int, front_attr: c_int, through: *mut bool) -> c_int {
    unsafe {
        if front_attr < 0 || back_attr < 0 {
            return front_attr;
        }

        let fattrs_raw = syn_attr2entry(front_attr);
        let fattrs = get_colors_force(fattrs_raw);
        let ratio = fattrs.hl_blend;
        if ratio <= 0 {
            *through = false;
            return front_attr;
        }

        let combine_tag = hl_attr_key(back_attr, front_attr);
        let map = if *through {
            &raw mut blendthrough_attr_entries
        } else {
            &raw mut blend_attr_entries
        };
        let mut id = map_get_u64_int(map, combine_tag);
        if id > 0 {
            return id;
        }

        let battrs_raw = syn_attr2entry(back_attr);
        let battrs = get_colors_force(battrs_raw);
        let mut cattrs: HlAttrs;

        if *through {
            cattrs = battrs;
            cattrs.rgb_fg_color = rgb_blend(ratio, battrs.rgb_fg_color, fattrs.rgb_bg_color);
            if (cattrs.rgb_ae_attr & HL_UNDERLINE_MASK) != 0 && battrs_raw.rgb_sp_color != -1 {
                cattrs.rgb_sp_color = rgb_blend(ratio, battrs.rgb_sp_color, fattrs.rgb_bg_color);
            } else {
                cattrs.rgb_sp_color = -1;
            }

            cattrs.cterm_bg_color = fattrs.cterm_bg_color;
            cattrs.cterm_fg_color =
                cterm_blend(ratio, battrs.cterm_fg_color, fattrs.cterm_bg_color) as i16;
            cattrs.rgb_ae_attr &= !(HL_FG_INDEXED | HL_BG_INDEXED);
        } else {
            cattrs = fattrs;
            cattrs.rgb_fg_color = rgb_blend(ratio / 2, battrs.rgb_fg_color, fattrs.rgb_fg_color);
            if (cattrs.rgb_ae_attr & HL_UNDERLINE_MASK) != 0 {
                cattrs.rgb_sp_color =
                    rgb_blend(ratio / 2, battrs.rgb_bg_color, fattrs.rgb_sp_color);
            } else {
                cattrs.rgb_sp_color = -1;
            }

            cattrs.rgb_ae_attr &= !(HL_FG_INDEXED | HL_BG_INDEXED);
        }

        if ratio == 100 && battrs_raw.rgb_bg_color == -1 {
            cattrs.rgb_bg_color = -1;
        } else {
            cattrs.rgb_bg_color = if battrs_raw.rgb_bg_color == -1 && fattrs_raw.rgb_bg_color == -1
            {
                -1
            } else {
                rgb_blend(ratio, battrs.rgb_bg_color, fattrs.rgb_bg_color)
            };
        }
        cattrs.hl_blend = -1;
        let kind = if *through { kHlBlendThrough } else { kHlBlend };
        id = get_attr_entry(HlEntry {
            attr: cattrs,
            kind,
            id1: back_attr,
            id2: front_attr,
            winid: 0,
        });
        if id > 0 {
            map_put_u64_int(map, combine_tag, id);
        }
        id
    }
}

const fn rgb_blend(ratio: c_int, rgb1: c_int, rgb2: c_int) -> c_int {
    let a = ratio;
    let b = 100 - ratio;
    let r1 = (rgb1 & 0xFF0000) >> 16;
    let g1 = (rgb1 & 0x00FF00) >> 8;
    let b1 = rgb1 & 0x0000FF;
    let r2 = (rgb2 & 0xFF0000) >> 16;
    let g2 = (rgb2 & 0x00FF00) >> 8;
    let b2 = rgb2 & 0x0000FF;
    let mr = (a * r1 + b * r2) / 100;
    let mg = (a * g1 + b * g2) / 100;
    let mb = (a * b1 + b * b2) / 100;
    (mr << 16) + (mg << 8) + mb
}

const fn cterm_blend(ratio: c_int, c1: i16, c2: i16) -> c_int {
    let rgb1 = hl_cterm2rgb_color(c1 as c_int);
    let rgb2 = hl_cterm2rgb_color(c2 as c_int);
    let rgb_blended = rgb_blend(ratio, rgb1, rgb2);
    hl_rgb2cterm_color(rgb_blended)
}

const fn hl_rgb2cterm_color(rgb: c_int) -> c_int {
    let r = (rgb & 0xFF0000) >> 16;
    let g = (rgb & 0x00FF00) >> 8;
    let b = rgb & 0x0000FF;

    (r * 6 / 256) * 36 + (g * 6 / 256) * 6 + (b * 6 / 256)
}

const fn hl_cterm2rgb_color(nr: c_int) -> c_int {
    const CUBE_VALUE: [c_int; 6] = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
    const GREY_RAMP: [c_int; 24] = [
        0x08, 0x12, 0x1C, 0x26, 0x30, 0x3A, 0x44, 0x4E, 0x58, 0x62, 0x6C, 0x76, 0x80, 0x8A, 0x94,
        0x9E, 0xA8, 0xB2, 0xBC, 0xC6, 0xD0, 0xDA, 0xE4, 0xEE,
    ];
    const ANSI_TABLE: [[u8; 4]; 16] = [
        [0, 0, 0, 1],
        [224, 0, 0, 2],
        [0, 224, 0, 3],
        [224, 224, 0, 4],
        [0, 0, 224, 5],
        [224, 0, 224, 6],
        [0, 224, 224, 7],
        [224, 224, 224, 8],
        [128, 128, 128, 9],
        [255, 64, 64, 10],
        [64, 255, 64, 11],
        [255, 255, 64, 12],
        [64, 64, 255, 13],
        [255, 64, 255, 14],
        [64, 255, 255, 15],
        [255, 255, 255, 16],
    ];

    let mut r = 0;
    let mut g = 0;
    let mut b = 0;

    if nr >= 0 && nr < 16 {
        r = ANSI_TABLE[nr as usize][0] as c_int;
        g = ANSI_TABLE[nr as usize][1] as c_int;
        b = ANSI_TABLE[nr as usize][2] as c_int;
    } else if nr >= 16 && nr < 232 {
        let idx = (nr - 16) as usize;
        r = CUBE_VALUE[(idx / 36) % 6];
        g = CUBE_VALUE[(idx / 6) % 6];
        b = CUBE_VALUE[idx % 6];
    } else if nr >= 232 && nr < 256 {
        let idx = (nr - 232) as usize;
        r = GREY_RAMP[idx];
        g = GREY_RAMP[idx];
        b = GREY_RAMP[idx];
    }
    (r << 16) + (g << 8) + b
}

pub unsafe fn syn_attr2entry(attr: c_int) -> HlAttrs {
    unsafe {
        let entries_ptr = &raw const attr_entries;
        if attr <= 0 || (attr as usize) >= (*entries_ptr).len() {
            return HLATTRS_INIT;
        }
        (*entries_ptr).as_slice()[attr as usize].attr
    }
}

pub unsafe fn hl_get_attr_by_id(
    attr_id: Integer,
    rgb: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let dic = Dict {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };

        if attr_id == 0 {
            return dic;
        }

        let entries_ptr = &raw const attr_entries;
        if attr_id < 0 || (attr_id as usize) >= (*entries_ptr).len() {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"Invalid attribute id: %lld".as_ptr(),
                attr_id,
            );
            return dic;
        }
        let mut retval = arena_dict(arena, HLATTRS_DICT_SIZE);
        hlattrs2dict(
            &raw mut retval,
            ptr::null_mut(),
            syn_attr2entry(attr_id as c_int),
            rgb,
            false,
        );
        retval
    }
}

pub unsafe fn hlattrs2dict(
    hl: *mut Dict,
    mut hl_attrs: *mut Dict,
    ae: HlAttrs,
    use_rgb: bool,
    short_keys: bool,
) {
    unsafe {
        if hl_attrs.is_null() {
            hl_attrs = hl;
        }
        assert!((*hl).capacity >= HLATTRS_DICT_SIZE);
        assert!((*hl_attrs).capacity >= HLATTRS_DICT_SIZE);
        let mask = if use_rgb {
            ae.rgb_ae_attr
        } else {
            ae.cterm_ae_attr
        };

        if (mask & HL_INVERSE) != 0 {
            put_c(&mut *hl_attrs, c"reverse", boolean_obj(true));
        }
        if (mask & HL_BOLD) != 0 {
            put_c(&mut *hl_attrs, c"bold", boolean_obj(true));
        }
        if (mask & HL_ITALIC) != 0 {
            put_c(&mut *hl_attrs, c"italic", boolean_obj(true));
        }

        match mask & HL_UNDERLINE_MASK {
            HL_UNDERLINE => put_c(&mut *hl_attrs, c"underline", boolean_obj(true)),
            HL_UNDERCURL => put_c(&mut *hl_attrs, c"undercurl", boolean_obj(true)),
            HL_UNDERDOUBLE => put_c(&mut *hl_attrs, c"underdouble", boolean_obj(true)),
            HL_UNDERDOTTED => put_c(&mut *hl_attrs, c"underdotted", boolean_obj(true)),
            HL_UNDERDASHED => put_c(&mut *hl_attrs, c"underdashed", boolean_obj(true)),
            _ => {}
        }

        if (mask & HL_STANDOUT) != 0 {
            put_c(&mut *hl_attrs, c"standout", boolean_obj(true));
        }
        if (mask & HL_STRIKETHROUGH) != 0 {
            put_c(&mut *hl_attrs, c"strikethrough", boolean_obj(true));
        }
        if (mask & HL_ALTFONT) != 0 {
            put_c(&mut *hl_attrs, c"altfont", boolean_obj(true));
        }
        if (mask & HL_DIM) != 0 {
            put_c(&mut *hl_attrs, c"dim", boolean_obj(true));
        }
        if (mask & HL_BLINK) != 0 {
            put_c(&mut *hl_attrs, c"blink", boolean_obj(true));
        }
        if (mask & HL_CONCEALED) != 0 {
            put_c(&mut *hl_attrs, c"conceal", boolean_obj(true));
        }
        if (mask & HL_OVERLINE) != 0 {
            put_c(&mut *hl_attrs, c"overline", boolean_obj(true));
        }
        if (mask & HL_NOCOMBINE) != 0 {
            put_c(&mut *hl_attrs, c"nocombine", boolean_obj(true));
        }

        if use_rgb {
            if ae.rgb_fg_color != -1 {
                put_c(
                    &mut *hl,
                    if short_keys { c"fg" } else { c"foreground" },
                    integer_obj(Integer::from(ae.rgb_fg_color)),
                );
            }
            if ae.rgb_bg_color != -1 {
                put_c(
                    &mut *hl,
                    if short_keys { c"bg" } else { c"background" },
                    integer_obj(Integer::from(ae.rgb_bg_color)),
                );
            }
            if ae.rgb_sp_color != -1 {
                put_c(
                    &mut *hl,
                    if short_keys { c"sp" } else { c"special" },
                    integer_obj(Integer::from(ae.rgb_sp_color)),
                );
            }
            if (mask & HL_FG_INDEXED) != 0 {
                put_c(&mut *hl, c"fg_indexed", boolean_obj(true));
            }
            if (mask & HL_BG_INDEXED) != 0 {
                put_c(&mut *hl, c"bg_indexed", boolean_obj(true));
            }
            let font = hl_get_font(ae.font);
            if !font.is_null() {
                put_c(&mut *hl, c"font", string_obj(cstr_as_string(font)));
            }
        } else {
            if ae.cterm_fg_color != 0 {
                put_c(
                    &mut *hl,
                    if short_keys {
                        c"ctermfg"
                    } else {
                        c"foreground"
                    },
                    integer_obj(Integer::from(ae.cterm_fg_color - 1)),
                );
            }
            if ae.cterm_bg_color != 0 {
                put_c(
                    &mut *hl,
                    if short_keys {
                        c"ctermbg"
                    } else {
                        c"background"
                    },
                    integer_obj(Integer::from(ae.cterm_bg_color - 1)),
                );
            }
        }

        if ae.hl_blend > -1 && (use_rgb || !short_keys) {
            put_c(&mut *hl, c"blend", integer_obj(Integer::from(ae.hl_blend)));
        }
    }
}

#[inline]
const fn apply_flag_with_key(is_set: u64, opt_idx: u64, val: bool, mut m: i32, flag: i32) -> i32 {
    if has_key(is_set, opt_idx) {
        let cmask = if (flag & HL_UNDERLINE_MASK) != 0 {
            HL_UNDERLINE_MASK
        } else {
            flag
        };
        if val {
            m = (m & !cmask) | flag;
        } else if (m & cmask) == flag {
            m &= !cmask;
        }
    }
    m
}

#[inline]
const fn apply_cterm_flag(val: bool, mut m: i32, flag: i32) -> i32 {
    if val {
        if (flag & HL_UNDERLINE_MASK) != 0 {
            m &= !HL_UNDERLINE_MASK;
        }
        m |= flag;
    }
    m
}

pub unsafe fn dict2hlattrs(
    dict: *mut KeyDict_highlight,
    use_rgb: bool,
    link_id: *mut c_int,
    base: *mut HlAttrs,
    err: *mut Error,
) -> HlAttrs {
    unsafe {
        let mut hlattrs = HLATTRS_INIT;
        let mut fg: i32 = if !base.is_null() {
            (*base).rgb_fg_color
        } else {
            -1
        };
        let mut bg: i32 = if !base.is_null() {
            (*base).rgb_bg_color
        } else {
            -1
        };
        let mut ctermfg: i32 = if !base.is_null() {
            if (*base).cterm_fg_color == 0 {
                -1
            } else {
                i32::from((*base).cterm_fg_color) - 1
            }
        } else {
            -1
        };
        let mut ctermbg: i32 = if !base.is_null() {
            if (*base).cterm_bg_color == 0 {
                -1
            } else {
                i32::from((*base).cterm_bg_color) - 1
            }
        } else {
            -1
        };
        let mut sp: i32 = if !base.is_null() {
            (*base).rgb_sp_color
        } else {
            -1
        };
        let mut blend: i32 = if !base.is_null() {
            (*base).hl_blend
        } else {
            -1
        };
        let mut mask: i32 = if !base.is_null() {
            (*base).rgb_ae_attr
        } else {
            0
        };
        let mut cterm_mask: i32 = if !base.is_null() {
            (*base).cterm_ae_attr
        } else {
            0
        };
        let mut font: i32 = if !base.is_null() { (*base).font } else { -1 };
        let mut cterm_mask_provided = false;

        let is_set = (*dict).is_set__highlight_;
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__reverse,
            (*dict).reverse,
            mask,
            HL_INVERSE,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__bold,
            (*dict).bold,
            mask,
            HL_BOLD,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__italic,
            (*dict).italic,
            mask,
            HL_ITALIC,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__underline,
            (*dict).underline,
            mask,
            HL_UNDERLINE,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__undercurl,
            (*dict).undercurl,
            mask,
            HL_UNDERCURL,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__underdouble,
            (*dict).underdouble,
            mask,
            HL_UNDERDOUBLE,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__underdotted,
            (*dict).underdotted,
            mask,
            HL_UNDERDOTTED,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__underdashed,
            (*dict).underdashed,
            mask,
            HL_UNDERDASHED,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__standout,
            (*dict).standout,
            mask,
            HL_STANDOUT,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__strikethrough,
            (*dict).strikethrough,
            mask,
            HL_STRIKETHROUGH,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__altfont,
            (*dict).altfont,
            mask,
            HL_ALTFONT,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__dim,
            (*dict).dim,
            mask,
            HL_DIM,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__blink,
            (*dict).blink,
            mask,
            HL_BLINK,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__conceal,
            (*dict).conceal,
            mask,
            HL_CONCEALED,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__overline,
            (*dict).overline,
            mask,
            HL_OVERLINE,
        );
        if use_rgb {
            mask = apply_flag_with_key(
                is_set,
                KEYSET_OPTIDX_highlight__fg_indexed,
                (*dict).fg_indexed,
                mask,
                HL_FG_INDEXED,
            );
            mask = apply_flag_with_key(
                is_set,
                KEYSET_OPTIDX_highlight__bg_indexed,
                (*dict).bg_indexed,
                mask,
                HL_BG_INDEXED,
            );
        }
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__nocombine,
            (*dict).nocombine,
            mask,
            HL_NOCOMBINE,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_highlight__default,
            (*dict).default_,
            mask,
            HL_DEFAULT,
        );

        if has_key(is_set, KEYSET_OPTIDX_highlight__fg) {
            fg = object_to_color((*dict).fg, c"fg".as_ptr().cast_mut(), use_rgb, err);
        } else if has_key(is_set, KEYSET_OPTIDX_highlight__foreground) {
            fg = object_to_color(
                (*dict).foreground,
                c"foreground".as_ptr().cast_mut(),
                use_rgb,
                err,
            );
        }
        if (*err).err_type != ErrorType::kErrorTypeNone {
            return hlattrs;
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__bg) {
            bg = object_to_color((*dict).bg, c"bg".as_ptr().cast_mut(), use_rgb, err);
        } else if has_key(is_set, KEYSET_OPTIDX_highlight__background) {
            bg = object_to_color(
                (*dict).background,
                c"background".as_ptr().cast_mut(),
                use_rgb,
                err,
            );
        }
        if (*err).err_type != ErrorType::kErrorTypeNone {
            return hlattrs;
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__sp) {
            sp = object_to_color((*dict).sp, c"sp".as_ptr().cast_mut(), true, err);
        } else if has_key(is_set, KEYSET_OPTIDX_highlight__special) {
            sp = object_to_color((*dict).special, c"special".as_ptr().cast_mut(), true, err);
        }
        if (*err).err_type != ErrorType::kErrorTypeNone {
            return hlattrs;
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__blend) {
            let blend0 = (*dict).blend;
            if !(0..=100).contains(&blend0) {
                api_err_invalid(err, c"blend".as_ptr(), c"out of range".as_ptr(), 0, false);
                return hlattrs;
            }
            blend = blend0 as i32;
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__link)
            || has_key(is_set, KEYSET_OPTIDX_highlight__link_global)
        {
            if link_id.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid Key: '%s'".as_ptr(),
                    if has_key(is_set, KEYSET_OPTIDX_highlight__link_global) {
                        c"link_global".as_ptr()
                    } else {
                        c"link".as_ptr()
                    },
                );
                return hlattrs;
            }
            if has_key(is_set, KEYSET_OPTIDX_highlight__link_global) {
                *link_id = (*dict).link_global as c_int;
                mask |= HL_GLOBAL;
            } else {
                *link_id = (*dict).link as c_int;
            }

            if (*err).err_type != ErrorType::kErrorTypeNone {
                return hlattrs;
            }
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__cterm) {
            let mut cterm = KeyDict_highlight_cterm::default();
            if !api_dict_to_keydict(
                (&raw mut cterm).cast(),
                Some(KeyDict_highlight_cterm_get_field),
                (*dict).cterm,
                err,
            ) {
                return hlattrs;
            }

            cterm_mask_provided = true;
            cterm_mask = 0;
            cterm_mask = apply_cterm_flag(cterm.reverse, cterm_mask, HL_INVERSE);
            cterm_mask = apply_cterm_flag(cterm.bold, cterm_mask, HL_BOLD);
            cterm_mask = apply_cterm_flag(cterm.italic, cterm_mask, HL_ITALIC);
            cterm_mask = apply_cterm_flag(cterm.underline, cterm_mask, HL_UNDERLINE);
            cterm_mask = apply_cterm_flag(cterm.undercurl, cterm_mask, HL_UNDERCURL);
            cterm_mask = apply_cterm_flag(cterm.underdouble, cterm_mask, HL_UNDERDOUBLE);
            cterm_mask = apply_cterm_flag(cterm.underdotted, cterm_mask, HL_UNDERDOTTED);
            cterm_mask = apply_cterm_flag(cterm.underdashed, cterm_mask, HL_UNDERDASHED);
            cterm_mask = apply_cterm_flag(cterm.standout, cterm_mask, HL_STANDOUT);
            cterm_mask = apply_cterm_flag(cterm.strikethrough, cterm_mask, HL_STRIKETHROUGH);
            cterm_mask = apply_cterm_flag(cterm.altfont, cterm_mask, HL_ALTFONT);
            cterm_mask = apply_cterm_flag(cterm.dim, cterm_mask, HL_DIM);
            cterm_mask = apply_cterm_flag(cterm.blink, cterm_mask, HL_BLINK);
            cterm_mask = apply_cterm_flag(cterm.conceal, cterm_mask, HL_CONCEALED);
            cterm_mask = apply_cterm_flag(cterm.overline, cterm_mask, HL_OVERLINE);
            cterm_mask = apply_cterm_flag(cterm.nocombine, cterm_mask, HL_NOCOMBINE);
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__ctermfg) {
            ctermfg = object_to_color((*dict).ctermfg, c"ctermfg".as_ptr().cast_mut(), false, err);
            if (*err).err_type != ErrorType::kErrorTypeNone {
                return hlattrs;
            }
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__ctermbg) {
            ctermbg = object_to_color((*dict).ctermbg, c"ctermbg".as_ptr().cast_mut(), false, err);
            if (*err).err_type != ErrorType::kErrorTypeNone {
                return hlattrs;
            }
        }

        if use_rgb {
            if !cterm_mask_provided {
                cterm_mask = mask;
            }
            hlattrs.rgb_ae_attr = mask;
            hlattrs.rgb_bg_color = bg;
            hlattrs.rgb_fg_color = fg;
            hlattrs.rgb_sp_color = sp;
            hlattrs.hl_blend = blend;
            hlattrs.cterm_bg_color = if ctermbg == -1 {
                0
            } else {
                (ctermbg + 1) as i16
            };
            hlattrs.cterm_fg_color = if ctermfg == -1 {
                0
            } else {
                (ctermfg + 1) as i16
            };
            hlattrs.cterm_ae_attr = cterm_mask;
        } else {
            hlattrs.cterm_bg_color = if bg == -1 { 0 } else { (bg + 1) as i16 };
            hlattrs.cterm_fg_color = if fg == -1 { 0 } else { (fg + 1) as i16 };
            hlattrs.cterm_ae_attr = mask;
        }

        if has_key(is_set, KEYSET_OPTIDX_highlight__font) {
            let str_ = (*dict).font;
            font = if str_.size > 0 && !is_none_str(str_.data) {
                hl_add_font_idx(str_.data)
            } else {
                -1
            };
        }
        hlattrs.font = font;

        hlattrs
    }
}

pub unsafe fn object_to_color(val: Object, key: *mut c_char, rgb: bool, err: *mut Error) -> c_int {
    unsafe {
        if val.obj_type == ObjectType::kObjectTypeInteger {
            val.data.integer as c_int
        } else if val.obj_type == ObjectType::kObjectTypeString {
            let str_ = val.data.string;
            if str_.size == 0 || is_none_str(str_.data) {
                return -1;
            }
            let color = if rgb {
                let mut dummy: c_int = 0;
                name_to_color(str_.data, &raw mut dummy)
            } else {
                name_to_ctermcolor(str_.data)
            };
            if color < 0 {
                api_err_invalid(err, c"highlight color".as_ptr(), str_.data, 0, true);
                return color;
            }
            color
        } else {
            api_err_exp(err, key, c"String or Integer".as_ptr(), ptr::null());
            0
        }
    }
}

pub unsafe fn hl_inspect(attr: c_int, arena: *mut Arena) -> Array {
    unsafe {
        if !hlstate_active {
            return Array {
                size: 0,
                capacity: 0,
                items: ptr::null_mut(),
            };
        }
        let mut ret = arena_array(arena, hl_inspect_size(attr));
        hl_inspect_impl(&raw mut ret, attr, arena);
        ret
    }
}

unsafe fn hl_inspect_size(attr: c_int) -> usize {
    unsafe {
        let entries_ptr = &raw const attr_entries;
        if attr <= 0 || (attr as usize) >= (*entries_ptr).len() {
            return 0;
        }

        let e = (*entries_ptr).as_slice()[attr as usize];
        if e.kind == kHlCombine || e.kind == kHlBlend || e.kind == kHlBlendThrough {
            return hl_inspect_size(e.id1) + hl_inspect_size(e.id2);
        }
        1
    }
}

unsafe fn hl_inspect_impl(arr: *mut Array, attr: c_int, arena: *mut Arena) {
    unsafe {
        let entries_ptr = &raw const attr_entries;
        if attr <= 0 || (attr as usize) >= (*entries_ptr).len() {
            return;
        }

        let e = (*entries_ptr).as_slice()[attr as usize];
        let mut item: Dict;
        match e.kind {
            kHlSyntax => {
                item = arena_dict(arena, 3);
                put_c(&mut item, c"kind", cstr_as_obj(c"syntax".as_ptr()));
                put_c(&mut item, c"hi_name", cstr_as_obj(syn_id2name(e.id1)));
            }
            kHlUI => {
                item = arena_dict(arena, 4);
                put_c(&mut item, c"kind", cstr_as_obj(c"ui".as_ptr()));
                let ui_name = if e.id1 == -1 {
                    c"Normal".as_ptr()
                } else {
                    hlf_names[e.id1 as usize]
                };
                put_c(&mut item, c"ui_name", cstr_as_obj(ui_name));
                put_c(&mut item, c"hi_name", cstr_as_obj(syn_id2name(e.id2)));
            }
            kHlTerminal => {
                item = arena_dict(arena, 2);
                put_c(&mut item, c"kind", cstr_as_obj(c"term".as_ptr()));
            }
            kHlCombine | kHlBlend | kHlBlendThrough => {
                hl_inspect_impl(arr, e.id1, arena);
                hl_inspect_impl(arr, e.id2, arena);
                return;
            }
            _ => return,
        }
        put_c(&mut item, c"id", integer_obj(Integer::from(attr)));
        add_c(&mut *arr, dict_obj(item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hl_font_interning_handles_null_and_roundtrips() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            assert_eq!(hl_add_font_idx(ptr::null()), -1);
            assert_eq!(hl_add_font_idx(c"".as_ptr()), -1);
            let idx = hl_add_font_idx(c"Monospace:h12".as_ptr());
            assert!(idx >= 0);
            assert_eq!(hl_add_font_idx(c"Monospace:h12".as_ptr()), idx);
            let got = hl_get_font(idx);
            assert_eq!(CStr::from_ptr(got).to_bytes(), b"Monospace:h12");
        }
    }
}
