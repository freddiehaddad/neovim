//! Extmark and namespace API functions (`src/nvim/api/extmark.c`).

use crate::api::private::helpers::{
    MAXLNUM, add_c, api_set_error, api_typename, arena_array, arena_dict, array_obj, boolean_obj,
    cstr_as_string, dict_obj, error_set, find_window_by_handle, has_key,
    integer_obj, object_to_hl_id, put_c, string_obj, string_to_cstr,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType, api_err_exp,
    api_err_invalid, api_err_range,
};
use crate::api::tabpage::{Buffer, Window};
use crate::buffer::buf_T;
use crate::charset::transstr;
use crate::decoration::{
    DECOR_ID_INVALID, DECOR_SIGN_HIGHLIGHT_INIT, DecorInline, DecorPriority, DecorVirtText,
    DecorVirtTextData, HLGroupID, NS, VirtLine, VirtText, VirtTextChunk, decor_free, decor_put_sh,
    decor_put_vt, decor_range_add_sh, decor_range_add_virt, decor_sh_from_inline,
    decor_to_dict_legacy, hl_group_name, kHlModeUnknown, kSHConceal, kSHConcealLines,
    kSHConcealOff, kSHHlEol, kSHIsSign, kSHSpellOff, kSHSpellOn, kSHUIWatched, kSHUIWatchedOverlay,
    kVLLeftcol, kVLOverflowAuto, kVLOverflowScroll, kVLOverflowTrunc, kVLOverflowWrap,
    kVPosEndOfLine, kVPosEndOfLineRightAlign, kVPosInline, kVPosOverlay, kVPosRightAlign,
    kVPosWinCol, kVTHide, kVTIsLines, kVTLinesAbove, kVTRepeatLinebreak,
};
use crate::decoration_provider::{decor_provider_clear, get_decor_provider, kDecorProviderActive};
use crate::drawline::kHlModeBlend;
use crate::drawline::kHlModeCombine;
use crate::drawline::kHlModeReplace;
use crate::drawscreen::{UPD_NOT_VALID, redraw_all_later};
use crate::eval::vars::MAXCOL;
use crate::extmark::{
    extmark_clear, extmark_del_id, extmark_from_id, extmark_get, extmark_set, kExtmarkHighlight,
    kExtmarkNone, kExtmarkSign, kExtmarkVirtLines, kExtmarkVirtText,
};
use crate::globals::*;
use crate::grid::{schar_from_cluster, schar_high};
use crate::highlight::OptionalKeys;
use crate::lua::converter::{Boolean, LUA_NOREF, LuaRef};
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DECOR_PRIORITY_BASE, DecorExt, DecorInlineData,
    MT_FLAG_DECOR_CONCEAL_LINES, MT_FLAG_DECOR_HL, MT_FLAG_DECOR_SIGNHL, MT_FLAG_DECOR_SIGNTEXT,
    MT_FLAG_DECOR_VIRT_LINES, MT_FLAG_DECOR_VIRT_TEXT_INLINE, MTPair, mt_decor, mt_inspect,
    mt_invalid, mt_invalidate, mt_no_undo, mt_paired, mt_right,
};
use crate::mbyte::{mb_string2cells, utf_ClusterInfo, utf_ptr2StrCharInfo};
use crate::memline::ml_get_buf_len;
use crate::memory::{Arena};
use crate::r#move::changed_window_setting;
use crate::msgpack_rpc::packer::Integer;
use crate::sign::init_sign_text;
use crate::window::win_T;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::cursor::{colnr_T, linenr_T};

const SET_EXTMARK_TABLE_SIZE: usize = 36;

const KEYSET_OPTIDX_get_extmark__hl_name: u64 = 2;
const KEYSET_OPTIDX_get_extmarks__type: u64 = 1;
const KEYSET_OPTIDX_get_extmarks__limit: u64 = 2;
const KEYSET_OPTIDX_get_extmarks__hl_name: u64 = 4;

const KEYSET_OPTIDX_set_extmark__id: u64 = 1;
const KEYSET_OPTIDX_set_extmark__url: u64 = 2;
const KEYSET_OPTIDX_set_extmark__spell: u64 = 3;
const KEYSET_OPTIDX_set_extmark__strict: u64 = 6;
const KEYSET_OPTIDX_set_extmark__end_col: u64 = 7;
const KEYSET_OPTIDX_set_extmark__conceal: u64 = 8;
const KEYSET_OPTIDX_set_extmark__hl_mode: u64 = 9;
const KEYSET_OPTIDX_set_extmark__end_row: u64 = 10;
const KEYSET_OPTIDX_set_extmark__end_line: u64 = 11;
const KEYSET_OPTIDX_set_extmark__hl_group: u64 = 12;
const KEYSET_OPTIDX_set_extmark__priority: u64 = 13;
const KEYSET_OPTIDX_set_extmark__sign_text: u64 = 15;
const KEYSET_OPTIDX_set_extmark__virt_text: u64 = 16;
const KEYSET_OPTIDX_set_extmark__virt_lines: u64 = 19;
const KEYSET_OPTIDX_set_extmark___subpriority: u64 = 20;
const KEYSET_OPTIDX_set_extmark__undo_restore: u64 = 21;
const KEYSET_OPTIDX_set_extmark__conceal_lines: u64 = 22;
const KEYSET_OPTIDX_set_extmark__right_gravity: u64 = 24;
const KEYSET_OPTIDX_set_extmark__virt_text_pos: u64 = 26;
const KEYSET_OPTIDX_set_extmark__end_right_gravity: u64 = 30;
const KEYSET_OPTIDX_set_extmark__virt_text_win_col: u64 = 31;
const KEYSET_OPTIDX_set_extmark__virt_lines_overflow: u64 = 34;

const KEYSET_OPTIDX_ns_opts__wins: u64 = 1;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_get_extmark {
    pub is_set__get_extmark_: OptionalKeys,
    pub details: Boolean,
    pub hl_name: Boolean,
}

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_get_extmarks {
    pub is_set__get_extmarks_: OptionalKeys,
    pub limit: Integer,
    pub details: Boolean,
    pub hl_name: Boolean,
    pub overlap: Boolean,
    pub type_: NvimString,
}

#[repr(C)]
#[derive(Default)]
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
#[derive(Default)]
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
#[derive(Clone, Copy, Default)]
pub struct KeyDict_ns_opts {
    pub is_set__ns_opts_: OptionalKeys,
    pub wins: Array,
}

#[inline]
pub fn ns_in_win(ns_id: u32, wp: &mut win_T) -> bool {
    unsafe {
        let local_ptr = &raw const g_st().namespace_localscope;
        if !(*local_ptr).contains(&ns_id) {
            return true;
        }
        wp.w_ns_set.contains(&ns_id)
    }
}

// DLLEXPORT void api_extmark_free_all_mem(void);
pub fn api_extmark_free_all_mem() {
    unsafe {
        let ns_ptr = &raw mut g_st().namespace_ids;
        *ns_ptr = Vec::new();
        let local_ptr = &raw mut g_st().namespace_localscope;
        *local_ptr = std::collections::BTreeSet::new();
    }
}

