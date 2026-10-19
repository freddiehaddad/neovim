//! Extmark and namespace API functions (`src/nvim/api/extmark.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_typename;
#[allow(unused_imports)]
use crate::api::private::helpers::{
    arena_array, arena_dict, copy_string, find_buffer_by_handle, find_window_by_handle,
    object_to_hl_id,
};
#[allow(unused_imports)]
use crate::charset::{transstr, vim_isprintc};
#[allow(unused_imports)]
use crate::decoration::{
    clear_virtlines, clear_virttext, decor_free, decor_put_sh, decor_put_vt, decor_range_add_sh,
    decor_range_add_virt, decor_sh_from_inline, decor_to_dict_legacy, hl_group_name,
};
#[allow(unused_imports)]
use crate::decoration_provider::{decor_provider_clear, get_decor_provider};
#[allow(unused_imports)]
use crate::drawscreen::redraw_all_later;
#[allow(unused_imports)]
use crate::extmark::{extmark_clear, extmark_del_id, extmark_from_id, extmark_get, extmark_set};
use crate::globals::*;
#[allow(unused_imports)]
use crate::grid::schar_high;
#[allow(unused_imports)]
use crate::marktree::mt_inspect;
#[allow(unused_imports)]
use crate::mbyte::mb_string2cells;
#[allow(unused_imports)]
use crate::mbyte::utfc_ptr2schar;
#[allow(unused_imports)]
use crate::memline::ml_get_buf_len;
#[allow(unused_imports)]
use crate::r#move::changed_window_setting;
#[allow(unused_imports)]
use crate::sign::init_sign_text;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::api::autocmd::OptionalKeys;
use crate::api::deprecated::{
    DecorInline, DecorVirtText, DecorVirtTextData, VirtLine, VirtLineOverflow, VirtLines, VirtText,
    VirtTextChunk, VirtTextPos,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType, api_err_exp,
    api_err_invalid,
};
use crate::api::tabpage::{Buffer, Window, win_T};
use crate::eval::deprecated::buf_T;
use crate::kvec::KVec;
use crate::lua::converter::{Boolean, LuaRef};
use crate::map::{
    Set, map_put_ref_String_int, mh_delete_uint32_t, mh_get_String, mh_get_ptr_t, mh_get_uint32_t,
    mh_put_ptr_t, mh_put_uint32_t, ptr_t,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, MHPutStatus, MapHash, NvimString};
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DECOR_PRIORITY_BASE, DecorExt, DecorInlineData,
    MT_FLAG_DECOR_CONCEAL_LINES, MT_FLAG_DECOR_EXT, MT_FLAG_DECOR_HL, MT_FLAG_DECOR_SIGNHL,
    MT_FLAG_DECOR_SIGNTEXT, MT_FLAG_DECOR_VIRT_LINES, MT_FLAG_DECOR_VIRT_TEXT_INLINE,
    MT_FLAG_INVALID, MT_FLAG_INVALIDATE, MT_FLAG_NO_UNDO, MT_FLAG_PAIRED, MT_FLAG_RIGHT_GRAVITY,
    MTKey, MTPair, MarkTreeIter,
};
use crate::memory::{Arena, xfree, xmemdupz};
use crate::msgpack_rpc::packer::{Integer, handle_T};

pub type NS = handle_T;
pub type HLGroupID = Integer;
pub type colnr_T = c_int;
pub type linenr_T = i32;
pub type DecorPriority = u16;
pub type ExtmarkType = c_int;
pub type ExtmarkInfoArray = KVec<MTPair>;

const NUL: c_char = 0;
const MAXLNUM: i64 = 0x7fffffff;
const MAXCOL: i64 = 0x7fffffff;
const DECOR_ID_INVALID: u32 = u32::MAX;
const LUA_NOREF: LuaRef = -2;
const UPD_NOT_VALID: c_int = 40;
const SET_EXTMARK_TABLE_SIZE: usize = 36;

const kExtmarkNone: ExtmarkType = 0x1;
const kExtmarkSign: ExtmarkType = 0x2;
const kExtmarkVirtText: ExtmarkType = 0x8;
const kExtmarkVirtLines: ExtmarkType = 0x10;
const kExtmarkHighlight: ExtmarkType = 0x20;

const kVPosEndOfLine: VirtTextPos = 0;
const kVPosEndOfLineRightAlign: VirtTextPos = 1;
const kVPosInline: VirtTextPos = 2;
const kVPosOverlay: VirtTextPos = 3;
const kVPosRightAlign: VirtTextPos = 4;
const kVPosWinCol: VirtTextPos = 5;

const kVLLeftcol: c_int = 1;

const kVLOverflowTrunc: VirtLineOverflow = 0;
const kVLOverflowScroll: VirtLineOverflow = 1;
const kVLOverflowWrap: VirtLineOverflow = 2;
const kVLOverflowAuto: VirtLineOverflow = 3;

const kHlModeUnknown: u8 = 0;
const kHlModeReplace: u8 = 1;
const kHlModeCombine: u8 = 2;
const kHlModeBlend: u8 = 3;

const kSHIsSign: u16 = 1;
const kSHHlEol: u16 = 2;
const kSHUIWatched: u16 = 4;
const kSHUIWatchedOverlay: u16 = 8;
const kSHSpellOn: u16 = 16;
const kSHSpellOff: u16 = 32;
const kSHConceal: u16 = 64;
const kSHConcealLines: u16 = 128;
const kSHConcealOff: u16 = 256;

const kVTIsLines: u8 = 1;
const kVTHide: u8 = 2;
const kVTLinesAbove: u8 = 4;
const kVTRepeatLinebreak: u8 = 8;

const kDecorProviderActive: c_int = 1;

const KEYSET_OPTIDX_get_extmark__details: u64 = 1;
const KEYSET_OPTIDX_get_extmark__hl_name: u64 = 2;

const KEYSET_OPTIDX_get_extmarks__type: u64 = 1;
const KEYSET_OPTIDX_get_extmarks__limit: u64 = 2;
const KEYSET_OPTIDX_get_extmarks__details: u64 = 3;
const KEYSET_OPTIDX_get_extmarks__hl_name: u64 = 4;
const KEYSET_OPTIDX_get_extmarks__overlap: u64 = 5;

