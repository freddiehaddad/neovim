//! Vimscript `typval_T`, `list_T`, `dict_T`, `blob_T`, and `Callback` operations (`src/nvim/eval/typval.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::errors::*;
#[allow(unused_imports)]
use crate::eval::userfunc::{call_func, get_funccal_local_ht};
#[allow(unused_imports)]
use crate::eval::{
    callback_call, func_equal, get_copyID, partial_unref, set_selfdict, var_item_copy, var2fpos,
};
#[allow(unused_imports)]
use crate::eval::{callback_from_typval, partial_name};
use crate::globals::*;
#[allow(unused_imports)]
use crate::lua::executor::nlua_funcref_str;
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, api_new_luaref};
use crate::message::semsg;
#[allow(unused_imports)]
use crate::message::{emsg, internal_error};
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::strings::vim_stricmp;
#[allow(unused_imports)]
use crate::strings::{gettext, qsort, strcmp, strncmp};
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::mem::offset_of;
use std::ptr;

use crate::api::private::converter::{partial_T, ufunc_T};
use crate::api::tabpage::win_T;
use crate::charset::vim_str2nr;
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::cursor::linenr_T;
use crate::eval::decode::{dict_T, dictitem_T};
use crate::eval::deprecated::{Callback, EvalFuncData, buf_T, pos_T};
use crate::eval::encode::{
    encode_bool_var_names, encode_special_var_names, encode_tv2echo, encode_tv2string,
};
use crate::eval::executor::{blob_T, eexe_mod_op};
use crate::eval::gc::{gc_first_dict, gc_first_list};
use crate::eval::userfunc::{func_ref, func_unref, funcexe_T};
use crate::eval::vars::{
    get_globvar_dict, valid_varname, var_check_fixed, var_check_ro, var_wrong_func_name,
};
use crate::garray::{ga_append, ga_clear, ga_concat_len, ga_grow, ga_init, garray_T};
use crate::hashtab::{
    hash_add, hash_clear, hash_find, hash_find_len, hash_init, hash_lock, hash_remove, hash_unlock,
    hashitem_T, hashitem_empty, hashtab_T,
};
use crate::lua::converter::LuaRef;
use crate::map_glyph_cache::NvimString;
use crate::mbyte::{mb_strcmp_ic, string_convert, utf_char2bytes, utfc_ptr2len, vimconv_T};
use crate::memory::{
    Arena, strequal, xcalloc, xfree, xmalloc, xmallocz, xmemdup, xmemdupz, xrealloc, xstrdup,
    xstrlcpy, xstrndup,
};
use crate::os::input::{fast_breakcheck, line_breakcheck};

pub use crate::eval::buffer::varnumber_T;
pub type float_T = f64;
pub type VarLockStatus = c_int;
pub type BoolVarValue = c_int;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const NOTDONE: c_int = 2;

const DO_NOT_FREE_CNT: c_int = c_int::MAX / 2;
const DICT_MAXNEST: c_int = 100;
const NUMBUFLEN: usize = 65;
const MB_MAXBYTES: usize = 30;
const LUA_NOREF: LuaRef = -2;
const ITEM_COMPARE_FAIL: c_int = 999;

const VAR_UNLOCKED: VarLockStatus = 0;
const VAR_LOCKED: VarLockStatus = 1;
const VAR_FIXED: VarLockStatus = 2;

const VAR_NO_SCOPE: c_int = 0;

const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;
const kSpecialVarNull: c_int = 0;

const kListLenMayKnow: isize = -3;

const DI_FLAGS_RO: u8 = 1;
const DI_FLAGS_FIX: u8 = 4;
const DI_FLAGS_ALLOC: u8 = 16;

const CONV_NONE: c_int = 0;
const STR2NR_ALL: c_int = 0x7f;

pub const TV_TRANSLATE: usize = usize::MAX;
pub const TV_CSTRING: usize = usize::MAX - 1;

const kCallbackNone: c_int = 0;
const kCallbackFuncref: c_int = 1;
const kCallbackPartial: c_int = 2;
const kCallbackLua: c_int = 3;
const kCallbackExpr: c_int = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DictListType {
    Keys,
    Values,
    Items,
}

#[repr(C)]
pub struct listwatch_T {
    pub lw_item: *mut listitem_T,
    pub lw_next: *mut listwatch_T,
}

#[repr(C)]
pub struct staticList10_T {
    pub sl_list: list_T,
    pub sl_items: [listitem_T; 10],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct QUEUE {
    pub next: *mut QUEUE,
    pub prev: *mut QUEUE,
}

#[repr(C)]
pub struct DictWatcher {
    pub callback: Callback,
    pub key_pattern: *mut c_char,
    pub key_pattern_len: usize,
    pub node: QUEUE,
    pub busy: bool,
    pub needs_free: bool,
}

#[repr(C)]
struct sortinfo_T {
    item_compare_ic: c_int,
    item_compare_lc: bool,
    item_compare_numeric: bool,
    item_compare_numbers: bool,
    item_compare_float: bool,
    item_compare_func: *const c_char,
    item_compare_partial: *mut partial_T,
    item_compare_selfdict: *mut dict_T,
    item_compare_func_err: bool,
    item_compare_keys_ready: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
union ListSortKey {
    inum: varnumber_T,
    fnum: f64,
}

#[repr(C)]
struct ListSortItem {
    item: *mut listitem_T,
    idx: c_int,
    key: ListSortKey,
}

type ListSorter = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

#[repr(C)]
struct Join {
    s: NvimString,
    tofree: *mut c_char,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MPConvStackValType {
    Dict,
    List,
    Partial,
    PartialList,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MPConvPartialStage {
    Args,
    SelfDict,
    End,
}

#[derive(Clone, Copy)]
enum MPConvStackData {
    Dict {
        dict: *mut dict_T,
        dictp: *mut *mut dict_T,
        hi: *mut hashitem_T,
        todo: usize,
    },
    List {
        list: *mut list_T,
        li: *mut listitem_T,
    },
    Partial {
        stage: MPConvPartialStage,
        pt: *mut partial_T,
    },
    PartialList {
        arg: *mut typval_T,
        argv: *mut typval_T,
        todo: usize,
    },
}

#[derive(Clone, Copy)]
struct MPConvStackVal {
    val_type: MPConvStackValType,
    tv: *mut typval_T,
    saved_copy_id: c_int,
    data: MPConvStackData,
}

#[repr(transparent)]
pub struct SyncCStrPtr(pub *const c_char);
unsafe impl Sync for SyncCStrPtr {}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut tv_in_free_unref_items: bool = false;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static tv_empty_string: SyncCStrPtr = SyncCStrPtr(c"".as_ptr());

static mut SORTINFO: *mut sortinfo_T = ptr::null_mut();
static mut TV_EQUAL_RECURSE_LIMIT: c_int = 0;

static E_VARIABLE_NESTED_TOO_DEEP_FOR_UNLOCK: &CStr =
    c"E743: Variable nested too deep for (un)lock";
static E_USING_INVALID_VALUE_AS_STRING: &CStr = c"E908: Using an invalid value as a String";
static E_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1174: String required for argument %d";
static E_NON_EMPTY_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1175: Non-empty string required for argument %d";
static E_DICT_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1206: Dictionary required for argument %d";
static E_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1210: Number required for argument %d";
static E_LIST_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1211: List required for argument %d";
static E_BOOL_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1212: Bool required for argument %d";
static E_FLOAT_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1219: Float or Number required for argument %d";
static E_STRING_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1220: String or Number required for argument %d";
static E_STRING_OR_LIST_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1222: String or List required for argument %d";
static E_LIST_DICT_BLOB_OR_STRING_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1225: List, Dictionary, Blob or String required for argument %d";
static E_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1226: List or Blob required for argument %d";
static E_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr = c"E1238: Blob required for argument %d";
static E_STRING_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1252: String, List or Blob required for argument %d";
static E_STRING_OR_FUNCTION_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1256: String or function required for argument %d";
static E_NON_NULL_DICT_REQUIRED_FOR_ARGUMENT_NR: &CStr =
    c"E1297: Non-NULL Dictionary required for argument %d";

unsafe extern "C" {
    fn strcoll(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strtod(nptr: *const c_char, endptr: *mut *mut c_char) -> f64;
}

unsafe extern "C" {

    #[cfg(not(windows))]
    fn strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;

}

#[inline]
unsafe fn tr(s: *const c_char) -> *mut c_char {
    #[cfg(not(test))]
    unsafe {
        gettext(s)
    }
    #[cfg(test)]
    {
        s.cast_mut()
    }
}

#[inline]
unsafe fn c_strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

#[inline]
unsafe fn nlua_clear_ref(r: &mut LuaRef) {
    #[cfg(not(test))]
    if *r != LUA_NOREF {
        unsafe { api_free_luaref(*r) };
        *r = LUA_NOREF;
    }
    #[cfg(test)]
    {
        *r = LUA_NOREF;
    }
}

#[inline]
pub(crate) unsafe fn tv_dict_hi2di(hi: *const hashitem_T) -> *mut dictitem_T {
    unsafe {
        (*hi)
            .hi_key
            .sub(offset_of!(dictitem_T, di_key))
            .cast::<dictitem_T>()
    }
}

#[inline]
unsafe fn dict_watchers_queue(d: *const dict_T) -> *mut QUEUE {
    unsafe { (&raw const (*d).watchers).cast_mut().cast::<QUEUE>() }
}

#[inline]
unsafe fn queue_empty(q: *const QUEUE) -> bool {
    unsafe { ptr::eq(q, (*q).next) }
}

#[inline]
unsafe fn queue_init(q: *mut QUEUE) {
    unsafe {
        (*q).next = q;
        (*q).prev = q;
    }
}

#[inline]
unsafe fn queue_insert_tail(h: *mut QUEUE, q: *mut QUEUE) {
    unsafe {
        (*q).next = h;
        (*q).prev = (*h).prev;
        (*(*q).prev).next = q;
        (*h).prev = q;
    }
}

#[inline]
unsafe fn queue_remove(q: *mut QUEUE) {
    unsafe {
        (*(*q).prev).next = (*q).next;
        (*(*q).next).prev = (*q).prev;
    }
}

#[inline]
unsafe fn tv_dict_watcher_node_data(q: *mut QUEUE) -> *mut DictWatcher {
    unsafe {
        q.cast::<u8>()
            .sub(offset_of!(DictWatcher, node))
            .cast::<DictWatcher>()
    }
}

#[inline]
unsafe fn tv_list_ref_inline(l: *mut list_T) {
    if !l.is_null() {
        unsafe {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_list_set_ret_inline(tv: *mut typval_T, l: *mut list_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).vval.v_list = l;
        tv_list_ref_inline(l);
    }
}

#[inline]
unsafe fn tv_list_locked_inline(l: *const list_T) -> VarLockStatus {
    if l.is_null() {
        VAR_FIXED
    } else {
        unsafe { (*l).lv_lock }
    }
}

#[inline]
unsafe fn tv_list_set_lock_inline(l: *mut list_T, lock: VarLockStatus) {
    if l.is_null() {
        debug_assert_eq!(lock, VAR_FIXED);
        return;
    }
    unsafe {
        (*l).lv_lock = lock;
    }
}

#[inline]
unsafe fn tv_list_len_inline(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_uidx_inline(l: *const list_T, mut n: c_int) -> c_int {
    unsafe {
        let len = tv_list_len_inline(l);
        if n < 0 {
            n += len;
        }
        if n < 0 || n >= len { -1 } else { n }
    }
}

#[inline]
unsafe fn tv_list_first_inline(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

#[inline]
unsafe fn tv_list_last_inline(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_last }
    }
}

#[inline]
unsafe fn tv_dict_set_ret_inline(tv: *mut typval_T, d: *mut dict_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval.v_dict = d;
        if !d.is_null() {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_dict_len_inline(d: *const dict_T) -> isize {
    if d.is_null() {
        0
    } else {
        unsafe { (*d).dv_hashtab.ht_used as isize }
    }
}

#[inline]
unsafe fn tv_dict_is_watched_inline(d: *const dict_T) -> bool {
    !d.is_null() && unsafe { !queue_empty(dict_watchers_queue(d)) }
}

#[inline]
unsafe fn tv_blob_set_ret_inline(tv: *mut typval_T, b: *mut blob_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_BLOB;
        (*tv).vval.v_blob = b;
        if !b.is_null() {
            (*b).bv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_blob_len_inline(b: *const blob_T) -> c_int {
    if b.is_null() {
        0
    } else {
        unsafe { (*b).bv_ga.ga_len }
    }
}

#[inline]
unsafe fn tv_blob_get_inline(b: *const blob_T, idx: c_int) -> u8 {
    unsafe { *(*b).bv_ga.ga_data.cast::<u8>().offset(idx as isize) }
}

#[inline]
unsafe fn tv_blob_set_inline(blob: *mut blob_T, idx: c_int, c: u8) {
    unsafe {
        *(*blob).bv_ga.ga_data.cast::<u8>().offset(idx as isize) = c;
    }
}

#[inline]
const fn tv_is_func_inline(tv: typval_T) -> bool {
    matches!(tv.v_type, VarType::VAR_FUNC | VarType::VAR_PARTIAL)
}

#[inline]
const fn tv_initial_value() -> typval_T {
    typval_T {
        v_type: VarType::VAR_UNKNOWN,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval { v_number: 0 },
    }
}

/// Allocate a list item.
unsafe fn tv_list_item_alloc() -> *mut listitem_T {
    unsafe { xmalloc(size_of::<listitem_T>()).cast::<listitem_T>() }
}

/// Remove a list item from a List and free it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_item_remove(
    l: *mut list_T,
    item: *mut listitem_T,
) -> *mut listitem_T {
    unsafe {
        let next_item = (*item).li_next;
        tv_list_drop_items(l, item, item);
        tv_clear(&raw mut (*item).li_tv);
        xfree(item.cast());
        next_item
    }
}

/// Add a watcher to a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_watch_add(l: *mut list_T, lw: *mut listwatch_T) {
    unsafe {
        (*lw).lw_next = (*l).lv_watch.cast::<listwatch_T>();
        (*l).lv_watch = lw.cast::<c_void>();
    }
}

/// Remove a watcher from a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_watch_remove(l: *mut list_T, lwrem: *mut listwatch_T) {
    unsafe {
        let mut lwp = (&raw mut (*l).lv_watch).cast::<*mut listwatch_T>();
        let mut lw = (*l).lv_watch.cast::<listwatch_T>();
        while !lw.is_null() {
            if lw == lwrem {
                *lwp = (*lw).lw_next;
                break;
            }
            lwp = &raw mut (*lw).lw_next;
            lw = (*lw).lw_next;
        }
    }
}

/// Advance watchers to the next item.
unsafe fn tv_list_watch_fix(l: *mut list_T, item: *const listitem_T) {
    unsafe {
        let mut lw = (*l).lv_watch.cast::<listwatch_T>();
        while !lw.is_null() {
            if ptr::eq((*lw).lw_item, item) {
                (*lw).lw_item = (*item).li_next;
            }
            lw = (*lw).lw_next;
        }
    }
}

/// Allocate an empty list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_alloc(_len: isize) -> *mut list_T {
    unsafe {
        let list = xcalloc(1, size_of::<list_T>()).cast::<list_T>();

        if !gc_first_list.is_null() {
            (*gc_first_list.cast::<list_T>()).lv_used_prev = list;
        }
        (*list).lv_used_prev = ptr::null_mut();
        (*list).lv_used_next = gc_first_list.cast::<list_T>();
        gc_first_list = list.cast::<c_void>();
        (*list).lua_table_ref = LUA_NOREF;
        list
    }
}

/// Initialize a static list with 10 items.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_init_static10(sl: *mut staticList10_T) {
    unsafe {
        const SL_SIZE: usize = 10;
        ptr::write_bytes(sl, 0, 1);
        let l = &raw mut (*sl).sl_list;
        let items = (&raw mut (*sl).sl_items).cast::<listitem_T>();

        (*l).lv_first = items;
        (*l).lv_last = items.add(SL_SIZE - 1);
        (*l).lv_refcount = DO_NOT_FREE_CNT;
        tv_list_set_lock_inline(l, VAR_FIXED);
        (*l).lv_len = 10;

        (*items).li_prev = ptr::null_mut();
        (*items).li_next = items.add(1);
        (*items.add(SL_SIZE - 1)).li_prev = items.add(SL_SIZE - 2);
        (*items.add(SL_SIZE - 1)).li_next = ptr::null_mut();

        for i in 1..(SL_SIZE - 1) {
            let li = items.add(i);
            (*li).li_prev = li.sub(1);
            (*li).li_next = li.add(1);
        }
    }
}

/// Initialize static list with undefined number of elements.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_init_static(l: *mut list_T) {
    unsafe {
        ptr::write_bytes(l, 0, 1);
        (*l).lv_refcount = DO_NOT_FREE_CNT;
    }
}

/// Free items contained in a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_free_contents(l: *mut list_T) {
    unsafe {
        let mut item = (*l).lv_first;
        while !item.is_null() {
            (*l).lv_first = (*item).li_next;
            tv_clear(&raw mut (*item).li_tv);
            xfree(item.cast());
            item = (*l).lv_first;
        }
        (*l).lv_len = 0;
        (*l).lv_idx_item = ptr::null_mut();
        (*l).lv_last = ptr::null_mut();
        debug_assert!((*l).lv_watch.is_null());
    }
}

/// Free a list itself, ignoring items it contains.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_free_list(l: *mut list_T) {
    unsafe {
        if (*l).lv_used_prev.is_null() {
            gc_first_list = (*l).lv_used_next.cast::<c_void>();
        } else {
            (*(*l).lv_used_prev).lv_used_next = (*l).lv_used_next;
        }
        if !(*l).lv_used_next.is_null() {
            (*(*l).lv_used_next).lv_used_prev = (*l).lv_used_prev;
        }

        nlua_clear_ref(&mut (*l).lua_table_ref);
        xfree(l.cast());
    }
}

/// Free a list, including all items it points to.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_free(l: *mut list_T) {
    unsafe {
        if tv_in_free_unref_items {
            return;
        }

        tv_list_free_contents(l);
        tv_list_free_list(l);
    }
}

/// Unreference a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_unref(l: *mut list_T) {
    unsafe {
        if !l.is_null() {
            (*l).lv_refcount -= 1;
            if (*l).lv_refcount <= 0 {
                tv_list_free(l);
            }
        }
    }
}

