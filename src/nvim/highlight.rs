//! Low-level code for UI and syntax highlighting (`src/nvim/highlight.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::extmark::NS;
use crate::api::private::helpers::{
    add_c, api_dict_to_keydict, api_set_error, arena_array, arena_dict, boolean_obj, cstr_as_obj,
    cstr_as_string, dict_obj, integer_obj, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_exp,
    api_err_invalid,
};
use crate::api::tabpage::win_T;
use crate::api::ui::{remote_ui_hl_attr_define, remote_ui_hl_group_set};
use crate::decoration_provider::get_decor_provider;
use crate::drawscreen::screen_invalidate_highlights;
use crate::globals::*;
use crate::highlight_group::{
    highlight_attr_set_all, highlight_changed, name_to_color, name_to_ctermcolor, set_hl_group,
    syn_check_group, syn_id2name, syn_ns_id2attr,
};
use crate::lua::converter::{Boolean, LuaRef};
use crate::lua::executor::nlua_call_ref;
use crate::map::{
    ColorItem, ColorKey, HlAttrs, HlEntry, Map, Set, cstr_t, map_put_ref_ColorKey_ColorItem,
    map_put_ref_int_ptr_t, map_put_ref_uint64_t_int, map_ref_ColorKey_ColorItem, map_ref_int_ptr_t,
    map_ref_uint64_t_int, mh_clear, mh_get_ColorKey, mh_put_HlEntry, mh_put_cstr_t, ptr_t,
};
use crate::map_glyph_cache::{MHPutStatus, MapHash, NvimString};
use crate::memory::{Arena, arena_finish, arena_mem_free, xfree, xmalloc, xstrdup};
use crate::message::emsg;
use crate::msgpack_rpc::packer::Integer;
use crate::option::check_blending;
use crate::option_vars::*;
use crate::popupmenu::pum_drawn;
use crate::strings::gettext;
use crate::ui::ui_call_hl_attr_define;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub type RgbValue = i32;
pub type OptionalKeys = u64;
pub use crate::api::extmark::HLGroupID;
pub use crate::msgpack_rpc::channel::RemoteUI;
pub type HlKind = c_int;

const LUA_NOREF: LuaRef = -2;
const kRetObject: c_int = 0;
const MAX_TYPENR: u32 = 65535;
const HLATTRS_DICT_SIZE: usize = 24;

const HLF_NONE: usize = 0;
const HLF_PNI: c_int = 41;
const HLF_PST: c_int = 50;
const HLF_INACTIVE: usize = 60;
const HLF_NFLOAT: usize = 62;
const HLF_BORDER: usize = 64;
const HLF_COUNT: usize = 76;

const kHlUnknown: HlKind = 0;
const kHlUI: HlKind = 1;
const kHlSyntax: HlKind = 2;
const kHlTerminal: HlKind = 3;
const kHlCombine: HlKind = 4;
const kHlBlend: HlKind = 5;
const kHlBlendThrough: HlKind = 6;
const kHlInvalid: HlKind = 7;

const HL_INVERSE: i32 = 0x01;
const HL_BOLD: i32 = 0x02;
const HL_ITALIC: i32 = 0x04;
const HL_UNDERLINE_MASK: i32 = 0x38;
const HL_UNDERLINE: i32 = 0x08;
const HL_UNDERCURL: i32 = 0x10;
const HL_UNDERDOUBLE: i32 = 0x18;
const HL_UNDERDOTTED: i32 = 0x20;
const HL_UNDERDASHED: i32 = 0x28;
const HL_STANDOUT: i32 = 0x0040;
const HL_STRIKETHROUGH: i32 = 0x0080;
const HL_ALTFONT: i32 = 0x0100;
const HL_DIM: i32 = 0x0200;
const HL_NOCOMBINE: i32 = 0x0400;
const HL_BG_INDEXED: i32 = 0x0800;
const HL_FG_INDEXED: i32 = 0x1000;
const HL_DEFAULT: i32 = 0x2000;
const HL_GLOBAL: i32 = 0x4000;
const HL_BLINK: i32 = 0x8000;
const HL_CONCEALED: i32 = 0x10000;
const HL_OVERLINE: i32 = 0x20000;