/// Creates a new namespace or gets an existing one. [namespace]()
///
/// Namespaces are used for buffer highlights and virtual text, see
/// |nvim_buf_set_extmark()|.
///
/// Namespaces can be named or anonymous. If `name` matches an existing
/// namespace, the associated id is returned. If `name` is an empty string
/// a new, anonymous namespace is created.
///
/// @param name Namespace name or empty string
/// @return Namespace id
// DLLEXPORT Integer nvim_create_namespace(String name)
//   FUNC_API_SINCE(5);
pub fn nvim_create_namespace(name: NvimString) -> Integer {
    unsafe {
        let name_bytes = if name.size == 0 || name.data.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(name.data.cast::<u8>(), name.size)
        };
        let ns_ptr = &raw mut g_st().namespace_ids;
        if !name_bytes.is_empty()
            && let Some((_, id)) = (*ns_ptr).iter().find(|(k, _)| k.as_bytes() == name_bytes)
        {
            return i64::from(*id);
        }
        let id = g_st().next_namespace_id;
        g_st().next_namespace_id += 1;
        if !name_bytes.is_empty() {
            (*ns_ptr).push((
                std::ffi::CString::from_vec_unchecked(name_bytes.to_vec()),
                id,
            ));
        }
        i64::from(id)
    }
}

/// Gets existing, non-anonymous |namespace|s.
///
/// @return dict that maps from names to namespace ids.
// DLLEXPORT DictOf(Integer) nvim_get_namespaces(Arena *arena)
//   FUNC_API_SINCE(5);
pub fn nvim_get_namespaces(arena: &mut Arena) -> Dict {
    unsafe {
        let ns_ptr = &raw const g_st().namespace_ids;
        let mut retval = arena_dict(arena, (*ns_ptr).len());
        for (name, id) in &*ns_ptr {
            *retval.items.add(retval.size) = KeyValuePair {
                key: cstr_as_string(name.as_ptr()),
                value: integer_obj(i64::from(*id)),
            };
            retval.size += 1;
        }
        retval
    }
}

// DLLEXPORT const char *describe_ns(NS ns_id, const char *unknown);
pub fn describe_ns(ns_id: NS, unknown: &std::ffi::CStr) -> *const c_char {
    unsafe {
        let ns_ptr = &raw const g_st().namespace_ids;
        for (name, id) in &*ns_ptr {
            if *id == ns_id && !name.as_bytes().is_empty() {
                return name.as_ptr();
            }
        }
        unknown.as_ptr()
    }
}

// DLLEXPORT bool ns_initialized(uint32_t ns);
pub fn ns_initialized(ns: u32) -> bool {
    if ns < 1 {
        return false;
    }
    ns < g_st().next_namespace_id as u32
}