/// Remove items "item" to "item2" from list "l".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_drop_items(
    l: *mut list_T,
    item: *mut listitem_T,
    item2: *mut listitem_T,
) {
    unsafe {
        let mut ip = item;
        while ip != (*item2).li_next {
            (*l).lv_len -= 1;
            tv_list_watch_fix(l, ip);
            ip = (*ip).li_next;
        }

        if (*item2).li_next.is_null() {
            (*l).lv_last = (*item).li_prev;
        } else {
            (*(*item2).li_next).li_prev = (*item).li_prev;
        }
        if (*item).li_prev.is_null() {
            (*l).lv_first = (*item2).li_next;
        } else {
            (*(*item).li_prev).li_next = (*item2).li_next;
        }
        (*l).lv_idx_item = ptr::null_mut();
    }
}

/// Like tv_list_drop_items, but also frees all removed items.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_remove_items(
    l: *mut list_T,
    item: *mut listitem_T,
    item2: *mut listitem_T,
) {
    unsafe {
        tv_list_drop_items(l, item, item2);
        let mut li = item;
        loop {
            tv_clear(&raw mut (*li).li_tv);
            let nli = (*li).li_next;
            xfree(li.cast());
            if li == item2 {
                break;
            }
            li = nli;
        }
    }
}

/// Move items "item" to "item2" from list "l" to the end of the list "tgt_l".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_move_items(
    l: *mut list_T,
    item: *mut listitem_T,
    item2: *mut listitem_T,
    tgt_l: *mut list_T,
    cnt: c_int,
) {
    unsafe {
        tv_list_drop_items(l, item, item2);
        (*item).li_prev = (*tgt_l).lv_last;
        (*item2).li_next = ptr::null_mut();
        if (*tgt_l).lv_last.is_null() {
            (*tgt_l).lv_first = item;
        } else {
            (*(*tgt_l).lv_last).li_next = item;
        }
        (*tgt_l).lv_last = item2;
        (*tgt_l).lv_len += cnt;
    }
}

/// Insert list item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_insert(
    l: *mut list_T,
    ni: *mut listitem_T,
    item: *mut listitem_T,
) {
    unsafe {
        if item.is_null() {
            tv_list_append(l, ni);
        } else {
            (*ni).li_prev = (*item).li_prev;
            (*ni).li_next = item;
            if (*item).li_prev.is_null() {
                (*l).lv_first = ni;
                (*l).lv_idx += 1;
            } else {
                (*(*item).li_prev).li_next = ni;
                (*l).lv_idx_item = ptr::null_mut();
            }
            (*item).li_prev = ni;
            (*l).lv_len += 1;
        }
    }
}

/// Insert Vimscript value into a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_insert_tv(
    l: *mut list_T,
    tv: *mut typval_T,
    item: *mut listitem_T,
) {
    unsafe {
        let ni = tv_list_item_alloc();
        tv_copy(tv, &raw mut (*ni).li_tv);
        tv_list_insert(l, ni, item);
    }
}

/// Append item to the end of list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append(l: *mut list_T, item: *mut listitem_T) {
    unsafe {
        if (*l).lv_last.is_null() {
            (*l).lv_first = item;
            (*l).lv_last = item;
            (*item).li_prev = ptr::null_mut();
        } else {
            (*(*l).lv_last).li_next = item;
            (*item).li_prev = (*l).lv_last;
            (*l).lv_last = item;
        }
        (*l).lv_len += 1;
        (*item).li_next = ptr::null_mut();
    }
}

/// Append Vimscript value to the end of list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_tv(l: *mut list_T, tv: *mut typval_T) {
    unsafe {
        let li = tv_list_item_alloc();
        tv_copy(tv, &raw mut (*li).li_tv);
        tv_list_append(l, li);
    }
}

/// Like tv_list_append_tv(), but tv is moved to a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_owned_tv(l: *mut list_T, tv: typval_T) -> *mut typval_T {
    unsafe {
        let li = tv_list_item_alloc();
        (*li).li_tv = tv;
        tv_list_append(l, li);
        &raw mut (*li).li_tv
    }
}

/// Append a list to a list as one item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_list(l: *mut list_T, itemlist: *mut list_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_LIST,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_list: itemlist },
            },
        );
        tv_list_ref_inline(itemlist);
    }
}

/// Append a dictionary to a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_dict(l: *mut list_T, dict: *mut dict_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_DICT,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_dict: dict },
            },
        );
        if !dict.is_null() {
            (*dict).dv_refcount += 1;
        }
    }
}

/// Make a copy of "str" and append it as an item to list "l".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_string(l: *mut list_T, str_: *const c_char, len: isize) {
    unsafe {
        let v_string = if str_.is_null() {
            ptr::null_mut()
        } else if len >= 0 {
            xmemdupz(str_.cast(), len as usize).cast::<c_char>()
        } else {
            xstrdup(str_)
        };
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_string },
            },
        );
    }
}

/// Append given string to the list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_allocated_string(l: *mut list_T, str_: *mut c_char) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_string: str_ },
            },
        );
    }
}

/// Append number to the list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_append_number(l: *mut list_T, n: varnumber_T) {
    unsafe {
        tv_list_append_owned_tv(
            l,
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval { v_number: n },
            },
        );
    }
}

/// Make a copy of list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_copy(
    conv: *const vimconv_T,
    orig: *mut list_T,
    deep: bool,
    copyID: c_int,
) -> *mut list_T {
    unsafe {
        if orig.is_null() {
            return ptr::null_mut();
        }

        let copy = tv_list_alloc(tv_list_len_inline(orig) as isize);
        tv_list_ref_inline(copy);
        if copyID != 0 {
            (*orig).lv_copyID = copyID;
            (*orig).lv_copylist = copy;
        }
        let mut item = (*orig).lv_first;
        while !item.is_null() {
            if got_int {
                break;
            }
            let ni = tv_list_item_alloc();
            if deep {
                if var_item_copy(
                    conv.cast(),
                    &raw mut (*item).li_tv,
                    &raw mut (*ni).li_tv,
                    deep,
                    copyID,
                ) == FAIL
                {
                    xfree(ni.cast());
                    tv_list_unref(copy);
                    return ptr::null_mut();
                }
            } else {
                tv_copy(&raw const (*item).li_tv, &raw mut (*ni).li_tv);
            }
            tv_list_append(copy, ni);
            item = (*item).li_next;
        }

        copy
    }
}

/// Get the list item in "l" with index "n1".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_check_range_index_one(
    l: *mut list_T,
    n1: *mut c_int,
    quiet: bool,
) -> *mut listitem_T {
    unsafe {
        let li = tv_list_find_index(l, n1);
        if !li.is_null() {
            return li;
        }

        if !quiet {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n1));
        }
        ptr::null_mut()
    }
}

/// Check that "n2" can be used as the second index in a range of list "l".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_check_range_index_two(
    l: *mut list_T,
    n1: *mut c_int,
    li1: *const listitem_T,
    n2: *mut c_int,
    quiet: bool,
) -> c_int {
    unsafe {
        if *n2 < 0 {
            let ni = tv_list_find(l, *n2);
            if ni.is_null() {
                if !quiet {
                    semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n2));
                }
                return FAIL;
            }
            *n2 = tv_list_idx_of_item(l, ni);
        }

        if *n1 < 0 {
            *n1 = tv_list_idx_of_item(l, li1);
        }
        if *n2 < *n1 {
            if !quiet {
                semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(*n2));
            }
            return FAIL;
        }
        OK
    }
}

/// Assign values from list "src" into a range of "dest".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_assign_range(
    dest: *mut list_T,
    src: *mut list_T,
    idx1_arg: c_int,
    idx2: c_int,
    empty_idx2: bool,
    op: *const c_char,
    varname: *const c_char,
) -> c_int {
    unsafe {
        let mut idx1 = idx1_arg;
        let first_li = tv_list_find_index(dest, &raw mut idx1);
        let mut src_li: *mut listitem_T;

        let mut idx = idx1;
        let mut dest_li = first_li;
        src_li = tv_list_first_inline(src);
        while !src_li.is_null() && !dest_li.is_null() {
            if value_check_lock((*dest_li).li_tv.v_lock, varname, TV_CSTRING) {
                return FAIL;
            }
            src_li = (*src_li).li_next;
            if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
                break;
            }
            dest_li = (*dest_li).li_next;
            idx += 1;
        }

        idx = idx1;
        dest_li = first_li;
        src_li = tv_list_first_inline(src);
        while !src_li.is_null() {
            debug_assert!(!dest_li.is_null());
            if !op.is_null() && *op != b'=' as c_char {
                eexe_mod_op(&raw mut (*dest_li).li_tv, &raw mut (*src_li).li_tv, op);
            } else {
                tv_clear(&raw mut (*dest_li).li_tv);
                tv_copy(&raw const (*src_li).li_tv, &raw mut (*dest_li).li_tv);
            }
            src_li = (*src_li).li_next;
            if src_li.is_null() || (!empty_idx2 && idx2 == idx) {
                break;
            }
            if (*dest_li).li_next.is_null() {
                tv_list_append_number(dest, 0);
                dest_li = tv_list_last_inline(dest);
            } else {
                dest_li = (*dest_li).li_next;
            }
            idx += 1;
        }
        if !src_li.is_null() {
            emsg(tr(c"E710: List value has more items than target".as_ptr()));
            return FAIL;
        }
        if if empty_idx2 {
            !dest_li.is_null() && !(*dest_li).li_next.is_null()
        } else {
            idx != idx2
        } {
            emsg(tr(c"E711: List value has not enough items".as_ptr()));
            return FAIL;
        }
        OK
    }
}

/// Flatten up to "maxitems" in "list", starting at "first" to depth "maxdepth".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_flatten(
    list: *mut list_T,
    first: *mut listitem_T,
    maxitems: i64,
    maxdepth: i64,
) {
    unsafe {
        if maxdepth == 0 {
            return;
        }

        let mut item = if first.is_null() {
            (*list).lv_first
        } else {
            first
        };
        let mut done: i64 = 0;

        while !item.is_null() && done < maxitems {
            let next = (*item).li_next;

            fast_breakcheck();
            if got_int {
                return;
            }
            if (*item).li_tv.v_type == VarType::VAR_LIST {
                let itemlist = (*item).li_tv.vval.v_list;

                tv_list_drop_items(list, item, item);
                tv_list_extend(list, itemlist, next);

                if maxdepth > 0 {
                    let sub_first = if (*item).li_prev.is_null() {
                        (*list).lv_first
                    } else {
                        (*(*item).li_prev).li_next
                    };
                    tv_list_flatten(list, sub_first, i64::from((*itemlist).lv_len), maxdepth - 1);
                }
                tv_clear(&raw mut (*item).li_tv);
                xfree(item.cast());
            }

            done += 1;
            item = next;
        }
    }
}

/// "items(blob)" function.
unsafe fn tv_blob2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let blob = (*argvars).vval.v_blob;

        tv_list_alloc_ret(rettv, tv_blob_len_inline(blob) as isize);

        for i in 0..tv_blob_len_inline(blob) {
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list, l2);
            tv_list_append_number(l2, i64::from(i));
            tv_list_append_number(l2, i64::from(tv_blob_get_inline(blob, i)));
        }
    }
}

/// "items(dict)" function.
unsafe fn tv_dict2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Items);
    }
}

/// "items(list)" function.
unsafe fn tv_list2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let l = (*argvars).vval.v_list;

        tv_list_alloc_ret(rettv, tv_list_len_inline(l) as isize);
        if l.is_null() {
            return;
        }

        let mut idx: varnumber_T = 0;
        let mut li = (*l).lv_first;
        while !li.is_null() {
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list, l2);
            tv_list_append_number(l2, idx);
            tv_list_append_tv(l2, &raw mut (*li).li_tv);
            idx += 1;
            li = (*li).li_next;
        }
    }
}

/// "items(string)" function.
unsafe fn tv_string2items(argvars: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let mut p: *const c_char = (*argvars).vval.v_string;

        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if p.is_null() {
            return;
        }

        let mut idx: varnumber_T = 0;
        while *p != NUL {
            let len = utfc_ptr2len(p);
            if len == 0 {
                break;
            }
            let l2 = tv_list_alloc(2);
            tv_list_append_list((*rettv).vval.v_list, l2);
            tv_list_append_number(l2, idx);
            tv_list_append_string(l2, p, len as isize);
            p = p.add(len as usize);
            idx += 1;
        }
    }
}

/// Extend first list with the second.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_extend(l1: *mut list_T, l2: *mut list_T, bef: *mut listitem_T) {
    unsafe {
        let mut todo = tv_list_len_inline(l2);
        if todo == 0 {
            return;
        }

        let befbef = if bef.is_null() {
            ptr::null_mut()
        } else {
            (*bef).li_prev
        };
        let saved_next = if befbef.is_null() {
            ptr::null_mut()
        } else {
            (*befbef).li_next
        };
        let mut item = tv_list_first_inline(l2);
        while !item.is_null() && todo > 0 {
            todo -= 1;
            tv_list_insert_tv(l1, &raw mut (*item).li_tv, bef);
            item = if item == befbef {
                saved_next
            } else {
                (*item).li_next
            };
        }
    }
}

/// Concatenate lists into a new list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_concat(
    l1: *mut list_T,
    l2: *mut list_T,
    tv: *mut typval_T,
) -> c_int {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).v_lock = VAR_UNLOCKED;
        let l = if l1.is_null() && l2.is_null() {
            ptr::null_mut()
        } else if l1.is_null() {
            tv_list_copy(ptr::null(), l2, false, 0)
        } else {
            let copy = tv_list_copy(ptr::null(), l1, false, 0);
            if !copy.is_null() && !l2.is_null() {
                tv_list_extend(copy, l2, ptr::null_mut());
            }
            copy
        };
        if l.is_null() && !(l1.is_null() && l2.is_null()) {
            return FAIL;
        }

        (*tv).vval.v_list = l;
        OK
    }
}

unsafe fn tv_list_slice(ol: *mut list_T, mut n1: varnumber_T, n2: varnumber_T) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc((n2 - n1 + 1) as isize);
        let mut item = tv_list_find(ol, n1 as c_int);
        while n1 <= n2 {
            tv_list_append_tv(l, &raw mut (*item).li_tv);
            item = (*item).li_next;
            n1 += 1;
        }
        l
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_slice_or_index(
    _list: *mut list_T,
    range: bool,
    n1_arg: varnumber_T,
    n2_arg: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
    verbose: bool,
) -> c_int {
    unsafe {
        let len = i64::from(tv_list_len_inline((*rettv).vval.v_list));
        let mut n1 = n1_arg;
        let mut n2 = n2_arg;

        if n1 < 0 {
            n1 += len;
        }
        if n1 < 0 || n1 >= len {
            if !range {
                if verbose {
                    semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), n1_arg);
                }
                return FAIL;
            }
            n1 = len;
        }
        if range {
            if n2 < 0 {
                n2 += len;
            } else if n2 >= len {
                n2 = len - if exclusive { 0 } else { 1 };
            }
            if exclusive {
                n2 -= 1;
            }
            if n2 < 0 || n2 + 1 < n1 {
                n2 = -1;
            }
            let l = tv_list_slice((*rettv).vval.v_list, n1, n2);
            tv_clear(rettv);
            tv_list_set_ret_inline(rettv, l);
        } else {
            let found = tv_list_find((*rettv).vval.v_list, n1 as c_int);
            let mut var1 = tv_initial_value();
            tv_copy(&raw const (*found).li_tv, &raw mut var1);
            tv_clear(rettv);
            *rettv = var1;
        }
        OK
    }
}

unsafe fn list_join_inner(
    gap: *mut garray_T,
    l: *mut list_T,
    sep: *const c_char,
    join_gap: *mut garray_T,
) -> c_int {
    unsafe {
        let mut sumlen: usize = 0;
        let mut first = true;

        let mut item = (*l).lv_first;
        while !item.is_null() {
            if got_int {
                break;
            }
            let mut s = NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            s.data = encode_tv2echo(&raw mut (*item).li_tv, &raw mut s.size);
            if s.data.is_null() {
                return FAIL;
            }

            sumlen += s.size;

            ga_grow(join_gap, 1);
            let p = (*join_gap)
                .ga_data
                .cast::<Join>()
                .offset((*join_gap).ga_len as isize);
            (*join_gap).ga_len += 1;
            (*p).s = s;
            (*p).tofree = s.data;

            line_breakcheck();
            item = (*item).li_next;
        }

        let seplen = c_strlen(sep);
        if (*join_gap).ga_len >= 2 {
            sumlen += seplen * ((*join_gap).ga_len as usize - 1);
        }
        ga_grow(gap, (sumlen + 2) as c_int);

        let mut i: c_int = 0;
        while i < (*join_gap).ga_len && !got_int {
            if first {
                first = false;
            } else {
                ga_concat_len(gap, sep, seplen);
            }
            let p = (*join_gap).ga_data.cast::<Join>().offset(i as isize);
            if !(*p).s.data.is_null() {
                ga_concat_len(gap, (*p).s.data, (*p).s.size);
            }
            line_breakcheck();
            i += 1;
        }

        OK
    }
}