const KEYSET_OPTIDX_highlight__bg: c_int = 1;
const KEYSET_OPTIDX_highlight__fg: c_int = 2;
const KEYSET_OPTIDX_highlight__sp: c_int = 3;
const KEYSET_OPTIDX_highlight__dim: c_int = 4;
const KEYSET_OPTIDX_highlight__bold: c_int = 6;
const KEYSET_OPTIDX_highlight__font: c_int = 7;
const KEYSET_OPTIDX_highlight__link: c_int = 8;
const KEYSET_OPTIDX_highlight__blend: c_int = 9;
const KEYSET_OPTIDX_highlight__blink: c_int = 11;
const KEYSET_OPTIDX_highlight__cterm: c_int = 12;
const KEYSET_OPTIDX_highlight__italic: c_int = 13;
const KEYSET_OPTIDX_highlight__reverse: c_int = 15;
const KEYSET_OPTIDX_highlight__default: c_int = 16;
const KEYSET_OPTIDX_highlight__altfont: c_int = 17;
const KEYSET_OPTIDX_highlight__conceal: c_int = 18;
const KEYSET_OPTIDX_highlight__special: c_int = 19;
const KEYSET_OPTIDX_highlight__ctermfg: c_int = 20;
const KEYSET_OPTIDX_highlight__ctermbg: c_int = 21;
const KEYSET_OPTIDX_highlight__fallback: c_int = 22;
const KEYSET_OPTIDX_highlight__overline: c_int = 23;
const KEYSET_OPTIDX_highlight__standout: c_int = 24;
const KEYSET_OPTIDX_highlight__nocombine: c_int = 25;
const KEYSET_OPTIDX_highlight__undercurl: c_int = 26;
const KEYSET_OPTIDX_highlight__underline: c_int = 27;
const KEYSET_OPTIDX_highlight__background: c_int = 28;
const KEYSET_OPTIDX_highlight__bg_indexed: c_int = 29;
const KEYSET_OPTIDX_highlight__foreground: c_int = 30;
const KEYSET_OPTIDX_highlight__fg_indexed: c_int = 31;
const KEYSET_OPTIDX_highlight__link_global: c_int = 32;
const KEYSET_OPTIDX_highlight__underdashed: c_int = 33;
const KEYSET_OPTIDX_highlight__underdotted: c_int = 34;
const KEYSET_OPTIDX_highlight__underdouble: c_int = 35;
const KEYSET_OPTIDX_highlight__strikethrough: c_int = 36;

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

type NSHlAttr = [c_int; HLF_COUNT];

const fn empty_map_hash() -> MapHash {
    MapHash {
        n_buckets: 0,
        size: 0,
        n_occupied: 0,
        upper_bound: 0,
        n_keys: 0,
        keys_capacity: 0,
        hash: ptr::null_mut(),
    }
}

