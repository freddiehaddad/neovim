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
use crate::lua::executor::{K_RET_OBJECT, nlua_call_ref};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::message::emsg;
use crate::msgpack_rpc::packer::Integer;
use crate::option::check_blending;
use crate::option_vars::*;
use crate::popupmenu::pum_drawn;
use crate::strings::gettext;
use crate::ui::ui_call_hl_attr_define;
use crate::window::WinInfoInfo;
use std::ffi::CStr;
use std::ptr;

pub type RgbValue = i32;
pub type OptionalKeys = u64;
use crate::msgpack_rpc::channel::RemoteUI;
pub type HlKind = i32;

const MAX_TYPENR: u32 = 65535;

pub(crate) const HLF_NONE: usize = 0;
pub(crate) const HLF_PNI: i32 = 41;
pub(crate) const HLF_PST: i32 = 50;
pub(crate) const HLF_INACTIVE: usize = 60;
const HLF_NFLOAT: usize = 62;
const HLF_BORDER: usize = 64;
pub(crate) const HLF_COUNT: usize = 76;

const K_HL_UNKNOWN: HlKind = 0;
const K_HL_UI: HlKind = 1;
const K_HL_SYNTAX: HlKind = 2;
const K_HL_TERMINAL: HlKind = 3;
const K_HL_COMBINE: HlKind = 4;
const K_HL_BLEND: HlKind = 5;
const K_HL_BLEND_THROUGH: HlKind = 6;
const K_HL_INVALID: HlKind = 7;

pub(crate) const HL_ALTFONT: i32 = 0x0100;
pub(crate) const HL_BG_INDEXED: i32 = 0x0800;
pub(crate) const HL_FG_INDEXED: i32 = 0x1000;
pub(crate) const HL_DEFAULT: i32 = 0x2000;
pub(crate) const HL_GLOBAL: i32 = 0x4000;

pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_BG: u64 = 1;
pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_FG: u64 = 2;
pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_SP: u64 = 3;
const KEYSET_OPTIDX_HIGHLIGHT_DIM: u64 = 4;
pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_URL: u64 = 5;
const KEYSET_OPTIDX_HIGHLIGHT_BOLD: u64 = 6;
pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_FONT: u64 = 7;
const KEYSET_OPTIDX_HIGHLIGHT_LINK: u64 = 8;
const KEYSET_OPTIDX_HIGHLIGHT_BLEND: u64 = 9;
const KEYSET_OPTIDX_HIGHLIGHT_BLINK: u64 = 11;
const KEYSET_OPTIDX_HIGHLIGHT_CTERM: u64 = 12;
const KEYSET_OPTIDX_HIGHLIGHT_ITALIC: u64 = 13;
pub(crate) const KEYSET_OPTIDX_HIGHLIGHT_UPDATE: u64 = 14;
const KEYSET_OPTIDX_HIGHLIGHT_REVERSE: u64 = 15;
const KEYSET_OPTIDX_HIGHLIGHT_DEFAULT: u64 = 16;
const KEYSET_OPTIDX_HIGHLIGHT_ALTFONT: u64 = 17;
const KEYSET_OPTIDX_HIGHLIGHT_CONCEAL: u64 = 18;
const KEYSET_OPTIDX_HIGHLIGHT_SPECIAL: u64 = 19;
const KEYSET_OPTIDX_HIGHLIGHT_CTERMFG: u64 = 20;
const KEYSET_OPTIDX_HIGHLIGHT_CTERMBG: u64 = 21;
const KEYSET_OPTIDX_HIGHLIGHT_FALLBACK: u64 = 22;
const KEYSET_OPTIDX_HIGHLIGHT_OVERLINE: u64 = 23;
const KEYSET_OPTIDX_HIGHLIGHT_STANDOUT: u64 = 24;
const KEYSET_OPTIDX_HIGHLIGHT_NOCOMBINE: u64 = 25;
const KEYSET_OPTIDX_HIGHLIGHT_UNDERCURL: u64 = 26;
const KEYSET_OPTIDX_HIGHLIGHT_UNDERLINE: u64 = 27;
const KEYSET_OPTIDX_HIGHLIGHT_BACKGROUND: u64 = 28;
const KEYSET_OPTIDX_HIGHLIGHT_BG_INDEXED: u64 = 29;
const KEYSET_OPTIDX_HIGHLIGHT_FOREGROUND: u64 = 30;
const KEYSET_OPTIDX_HIGHLIGHT_FG_INDEXED: u64 = 31;
const KEYSET_OPTIDX_HIGHLIGHT_LINK_GLOBAL: u64 = 32;
const KEYSET_OPTIDX_HIGHLIGHT_UNDERDASHED: u64 = 33;
const KEYSET_OPTIDX_HIGHLIGHT_UNDERDOTTED: u64 = 34;
const KEYSET_OPTIDX_HIGHLIGHT_UNDERDOUBLE: u64 = 35;
const KEYSET_OPTIDX_HIGHLIGHT_STRIKETHROUGH: u64 = 36;

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
#[derive(Clone, Copy, Default)]
pub struct KeyDict_highlight {
    pub is_set_highlight: OptionalKeys,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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
    pub kind: i32,
    pub id1: i32,
    pub id2: i32,
    pub winid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColorKey {
    pub ns_id: i32,
    pub syn_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColorItem {
    pub attr_id: i32,
    pub link_id: i32,
    pub version: i32,
    pub is_default: bool,
    pub link_global: bool,
}

pub type NSHlAttr = [std::cell::Cell<i32>; HLF_COUNT];

use crate::api::private::helpers::has_key;

struct HighlightModuleState {
    hlstate_active: bool,
    attr_entries: Vec<HlEntry>,
    attr_entry_ids: std::collections::BTreeMap<HlEntry, u32>,
    combine_attr_entries: std::collections::BTreeMap<u64, i32>,
    blend_attr_entries: std::collections::BTreeMap<u64, i32>,
    blendthrough_attr_entries: std::collections::BTreeMap<u64, i32>,
    urls: Vec<std::ffi::CString>,
    fonts: Vec<std::ffi::CString>,
    ns_hls: std::collections::BTreeMap<ColorKey, ColorItem>,
    ns_hl_attr: std::collections::BTreeMap<i32, std::rc::Rc<NSHlAttr>>,
    get_attr_entry_recursive: bool,
    ns_get_hl_recursive: i32,
}

impl HighlightModuleState {
    const fn new() -> Self {
        Self {
            hlstate_active: false,
            attr_entries: Vec::new(),
            attr_entry_ids: std::collections::BTreeMap::new(),
            combine_attr_entries: std::collections::BTreeMap::new(),
            blend_attr_entries: std::collections::BTreeMap::new(),
            blendthrough_attr_entries: std::collections::BTreeMap::new(),
            urls: Vec::new(),
            fonts: Vec::new(),
            ns_hls: std::collections::BTreeMap::new(),
            ns_hl_attr: std::collections::BTreeMap::new(),
            get_attr_entry_recursive: false,
            ns_get_hl_recursive: 0,
        }
    }
}

static HL_STATE: GlobalState<HighlightModuleState> = GlobalState::new(HighlightModuleState::new());

#[inline]
fn hl_st() -> &'static mut HighlightModuleState {
    unsafe { &mut *HL_STATE.as_ptr() }
}

#[inline]
fn is_none_str(s: &[u8]) -> bool {
    s.eq_ignore_ascii_case(b"NONE")
}

use crate::api::private::dispatch::{
    key_dict_highlight_cterm_get_field, key_dict_highlight_get_field,
};
use crate::drawline::{HL_OVERLINE, HL_STRIKETHROUGH, HL_UNDERLINE_MASK};
use crate::eval::funcs::{
    HL_BLINK, HL_BOLD, HL_CONCEALED, HL_DIM, HL_INVERSE, HL_ITALIC, HL_NOCOMBINE, HL_STANDOUT,
    HL_UNDERCURL, HL_UNDERDASHED, HL_UNDERDOTTED, HL_UNDERDOUBLE, HL_UNDERLINE,
};
use crate::lua::converter::LUA_NOREF;

#[inline]
pub fn win_hl_attr(wp: &mut WinInfoInfo, hlf: i32) -> i32 {
    if let Some(ref tbl) = wp.w_ns_hl_attr
        && g_st().ns_hl_fast < 0
    {
        tbl[hlf as usize].get()
    } else {
        hl_attr(hlf)
    }
}

#[inline]
pub fn hl_attr(n: i32) -> i32 {
    unsafe {
        let ha = &raw const g_st().hl_attr_active;
        if let Some(ref tbl) = *ha {
            tbl[n as usize].get()
        } else {
            g_st().highlight_attr[n as usize]
        }
    }
}

#[inline]
const fn hl_attr_key(a: i32, b: i32) -> u64 {
    ((a as u32 as u64) << 32) | (b as u32 as u64)
}

pub fn highlight_init() {
    if hl_st().attr_entries.is_empty() {
        let entry = HlEntry {
            attr: HLATTRS_INIT,
            kind: K_HL_INVALID,
            id1: 0,
            id2: 0,
            winid: 0,
        };
        hl_st().attr_entries.push(entry);
        hl_st().attr_entry_ids.insert(entry, 0);
    }
}

pub fn highlight_use_hlstate() -> bool {
    if hl_st().hlstate_active {
        return false;
    }
    hl_st().hlstate_active = true;
    clear_hl_tables(true);
    true
}

fn get_attr_entry(mut entry: HlEntry) -> i32 {
    let mut retried = false;
    if !hl_st().hlstate_active {
        entry.kind = K_HL_UNKNOWN;
        entry.id1 = 0;
        entry.id2 = 0;
    }

    loop {
        if let Some(&k) = hl_st().attr_entry_ids.get(&entry) {
            return k as i32;
        }

        if hl_st().attr_entries.len() as u32 >= MAX_TYPENR {
            if hl_st().get_attr_entry_recursive || retried {
                emsg(gettext(
                    c"E424: Too many different highlighting attributes in use",
                ));
                return 0;
            }
            hl_st().get_attr_entry_recursive = true;
            clear_hl_tables(true);
            hl_st().get_attr_entry_recursive = false;

            if entry.kind == K_HL_COMBINE {
                return 0;
            }
            retried = true;
            continue;
        }

        let k = hl_st().attr_entries.len() as u32;
        hl_st().attr_entries.push(entry);
        hl_st().attr_entry_ids.insert(entry, k);

        let id = k as i32;
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let inspect = hl_inspect(id, &mut arena);
        ui_call_hl_attr_define(Integer::from(id), entry.attr, entry.attr, inspect);
        arena_mem_free(arena_finish(&mut arena));
        return id;
    }
}

pub fn ui_send_all_hls(ui: &mut RemoteUI) {
    let len = hl_st().attr_entries.len();
    for i in 1..len {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let inspect = hl_inspect(i as i32, &mut arena);
        let attr = hl_st().attr_entries.as_slice()[i].attr;
        remote_ui_hl_attr_define(ui, i as Integer, attr, attr, inspect);
        arena_mem_free(arena_finish(&mut arena));
    }
    for hlf in 0..HLF_COUNT {
        remote_ui_hl_group_set(
            ui,
            cstr_as_string(hlf_name_ptr(hlf)),
            Integer::from(g_st().highlight_attr[hlf]),
        );
    }
}

pub fn hl_get_syn_attr(ns_id: i32, idx: i32, at_en: HlAttrs) -> i32 {
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
            kind: K_HL_SYNTAX,
            id1: idx,
            id2: ns_id,
            winid: 0,
        });
    }
    0
}