const KEYSET_OPTIDX_set_extmark__id: u64 = 1;
const KEYSET_OPTIDX_set_extmark__url: u64 = 2;
const KEYSET_OPTIDX_set_extmark__spell: u64 = 3;
const KEYSET_OPTIDX_set_extmark__scoped: u64 = 4;
const KEYSET_OPTIDX_set_extmark__hl_eol: u64 = 5;
const KEYSET_OPTIDX_set_extmark__strict: u64 = 6;
const KEYSET_OPTIDX_set_extmark__end_col: u64 = 7;
const KEYSET_OPTIDX_set_extmark__conceal: u64 = 8;
const KEYSET_OPTIDX_set_extmark__hl_mode: u64 = 9;
const KEYSET_OPTIDX_set_extmark__end_row: u64 = 10;
const KEYSET_OPTIDX_set_extmark__end_line: u64 = 11;
const KEYSET_OPTIDX_set_extmark__hl_group: u64 = 12;
const KEYSET_OPTIDX_set_extmark__priority: u64 = 13;
const KEYSET_OPTIDX_set_extmark__ephemeral: u64 = 14;
const KEYSET_OPTIDX_set_extmark__sign_text: u64 = 15;
const KEYSET_OPTIDX_set_extmark__virt_text: u64 = 16;
const KEYSET_OPTIDX_set_extmark__invalidate: u64 = 17;
const KEYSET_OPTIDX_set_extmark__ui_watched: u64 = 18;
const KEYSET_OPTIDX_set_extmark__virt_lines: u64 = 19;
const KEYSET_OPTIDX_set_extmark___subpriority: u64 = 20;
const KEYSET_OPTIDX_set_extmark__undo_restore: u64 = 21;
const KEYSET_OPTIDX_set_extmark__conceal_lines: u64 = 22;
const KEYSET_OPTIDX_set_extmark__line_hl_group: u64 = 23;
const KEYSET_OPTIDX_set_extmark__right_gravity: u64 = 24;
const KEYSET_OPTIDX_set_extmark__sign_hl_group: u64 = 25;
const KEYSET_OPTIDX_set_extmark__virt_text_pos: u64 = 26;
const KEYSET_OPTIDX_set_extmark__virt_text_hide: u64 = 27;
const KEYSET_OPTIDX_set_extmark__number_hl_group: u64 = 28;
const KEYSET_OPTIDX_set_extmark__virt_lines_above: u64 = 29;
const KEYSET_OPTIDX_set_extmark__end_right_gravity: u64 = 30;
const KEYSET_OPTIDX_set_extmark__virt_text_win_col: u64 = 31;
const KEYSET_OPTIDX_set_extmark__virt_lines_leftcol: u64 = 32;
const KEYSET_OPTIDX_set_extmark__cursorline_hl_group: u64 = 33;
const KEYSET_OPTIDX_set_extmark__virt_lines_overflow: u64 = 34;
const KEYSET_OPTIDX_set_extmark__virt_text_repeat_linebreak: u64 = 35;

const KEYSET_OPTIDX_ns_opts__wins: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorSignHighlight {
    pub flags: u16,
    pub priority: DecorPriority,
    pub hl_id: c_int,
    pub text: [u32; 2],
    pub sign_name: *mut c_char,
    pub sign_add_id: c_int,
    pub number_hl_id: c_int,
    pub line_hl_id: c_int,
    pub cursorline_hl_id: c_int,
    pub next: u32,
    pub url: *const c_char,
}

const DECOR_SIGN_HIGHLIGHT_INIT: DecorSignHighlight = DecorSignHighlight {
    flags: 0,
    priority: DECOR_PRIORITY_BASE,
    hl_id: 0,
    text: [0, 0],
    sign_name: ptr::null_mut(),
    sign_add_id: 0,
    number_hl_id: 0,
    line_hl_id: 0,
    cursorline_hl_id: 0,
    next: DECOR_ID_INVALID,
    url: ptr::null(),
};

#[repr(C)]
pub struct DecorProvider {
    pub ns_id: NS,
    pub state: c_int,
    pub win_skip_row: c_int,
    pub win_skip_col: c_int,
    pub redraw_start: LuaRef,
    pub redraw_buf: LuaRef,
    pub redraw_win: LuaRef,
    pub redraw_line: LuaRef,
    pub redraw_range: LuaRef,
    pub redraw_end: LuaRef,
    pub hl_def: LuaRef,
    pub spell_nav: LuaRef,
    pub conceal_line: LuaRef,
    pub hl_valid: c_int,
    pub hl_cached: bool,
    pub error_count: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorRangeUI {
    pub ns_id: u32,
    pub mark_id: u32,
    pub pos: VirtTextPos,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DecorRangeData {
    pub sh: DecorSignHighlight,
    pub vt: *mut DecorVirtText,
    pub ui: DecorRangeUI,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorRange {
    pub start_row: c_int,
    pub start_col: c_int,
    pub end_row: c_int,
    pub end_col: c_int,
    pub ordering: c_int,
    pub priority_internal: u32,
    pub owned: bool,
    pub kind: u8,
    pub data: DecorRangeData,
    pub attr_id: c_int,
    pub draw_col: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DecorRangeSlot {
    pub range: DecorRange,
    pub next_free_i: c_int,
}

#[repr(C)]
pub struct DecorState {
    pub itr: [MarkTreeIter; 1],
    pub slots: KVec<DecorRangeSlot>,
    pub ranges_i: KVec<c_int>,
    pub current_end: c_int,
    pub future_begin: c_int,
    pub free_slot_i: c_int,
    pub new_range_ordering: c_int,
    pub win: *mut win_T,
    pub top_row: c_int,
    pub row: c_int,
    pub col_last: c_int,
    pub current: c_int,
    pub current_hl_eol: c_int,
    pub eol_col: c_int,
    pub conceal: c_int,
    pub conceal_char: u32,
    pub conceal_attr: c_int,
    pub spell: c_int,
    pub running_decor_provider: bool,
    pub itr_valid: bool,
}

#[repr(C)]
pub struct KeyDict_get_extmark {
    pub is_set__get_extmark_: OptionalKeys,
    pub details: Boolean,
    pub hl_name: Boolean,
}

#[repr(C)]
pub struct KeyDict_get_extmarks {
    pub is_set__get_extmarks_: OptionalKeys,
    pub limit: Integer,
    pub details: Boolean,
    pub hl_name: Boolean,
    pub overlap: Boolean,
    pub type_: NvimString,
}

#[repr(C)]
pub struct KeyDict_set_extmark {
    pub is_set__set_extmark_: OptionalKeys,
    pub id: Integer,
    pub end_line: Integer,
    pub end_row: Integer,
    pub end_col: Integer,
    pub hl_group: Object,
    pub virt_text: Array,
    pub virt_text_pos: NvimString,
    pub virt_text_win_col: Integer,
    pub virt_text_hide: Boolean,
    pub virt_text_repeat_linebreak: Boolean,
    pub hl_eol: Boolean,
    pub hl_mode: NvimString,
    pub invalidate: Boolean,
    pub ephemeral: Boolean,
    pub priority: Integer,
    pub right_gravity: Boolean,
    pub end_right_gravity: Boolean,
    pub virt_lines: Array,
    pub virt_lines_above: Boolean,
    pub virt_lines_leftcol: Boolean,
    pub virt_lines_overflow: NvimString,
    pub strict: Boolean,
    pub sign_text: NvimString,
    pub sign_hl_group: HLGroupID,
    pub number_hl_group: HLGroupID,
    pub line_hl_group: HLGroupID,
    pub cursorline_hl_group: HLGroupID,
    pub conceal: Object,
    pub conceal_lines: NvimString,
    pub spell: Boolean,
    pub ui_watched: Boolean,
    pub undo_restore: Boolean,
    pub url: NvimString,
    pub scoped: Boolean,
    pub _subpriority: Integer,
}

#[repr(C)]
pub struct KeyDict_set_decoration_provider {
    pub is_set__set_decoration_provider_: OptionalKeys,
    pub on_start: LuaRef,
    pub on_buf: LuaRef,
    pub on_win: LuaRef,
    pub on_line: LuaRef,
    pub on_range: LuaRef,
    pub on_end: LuaRef,
    pub _on_hl_def: LuaRef,
    pub _on_spell_nav: LuaRef,
    pub _on_conceal_line: LuaRef,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_ns_opts {
    pub is_set__ns_opts_: OptionalKeys,
    pub wins: Array,
}

unsafe extern "C" {

    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
}

#[inline]
const fn boolean_obj(b: Boolean) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: b },
    }
}

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
        NvimString {
            data: s as *mut c_char,
            size: len,
        }
    }
}