// DLLEXPORT Array virt_text_to_array(VirtText vt, bool hl_name, Arena *arena);
pub fn virt_text_to_array(vt: &[VirtTextChunk], hl_name: bool, arena: &mut Arena) -> Array {
    let mut chunks = arena_array(arena, vt.len());
        let mut i = 0usize;
        while i < vt.len() {
            let mut j = i;
            while j < vt.len() {
                if vt[j].text.is_some() {
                    break;
                }
                j += 1;
            }

            let mut hl_array = arena_array(arena, if i < j { j - i + 1 } else { 0 });
            while i < j {
                let hl_id = vt[i].hl_id;
                if hl_id >= 0 {
                    add_c(&mut hl_array, hl_group_name(hl_id, hl_name));
                }
                i += 1;
            }

            let text = vt[i].text.as_ref().map_or(ptr::null(), |s| s.as_ptr());
            let hl_id = vt[i].hl_id;
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

fn extmark_to_array(
    extmark: MTPair,
    id: bool,
    add_dict: bool,
    hl_name: bool,
    arena: &mut Arena,
) -> Array {
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

/// Gets the position (0-indexed) of an |extmark|.
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param id  Extmark id
/// @param opts  Optional parameters. Keys:
///          - details: Whether to include the details dict
///          - hl_name: Whether to include highlight group name instead of id, true if omitted
/// @param[out] err   Error details, if any
/// @return 0-indexed (row, col, details?) tuple or empty list () if extmark id was absent.  The
/// optional `details` dictionary contains the same keys as `opts` in |nvim_buf_set_extmark()|,
/// except for `id`, `conceal_lines` and `ephemeral`. It also contains the following keys:
///
/// - ns_id: |namespace| id
/// - invalid: boolean that indicates whether the mark is hidden because the entirety of
/// text span range is deleted. See also the key `invalidate` in |nvim_buf_set_extmark()|.
// DLLEXPORT Tuple(Integer, Integer, *DictAs(extmark_details))
// nvim_buf_get_extmark_by_id(Buffer buf, Integer ns_id, Integer id, Dict(get_extmark) *opts,
//                            Arena *arena, Error *err)
//   FUNC_API_SINCE(7);
pub fn nvim_buf_get_extmark_by_id(
    buf: Buffer,
    ns_id: Integer,
    id: Integer,
    opts: &mut KeyDict_get_extmark,
    arena: &mut Arena,
    err: &mut Error,
) -> Array {
     {
        let rv = Array::new();
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return rv;
        };

        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
            return rv;
        }

        let details = opts.details;
        let hl_name = if has_key(
            opts.is_set__get_extmark_,
            KEYSET_OPTIDX_get_extmark__hl_name,
        ) {
            opts.hl_name
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

/// Gets |extmarks| in "traversal order" from a |charwise| region defined by buffer positions
/// (inclusive, 0-indexed |api-indexing|).
///
/// Region can be given as (row,col) tuples, or valid extmark ids (whose
/// positions define the bounds). 0 and -1 are understood as (0,0) and (-1,-1)
/// respectively, thus the following are equivalent:
///
/// ```lua
/// vim.api.nvim_buf_get_extmarks(0, my_ns, 0, -1)
/// vim.api.nvim_buf_get_extmarks(0, my_ns, {0,0}, {-1,-1})
/// ```
///
/// If `end` is less than `start`, marks are returned in reverse order.
/// (Useful with `limit`, to get the first marks prior to a given position.)
///
/// Note:
/// - For a reverse range, `limit` does not actually affect the traversed range, just how many marks
///   are returned
/// - When using extmark ranges (marks with a end_row/end_col position) the `overlap` option might
///   be useful. Otherwise only the start position of an extmark will be considered.
/// - The |:marks| command can list extmarks.
/// - Legacy signs placed through the |:sign| commands are implemented as extmarks. Their details
///   array will contain a `sign_name` field.
///
/// Example:
///
/// ```lua
/// local api = vim.api
/// local pos = api.nvim_win_get_cursor(0)
/// local ns  = api.nvim_create_namespace('my-plugin')
/// -- Create new extmark at line 1, column 1.
/// local m1  = api.nvim_buf_set_extmark(0, ns, 0, 0)
/// -- Create new extmark at line 3, column 1.
/// local m2  = api.nvim_buf_set_extmark(0, ns, 2, 0)
/// -- Get extmarks only from line 3.
/// local ms  = api.nvim_buf_get_extmarks(0, ns, {2,0}, {2,0})
/// -- Get all marks in this buffer + namespace.
/// local all = api.nvim_buf_get_extmarks(0, ns, 0, -1)
/// vim.print(ms)
/// ```
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id, or -1 for all namespaces
/// @param start  Start of range: a 0-indexed (row, col) or valid extmark id
/// (whose position defines the bound). |api-indexing|
/// @param end  End of range (inclusive): a 0-indexed (row, col) or valid
/// extmark id (whose position defines the bound). |api-indexing|
/// @param opts  Optional parameters. Keys:
///          - details: Whether to include the details dict
///          - hl_name: Whether to include highlight group name instead of id, true if omitted
///          - limit:  Maximum number of marks to return
///          - overlap: Also include marks which overlap the range, even if
///                     their start position is less than `start`
///          - type: Filter marks by type: "highlight", "sign", "virt_text" and "virt_lines"
/// @param[out] err   Error details, if any
/// @return List of `[extmark_id, row, col, details?]` tuples in "traversal order". For the
/// `details` dictionary, see |nvim_buf_get_extmark_by_id()|.
// DLLEXPORT ArrayOf(DictAs(get_extmark_item)) nvim_buf_get_extmarks(Buffer buf, Integer ns_id,
//                                                                   Object start, Object end,
//                                                                   Dict(get_extmarks) *opts,
//                                                                   Arena *arena, Error *err)
//   FUNC_API_SINCE(7);
pub fn nvim_buf_get_extmarks(
    buf: Buffer,
    ns_id: Integer,
    start: Object,
    end: Object,
    opts: &mut KeyDict_get_extmarks,
    arena: &mut Arena,
    err: &mut Error,
) -> Array {
     {
        let mut rv = Array::new();
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return rv;
        };

        if ns_id != -1 && !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
            return rv;
        }

        let mask = opts.is_set__get_extmarks_;
        let details = opts.details;
        let hl_name = if has_key(mask, KEYSET_OPTIDX_get_extmarks__hl_name) {
            opts.hl_name
        } else {
            true
        };

        let mut type_filter = kExtmarkNone;
        if has_key(mask, KEYSET_OPTIDX_get_extmarks__type) {
            if crate::memory::cstr_as_opt_bytes(opts.type_.data) == Some(b"sign") {
                type_filter = kExtmarkSign;
            } else if crate::memory::cstr_as_opt_bytes(opts.type_.data) == Some(b"virt_text") {
                type_filter = kExtmarkVirtText;
            } else if crate::memory::cstr_as_opt_bytes(opts.type_.data) == Some(b"virt_lines") {
                type_filter = kExtmarkVirtLines;
            } else if crate::memory::cstr_as_opt_bytes(opts.type_.data) == Some(b"highlight") {
                type_filter = kExtmarkHighlight;
            } else {
                api_err_exp(
                    err,
                    c"type",
                    c"sign, virt_text, virt_lines or highlight",
                    opts.type_.data,
                );
                return rv;
            }
        }

        let mut limit = if has_key(mask, KEYSET_OPTIDX_get_extmarks__limit) {
            opts.limit
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
        if !extmark_get_index_from_obj( b, ns_id, start, &mut l_row, &mut l_col, err) {
            return rv;
        }

        let mut u_row: c_int = 0;
        let mut u_col: colnr_T = 0;
        if !extmark_get_index_from_obj( b, ns_id, end, &mut u_row, &mut u_col, err) {
            return rv;
        }

        let rv_limit = limit as usize;
        let reverse = l_row > u_row || (l_row == u_row && l_col > u_col);
        if reverse {
            limit = i64::MAX;
            std::mem::swap(&mut l_row, &mut u_row);
            std::mem::swap(&mut l_col, &mut u_col);
        }

        let marks = extmark_get(
            b,
            ns_id as u32,
            l_row,
            l_col,
            u_row,
            u_col,
            limit,
            type_filter,
            opts.overlap,
        );

        rv = arena_array(arena, marks.len().min(rv_limit));
        if reverse {
            for &mark in marks.iter().rev() {
                if rv.size >= rv_limit {
                    break;
                }
                add_c(
                    &mut rv,
                    array_obj(extmark_to_array(mark, true, details, hl_name, arena)),
                );
            }
        } else {
            for &mark in &marks {
                add_c(
                    &mut rv,
                    array_obj(extmark_to_array(mark, true, details, hl_name, arena)),
                );
            }
        }

        rv
    }
}

/// Creates or updates an |extmark|.
///
/// By default a new extmark is created when no id is passed in, but it is also
/// possible to create a new mark by passing in a previously unused id or move
/// an existing mark by passing in its id. The caller must then keep track of
/// existing and unused ids itself. (Useful over RPC, to avoid waiting for the
/// return value.)
///
/// Using the optional arguments, it is possible to use this to highlight
/// a range of text, and also to associate virtual text to the mark.
///
/// If present, the position defined by `end_col` and `end_row` should be after
/// the start position in order for the extmark to cover a range.
/// An earlier end position is not an error, but then it behaves like an empty
/// range (no highlighting).
///
/// @param buf  Buffer id, or 0 for current buffer
/// @param ns_id  |namespace| id
/// @param line  Line where to place the mark, 0-based. |api-indexing|
/// @param col  Column where to place the mark, 0-based. |api-indexing|
/// @param opts  Optional parameters:
///               - conceal: (boolean|string)
///                   - string: Single char or empty string, enables conceal similar to |:syn-conceal|.
///                     Non-empty char is used as |:syn-cchar|. Highlighted with "hl_group" if
///                     defined, else defaults to |hl-Conceal|.
///                   - boolean: true is equivalent to "", false removes any existing conceal.
///               - conceal_lines: (string) Line-level conceal. When set to an empty string (other
///                 values reserved for future use), the lines in the extmark range are not drawn;
///                 the next non-concealed line is drawn in their place. Requires 'conceallevel' >=
///                 2; subject to 'concealcursor' on the cursor line.
///               - cursorline_hl_group: Highlight group used for the 'signcolumn' text when the
///                 cursor is on the same line as the mark and 'cursorline' is enabled.
///               - end_col: End-column of the mark, 0-based exclusive, or -1 to extend the range to
///                 end of line (if strict=false).
///               - end_right_gravity: (boolean, default: false) Indicates which direction the extmark
///                 end-position (if any) will be shifted when new text is inserted (true=right, false=left).
///               - end_row: End-line of the mark, 0-based inclusive.
///               - ephemeral: For use with |nvim_set_decoration_provider()| callbacks. The mark
///                 will only be used for the current redraw cycle, and not stored in the buffer.
///               - hl_eol: When true, for a multiline highlight covering the EOL of a line,
///                 continue the highlight for the rest of the screen line (just like for diff and
///                 cursorline highlight).
///               - hl_group: Highlight group for the text range. This and below highlight groups
///                 can be given as a string or as an integer (|nvim_get_hl_id_by_name()|). Multiple
///                 highlight groups can be stacked by passing an array (highest-priority last).
///               - hl_mode: Controls how highlights are combined with the highlights of the text.
///                 Only affects virt_text highlights, but might affect `hl_group` in the future.
///                 - "replace": Only show the virt_text color. This is the default.
///                 - "combine": Combine with background text color.
///                 - "blend": Blend with background text color. Not supported for "inline" virt_text.
///               - id: Update this extmark instead of creating a new one.
///               - invalidate: (boolean) Indicates whether to hide the extmark if the entirety of
///                 its range is deleted. For hidden marks, an "invalid" key is added to the
///                 "details" array of |nvim_buf_get_extmarks()| and family. If "undo_restore" is
///                 false, the extmark is deleted instead.
///               - line_hl_group: Highlight group used for the whole line.
///               - number_hl_group: Highlight group used for the number column.
///               - priority: Priority of the highlight group, sign attribute or virtual text. For
///                 virtual text, the highest-priority item is drawn last. For example treesitter
///                 highlighting uses a value of 100.
///               - right_gravity: (boolean, default: true) Indicates which direction the extmark
///                 will be shifted when new text is inserted (true=right, false=left).
///               - sign_hl_group: Highlight group for `sign_text`.
///               - sign_text: (string, length 1-2) Displayed in 'signcolumn'.
///               - spell: (boolean) Indicates that spell checking should be performed within this
///                 extmark.
///               - strict: (boolean, default: true) Indicates extmark should not be placed if the
///                 line or column value is past end-of-buffer or end-of-line respectively.
///               - ui_watched: (boolean) Indicates the mark should be drawn by a UI; activates
///                 `win_extmark` [ui-multigrid] events. Note: the mark is positioned by virt_text
///                 attributes. Can be used with virt_text.
///               - undo_restore: (default: true) Restore the exact position of the mark if text
///                 around the mark was deleted and then restored by undo.
///               - url: URL stored on this extmark. In the TUI, the OSC 8 control
///                   sequence is used to generate a clickable hyperlink to this URL.
///               - virt_lines: List of virtual lines to place next to the mark, where each line is
///                 a list of `[text_chunk, hl]` tuples. Buffer and window options do not affect the
///                 display of the text, except 'tabstop' is used for hard tabs. The 'wrap' and
///                 'linebreak' options do not take effect, so the number of extra screen lines will
///                 match the list-size (but see `virt_lines_overflow`). By default lines are placed
///                 below the marked line. Special case: If the last `text_chunk` is empty,
///                 the corresponding `hl` will extend to the end of the line.
///               - virt_lines_above: Place virtual lines above instead.
///               - virt_lines_leftcol: Place virtual lines in the leftmost column of the window,
///                 bypassing sign and number columns.
///               - virt_lines_overflow: Controls display of virtual lines exceeding the viewport:
///                 - "auto": Decided by the 'wrap' option.
///                 - "scroll": Scroll horizontally with 'nowrap', otherwise the same as "trunc".
///                 - "trunc": Truncate on the right (default).
///                 - "wrap": Always wrap (regardless of the 'wrap' option).
///               - virt_text: [](virtual-text) stored and displayed by this mark. List of `[text, hl]`
///                 tuples, each representing a text chunk with a highlight `hl` which may be
///                 a single highlight group or a list of stacked highlight groups (highest-priority
///                 last).
///               - virt_text_hide: Hide the virtual text when the background text is selected or
///                 hidden because of scrolling with 'nowrap' or 'smoothscroll'. Currently only
///                 affects "overlay" virt_text.
///               - virt_text_pos: Position of virtual text:
///                 - "eol": One cell after the line text or its displayed 'listchars' EOL character
///                   (default).
///                 - "eol_right_align": Display right-aligned in the window unless the virtual text
///                   is longer than the space available. If the virtual text is too long, it is
///                   truncated to fit in the window after the EOL character. If the line is
///                   wrapped, the virtual text is shown after the end of the line rather than the
///                   previous screen line.
///                 - "overlay": Display over the specified column, without shifting the underlying text.
///                 - "right_align": Display right-aligned in the window.
///                 - "inline": Display at the specified column, and right-shift the buffer text as needed.
///               - virt_text_repeat_linebreak: Repeat the virtual text on wrapped lines.
///               - virt_text_win_col: Position the virtual text at a fixed window column (starting
///                 from the first text column of the screen line) instead of "virt_text_pos".
///
/// @param[out]  err   Error details, if any
/// @return Id of the created/updated extmark
// DLLEXPORT Integer nvim_buf_set_extmark(Buffer buf, Integer ns_id, Integer line, Integer col,
//                                        Dict(set_extmark) *opts, Error *err)
//   FUNC_API_SINCE(7);
pub fn nvim_buf_set_extmark(
    buf: Buffer,
    ns_id: Integer,
    mut line: Integer,
    mut col: Integer,
    opts: &mut KeyDict_set_extmark,
    err: &mut Error,
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
            data: DecorVirtTextData::empty(),
            next: None,
        };
        let mut virt_lines = DecorVirtText {
            flags: kVTIsLines,
            hl_mode: kHlModeUnknown,
            priority: DECOR_PRIORITY_BASE,
            width: 0,
            col: 0,
            pos: kVPosEndOfLine,
            data: DecorVirtTextData::empty_lines(),
            next: None,
        };
        let mut url: Option<std::ffi::CString> = None;
        let mut has_hl = false;
        let mut has_hl_multiple = false;

        'error: {
            let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
                break 'error;
            };

            if !ns_initialized(ns_id as u32) {
                api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
                break 'error;
            }

            let mask = opts.is_set__set_extmark_;
            let mut id: u32 = 0;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__id) {
                if opts.id <= 0 {
                    api_err_exp(
                        err,
                        c"id",
                        c"positive Integer",
                        ptr::null(),
                    );
                    break 'error;
                }
                id = opts.id as u32;
            }

            let mut line2: c_int = -1;
            let mut did_end_line = false;

            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_line) {
                if has_key(mask, KEYSET_OPTIDX_set_extmark__end_row) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s",
                        c"cannot use both 'end_row' and 'end_line'",
                    );
                    break 'error;
                }
                opts.end_row = opts.end_line;
                did_end_line = true;
            }

            let strict = if has_key(mask, KEYSET_OPTIDX_set_extmark__strict) {
                opts.strict
            } else {
                true
            };

            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_row) || did_end_line {
                let val = opts.end_row;
                if val < 0 || (val > i64::from(b.b_ml.ml_line_count) && strict) {
                    api_err_range(err, c"end_row");
                    break 'error;
                }
                line2 = val as c_int;
            }

            let mut col2: colnr_T = -1;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__end_col) {
                let mut val = opts.end_col;
                if !(-1..=MAXCOL).contains(&val) {
                    api_err_range(err, c"end_col");
                    break 'error;
                }
                if val == -1 {
                    val = MAXCOL;
                }
                col2 = val as colnr_T;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__hl_group) {
                if opts.hl_group.obj_type == ObjectType::kObjectTypeArray {
                    let arr = opts.hl_group.data.array();
                    if arr.size >= 1 {
                        hl.hl_id =
                            object_to_hl_id(arr[0], c"hl_group item", err);
                        if error_set(err) {
                            break 'error;
                        }
                    }
                    for i in 1..arr.size {
                        let hl_id =
                            object_to_hl_id(arr[i], c"hl_group item", err);
                        if error_set(err) {
                            break 'error;
                        }
                        if hl_id != 0 {
                            has_hl_multiple = true;
                        }
                    }
                } else {
                    hl.hl_id = object_to_hl_id(opts.hl_group, c"hl_group", err);
                    if error_set(err) {
                        break 'error;
                    }
                }
                has_hl = hl.hl_id > 0;
            }

            sign.hl_id = opts.sign_hl_group as c_int;
            sign.cursorline_hl_id = opts.cursorline_hl_group as c_int;
            sign.number_hl_id = opts.number_hl_group as c_int;
            sign.line_hl_id = opts.line_hl_group as c_int;

            if sign.hl_id != 0
                || sign.cursorline_hl_id != 0
                || sign.number_hl_id != 0
                || sign.line_hl_id != 0
            {
                sign.flags |= kSHIsSign;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__conceal) {
                if opts.conceal.obj_type != ObjectType::kObjectTypeBoolean
                    && opts.conceal.obj_type != ObjectType::kObjectTypeString
                {
                    api_err_exp(
                        err,
                        c"conceal",
                        c"String or Boolean",
                        api_typename(opts.conceal.obj_type),
                    );
                    break 'error;
                }

                has_hl = true;
                if opts.conceal.obj_type == ObjectType::kObjectTypeBoolean {
                    hl.flags |= if opts.conceal.data.boolean() {
                        kSHConceal
                    } else {
                        kSHConcealOff
                    };
                } else {
                    let conceal_str = opts.conceal.data.string();
                    hl.flags |= kSHConceal;
                    if conceal_str.size > 0 {
                        let ci = utf_ptr2StrCharInfo(conceal_str.data);
                        let cli = utf_ClusterInfo(ci);
                        if ci.chr.value < 0x20 || cli.cells > 2 {
                            api_set_error!(
                                err,
                                ErrorType::kErrorTypeValidation,
                                c"%s",
                                c"conceal char must be printable",
                            );
                            break 'error;
                        }
                        hl.conceal_char = schar_from_cluster(ci, cli);
                    }
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__conceal_lines) {
                hl.flags |= kSHConcealLines;
                has_hl = true;
                if opts.conceal_lines.size > 0 && !crate::memory::cstr_is_empty(opts.conceal_lines.data) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s",
                        c"conceal_lines has to be an empty string",
                    );
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text) {
                virt_text.data = DecorVirtTextData::VirtText(parse_virt_text(
                    opts.virt_text,
                    err,
                    &mut virt_text.width,
                    false,
                ));
                if error_set(err) {
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text_pos) {
                let str_val = opts.virt_text_pos;
                if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"eol") {
                    virt_text.pos = kVPosEndOfLine;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"overlay") {
                    virt_text.pos = kVPosOverlay;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"right_align") {
                    virt_text.pos = kVPosRightAlign;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"eol_right_align") {
                    virt_text.pos = kVPosEndOfLineRightAlign;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"inline") {
                    virt_text.pos = kVPosInline;
                } else {
                    api_err_invalid(err, c"virt_text_pos", str_val.data, 0, true);
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_text_win_col) {
                virt_text.col = opts.virt_text_win_col as c_int;
                virt_text.pos = kVPosWinCol;
            }

            hl.flags |= if opts.hl_eol { kSHHlEol } else { 0 };
            virt_text.flags |= (if opts.virt_text_hide { kVTHide } else { 0 })
                | (if opts.virt_text_repeat_linebreak {
                    kVTRepeatLinebreak
                } else {
                    0
                });

            if has_key(mask, KEYSET_OPTIDX_set_extmark__hl_mode) {
                let str_val = opts.hl_mode;
                if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"replace") {
                    virt_text.hl_mode = kHlModeReplace;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"combine") {
                    virt_text.hl_mode = kHlModeCombine;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"blend") {
                    if virt_text.pos == kVPosInline {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"%s",
                            c"cannot use 'blend' hl_mode with inline virtual text",
                        );
                        break 'error;
                    }
                    virt_text.hl_mode = kHlModeBlend;
                } else {
                    api_err_invalid(err, c"hl_mode", str_val.data, 0, true);
                    break 'error;
                }
            }

            let virt_lines_flags = if opts.virt_lines_leftcol {
                kVLLeftcol
            } else {
                0
            };
            let mut virt_lines_overflow = kVLOverflowTrunc;
            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_lines_overflow) {
                let str_val = opts.virt_lines_overflow;
                if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"scroll") {
                    virt_lines_overflow = kVLOverflowScroll;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"wrap") {
                    virt_lines_overflow = kVLOverflowWrap;
                } else if crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"auto") {
                    virt_lines_overflow = kVLOverflowAuto;
                } else if !(crate::memory::cstr_as_opt_bytes(str_val.data) == Some(b"trunc")) {
                    api_err_invalid(err, c"virt_lines_overflow", str_val.data, 0, true);
                    break 'error;
                }
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__virt_lines) {
                let a = opts.virt_lines;
                for j in 0..a.size {
                    let item = a[j];
                    if item.obj_type != ObjectType::kObjectTypeArray {
                        api_err_exp(
                            err,
                            c"virt_text_line",
                            api_typename(ObjectType::kObjectTypeArray),
                            api_typename(item.obj_type),
                        );
                        break 'error;
                    }
                    let mut dummig: c_int = 0;
                    let jtem = parse_virt_text(item.data.array(), err, &mut dummig, false);
                    virt_lines.data.virt_lines_mut().push(VirtLine {
                        line: jtem,
                        flags: virt_lines_flags,
                        overflow: virt_lines_overflow,
                    });
                    if error_set(err) {
                        break 'error;
                    }
                }
            }

            virt_lines.flags |= if opts.virt_lines_above {
                kVTLinesAbove
            } else {
                0
            };

            if has_key(mask, KEYSET_OPTIDX_set_extmark__priority) {
                if !((0..=i64::from(u16::MAX)).contains(&opts.priority)) {
                    api_err_range(err, c"priority");
                    break 'error;
                }
                let prio = opts.priority as DecorPriority;
                hl.priority = prio;
                sign.priority = prio;
                virt_text.priority = prio;
                virt_lines.priority = prio;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__sign_text) {
                sign.text[0] = 0;
                if !init_sign_text(
                    None,
                    sign.text.as_mut_ptr(),
                    opts.sign_text.data,
                )
                {
                    api_err_invalid(err, c"sign_text", c"", 0, true);
                    break 'error;
                }
                sign.flags |= kSHIsSign;
            }

            let right_gravity = if has_key(mask, KEYSET_OPTIDX_set_extmark__right_gravity) {
                opts.right_gravity
            } else {
                true
            };

            if line2 == -1
                && col2 == -1
                && has_key(mask, KEYSET_OPTIDX_set_extmark__end_right_gravity)
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s",
                    c"cannot set end_right_gravity without end_row or end_col",
                );
                break 'error;
            }

            let mut len: colnr_T = 0;

            if has_key(mask, KEYSET_OPTIDX_set_extmark__spell) {
                hl.flags |= if opts.spell {
                    kSHSpellOn
                } else {
                    kSHSpellOff
                };
                has_hl = true;
            }

            if has_key(mask, KEYSET_OPTIDX_set_extmark__url) {
                url = Some(string_to_cstr(opts.url));
                has_hl = true;
            }

            if opts.ui_watched {
                hl.flags |= kSHUIWatched;
                if virt_text.pos == kVPosOverlay {
                    hl.flags |= kSHUIWatchedOverlay;
                }
                has_hl = true;
            }

            if line < 0 {
                api_err_range(err, c"line");
                break 'error;
            }

            if line > i64::from(b.b_ml.ml_line_count) {
                if strict {
                    api_err_range(err, c"line");
                    break 'error;
                }
                line = i64::from(b.b_ml.ml_line_count);
            } else if line < i64::from(b.b_ml.ml_line_count) {
                len = if opts.ephemeral {
                    MAXCOL as colnr_T
                } else {
                    ml_get_buf_len(&mut *b, (line as linenr_T) + 1)
                };
            }

            if col == -1 {
                col = i64::from(len);
            } else if col > i64::from(len) {
                if strict {
                    api_err_range(err, c"col");
                    break 'error;
                }
                col = i64::from(len);
            } else if col < -1 {
                api_err_range(err, c"col");
                break 'error;
            }

            if col2 >= 0 {
                if line2 >= 0 && line2 < b.b_ml.ml_line_count {
                    len = if opts.ephemeral {
                        MAXCOL as colnr_T
                    } else {
                        ml_get_buf_len(&mut *b, line2 + 1)
                    };
                } else if line2 == b.b_ml.ml_line_count {
                    len = 0;
                } else {
                    line2 = line as c_int;
                }
                if col2 > len {
                    if strict {
                        api_err_range(err, c"end_col");
                        break 'error;
                    }
                    col2 = len;
                }
            } else if line2 >= 0 {
                col2 = 0;
            }

            if opts.ephemeral && !g_st().decor_state.win.is_null() && (*g_st().decor_state.win).w_buffer == (&raw mut *b) {
                let r = line as c_int;
                let c = col as c_int;
                if line2 == -1 {
                    line2 = r;
                    col2 = c;
                }

                let mut subpriority: DecorPriority = 0;
                if has_key(mask, KEYSET_OPTIDX_set_extmark___subpriority) {
                    if !((0..=i64::from(u16::MAX)).contains(&opts._subpriority)) {
                        api_err_range(err, c"_subpriority");
                        break 'error;
                    }
                    subpriority = opts._subpriority as DecorPriority;
                }

                if !virt_text.data.virt_text().is_empty() {
                    decor_range_add_virt(
                        &mut g_st().decor_state,
                        r,
                        c,
                        line2,
                        col2,
                        decor_put_vt(virt_text, ptr::null_mut()),
                        true,
                    );
                }
                if !virt_lines.data.virt_lines().is_empty() {
                    decor_range_add_virt(
                        &mut g_st().decor_state,
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
                    sh.url = url.take();
                    decor_range_add_sh(
                        &mut g_st().decor_state,
                        r,
                        c,
                        line2,
                        col2,
                        &sh,
                        true,
                        ns_id as u32,
                        id,
                        subpriority,
                    );
                }
            } else {
                if opts.ephemeral {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"cannot set emphemeral mark outside of a decoration provider",
                    );
                    break 'error;
                }

                let mut decor_flags: u16 = 0;
                let mut decor_alloc: *mut DecorVirtText = ptr::null_mut();
                if !virt_text.data.virt_text().is_empty() {
                    if virt_text.pos == kVPosInline {
                        decor_flags |= MT_FLAG_DECOR_VIRT_TEXT_INLINE;
                    }
                    decor_alloc = decor_put_vt(virt_text, decor_alloc);
                }
                if !virt_lines.data.virt_lines().is_empty() {
                    decor_alloc = decor_put_vt(virt_lines, decor_alloc);
                    decor_flags |= MT_FLAG_DECOR_VIRT_LINES;
                }

                let mut decor_indexed: u32 = DECOR_ID_INVALID;

                if (sign.flags & kSHIsSign) != 0 {
                    if sign.text[0] != 0 {
                        decor_flags |= MT_FLAG_DECOR_SIGNTEXT;
                    }
                    if sign.number_hl_id != 0 || sign.line_hl_id != 0 || sign.cursorline_hl_id != 0
                    {
                        decor_flags |= MT_FLAG_DECOR_SIGNHL;
                    }
                    sign.next = decor_indexed;
                    decor_indexed = decor_put_sh(sign);
                }

                if has_hl_multiple {
                    let arr = opts.hl_group.data.array();
                    let mut i = arr.size - 1;
                    while i > 0 {
                        let hl_id =
                            object_to_hl_id(arr[i], c"hl_group item", err);
                        if hl_id > 0 {
                            let mut sh = DECOR_SIGN_HIGHLIGHT_INIT;
                            sh.hl_id = hl_id;
                            sh.flags = if opts.hl_eol { kSHHlEol } else { 0 };
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
                    data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
                };
                if !decor_alloc.is_null()
                    || decor_indexed != DECOR_ID_INVALID
                    || url.is_some()
                    || schar_high(hl.conceal_char)
                {
                    if has_hl {
                        let mut sh = decor_sh_from_inline(hl);
                        sh.url = url.take();
                        sh.next = decor_indexed;
                        decor_indexed = decor_put_sh(sh);
                    }
                    decor.data = DecorInlineData::Ext(DecorExt {
                        sh_idx: decor_indexed,
                        vt: decor_alloc.cast(),
                    });
                } else {
                    decor.data = DecorInlineData::Hl(hl);
                }

                if has_hl {
                    decor_flags |= MT_FLAG_DECOR_HL;
                }

                let undo_restore = if has_key(mask, KEYSET_OPTIDX_set_extmark__undo_restore) {
                    opts.undo_restore
                } else {
                    true
                };

                extmark_set(
                    b,
                    ns_id as u32,
                    Some(&mut id),
                    line as c_int,
                    col as colnr_T,
                    line2,
                    col2,
                    decor,
                    decor_flags,
                    right_gravity,
                    opts.end_right_gravity,
                    !undo_restore,
                    opts.invalidate,
                );
                if error_set(err) {
                    decor_free(decor);
                    return 0;
                }
            }

            return i64::from(id);
        }

        0
    }
}