/// Join list into a string using given separator.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_join(
    gap: *mut garray_T,
    l: *mut list_T,
    sep: *const c_char,
) -> c_int {
    unsafe {
        if tv_list_len_inline(l) == 0 {
            return OK;
        }

        let mut join_ga: garray_T = std::mem::zeroed();
        ga_init(
            &raw mut join_ga,
            size_of::<Join>() as c_int,
            tv_list_len_inline(l),
        );
        let retval = list_join_inner(gap, l, sep, &raw mut join_ga);

        for i in 0..join_ga.ga_len {
            let p = join_ga.ga_data.cast::<Join>().offset(i as isize);
            xfree((*p).tofree.cast());
        }
        ga_clear(&raw mut join_ga);

        retval
    }
}

/// "join()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_join(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(tr(e_listreq.as_ptr()));
            return;
        }
        let sep = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            c" ".as_ptr()
        } else {
            tv_get_string_chk(argvars.add(1))
        };

        (*rettv).v_type = VarType::VAR_STRING;

        if !sep.is_null() {
            let mut ga: garray_T = std::mem::zeroed();
            ga_init(&raw mut ga, size_of::<c_char>() as c_int, 80);
            tv_list_join(&raw mut ga, (*argvars).vval.v_list, sep);
            ga_append(&raw mut ga, NUL as u8);
            (*rettv).vval.v_string = ga.ga_data.cast::<c_char>();
        } else {
            (*rettv).vval.v_string = ptr::null_mut();
        }
    }
}

/// "list2str()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_list2str(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(tr(e_invarg.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, 1, 80);
        let mut buf: [c_char; MB_MAXBYTES + 1] = [0; MB_MAXBYTES + 1];

        let mut li = (*l).lv_first;
        while !li.is_null() {
            let n = tv_get_number(&raw const (*li).li_tv);
            let buflen = utf_char2bytes(n as c_int, buf.as_mut_ptr()) as usize;
            buf[buflen] = NUL;
            ga_concat_len(&raw mut ga, buf.as_ptr(), buflen);
            li = (*li).li_next;
        }
        ga_append(&raw mut ga, NUL as u8);

        (*rettv).vval.v_string = ga.ga_data.cast::<c_char>();
    }
}

/// "remove({list})" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        let l = (*argvars).vval.v_list;
        if value_check_lock(tv_list_locked_inline(l), arg_errmsg, TV_TRANSLATE) {
            return;
        }

        let mut error = false;
        let idx = tv_get_number_chk(argvars.add(1), &raw mut error);

        if error {
            return;
        }
        let item = tv_list_find(l, idx as c_int);
        if item.is_null() {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), idx);
        } else if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            tv_list_drop_items(l, item, item);
            *rettv = (*item).li_tv;
            xfree(item.cast());
        } else {
            let end = tv_get_number_chk(argvars.add(2), &raw mut error);
            if error {
                return;
            }
            let item2 = tv_list_find(l, end as c_int);
            if item2.is_null() {
                semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), end);
            } else {
                let mut cnt: c_int = 0;
                let mut li = item;
                while !li.is_null() {
                    cnt += 1;
                    if li == item2 {
                        break;
                    }
                    li = (*li).li_next;
                }
                if li.is_null() {
                    emsg(tr(e_invrange.as_ptr()));
                } else {
                    let tgt = tv_list_alloc_ret(rettv, cnt as isize);
                    tv_list_move_items(l, item, item2, tgt, cnt);
                }
            }
        }
    }
}

/// Compare functions for f_sort() and f_uniq().
unsafe fn item_compare(s1: *const c_void, s2: *const c_void, keep_zero: bool) -> c_int {
    unsafe {
        let si1 = s1.cast::<ListSortItem>();
        let si2 = s2.cast::<ListSortItem>();
        let item1 = *s1.cast::<*mut listitem_T>();
        let item2 = *s2.cast::<*mut listitem_T>();

        let tv1 = &raw mut (*item1).li_tv;
        let tv2 = &raw mut (*item2).li_tv;

        let mut res: c_int;

        if (*SORTINFO).item_compare_numbers {
            let v1 = if (*SORTINFO).item_compare_keys_ready {
                (*si1).key.inum
            } else {
                tv_get_number(tv1)
            };
            let v2 = if (*SORTINFO).item_compare_keys_ready {
                (*si2).key.inum
            } else {
                tv_get_number(tv2)
            };

            res = if v1 == v2 {
                0
            } else if v1 > v2 {
                1
            } else {
                -1
            };
            if res == 0 && !keep_zero {
                res = if (*si1).idx > (*si2).idx { 1 } else { -1 };
            }
            return res;
        }

        if (*SORTINFO).item_compare_float {
            let v1 = if (*SORTINFO).item_compare_keys_ready {
                (*si1).key.fnum
            } else {
                tv_get_float(tv1)
            };
            let v2 = if (*SORTINFO).item_compare_keys_ready {
                (*si2).key.fnum
            } else {
                tv_get_float(tv2)
            };

            res = if v1 == v2 {
                0
            } else if v1 > v2 {
                1
            } else {
                -1
            };
            if res == 0 && !keep_zero {
                res = if (*si1).idx > (*si2).idx { 1 } else { -1 };
            }
            return res;
        }

        if (*SORTINFO).item_compare_numeric && (*SORTINFO).item_compare_keys_ready {
            let n1 = (*si1).key.fnum;
            let n2 = (*si2).key.fnum;

            res = if n1 == n2 {
                0
            } else if n1 > n2 {
                1
            } else {
                -1
            };
            if res == 0 && !keep_zero {
                res = if (*si1).idx > (*si2).idx { 1 } else { -1 };
            }
            return res;
        }

        let mut tofree1: *mut c_char = ptr::null_mut();
        let mut tofree2: *mut c_char = ptr::null_mut();
        let mut p1: *mut c_char;
        let mut p2: *mut c_char;

        if (*tv1).v_type == VarType::VAR_STRING {
            if (*tv2).v_type != VarType::VAR_STRING || (*SORTINFO).item_compare_numeric {
                p1 = c"'".as_ptr().cast_mut();
            } else {
                p1 = (*tv1).vval.v_string;
            }
        } else {
            tofree1 = encode_tv2string(tv1, ptr::null_mut());
            p1 = tofree1;
        }
        if (*tv2).v_type == VarType::VAR_STRING {
            if (*tv1).v_type != VarType::VAR_STRING || (*SORTINFO).item_compare_numeric {
                p2 = c"'".as_ptr().cast_mut();
            } else {
                p2 = (*tv2).vval.v_string;
            }
        } else {
            tofree2 = encode_tv2string(tv2, ptr::null_mut());
            p2 = tofree2;
        }
        if p1.is_null() {
            p1 = c"".as_ptr().cast_mut();
        }
        if p2.is_null() {
            p2 = c"".as_ptr().cast_mut();
        }
        if !(*SORTINFO).item_compare_numeric {
            if (*SORTINFO).item_compare_lc {
                res = strcoll(p1, p2);
            } else if (*SORTINFO).item_compare_ic != 0 {
                #[cfg(windows)]
                {
                    res = vim_stricmp(p1, p2);
                }
                #[cfg(not(windows))]
                {
                    res = strcasecmp(p1, p2);
                }
            } else {
                res = strcmp(p1, p2);
            }
        } else {
            let n1 = strtod(p1, &raw mut p1);
            let n2 = strtod(p2, &raw mut p2);
            res = if n1 == n2 {
                0
            } else if n1 > n2 {
                1
            } else {
                -1
            };
        }

        xfree(tofree1.cast());
        xfree(tofree2.cast());

        if res == 0 && !keep_zero {
            res = if (*si1).idx > (*si2).idx { 1 } else { -1 };
        }
        res
    }
}

unsafe extern "C" fn item_compare_keeping_zero(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe { item_compare(s1, s2, true) }
}

unsafe extern "C" fn item_compare_not_keeping_zero(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe { item_compare(s1, s2, false) }
}

unsafe fn item_compare2(s1: *const c_void, s2: *const c_void, keep_zero: bool) -> c_int {
    unsafe {
        if (*SORTINFO).item_compare_func_err {
            return 0;
        }

        let si1 = s1.cast::<ListSortItem>();
        let si2 = s2.cast::<ListSortItem>();
        let item1 = *s1.cast::<*mut listitem_T>();
        let item2 = *s2.cast::<*mut listitem_T>();

        let partial = (*SORTINFO).item_compare_partial;
        let func_name = if partial.is_null() {
            (*SORTINFO).item_compare_func
        } else {
            partial_name(partial)
        };

        let mut argv: [typval_T; 3] = [tv_initial_value(); 3];
        tv_copy(&raw const (*item1).li_tv, &raw mut argv[0]);
        tv_copy(&raw const (*item2).li_tv, &raw mut argv[1]);

        let mut rettv = tv_initial_value();
        let mut funcexe: funcexe_T = std::mem::zeroed();
        funcexe.fe_evaluate = true;
        funcexe.fe_partial = partial;
        funcexe.fe_selfdict = (*SORTINFO).item_compare_selfdict;
        let mut res = call_func(
            func_name,
            -1,
            &raw mut rettv,
            2,
            argv.as_mut_ptr(),
            &raw mut funcexe,
        );
        tv_clear(&raw mut argv[0]);
        tv_clear(&raw mut argv[1]);

        if res == FAIL {
            res = ITEM_COMPARE_FAIL;
            (*SORTINFO).item_compare_func_err = true;
        } else {
            let n = tv_get_number_chk(&raw const rettv, &raw mut (*SORTINFO).item_compare_func_err);
            res = if n > 0 {
                1
            } else if n < 0 {
                -1
            } else {
                0
            };
        }
        if (*SORTINFO).item_compare_func_err {
            res = ITEM_COMPARE_FAIL;
        }
        tv_clear(&raw mut rettv);

        if res == 0 && !keep_zero {
            res = if (*si1).idx > (*si2).idx { 1 } else { -1 };
        }

        res
    }
}

unsafe extern "C" fn item_compare2_keeping_zero(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe { item_compare2(s1, s2, true) }
}

unsafe extern "C" fn item_compare2_not_keeping_zero(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe { item_compare2(s1, s2, false) }
}

unsafe fn sort_compute_keys(ptrs: *mut ListSortItem, len: c_int, info: *mut sortinfo_T) {
    unsafe {
        if (*info).item_compare_numbers {
            for i in 0..len as usize {
                let item = (*ptrs.add(i)).item;
                (*ptrs.add(i)).key.inum = tv_get_number(&raw const (*item).li_tv);
            }
        } else if (*info).item_compare_float {
            for i in 0..len as usize {
                let item = (*ptrs.add(i)).item;
                (*ptrs.add(i)).key.fnum = tv_get_float(&raw const (*item).li_tv);
            }
        } else {
            for i in 0..len as usize {
                let item = (*ptrs.add(i)).item;
                let tv = &raw mut (*item).li_tv;
                if (*tv).v_type == VarType::VAR_STRING {
                    (*ptrs.add(i)).key.fnum = 0.0;
                } else {
                    let p = encode_tv2string(tv, ptr::null_mut());
                    (*ptrs.add(i)).key.fnum = if p.is_null() {
                        0.0
                    } else {
                        strtod(p, ptr::null_mut())
                    };
                    xfree(p.cast());
                }
            }
        }
        (*info).item_compare_keys_ready = true;
    }
}

unsafe fn do_sort(l: *mut list_T, info: *mut sortinfo_T) {
    unsafe {
        let len = tv_list_len_inline(l);
        let ptrs =
            xmalloc((len as c_uint as usize) * size_of::<ListSortItem>()).cast::<ListSortItem>();

        let mut i: c_int = 0;
        let mut li = (*l).lv_first;
        while !li.is_null() {
            (*ptrs.offset(i as isize)).item = li;
            (*ptrs.offset(i as isize)).idx = i;
            i += 1;
            li = (*li).li_next;
        }

        (*info).item_compare_func_err = false;
        (*info).item_compare_keys_ready = false;

        if (*info).item_compare_func.is_null()
            && (*info).item_compare_partial.is_null()
            && ((*info).item_compare_numbers
                || (*info).item_compare_float
                || (*info).item_compare_numeric)
        {
            sort_compute_keys(ptrs, len, info);
        }
        let item_compare_func: ListSorter =
            if (*info).item_compare_func.is_null() && (*info).item_compare_partial.is_null() {
                item_compare_not_keeping_zero
            } else {
                item_compare2_not_keeping_zero
            };

        qsort(
            ptrs.cast(),
            len as usize,
            size_of::<ListSortItem>(),
            item_compare_func,
        );
        if !(*info).item_compare_func_err {
            (*l).lv_first = ptr::null_mut();
            (*l).lv_last = ptr::null_mut();
            (*l).lv_idx_item = ptr::null_mut();
            (*l).lv_len = 0;
            for idx in 0..len as usize {
                tv_list_append(l, (*ptrs.add(idx)).item);
            }
        }
        if (*info).item_compare_func_err {
            emsg(tr(c"E702: Sort compare function failed".as_ptr()));
        }

        xfree(ptrs.cast());
    }
}

unsafe fn do_uniq(l: *mut list_T, info: *mut sortinfo_T) {
    unsafe {
        let len = tv_list_len_inline(l);
        let ptrs =
            xmalloc((len as c_uint as usize) * size_of::<ListSortItem>()).cast::<ListSortItem>();

        (*info).item_compare_func_err = false;
        let item_compare_func: ListSorter =
            if (*info).item_compare_func.is_null() && (*info).item_compare_partial.is_null() {
                item_compare_keeping_zero
            } else {
                item_compare2_keeping_zero
            };

        let first = tv_list_first_inline(l);
        let mut li = if first.is_null() {
            ptr::null_mut()
        } else {
            (*first).li_next
        };
        while !li.is_null() {
            let prev_li = (*li).li_prev;
            if item_compare_func((&raw const prev_li).cast(), (&raw const li).cast()) == 0 {
                li = tv_list_item_remove(l, li);
            } else {
                li = (*li).li_next;
            }
            if (*info).item_compare_func_err {
                emsg(tr(c"E882: Uniq compare function failed".as_ptr()));
                break;
            }
        }

        xfree(ptrs.cast());
    }
}

unsafe fn parse_sort_uniq_args(argvars: *mut typval_T, info: *mut sortinfo_T) -> c_int {
    unsafe {
        (*info).item_compare_ic = 0;
        (*info).item_compare_lc = false;
        (*info).item_compare_numeric = false;
        (*info).item_compare_numbers = false;
        (*info).item_compare_float = false;
        (*info).item_compare_func = ptr::null();
        (*info).item_compare_partial = ptr::null_mut();
        (*info).item_compare_selfdict = ptr::null_mut();
        (*info).item_compare_keys_ready = false;

        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            return OK;
        }

        if (*argvars.add(1)).v_type == VarType::VAR_FUNC {
            (*info).item_compare_func = (*argvars.add(1)).vval.v_string;
        } else if (*argvars.add(1)).v_type == VarType::VAR_PARTIAL {
            (*info).item_compare_partial = (*argvars.add(1)).vval.v_ptr.cast::<partial_T>();
        } else {
            let mut error = false;
            let nr = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return FAIL;
            }
            if nr == 1 {
                (*info).item_compare_ic = 1;
            } else if (*argvars.add(1)).v_type != VarType::VAR_NUMBER {
                (*info).item_compare_func = tv_get_string(argvars.add(1));
            } else if nr != 0 {
                emsg(tr(e_invarg.as_ptr()));
                return FAIL;
            }
            if !(*info).item_compare_func.is_null() {
                let fn_cstr = CStr::from_ptr((*info).item_compare_func).to_bytes();
                match fn_cstr {
                    b"" => (*info).item_compare_func = ptr::null(),
                    b"n" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_numeric = true;
                    }
                    b"N" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_numbers = true;
                    }
                    b"f" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_float = true;
                    }
                    b"i" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_ic = 1;
                    }
                    b"l" => {
                        (*info).item_compare_func = ptr::null();
                        (*info).item_compare_lc = true;
                    }
                    _ => {}
                }
            }
        }

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_dict_arg(argvars, 2) == FAIL {
                return FAIL;
            }
            (*info).item_compare_selfdict = (*argvars.add(2)).vval.v_dict;
        }

        OK
    }
}

unsafe fn do_sort_uniq(argvars: *mut typval_T, rettv: *mut typval_T, sort: bool) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg!(
                tr(e_listarg.as_ptr()),
                if sort {
                    c"sort()".as_ptr()
                } else {
                    c"uniq()".as_ptr()
                },
            );
            return;
        }

        let mut info: sortinfo_T = std::mem::zeroed();
        let old_sortinfo = SORTINFO;
        SORTINFO = &raw mut info;

        let arg_errmsg = if sort {
            c"sort() argument".as_ptr()
        } else {
            c"uniq() argument".as_ptr()
        };
        let l = (*argvars).vval.v_list;
        if !value_check_lock(tv_list_locked_inline(l), arg_errmsg, TV_TRANSLATE) {
            tv_list_set_ret_inline(rettv, l);

            let len = tv_list_len_inline(l);
            if len > 1 && parse_sort_uniq_args(argvars, &raw mut info) != FAIL {
                if sort {
                    do_sort(l, &raw mut info);
                } else {
                    do_uniq(l, &raw mut info);
                }
            }
        }

        SORTINFO = old_sortinfo;
    }
}

/// "sort({list})" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_sort(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        do_sort_uniq(argvars, rettv, true);
    }
}

/// "uniq({list})" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_uniq(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        do_sort_uniq(argvars, rettv, false);
    }
}

/// Check whether two lists are equal.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_equal(l1: *mut list_T, l2: *mut list_T, ic: bool) -> bool {
    unsafe {
        if l1 == l2 {
            return true;
        }
        if tv_list_len_inline(l1) != tv_list_len_inline(l2) {
            return false;
        }
        if tv_list_len_inline(l1) == 0 {
            return true;
        }
        if l1.is_null() || l2.is_null() {
            return false;
        }

        let mut item1 = tv_list_first_inline(l1);
        let mut item2 = tv_list_first_inline(l2);
        while !item1.is_null() && !item2.is_null() {
            if !tv_equal(&raw mut (*item1).li_tv, &raw mut (*item2).li_tv, ic) {
                return false;
            }
            item1 = (*item1).li_next;
            item2 = (*item2).li_next;
        }
        debug_assert!(item1.is_null() && item2.is_null());
        true
    }
}