pub fn ns_hl_def(
    ns_id: NS,
    hl_id: i32,
    attrs: HlAttrs,
    link_id: i32,
    dict: Option<&KeyDict_highlight>,
) {
    if ns_id == 0 {
        let dict = dict.expect("ns_hl_def with ns_id == 0 requires dict");
        set_hl_group(hl_id, attrs, dict, link_id);
        return;
    }
    let key = ColorKey {
        ns_id,
        syn_id: hl_id,
    };
    if (attrs.rgb_ae_attr & HL_DEFAULT) != 0 && hl_st().ns_hls.contains_key(&key) {
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
        version: unsafe { (*p).hl_valid },
        is_default: (attrs.rgb_ae_attr & HL_DEFAULT) != 0,
        link_global: (attrs.rgb_ae_attr & HL_GLOBAL) != 0,
    };
    hl_st().ns_hls.insert(key, it);
    unsafe {
        (*p).hl_cached = false;
    }
}

pub fn ns_get_hl(ns_hl: &mut NS, hl_id: i32, link: bool, nodefault: bool) -> i32 {
    if *ns_hl == 0 {
        return -1;
    }

    if *ns_hl < 0 {
        if g_st().ns_hl_active <= 0 {
            return -1;
        }
        *ns_hl = g_st().ns_hl_active;
    }

    let ns_id = *ns_hl;
    let p = get_decor_provider(ns_id, true);
    let key = ColorKey {
        ns_id,
        syn_id: hl_id,
    };
    let mut it = hl_st().ns_hls.get(&key).copied().unwrap_or(ColorItem {
        attr_id: -1,
        link_id: -1,
        version: -1,
        is_default: false,
        link_global: false,
    });
    let mut valid_item = it.version >= unsafe { (*p).hl_valid };

    if !valid_item && unsafe { (*p).hl_def } != LUA_NOREF && hl_st().ns_get_hl_recursive == 0 {
        let mut args_storage = [Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Integer(0),
        }; 3];
        let mut args = Array {
            size: 0,
            capacity: 3,
            items: args_storage.as_mut_ptr(),
        };
        add_c(&mut args, integer_obj(Integer::from(ns_id)));
        add_c(&mut args, cstr_as_obj(syn_id2name(hl_id)));
        add_c(&mut args, boolean_obj(link));

        let mut err = Error::new();
        hl_st().ns_get_hl_recursive += 1;
        let ret = nlua_call_ref(
            unsafe { (*p).hl_def },
            c"hl_def",
            args,
            K_RET_OBJECT,
            None,
            Some(&mut err),
        );
        hl_st().ns_get_hl_recursive -= 1;

        let mut fallback = true;
        let mut tmp = false;
        let mut attrs = HLATTRS_INIT;
        if ret.obj_type == ObjectType::ObjectTypeDict {
            fallback = false;
            let mut dict = KeyDict_highlight::default();
            if api_dict_to_keydict(
                (&raw mut dict).cast(),
                Some(key_dict_highlight_get_field),
                ret.data.dict(),
                &mut err,
            ) {
                attrs = dict2hlattrs(&mut dict, true, Some(&mut it.link_id), None, &mut err);
                fallback = if has_key(dict.is_set_highlight, KEYSET_OPTIDX_HIGHLIGHT_FALLBACK) {
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
        it.version = unsafe { (*p).hl_valid } - i32::from(tmp);
        it.is_default = (attrs.rgb_ae_attr & HL_DEFAULT) != 0;
        it.link_global = (attrs.rgb_ae_attr & HL_GLOBAL) != 0;
        hl_st().ns_hls.insert(key, it);
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

pub fn hl_check_ns() -> bool {
    let ns = if g_st().ns_hl_fast > 0 {
        g_st().ns_hl_fast
    } else if g_st().ns_hl_win >= 0 {
        g_st().ns_hl_win
    } else {
        g_st().ns_hl_global
    };
    if g_st().ns_hl_active == ns {
        return false;
    }

    g_st().ns_hl_active = ns;
    g_st().hl_attr_active = None;
    if ns > 0 {
        update_ns_hl(ns);
        if let Some(hl_def) = hl_st().ns_hl_attr.get(&ns) {
            g_st().hl_attr_active = Some(hl_def.clone());
        }
    }
    true
}

pub fn win_check_ns_hl(wp: Option<&WinInfoInfo>) -> bool {
    g_st().ns_hl_win = wp.map_or(-1, |wp| wp.w_ns_hl);
    hl_check_ns()
}

pub fn hl_ns_get_attrs(
    ns_id: i32,
    hl_id: i32,
    mut optional: Option<&mut bool>,
    attrs: &mut HlAttrs,
) -> bool {
    let mut opt = if let Some(optional) = optional.as_deref_mut() {
        *optional
    } else {
        true
    };
    let syn_attr = syn_ns_id2attr(ns_id, hl_id, &mut opt);
    if let Some(optional) = optional {
        *optional = opt;
    }
    if syn_attr <= 0 {
        return false;
    }
    *attrs = syn_attr2entry(syn_attr);
    true
}

pub fn hl_get_ui_attr(ns_id: i32, idx: i32, final_id: i32, mut optional: bool) -> i32 {
    let mut attrs = HLATTRS_INIT;
    let mut available = false;

    if final_id > 0 {
        available = hl_ns_get_attrs(ns_id, final_id, Some(&mut optional), &mut attrs);
    }

    if (HLF_PNI..=HLF_PST).contains(&idx) {
        if attrs.hl_blend == -1 && opt_vars().p_pb > 0 {
            attrs.hl_blend = opt_vars().p_pb as i32;
        }
        if pum_drawn() {
            g_st().must_redraw_pum = true;
        }
    }

    if optional && !available {
        return 0;
    }
    get_attr_entry(HlEntry {
        attr: attrs,
        kind: K_HL_UI,
        id1: idx,
        id2: final_id,
        winid: 0,
    })
}

pub fn hl_apply_winblend(winbl: i32, mut attr: i32) -> i32 {
    let mut entry = hl_st().attr_entries.as_slice()[attr as usize];
    if entry.attr.hl_blend == -1 && winbl > 0 {
        entry.attr.hl_blend = winbl;
        attr = get_attr_entry(entry);
    }
    attr
}

pub fn update_window_hl(wp: &mut WinInfoInfo, invalid: bool) {
    let ns_id = wp.w_ns_hl;

    update_ns_hl(ns_id);
    if ns_id != wp.w_ns_hl_active {
        wp.w_ns_hl_active = ns_id;

        wp.w_ns_hl_attr = hl_st().ns_hl_attr.get(&ns_id).cloned();
    }

    let w_ns_hl_attr = wp.w_ns_hl_attr.clone();
    let hl_def_get = |idx: usize| -> i32 {
        if let Some(ref tbl) = w_ns_hl_attr {
            tbl[idx].get()
        } else {
            hl_attr(idx as i32)
        }
    };

    if wp.w_hl_needs_update == 0 && !invalid {
        return;
    }
    wp.w_hl_needs_update = 0;

    let float_win = wp.w_floating && !wp.w_config.external;
    if float_win && hl_def_get(HLF_NFLOAT) != 0 && ns_id > 0 {
        wp.w_hl_attr_normal = hl_def_get(HLF_NFLOAT);
    } else if hl_def_get(HLF_NONE) > 0 {
        wp.w_hl_attr_normal = hl_def_get(HLF_NONE);
    } else if float_win {
        wp.w_hl_attr_normal = if hl_attr(HLF_NFLOAT as i32) > 0 {
            hl_attr(HLF_NFLOAT as i32)
        } else {
            g_st().highlight_attr[HLF_NFLOAT]
        };
    } else {
        wp.w_hl_attr_normal = 0;
    }

    if wp.w_floating {
        wp.w_hl_attr_normal =
            hl_apply_winblend(wp.w_onebuf_opt.wo_winbl as i32, wp.w_hl_attr_normal);
    }

    wp.w_config.shadow = false;
    if wp.w_floating && wp.w_config.border {
        for i in 0..8 {
            let mut attr = hl_def_get(HLF_BORDER);
            if wp.w_config.border_hl_ids[i] != 0 {
                attr = hl_get_ui_attr(
                    ns_id,
                    HLF_BORDER as i32,
                    wp.w_config.border_hl_ids[i],
                    false,
                );
            }
            attr = hl_apply_winblend(wp.w_onebuf_opt.wo_winbl as i32, attr);
            if syn_attr2entry(attr).hl_blend > 0 {
                wp.w_config.shadow = true;
            }
            wp.w_config.border_attr[i] = attr;
        }
    }

    check_blending(&mut *wp);

    if hl_def_get(HLF_INACTIVE) == 0 {
        wp.w_hl_attr_normalnc = hl_combine_attr(hl_attr(HLF_INACTIVE as i32), wp.w_hl_attr_normal);
    } else {
        wp.w_hl_attr_normalnc = hl_def_get(HLF_INACTIVE);
    }

    if wp.w_floating {
        wp.w_hl_attr_normalnc =
            hl_apply_winblend(wp.w_onebuf_opt.wo_winbl as i32, wp.w_hl_attr_normalnc);
    }
}

pub fn update_ns_hl(ns_id: i32) {
    if ns_id <= 0 {
        return;
    }
    let mut p = get_decor_provider(ns_id, true);
    if unsafe { (*p).hl_cached } {
        return;
    }

    let alloc = hl_st()
        .ns_hl_attr
        .entry(ns_id)
        .or_insert_with(|| std::rc::Rc::new([const { std::cell::Cell::new(0) }; HLF_COUNT]))
        .clone();

    for hlf in 1..HLF_COUNT {
        let name_cstr = HLF_NAMES[hlf].unwrap();
        let id = syn_check_group(name_cstr.as_ptr(), name_cstr.to_bytes().len());
        let optional = hlf == HLF_INACTIVE || hlf == HLF_NFLOAT;
        alloc[hlf].set(hl_get_ui_attr(ns_id, hlf as i32, id, optional));
    }

    let normality = syn_check_group(c"Normal", 6);
    alloc[HLF_NONE].set(hl_get_ui_attr(ns_id, -1, normality, true));

    p = get_decor_provider(ns_id, true);
    unsafe {
        (*p).hl_cached = true;
    }
}

pub fn win_bg_attr(wp: &mut WinInfoInfo) -> i32 {
    if g_st().ns_hl_fast < 0 {
        let local = if ptr::eq(wp, curwin_ptr()) {
            wp.w_hl_attr_normal
        } else {
            wp.w_hl_attr_normalnc
        };
        if local != 0 {
            return local;
        }
    }

    if ptr::eq(wp, curwin_ptr()) || hl_attr(HLF_INACTIVE as i32) == 0 {
        hl_attr(HLF_NONE as i32)
    } else {
        hl_attr(HLF_INACTIVE as i32)
    }
}

pub fn hl_get_underline() -> i32 {
    let mut attrs = HLATTRS_INIT;
    attrs.cterm_ae_attr = HL_UNDERLINE;
    attrs.rgb_ae_attr = HL_UNDERLINE;
    get_attr_entry(HlEntry {
        attr: attrs,
        kind: K_HL_UI,
        id1: 0,
        id2: 0,
        winid: 0,
    })
}

pub fn hl_add_url(attr: i32, url: &CStr) -> i32 {
    let mut attrs = HLATTRS_INIT;
    let k = if let Some(pos) = hl_st().urls.iter().position(|u| u.as_c_str() == url) {
        pos
    } else {
        let idx = hl_st().urls.len();
        hl_st().urls.push(url.to_owned());
        idx
    };
    attrs.url = k as i32;

    let new = get_attr_entry(HlEntry {
        attr: attrs,
        kind: K_HL_UI,
        id1: 0,
        id2: 0,
        winid: 0,
    });
    hl_combine_attr(attr, new)
}

pub fn hl_get_url(index: u32) -> &'static CStr {
    hl_st().urls.as_slice()[index as usize].as_c_str()
}

pub fn hl_add_font_idx(font_name: Option<&[u8]>) -> i32 {
    let Some(font_bytes) = font_name else {
        return -1;
    };
    if font_bytes.is_empty() {
        return -1;
    }
    if let Some(pos) = hl_st()
        .fonts
        .iter()
        .position(|f| f.as_bytes() == font_bytes)
    {
        return pos as i32;
    }
    let idx = hl_st().fonts.len() as i32;
    hl_st()
        .fonts
        .push(crate::memory::cstr_from_bytes(font_bytes));
    idx
}

pub fn hl_get_font(index: i32) -> Option<&'static CStr> {
    if index < 0 || (index as usize) >= hl_st().fonts.len() {
        return None;
    }
    Some(hl_st().fonts.as_slice()[index as usize].as_c_str())
}

pub fn hl_get_term_attr(aep: &mut HlAttrs) -> i32 {
    get_attr_entry(HlEntry {
        attr: *aep,
        kind: K_HL_TERMINAL,
        id1: 0,
        id2: 0,
        winid: 0,
    })
}

pub fn clear_hl_tables(reinit: bool) {
    if reinit {
        hl_st().attr_entries.clear();
        hl_st().attr_entry_ids.clear();
        highlight_init();
        hl_st().combine_attr_entries.clear();
        hl_st().blend_attr_entries.clear();
        hl_st().blendthrough_attr_entries.clear();
        hl_st().urls.clear();
        g_st().highlight_attr_last = [-1; HLF_COUNT];
        highlight_attr_set_all();
        highlight_changed();
        screen_invalidate_highlights();
    } else {
        hl_st().attr_entries = Vec::new();
        hl_st().attr_entry_ids = std::collections::BTreeMap::new();
        hl_st().combine_attr_entries = std::collections::BTreeMap::new();
        hl_st().blend_attr_entries = std::collections::BTreeMap::new();
        hl_st().blendthrough_attr_entries = std::collections::BTreeMap::new();
        hl_st().ns_hls = std::collections::BTreeMap::new();
        hl_st().ns_hl_attr = std::collections::BTreeMap::new();
        hl_st().urls = Vec::new();
        hl_st().fonts = Vec::new();
    }
}

pub fn hl_invalidate_blends() {
    hl_st().blend_attr_entries.clear();
    hl_st().blendthrough_attr_entries.clear();
    highlight_changed();
    update_window_hl(curwin_mut(), true);
}

#[inline]
const fn hl_combine_ae(char_ae: i32, prim_ae: i32) -> i32 {
    let char_ul = char_ae & HL_UNDERLINE_MASK;
    let prim_ul = prim_ae & HL_UNDERLINE_MASK;
    let new_ul = if prim_ul != 0 { prim_ul } else { char_ul };
    (char_ae & !HL_UNDERLINE_MASK) | (prim_ae & !HL_UNDERLINE_MASK) | new_ul
}

pub fn hl_combine_attr(char_attr: i32, prim_attr: i32) -> i32 {
    if char_attr == 0 {
        return prim_attr;
    } else if prim_attr == 0 {
        return char_attr;
    }

    let combine_tag = hl_attr_key(char_attr, prim_attr);
    let mut id = hl_st()
        .combine_attr_entries
        .get(&combine_tag)
        .copied()
        .unwrap_or(0);
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
        kind: K_HL_COMBINE,
        id1: char_attr,
        id2: prim_attr,
        winid: 0,
    });
    if id > 0 {
        hl_st().combine_attr_entries.insert(combine_tag, id);
    }

    id
}