/// Removes an |extmark|.
///
/// @param buf Buffer id, or 0 for current buffer
/// @param ns_id |namespace| id
/// @param id Extmark id
/// @param[out] err   Error details, if any
/// @return true if the extmark was found, else false
// DLLEXPORT Boolean nvim_buf_del_extmark(Buffer buf, Integer ns_id, Integer id, Error *err)
//   FUNC_API_SINCE(7);
pub fn nvim_buf_del_extmark(
    buf: Buffer,
    ns_id: Integer,
    id: Integer,
    err: &mut Error,
) -> Boolean {
     {
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return false;
        };
        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
            return false;
        }
        extmark_del_id(b, ns_id as u32, id as u32)
    }
}

/// Clears |namespace|d objects (highlights, |extmarks|, virtual text) from
/// a region.
///
/// Lines are 0-indexed. |api-indexing|  To clear the namespace in the entire
/// buffer, specify line_start=0 and line_end=-1.
///
/// @param buf     Buffer id, or 0 for current buffer
/// @param ns_id      Namespace to clear, or -1 to clear all namespaces.
/// @param line_start Start of range of lines to clear
/// @param line_end   End of range of lines to clear (exclusive) or -1 to clear
///                   to end of buffer.
/// @param[out] err   Error details, if any
// DLLEXPORT void nvim_buf_clear_namespace(Buffer buf, Integer ns_id, Integer line_start,
//                                         Integer line_end, Error *err)
//   FUNC_API_SINCE(5);
pub fn nvim_buf_clear_namespace(
    buf: Buffer,
    ns_id: Integer,
    line_start: Integer,
    mut line_end: Integer,
    err: &mut Error,
) {
    let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return;
        };

        if !(0..MAXLNUM).contains(&line_start) {
            api_err_range(err, c"line number");
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

/// Set or change decoration provider for a |namespace|
///
/// This is a very general purpose interface for having Lua callbacks
/// being triggered during the redraw code.
///
/// The expected usage is to set |extmarks| for the currently
/// redrawn buffer. |nvim_buf_set_extmark()| can be called to add marks
/// on a per-window or per-lines basis. Use the `ephemeral` key to only
/// use the mark for the current screen redraw (the callback will be called
/// again for the next redraw).
///
/// Note: this function should not be called often. Rather, the callbacks
/// themselves can be used to throttle unneeded callbacks. the `on_start`
/// callback can return `false` to disable the provider until the next redraw.
/// Similarly, return `false` in `on_win` will skip the `on_line` and `on_range` calls
/// for that window (but any extmarks set in `on_win` will still be used).
/// A plugin managing multiple sources of decoration should ideally only set
/// one provider, and merge the sources internally. You can use multiple `ns_id`
/// for the extmarks set/modified inside the callback anyway.
///
/// Note: doing anything other than setting extmarks is considered experimental.
/// Doing things like changing options are not explicitly forbidden, but is
/// likely to have unexpected consequences (such as 100% CPU consumption).
/// Doing `vim.rpcnotify` should be OK, but `vim.rpcrequest` is quite dubious
/// for the moment.
///
/// Note: It is not allowed to remove or update extmarks in `on_line` or `on_range` callbacks.
///
/// Note: Callbacks will not run for |window-hidden|, as they are not redrawn.
///
/// @param ns_id  |namespace| id
/// @param opts  Table of callbacks:
///             - on_buf: called for each buffer being redrawn (once per edit,
///               before window callbacks)
///               ```
///                 ["buf", bufnr, tick]
///               ```
///             - on_end: called at the end of a redraw cycle
///               ```
///                 ["end", tick]
///               ```
///             - on_line: (deprecated, use on_range instead)
///               ```
///                 ["line", winid, bufnr, row]
///               ```
///             - on_range: called for each buffer range being redrawn.
///               Range is end-exclusive and may span multiple lines. Range
///               bounds point to the first byte of a character. An end position
///               of the form (lnum, 0), including (number of lines, 0), is valid
///               and indicates that EOL of the preceding line is included.
///               ```
///                 ["range", winid, bufnr, begin_row, begin_col, end_row, end_col]
///               ```
///
///               In addition to returning a boolean, it is also allowed to
///               return a `skip_row, skip_col` pair of integers. This implies
///               that this function does not need to be called until a range
///               which continues beyond the skipped position. A single integer
///               return value `skip_row` is short for `skip_row, 0`
///
///             - on_start: called first on each screen redraw
///               ```
///                 ["start", tick]
///               ```
///             - on_win: called when starting to redraw a specific window.
///               ```
///                 ["win", winid, bufnr, toprow, botrow]
///               ```
// DLLEXPORT void nvim_set_decoration_provider(Integer ns_id, Dict(set_decoration_provider) *opts,
//                                             Error *err)
//   FUNC_API_SINCE(7) FUNC_API_LUA_ONLY;
pub fn nvim_set_decoration_provider(
    ns_id: Integer,
    opts: &mut KeyDict_set_decoration_provider,
    _err: &mut Error,
) {
    unsafe {
        let p = get_decor_provider(ns_id as NS, true);
        assert!(!p.is_null());
        decor_provider_clear(p.as_mut());

        redraw_all_later(UPD_NOT_VALID);

        let pairs: [(*mut LuaRef, *mut LuaRef); 9] = [
            (&raw mut opts.on_start, &raw mut (*p).redraw_start),
            (&raw mut opts.on_buf, &raw mut (*p).redraw_buf),
            (&raw mut opts.on_win, &raw mut (*p).redraw_win),
            (&raw mut opts.on_line, &raw mut (*p).redraw_line),
            (&raw mut opts.on_range, &raw mut (*p).redraw_range),
            (&raw mut opts.on_end, &raw mut (*p).redraw_end),
            (&raw mut opts._on_hl_def, &raw mut (*p).hl_def),
            (&raw mut opts._on_spell_nav, &raw mut (*p).spell_nav),
            (
                &raw mut opts._on_conceal_line,
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

fn extmark_get_index_from_obj(
    buf: &mut buf_T,
    ns_id: Integer,
    obj: Object,
    row: &mut c_int,
    col: &mut colnr_T,
    err: &mut Error,
) -> bool {
     {
        if obj.obj_type == ObjectType::kObjectTypeInteger {
            let id = obj.data.integer();
            if id == 0 {
                *row = 0;
                *col = 0;
                return true;
            } else if id == -1 {
                *row = MAXLNUM as c_int;
                *col = MAXCOL as colnr_T;
                return true;
            } else if id < 0 {
                api_err_invalid(err, c"mark id", ptr::null(), id, false);
                return false;
            }

            let extmark = extmark_from_id(buf, ns_id as u32, id as u32);
            if extmark.start.pos.row < 0 {
                api_err_invalid(err, c"mark id (not found)", ptr::null(), id, false);
                return false;
            }
            *row = extmark.start.pos.row;
            *col = extmark.start.pos.col;
            true
        } else if obj.obj_type == ObjectType::kObjectTypeArray {
            let pos = obj.data.array();
            if pos.size != 2
                || pos[0].obj_type != ObjectType::kObjectTypeInteger
                || pos[1].obj_type != ObjectType::kObjectTypeInteger
            {
                api_err_exp(
                    err,
                    c"mark position",
                    c"2 Integer items",
                    ptr::null(),
                );
                return false;
            }

            let pos_row = pos[0].data.integer();
            let pos_col = pos[1].data.integer();
            *row = (if pos_row >= 0 { pos_row } else { MAXLNUM }) as c_int;
            *col = (if pos_col >= 0 { pos_col } else { MAXCOL }) as colnr_T;
            true
        } else {
            api_err_exp(
                err,
                c"mark position",
                c"mark id Integer or 2-item Array",
                ptr::null(),
            );
            false
        }
    }
}

// DLLEXPORT VirtText parse_virt_text(Array chunks, Error *err, int *width, bool untab);
pub fn parse_virt_text(
    chunks: Array,
    err: &mut Error,
    width: *mut c_int,
    untab: bool,
) -> VirtText {
    unsafe {
        let mut virt_text = VirtText::new();
        let mut w: c_int = 0;

        'free_exit: {
            for i in 0..chunks.size {
                let item = chunks[i];
                if item.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"chunk",
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(item.obj_type),
                    );
                    break 'free_exit;
                }
                let chunk = item.data.array();
                if chunk.size == 0
                    || chunk.size > 2
                    || chunk[0].obj_type != ObjectType::kObjectTypeString
                {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s",
                        c"Invalid chunk: expected Array with 1 or 2 Strings",
                    );
                    break 'free_exit;
                }

                let str_val = chunk[0].data.string();
                let mut hl_id: c_int = -1;
                if chunk.size == 2 {
                    let hl = chunk[1];
                    if hl.obj_type == ObjectType::kObjectTypeArray {
                        let arr = hl.data.array();
                        for j in 0..arr.size {
                            hl_id = object_to_hl_id(
                                arr[j],
                                c"virt_text highlight",
                                err,
                            );
                            if error_set(err) {
                                break 'free_exit;
                            }
                            if j < arr.size - 1 {
                                virt_text.push(VirtTextChunk { text: None, hl_id });
                            }
                        }
                    } else {
                        hl_id = object_to_hl_id(hl, c"virt_text highlight", err);
                        if error_set(err) {
                            break 'free_exit;
                        }
                    }
                }

                let text_cstr = transstr(
                    if str_val.size > 0 {
                        str_val.data
                    } else {
                        c"".as_ptr()
                    },
                    untab,
                );
                w += mb_string2cells(text_cstr.as_ptr()) as c_int;

                virt_text.push(VirtTextChunk {
                    text: Some(text_cstr),
                    hl_id,
                });
            }

            if !width.is_null() {
                *width = w;
            }
            return virt_text;
        }

        virt_text.clear();
        virt_text
    }
}

/// @nodoc
// DLLEXPORT String nvim__buf_debug_extmarks(Buffer buf, Boolean keys, Boolean dot, Error *err)
//   FUNC_API_SINCE(7) FUNC_API_RET_ALLOC;
pub fn nvim__buf_debug_extmarks(
    buf: Buffer,
    keys: Boolean,
    dot: Boolean,
    err: &mut Error,
) -> NvimString {
     {
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle_mut(buf, err) else {
            return NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
        };
        mt_inspect(&b.b_marktree, keys, dot)
    }
}

/// Set some properties for namespace
///
/// @param ns_id Namespace
/// @param opts Optional parameters to set:
///           - wins: a list of windows to be scoped in
///
// DLLEXPORT void nvim__ns_set(Integer ns_id, Dict(ns_opts) *opts, Error *err);
pub fn nvim__ns_set(ns_id: Integer, opts: &mut KeyDict_ns_opts, err: &mut Error) {
    unsafe {
        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
            return;
        }

        let mut set_scoped = true;

        if has_key(opts.is_set__ns_opts_, KEYSET_OPTIDX_ns_opts__wins) {
            if opts.wins.size == 0 {
                set_scoped = false;
            }

            let mut windows: std::collections::HashSet<*mut win_T> =
                std::collections::HashSet::new();
            for i in 0..opts.wins.size {
                let win = (*opts.wins.items.add(i)).data.integer();
                let wp = find_window_by_handle(win as Window, err);
                if wp.is_null() {
                    return;
                }
                windows.insert(wp);
            }

            for tp in for_all_tabpages_ptr() {
                for wp in for_all_windows_in_tab_ptr( &*tp) {
                    if windows.contains(&wp)
                        && (*wp).w_ns_set.insert(ns_id as u32)
                        && (*wp).buf_mut().b_extmark_ns.contains_key(&(ns_id as u32))
                    {
                        changed_window_setting(&mut *wp);
                    }

                    if !windows.contains(&wp)
                        && (*wp).w_ns_set.remove(&(ns_id as u32))
                        && (*wp).buf_mut().b_extmark_ns.contains_key(&(ns_id as u32))
                    {
                        changed_window_setting(&mut *wp);
                    }
                }
            }
        }

        let local_ptr = &raw mut g_st().namespace_localscope;
        if (set_scoped && (*local_ptr).insert(ns_id as u32))
            || (!set_scoped && (*local_ptr).remove(&(ns_id as u32)))
        {
            for tp in for_all_tabpages_ptr() {
                for wp in for_all_windows_in_tab_mut( &*tp) {
                    if wp.buf_mut().b_extmark_ns.contains_key(&(ns_id as u32)) {
                        changed_window_setting(wp);
                    }
                }
            }
        }
    }
}