/// Reverse list in-place.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_reverse(l: *mut list_T) {
    unsafe {
        if tv_list_len_inline(l) <= 1 {
            return;
        }
        std::mem::swap(&mut (*l).lv_first, &mut (*l).lv_last);
        let mut li = (*l).lv_first;
        while !li.is_null() {
            std::mem::swap(&mut (*li).li_next, &mut (*li).li_prev);
            li = (*li).li_next;
        }
        (*l).lv_idx = (*l).lv_len - (*l).lv_idx - 1;
    }
}

/// Locate item with a given index in a list and return it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_find(l: *mut list_T, mut n: c_int) -> *mut listitem_T {
    unsafe {
        if l.is_null() {
            return ptr::null_mut();
        }

        n = tv_list_uidx_inline(l, n);
        if n == -1 {
            return ptr::null_mut();
        }

        let mut idx: c_int;
        let mut item: *mut listitem_T;

        if !(*l).lv_idx_item.is_null() {
            if n < (*l).lv_idx / 2 {
                item = (*l).lv_first;
                idx = 0;
            } else if n > ((*l).lv_idx + (*l).lv_len) / 2 {
                item = (*l).lv_last;
                idx = (*l).lv_len - 1;
            } else {
                item = (*l).lv_idx_item;
                idx = (*l).lv_idx;
            }
        } else if n < (*l).lv_len / 2 {
            item = (*l).lv_first;
            idx = 0;
        } else {
            item = (*l).lv_last;
            idx = (*l).lv_len - 1;
        }

        while n > idx {
            item = (*item).li_next;
            idx += 1;
        }
        while n < idx {
            item = (*item).li_prev;
            idx -= 1;
        }

        debug_assert_eq!(idx, n);
        (*l).lv_idx = idx;
        (*l).lv_idx_item = item;

        item
    }
}

/// Get list item l[n] as a number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_find_nr(
    l: *mut list_T,
    n: c_int,
    ret_error: *mut bool,
) -> varnumber_T {
    unsafe {
        let li = tv_list_find(l, n);
        if li.is_null() {
            if !ret_error.is_null() {
                *ret_error = true;
            }
            return -1;
        }
        tv_get_number_chk(&raw const (*li).li_tv, ret_error)
    }
}

/// Get list item l[n] as a string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_find_str(l: *mut list_T, n: c_int) -> *const c_char {
    unsafe {
        let li = tv_list_find(l, n);
        if li.is_null() {
            semsg!(tr(e_list_index_out_of_range_nr.as_ptr()), i64::from(n));
            return ptr::null();
        }
        tv_get_string(&raw const (*li).li_tv)
    }
}

unsafe fn tv_list_find_index(l: *mut list_T, idx: *mut c_int) -> *mut listitem_T {
    unsafe {
        let mut li = tv_list_find(l, *idx);
        if !li.is_null() {
            return li;
        }

        if *idx < 0 {
            *idx = 0;
            li = tv_list_find(l, *idx);
        }
        li
    }
}

/// Locate item in a list and return its index.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_idx_of_item(l: *const list_T, item: *const listitem_T) -> c_int {
    unsafe {
        if l.is_null() {
            return -1;
        }
        let mut idx: c_int = 0;
        let mut li: *const listitem_T = (*l).lv_first;
        while !li.is_null() {
            if ptr::eq(li, item) {
                return idx;
            }
            idx += 1;
            li = (*li).li_next;
        }
        -1
    }
}

/// Perform all necessary cleanup for a `DictWatcher` instance.
unsafe fn tv_dict_watcher_free(watcher: *mut DictWatcher) {
    unsafe {
        callback_free(&raw mut (*watcher).callback);
        xfree((*watcher).key_pattern.cast());
        xfree(watcher.cast());
    }
}

/// Add watcher to a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_watcher_add(
    dict: *mut dict_T,
    key_pattern: *const c_char,
    key_pattern_len: usize,
    callback: Callback,
) {
    unsafe {
        if dict.is_null() {
            return;
        }
        let watcher = xmalloc(size_of::<DictWatcher>()).cast::<DictWatcher>();
        (*watcher).key_pattern = xmemdupz(key_pattern.cast(), key_pattern_len).cast::<c_char>();
        (*watcher).key_pattern_len = key_pattern_len;
        (*watcher).callback = callback;
        (*watcher).busy = false;
        (*watcher).needs_free = false;
        queue_insert_tail(dict_watchers_queue(dict), &raw mut (*watcher).node);
    }
}

/// Check whether two callbacks are equal.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_callback_equal(cb1: *const Callback, cb2: *const Callback) -> bool {
    unsafe {
        if (*cb1).cb_type != (*cb2).cb_type {
            return false;
        }
        match (*cb1).cb_type {
            kCallbackFuncref => {
                strequal((*cb1).data.cast::<c_char>(), (*cb2).data.cast::<c_char>())
            }
            kCallbackPartial => (*cb1).data == (*cb2).data,
            kCallbackLua => (*cb1).data as isize as LuaRef == (*cb2).data as isize as LuaRef,
            kCallbackExpr => strequal((*cb1).data.cast::<c_char>(), (*cb2).data.cast::<c_char>()),
            kCallbackNone => true,
            _ => std::process::abort(),
        }
    }
}

/// Unref/free callback.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn callback_free(callback: *mut Callback) {
    unsafe {
        match (*callback).cb_type {
            kCallbackFuncref => {
                let funcref = (*callback).data.cast::<c_char>();
                #[cfg(not(test))]
                func_unref(funcref);
                xfree(funcref.cast());
            }
            kCallbackPartial => {
                #[cfg(not(test))]
                partial_unref((*callback).data.cast::<partial_T>());
            }
            kCallbackLua => {
                let mut luaref = (*callback).data as isize as LuaRef;
                nlua_clear_ref(&mut luaref);
            }
            kCallbackExpr => {
                xfree((*callback).data);
            }
            _ => {}
        }
        (*callback).cb_type = kCallbackNone;
        (*callback).data = ptr::null_mut();
    }
}

/// Copy a callback into a typval_T.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn callback_put(cb: *mut Callback, tv: *mut typval_T) {
    unsafe {
        match (*cb).cb_type {
            kCallbackPartial => {
                let pt = (*cb).data.cast::<partial_T>();
                (*tv).v_type = VarType::VAR_PARTIAL;
                (*tv).vval.v_ptr = pt.cast();
                (*pt).pt_refcount += 1;
            }
            kCallbackFuncref => {
                let funcref = (*cb).data.cast::<c_char>();
                (*tv).v_type = VarType::VAR_FUNC;
                (*tv).vval.v_string = xstrdup(funcref);
                #[cfg(not(test))]
                func_ref((*tv).vval.v_string);
            }
            _ => {
                (*tv).v_type = VarType::VAR_SPECIAL;
                (*tv).vval.v_special = kSpecialVarNull;
            }
        }
    }
}

/// Copy callback from "src" to "dest", incrementing the refcounts.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn callback_copy(dest: *mut Callback, src: *mut Callback) {
    unsafe {
        (*dest).cb_type = (*src).cb_type;
        match (*src).cb_type {
            kCallbackPartial => {
                let pt = (*src).data.cast::<partial_T>();
                (*dest).data = pt.cast();
                (*pt).pt_refcount += 1;
            }
            kCallbackFuncref => {
                let funcref = (*src).data.cast::<c_char>();
                (*dest).data = xstrdup(funcref).cast();
                func_ref(funcref);
            }
            kCallbackLua => {
                let new_ref = api_new_luaref((*src).data as isize as LuaRef);
                (*dest).data = new_ref as isize as *mut c_void;
            }
            kCallbackExpr => {
                (*dest).data = xstrdup((*src).data.cast::<c_char>()).cast();
            }
            _ => {
                (*dest).data = ptr::null_mut();
            }
        }
    }
}

/// Generate a string description of a callback.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn callback_to_string(cb: *mut Callback, arena: *mut Arena) -> *mut c_char {
    unsafe {
        if (*cb).cb_type == kCallbackLua {
            return nlua_funcref_str((*cb).data as isize as LuaRef, arena.cast(), true);
        }

        const MSGLEN: usize = 100;
        let msg = xmallocz(MSGLEN).cast::<c_char>();

        match (*cb).cb_type {
            kCallbackFuncref => {
                let s = CStr::from_ptr((*cb).data.cast::<c_char>()).to_bytes();
                let prefix = b"<vim function: ";
                let suffix = b">";
                let copy_len = s.len().min(MSGLEN - prefix.len() - suffix.len());
                ptr::copy_nonoverlapping(prefix.as_ptr().cast::<c_char>(), msg, prefix.len());
                ptr::copy_nonoverlapping(
                    s.as_ptr().cast::<c_char>(),
                    msg.add(prefix.len()),
                    copy_len,
                );
                let after = prefix.len() + copy_len;
                if after < MSGLEN {
                    *msg.add(after) = b'>' as c_char;
                    *msg.add((after + 1).min(MSGLEN)) = NUL;
                } else {
                    *msg.add(MSGLEN) = NUL;
                }
            }
            kCallbackPartial => {
                let pt = (*cb).data.cast::<partial_T>();
                let s = CStr::from_ptr((*pt).pt_name).to_bytes();
                let prefix = b"<vim partial: ";
                let suffix = b">";
                let copy_len = s.len().min(MSGLEN - prefix.len() - suffix.len());
                ptr::copy_nonoverlapping(prefix.as_ptr().cast::<c_char>(), msg, prefix.len());
                ptr::copy_nonoverlapping(
                    s.as_ptr().cast::<c_char>(),
                    msg.add(prefix.len()),
                    copy_len,
                );
                let after = prefix.len() + copy_len;
                if after < MSGLEN {
                    *msg.add(after) = b'>' as c_char;
                    *msg.add((after + 1).min(MSGLEN)) = NUL;
                } else {
                    *msg.add(MSGLEN) = NUL;
                }
            }
            _ => {
                *msg = NUL;
            }
        }
        msg
    }
}

/// Remove watcher from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_watcher_remove(
    dict: *mut dict_T,
    key_pattern: *const c_char,
    key_pattern_len: usize,
    callback: Callback,
) -> bool {
    unsafe {
        if dict.is_null() {
            return false;
        }

        let head = dict_watchers_queue(dict);
        let mut w = (*head).next;
        let mut watcher: *mut DictWatcher = ptr::null_mut();
        let mut matched = false;
        let mut queue_is_busy = false;
        while w != head {
            let next = (*w).next;
            watcher = tv_dict_watcher_node_data(w);
            if (*watcher).busy {
                queue_is_busy = true;
            }
            if tv_callback_equal(&raw const (*watcher).callback, &raw const callback)
                && (*watcher).key_pattern_len == key_pattern_len
                && std::slice::from_raw_parts((*watcher).key_pattern.cast::<u8>(), key_pattern_len)
                    == std::slice::from_raw_parts(key_pattern.cast::<u8>(), key_pattern_len)
            {
                matched = true;
                break;
            }
            w = next;
        }

        if !matched {
            return false;
        }

        if queue_is_busy {
            (*watcher).needs_free = true;
        } else {
            queue_remove(w);
            tv_dict_watcher_free(watcher);
        }
        true
    }
}

/// Test if `key` matches with `watcher->key_pattern`.
unsafe fn tv_dict_watcher_matches(watcher: *mut DictWatcher, key: *const c_char) -> bool {
    unsafe {
        let len = (*watcher).key_pattern_len;
        if len > 0 && *(*watcher).key_pattern.add(len - 1) == b'*' as c_char {
            strncmp(key, (*watcher).key_pattern, len - 1) == 0
        } else {
            strcmp(key, (*watcher).key_pattern) == 0
        }
    }
}

/// Send a change notification to all dictionary watchers that match given key.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_watcher_notify(
    dict: *mut dict_T,
    key: *const c_char,
    newtv: *mut typval_T,
    oldtv: *mut typval_T,
) {
    unsafe {
        let mut argv: [typval_T; 3] = [tv_initial_value(); 3];

        argv[0].v_type = VarType::VAR_DICT;
        argv[0].v_lock = VAR_UNLOCKED;
        argv[0].vval.v_dict = dict;
        argv[1].v_type = VarType::VAR_STRING;
        argv[1].v_lock = VAR_UNLOCKED;
        argv[1].vval.v_string = xstrdup(key);
        argv[2].v_type = VarType::VAR_DICT;
        argv[2].v_lock = VAR_UNLOCKED;
        let changes_dict = tv_dict_alloc();
        argv[2].vval.v_dict = changes_dict;
        (*changes_dict).dv_refcount += 1;

        if !newtv.is_null() {
            let v = tv_dict_item_alloc_len(c"new".as_ptr(), 3);
            tv_copy(newtv, &raw mut (*v).di_tv);
            tv_dict_add(changes_dict, v);
        }

        if !oldtv.is_null() && (*oldtv).v_type != VarType::VAR_UNKNOWN {
            let v = tv_dict_item_alloc_len(c"old".as_ptr(), 3);
            tv_copy(oldtv, &raw mut (*v).di_tv);
            tv_dict_add(changes_dict, v);
        }

        let mut any_needs_free = false;
        (*dict).dv_refcount += 1;
        let head = dict_watchers_queue(dict);
        let mut w = (*head).next;
        while w != head {
            let next = (*w).next;
            let watcher = tv_dict_watcher_node_data(w);
            if !(*watcher).busy && tv_dict_watcher_matches(watcher, key) {
                let mut rettv = tv_initial_value();
                (*watcher).busy = true;
                callback_call(
                    &raw mut (*watcher).callback,
                    3,
                    argv.as_mut_ptr(),
                    &raw mut rettv,
                );
                (*watcher).busy = false;
                tv_clear(&raw mut rettv);
                if (*watcher).needs_free {
                    any_needs_free = true;
                }
            }
            w = next;
        }
        if any_needs_free {
            let mut w = (*head).next;
            while w != head {
                let next = (*w).next;
                let watcher = tv_dict_watcher_node_data(w);
                if (*watcher).needs_free {
                    queue_remove(w);
                    tv_dict_watcher_free(watcher);
                }
                w = next;
            }
        }
        tv_dict_unref(dict);

        for item in argv.iter_mut().skip(1) {
            tv_clear(item);
        }
    }
}

/// Allocate a dictionary item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_item_alloc_len(
    key: *const c_char,
    key_len: usize,
) -> *mut dictitem_T {
    unsafe {
        let alloc_size = size_of::<dictitem_T>().max(offset_of!(dictitem_T, di_key) + key_len + 1);
        let di = xmalloc(alloc_size).cast::<dictitem_T>();
        let key_dst = (&raw mut (*di).di_key).cast::<c_char>();
        ptr::copy_nonoverlapping(key, key_dst, key_len);
        *key_dst.add(key_len) = NUL;
        (*di).di_flags = DI_FLAGS_ALLOC;
        (*di).di_tv.v_lock = VAR_UNLOCKED;
        (*di).di_tv.v_type = VarType::VAR_UNKNOWN;
        di
    }
}

/// Allocate a dictionary item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_item_alloc(key: *const c_char) -> *mut dictitem_T {
    unsafe { tv_dict_item_alloc_len(key, c_strlen(key)) }
}

/// Free a dictionary item, also clearing the value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_item_free(item: *mut dictitem_T) {
    unsafe {
        tv_clear(&raw mut (*item).di_tv);
        if ((*item).di_flags & DI_FLAGS_ALLOC) != 0 {
            xfree(item.cast());
        }
    }
}

/// Make a copy of a dictionary item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_item_copy(di: *mut dictitem_T) -> *mut dictitem_T {
    unsafe {
        let key_ptr = (&raw const (*di).di_key).cast::<c_char>();
        let new_di = tv_dict_item_alloc(key_ptr);
        tv_copy(&raw const (*di).di_tv, &raw mut (*new_di).di_tv);
        new_di
    }
}

/// Remove item from dictionary and free it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_item_remove(dict: *mut dict_T, item: *mut dictitem_T) {
    unsafe {
        let key_ptr = (&raw const (*item).di_key).cast::<c_char>();
        let hi = hash_find(&raw const (*dict).dv_hashtab, key_ptr);
        if hashitem_empty(&*hi) {
            #[cfg(not(test))]
            semsg!(tr(e_intern2.as_ptr()), c"tv_dict_item_remove()".as_ptr());
        } else {
            hash_remove(&raw mut (*dict).dv_hashtab, hi);
        }
        tv_dict_item_free(item);
    }
}

/// Allocate an empty dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_alloc() -> *mut dict_T {
    unsafe {
        let d = xcalloc(1, size_of::<dict_T>()).cast::<dict_T>();

        if !gc_first_dict.is_null() {
            (*gc_first_dict.cast::<dict_T>()).dv_used_prev = d;
        }
        (*d).dv_used_next = gc_first_dict.cast::<dict_T>();
        (*d).dv_used_prev = ptr::null_mut();
        gc_first_dict = d.cast::<c_void>();

        hash_init(&raw mut (*d).dv_hashtab);
        (*d).dv_lock = VAR_UNLOCKED;
        (*d).dv_scope = VAR_NO_SCOPE;
        (*d).dv_refcount = 0;
        (*d).dv_copyID = 0;
        queue_init(dict_watchers_queue(d));

        (*d).lua_table_ref = LUA_NOREF;

        d
    }
}

/// Free items contained in a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_free_contents(d: *mut dict_T) {
    unsafe {
        hash_lock(&raw mut (*d).dv_hashtab);
        debug_assert!((*d).dv_hashtab.ht_locked > 0);
        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                hash_remove(&raw mut (*d).dv_hashtab, hi);
                tv_dict_item_free(di);
            }
            hi = hi.add(1);
        }

        let head = dict_watchers_queue(d);
        while !queue_empty(head) {
            let w = (*head).next;
            queue_remove(w);
            let watcher = tv_dict_watcher_node_data(w);
            tv_dict_watcher_free(watcher);
        }

        hash_clear(&raw mut (*d).dv_hashtab);
        (*d).dv_hashtab.ht_locked -= 1;
        hash_init(&raw mut (*d).dv_hashtab);
    }
}