fn get_colors_force(mut attrs: HlAttrs) -> HlAttrs {
    if attrs.rgb_bg_color == -1 {
        attrs.rgb_bg_color = g_st().normal_bg;
    }
    if attrs.rgb_fg_color == -1 {
        attrs.rgb_fg_color = g_st().normal_fg;
    }
    if attrs.rgb_sp_color == -1 {
        attrs.rgb_sp_color = g_st().normal_sp;
    }
    let dark = crate::memory::cstr_first_byte(opt_vars().p_bg) == b'd';
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

pub fn hl_blend_attrs(back_attr: i32, front_attr: i32, through: &mut bool) -> i32 {
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
    let map_ptr = if *through {
        &raw mut hl_st().blendthrough_attr_entries
    } else {
        &raw mut hl_st().blend_attr_entries
    };
    let map = unsafe { &mut *map_ptr };
    let mut id = map.get(&combine_tag).copied().unwrap_or(0);
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
            cattrs.rgb_sp_color = rgb_blend(ratio / 2, battrs.rgb_bg_color, fattrs.rgb_sp_color);
        } else {
            cattrs.rgb_sp_color = -1;
        }

        cattrs.rgb_ae_attr &= !(HL_FG_INDEXED | HL_BG_INDEXED);
    }

    if ratio == 100 && battrs_raw.rgb_bg_color == -1 {
        cattrs.rgb_bg_color = -1;
    } else {
        cattrs.rgb_bg_color = if battrs_raw.rgb_bg_color == -1 && fattrs_raw.rgb_bg_color == -1 {
            -1
        } else {
            rgb_blend(ratio, battrs.rgb_bg_color, fattrs.rgb_bg_color)
        };
    }
    cattrs.hl_blend = -1;
    let kind = if *through {
        K_HL_BLEND_THROUGH
    } else {
        K_HL_BLEND
    };
    id = get_attr_entry(HlEntry {
        attr: cattrs,
        kind,
        id1: back_attr,
        id2: front_attr,
        winid: 0,
    });
    if id > 0 {
        map.insert(combine_tag, id);
    }
    id
}