#[inline]
const fn cstr_literal(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
unsafe fn string_to_cstr(str: NvimString) -> *mut c_char {
    unsafe { xmemdupz(str.data.cast(), str.size).cast() }
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn strequal(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

#[inline]
unsafe fn api_err_range(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
    }
}

#[inline]
unsafe fn add_c(arr: &mut Array, item: Object) {
    unsafe {
        *arr.items.add(arr.size) = item;
        arr.size += 1;
    }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_literal(key),
            value,
        };
        dict.size += 1;
    }
}

#[inline]
unsafe fn set_destroy<K>(set: *mut Set<K>) {
    unsafe {
        xfree((*set).keys.cast());
        xfree((*set).h.hash.cast());
        *set = Set {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        };
    }
}

#[inline]
unsafe fn set_has_u32(set: *mut Set<u32>, key: u32) -> bool {
    unsafe { mh_get_uint32_t(set, key) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_u32(set: *mut Set<u32>, key: u32) {
    unsafe {
        let mut status = MHPutStatus::kMHExisting;
        let _ = mh_put_uint32_t(set, key, &mut status);
    }
}

#[inline]
unsafe fn set_del_u32(set: *mut Set<u32>, mut key: u32) {
    unsafe {
        let _ = mh_delete_uint32_t(set, &mut key);
    }
}

#[inline]
unsafe fn set_has_ptr(set: *mut Set<ptr_t>, key: ptr_t) -> bool {
    unsafe { mh_get_ptr_t(set, key) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_ptr(set: *mut Set<ptr_t>, key: ptr_t) {
    unsafe {
        let mut status = MHPutStatus::kMHExisting;
        let _ = mh_put_ptr_t(set, key, &mut status);
    }
}

#[inline]
pub unsafe fn ns_in_win(ns_id: u32, wp: *mut win_T) -> bool {
    unsafe {
        if !set_has_u32(&raw mut namespace_localscope, ns_id) {
            return true;
        }
        set_has_u32(&raw mut (*wp).w_ns_set, ns_id)
    }
}

#[inline]
const fn mt_right(key: MTKey) -> bool {
    (key.flags & MT_FLAG_RIGHT_GRAVITY) != 0
}

#[inline]
const fn mt_paired(key: MTKey) -> bool {
    (key.flags & MT_FLAG_PAIRED) != 0
}

#[inline]
const fn mt_no_undo(key: MTKey) -> bool {
    (key.flags & MT_FLAG_NO_UNDO) != 0
}

#[inline]
const fn mt_invalidate(key: MTKey) -> bool {
    (key.flags & MT_FLAG_INVALIDATE) != 0
}

#[inline]
const fn mt_invalid(key: MTKey) -> bool {
    (key.flags & MT_FLAG_INVALID) != 0
}

#[inline]
const fn mt_decor(key: MTKey) -> DecorInline {
    DecorInline {
        ext: (key.flags & MT_FLAG_DECOR_EXT) != 0,
        data: key.decor_data,
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_extmark_free_all_mem() {
    unsafe {
        for i in 0..namespace_ids.set.h.n_keys {
            let name = *namespace_ids.set.keys.add(i as usize);
            xfree(name.data.cast());
        }
        set_destroy(&raw mut namespace_ids.set);
        xfree(namespace_ids.values.cast());
        namespace_ids.values = ptr::null_mut();
        set_destroy(&raw mut namespace_localscope);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_create_namespace(name: NvimString) -> Integer {
    unsafe {
        let k = mh_get_String(&raw mut namespace_ids.set, name);
        let mut id = if k == MH_TOMBSTONE {
            0
        } else {
            *namespace_ids.values.add(k as usize)
        };
        if id > 0 {
            return i64::from(id);
        }
        id = next_namespace_id;
        next_namespace_id += 1;
        if name.size > 0 {
            let name_alloc = copy_string(name, ptr::null_mut());
            let val = map_put_ref_String_int(
                &raw mut namespace_ids,
                name_alloc,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            *val = id;
        }
        i64::from(id)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_namespaces(arena: *mut Arena) -> Dict {
    unsafe {
        let mut retval = arena_dict(arena, namespace_ids.set.h.size as usize);
        for i in 0..namespace_ids.set.h.n_keys {
            let name = *namespace_ids.set.keys.add(i as usize);
            let id = *namespace_ids.values.add(i as usize);
            *retval.items.add(retval.size) = KeyValuePair {
                key: cstr_as_string(name.data),
                value: integer_obj(i64::from(id)),
            };
            retval.size += 1;
        }
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn describe_ns(ns_id: NS, unknown: *const c_char) -> *const c_char {
    unsafe {
        for i in 0..namespace_ids.set.h.n_keys {
            let name = *namespace_ids.set.keys.add(i as usize);
            let id = *namespace_ids.values.add(i as usize);
            if id == ns_id && name.size > 0 {
                return name.data;
            }
        }
        unknown
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ns_initialized(ns: u32) -> bool {
    if ns < 1 {
        return false;
    }
    ns < unsafe { next_namespace_id as u32 }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn virt_text_to_array(
    vt: VirtText,
    hl_name: bool,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut chunks = arena_array(arena, vt.size);
        let mut i = 0usize;
        while i < vt.size {
            let mut j = i;
            while j < vt.size {
                if !(*vt.items.add(j)).text.is_null() {
                    break;
                }
                j += 1;
            }

            let mut hl_array = arena_array(arena, if i < j { j - i + 1 } else { 0 });
            while i < j {
                let hl_id = (*vt.items.add(i)).hl_id;
                if hl_id >= 0 {
                    add_c(&mut hl_array, hl_group_name(hl_id, hl_name));
                }
                i += 1;
            }

            let text = (*vt.items.add(i)).text;
            let hl_id = (*vt.items.add(i)).hl_id;
            let mut chunk = arena_array(arena, 2);
            add_c(&mut chunk, string_obj(cstr_as_string(text)));
            if hl_array.size > 0 {
                if hl_id >= 0 {
                    add_c(&mut hl_array, hl_group_name(hl_id, hl_name));
                }
                add_c(&mut chunk, array_obj(hl_array));
            } else if hl_id >= 0 {
                add_c(&mut chunk, hl_group_name(hl_id, hl_name));
            }
            add_c(&mut chunks, array_obj(chunk));
            i += 1;
        }
        chunks
    }
}

unsafe fn extmark_to_array(
    extmark: MTPair,
    id: bool,
    add_dict: bool,
    hl_name: bool,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let start = extmark.start;
        let mut rv = arena_array(arena, 4);
        if id {
            add_c(&mut rv, integer_obj(i64::from(start.id)));
        }
        add_c(&mut rv, integer_obj(i64::from(start.pos.row)));
        add_c(&mut rv, integer_obj(i64::from(start.pos.col)));

        if add_dict {
            let mut dict = arena_dict(arena, SET_EXTMARK_TABLE_SIZE);
            put_c(&mut dict, c"ns_id", integer_obj(i64::from(start.ns)));
            put_c(&mut dict, c"right_gravity", boolean_obj(mt_right(start)));

            if mt_paired(start) {
                put_c(
                    &mut dict,
                    c"end_row",
                    integer_obj(i64::from(extmark.end_pos.row)),
                );
                put_c(
                    &mut dict,
                    c"end_col",
                    integer_obj(i64::from(extmark.end_pos.col)),
                );
                put_c(
                    &mut dict,
                    c"end_right_gravity",
                    boolean_obj(extmark.end_right_gravity),
                );
            }

            if mt_no_undo(start) {
                put_c(&mut dict, c"undo_restore", boolean_obj(false));
            }

            if mt_invalidate(start) {
                put_c(&mut dict, c"invalidate", boolean_obj(true));
            }
            if mt_invalid(start) {
                put_c(&mut dict, c"invalid", boolean_obj(true));
            }

            decor_to_dict_legacy(&mut dict, mt_decor(start), hl_name, arena);
            add_c(&mut rv, dict_obj(dict));
        }

        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_extmark_by_id(
    buf: Buffer,
    ns_id: Integer,
    id: Integer,
    opts: *mut KeyDict_get_extmark,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
            return rv;
        }

        let details = (*opts).details;
        let hl_name = if has_key(
            (*opts).is_set__get_extmark_,
            KEYSET_OPTIDX_get_extmark__hl_name,
        ) {
            (*opts).hl_name
        } else {
            true
        };

        let extmark = extmark_from_id(b, ns_id as u32, id as u32);
        if extmark.start.pos.row < 0 {
            return rv;
        }
        extmark_to_array(extmark, false, details, hl_name, arena)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_extmarks(
    buf: Buffer,
    ns_id: Integer,
    start: Object,
    end: Object,
    opts: *mut KeyDict_get_extmarks,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = Array::new();
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return rv;
        }

        if ns_id != -1 && !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
            return rv;
        }

        let mask = (*opts).is_set__get_extmarks_;
        let details = (*opts).details;
        let hl_name = if has_key(mask, KEYSET_OPTIDX_get_extmarks__hl_name) {
            (*opts).hl_name
        } else {
            true
        };

        let mut type_filter = kExtmarkNone;
        if has_key(mask, KEYSET_OPTIDX_get_extmarks__type) {
            if strequal((*opts).type_.data, "sign") {
                type_filter = kExtmarkSign;
            } else if strequal((*opts).type_.data, "virt_text") {
                type_filter = kExtmarkVirtText;
            } else if strequal((*opts).type_.data, "virt_lines") {
                type_filter = kExtmarkVirtLines;
            } else if strequal((*opts).type_.data, "highlight") {
                type_filter = kExtmarkHighlight;
            } else {
                api_err_exp(
                    err,
                    c"type".as_ptr(),
                    c"sign, virt_text, virt_lines or highlight".as_ptr(),
                    (*opts).type_.data,
                );
                return rv;
            }
        }

        let mut limit = if has_key(mask, KEYSET_OPTIDX_get_extmarks__limit) {
            (*opts).limit
        } else {
            -1
        };

        if limit == 0 {
            return rv;
        } else if limit < 0 {
            limit = i64::MAX;
        }

        let mut l_row: c_int = 0;
        let mut l_col: colnr_T = 0;
        if !extmark_get_index_from_obj(b, ns_id, start, &mut l_row, &mut l_col, err) {
            return rv;
        }

        let mut u_row: c_int = 0;
        let mut u_col: colnr_T = 0;
        if !extmark_get_index_from_obj(b, ns_id, end, &mut u_row, &mut u_col, err) {
            return rv;
        }

        let rv_limit = limit as usize;
        let reverse = l_row > u_row || (l_row == u_row && l_col > u_col);
        if reverse {
            limit = i64::MAX;
            std::mem::swap(&mut l_row, &mut u_row);
            std::mem::swap(&mut l_col, &mut u_col);
        }

        let mut marks = extmark_get(
            b,
            ns_id as u32,
            l_row,
            l_col,
            u_row,
            u_col,
            limit,
            type_filter,
            (*opts).overlap,
        );

        rv = arena_array(arena, marks.size.min(rv_limit));
        if reverse {
            let mut i = marks.size as isize - 1;
            while i >= 0 && rv.size < rv_limit {
                add_c(
                    &mut rv,
                    array_obj(extmark_to_array(
                        *marks.items.add(i as usize),
                        true,
                        details,
                        hl_name,
                        arena,
                    )),
                );
                i -= 1;
            }
        } else {
            for i in 0..marks.size {
                add_c(
                    &mut rv,
                    array_obj(extmark_to_array(
                        *marks.items.add(i),
                        true,
                        details,
                        hl_name,
                        arena,
                    )),
                );
            }
        }

        marks.destroy();
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_extmark(
    buf: Buffer,
    ns_id: Integer,
    mut line: Integer,
    mut col: Integer,
    opts: *mut KeyDict_set_extmark,
    err: *mut Error,
) -> Integer {
    unsafe {
        let mut hl = DECOR_HIGHLIGHT_INLINE_INIT;
        let mut sign = DECOR_SIGN_HIGHLIGHT_INIT;
        let mut virt_text = DecorVirtText {
            flags: 0,
            hl_mode: kHlModeUnknown,
            priority: DECOR_PRIORITY_BASE,
            width: 0,
            col: 0,
            pos: kVPosEndOfLine,
            data: DecorVirtTextData {
                virt_text: VirtText::new(),
            },
            next: ptr::null_mut(),
        };
        let mut virt_lines = DecorVirtText {
            flags: kVTIsLines,
            hl_mode: kHlModeUnknown,
            priority: DECOR_PRIORITY_BASE,
            width: 0,
            col: 0,
            pos: kVPosEndOfLine,
            data: DecorVirtTextData {
                virt_lines: VirtLines::new(),
            },
            next: ptr::null_mut(),
        };
        let mut url: *mut c_char = ptr::null_mut();
        let mut has_hl = false;
        let mut has_hl_multiple = false;

        'error: {
            let b = find_buffer_by_handle(buf, err);
            if b.is_null() {
                break 'error;
            }

            if !ns_initialized(ns_id as u32) {
                api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
                break 'error;
            }

            let mask = (*opts).is_set__set_extmark_;
            let mut id: u32 = 0;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__id) {
                if (*opts).id <= 0 {
                    api_err_exp(
                        err,
                        c"id".as_ptr(),
                        c"positive Integer".as_ptr(),
                        ptr::null(),
                    );
                    break 'error;
                }
                id = (*opts).id as u32;
            }

            let mut line2: c_int = -1;
            let mut did_end_line = false;

            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_line) {
                if has_key(mask, KEYSET_OPTIDX_set_extmark__end_row) {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"cannot use both 'end_row' and 'end_line'".as_ptr(),
                    );
                    break 'error;
                }
                (*opts).end_row = (*opts).end_line;
                did_end_line = true;
            }

            let strict = if has_key(mask, KEYSET_OPTIDX_set_extmark__strict) {
                (*opts).strict
            } else {
                true
            };

            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_row) || did_end_line {
                let val = (*opts).end_row;
                if val < 0 || (val > i64::from((*b).b_ml.ml_line_count) && strict) {
                    api_err_range(err, c"end_row".as_ptr());
                    break 'error;
                }
                line2 = val as c_int;
            }

            let mut col2: colnr_T = -1;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_col) {
                let mut val = (*opts).end_col;
                if !(-1..=MAXCOL).contains(&val) {
                    api_err_range(err, c"end_col".as_ptr());
                    break 'error;
                }
                if val == -1 {
                    val = MAXCOL;
                }
                col2 = val as colnr_T;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__hl_group) {
                if (*opts).hl_group.obj_type == ObjectType::kObjectTypeArray {
                    let arr = (*opts).hl_group.data.array;
                    if arr.size >= 1 {
                        hl.hl_id =
                            object_to_hl_id(*arr.items.add(0), c"hl_group item".as_ptr(), err);
                        if error_set(err) {
                            break 'error;
                        }
                    }
                    for i in 1..arr.size {
                        let hl_id =
                            object_to_hl_id(*arr.items.add(i), c"hl_group item".as_ptr(), err);
                        if error_set(err) {
                            break 'error;
                        }
                        if hl_id != 0 {
                            has_hl_multiple = true;
                        }
                    }
                } else {
                    hl.hl_id = object_to_hl_id((*opts).hl_group, c"hl_group".as_ptr(), err);
                    if error_set(err) {
                        break 'error;
                    }
                }
                has_hl = hl.hl_id > 0;
            }

            sign.hl_id = (*opts).sign_hl_group as c_int;
            sign.cursorline_hl_id = (*opts).cursorline_hl_group as c_int;
            sign.number_hl_id = (*opts).number_hl_group as c_int;
            sign.line_hl_id = (*opts).line_hl_group as c_int;

            if sign.hl_id != 0
                || sign.cursorline_hl_id != 0
                || sign.number_hl_id != 0
                || sign.line_hl_id != 0
            {
                sign.flags |= kSHIsSign;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__conceal) {
                if (*opts).conceal.obj_type != ObjectType::kObjectTypeBoolean
                    && (*opts).conceal.obj_type != ObjectType::kObjectTypeString
                {
                    api_err_exp(
                        err,
                        c"conceal".as_ptr(),
                        c"String or Boolean".as_ptr(),
                        api_typename((*opts).conceal.obj_type),
                    );
                    break 'error;
                }

                has_hl = true;
                if (*opts).conceal.obj_type == ObjectType::kObjectTypeBoolean {
                    hl.flags |= if (*opts).conceal.data.boolean {
                        kSHConceal
                    } else {
                        kSHConcealOff
                    };
                } else {
                    let conceal_str = (*opts).conceal.data.string;
                    hl.flags |= kSHConceal;
                    if conceal_str.size > 0 {
                        let mut ch: c_int = 0;
                        hl.conceal_char = utfc_ptr2schar(conceal_str.data, &mut ch);
                        if hl.conceal_char == 0 || !vim_isprintc(ch) {
                            api_set_error(
                                err,
                                ErrorType::kErrorTypeValidation,
                                c"%s".as_ptr(),
                                c"conceal char must be printable".as_ptr(),
                            );
                            break 'error;
                        }
                    }
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__conceal_lines) {
                hl.flags |= kSHConcealLines;
                has_hl = true;
                if (*opts).conceal_lines.size > 0 && *(*opts).conceal_lines.data != NUL {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"conceal_lines has to be an empty string".as_ptr(),
                    );
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text) {
                virt_text.data.virt_text =
                    parse_virt_text((*opts).virt_text, err, &mut virt_text.width, false);
                if error_set(err) {
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text_pos) {
                let str_val = (*opts).virt_text_pos;
                if strequal(str_val.data, "eol") {
                    virt_text.pos = kVPosEndOfLine;
                } else if strequal(str_val.data, "overlay") {
                    virt_text.pos = kVPosOverlay;
                } else if strequal(str_val.data, "right_align") {
                    virt_text.pos = kVPosRightAlign;
                } else if strequal(str_val.data, "eol_right_align") {
                    virt_text.pos = kVPosEndOfLineRightAlign;
                } else if strequal(str_val.data, "inline") {
                    virt_text.pos = kVPosInline;
                } else {
                    api_err_invalid(err, c"virt_text_pos".as_ptr(), str_val.data, 0, true);
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text_win_col) {
                virt_text.col = (*opts).virt_text_win_col as c_int;
                virt_text.pos = kVPosWinCol;
            }

            hl.flags |= if (*opts).hl_eol { kSHHlEol } else { 0 };
            virt_text.flags |= (if (*opts).virt_text_hide { kVTHide } else { 0 })
                | (if (*opts).virt_text_repeat_linebreak {
                    kVTRepeatLinebreak
                } else {
                    0
                });

            if has_key(mask, KEYSET_OPTIDX_set_extmark__hl_mode) {
                let str_val = (*opts).hl_mode;
                if strequal(str_val.data, "replace") {
                    virt_text.hl_mode = kHlModeReplace;
                } else if strequal(str_val.data, "combine") {
                    virt_text.hl_mode = kHlModeCombine;
                } else if strequal(str_val.data, "blend") {
                    if virt_text.pos == kVPosInline {
                        api_set_error(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"%s".as_ptr(),
                            c"cannot use 'blend' hl_mode with inline virtual text".as_ptr(),
                        );
                        break 'error;
                    }
                    virt_text.hl_mode = kHlModeBlend;
                } else {
                    api_err_invalid(err, c"hl_mode".as_ptr(), str_val.data, 0, true);
                    break 'error;
                }
            }

            let virt_lines_flags = if (*opts).virt_lines_leftcol {
                kVLLeftcol
            } else {
                0
            };
            let mut virt_lines_overflow = kVLOverflowTrunc;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_lines_overflow) {
                let str_val = (*opts).virt_lines_overflow;
                if strequal(str_val.data, "scroll") {
                    virt_lines_overflow = kVLOverflowScroll;
                } else if strequal(str_val.data, "wrap") {
                    virt_lines_overflow = kVLOverflowWrap;
                } else if strequal(str_val.data, "auto") {
                    virt_lines_overflow = kVLOverflowAuto;
                } else if !strequal(str_val.data, "trunc") {
                    api_err_invalid(err, c"virt_lines_overflow".as_ptr(), str_val.data, 0, true);
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_lines) {
                let a = (*opts).virt_lines;
                for j in 0..a.size {
                    let item = *a.items.add(j);
                    if item.obj_type != ObjectType::kObjectTypeArray {
                        api_err_exp(
                            err,
                            c"virt_text_line".as_ptr(),
                            api_typename(ObjectType::kObjectTypeArray),
                            api_typename(item.obj_type),
                        );
                        break 'error;
                    }
                    let mut dummig: c_int = 0;
                    let jtem = parse_virt_text(item.data.array, err, &mut dummig, false);
                    virt_lines.data.virt_lines.push(VirtLine {
                        line: jtem,
                        flags: virt_lines_flags,
                        overflow: virt_lines_overflow,
                    });
                    if error_set(err) {
                        break 'error;
                    }
                }
            }

            virt_lines.flags |= if (*opts).virt_lines_above {
                kVTLinesAbove
            } else {
                0
            };

            if has_key(mask, KEYSET_OPTIDX_set_extmark__priority) {
                if !((0..=i64::from(u16::MAX)).contains(&(*opts).priority)) {
                    api_err_range(err, c"priority".as_ptr());
                    break 'error;
                }
                let prio = (*opts).priority as DecorPriority;
                hl.priority = prio;
                sign.priority = prio;
                virt_text.priority = prio;
                virt_lines.priority = prio;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__sign_text) {
                sign.text[0] = 0;
                if init_sign_text(
                    ptr::null_mut(),
                    sign.text.as_mut_ptr(),
                    (*opts).sign_text.data,
                ) == 0
                {
                    api_err_invalid(err, c"sign_text".as_ptr(), c"".as_ptr(), 0, true);
                    break 'error;
                }
                sign.flags |= kSHIsSign;
            }

            let right_gravity = if has_key(mask, KEYSET_OPTIDX_set_extmark__right_gravity) {
                (*opts).right_gravity
            } else {
                true
            };

            if line2 == -1
                && col2 == -1
                && has_key(mask, KEYSET_OPTIDX_set_extmark__end_right_gravity)
            {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"cannot set end_right_gravity without end_row or end_col".as_ptr(),
                );
                break 'error;
            }

            let mut len: colnr_T = 0;

            if has_key(mask, KEYSET_OPTIDX_set_extmark__spell) {
                hl.flags |= if (*opts).spell {
                    kSHSpellOn
                } else {
                    kSHSpellOff
                };
                has_hl = true;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__url) {
                url = string_to_cstr((*opts).url);
                has_hl = true;
            }

            if (*opts).ui_watched {
                hl.flags |= kSHUIWatched;
                if virt_text.pos == kVPosOverlay {
                    hl.flags |= kSHUIWatchedOverlay;
                }
                has_hl = true;
            }

            if line < 0 {
                api_err_range(err, c"line".as_ptr());
                break 'error;
            }

            if line > i64::from((*b).b_ml.ml_line_count) {
                if strict {
                    api_err_range(err, c"line".as_ptr());
                    break 'error;
                }
                line = i64::from((*b).b_ml.ml_line_count);
            } else if line < i64::from((*b).b_ml.ml_line_count) {
                len = if (*opts).ephemeral {
                    MAXCOL as colnr_T
                } else {
                    ml_get_buf_len(b, (line as linenr_T) + 1)
                };
            }

            if col == -1 {
                col = i64::from(len);
            } else if col > i64::from(len) {
                if strict {
                    api_err_range(err, c"col".as_ptr());
                    break 'error;
                }
                col = i64::from(len);
            } else if col < -1 {
                api_err_range(err, c"col".as_ptr());
                break 'error;
            }

            if col2 >= 0 {
                if line2 >= 0 && line2 < (*b).b_ml.ml_line_count {
                    len = if (*opts).ephemeral {
                        MAXCOL as colnr_T
                    } else {
                        ml_get_buf_len(b, line2 + 1)
                    };
                } else if line2 == (*b).b_ml.ml_line_count {
                    len = 0;
                } else {
                    line2 = line as c_int;
                }
                if col2 > len {
                    if strict {
                        api_err_range(err, c"end_col".as_ptr());
                        break 'error;
                    }
                    col2 = len;
                }
            } else if line2 >= 0 {
                col2 = 0;
            }

            if (*opts).ephemeral && !decor_state.win.is_null() && (*decor_state.win).w_buffer == b {
                let r = line as c_int;
                let c = col as c_int;
                if line2 == -1 {
                    line2 = r;
                    col2 = c;
                }

                let mut subpriority: DecorPriority = 0;
                if has_key(mask, KEYSET_OPTIDX_set_extmark___subpriority) {
                    if !((0..=i64::from(u16::MAX)).contains(&(*opts)._subpriority)) {
                        api_err_range(err, c"_subpriority".as_ptr());
                        break 'error;
                    }
                    subpriority = (*opts)._subpriority as DecorPriority;
                }

                if virt_text.data.virt_text.size > 0 {
                    decor_range_add_virt(
                        &raw mut decor_state,
                        r,
                        c,
                        line2,
                        col2,
                        decor_put_vt(virt_text, ptr::null_mut()),
                        true,
                    );
                }
                if virt_lines.data.virt_lines.size > 0 {
                    decor_range_add_virt(
                        &raw mut decor_state,
                        r,
                        c,
                        line2,
                        col2,
                        decor_put_vt(virt_lines, ptr::null_mut()),
                        true,
                    );
                }
                if has_hl {
                    let mut sh = decor_sh_from_inline(hl);
                    sh.url = url;
                    decor_range_add_sh(
                        &raw mut decor_state,
                        r,
                        c,
                        line2,
                        col2,
                        &mut sh,
                        true,
                        ns_id as u32,
                        id,
                        subpriority,
                    );
                }
            } else {
                if (*opts).ephemeral {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeException,
                        c"cannot set emphemeral mark outside of a decoration provider".as_ptr(),
                    );
                    break 'error;
                }

                let mut decor_flags: u16 = 0;
                let mut decor_alloc: *mut DecorVirtText = ptr::null_mut();
                if virt_text.data.virt_text.size > 0 {
                    decor_alloc = decor_put_vt(virt_text, decor_alloc);
                    if virt_text.pos == kVPosInline {
                        decor_flags |= MT_FLAG_DECOR_VIRT_TEXT_INLINE;
                    }
                }
                if virt_lines.data.virt_lines.size > 0 {
                    decor_alloc = decor_put_vt(virt_lines, decor_alloc);
                    decor_flags |= MT_FLAG_DECOR_VIRT_LINES;
                }

                let mut decor_indexed: u32 = DECOR_ID_INVALID;

                if (sign.flags & kSHIsSign) != 0 {
                    sign.next = decor_indexed;
                    decor_indexed = decor_put_sh(sign);
                    if sign.text[0] != 0 {
                        decor_flags |= MT_FLAG_DECOR_SIGNTEXT;
                    }
                    if sign.number_hl_id != 0 || sign.line_hl_id != 0 || sign.cursorline_hl_id != 0
                    {
                        decor_flags |= MT_FLAG_DECOR_SIGNHL;
                    }
                }

                if has_hl_multiple {
                    let arr = (*opts).hl_group.data.array;
                    let mut i = arr.size - 1;
                    while i > 0 {
                        let hl_id =
                            object_to_hl_id(*arr.items.add(i), c"hl_group item".as_ptr(), err);
                        if hl_id > 0 {
                            let mut sh = DECOR_SIGN_HIGHLIGHT_INIT;
                            sh.hl_id = hl_id;
                            sh.flags = if (*opts).hl_eol { kSHHlEol } else { 0 };
                            sh.next = decor_indexed;
                            decor_indexed = decor_put_sh(sh);
                            decor_flags |= MT_FLAG_DECOR_HL;
                        }
                        i -= 1;
                    }
                }

                if (hl.flags & kSHConcealLines) != 0 {
                    decor_flags |= MT_FLAG_DECOR_CONCEAL_LINES;
                }

                let mut decor = DecorInline {
                    ext: false,
                    data: DecorInlineData {
                        hl: DECOR_HIGHLIGHT_INLINE_INIT,
                    },
                };
                if !decor_alloc.is_null()
                    || decor_indexed != DECOR_ID_INVALID
                    || !url.is_null()
                    || schar_high(hl.conceal_char)
                {
                    if has_hl {
                        let mut sh = decor_sh_from_inline(hl);
                        sh.url = url;
                        sh.next = decor_indexed;
                        decor_indexed = decor_put_sh(sh);
                    }
                    decor.ext = true;
                    decor.data.ext = DecorExt {
                        sh_idx: decor_indexed,
                        vt: decor_alloc.cast(),
                    };
                } else {
                    decor.data.hl = hl;
                }

                if has_hl {
                    decor_flags |= MT_FLAG_DECOR_HL;
                }

                let undo_restore = if has_key(mask, KEYSET_OPTIDX_set_extmark__undo_restore) {
                    (*opts).undo_restore
                } else {
                    true
                };

                extmark_set(
                    b,
                    ns_id as u32,
                    &mut id,
                    line as c_int,
                    col as colnr_T,
                    line2,
                    col2,
                    decor,
                    decor_flags,
                    right_gravity,
                    (*opts).end_right_gravity,
                    !undo_restore,
                    (*opts).invalidate,
                    err,
                );
                if error_set(err) {
                    decor_free(decor);
                    return 0;
                }
            }

            return i64::from(id);
        }

        clear_virttext(&raw mut virt_text.data.virt_text);
        clear_virtlines(&raw mut virt_lines.data.virt_lines);
        if !url.is_null() {
            xfree(url.cast());
        }
        0
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_del_extmark(
    buf: Buffer,
    ns_id: Integer,
    id: Integer,
    err: *mut Error,
) -> Boolean {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return false;
        }
        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
            return false;
        }
        extmark_del_id(b, ns_id as u32, id as u32)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_clear_namespace(
    buf: Buffer,
    ns_id: Integer,
    line_start: Integer,
    mut line_end: Integer,
    err: *mut Error,
) {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return;
        }

        if !(0..MAXLNUM).contains(&line_start) {
            api_err_range(err, c"line number".as_ptr());
            return;
        }

        if !(0..=MAXLNUM).contains(&line_end) {
            line_end = MAXLNUM;
        }
        extmark_clear(
            b,
            if ns_id < 0 { 0 } else { ns_id as u32 },
            line_start as c_int,
            0,
            (line_end as c_int) - 1,
            MAXCOL as colnr_T,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_decoration_provider(
    ns_id: Integer,
    opts: *mut KeyDict_set_decoration_provider,
    _err: *mut Error,
) {
    unsafe {
        let p = get_decor_provider(ns_id as NS, true);
        assert!(!p.is_null());
        decor_provider_clear(p);

        redraw_all_later(UPD_NOT_VALID);

        let pairs: [(*mut LuaRef, *mut LuaRef); 9] = [
            (&raw mut (*opts).on_start, &raw mut (*p).redraw_start),
            (&raw mut (*opts).on_buf, &raw mut (*p).redraw_buf),
            (&raw mut (*opts).on_win, &raw mut (*p).redraw_win),
            (&raw mut (*opts).on_line, &raw mut (*p).redraw_line),
            (&raw mut (*opts).on_range, &raw mut (*p).redraw_range),
            (&raw mut (*opts).on_end, &raw mut (*p).redraw_end),
            (&raw mut (*opts)._on_hl_def, &raw mut (*p).hl_def),
            (&raw mut (*opts)._on_spell_nav, &raw mut (*p).spell_nav),
            (
                &raw mut (*opts)._on_conceal_line,
                &raw mut (*p).conceal_line,
            ),
        ];

        for (src, dst) in pairs {
            if *src > 0 {
                *dst = *src;
                *src = LUA_NOREF;
            }
        }

        (*p).state = kDecorProviderActive;
        (*p).hl_valid += 1;
        (*p).hl_cached = false;
    }
}

unsafe fn extmark_get_index_from_obj(
    buf: *mut buf_T,
    ns_id: Integer,
    obj: Object,
    row: *mut c_int,
    col: *mut colnr_T,
    err: *mut Error,
) -> bool {
    unsafe {
        if obj.obj_type == ObjectType::kObjectTypeInteger {
            let id = obj.data.integer;
            if id == 0 {
                *row = 0;
                *col = 0;
                return true;
            } else if id == -1 {
                *row = MAXLNUM as c_int;
                *col = MAXCOL as colnr_T;
                return true;
            } else if id < 0 {
                api_err_invalid(err, c"mark id".as_ptr(), ptr::null(), id, false);
                return false;
            }

            let extmark = extmark_from_id(buf, ns_id as u32, id as u32);
            if extmark.start.pos.row < 0 {
                api_err_invalid(err, c"mark id (not found)".as_ptr(), ptr::null(), id, false);
                return false;
            }
            *row = extmark.start.pos.row;
            *col = extmark.start.pos.col;
            true
        } else if obj.obj_type == ObjectType::kObjectTypeArray {
            let pos = obj.data.array;
            if pos.size != 2
                || (*pos.items.add(0)).obj_type != ObjectType::kObjectTypeInteger
                || (*pos.items.add(1)).obj_type != ObjectType::kObjectTypeInteger
            {
                api_err_exp(
                    err,
                    c"mark position".as_ptr(),
                    c"2 Integer items".as_ptr(),
                    ptr::null(),
                );
                return false;
            }

            let pos_row = (*pos.items.add(0)).data.integer;
            let pos_col = (*pos.items.add(1)).data.integer;
            *row = (if pos_row >= 0 { pos_row } else { MAXLNUM }) as c_int;
            *col = (if pos_col >= 0 { pos_col } else { MAXCOL }) as colnr_T;
            true
        } else {
            api_err_exp(
                err,
                c"mark position".as_ptr(),
                c"mark id Integer or 2-item Array".as_ptr(),
                ptr::null(),
            );
            false
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_virt_text(
    chunks: Array,
    err: *mut Error,
    width: *mut c_int,
    untab: bool,
) -> VirtText {
    unsafe {
        let mut virt_text = VirtText::new();
        let mut w: c_int = 0;

        'free_exit: {
            for i in 0..chunks.size {
                let item = *chunks.items.add(i);
                if item.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"chunk".as_ptr(),
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(item.obj_type),
                    );
                    break 'free_exit;
                }
                let chunk = item.data.array;
                if chunk.size == 0
                    || chunk.size > 2
                    || (*chunk.items.add(0)).obj_type != ObjectType::kObjectTypeString
                {
                    api_set_error(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Invalid chunk: expected Array with 1 or 2 Strings".as_ptr(),
                    );
                    break 'free_exit;
                }

                let str_val = (*chunk.items.add(0)).data.string;
                let mut hl_id: c_int = -1;
                if chunk.size == 2 {
                    let hl = *chunk.items.add(1);
                    if hl.obj_type == ObjectType::kObjectTypeArray {
                        let arr = hl.data.array;
                        for j in 0..arr.size {
                            hl_id = object_to_hl_id(
                                *arr.items.add(j),
                                c"virt_text highlight".as_ptr(),
                                err,
                            );
                            if error_set(err) {
                                break 'free_exit;
                            }
                            if j < arr.size - 1 {
                                virt_text.push(VirtTextChunk {
                                    text: ptr::null_mut(),
                                    hl_id,
                                });
                            }
                        }
                    } else {
                        hl_id = object_to_hl_id(hl, c"virt_text highlight".as_ptr(), err);
                        if error_set(err) {
                            break 'free_exit;
                        }
                    }
                }

                let text = transstr(
                    if str_val.size > 0 {
                        str_val.data
                    } else {
                        c"".as_ptr()
                    },
                    untab,
                );
                w += mb_string2cells(text) as c_int;

                virt_text.push(VirtTextChunk { text, hl_id });
            }

            if !width.is_null() {
                *width = w;
            }
            return virt_text;
        }

        clear_virttext(&mut virt_text);
        virt_text
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__buf_debug_extmarks(
    buf: Buffer,
    keys: Boolean,
    dot: Boolean,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let b = find_buffer_by_handle(buf, err);
        if b.is_null() {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        }
        mt_inspect((*b).b_marktree.as_mut_ptr(), keys, dot)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__ns_set(ns_id: Integer, opts: *mut KeyDict_ns_opts, err: *mut Error) {
    unsafe {
        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
            return;
        }

        let mut set_scoped = true;

        if has_key((*opts).is_set__ns_opts_, KEYSET_OPTIDX_ns_opts__wins) {
            if (*opts).wins.size == 0 {
                set_scoped = false;
            }

            let mut windows = Set::<ptr_t> {
                h: MapHash {
                    n_buckets: 0,
                    size: 0,
                    n_occupied: 0,
                    upper_bound: 0,
                    n_keys: 0,
                    keys_capacity: 0,
                    hash: ptr::null_mut(),
                },
                keys: ptr::null_mut(),
            };
            for i in 0..(*opts).wins.size {
                let win = (*(*opts).wins.items.add(i)).data.integer;
                let wp = find_window_by_handle(win as Window, err);
                if wp.is_null() {
                    set_destroy(&mut windows);
                    return;
                }
                set_put_ptr(&mut windows, wp.cast());
            }

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if set_has_ptr(&mut windows, wp.cast())
                        && !set_has_u32(&raw mut (*wp).w_ns_set, ns_id as u32)
                    {
                        set_put_u32(&raw mut (*wp).w_ns_set, ns_id as u32);
                        if set_has_u32(&raw mut (*(*wp).w_buffer).b_extmark_ns[0].set, ns_id as u32)
                        {
                            changed_window_setting(wp);
                        }
                    }

                    if set_has_u32(&raw mut (*wp).w_ns_set, ns_id as u32)
                        && !set_has_ptr(&mut windows, wp.cast())
                    {
                        set_del_u32(&raw mut (*wp).w_ns_set, ns_id as u32);
                        if set_has_u32(&raw mut (*(*wp).w_buffer).b_extmark_ns[0].set, ns_id as u32)
                        {
                            changed_window_setting(wp);
                        }
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }

            set_destroy(&mut windows);
        }

        if set_scoped && !set_has_u32(&raw mut namespace_localscope, ns_id as u32) {
            set_put_u32(&raw mut namespace_localscope, ns_id as u32);

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if set_has_u32(&raw mut (*(*wp).w_buffer).b_extmark_ns[0].set, ns_id as u32) {
                        changed_window_setting(wp);
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
        } else if !set_scoped && set_has_u32(&raw mut namespace_localscope, ns_id as u32) {
            set_del_u32(&raw mut namespace_localscope, ns_id as u32);

            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if set_has_u32(&raw mut (*(*wp).w_buffer).b_extmark_ns[0].set, ns_id as u32) {
                        changed_window_setting(wp);
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim__ns_get(
    ns_id: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> KeyDict_ns_opts {
    unsafe {
        let mut opts = KeyDict_ns_opts {
            is_set__ns_opts_: 1u64 << KEYSET_OPTIDX_ns_opts__wins,
            wins: Array::new(),
        };

        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id".as_ptr(), ptr::null(), ns_id, false);
            return opts;
        }

        if !set_has_u32(&raw mut namespace_localscope, ns_id as u32) {
            return opts;
        }

        let mut count = 0usize;
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if set_has_u32(&raw mut (*wp).w_ns_set, ns_id as u32) {
                    count += 1;
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        let mut windows = arena_array(arena, count);

        tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                if set_has_u32(&raw mut (*wp).w_ns_set, ns_id as u32) {
                    windows.push(integer_obj(i64::from((*wp).handle)));
                }
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        opts.wins = windows;
        opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marktree::MTPos;

    #[test]
    fn mt_key_flag_predicates() {
        let key = MTKey {
            pos: MTPos { row: 1, col: 2 },
            ns: 3,
            id: 4,
            flags: MT_FLAG_RIGHT_GRAVITY | MT_FLAG_PAIRED | MT_FLAG_INVALIDATE,
            decor_data: DecorInlineData {
                hl: DECOR_HIGHLIGHT_INLINE_INIT,
            },
        };
        assert!(mt_right(key));
        assert!(mt_paired(key));
        assert!(mt_invalidate(key));
        assert!(!mt_no_undo(key));
        assert!(!mt_invalid(key));
    }
}