/// Free a dictionary itself, ignoring items it contains.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_free_dict(d: *mut dict_T) {
    unsafe {
        if (*d).dv_used_prev.is_null() {
            gc_first_dict = (*d).dv_used_next.cast::<c_void>();
        } else {
            (*(*d).dv_used_prev).dv_used_next = (*d).dv_used_next;
        }
        if !(*d).dv_used_next.is_null() {
            (*(*d).dv_used_next).dv_used_prev = (*d).dv_used_prev;
        }

        nlua_clear_ref(&mut (*d).lua_table_ref);
        xfree(d.cast());
    }
}

/// Free a dictionary, including all items it contains.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_free(d: *mut dict_T) {
    unsafe {
        if tv_in_free_unref_items {
            return;
        }

        tv_dict_free_contents(d);
        tv_dict_free_dict(d);
    }
}

/// Unreference a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_unref(d: *mut dict_T) {
    unsafe {
        if !d.is_null() {
            (*d).dv_refcount -= 1;
            if (*d).dv_refcount <= 0 {
                tv_dict_free(d);
            }
        }
    }
}

/// Find item in dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_find(
    d: *const dict_T,
    key: *const c_char,
    len: isize,
) -> *mut dictitem_T {
    unsafe {
        if d.is_null() {
            return ptr::null_mut();
        }
        let hi = if len < 0 {
            hash_find(&raw const (*d).dv_hashtab, key)
        } else {
            hash_find_len(&raw const (*d).dv_hashtab, key, len as usize)
        };
        if hashitem_empty(&*hi) {
            return ptr::null_mut();
        }
        tv_dict_hi2di(hi)
    }
}

/// Check if a key is present in a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_has_key(d: *const dict_T, key: *const c_char) -> bool {
    unsafe { !tv_dict_find(d, key, -1).is_null() }
}

/// Get a typval item from a dictionary and copy it into "rettv".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_tv(
    d: *mut dict_T,
    key: *const c_char,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return FAIL;
        }

        tv_copy(&raw const (*di).di_tv, rettv);
        OK
    }
}

/// Gets a number item from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_number(d: *const dict_T, key: *const c_char) -> varnumber_T {
    unsafe { tv_dict_get_number_def(d, key, 0) }
}

/// Gets a number item from a dictionary, or a given default value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_number_def(
    d: *const dict_T,
    key: *const c_char,
    def: c_int,
) -> varnumber_T {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return i64::from(def);
        }
        tv_get_number(&raw const (*di).di_tv)
    }
}

/// Gets a bool item from a dictionary, or a given default value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_bool(
    d: *const dict_T,
    key: *const c_char,
    def: c_int,
) -> varnumber_T {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return i64::from(def);
        }
        tv_get_bool(&raw const (*di).di_tv)
    }
}

/// Converts a dict to an environment.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_to_env(denv: *mut dict_T) -> *mut *mut c_char {
    unsafe {
        let env_size = tv_dict_len_inline(denv) as usize;
        let mut i: usize = 0;
        let env = xmalloc((env_size + 1) * size_of::<*mut c_char>()).cast::<*mut c_char>();

        if !denv.is_null() {
            let mut todo = (*denv).dv_hashtab.ht_used;
            let mut hi = (*denv).dv_hashtab.ht_array;
            while todo > 0 {
                if !hashitem_empty(&*hi) {
                    todo -= 1;
                    let var = tv_dict_hi2di(hi);
                    let str_ = tv_get_string(&raw const (*var).di_tv);
                    debug_assert!(!str_.is_null());
                    let key_ptr = (&raw const (*var).di_key).cast::<c_char>();
                    let key_len = c_strlen(key_ptr);
                    let val_len = c_strlen(str_);
                    let len = key_len + val_len + 2;
                    let entry = xmalloc(len).cast::<c_char>();
                    ptr::copy_nonoverlapping(key_ptr, entry, key_len);
                    *entry.add(key_len) = b'=' as c_char;
                    ptr::copy_nonoverlapping(str_, entry.add(key_len + 1), val_len + 1);
                    *env.add(i) = entry;
                    i += 1;
                }
                hi = hi.add(1);
            }
        }

        *env.add(env_size) = ptr::null_mut();
        env
    }
}

/// Get a string item from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_string(
    d: *const dict_T,
    key: *const c_char,
    save: bool,
) -> *mut c_char {
    unsafe {
        static mut NUMBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        let s = tv_dict_get_string_buf(d, key, (&raw mut NUMBUF).cast::<c_char>());
        if save && !s.is_null() {
            return xstrdup(s);
        }
        s.cast_mut()
    }
}

/// Get a string item from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_string_buf(
    d: *const dict_T,
    key: *const c_char,
    numbuf: *mut c_char,
) -> *const c_char {
    unsafe {
        let di = tv_dict_find(d, key, -1);
        if di.is_null() {
            return ptr::null();
        }
        tv_get_string_buf(&raw const (*di).di_tv, numbuf)
    }
}

/// Get a string item from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_string_buf_chk(
    d: *const dict_T,
    key: *const c_char,
    key_len: isize,
    numbuf: *mut c_char,
    def: *const c_char,
) -> *const c_char {
    unsafe {
        let di = tv_dict_find(d, key, key_len);
        if di.is_null() {
            return def;
        }
        tv_get_string_buf_chk(&raw const (*di).di_tv, numbuf)
    }
}

/// Get a function from a dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_get_callback(
    d: *mut dict_T,
    key: *const c_char,
    key_len: isize,
    result: *mut Callback,
) -> bool {
    unsafe {
        (*result).cb_type = kCallbackNone;

        let di = tv_dict_find(d, key, key_len);
        if di.is_null() {
            return true;
        }

        if !tv_is_func_inline((*di).di_tv) && (*di).di_tv.v_type != VarType::VAR_STRING {
            emsg(tr(
                c"E6000: Argument is not a function or function name".as_ptr()
            ));
            return false;
        }

        let mut tv = tv_initial_value();
        tv_copy(&raw const (*di).di_tv, &raw mut tv);
        set_selfdict(&raw mut tv, d);
        let res = callback_from_typval(result, &raw mut tv);
        tv_clear(&raw mut tv);
        res
    }
}

/// Check for adding a function to g: or l:.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_wrong_func_name(
    d: *mut dict_T,
    tv: *mut typval_T,
    name: *const c_char,
) -> c_int {
    #[cfg(not(test))]
    unsafe {
        c_int::from(
            (d == get_globvar_dict()
                || ptr::eq(&raw const (*d).dv_hashtab, get_funccal_local_ht()))
                && tv_is_func_inline(*tv)
                && var_wrong_func_name(name, true),
        )
    }
    #[cfg(test)]
    {
        let _ = (d, tv, name);
        0
    }
}

/// Add item to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add(d: *mut dict_T, item: *mut dictitem_T) -> c_int {
    unsafe {
        let key_ptr = (&raw mut (*item).di_key).cast::<c_char>();
        if tv_dict_wrong_func_name(d, &raw mut (*item).di_tv, key_ptr) != 0 {
            return FAIL;
        }
        hash_add(&raw mut (*d).dv_hashtab, key_ptr)
    }
}

/// Add a list entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_list(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    list: *mut list_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_LIST;
        (*item).di_tv.vval.v_list = list;
        if tv_dict_add(d, item) == FAIL {
            (*item).di_tv.vval.v_list = ptr::null_mut();
            tv_dict_item_free(item);
            return FAIL;
        }
        tv_list_ref_inline(list);
        OK
    }
}

/// Add a typval entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_tv(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    tv: *mut typval_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        tv_copy(tv, &raw mut (*item).di_tv);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a dictionary entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_dict(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    dict: *mut dict_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_DICT;
        (*item).di_tv.vval.v_dict = dict;
        if tv_dict_add(d, item) == FAIL {
            (*item).di_tv.vval.v_dict = ptr::null_mut();
            tv_dict_item_free(item);
            return FAIL;
        }
        (*dict).dv_refcount += 1;
        OK
    }
}

/// Add a number entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_nr(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    nr: varnumber_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_NUMBER;
        (*item).di_tv.vval.v_number = nr;
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a floating point number entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_float(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    nr: float_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_FLOAT;
        (*item).di_tv.vval.v_float = nr;
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a boolean entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_bool(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: BoolVarValue,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_BOOL;
        (*item).di_tv.vval.v_bool = val;
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a string entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_str(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: *const c_char,
) -> c_int {
    unsafe { tv_dict_add_str_len(d, key, key_len, val, -1) }
}

/// Add a string entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_str_len(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: *const c_char,
    len: c_int,
) -> c_int {
    unsafe {
        let s = if val.is_null() {
            ptr::null_mut()
        } else if len < 0 {
            xstrdup(val)
        } else {
            xstrndup(val, len as usize)
        };
        tv_dict_add_allocated_str(d, key, key_len, s)
    }
}

/// Add a string entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_allocated_str(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    val: *mut c_char,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_STRING;
        (*item).di_tv.vval.v_string = val;
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Add a function entry to dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_add_func(
    d: *mut dict_T,
    key: *const c_char,
    key_len: usize,
    fp: *mut ufunc_T,
) -> c_int {
    unsafe {
        let item = tv_dict_item_alloc_len(key, key_len);
        (*item).di_tv.v_type = VarType::VAR_FUNC;
        let name_ptr = (&raw const (*fp).uf_name).cast::<c_char>();
        (*item).di_tv.vval.v_string = xmemdupz(name_ptr.cast(), (*fp).uf_namelen).cast::<c_char>();
        func_ref((*item).di_tv.vval.v_string);
        if tv_dict_add(d, item) == FAIL {
            tv_dict_item_free(item);
            return FAIL;
        }
        OK
    }
}

/// Clear all the keys of a Dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_clear(d: *mut dict_T) {
    unsafe {
        hash_lock(&raw mut (*d).dv_hashtab);
        debug_assert!((*d).dv_hashtab.ht_locked > 0);

        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                tv_dict_item_free(tv_dict_hi2di(hi));
                hash_remove(&raw mut (*d).dv_hashtab, hi);
            }
            hi = hi.add(1);
        }

        hash_unlock(&raw mut (*d).dv_hashtab);
    }
}

/// Extend dictionary with items from another dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_extend(d1: *mut dict_T, d2: *mut dict_T, action: *const c_char) {
    unsafe {
        let watched = tv_dict_is_watched_inline(d1);
        let arg_errmsg = tr(c"extend() argument".as_ptr());
        let arg_errmsg_len = c_strlen(arg_errmsg);

        if *action == b'm' as c_char {
            hash_lock(&raw mut (*d2).dv_hashtab);
        }

        let mut todo = (*d2).dv_hashtab.ht_used;
        let mut hi2 = (*d2).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi2) {
                todo -= 1;
                let di2 = tv_dict_hi2di(hi2);
                let key2 = (&raw const (*di2).di_key).cast::<c_char>();
                let di1 = tv_dict_find(d1, key2, -1);
                if (*d1).dv_scope != VAR_NO_SCOPE && !valid_varname(key2) {
                    break;
                }
                if di1.is_null() {
                    if *action == b'm' as c_char {
                        let new_di = di2;
                        if tv_dict_add(d1, new_di) == OK {
                            hash_remove(&raw mut (*d2).dv_hashtab, hi2);
                            tv_dict_watcher_notify(
                                d1,
                                (&raw const (*new_di).di_key).cast::<c_char>(),
                                &raw mut (*new_di).di_tv,
                                ptr::null_mut(),
                            );
                        }
                    } else {
                        let new_di = tv_dict_item_copy(di2);
                        if tv_dict_add(d1, new_di) == FAIL {
                            tv_dict_item_free(new_di);
                        } else if watched {
                            tv_dict_watcher_notify(
                                d1,
                                (&raw const (*new_di).di_key).cast::<c_char>(),
                                &raw mut (*new_di).di_tv,
                                ptr::null_mut(),
                            );
                        }
                    }
                } else if *action == b'e' as c_char {
                    semsg!(tr(c"E737: Key already exists: %s".as_ptr()), key2);
                    break;
                } else if *action == b'f' as c_char && di2 != di1 {
                    if value_check_lock((*di1).di_tv.v_lock, arg_errmsg, arg_errmsg_len)
                        || var_check_ro(c_int::from((*di1).di_flags), arg_errmsg, arg_errmsg_len)
                    {
                        break;
                    }
                    if tv_dict_wrong_func_name(d1, &raw mut (*di2).di_tv, key2) != 0 {
                        break;
                    }

                    let mut oldtv = tv_initial_value();
                    if watched {
                        tv_copy(&raw const (*di1).di_tv, &raw mut oldtv);
                    }

                    tv_clear(&raw mut (*di1).di_tv);
                    tv_copy(&raw const (*di2).di_tv, &raw mut (*di1).di_tv);

                    if watched {
                        tv_dict_watcher_notify(
                            d1,
                            (&raw const (*di1).di_key).cast::<c_char>(),
                            &raw mut (*di1).di_tv,
                            &raw mut oldtv,
                        );
                        tv_clear(&raw mut oldtv);
                    }
                }
            }
            hi2 = hi2.add(1);
        }

        if *action == b'm' as c_char {
            hash_unlock(&raw mut (*d2).dv_hashtab);
        }
    }
}

/// Compare two dictionaries.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_equal(d1: *mut dict_T, d2: *mut dict_T, ic: bool) -> bool {
    unsafe {
        if d1 == d2 {
            return true;
        }
        if tv_dict_len_inline(d1) != tv_dict_len_inline(d2) {
            return false;
        }
        if tv_dict_len_inline(d1) == 0 {
            return true;
        }
        if d1.is_null() || d2.is_null() {
            return false;
        }

        let mut todo = (*d1).dv_hashtab.ht_used;
        let mut hi = (*d1).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di1 = tv_dict_hi2di(hi);
                let key1 = (&raw const (*di1).di_key).cast::<c_char>();
                let di2 = tv_dict_find(d2, key1, -1);
                if di2.is_null() {
                    return false;
                }
                if !tv_equal(&raw mut (*di1).di_tv, &raw mut (*di2).di_tv, ic) {
                    return false;
                }
            }
            hi = hi.add(1);
        }
        true
    }
}

/// Make a copy of dictionary.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_copy(
    conv: *const vimconv_T,
    orig: *mut dict_T,
    deep: bool,
    copyID: c_int,
) -> *mut dict_T {
    unsafe {
        if orig.is_null() {
            return ptr::null_mut();
        }

        let mut copy = tv_dict_alloc();
        if copyID != 0 {
            (*orig).dv_copyID = copyID;
            (*orig).dv_copydict = copy;
        }
        let mut todo = (*orig).dv_hashtab.ht_used;
        let mut hi = (*orig).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                if got_int {
                    break;
                }
                let di = tv_dict_hi2di(hi);
                let key_ptr = (&raw mut (*di).di_key).cast::<c_char>();
                let new_di = if conv.is_null() || (*conv).vc_type == CONV_NONE {
                    tv_dict_item_alloc(key_ptr)
                } else {
                    let mut len = c_strlen(key_ptr);
                    let key = string_convert(conv, key_ptr, &raw mut len);
                    if key.is_null() {
                        tv_dict_item_alloc_len(key_ptr, len)
                    } else {
                        let item = tv_dict_item_alloc_len(key, len);
                        xfree(key.cast());
                        item
                    }
                };
                if deep {
                    if var_item_copy(
                        conv.cast(),
                        &raw mut (*di).di_tv,
                        &raw mut (*new_di).di_tv,
                        deep,
                        copyID,
                    ) == FAIL
                    {
                        xfree(new_di.cast());
                        break;
                    }
                } else {
                    tv_copy(&raw const (*di).di_tv, &raw mut (*new_di).di_tv);
                }
                if tv_dict_add(copy, new_di) == FAIL {
                    tv_dict_item_free(new_di);
                    break;
                }
            }
            hi = hi.add(1);
        }

        (*copy).dv_refcount += 1;
        if got_int {
            tv_dict_unref(copy);
            copy = ptr::null_mut();
        }

        copy
    }
}

/// Set all existing keys in "dict" as read-only.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_set_keys_readonly(dict: *mut dict_T) {
    unsafe {
        let mut todo = (*dict).dv_hashtab.ht_used;
        let mut hi = (*dict).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                (*di).di_flags |= DI_FLAGS_RO | DI_FLAGS_FIX;
            }
            hi = hi.add(1);
        }
    }
}

/// Allocate an empty blob.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_alloc() -> *mut blob_T {
    unsafe {
        let blob = xcalloc(1, size_of::<blob_T>()).cast::<blob_T>();
        ga_init(&raw mut (*blob).bv_ga, 1, 100);
        blob
    }
}

/// Free a blob. Ignores the reference count.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_free(b: *mut blob_T) {
    unsafe {
        ga_clear(&raw mut (*b).bv_ga);
        xfree(b.cast());
    }
}

/// Unreference a blob.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_unref(b: *mut blob_T) {
    unsafe {
        if !b.is_null() {
            (*b).bv_refcount -= 1;
            if (*b).bv_refcount <= 0 {
                tv_blob_free(b);
            }
        }
    }
}

/// Check whether two blobs are equal.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_equal(b1: *const blob_T, b2: *const blob_T) -> bool {
    unsafe {
        let len1 = tv_blob_len_inline(b1);
        let len2 = tv_blob_len_inline(b2);

        if len1 == 0 && len2 == 0 {
            return true;
        }
        if ptr::eq(b1, b2) {
            return true;
        }
        if len1 != len2 {
            return false;
        }

        for i in 0..(*b1).bv_ga.ga_len {
            if tv_blob_get_inline(b1, i) != tv_blob_get_inline(b2, i) {
                return false;
            }
        }
        true
    }
}