const fn rgb_blend(ratio: i32, rgb1: i32, rgb2: i32) -> i32 {
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

const fn cterm_blend(ratio: i32, c1: i16, c2: i16) -> i32 {
    let rgb1 = hl_cterm2rgb_color(c1 as i32);
    let rgb2 = hl_cterm2rgb_color(c2 as i32);
    let rgb_blended = rgb_blend(ratio, rgb1, rgb2);
    hl_rgb2cterm_color(rgb_blended)
}

const fn hl_rgb2cterm_color(rgb: i32) -> i32 {
    let r = (rgb & 0xFF0000) >> 16;
    let g = (rgb & 0x00FF00) >> 8;
    let b = rgb & 0x0000FF;

    (r * 6 / 256) * 36 + (g * 6 / 256) * 6 + (b * 6 / 256)
}

const fn hl_cterm2rgb_color(nr: i32) -> i32 {
    const CUBE_VALUE: [i32; 6] = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
    const GREY_RAMP: [i32; 24] = [
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
        r = ANSI_TABLE[nr as usize][0] as i32;
        g = ANSI_TABLE[nr as usize][1] as i32;
        b = ANSI_TABLE[nr as usize][2] as i32;
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

pub fn syn_attr2entry(attr: i32) -> HlAttrs {
    if attr <= 0 || (attr as usize) >= hl_st().attr_entries.len() {
        return HLATTRS_INIT;
    }
    hl_st().attr_entries.as_slice()[attr as usize].attr
}

pub fn hl_get_attr_by_id(
    attr_id: Integer,
    rgb: Boolean,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
    let dic = Dict {
        size: 0,
        capacity: 0,
        items: ptr::null_mut(),
    };

    if attr_id == 0 {
        return dic;
    }

    if attr_id < 0 || (attr_id as usize) >= hl_st().attr_entries.len() {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"Invalid attribute id: %lld",
            attr_id,
        );
        return dic;
    }
    let mut retval = arena_dict(arena, HLATTRS_DICT_SIZE);
    hlattrs2dict(
        &mut retval,
        None,
        syn_attr2entry(attr_id as i32),
        rgb,
        false,
    );
    retval
}

pub fn hlattrs2dict(
    hl: &mut Dict,
    hl_attrs: Option<&mut Dict>,
    ae: HlAttrs,
    use_rgb: bool,
    short_keys: bool,
) {
    unsafe {
        let hl_attrs: *mut Dict = match hl_attrs {
            Some(a) => &raw mut *a,
            None => &raw mut *hl,
        };
        assert!(hl.capacity >= HLATTRS_DICT_SIZE);
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
            if let Some(font) = hl_get_font(ae.font) {
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

pub fn dict2hlattrs(
    dict: &mut KeyDict_highlight,
    use_rgb: bool,
    mut link_id: Option<&mut i32>,
    base: Option<&HlAttrs>,
    err: &mut Error,
) -> HlAttrs {
    let mut hlattrs = HLATTRS_INIT;
    let mut fg: i32 = if let Some(base) = base {
        base.rgb_fg_color
    } else {
        -1
    };
    let mut bg: i32 = if let Some(base) = base {
        base.rgb_bg_color
    } else {
        -1
    };
    let mut ctermfg: i32 = if let Some(base) = base {
        if base.cterm_fg_color == 0 {
            -1
        } else {
            i32::from(base.cterm_fg_color) - 1
        }
    } else {
        -1
    };
    let mut ctermbg: i32 = if let Some(base) = base {
        if base.cterm_bg_color == 0 {
            -1
        } else {
            i32::from(base.cterm_bg_color) - 1
        }
    } else {
        -1
    };
    let mut sp: i32 = if let Some(base) = base {
        base.rgb_sp_color
    } else {
        -1
    };
    let mut blend: i32 = if let Some(base) = base {
        base.hl_blend
    } else {
        -1
    };
    let mut mask: i32 = if let Some(base) = base {
        base.rgb_ae_attr
    } else {
        0
    };
    let mut cterm_mask: i32 = if let Some(base) = base {
        base.cterm_ae_attr
    } else {
        0
    };
    let mut font: i32 = if let Some(base) = base { base.font } else { -1 };
    let mut cterm_mask_provided = false;

    let is_set = dict.is_set_highlight;
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_REVERSE,
        dict.reverse,
        mask,
        HL_INVERSE,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_BOLD,
        dict.bold,
        mask,
        HL_BOLD,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_ITALIC,
        dict.italic,
        mask,
        HL_ITALIC,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_UNDERLINE,
        dict.underline,
        mask,
        HL_UNDERLINE,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_UNDERCURL,
        dict.undercurl,
        mask,
        HL_UNDERCURL,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_UNDERDOUBLE,
        dict.underdouble,
        mask,
        HL_UNDERDOUBLE,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_UNDERDOTTED,
        dict.underdotted,
        mask,
        HL_UNDERDOTTED,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_UNDERDASHED,
        dict.underdashed,
        mask,
        HL_UNDERDASHED,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_STANDOUT,
        dict.standout,
        mask,
        HL_STANDOUT,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_STRIKETHROUGH,
        dict.strikethrough,
        mask,
        HL_STRIKETHROUGH,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_ALTFONT,
        dict.altfont,
        mask,
        HL_ALTFONT,
    );
    mask = apply_flag_with_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_DIM, dict.dim, mask, HL_DIM);
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_BLINK,
        dict.blink,
        mask,
        HL_BLINK,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_CONCEAL,
        dict.conceal,
        mask,
        HL_CONCEALED,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_OVERLINE,
        dict.overline,
        mask,
        HL_OVERLINE,
    );
    if use_rgb {
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_HIGHLIGHT_FG_INDEXED,
            dict.fg_indexed,
            mask,
            HL_FG_INDEXED,
        );
        mask = apply_flag_with_key(
            is_set,
            KEYSET_OPTIDX_HIGHLIGHT_BG_INDEXED,
            dict.bg_indexed,
            mask,
            HL_BG_INDEXED,
        );
    }
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_NOCOMBINE,
        dict.nocombine,
        mask,
        HL_NOCOMBINE,
    );
    mask = apply_flag_with_key(
        is_set,
        KEYSET_OPTIDX_HIGHLIGHT_DEFAULT,
        dict.default_,
        mask,
        HL_DEFAULT,
    );

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_FG) {
        fg = object_to_color(dict.fg, c"fg", use_rgb, err);
    } else if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_FOREGROUND) {
        fg = object_to_color(dict.foreground, c"foreground", use_rgb, err);
    }
    if err.err_type != ErrorType::ErrorTypeNone {
        return hlattrs;
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_BG) {
        bg = object_to_color(dict.bg, c"bg", use_rgb, err);
    } else if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_BACKGROUND) {
        bg = object_to_color(dict.background, c"background", use_rgb, err);
    }
    if err.err_type != ErrorType::ErrorTypeNone {
        return hlattrs;
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_SP) {
        sp = object_to_color(dict.sp, c"sp", true, err);
    } else if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_SPECIAL) {
        sp = object_to_color(dict.special, c"special", true, err);
    }
    if err.err_type != ErrorType::ErrorTypeNone {
        return hlattrs;
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_BLEND) {
        let blend0 = dict.blend;
        if !(0..=100).contains(&blend0) {
            api_err_invalid(err, c"blend", c"out of range", 0, false);
            return hlattrs;
        }
        blend = blend0 as i32;
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_LINK)
        || has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_LINK_GLOBAL)
    {
        if link_id.is_none() {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid Key: '%s'",
                if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_LINK_GLOBAL) {
                    c"link_global".as_ptr()
                } else {
                    c"link".as_ptr()
                },
            );
            return hlattrs;
        }
        if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_LINK_GLOBAL) {
            *link_id.as_deref_mut().unwrap() = dict.link_global as i32;
            mask |= HL_GLOBAL;
        } else {
            *link_id.unwrap() = dict.link as i32;
        }

        if err.err_type != ErrorType::ErrorTypeNone {
            return hlattrs;
        }
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_CTERM) {
        let mut cterm = KeyDict_highlight_cterm::default();
        if !api_dict_to_keydict(
            (&raw mut cterm).cast(),
            Some(key_dict_highlight_cterm_get_field),
            dict.cterm,
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

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_CTERMFG) {
        ctermfg = object_to_color(dict.ctermfg, c"ctermfg", false, err);
        if err.err_type != ErrorType::ErrorTypeNone {
            return hlattrs;
        }
    }

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_CTERMBG) {
        ctermbg = object_to_color(dict.ctermbg, c"ctermbg", false, err);
        if err.err_type != ErrorType::ErrorTypeNone {
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

    if has_key(is_set, KEYSET_OPTIDX_HIGHLIGHT_FONT) {
        let str_ = dict.font;
        let bytes = str_.cstr_bytes();
        font = if str_.size > 0 && !is_none_str(bytes) {
            hl_add_font_idx(Some(bytes))
        } else {
            -1
        };
    }
    hlattrs.font = font;

    hlattrs
}

pub fn object_to_color(val: Object, key: &std::ffi::CStr, rgb: bool, err: &mut Error) -> i32 {
    if val.obj_type == ObjectType::ObjectTypeInteger {
        val.data.integer() as i32
    } else if val.obj_type == ObjectType::ObjectTypeString {
        let str_ = val.data.string();
        if str_.size == 0 || is_none_str(str_.cstr_bytes()) {
            return -1;
        }
        let color = if rgb {
            let mut dummy: i32 = 0;
            name_to_color(str_.data, &raw mut dummy)
        } else {
            name_to_ctermcolor(str_.data)
        };
        if color < 0 {
            api_err_invalid(err, c"highlight color", str_.data, 0, true);
            return color;
        }
        color
    } else {
        api_err_exp(
            err,
            key.as_ptr().cast_mut(),
            c"String or Integer",
            ptr::null(),
        );
        0
    }
}

pub fn hl_inspect(attr: i32, arena: &mut Arena) -> Array {
    if !hl_st().hlstate_active {
        return Array {
            size: 0,
            capacity: 0,
            items: ptr::null_mut(),
        };
    }
    let mut ret = arena_array(arena, hl_inspect_size(attr));
    hl_inspect_impl(&mut ret, attr, arena);
    ret
}

fn hl_inspect_size(attr: i32) -> usize {
    if attr <= 0 || (attr as usize) >= hl_st().attr_entries.len() {
        return 0;
    }

    let e = hl_st().attr_entries.as_slice()[attr as usize];
    if e.kind == K_HL_COMBINE || e.kind == K_HL_BLEND || e.kind == K_HL_BLEND_THROUGH {
        return hl_inspect_size(e.id1) + hl_inspect_size(e.id2);
    }
    1
}

fn hl_inspect_impl(arr: &mut Array, attr: i32, arena: &mut Arena) {
    if attr <= 0 || (attr as usize) >= hl_st().attr_entries.len() {
        return;
    }

    let e = hl_st().attr_entries.as_slice()[attr as usize];
    let mut item: Dict;
    match e.kind {
        K_HL_SYNTAX => {
            item = arena_dict(arena, 3);
            put_c(&mut item, c"kind", cstr_as_obj(c"syntax"));
            put_c(&mut item, c"hi_name", cstr_as_obj(syn_id2name(e.id1)));
        }
        K_HL_UI => {
            item = arena_dict(arena, 4);
            put_c(&mut item, c"kind", cstr_as_obj(c"ui"));
            let ui_name = if e.id1 == -1 {
                Some(c"Normal")
            } else {
                hlf_name_ptr(e.id1 as usize)
            };
            put_c(&mut item, c"ui_name", cstr_as_obj(ui_name));
            put_c(&mut item, c"hi_name", cstr_as_obj(syn_id2name(e.id2)));
        }
        K_HL_TERMINAL => {
            item = arena_dict(arena, 2);
            put_c(&mut item, c"kind", cstr_as_obj(c"term"));
        }
        K_HL_COMBINE | K_HL_BLEND | K_HL_BLEND_THROUGH => {
            hl_inspect_impl(arr, e.id1, arena);
            hl_inspect_impl(arr, e.id2, arena);
            return;
        }
        _ => return,
    }
    put_c(&mut item, c"id", integer_obj(Integer::from(attr)));
    add_c(arr, dict_obj(item));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hl_font_interning_handles_null_and_roundtrips() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            assert_eq!(hl_add_font_idx(None), -1);
            assert_eq!(hl_add_font_idx(Some(b"")), -1);
            let idx = hl_add_font_idx(Some(b"Monospace:h12"));
            assert!(idx >= 0);
            assert_eq!(hl_add_font_idx(Some(b"Monospace:h12")), idx);
            let got = hl_get_font(idx).unwrap();
            assert_eq!(got.to_bytes(), b"Monospace:h12");
        }
    }
}