/// Get the properties for namespace
///
/// @param ns_id Namespace
/// @return  Map defining the namespace properties, see |nvim__ns_set()|
// DLLEXPORT Dict(ns_opts) nvim__ns_get(Integer ns_id, Arena *arena, Error *err);
pub fn nvim__ns_get(
    ns_id: Integer,
    arena: &mut Arena,
    err: &mut Error,
) -> KeyDict_ns_opts {
    unsafe {
        let mut opts = KeyDict_ns_opts {
            is_set__ns_opts_: 1u64 << KEYSET_OPTIDX_ns_opts__wins,
            wins: Array::new(),
        };

        if !ns_initialized(ns_id as u32) {
            api_err_invalid(err, c"ns_id", ptr::null(), ns_id, false);
            return opts;
        }

        let local_ptr = &raw const g_st().namespace_localscope;
        if !(*local_ptr).contains(&(ns_id as u32)) {
            return opts;
        }

        let mut count = 0usize;
        let mut tp = g_st().first_tabpage;
        while !tp.is_null() {
            for wp in for_all_windows_in_tab_mut( &*tp) {
                if wp.w_ns_set.contains(&(ns_id as u32)) {
                    count += 1;
                }
            }
            tp = (*tp).tp_next;
        }

        let mut windows = arena_array(arena, count);

        tp = g_st().first_tabpage;
        while !tp.is_null() {
            for wp in for_all_windows_in_tab_mut( &*tp) {
                if wp.w_ns_set.contains(&(ns_id as u32)) {
                    windows.push(integer_obj(i64::from(wp.handle)));
                }
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
    use crate::marktree::{
        MT_FLAG_INVALIDATE, MT_FLAG_PAIRED, MT_FLAG_RIGHT_GRAVITY, MTKey, MTPos,
    };

    #[test]
    fn mt_key_flag_predicates() {
        let key = MTKey {
            pos: MTPos { row: 1, col: 2 },
            ns: 3,
            id: 4,
            flags: MT_FLAG_RIGHT_GRAVITY | MT_FLAG_PAIRED | MT_FLAG_INVALIDATE,
            decor_data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
        };
        assert!(mt_right(key));
        assert!(mt_paired(key));
        assert!(mt_invalidate(key));
        assert!(!mt_no_undo(key));
        assert!(!mt_invalid(key));
    }
}