unsafe fn tv_blob_slice(
    _blob: *const blob_T,
    len: c_int,
    mut n1: varnumber_T,
    mut n2: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let len64 = i64::from(len);
        if n1 < 0 {
            n1 += len64;
            if n1 < 0 {
                n1 = 0;
            }
        }
        if n2 < 0 {
            n2 += len64;
        } else if n2 >= len64 {
            n2 = len64 - if exclusive { 0 } else { 1 };
        }
        if exclusive {
            n2 -= 1;
        }
        if n1 >= len64 || n2 < 0 || n1 > n2 {
            tv_clear(rettv);
            (*rettv).v_type = VarType::VAR_BLOB;
            (*rettv).vval.v_blob = ptr::null_mut();
        } else {
            let new_blob = tv_blob_alloc();
            let slice_len = (n2 - n1 + 1) as c_int;
            ga_grow(&raw mut (*new_blob).bv_ga, slice_len);
            (*new_blob).bv_ga.ga_len = slice_len;
            for i in (n1 as c_int)..=(n2 as c_int) {
                tv_blob_set_inline(
                    new_blob,
                    i - (n1 as c_int),
                    tv_blob_get_inline((*rettv).vval.v_blob, i),
                );
            }
            tv_clear(rettv);
            tv_blob_set_ret_inline(rettv, new_blob);
        }

        OK
    }
}

unsafe fn tv_blob_index(
    _blob: *const blob_T,
    len: c_int,
    mut idx: varnumber_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let len64 = i64::from(len);
        if idx < 0 {
            idx += len64;
        }
        if idx < len64 && idx >= 0 {
            let v = i64::from(tv_blob_get_inline((*rettv).vval.v_blob, idx as c_int));
            tv_clear(rettv);
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = v;
        } else {
            semsg!(tr(e_blobidx.as_ptr()), idx);
            return FAIL;
        }

        OK
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_slice_or_index(
    blob: *const blob_T,
    is_range: bool,
    n1: varnumber_T,
    n2: varnumber_T,
    exclusive: bool,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let len = tv_blob_len_inline((*rettv).vval.v_blob);
        if is_range {
            tv_blob_slice(blob, len, n1, n2, exclusive, rettv)
        } else {
            tv_blob_index(blob, len, n1, rettv)
        }
    }
}

/// Check if "n1" is a valid index for a blob with length "bloblen".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_check_index(
    bloblen: c_int,
    n1: varnumber_T,
    quiet: bool,
) -> c_int {
    unsafe {
        if n1 < 0 || n1 > i64::from(bloblen) {
            if !quiet {
                semsg!(tr(e_blobidx.as_ptr()), n1);
            }
            return FAIL;
        }
        OK
    }
}

/// Check if "n1"-"n2" is a valid range for a blob with length "bloblen".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_check_range(
    bloblen: c_int,
    n1: varnumber_T,
    n2: varnumber_T,
    quiet: bool,
) -> c_int {
    unsafe {
        if n2 < 0 || n2 >= i64::from(bloblen) || n2 < n1 {
            if !quiet {
                semsg!(tr(e_blobidx.as_ptr()), n2);
            }
            return FAIL;
        }
        OK
    }
}

/// Set bytes "n1" to "n2" (inclusive) in "dest" to the value of "src".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_set_range(
    dest: *mut blob_T,
    n1: varnumber_T,
    n2: varnumber_T,
    src: *mut typval_T,
) -> c_int {
    unsafe {
        if n2 - n1 + 1 != i64::from(tv_blob_len_inline((*src).vval.v_blob)) {
            emsg(tr(
                c"E972: Blob value does not have the right number of bytes".as_ptr(),
            ));
            return FAIL;
        }

        for (ir, il) in (0_i32..).zip((n1 as c_int)..=(n2 as c_int)) {
            tv_blob_set_inline(dest, il, tv_blob_get_inline((*src).vval.v_blob, ir));
        }
        OK
    }
}

/// Store one byte "byte" in blob "blob" at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_set_append(blob: *mut blob_T, idx: c_int, byte: u8) {
    unsafe {
        let gap = &raw mut (*blob).bv_ga;
        if idx <= (*gap).ga_len {
            if idx == (*gap).ga_len {
                ga_grow(gap, 1);
                (*gap).ga_len += 1;
            }
            tv_blob_set_inline(blob, idx, byte);
        }
    }
}

/// "remove({blob})" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        let b = (*argvars).vval.v_blob;
        if !b.is_null() && value_check_lock((*b).bv_lock, arg_errmsg, TV_TRANSLATE) {
            return;
        }

        let mut error = false;
        let mut idx = tv_get_number_chk(argvars.add(1), &raw mut error);

        if !error {
            let len = i64::from(tv_blob_len_inline(b));
            if idx < 0 {
                idx += len;
            }
            if idx < 0 || idx >= len {
                semsg!(tr(e_blobidx.as_ptr()), idx);
                return;
            }
            if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
                let p = (*b).bv_ga.ga_data.cast::<u8>();
                (*rettv).vval.v_number = i64::from(*p.offset(idx as isize));
                ptr::copy(
                    p.offset(idx as isize + 1),
                    p.offset(idx as isize),
                    (len - idx - 1) as usize,
                );
                (*b).bv_ga.ga_len -= 1;
            } else {
                let mut end = tv_get_number_chk(argvars.add(2), &raw mut error);
                if error {
                    return;
                }
                if end < 0 {
                    end += len;
                }
                if end >= len || idx > end {
                    semsg!(tr(e_blobidx.as_ptr()), end);
                    return;
                }
                let blob = tv_blob_alloc();
                let count = (end - idx + 1) as c_int;
                (*blob).bv_ga.ga_len = count;
                ga_grow(&raw mut (*blob).bv_ga, count);

                let p = (*b).bv_ga.ga_data.cast::<u8>();
                ptr::copy(
                    p.offset(idx as isize),
                    (*blob).bv_ga.ga_data.cast::<u8>(),
                    count as usize,
                );
                tv_blob_set_ret_inline(rettv, blob);

                if len - end - 1 > 0 {
                    ptr::copy(
                        p.offset(end as isize + 1),
                        p.offset(idx as isize),
                        (len - end - 1) as usize,
                    );
                }
                (*b).bv_ga.ga_len -= count;
            }
        }
    }
}

/// blob2list() function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_blob2list(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if tv_check_for_blob_arg(argvars, 0) == FAIL {
            return;
        }

        let blob = (*argvars).vval.v_blob;
        let l = (*rettv).vval.v_list;
        for i in 0..tv_blob_len_inline(blob) {
            tv_list_append_number(l, i64::from(tv_blob_get_inline(blob, i)));
        }
    }
}

/// list2blob() function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_list2blob(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let blob = tv_blob_alloc_ret(rettv);

        if tv_check_for_list_arg(argvars, 0) == FAIL {
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        ga_grow(&raw mut (*blob).bv_ga, tv_list_len_inline(l));

        let mut li = (*l).lv_first;
        while !li.is_null() {
            let mut error = false;
            let n = tv_get_number_chk(&raw const (*li).li_tv, &raw mut error);
            if error || !(0..=255).contains(&n) {
                if !error {
                    semsg!(tr(e_invalid_value_for_blob_nr.as_ptr()), n as c_int);
                }
                ga_clear(&raw mut (*blob).bv_ga);
                return;
            }
            let len = (*blob).bv_ga.ga_len;
            *(*blob).bv_ga.ga_data.cast::<u8>().offset(len as isize) = n as u8;
            (*blob).bv_ga.ga_len += 1;
            li = (*li).li_next;
        }
    }
}

/// Allocate an empty list for a return value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_list_alloc_ret(ret_tv: *mut typval_T, len: isize) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(len);
        tv_list_set_ret_inline(ret_tv, l);
        (*ret_tv).v_lock = VAR_UNLOCKED;
        l
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_alloc_lock(lock: VarLockStatus) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();
        (*d).dv_lock = lock;
        d
    }
}

/// Allocate an empty dictionary for a return value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_alloc_ret(ret_tv: *mut typval_T) {
    unsafe {
        let d = tv_dict_alloc_lock(VAR_UNLOCKED);
        tv_dict_set_ret_inline(ret_tv, d);
    }
}

unsafe fn tv_dict2list(argvars: *mut typval_T, rettv: *mut typval_T, what: DictListType) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            tv_list_alloc_ret(rettv, 0);
            return;
        }

        let d = (*argvars).vval.v_dict;
        tv_list_alloc_ret(rettv, tv_dict_len_inline(d));
        if d.is_null() {
            return;
        }

        let mut todo = (*d).dv_hashtab.ht_used;
        let mut hi = (*d).dv_hashtab.ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let di = tv_dict_hi2di(hi);
                let key_ptr = (&raw const (*di).di_key).cast::<c_char>();
                let mut tv_item = tv_initial_value();

                match what {
                    DictListType::Keys => {
                        tv_item.v_type = VarType::VAR_STRING;
                        tv_item.vval.v_string = xstrdup(key_ptr);
                    }
                    DictListType::Values => {
                        tv_copy(&raw const (*di).di_tv, &raw mut tv_item);
                    }
                    DictListType::Items => {
                        let sub_l = tv_list_alloc(2);
                        tv_item.v_type = VarType::VAR_LIST;
                        tv_item.vval.v_list = sub_l;
                        tv_list_ref_inline(sub_l);
                        tv_list_append_string(sub_l, key_ptr, -1);
                        tv_list_append_tv(sub_l, &raw mut (*di).di_tv);
                    }
                }

                tv_list_append_owned_tv((*rettv).vval.v_list, tv_item);
            }
            hi = hi.add(1);
        }
    }
}

/// "items()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_items(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        match (*argvars).v_type {
            VarType::VAR_STRING => tv_string2items(argvars, rettv),
            VarType::VAR_LIST => tv_list2items(argvars, rettv),
            VarType::VAR_BLOB => tv_blob2items(argvars, rettv),
            VarType::VAR_DICT => tv_dict2items(argvars, rettv),
            _ => {
                semsg!(
                    tr(E_LIST_DICT_BLOB_OR_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                    1,
                );
            }
        }
    }
}

/// "keys()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_keys(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Keys);
    }
}

/// "values(dict)" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_values(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict2list(argvars, rettv, DictListType::Values);
    }
}

/// "has_key()" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_has_key(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            return;
        }

        if (*argvars).vval.v_dict.is_null() {
            return;
        }

        (*rettv).vval.v_number = i64::from(
            !tv_dict_find((*argvars).vval.v_dict, tv_get_string(argvars.add(1)), -1).is_null(),
        );
    }
}

/// "remove({dict})" function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_dict_remove(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_errmsg: *const c_char,
) {
    unsafe {
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            semsg!(tr(e_toomanyarg.as_ptr()), c"remove()".as_ptr());
        } else {
            let d = (*argvars).vval.v_dict;
            if !d.is_null() && !value_check_lock((*d).dv_lock, arg_errmsg, TV_TRANSLATE) {
                let key = tv_get_string_chk(argvars.add(1));
                if !key.is_null() {
                    let di = tv_dict_find(d, key, -1);
                    if di.is_null() {
                        semsg!(tr(e_dictkey.as_ptr()), key);
                    } else if !var_check_fixed(
                        c_int::from((*di).di_flags),
                        arg_errmsg,
                        TV_TRANSLATE,
                    ) && !var_check_ro(
                        c_int::from((*di).di_flags),
                        arg_errmsg,
                        TV_TRANSLATE,
                    ) {
                        *rettv = (*di).di_tv;
                        (*di).di_tv = tv_initial_value();
                        tv_dict_item_remove(d, di);
                        if tv_dict_is_watched_inline(d) {
                            tv_dict_watcher_notify(d, key, ptr::null_mut(), rettv);
                        }
                    }
                }
            }
        }
    }
}

/// Allocate an empty blob for a return value.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_alloc_ret(ret_tv: *mut typval_T) -> *mut blob_T {
    unsafe {
        let b = tv_blob_alloc();
        tv_blob_set_ret_inline(ret_tv, b);
        b
    }
}

/// Copy a blob typval to a different typval.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_blob_copy(from: *mut blob_T, to: *mut typval_T) {
    unsafe {
        (*to).v_type = VarType::VAR_BLOB;
        (*to).v_lock = VAR_UNLOCKED;
        if from.is_null() {
            (*to).vval.v_blob = ptr::null_mut();
        } else {
            tv_blob_alloc_ret(to);
            let len = (*from).bv_ga.ga_len;
            let dst_blob = (*to).vval.v_blob;
            if len > 0 {
                (*dst_blob).bv_ga.ga_data = xmemdup((*from).bv_ga.ga_data, len as usize);
            }
            (*dst_blob).bv_ga.ga_len = len;
            (*dst_blob).bv_ga.ga_maxlen = len;
        }
    }
}