static mut hlstate_active: bool = false;
static mut attr_entries: Set<HlEntry> = Set {
    h: empty_map_hash(),
    keys: ptr::null_mut(),
};
static mut combine_attr_entries: Map<u64, c_int> = Map {
    set: Set {
        h: empty_map_hash(),
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};
static mut blend_attr_entries: Map<u64, c_int> = Map {
    set: Set {
        h: empty_map_hash(),
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};
static mut blendthrough_attr_entries: Map<u64, c_int> = Map {
    set: Set {
        h: empty_map_hash(),
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};
static mut urls: Set<cstr_t> = Set {
    h: empty_map_hash(),
    keys: ptr::null_mut(),
};
static mut fonts: Set<cstr_t> = Set {
    h: empty_map_hash(),
    keys: ptr::null_mut(),
};
static mut ns_hls: Map<ColorKey, ColorItem> = Map {
    set: Set {
        h: empty_map_hash(),
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};
static mut ns_hl_attr: Map<c_int, ptr_t> = Map {
    set: Set {
        h: empty_map_hash(),
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

static mut get_attr_entry_recursive: bool = false;
static mut ns_get_hl_recursive: c_int = 0;

#[inline]
unsafe fn is_none_str(s: *const c_char) -> bool {
    !s.is_null() && unsafe { CStr::from_ptr(s).to_bytes().eq_ignore_ascii_case(b"NONE") }
}

use crate::api::private::dispatch::{
    KeyDict_highlight_cterm_get_field, KeyDict_highlight_get_field,
};

#[inline]
const fn has_key(mask: OptionalKeys, idx: c_int) -> bool {
    (mask & (1u64 << idx)) != 0
}

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
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    debug_assert!(dict.size < dict.capacity);
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: NvimString {
                data: key.as_ptr().cast_mut(),
                size: key.to_bytes().len(),
            },
            value,
        };
    }
    dict.size += 1;
}

#[inline]
unsafe fn map_get_u64_int(map: *mut Map<u64, c_int>, key: u64) -> c_int {
    unsafe {
        let ptr = map_ref_uint64_t_int(map, key, ptr::null_mut());
        if ptr.is_null() { 0 } else { *ptr }
    }
}

#[inline]
unsafe fn map_put_u64_int(map: *mut Map<u64, c_int>, key: u64, val: c_int) {
    unsafe {
        let ptr = map_put_ref_uint64_t_int(map, key, ptr::null_mut(), ptr::null_mut());
        *ptr = val;
    }
}

#[inline]
unsafe fn map_get_color(map: *mut Map<ColorKey, ColorItem>, key: ColorKey) -> ColorItem {
    unsafe {
        let ptr = map_ref_ColorKey_ColorItem(map, key, ptr::null_mut());
        if ptr.is_null() {
            ColorItem {
                attr_id: -1,
                link_id: -1,
                version: -1,
                is_default: false,
                link_global: false,
            }
        } else {
            *ptr
        }
    }
}

#[inline]
unsafe fn map_put_color(map: *mut Map<ColorKey, ColorItem>, key: ColorKey, val: ColorItem) {
    unsafe {
        let ptr = map_put_ref_ColorKey_ColorItem(map, key, ptr::null_mut(), ptr::null_mut());
        *ptr = val;
    }
}

#[inline]
unsafe fn pmap_get_int(map: *mut Map<c_int, ptr_t>, key: c_int) -> ptr_t {
    unsafe {
        let ptr = map_ref_int_ptr_t(map, key, ptr::null_mut());
        if ptr.is_null() { ptr::null_mut() } else { *ptr }
    }
}

#[inline]
unsafe fn set_destroy<K>(set: *mut Set<K>) {
    unsafe {
        xfree((*set).keys.cast());
        xfree((*set).h.hash.cast());
        *set = Set {
            h: empty_map_hash(),
            keys: ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn map_destroy<K, V>(map: *mut Map<K, V>) {
    unsafe {
        set_destroy(&raw mut (*map).set);
        xfree((*map).values.cast());
        (*map).values = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn highlight_init() {
    unsafe {
        let mut status = MHPutStatus::kMHExisting;
        mh_put_HlEntry(
            &raw mut attr_entries,
            HlEntry {
                attr: HLATTRS_INIT,
                kind: kHlInvalid,
                id1: 0,
                id2: 0,
                winid: 0,
            },
            &raw mut status,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn highlight_use_hlstate() -> bool {
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
            let mut status = MHPutStatus::kMHExisting;
            let k = mh_put_HlEntry(&raw mut attr_entries, entry, &raw mut status);
            if status == MHPutStatus::kMHExisting {
                return k as c_int;
            }

            if attr_entries.h.size > MAX_TYPENR {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ui_send_all_hls(ui: *mut RemoteUI) {
    unsafe {
        for i in 1..(attr_entries.h.size as usize) {
            let mut arena = Arena {
                cur_blk: ptr::null_mut(),
                pos: 0,
                size: 0,
            };
            let inspect = hl_inspect(i as c_int, &raw mut arena);
            let attr = (*attr_entries.keys.add(i)).attr;
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_syn_attr(ns_id: c_int, idx: c_int, at_en: HlAttrs) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ns_hl_def(
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
        if (attrs.rgb_ae_attr & HL_DEFAULT) != 0
            && mh_get_ColorKey(&raw mut ns_hls.set, key) != u32::MAX
        {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ns_get_hl(
    ns_hl: *mut NS,
    hl_id: c_int,
    link: bool,
    nodefault: bool,
) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_check_ns() -> bool {
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
            let hl_def = pmap_get_int(&raw mut ns_hl_attr, ns).cast::<NSHlAttr>();
            if !hl_def.is_null() {
                hl_attr_active = (*hl_def).as_mut_ptr();
            }
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_check_ns_hl(wp: *mut win_T) -> bool {
    unsafe {
        ns_hl_win = if !wp.is_null() { (*wp).w_ns_hl } else { -1 };
        hl_check_ns()
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_ns_get_attrs(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_ui_attr(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_apply_winblend(winbl: c_int, mut attr: c_int) -> c_int {
    unsafe {
        let mut entry = *attr_entries.keys.add(attr as usize);
        if entry.attr.hl_blend == -1 && winbl > 0 {
            entry.attr.hl_blend = winbl;
            attr = get_attr_entry(entry);
        }
        attr
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_window_hl(wp: *mut win_T, invalid: bool) {
    unsafe {
        let ns_id = (*wp).w_ns_hl;

        update_ns_hl(ns_id);
        if ns_id != (*wp).w_ns_hl_active {
            (*wp).w_ns_hl_active = ns_id;

            let hl_def_ptr = pmap_get_int(&raw mut ns_hl_attr, ns_id).cast::<NSHlAttr>();
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_ns_hl(ns_id: c_int) {
    unsafe {
        if ns_id <= 0 {
            return;
        }
        let mut p = get_decor_provider(ns_id, true);
        if (*p).hl_cached {
            return;
        }

        let alloc =
            map_put_ref_int_ptr_t(&raw mut ns_hl_attr, ns_id, ptr::null_mut(), ptr::null_mut())
                .cast::<*mut NSHlAttr>();
        if (*alloc).is_null() {
            *alloc = xmalloc(size_of::<NSHlAttr>()).cast();
        }
        let hl_attrs = (*(*alloc)).as_mut_ptr();

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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn win_bg_attr(wp: *mut win_T) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_underline() -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_add_url(attr: c_int, url: *const c_char) -> c_int {
    unsafe {
        let mut attrs = HLATTRS_INIT;
        let mut status = MHPutStatus::kMHExisting;
        let k = mh_put_cstr_t(&raw mut urls, url, &raw mut status);
        if status != MHPutStatus::kMHExisting {
            *urls.keys.add(k as usize) = xstrdup(url);
        }
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_url(index: u32) -> *const c_char {
    unsafe {
        assert!(!urls.keys.is_null());
        *urls.keys.add(index as usize)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_add_font_idx(font_name: *const c_char) -> i32 {
    unsafe {
        if font_name.is_null() || *font_name == 0 {
            return -1;
        }
        let mut status = MHPutStatus::kMHExisting;
        let k = mh_put_cstr_t(&raw mut fonts, font_name, &raw mut status);
        if status != MHPutStatus::kMHExisting {
            *fonts.keys.add(k as usize) = xstrdup(font_name);
        }
        k as i32
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_font(index: i32) -> *const c_char {
    unsafe {
        if index < 0 || fonts.keys.is_null() {
            return ptr::null();
        }
        *fonts.keys.add(index as usize)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_term_attr(aep: *mut HlAttrs) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_hl_tables(reinit: bool) {
    unsafe {
        for i in 0..urls.h.n_keys {
            xfree((*urls.keys.add(i as usize)).cast_mut().cast());
        }

        if reinit {
            mh_clear(&raw mut attr_entries.h);
            highlight_init();
            mh_clear(&raw mut combine_attr_entries.set.h);
            mh_clear(&raw mut blend_attr_entries.set.h);
            mh_clear(&raw mut blendthrough_attr_entries.set.h);
            mh_clear(&raw mut urls.h);
            ptr::write_bytes(
                (&raw mut highlight_attr_last).cast::<u8>(),
                0xFF,
                size_of::<[c_int; HLF_COUNT]>(),
            );
            highlight_attr_set_all();
            highlight_changed();
            screen_invalidate_highlights();
        } else {
            for i in 0..fonts.h.n_keys {
                xfree((*fonts.keys.add(i as usize)).cast_mut().cast());
            }
            set_destroy(&raw mut attr_entries);
            map_destroy(&raw mut combine_attr_entries);
            map_destroy(&raw mut blend_attr_entries);
            map_destroy(&raw mut blendthrough_attr_entries);
            map_destroy(&raw mut ns_hls);
            set_destroy(&raw mut urls);
            set_destroy(&raw mut fonts);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_invalidate_blends() {
    unsafe {
        mh_clear(&raw mut blend_attr_entries.set.h);
        mh_clear(&raw mut blendthrough_attr_entries.set.h);
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_combine_attr(char_attr: c_int, prim_attr: c_int) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_blend_attrs(
    back_attr: c_int,
    front_attr: c_int,
    through: *mut bool,
) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn syn_attr2entry(attr: c_int) -> HlAttrs {
    unsafe {
        if attr <= 0 || attr >= attr_entries.h.size as c_int {
            return HLATTRS_INIT;
        }
        (*attr_entries.keys.add(attr as usize)).attr
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_get_attr_by_id(
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

        if attr_id < 0 || attr_id >= Integer::from(attr_entries.h.size) {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hlattrs2dict(
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
const fn apply_flag_with_key(is_set: u64, opt_idx: c_int, val: bool, mut m: i32, flag: i32) -> i32 {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn dict2hlattrs(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn object_to_color(
    val: Object,
    key: *mut c_char,
    rgb: bool,
    err: *mut Error,
) -> c_int {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn hl_inspect(attr: c_int, arena: *mut Arena) -> Array {
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
        if attr <= 0 || attr >= attr_entries.h.size as c_int {
            return 0;
        }

        let e = *attr_entries.keys.add(attr as usize);
        if e.kind == kHlCombine || e.kind == kHlBlend || e.kind == kHlBlendThrough {
            return hl_inspect_size(e.id1) + hl_inspect_size(e.id2);
        }
        1
    }
}

unsafe fn hl_inspect_impl(arr: *mut Array, attr: c_int, arena: *mut Arena) {
    unsafe {
        if attr <= 0 || attr >= attr_entries.h.size as c_int {
            return;
        }

        let e = *attr_entries.keys.add(attr as usize);
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