#[inline]
unsafe fn nothing_conv_func_start(tv: *mut typval_T, fun: *mut c_char) -> c_int {
    unsafe {
        (*tv).v_lock = VAR_UNLOCKED;
        if (*tv).v_type == VarType::VAR_PARTIAL {
            let pt = (*tv).vval.v_ptr.cast::<partial_T>();
            if !pt.is_null() && (*pt).pt_refcount > 1 {
                (*pt).pt_refcount -= 1;
                (*tv).vval.v_ptr = ptr::null_mut();
                return OK;
            }
        } else {
            #[cfg(not(test))]
            func_unref(fun);
            if !ptr::eq(fun.cast_const(), tv_empty_string.0) {
                xfree(fun.cast());
            }
            (*tv).vval.v_string = ptr::null_mut();
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_func_end(tv: *mut typval_T, copy_id: c_int) {
    unsafe {
        if (*tv).v_type == VarType::VAR_PARTIAL {
            let pt = (*tv).vval.v_ptr.cast::<partial_T>();
            if pt.is_null() {
                return;
            }
            let dict = (*pt).pt_dict.cast::<dict_T>();
            debug_assert!(dict.is_null() || (*dict).dv_copyID == copy_id);
            (*pt).pt_dict = ptr::null_mut();
            (*pt).pt_argc = 0;
            debug_assert!((*pt).pt_refcount <= 1);
            #[cfg(not(test))]
            partial_unref(pt);
            (*tv).vval.v_ptr = ptr::null_mut();
            debug_assert_eq!((*tv).v_lock, VAR_UNLOCKED);
        }
    }
}

#[inline]
unsafe fn nothing_conv_empty_dict(tv: *mut typval_T, dictp: *mut *mut dict_T) {
    unsafe {
        tv_dict_unref(*dictp);
        *dictp = ptr::null_mut();
        if !tv.is_null() {
            (*tv).v_lock = VAR_UNLOCKED;
        }
    }
}

#[inline]
unsafe fn nothing_conv_real_list_after_start(
    tv: *mut typval_T,
    mpsv: &mut MPConvStackVal,
) -> c_int {
    unsafe {
        debug_assert!(!tv.is_null());
        (*tv).v_lock = VAR_UNLOCKED;
        let list = (*tv).vval.v_list;
        if (*list).lv_refcount > 1 {
            (*list).lv_refcount -= 1;
            (*tv).vval.v_list = ptr::null_mut();
            if let MPConvStackData::List { ref mut li, .. } = mpsv.data {
                *li = ptr::null_mut();
            }
            return OK;
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_list_end(tv: *mut typval_T) {
    unsafe {
        if tv.is_null() {
            return;
        }
        debug_assert_eq!((*tv).v_type, VarType::VAR_LIST);
        let list = (*tv).vval.v_list;
        tv_list_unref(list);
        (*tv).vval.v_list = ptr::null_mut();
    }
}

#[inline]
unsafe fn nothing_conv_real_dict_after_start(
    tv: *mut typval_T,
    dictp: *mut *mut dict_T,
    mpsv: &mut MPConvStackVal,
) -> c_int {
    unsafe {
        if !tv.is_null() {
            (*tv).v_lock = VAR_UNLOCKED;
        }
        if !dictp.is_null() && (**dictp).dv_refcount > 1 {
            (**dictp).dv_refcount -= 1;
            *dictp = ptr::null_mut();
            if let MPConvStackData::Dict { ref mut todo, .. } = mpsv.data {
                *todo = 0;
            }
            return OK;
        }
        NOTDONE
    }
}

#[inline]
unsafe fn nothing_conv_dict_end(dictp: *mut *mut dict_T) {
    unsafe {
        if !dictp.is_null() {
            tv_dict_unref(*dictp);
            *dictp = ptr::null_mut();
        }
    }
}

unsafe fn nothing_convert_one_value(
    mpstack: &mut Vec<MPConvStackVal>,
    tv: *mut typval_T,
    copy_id: c_int,
) -> c_int {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_STRING => {
                xfree((*tv).vval.v_string.cast());
                (*tv).vval.v_string = ptr::null_mut();
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_NUMBER => {
                (*tv).vval.v_number = 0;
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_FLOAT => {
                (*tv).vval.v_float = 0.0;
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_BLOB => {
                tv_blob_unref((*tv).vval.v_blob);
                (*tv).vval.v_blob = ptr::null_mut();
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_FUNC => {
                let _ = nothing_conv_func_start(tv, (*tv).vval.v_string);
            }
            VarType::VAR_PARTIAL => {
                let pt = (*tv).vval.v_ptr.cast::<partial_T>();
                #[cfg(not(test))]
                let fun = if pt.is_null() {
                    ptr::null_mut()
                } else {
                    partial_name(pt)
                };
                #[cfg(test)]
                let fun: *mut c_char = ptr::null_mut();
                if nothing_conv_func_start(tv, fun) != NOTDONE {
                    return OK;
                }
                mpstack.push(MPConvStackVal {
                    val_type: MPConvStackValType::Partial,
                    tv,
                    saved_copy_id: copy_id - 1,
                    data: MPConvStackData::Partial {
                        stage: MPConvPartialStage::Args,
                        pt,
                    },
                });
            }
            VarType::VAR_LIST => {
                let list = (*tv).vval.v_list;
                if list.is_null() || tv_list_len_inline(list) == 0 {
                    tv_list_unref(list);
                    (*tv).vval.v_list = ptr::null_mut();
                    (*tv).v_lock = VAR_UNLOCKED;
                    return OK;
                }
                let saved_copy_id = (*list).lv_copyID;
                if (*list).lv_copyID == copy_id {
                    return OK;
                }
                (*list).lv_copyID = copy_id;
                let mut mpsv = MPConvStackVal {
                    val_type: MPConvStackValType::List,
                    tv,
                    saved_copy_id,
                    data: MPConvStackData::List {
                        list,
                        li: tv_list_first_inline(list),
                    },
                };
                let _ = nothing_conv_real_list_after_start(tv, &mut mpsv);
                mpstack.push(mpsv);
            }
            VarType::VAR_BOOL => {
                (*tv).vval.v_bool = kBoolVarFalse;
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_SPECIAL => {
                (*tv).vval.v_special = kSpecialVarNull;
                (*tv).v_lock = VAR_UNLOCKED;
            }
            VarType::VAR_DICT => {
                let dict = (*tv).vval.v_dict;
                let dictp = &raw mut (*tv).vval.v_dict;
                if dict.is_null() || (*dict).dv_hashtab.ht_used == 0 {
                    nothing_conv_empty_dict(tv, dictp);
                    return OK;
                }
                let saved_copy_id = (*dict).dv_copyID;
                if (*dict).dv_copyID == copy_id {
                    return OK;
                }
                (*dict).dv_copyID = copy_id;
                let mut mpsv = MPConvStackVal {
                    val_type: MPConvStackValType::Dict,
                    tv,
                    saved_copy_id,
                    data: MPConvStackData::Dict {
                        dict,
                        dictp,
                        hi: (*dict).dv_hashtab.ht_array,
                        todo: (*dict).dv_hashtab.ht_used,
                    },
                };
                let _ = nothing_conv_real_dict_after_start(tv, dictp, &mut mpsv);
                mpstack.push(mpsv);
            }
            VarType::VAR_UNKNOWN => {
                #[cfg(not(test))]
                internal_error(c"_typval_encode_nothing_convert_one_value()".as_ptr());
                return FAIL;
            }
        }
        OK
    }
}

unsafe fn encode_vim_to_nothing(top_tv: *mut typval_T) -> c_int {
    unsafe {
        #[cfg(not(test))]
        let copy_id = get_copyID();
        #[cfg(test)]
        let copy_id: c_int = 1;

        let mut mpstack: Vec<MPConvStackVal> = Vec::with_capacity(8);
        if nothing_convert_one_value(&mut mpstack, top_tv, copy_id) == FAIL {
            return FAIL;
        }

        while !mpstack.is_empty() {
            let last_idx = mpstack.len() - 1;
            let val_type = mpstack[last_idx].val_type;
            let tv: *mut typval_T = match val_type {
                MPConvStackValType::Dict => {
                    let (dict, dictp, mut hi, todo, saved_copy_id) = match mpstack[last_idx].data {
                        MPConvStackData::Dict {
                            dict,
                            dictp,
                            hi,
                            todo,
                        } => (dict, dictp, hi, todo, mpstack[last_idx].saved_copy_id),
                        _ => unreachable!(),
                    };
                    if todo == 0 {
                        mpstack.pop();
                        (*dict).dv_copyID = saved_copy_id;
                        nothing_conv_dict_end(dictp);
                        continue;
                    }
                    while hashitem_empty(&*hi) {
                        hi = hi.add(1);
                    }
                    let di = tv_dict_hi2di(hi);
                    if let MPConvStackData::Dict {
                        hi: ref mut hi_ref,
                        todo: ref mut todo_ref,
                        ..
                    } = mpstack[last_idx].data
                    {
                        *todo_ref -= 1;
                        *hi_ref = hi.add(1);
                    }
                    &raw mut (*di).di_tv
                }
                MPConvStackValType::List => {
                    let (list, li, saved_copy_id, cur_tv) = match mpstack[last_idx].data {
                        MPConvStackData::List { list, li } => (
                            list,
                            li,
                            mpstack[last_idx].saved_copy_id,
                            mpstack[last_idx].tv,
                        ),
                        _ => unreachable!(),
                    };
                    if li.is_null() {
                        mpstack.pop();
                        (*list).lv_copyID = saved_copy_id;
                        nothing_conv_list_end(cur_tv);
                        continue;
                    }
                    let item_tv = &raw mut (*li).li_tv;
                    if let MPConvStackData::List {
                        li: ref mut li_ref, ..
                    } = mpstack[last_idx].data
                    {
                        *li_ref = (*li).li_next;
                    }
                    item_tv
                }
                MPConvStackValType::Partial => {
                    let (stage, pt, cur_tv) = match mpstack[last_idx].data {
                        MPConvStackData::Partial { stage, pt } => (stage, pt, mpstack[last_idx].tv),
                        _ => unreachable!(),
                    };
                    match stage {
                        MPConvPartialStage::Args => {
                            if let MPConvStackData::Partial {
                                stage: ref mut stage_ref,
                                ..
                            } = mpstack[last_idx].data
                            {
                                *stage_ref = MPConvPartialStage::SelfDict;
                            }
                            if !pt.is_null() && (*pt).pt_argc > 0 {
                                mpstack.push(MPConvStackVal {
                                    val_type: MPConvStackValType::PartialList,
                                    tv: ptr::null_mut(),
                                    saved_copy_id: copy_id - 1,
                                    data: MPConvStackData::PartialList {
                                        arg: (*pt).pt_argv,
                                        argv: (*pt).pt_argv,
                                        todo: (*pt).pt_argc as usize,
                                    },
                                });
                            }
                        }
                        MPConvPartialStage::SelfDict => {
                            if let MPConvStackData::Partial {
                                stage: ref mut stage_ref,
                                ..
                            } = mpstack[last_idx].data
                            {
                                *stage_ref = MPConvPartialStage::End;
                            }
                            let dict = if pt.is_null() {
                                ptr::null_mut()
                            } else {
                                (*pt).pt_dict.cast::<dict_T>()
                            };
                            if !dict.is_null() {
                                let dictp = (&raw mut (*pt).pt_dict).cast::<*mut dict_T>();
                                if (*dict).dv_hashtab.ht_used == 0 {
                                    nothing_conv_empty_dict(ptr::null_mut(), dictp);
                                    continue;
                                }
                                let saved_copy_id = (*dict).dv_copyID;
                                if (*dict).dv_copyID == copy_id {
                                    continue;
                                }
                                (*dict).dv_copyID = copy_id;
                                let mut mpsv = MPConvStackVal {
                                    val_type: MPConvStackValType::Dict,
                                    tv: ptr::null_mut(),
                                    saved_copy_id,
                                    data: MPConvStackData::Dict {
                                        dict,
                                        dictp,
                                        hi: (*dict).dv_hashtab.ht_array,
                                        todo: (*dict).dv_hashtab.ht_used,
                                    },
                                };
                                let _ = nothing_conv_real_dict_after_start(
                                    ptr::null_mut(),
                                    dictp,
                                    &mut mpsv,
                                );
                                mpstack.push(mpsv);
                            }
                        }
                        MPConvPartialStage::End => {
                            nothing_conv_func_end(cur_tv, copy_id);
                            mpstack.pop();
                        }
                    }
                    continue;
                }
                MPConvStackValType::PartialList => {
                    let (arg, todo, cur_tv) = match mpstack[last_idx].data {
                        MPConvStackData::PartialList { arg, todo, .. } => {
                            (arg, todo, mpstack[last_idx].tv)
                        }
                        _ => unreachable!(),
                    };
                    if todo == 0 {
                        mpstack.pop();
                        nothing_conv_list_end(cur_tv);
                        continue;
                    }
                    if let MPConvStackData::PartialList {
                        arg: ref mut arg_ref,
                        todo: ref mut todo_ref,
                        ..
                    } = mpstack[last_idx].data
                    {
                        *arg_ref = arg.add(1);
                        *todo_ref -= 1;
                    }
                    arg
                }
            };
            debug_assert!(!tv.is_null());
            if nothing_convert_one_value(&mut mpstack, tv, copy_id) == FAIL {
                return FAIL;
            }
        }
        OK
    }
}

/// Free memory for a variable value and set the value to NULL or 0.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_clear(tv: *mut typval_T) {
    unsafe {
        if tv.is_null() || (*tv).v_type == VarType::VAR_UNKNOWN {
            return;
        }
        let evn_ret = encode_vim_to_nothing(tv);
        debug_assert_eq!(evn_ret, OK);
    }
}

/// Free allocated Vimscript object and value stored inside.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_free(tv: *mut typval_T) {
    unsafe {
        if tv.is_null() {
            return;
        }

        match (*tv).v_type {
            VarType::VAR_PARTIAL => {
                #[cfg(not(test))]
                partial_unref((*tv).vval.v_ptr.cast::<partial_T>());
            }
            VarType::VAR_FUNC => {
                #[cfg(not(test))]
                func_unref((*tv).vval.v_string);
                xfree((*tv).vval.v_string.cast());
            }
            VarType::VAR_STRING => {
                xfree((*tv).vval.v_string.cast());
            }
            VarType::VAR_BLOB => {
                tv_blob_unref((*tv).vval.v_blob);
            }
            VarType::VAR_LIST => {
                tv_list_unref((*tv).vval.v_list);
            }
            VarType::VAR_DICT => {
                tv_dict_unref((*tv).vval.v_dict);
            }
            VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_NUMBER
            | VarType::VAR_FLOAT
            | VarType::VAR_UNKNOWN => {}
        }
        xfree(tv.cast());
    }
}

/// Copy typval from one location to another.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_copy(from: *const typval_T, to: *mut typval_T) {
    unsafe {
        (*to).v_type = (*from).v_type;
        (*to).v_lock = VAR_UNLOCKED;
        (*to).vval = (*from).vval;
        match (*from).v_type {
            VarType::VAR_NUMBER | VarType::VAR_FLOAT | VarType::VAR_BOOL | VarType::VAR_SPECIAL => {
            }
            VarType::VAR_STRING | VarType::VAR_FUNC => {
                if !(*from).vval.v_string.is_null() {
                    (*to).vval.v_string = xstrdup((*from).vval.v_string);
                    #[cfg(not(test))]
                    if (*from).v_type == VarType::VAR_FUNC {
                        func_ref((*to).vval.v_string);
                    }
                }
            }
            VarType::VAR_PARTIAL => {
                let pt = (*to).vval.v_ptr.cast::<partial_T>();
                if !pt.is_null() {
                    (*pt).pt_refcount += 1;
                }
            }
            VarType::VAR_BLOB => {
                if !(*from).vval.v_blob.is_null() {
                    (*(*from).vval.v_blob).bv_refcount += 1;
                }
            }
            VarType::VAR_LIST => {
                tv_list_ref_inline((*to).vval.v_list);
            }
            VarType::VAR_DICT => {
                if !(*from).vval.v_dict.is_null() {
                    (*(*from).vval.v_dict).dv_refcount += 1;
                }
            }
            VarType::VAR_UNKNOWN => {
                #[cfg(not(test))]
                semsg!(tr(e_intern2.as_ptr()), c"tv_copy(UNKNOWN)".as_ptr());
            }
        }
    }
}

#[inline]
const fn change_lock(lock: bool, var: VarLockStatus) -> VarLockStatus {
    if var == VAR_FIXED {
        VAR_FIXED
    } else if lock {
        VAR_LOCKED
    } else {
        VAR_UNLOCKED
    }
}

/// Lock or unlock an item.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_item_lock(
    tv: *mut typval_T,
    deep: c_int,
    lock: bool,
    check_refcount: bool,
) {
    unsafe {
        static mut RECURSE: c_int = 0;

        if RECURSE >= DICT_MAXNEST {
            #[cfg(not(test))]
            emsg(tr(E_VARIABLE_NESTED_TOO_DEEP_FOR_UNLOCK.as_ptr()));
            return;
        }
        if deep == 0 {
            return;
        }
        RECURSE += 1;

        (*tv).v_lock = change_lock(lock, (*tv).v_lock);

        match (*tv).v_type {
            VarType::VAR_BLOB => {
                let b = (*tv).vval.v_blob;
                if !b.is_null() && !(check_refcount && (*b).bv_refcount > 1) {
                    (*b).bv_lock = change_lock(lock, (*b).bv_lock);
                }
            }
            VarType::VAR_LIST => {
                let l = (*tv).vval.v_list;
                if !l.is_null() && !(check_refcount && (*l).lv_refcount > 1) {
                    (*l).lv_lock = change_lock(lock, (*l).lv_lock);
                    if !(0..=1).contains(&deep) {
                        let mut li = (*l).lv_first;
                        while !li.is_null() {
                            tv_item_lock(&raw mut (*li).li_tv, deep - 1, lock, check_refcount);
                            li = (*li).li_next;
                        }
                    }
                }
            }
            VarType::VAR_DICT => {
                let d = (*tv).vval.v_dict;
                if !d.is_null() && !(check_refcount && (*d).dv_refcount > 1) {
                    (*d).dv_lock = change_lock(lock, (*d).dv_lock);
                    if !(0..=1).contains(&deep) {
                        let mut todo = (*d).dv_hashtab.ht_used;
                        let mut hi = (*d).dv_hashtab.ht_array;
                        while todo > 0 {
                            if !hashitem_empty(&*hi) {
                                todo -= 1;
                                let di = tv_dict_hi2di(hi);
                                tv_item_lock(&raw mut (*di).di_tv, deep - 1, lock, check_refcount);
                            }
                            hi = hi.add(1);
                        }
                    }
                }
            }
            VarType::VAR_NUMBER
            | VarType::VAR_FLOAT
            | VarType::VAR_STRING
            | VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL => {}
            VarType::VAR_UNKNOWN => std::process::abort(),
        }
        RECURSE -= 1;
    }
}

/// Check whether Vimscript value is locked itself or refers to a locked container.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_islocked(tv: *const typval_T) -> bool {
    unsafe {
        (*tv).v_lock == VAR_LOCKED
            || ((*tv).v_type == VarType::VAR_LIST
                && tv_list_locked_inline((*tv).vval.v_list) == VAR_LOCKED)
            || ((*tv).v_type == VarType::VAR_DICT
                && !(*tv).vval.v_dict.is_null()
                && (*(*tv).vval.v_dict).dv_lock == VAR_LOCKED)
    }
}

/// Return true if typval is locked.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_lock(
    tv: *const typval_T,
    name: *const c_char,
    name_len: usize,
) -> bool {
    unsafe {
        let mut lock = VAR_UNLOCKED;

        match (*tv).v_type {
            VarType::VAR_BLOB if !(*tv).vval.v_blob.is_null() => {
                lock = (*(*tv).vval.v_blob).bv_lock;
            }
            VarType::VAR_LIST if !(*tv).vval.v_list.is_null() => {
                lock = (*(*tv).vval.v_list).lv_lock;
            }
            VarType::VAR_DICT if !(*tv).vval.v_dict.is_null() => {
                lock = (*(*tv).vval.v_dict).dv_lock;
            }
            _ => {}
        }
        value_check_lock((*tv).v_lock, name, name_len)
            || (lock != VAR_UNLOCKED && value_check_lock(lock, name, name_len))
    }
}

/// Return true if variable "name" has a locked (immutable) value.
#[allow(unused_unsafe)]
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn value_check_lock(
    lock: VarLockStatus,
    mut name: *const c_char,
    mut name_len: usize,
) -> bool {
    unsafe {
        match lock {
            VAR_UNLOCKED => return false,
            #[cfg(not(test))]
            VAR_LOCKED => {
                let error_message = if name.is_null() {
                    e_value_is_locked.as_ptr()
                } else {
                    e_value_is_locked_str.as_ptr()
                };
                if name.is_null() {
                    emsg(tr(error_message));
                } else {
                    if name_len == TV_TRANSLATE {
                        name = tr(name);
                        name_len = c_strlen(name);
                    } else if name_len == TV_CSTRING {
                        name_len = c_strlen(name);
                    }
                    semsg!(tr(error_message), name_len as c_int, name);
                }
            }
            #[cfg(not(test))]
            VAR_FIXED => {
                let error_message = if name.is_null() {
                    e_cannot_change_value.as_ptr()
                } else {
                    e_cannot_change_value_of_str.as_ptr()
                };
                if name.is_null() {
                    emsg(tr(error_message));
                } else {
                    if name_len == TV_TRANSLATE {
                        name = tr(name);
                        name_len = c_strlen(name);
                    } else if name_len == TV_CSTRING {
                        name_len = c_strlen(name);
                    }
                    semsg!(tr(error_message), name_len as c_int, name);
                }
            }
            _ => {
                let _ = (&mut name, &mut name_len);
            }
        }
        true
    }
}

/// Compare two Vimscript values.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_equal(tv1: *mut typval_T, tv2: *mut typval_T, ic: bool) -> bool {
    unsafe {
        static mut RECURSIVE_CNT: c_int = 0;

        if !(tv_is_func_inline(*tv1) && tv_is_func_inline(*tv2)) && (*tv1).v_type != (*tv2).v_type {
            return false;
        }

        if RECURSIVE_CNT == 0 {
            TV_EQUAL_RECURSE_LIMIT = 1000;
        }
        if RECURSIVE_CNT >= TV_EQUAL_RECURSE_LIMIT {
            TV_EQUAL_RECURSE_LIMIT -= 1;
            return true;
        }

        match (*tv1).v_type {
            VarType::VAR_LIST => {
                RECURSIVE_CNT += 1;
                let r = tv_list_equal((*tv1).vval.v_list, (*tv2).vval.v_list, ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_DICT => {
                RECURSIVE_CNT += 1;
                let r = tv_dict_equal((*tv1).vval.v_dict, (*tv2).vval.v_dict, ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                if ((*tv1).v_type == VarType::VAR_PARTIAL && (*tv1).vval.v_ptr.is_null())
                    || ((*tv2).v_type == VarType::VAR_PARTIAL && (*tv2).vval.v_ptr.is_null())
                {
                    return false;
                }
                RECURSIVE_CNT += 1;
                let r = func_equal(tv1, tv2, ic);
                RECURSIVE_CNT -= 1;
                r
            }
            VarType::VAR_BLOB => tv_blob_equal((*tv1).vval.v_blob, (*tv2).vval.v_blob),
            VarType::VAR_NUMBER => (*tv1).vval.v_number == (*tv2).vval.v_number,
            VarType::VAR_FLOAT => (*tv1).vval.v_float == (*tv2).vval.v_float,
            VarType::VAR_STRING => {
                let mut buf1: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                let mut buf2: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
                let s1 = tv_get_string_buf(tv1, buf1.as_mut_ptr());
                let s2 = tv_get_string_buf(tv2, buf2.as_mut_ptr());
                mb_strcmp_ic(ic, s1, s2) == 0
            }
            VarType::VAR_BOOL => (*tv1).vval.v_bool == (*tv2).vval.v_bool,
            VarType::VAR_SPECIAL => (*tv1).vval.v_special == (*tv2).vval.v_special,
            VarType::VAR_UNKNOWN => false,
        }
    }
}

/// Check that given value is a number or string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_str_or_nr(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER | VarType::VAR_STRING => true,
            VarType::VAR_FLOAT => {
                emsg(tr(
                    c"E805: Expected a Number or a String, Float found".as_ptr()
                ));
                false
            }
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                emsg(tr(
                    c"E703: Expected a Number or a String, Funcref found".as_ptr()
                ));
                false
            }
            VarType::VAR_LIST => {
                emsg(tr(
                    c"E745: Expected a Number or a String, List found".as_ptr()
                ));
                false
            }
            VarType::VAR_DICT => {
                emsg(tr(
                    c"E728: Expected a Number or a String, Dictionary found".as_ptr()
                ));
                false
            }
            VarType::VAR_BLOB => {
                emsg(tr(
                    c"E974: Expected a Number or a String, Blob found".as_ptr()
                ));
                false
            }
            VarType::VAR_BOOL => {
                emsg(tr(
                    c"E5299: Expected a Number or a String, Boolean found".as_ptr()
                ));
                false
            }
            VarType::VAR_SPECIAL => {
                emsg(tr(c"E5300: Expected a Number or a String".as_ptr()));
                false
            }
            VarType::VAR_UNKNOWN => {
                semsg!(
                    tr(e_intern2.as_ptr()),
                    c"tv_check_str_or_nr(UNKNOWN)".as_ptr(),
                );
                false
            }
        }
    }
}

#[inline]
const fn num_error(vt: VarType) -> *const c_char {
    match vt {
        VarType::VAR_PARTIAL | VarType::VAR_FUNC => c"E703: Using a Funcref as a Number".as_ptr(),
        VarType::VAR_LIST => c"E745: Using a List as a Number".as_ptr(),
        VarType::VAR_DICT => c"E728: Using a Dictionary as a Number".as_ptr(),
        VarType::VAR_FLOAT => c"E805: Using a Float as a Number".as_ptr(),
        VarType::VAR_BLOB => c"E974: Using a Blob as a Number".as_ptr(),
        VarType::VAR_UNKNOWN => c"E685: using an invalid value as a Number".as_ptr(),
        _ => c"".as_ptr(),
    }
}

#[inline]
const fn str_error(vt: VarType) -> *const c_char {
    match vt {
        VarType::VAR_PARTIAL | VarType::VAR_FUNC => c"E729: Using a Funcref as a String".as_ptr(),
        VarType::VAR_LIST => c"E730: Using a List as a String".as_ptr(),
        VarType::VAR_DICT => c"E731: Using a Dictionary as a String".as_ptr(),
        VarType::VAR_BLOB => c"E976: Using a Blob as a String".as_ptr(),
        VarType::VAR_UNKNOWN => E_USING_INVALID_VALUE_AS_STRING.as_ptr(),
        _ => c"".as_ptr(),
    }
}

/// Check that given value is a number or can be converted to it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_num(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_STRING => true,
            VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_FLOAT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(num_error((*tv).v_type)));
                false
            }
        }
    }
}

/// Check that given value is a Vimscript String or can be "cast" to it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_str(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_STRING
            | VarType::VAR_FLOAT => true,
            VarType::VAR_PARTIAL
            | VarType::VAR_FUNC
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(str_error((*tv).v_type)));
                false
            }
        }
    }
}

/// Get the number value of a Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_number(tv: *const typval_T) -> varnumber_T {
    unsafe {
        let mut error = false;
        tv_get_number_chk(tv, &raw mut error)
    }
}

/// Get the number value of a Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_number_chk(
    tv: *const typval_T,
    ret_error: *mut bool,
) -> varnumber_T {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_FUNC
            | VarType::VAR_PARTIAL
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_FLOAT => {
                #[cfg(not(test))]
                emsg(tr(num_error((*tv).v_type)));
            }
            VarType::VAR_NUMBER => return (*tv).vval.v_number,
            VarType::VAR_STRING => {
                let mut n: varnumber_T = 0;
                if !(*tv).vval.v_string.is_null() {
                    vim_str2nr(
                        (*tv).vval.v_string,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        STR2NR_ALL,
                        &raw mut n,
                        ptr::null_mut(),
                        0,
                        false,
                        ptr::null_mut(),
                    );
                }
                return n;
            }
            VarType::VAR_BOOL => return i64::from((*tv).vval.v_bool == kBoolVarTrue),
            VarType::VAR_SPECIAL => return 0,
            VarType::VAR_UNKNOWN => {
                #[cfg(not(test))]
                semsg!(tr(e_intern2.as_ptr()), c"tv_get_number(UNKNOWN)".as_ptr());
            }
        }
        if !ret_error.is_null() {
            *ret_error = true;
        }
        if ret_error.is_null() { -1 } else { 0 }
    }
}

/// Get the boolean value of a Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_bool(tv: *const typval_T) -> varnumber_T {
    unsafe { tv_get_number_chk(tv, ptr::null_mut()) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_bool_chk(tv: *const typval_T, ret_error: *mut bool) -> varnumber_T {
    unsafe { tv_get_number_chk(tv, ret_error) }
}

/// Get the line number from Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_lnum(tv: *const typval_T) -> linenr_T {
    unsafe {
        let did_emsg_before = did_emsg;
        let mut lnum = tv_get_number_chk(tv, ptr::null_mut()) as linenr_T;
        if lnum <= 0 && did_emsg_before == did_emsg && (*tv).v_type != VarType::VAR_NUMBER {
            let mut fnum: c_int = 0;
            let fp = var2fpos(tv, true, &raw mut fnum, false, curwin);
            if !fp.is_null() {
                lnum = (*fp).lnum;
            }
        }
        lnum
    }
}

/// Get the line number from Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_lnum_buf(tv: *const typval_T, buf: *const buf_T) -> linenr_T {
    unsafe {
        if (*tv).v_type == VarType::VAR_STRING
            && !(*tv).vval.v_string.is_null()
            && *(*tv).vval.v_string == b'$' as c_char
            && *(*tv).vval.v_string.add(1) == NUL
            && !buf.is_null()
        {
            return (*buf).b_ml.ml_line_count;
        }
        tv_get_number_chk(tv, ptr::null_mut()) as linenr_T
    }
}

/// Get the floating-point value of a Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_float(tv: *const typval_T) -> float_T {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => (*tv).vval.v_number as float_T,
            VarType::VAR_FLOAT => (*tv).vval.v_float,
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => {
                #[cfg(not(test))]
                emsg(tr(c"E891: Using a Funcref as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_STRING => {
                #[cfg(not(test))]
                emsg(tr(c"E892: Using a String as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_LIST => {
                #[cfg(not(test))]
                emsg(tr(c"E893: Using a List as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_DICT => {
                #[cfg(not(test))]
                emsg(tr(c"E894: Using a Dictionary as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_BOOL => {
                #[cfg(not(test))]
                emsg(tr(c"E362: Using a boolean value as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_SPECIAL => {
                #[cfg(not(test))]
                emsg(tr(c"E907: Using a special value as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_BLOB => {
                #[cfg(not(test))]
                emsg(tr(c"E975: Using a Blob as a Float".as_ptr()));
                0.0
            }
            VarType::VAR_UNKNOWN => {
                #[cfg(not(test))]
                semsg!(tr(e_intern2.as_ptr()), c"tv_get_float(UNKNOWN)".as_ptr());
                0.0
            }
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_string_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_STRING {
            semsg!(tr(E_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a non-empty string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_nonempty_string_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        if tv_check_for_string_arg(args, idx) == FAIL {
            return FAIL;
        }
        let s = (*args.offset(idx as isize)).vval.v_string;
        if s.is_null() || *s == NUL {
            semsg!(
                tr(E_NON_EMPTY_STRING_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional string argument at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_opt_string_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_string_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_number_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_NUMBER {
            semsg!(tr(E_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Check for an optional number argument at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_opt_number_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_number_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a float or a number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_float_or_nr_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_FLOAT && vt != VarType::VAR_NUMBER {
            semsg!(
                tr(E_FLOAT_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a bool.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_bool_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let arg = &*args.offset(idx as isize);
        if arg.v_type != VarType::VAR_BOOL
            && !(arg.v_type == VarType::VAR_NUMBER
                && (arg.vval.v_number == 0 || arg.vval.v_number == 1))
        {
            semsg!(tr(E_BOOL_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Check for an optional bool argument at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_opt_bool_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN {
            return OK;
        }
        tv_check_for_bool_arg(args, idx)
    }
}

/// Give an error and return FAIL unless "args[idx]" is a blob.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_blob_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_BLOB {
            semsg!(tr(E_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_list_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_LIST {
            semsg!(tr(E_LIST_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a dict.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type != VarType::VAR_DICT {
            semsg!(tr(E_DICT_REQUIRED_FOR_ARGUMENT_NR.as_ptr()), idx + 1);
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a non-NULL dict.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_nonnull_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if tv_check_for_dict_arg(args, idx) == FAIL {
            return FAIL;
        }
        if (*args.offset(idx as isize)).vval.v_dict.is_null() {
            semsg!(
                tr(E_NON_NULL_DICT_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional dict argument at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_opt_dict_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_dict_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_string_or_number_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_NUMBER {
            semsg!(
                tr(E_STRING_OR_NUMBER_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a buffer number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_buffer_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe { tv_check_for_string_or_number_arg(args, idx) }
}

/// Give an error and return FAIL unless "args[idx]" is a line number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_lnum_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe { tv_check_for_string_or_number_arg(args, idx) }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a list.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_string_or_list_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_LIST {
            semsg!(
                tr(E_STRING_OR_LIST_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string, a list or a blob.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_string_or_list_or_blob_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_STRING && vt != VarType::VAR_LIST && vt != VarType::VAR_BLOB {
            semsg!(
                tr(E_STRING_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Check for an optional string or list argument at "idx".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_opt_string_or_list_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        if (*args.offset(idx as isize)).v_type == VarType::VAR_UNKNOWN
            || tv_check_for_string_or_list_arg(args, idx) != FAIL
        {
            OK
        } else {
            FAIL
        }
    }
}

/// Give an error and return FAIL unless "args[idx]" is a string or a function reference.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_string_or_func_arg(
    args: *const typval_T,
    idx: c_int,
) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_PARTIAL && vt != VarType::VAR_FUNC && vt != VarType::VAR_STRING {
            semsg!(
                tr(E_STRING_OR_FUNCTION_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Give an error and return FAIL unless "args[idx]" is a list or a blob.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_check_for_list_or_blob_arg(args: *const typval_T, idx: c_int) -> c_int {
    unsafe {
        let vt = (*args.offset(idx as isize)).v_type;
        if vt != VarType::VAR_LIST && vt != VarType::VAR_BLOB {
            semsg!(
                tr(E_LIST_OR_BLOB_REQUIRED_FOR_ARGUMENT_NR.as_ptr()),
                idx + 1,
            );
            return FAIL;
        }
        OK
    }
}

/// Get the string value of a "stringish" Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_string_buf_chk(
    tv: *const typval_T,
    buf: *mut c_char,
) -> *const c_char {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => {
                use std::io::Write;
                let mut slice = std::slice::from_raw_parts_mut(buf.cast::<u8>(), NUMBUFLEN);
                let _ = write!(slice, "{}\0", (*tv).vval.v_number);
                buf
            }
            VarType::VAR_FLOAT => {
                vim_snprintf!(buf, NUMBUFLEN, c"%g".as_ptr(), (*tv).vval.v_float);
                buf
            }
            VarType::VAR_STRING => {
                if !(*tv).vval.v_string.is_null() {
                    (*tv).vval.v_string
                } else {
                    c"".as_ptr()
                }
            }
            VarType::VAR_BOOL => {
                xstrlcpy(
                    buf,
                    encode_bool_var_names.0[(*tv).vval.v_bool as usize],
                    NUMBUFLEN,
                );
                buf
            }
            VarType::VAR_SPECIAL => {
                xstrlcpy(
                    buf,
                    encode_special_var_names.0[(*tv).vval.v_special as usize],
                    NUMBUFLEN,
                );
                buf
            }
            VarType::VAR_PARTIAL
            | VarType::VAR_FUNC
            | VarType::VAR_LIST
            | VarType::VAR_DICT
            | VarType::VAR_BLOB
            | VarType::VAR_UNKNOWN => {
                emsg(tr(str_error((*tv).v_type)));
                ptr::null()
            }
        }
    }
}

/// Get the string value of a "stringish" Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_string_chk(tv: *const typval_T) -> *const c_char {
    unsafe {
        static mut MYBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        tv_get_string_buf_chk(tv, (&raw mut MYBUF).cast::<c_char>())
    }
}

/// Get the string value of a "stringish" Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_string(tv: *const typval_T) -> *const c_char {
    unsafe {
        static mut MYBUF: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        tv_get_string_buf(tv, (&raw mut MYBUF).cast::<c_char>())
    }
}

/// Get the string value of a "stringish" Vimscript object.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_string_buf(tv: *const typval_T, buf: *mut c_char) -> *const c_char {
    unsafe {
        let res = tv_get_string_buf_chk(tv, buf);
        if !res.is_null() { res } else { c"".as_ptr() }
    }
}

/// Return true when "tv" is not falsy.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv2bool(tv: *const typval_T) -> bool {
    unsafe {
        match (*tv).v_type {
            VarType::VAR_NUMBER => (*tv).vval.v_number != 0,
            VarType::VAR_FLOAT => (*tv).vval.v_float != 0.0,
            VarType::VAR_PARTIAL => !(*tv).vval.v_ptr.is_null(),
            VarType::VAR_FUNC | VarType::VAR_STRING => {
                !(*tv).vval.v_string.is_null() && *(*tv).vval.v_string != NUL
            }
            VarType::VAR_LIST => !(*tv).vval.v_list.is_null() && (*(*tv).vval.v_list).lv_len > 0,
            VarType::VAR_DICT => {
                !(*tv).vval.v_dict.is_null() && (*(*tv).vval.v_dict).dv_hashtab.ht_used > 0
            }
            VarType::VAR_BOOL => (*tv).vval.v_bool == kBoolVarTrue,
            VarType::VAR_SPECIAL => (*tv).vval.v_special != kSpecialVarNull,
            VarType::VAR_BLOB => {
                !(*tv).vval.v_blob.is_null() && (*(*tv).vval.v_blob).bv_ga.ga_len > 0
            }
            VarType::VAR_UNKNOWN => false,
        }
    }
}

/// Check that repeating "slen" items "n" times does not overflow int.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_repeat_count(slen: varnumber_T, n: varnumber_T) -> c_int {
    unsafe {
        if n <= i64::from(c_int::MAX) / slen {
            return OK;
        }

        let mut buf: [c_char; NUMBUFLEN] = [0; NUMBUFLEN];
        vim_snprintf!(buf.as_mut_ptr(), buf.len(), c"%lld".as_ptr(), n);
        semsg!(tr(e_val_too_large.as_ptr()), buf.as_ptr());
        FAIL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tv_list_operations() {
        unsafe {
            let l = tv_list_alloc(0);
            assert!(!l.is_null());
            (*l).lv_refcount = 1;
            tv_list_append_number(l, 10);
            tv_list_append_number(l, 20);
            tv_list_append_number(l, 30);
            assert_eq!(tv_list_len_inline(l), 3);

            let item1 = tv_list_find(l, 1);
            assert!(!item1.is_null());
            assert_eq!((*item1).li_tv.vval.v_number, 20);

            tv_list_reverse(l);
            let first = tv_list_find(l, 0);
            assert_eq!((*first).li_tv.vval.v_number, 30);

            tv_list_unref(l);
        }
    }

    #[test]
    fn test_tv_dict_operations() {
        unsafe {
            let d = tv_dict_alloc();
            assert!(!d.is_null());
            (*d).dv_refcount = 1;
            assert_eq!(tv_dict_add_nr(d, c"foo".as_ptr(), 3, 42), OK);
            assert!(tv_dict_has_key(d, c"foo".as_ptr()));
            assert!(!tv_dict_has_key(d, c"bar".as_ptr()));

            let di = tv_dict_find(d, c"foo".as_ptr(), -1);
            assert!(!di.is_null());
            assert_eq!((*di).di_tv.vval.v_number, 42);

            tv_dict_unref(d);
        }
    }

    #[test]
    fn test_tv2bool() {
        unsafe {
            let mut tv = tv_initial_value();
            tv.v_type = VarType::VAR_NUMBER;
            tv.vval.v_number = 0;
            assert!(!tv2bool(&raw const tv));
            tv.vval.v_number = 5;
            assert!(tv2bool(&raw const tv));
        }
    }
}
