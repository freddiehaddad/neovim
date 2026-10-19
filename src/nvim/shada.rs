//! ShaDa (Shared Data) file reader and writer (`src/nvim/shada.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::enum_variant_names,
    clippy::missing_safety_doc,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

use crate::globals::*;
use crate::option_vars::*;
use std::cmp::Ordering;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::private::validate::{Dict, KeyValuePair, Object, ObjectData, ObjectType};
use crate::api::tabpage::{pos_T, win_T, xfmark_T};
use crate::arglist::alist_T;
use crate::clipboard::{MotionType, TypvalVval, VarType, list_T, typval_T, yankreg_T};
use crate::cmdhist::{
    HIST_CMD, HIST_COUNT, HIST_DEBUG, HIST_EXPR, HIST_INPUT, HIST_SEARCH, clr_history,
    hist_get_array, hist_iter, histentry_T,
};
use crate::cursor::{colnr_T, linenr_T};
use crate::eval::decode::{MPACK_OK, decode_string, dictitem_T, unpack_typval};
use crate::eval::deprecated::{buf_T, fmark_T, fmarkv_T};
use crate::eval::encode::encode_vim_to_msgpack;
use crate::eval::fs::FileInfo;
use crate::eval::vars::{VV_OLDFILES, get_globvar_ht, get_vim_var_list, set_vim_var_list};
use crate::hashtab::{hashitem_T, hashitem_empty, hashtab_T};
use crate::lua::converter::{Boolean, KeySetLink};
use crate::map::{
    Map, Set, cstr_t, map_del_cstr_t_ptr_t, map_put_ref_cstr_t_ptr_t, map_ref_cstr_t_ptr_t,
    mh_get_cstr_t, mh_get_path_t, mh_get_ptr_t, mh_put_cstr_t, mh_put_path_t, mh_put_ptr_t, path_t,
    ptr_t,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, MHPutStatus, MapHash, NvimString};
use crate::mark::{
    cleanup_jumplist, free_fmark, free_xfmark, mark_buffer_iter, mark_get, mark_global_iter,
    mark_jumplist_iter, mark_set_global, mark_set_local, set_last_cursor, setpcmark,
};
use crate::math::{FAIL, OK};
use crate::mbyte::mb_strnicmp;
use crate::memory::{xcalloc, xfree, xmalloc, xmemdup, xmemdupz, xrealloc, xstrdup};
use crate::msgpack_rpc::packer::{
    Integer, MPACK_ITEM_SIZE, PackerBuffer, mpack_array, mpack_bin, mpack_bool, mpack_check_buffer,
    mpack_integer, mpack_map, mpack_raw, mpack_str, mpack_uint, mpack_uint64, packer_string_buffer,
    packer_take_string,
};
use crate::msgpack_rpc::unpacker::{
    AdditionalDataBuilder, AdditionalDataHeader, StringArray, push_additional_data, unpack_array,
    unpack_integer, unpack_keydict, unpack_skip, unpack_string,
};
use crate::os::env::{home_replace, home_replace_save, os_get_pid};
use crate::os::fileio::{
    ARENA_BLOCK_SIZE, FileDescriptor, K_FILE_CREATE, K_FILE_CREATE_ONLY, K_FILE_NO_SYMLINK,
    K_FILE_READ_ONLY, K_FILE_TRUNCATE, file_close, file_flush, file_open, file_open_buffer,
    file_read, file_skip, file_try_read_buffered,
};
#[cfg(unix)]
use crate::os::fs::os_fchown;
use crate::os::fs::{os_fileinfo, os_getperm, os_isdir, os_mkdir_recurse, os_remove};
use crate::os::stdpaths::stdpaths_user_state_subpath;
use crate::os::time::{Timestamp, os_time};
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{
    concat_fnames_realloc, kPathCmpLiteral, path_equal, path_tail_with_sep, path_try_shorten_fname,
};
use crate::register::{
    NUM_SAVED_REGISTERS, op_global_reg_iter, op_reg_get, op_reg_index, op_reg_set, reg_empty,
};
use crate::search::{
    SearchOffset, SearchPattern, get_search_pattern, get_search_pattern_timestamp,
    get_substitute_pattern, search_pattern_cleared, search_was_last_used, set_last_used_pattern,
    set_search_pattern, set_substitute_pattern,
};

pub type AdditionalData = c_void;
pub type OptInt = i64;
pub type OptionalKeys = u64;
pub type ShaDaReadFileFlags = c_int;
pub type var_flavour_T = c_int;

const VAR_FLAVOUR_DEFAULT: var_flavour_T = 1;
const VAR_FLAVOUR_SESSION: var_flavour_T = 2;
const VAR_FLAVOUR_SHADA: var_flavour_T = 4;

pub const kShaDaWantInfo: c_int = 1;
pub const kShaDaWantMarks: c_int = 2;
pub const kShaDaForceit: c_int = 4;
pub const kShaDaGetOldfiles: c_int = 8;
pub const kShaDaMissingError: c_int = 16;
pub const kShaDaNanos: c_int = 32;
pub const kShaDaNoHistory: c_int = 64;
pub const kShaDaNoOpt: c_int = 128;

const NUL: c_char = 0;
const MAXLNUM: linenr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;
const EXTRA_MARKS: usize = 10;
const NMARKS: usize = 26;
const NGLOBALMARKS: usize = NMARKS + EXTRA_MARKS;
const NLOCALMARKS: usize = NMARKS + 3;
const JUMPLISTSIZE: usize = 100;
const NS_PER_SEC: u64 = 1_000_000_000;
const VAR_TYPE_BLOB: Integer = 10;
const BLN_LISTED: c_int = 2;
const kMarkBufLocal: c_int = 0;
const kListLenUnknown: isize = -1;
const MPACK_EOF: c_int = 1;
const SHADA_MPACK_FREE_SPACE: usize = 4 * MPACK_ITEM_SIZE;

const S_IFMT: u64 = 0o170000;
const S_IFDIR: u64 = 0o040000;

#[cfg(unix)]
const ROOT_UID: u32 = 0;

#[cfg(windows)]
const UV_ENOENT: c_int = -4058;
#[cfg(windows)]
const UV_EEXIST: c_int = -4075;
#[cfg(windows)]
const UV_ELOOP: c_int = -4067;

#[cfg(not(windows))]
const UV_ENOENT: c_int = -2;
#[cfg(not(windows))]
const UV_EEXIST: c_int = -17;
#[cfg(target_os = "linux")]
const UV_ELOOP: c_int = -40;
#[cfg(all(not(windows), not(target_os = "linux")))]
const UV_ELOOP: c_int = -62;

const INIT_FMARKV: fmarkv_T = fmarkv_T {
    topline_offset: MAXLNUM,
    skipcol: 0,
};

const DEFAULT_POS: pos_T = pos_T {
    lnum: 1,
    col: 0,
    coladd: 0,
};

const EMPTY_MAP_HASH: MapHash = MapHash {
    n_buckets: 0,
    size: 0,
    n_occupied: 0,
    upper_bound: 0,
    n_keys: 0,
    keys_capacity: 0,
    hash: ptr::null_mut(),
};

const fn set_init<K>() -> Set<K> {
    Set {
        h: EMPTY_MAP_HASH,
        keys: ptr::null_mut(),
    }
}

const fn map_init<K, V>() -> Map<K, V> {
    Map {
        set: set_init(),
        values: ptr::null_mut(),
    }
}

const KEYSET_OPTIDX__shada_buflist_item__c: u32 = 1;
const KEYSET_OPTIDX__shada_buflist_item__f: u32 = 2;
const KEYSET_OPTIDX__shada_buflist_item__l: u32 = 3;

const KEYSET_OPTIDX__shada_mark__c: u32 = 1;
const KEYSET_OPTIDX__shada_mark__f: u32 = 2;
const KEYSET_OPTIDX__shada_mark__l: u32 = 3;
const KEYSET_OPTIDX__shada_mark__n: u32 = 4;

const KEYSET_OPTIDX__shada_register__n: u32 = 1;
const KEYSET_OPTIDX__shada_register__rc: u32 = 2;
const KEYSET_OPTIDX__shada_register__rt: u32 = 3;
const KEYSET_OPTIDX__shada_register__ru: u32 = 4;
const KEYSET_OPTIDX__shada_register__rw: u32 = 5;

const KEYSET_OPTIDX__shada_search_pat__sb: u32 = 1;
const KEYSET_OPTIDX__shada_search_pat__sc: u32 = 2;
const KEYSET_OPTIDX__shada_search_pat__se: u32 = 3;
const KEYSET_OPTIDX__shada_search_pat__sh: u32 = 4;
const KEYSET_OPTIDX__shada_search_pat__sl: u32 = 5;
const KEYSET_OPTIDX__shada_search_pat__sm: u32 = 6;
const KEYSET_OPTIDX__shada_search_pat__so: u32 = 7;
const KEYSET_OPTIDX__shada_search_pat__sp: u32 = 8;
const KEYSET_OPTIDX__shada_search_pat__ss: u32 = 9;
const KEYSET_OPTIDX__shada_search_pat__su: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict__shada_buflist_item {
    pub is_set___shada_buflist_item_: OptionalKeys,
    pub l: Integer,
    pub c: Integer,
    pub f: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict__shada_mark {
    pub is_set___shada_mark_: OptionalKeys,
    pub n: Integer,
    pub l: Integer,
    pub c: Integer,
    pub f: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict__shada_register {
    pub is_set___shada_register_: OptionalKeys,
    pub rc: StringArray,
    pub ru: Boolean,
    pub rt: Integer,
    pub n: Integer,
    pub rw: Integer,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict__shada_search_pat {
    pub is_set___shada_search_pat_: OptionalKeys,
    pub magic: Boolean,
    pub smartcase: Boolean,
    pub has_line_offset: Boolean,
    pub place_cursor_at_end: Boolean,
    pub is_last_used: Boolean,
    pub is_substitute_pattern: Boolean,
    pub highlighted: Boolean,
    pub search_backward: Boolean,
    pub offset: Integer,
    pub pat: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubReplacementString {
    pub sub: *mut c_char,
    pub timestamp: Timestamp,
    pub additional_data: *mut AdditionalData,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShadaEntryType {
    kSDItemUnknown = -1,
    kSDItemMissing = 0,
    kSDItemHeader = 1,
    kSDItemSearchPattern = 2,
    kSDItemSubString = 3,
    kSDItemHistoryEntry = 4,
    kSDItemRegister = 5,
    kSDItemVariable = 6,
    kSDItemGlobalMark = 7,
    kSDItemJump = 8,
    kSDItemBufferList = 9,
    kSDItemLocalMark = 10,
    kSDItemChange = 11,
}

const SHADA_LAST_ENTRY: u64 = ShadaEntryType::kSDItemChange as i32 as u64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShaDaReadResult {
    kSDReadStatusSuccess,
    kSDReadStatusFinished,
    kSDReadStatusReadError,
    kSDReadStatusNotShaDa,
    kSDReadStatusMalformed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShaDaWriteResult {
    kSDWriteSuccessful,
    kSDWriteReadNotShada,
    kSDWriteFailed,
    kSDWriteIgnError,
}

const kSDReadHeader: c_uint = 1 << (ShadaEntryType::kSDItemHeader as u32);
const kSDReadUndisableableData: c_uint = (1 << (ShadaEntryType::kSDItemSearchPattern as u32))
    | (1 << (ShadaEntryType::kSDItemSubString as u32))
    | (1 << (ShadaEntryType::kSDItemJump as u32));
const kSDReadRegisters: c_uint = 1 << (ShadaEntryType::kSDItemRegister as u32);
const kSDReadHistory: c_uint = 1 << (ShadaEntryType::kSDItemHistoryEntry as u32);
const kSDReadVariables: c_uint = 1 << (ShadaEntryType::kSDItemVariable as u32);
const kSDReadBufferList: c_uint = 1 << (ShadaEntryType::kSDItemBufferList as u32);
const kSDReadUnknown: c_uint = 1 << ((SHADA_LAST_ENTRY as u32) + 1);
const kSDReadGlobalMarks: c_uint = 1 << (ShadaEntryType::kSDItemGlobalMark as u32);
const kSDReadLocalMarks: c_uint = 1 << (ShadaEntryType::kSDItemLocalMark as u32);
const kSDReadChanges: c_uint = 1 << (ShadaEntryType::kSDItemChange as u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaFilemark {
    name: c_char,
    mark: pos_T,
    fname: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaHistoryItem {
    histtype: u8,
    string: *mut c_char,
    sep: c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaReg {
    name: c_char,
    reg_type: u8,
    contents: *mut NvimString,
    is_unnamed: bool,
    contents_size: usize,
    width: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaGlobalVar {
    name: *mut c_char,
    value: typval_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaUnknownItem {
    item_type: u64,
    contents: *mut c_char,
    size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaSubString {
    sub: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BufferListBuffer {
    pos: pos_T,
    fname: *mut c_char,
    additional_data: *mut AdditionalData,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaBufferList {
    size: usize,
    buffers: *mut BufferListBuffer,
}

#[repr(C)]
#[derive(Clone, Copy)]
union ShadaEntryData {
    header: Dict,
    filemark: ShadaFilemark,
    search_pattern: KeyDict__shada_search_pat,
    history_item: ShadaHistoryItem,
    reg: ShadaReg,
    global_var: ShadaGlobalVar,
    unknown_item: ShadaUnknownItem,
    sub_string: ShadaSubString,
    buffer_list: ShadaBufferList,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShadaEntry {
    entry_type: ShadaEntryType,
    can_free_entry: bool,
    timestamp: Timestamp,
    data: ShadaEntryData,
    additional_data: *mut AdditionalData,
}

#[repr(C)]
struct HMLListEntry {
    data: ShadaEntry,
    next: *mut HMLListEntry,
    prev: *mut HMLListEntry,
}

#[repr(C)]
struct HMLList {
    entries: *mut HMLListEntry,
    first: *mut HMLListEntry,
    last: *mut HMLListEntry,
    free_entry: *mut HMLListEntry,
    last_free_entry: *mut HMLListEntry,
    size: usize,
    num_entries: usize,
    contained_entries: Map<cstr_t, ptr_t>,
}

#[repr(C)]
struct HistoryMergerState {
    hmll: HMLList,
    do_merge: bool,
    reading: bool,
    iter: *const c_void,
    last_hist_entry: ShadaEntry,
    history_type: u8,
}

#[repr(C)]
struct FileMarks {
    marks: [ShadaEntry; NLOCALMARKS],
    changes: [ShadaEntry; JUMPLISTSIZE],
    changes_size: usize,
    additional_marks: *mut ShadaEntry,
    additional_marks_size: usize,
    greatest_timestamp: Timestamp,
}

#[repr(C)]
struct WriteMergerState {
    hms: [HistoryMergerState; HIST_COUNT],
    global_marks: [ShadaEntry; NMARKS],
    numbered_marks: [ShadaEntry; EXTRA_MARKS],
    registers: [ShadaEntry; NUM_SAVED_REGISTERS],
    jumps: [ShadaEntry; JUMPLISTSIZE],
    jumps_size: usize,
    search_pattern: ShadaEntry,
    sub_search_pattern: ShadaEntry,
    replacement: ShadaEntry,
    dumped_variables: Set<cstr_t>,
    file_marks: Map<cstr_t, ptr_t>,
}

const MISSING_ENTRY: ShadaEntry = ShadaEntry {
    entry_type: ShadaEntryType::kSDItemMissing,
    can_free_entry: false,
    timestamp: 0,
    data: ShadaEntryData {
        sub_string: ShadaSubString {
            sub: ptr::null_mut(),
        },
    },
    additional_data: ptr::null_mut(),
};

const DEFAULT_SEARCH_PAT: KeyDict__shada_search_pat = KeyDict__shada_search_pat {
    is_set___shada_search_pat_: 0,
    magic: true,
    smartcase: false,
    has_line_offset: false,
    place_cursor_at_end: false,
    is_last_used: true,
    is_substitute_pattern: false,
    highlighted: false,
    search_backward: false,
    offset: 0,
    pat: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
};

fn sd_default_value(t: ShadaEntryType) -> ShadaEntry {
    match t {
        ShadaEntryType::kSDItemUnknown | ShadaEntryType::kSDItemMissing => MISSING_ENTRY,
        ShadaEntryType::kSDItemHeader => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemHeader,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                header: Dict {
                    items: ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemSearchPattern => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemSearchPattern,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                search_pattern: DEFAULT_SEARCH_PAT,
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemSubString => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemSubString,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                sub_string: ShadaSubString {
                    sub: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemHistoryEntry => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemHistoryEntry,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                history_item: ShadaHistoryItem {
                    histtype: HIST_CMD as u8,
                    string: ptr::null_mut(),
                    sep: NUL,
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemRegister => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemRegister,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                reg: ShadaReg {
                    name: NUL,
                    reg_type: MotionType::kMTCharWise as u8,
                    contents: ptr::null_mut(),
                    is_unnamed: false,
                    contents_size: 0,
                    width: 0,
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemVariable => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemVariable,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                global_var: ShadaGlobalVar {
                    name: ptr::null_mut(),
                    value: typval_T {
                        v_type: VarType::VAR_UNKNOWN,
                        v_lock: 0,
                        vval: TypvalVval {
                            v_string: ptr::null_mut(),
                        },
                    },
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemGlobalMark => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemGlobalMark,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                filemark: ShadaFilemark {
                    name: b'"' as c_char,
                    mark: DEFAULT_POS,
                    fname: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemJump => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemJump,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                filemark: ShadaFilemark {
                    name: NUL,
                    mark: DEFAULT_POS,
                    fname: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemBufferList => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemBufferList,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                buffer_list: ShadaBufferList {
                    size: 0,
                    buffers: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemLocalMark => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemLocalMark,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                filemark: ShadaFilemark {
                    name: b'"' as c_char,
                    mark: DEFAULT_POS,
                    fname: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
        ShadaEntryType::kSDItemChange => ShadaEntry {
            entry_type: ShadaEntryType::kSDItemChange,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData {
                filemark: ShadaFilemark {
                    name: NUL,
                    mark: DEFAULT_POS,
                    fname: ptr::null_mut(),
                },
            },
            additional_data: ptr::null_mut(),
        },
    }
}

static mut default_shada_file: *mut c_char = ptr::null_mut();

unsafe extern "C" {
    static mut longVersion: *mut c_char;

    fn KeyDict__shada_buflist_item_get_field(str_: *const c_char, len: usize) -> *const KeySetLink;
    fn KeyDict__shada_mark_get_field(str_: *const c_char, len: usize) -> *const KeySetLink;
    fn KeyDict__shada_register_get_field(str_: *const c_char, len: usize) -> *const KeySetLink;
    fn KeyDict__shada_search_pat_get_field(str_: *const c_char, len: usize) -> *const KeySetLink;

    fn var_flavour(varname: *mut c_char) -> var_flavour_T;
    fn var_set_global(name: *const c_char, vartv: typval_T);

    fn semsg(fmt: *const c_char, ...) -> bool;
    fn smsg(attr: c_int, fmt: *const c_char, ...) -> c_int;
    fn siemsg(fmt: *const c_char, ...);
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn verbose_enter();
    fn verbose_leave();
    #[link_name = "uv_strerror"]
    fn os_strerror(err: c_int) -> *const c_char;

    fn tv_copy(from: *const typval_T, to: *mut typval_T);
    fn tv_clear(varp: *mut typval_T);
    fn tv_list_alloc(len: isize) -> *mut list_T;
    fn tv_list_append_allocated_string(l: *mut list_T, str_: *mut c_char);
    fn get_copyID() -> c_int;
    fn set_ref_in_ht(ht: *mut hashtab_T, copyID: c_int, list_stack: *mut *mut c_void) -> bool;
    fn set_ref_in_list_items(l: *mut list_T, copyID: c_int, ht_stack: *mut *mut c_void) -> bool;

    fn set_no_hlsearch(flag: bool);
    fn sub_get_replacement(ret_sub: *mut SubReplacementString);
    fn sub_set_replacement(sub: SubReplacementString);
    fn regtilde(source: *mut c_char, magic: c_int, preview: bool) -> *mut c_char;

    fn buflist_new(
        ffname_arg: *mut c_char,
        sfname_arg: *mut c_char,
        lnum: linenr_T,
        flags: c_int,
    ) -> *mut buf_T;
    fn buflist_setfpos(
        buf: *mut buf_T,
        win: *mut win_T,
        lnum: linenr_T,
        col: colnr_T,
        copy_options: bool,
    );
    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn bt_quickfix(buf: *const buf_T) -> bool;
    fn bt_terminal(buf: *const buf_T) -> bool;

    fn expand_env(src: *const c_char, dst: *mut c_char, dstlen: c_int) -> usize;
    fn modname(fname: *const c_char, ext: *const c_char, prepend_dot: bool) -> *mut c_char;
    fn vim_rename(from: *const c_char, to: *const c_char) -> c_int;
    fn copy_option_part(
        option: *mut *mut c_char,
        buf: *mut c_char,
        maxlen: usize,
        sep_chars: *const c_char,
    ) -> usize;
    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;

    fn api_free_dict(value: Dict);
    fn api_free_string(value: NvimString);
    fn copy_string(str_: NvimString, arena: *mut c_void) -> NvimString;

    #[cfg(unix)]
    fn getuid() -> u32;
    #[cfg(unix)]
    fn getgid() -> u32;
}

#[inline]
const fn static_cstr_as_string(s: &'static CStr) -> NvimString {
    NvimString {
        data: s.as_ptr().cast_mut(),
        size: s.to_bytes().len(),
    }
}

#[inline]
unsafe fn file_eof(fp: *const FileDescriptor) -> bool {
    unsafe { (*fp).eof && (*fp).read_pos == (*fp).write_pos }
}

#[inline]
#[cfg(unix)]
unsafe fn file_fd(fp: *const FileDescriptor) -> c_int {
    unsafe { (*fp).fd }
}

#[inline]
unsafe fn file_space(fp: *const FileDescriptor) -> usize {
    unsafe { (((*fp).buffer as usize) + ARENA_BLOCK_SIZE).saturating_sub((*fp).write_pos as usize) }
}

#[inline]
unsafe fn tv_list_len(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_dict_hi2di(hi: *const hashitem_T) -> *mut dictitem_T {
    unsafe {
        (*hi)
            .hi_key
            .byte_sub(std::mem::offset_of!(dictitem_T, di_key))
            .cast::<dictitem_T>()
    }
}

#[inline]
unsafe fn xmemdupz_cstr(data: *const c_char, len: usize) -> *mut c_char {
    unsafe { xmemdupz(data.cast::<c_void>(), len).cast::<c_char>() }
}

unsafe extern "C-unwind" fn shada_buflist_item_get_field_cb(
    str_: *const c_char,
    len: usize,
) -> *const KeySetLink {
    unsafe { KeyDict__shada_buflist_item_get_field(str_, len) }
}

unsafe extern "C-unwind" fn shada_mark_get_field_cb(
    str_: *const c_char,
    len: usize,
) -> *const KeySetLink {
    unsafe { KeyDict__shada_mark_get_field(str_, len) }
}

unsafe extern "C-unwind" fn shada_register_get_field_cb(
    str_: *const c_char,
    len: usize,
) -> *const KeySetLink {
    unsafe { KeyDict__shada_register_get_field(str_, len) }
}

unsafe extern "C-unwind" fn shada_search_pat_get_field_cb(
    str_: *const c_char,
    len: usize,
) -> *const KeySetLink {
    unsafe { KeyDict__shada_search_pat_get_field(str_, len) }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        NvimString {
            data: s.cast_mut(),
            size: unsafe { CStr::from_ptr(s).to_bytes().len() },
        }
    }
}

#[inline]
unsafe fn strequal(a: *const c_char, b: *const c_char) -> bool {
    if a.is_null() || b.is_null() {
        return a == b;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == CStr::from_ptr(b).to_bytes() }
}

#[inline]
const fn mark_global_index(name: c_char) -> c_int {
    let u = name as u8;
    if u.is_ascii_uppercase() {
        (u - b'A') as c_int
    } else if u.is_ascii_digit() {
        (NMARKS as c_int) + ((u - b'0') as c_int)
    } else {
        -1
    }
}

#[inline]
const fn mark_local_index(name: c_char) -> c_int {
    let u = name as u8;
    if u.is_ascii_lowercase() {
        (u - b'a') as c_int
    } else if u == b'"' {
        NMARKS as c_int
    } else if u == b'^' {
        (NMARKS + 1) as c_int
    } else if u == b'.' {
        (NMARKS + 2) as c_int
    } else {
        -1
    }
}

#[inline]
unsafe fn set_has_ptr(set: *mut Set<ptr_t>, key: ptr_t) -> bool {
    unsafe { mh_get_ptr_t(set, key) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_ptr(set: *mut Set<ptr_t>, key: ptr_t) -> bool {
    let mut status = MHPutStatus::kMHExisting;
    unsafe {
        mh_put_ptr_t(set, key, &mut status);
    }
    status != MHPutStatus::kMHExisting
}

#[inline]
unsafe fn set_has_cstr(set: *mut Set<cstr_t>, key: cstr_t) -> bool {
    unsafe { mh_get_cstr_t(set, key) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_cstr(set: *mut Set<cstr_t>, key: cstr_t) -> bool {
    let mut status = MHPutStatus::kMHExisting;
    unsafe {
        mh_put_cstr_t(set, key, &mut status);
    }
    status != MHPutStatus::kMHExisting
}

#[inline]
unsafe fn set_has_path(set: *mut Set<path_t>, key: path_t) -> bool {
    unsafe { mh_get_path_t(set, key) != MH_TOMBSTONE }
}

#[inline]
unsafe fn set_put_path(set: *mut Set<path_t>, key: path_t) -> bool {
    let mut status = MHPutStatus::kMHExisting;
    unsafe {
        mh_put_path_t(set, key, &mut status);
    }
    status != MHPutStatus::kMHExisting
}

#[inline]
unsafe fn set_destroy<K>(set: *mut Set<K>) {
    unsafe {
        xfree((*set).keys.cast::<c_void>());
        xfree((*set).h.hash.cast::<c_void>());
        *set = set_init();
    }
}

#[inline]
unsafe fn map_destroy<K, V>(map: *mut Map<K, V>) {
    unsafe {
        set_destroy(&mut (*map).set);
        xfree((*map).values.cast::<c_void>());
        (*map).values = ptr::null_mut();
    }
}

#[inline]
unsafe fn hmll_init(hmll: *mut HMLList, size: usize) {
    unsafe {
        let entries = xcalloc(size, std::mem::size_of::<HMLListEntry>()).cast::<HMLListEntry>();
        *hmll = HMLList {
            entries,
            first: ptr::null_mut(),
            last: ptr::null_mut(),
            free_entry: ptr::null_mut(),
            last_free_entry: entries,
            size,
            num_entries: 0,
            contained_entries: map_init(),
        };
    }
}

#[inline]
unsafe fn hmll_remove(hmll: *mut HMLList, hmll_entry: *mut HMLListEntry) {
    unsafe {
        if hmll_entry == (*hmll).last_free_entry.sub(1) {
            (*hmll).last_free_entry = (*hmll).last_free_entry.sub(1);
        } else {
            assert!((*hmll).free_entry.is_null());
            (*hmll).free_entry = hmll_entry;
        }
        let val = map_del_cstr_t_ptr_t(
            &mut (*hmll).contained_entries,
            (*hmll_entry).data.data.history_item.string,
            ptr::null_mut(),
        );
        assert!(!val.is_null());
        if (*hmll_entry).next.is_null() {
            (*hmll).last = (*hmll_entry).prev;
        } else {
            (*(*hmll_entry).next).prev = (*hmll_entry).prev;
        }
        if (*hmll_entry).prev.is_null() {
            (*hmll).first = (*hmll_entry).next;
        } else {
            (*(*hmll_entry).prev).next = (*hmll_entry).next;
        }
        (*hmll).num_entries -= 1;
        shada_free_shada_entry(&mut (*hmll_entry).data);
    }
}

#[inline]
unsafe fn hmll_insert(hmll: *mut HMLList, mut hmll_entry: *mut HMLListEntry, data: ShadaEntry) {
    unsafe {
        if (*hmll).num_entries == (*hmll).size {
            if hmll_entry == (*hmll).first {
                hmll_entry = ptr::null_mut();
            }
            assert!(!(*hmll).first.is_null());
            hmll_remove(hmll, (*hmll).first);
        }
        let target_entry: *mut HMLListEntry;
        if (*hmll).free_entry.is_null() {
            assert!(
                ((*hmll).last_free_entry.offset_from((*hmll).entries) as usize)
                    == (*hmll).num_entries
            );
            target_entry = (*hmll).last_free_entry;
            (*hmll).last_free_entry = (*hmll).last_free_entry.add(1);
        } else {
            assert!(
                ((*hmll).last_free_entry.offset_from((*hmll).entries) as usize) - 1
                    == (*hmll).num_entries
            );
            target_entry = (*hmll).free_entry;
            (*hmll).free_entry = ptr::null_mut();
        }
        (*target_entry).data = data;
        let mut new_item = false;
        let val = map_put_ref_cstr_t_ptr_t(
            &mut (*hmll).contained_entries,
            data.data.history_item.string,
            ptr::null_mut(),
            &mut new_item,
        );
        if new_item {
            *val = target_entry.cast::<c_void>();
        }
        (*hmll).num_entries += 1;
        (*target_entry).prev = hmll_entry;
        if hmll_entry.is_null() {
            (*target_entry).next = (*hmll).first;
            (*hmll).first = target_entry;
        } else {
            (*target_entry).next = (*hmll_entry).next;
            (*hmll_entry).next = target_entry;
        }
        if (*target_entry).next.is_null() {
            (*hmll).last = target_entry;
        } else {
            (*(*target_entry).next).prev = target_entry;
        }
    }
}

#[inline]
unsafe fn hmll_dealloc(hmll: *mut HMLList) {
    unsafe {
        map_destroy(&mut (*hmll).contained_entries);
        xfree((*hmll).entries.cast::<c_void>());
    }
}

unsafe fn sd_reader_skip(sd_reader: *mut FileDescriptor, offset: usize) -> ShaDaReadResult {
    unsafe {
        let skip_bytes = file_skip(sd_reader, offset);
        if skip_bytes < 0 {
            semsg(
                gettext(c"E886: System error while skipping in ShaDa file: %s".as_ptr()),
                os_strerror(skip_bytes as c_int),
            );
            return ShaDaReadResult::kSDReadStatusReadError;
        } else if skip_bytes != (offset as isize) {
            assert!(skip_bytes < (offset as isize));
            if file_eof(sd_reader) {
                semsg(
                    gettext(
                        c"E576: Reading ShaDa file: last entry specified that it occupies %llu bytes, but file ended earlier"
                            .as_ptr(),
                    ),
                    offset as u64,
                );
            } else {
                semsg(
                    gettext(c"E886: System error while skipping in ShaDa file: %s".as_ptr()),
                    gettext(c"too few bytes read".as_ptr()),
                );
            }
            return ShaDaReadResult::kSDReadStatusNotShaDa;
        }
        ShaDaReadResult::kSDReadStatusSuccess
    }
}

unsafe fn close_file(cookie: *mut FileDescriptor) {
    unsafe {
        let error = file_close(cookie, p_fs != 0);
        if error != 0 {
            semsg(
                gettext(c"E886: System error while closing ShaDa file: %s".as_ptr()),
                os_strerror(error),
            );
        }
    }
}

unsafe fn shada_read_file(file: *const c_char, flags: c_int) -> c_int {
    unsafe {
        let fname = shada_filename(file);
        if fname.is_null() {
            return FAIL;
        }

        let mut sd_reader = std::mem::zeroed::<FileDescriptor>();
        let of_ret = file_open(&mut sd_reader, fname, K_FILE_READ_ONLY, 0);

        if p_verbose > 1 {
            verbose_enter();
            let empty = c"".as_ptr();
            smsg(
                0,
                gettext(c"Reading ShaDa file \"%s\"%s%s%s%s".as_ptr()),
                fname,
                if (flags & kShaDaWantInfo) != 0 {
                    gettext(c" info".as_ptr()).cast_const()
                } else {
                    empty
                },
                if (flags & kShaDaWantMarks) != 0 {
                    gettext(c" marks".as_ptr()).cast_const()
                } else {
                    empty
                },
                if (flags & kShaDaGetOldfiles) != 0 {
                    gettext(c" oldfiles".as_ptr()).cast_const()
                } else {
                    empty
                },
                if of_ret != 0 {
                    gettext(c" FAILED".as_ptr()).cast_const()
                } else {
                    empty
                },
            );
            verbose_leave();
        }

        if of_ret != 0 {
            if of_ret != UV_ENOENT || (flags & kShaDaMissingError) != 0 {
                semsg(
                    gettext(
                        c"E886: System error while opening ShaDa file %s for reading: %s".as_ptr(),
                    ),
                    fname,
                    os_strerror(of_ret),
                );
            }
            xfree(fname.cast::<c_void>());
            return FAIL;
        }
        xfree(fname.cast::<c_void>());

        shada_read(&mut sd_reader, flags);
        close_file(&mut sd_reader);

        OK
    }
}

unsafe fn shada_hist_iter(
    iter: *const c_void,
    history_type: u8,
    zero: bool,
    hist: *mut ShadaEntry,
) -> *const c_void {
    unsafe {
        let mut hist_he = std::mem::zeroed::<histentry_T>();
        let ret = hist_iter(iter, history_type, zero, &mut hist_he);
        if hist_he.hisstr.is_null() {
            *hist = MISSING_ENTRY;
        } else {
            let sep = if (history_type as c_int) == HIST_SEARCH {
                *hist_he.hisstr.add(hist_he.hisstrlen + 1)
            } else {
                0
            };
            *hist = ShadaEntry {
                can_free_entry: zero,
                entry_type: ShadaEntryType::kSDItemHistoryEntry,
                timestamp: hist_he.timestamp,
                data: ShadaEntryData {
                    history_item: ShadaHistoryItem {
                        histtype: history_type,
                        string: hist_he.hisstr,
                        sep,
                    },
                },
                additional_data: hist_he.additional_data,
            };
        }
        ret
    }
}

unsafe fn hms_insert(hms_p: *mut HistoryMergerState, entry: ShadaEntry, do_iter: bool) {
    unsafe {
        if do_iter {
            while (*hms_p).last_hist_entry.entry_type != ShadaEntryType::kSDItemMissing
                && (*hms_p).last_hist_entry.timestamp < entry.timestamp
            {
                hms_insert(hms_p, (*hms_p).last_hist_entry, false);
                if (*hms_p).iter.is_null() {
                    (*hms_p).last_hist_entry.entry_type = ShadaEntryType::kSDItemMissing;
                    break;
                }
                (*hms_p).iter = shada_hist_iter(
                    (*hms_p).iter,
                    (*hms_p).history_type,
                    (*hms_p).reading,
                    &mut (*hms_p).last_hist_entry,
                );
            }
        }
        let hmll = &mut (*hms_p).hmll;
        let mut key_alloc: *mut cstr_t = ptr::null_mut();
        let val = map_ref_cstr_t_ptr_t(
            &mut hmll.contained_entries,
            entry.data.history_item.string,
            &mut key_alloc,
        );
        if !val.is_null() {
            let existing_entry = (*val).cast::<HMLListEntry>();
            if entry.timestamp > (*existing_entry).data.timestamp {
                hmll_remove(hmll, existing_entry);
            } else if !do_iter && entry.timestamp == (*existing_entry).data.timestamp {
                shada_free_shada_entry(&mut (*existing_entry).data);
                (*existing_entry).data = entry;
                *key_alloc = entry.data.history_item.string;
                return;
            } else {
                return;
            }
        }
        let mut insert_after = hmll.last;
        while !insert_after.is_null() {
            if (*insert_after).data.timestamp <= entry.timestamp {
                break;
            }
            insert_after = (*insert_after).prev;
        }
        hmll_insert(hmll, insert_after, entry);
    }
}

#[inline]
unsafe fn hms_init(
    hms_p: *mut HistoryMergerState,
    history_type: u8,
    num_elements: usize,
    do_merge: bool,
    reading: bool,
) {
    unsafe {
        hmll_init(&mut (*hms_p).hmll, num_elements);
        (*hms_p).do_merge = do_merge;
        (*hms_p).reading = reading;
        (*hms_p).iter = shada_hist_iter(
            ptr::null(),
            history_type,
            (*hms_p).reading,
            &mut (*hms_p).last_hist_entry,
        );
        (*hms_p).history_type = history_type;
    }
}

#[inline]
unsafe fn hms_insert_whole_neovim_history(hms_p: *mut HistoryMergerState) {
    unsafe {
        while (*hms_p).last_hist_entry.entry_type != ShadaEntryType::kSDItemMissing {
            hms_insert(hms_p, (*hms_p).last_hist_entry, false);
            if (*hms_p).iter.is_null() {
                break;
            }
            (*hms_p).iter = shada_hist_iter(
                (*hms_p).iter,
                (*hms_p).history_type,
                (*hms_p).reading,
                &mut (*hms_p).last_hist_entry,
            );
        }
    }
}

#[inline]
unsafe fn hms_to_he_array(
    hms_p: *const HistoryMergerState,
    hist_array: *mut histentry_T,
    new_hisidx: *mut c_int,
    new_hisnum: *mut c_int,
) {
    unsafe {
        let mut hist = hist_array;
        let mut cur_entry = (*hms_p).hmll.first;
        while !cur_entry.is_null() {
            (*hist).timestamp = (*cur_entry).data.timestamp;
            (*hist).hisnum = (hist.offset_from(hist_array) as c_int) + 1;
            let s = (*cur_entry).data.data.history_item.string;
            (*hist).hisstr = s;
            (*hist).hisstrlen = CStr::from_ptr(s).to_bytes().len();
            (*hist).additional_data = (*cur_entry).data.additional_data;
            hist = hist.add(1);
            cur_entry = (*cur_entry).next;
        }
        *new_hisnum = hist.offset_from(hist_array) as c_int;
        *new_hisidx = *new_hisnum - 1;
    }
}

#[inline]
unsafe fn hms_dealloc(hms_p: *mut HistoryMergerState) {
    unsafe {
        hmll_dealloc(&mut (*hms_p).hmll);
    }
}

unsafe fn var_shada_iter(
    iter: *const c_void,
    name: *mut *const c_char,
    rettv: *mut typval_T,
    flavour: var_flavour_T,
) -> *const c_void {
    unsafe {
        let globvarht = get_globvar_ht();
        let hifirst = (*globvarht).ht_array;
        let hinum = ((*globvarht).ht_mask as usize) + 1;
        *name = ptr::null();
        let mut hi: *const hashitem_T;
        if iter.is_null() {
            hi = (*globvarht).ht_array;
            while (hi.offset_from(hifirst) as usize) < hinum
                && (hashitem_empty(&*hi) || (var_flavour((*hi).hi_key) & flavour) == 0)
            {
                hi = hi.add(1);
            }
            if (hi.offset_from(hifirst) as usize) == hinum {
                return ptr::null();
            }
        } else {
            hi = iter.cast::<hashitem_T>();
        }
        let di = tv_dict_hi2di(hi);
        *name = (&raw const (*di).di_key).cast::<c_char>();
        tv_copy(&(*di).di_tv, rettv);
        loop {
            hi = hi.add(1);
            if (hi.offset_from(hifirst) as usize) >= hinum {
                break;
            }
            if !hashitem_empty(&*hi) && (var_flavour((*hi).hi_key) & flavour) != 0 {
                return hi.cast::<c_void>();
            }
        }
        ptr::null()
    }
}

unsafe fn find_buffer(fname_bufs: *mut Map<cstr_t, ptr_t>, fname: *const c_char) -> *mut buf_T {
    unsafe {
        let mut key_alloc: *mut cstr_t = ptr::null_mut();
        let mut new_item = false;
        let r = map_put_ref_cstr_t_ptr_t(fname_bufs, fname, &mut key_alloc, &mut new_item)
            .cast::<*mut buf_T>();
        if new_item {
            *key_alloc = xstrdup(fname);
        } else {
            return *r;
        }

        let mut buf = firstbuf;
        while !buf.is_null() {
            if !(*buf).b_ffname.is_null() && path_equal(fname, (*buf).b_ffname, kPathCmpLiteral) {
                *r = buf;
                return buf;
            }
            buf = (*buf).b_next;
        }
        *r = ptr::null_mut();
        ptr::null_mut()
    }
}

#[inline]
const fn marks_equal(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col
}

unsafe fn marklist_insert(
    jumps_arr: *mut c_void,
    jump_size: usize,
    jl_len: c_int,
    mut i: c_int,
) -> c_int {
    unsafe {
        let jumps = jumps_arr.cast::<u8>();
        if i > 0 {
            if jl_len == (JUMPLISTSIZE as c_int) {
                i -= 1;
                if i > 0 {
                    ptr::copy(jumps.add(jump_size), jumps, jump_size * (i as usize));
                }
            } else if i != jl_len {
                ptr::copy(
                    jumps.add((i as usize) * jump_size),
                    jumps.add(((i + 1) as usize) * jump_size),
                    jump_size * ((jl_len - i) as usize),
                );
            }
        } else if i == 0 {
            if jl_len == (JUMPLISTSIZE as c_int) {
                return -1;
            } else if jl_len > 0 {
                ptr::copy(jumps, jumps.add(jump_size), jump_size * (jl_len as usize));
            }
        }
        i
    }
}

unsafe fn shada_read(sd_reader: *mut FileDescriptor, flags: c_int) {
    unsafe {
        let mut oldfiles_list = get_vim_var_list(VV_OLDFILES);
        let force = (flags & kShaDaForceit) != 0;
        let get_old_files =
            (flags & kShaDaGetOldfiles) != 0 && (force || tv_list_len(oldfiles_list) == 0);
        let want_marks = (flags & kShaDaWantMarks) != 0;
        let no_opt = (flags & kShaDaNoOpt) != 0;
        let argcount = (*(*curwin).w_alist.cast::<alist_T>()).al_ga.ga_len;
        let srni_flags: c_uint = (if (flags & kShaDaWantInfo) != 0 {
            kSDReadUndisableableData
                | kSDReadRegisters
                | kSDReadGlobalMarks
                | if p_hi != 0 && (flags & kShaDaNoHistory) == 0 {
                    kSDReadHistory
                } else {
                    0
                }
                | if no_opt || !find_shada_parameter(b'!' as c_int).is_null() {
                    kSDReadVariables
                } else {
                    0
                }
                | if (no_opt || !find_shada_parameter(b'%' as c_int).is_null()) && argcount == 0 {
                    kSDReadBufferList
                } else {
                    0
                }
        } else {
            0
        }) | if want_marks && get_shada_parameter(b'\'' as c_int) > 0 {
            kSDReadLocalMarks | kSDReadChanges
        } else {
            0
        } | if get_old_files { kSDReadLocalMarks } else { 0 };

        if srni_flags == 0 {
            return;
        }
        let mut hms = std::mem::zeroed::<[HistoryMergerState; HIST_COUNT]>();
        if (srni_flags & kSDReadHistory) != 0 {
            for i in 0..HIST_COUNT {
                hms_init(&mut hms[i], i as u8, p_hi as usize, true, true);
            }
        }
        let mut cur_entry = MISSING_ENTRY;
        let mut cl_bufs: Set<ptr_t> = set_init();
        let mut fname_bufs: Map<cstr_t, ptr_t> = map_init();
        let mut oldfiles_set: Set<path_t> = set_init();
        if get_old_files && (oldfiles_list.is_null() || force) {
            oldfiles_list = tv_list_alloc(kListLenUnknown);
            set_vim_var_list(VV_OLDFILES, oldfiles_list);
        }

        loop {
            let srni_ret = shada_read_next_item(sd_reader, &mut cur_entry, srni_flags, 0);
            if srni_ret == ShaDaReadResult::kSDReadStatusFinished {
                break;
            }
            match srni_ret {
                ShaDaReadResult::kSDReadStatusSuccess => {}
                ShaDaReadResult::kSDReadStatusFinished => std::process::abort(),
                ShaDaReadResult::kSDReadStatusNotShaDa
                | ShaDaReadResult::kSDReadStatusReadError => break,
                ShaDaReadResult::kSDReadStatusMalformed => continue,
            }
            match cur_entry.entry_type {
                ShadaEntryType::kSDItemMissing => std::process::abort(),
                ShadaEntryType::kSDItemUnknown => {}
                ShadaEntryType::kSDItemHeader => {
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::kSDItemSearchPattern => {
                    let sp = cur_entry.data.search_pattern;
                    if !force {
                        let mut pat = std::mem::zeroed::<SearchPattern>();
                        if sp.is_substitute_pattern {
                            get_substitute_pattern(&mut pat);
                        } else {
                            get_search_pattern(&mut pat);
                        }
                        if !pat.pat.is_null() && pat.timestamp >= cur_entry.timestamp {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    }

                    let spat = SearchPattern {
                        magic: sp.magic,
                        no_scs: !sp.smartcase,
                        off: SearchOffset {
                            dir: (if sp.search_backward { b'?' } else { b'/' }) as c_char,
                            line: sp.has_line_offset,
                            end: sp.place_cursor_at_end,
                            off: sp.offset,
                        },
                        pat: sp.pat.data,
                        patlen: sp.pat.size,
                        additional_data: cur_entry.additional_data,
                        timestamp: cur_entry.timestamp,
                    };

                    if sp.is_substitute_pattern {
                        set_substitute_pattern(spat);
                    } else {
                        set_search_pattern(spat);
                    }

                    if sp.is_last_used {
                        set_last_used_pattern(sp.is_substitute_pattern);
                        set_no_hlsearch(!sp.highlighted);
                    }
                }
                ShadaEntryType::kSDItemSubString => {
                    if !force {
                        let mut sub = std::mem::zeroed::<SubReplacementString>();
                        sub_get_replacement(&mut sub);
                        if !sub.sub.is_null() && sub.timestamp >= cur_entry.timestamp {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    }
                    sub_set_replacement(SubReplacementString {
                        sub: cur_entry.data.sub_string.sub,
                        timestamp: cur_entry.timestamp,
                        additional_data: cur_entry.additional_data,
                    });
                    regtilde(cur_entry.data.sub_string.sub, p_magic, false);
                }
                ShadaEntryType::kSDItemHistoryEntry => {
                    let ht = cur_entry.data.history_item.histtype as usize;
                    if ht >= HIST_COUNT {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    hms_insert(&mut hms[ht], cur_entry, true);
                }
                ShadaEntryType::kSDItemRegister => {
                    let r = cur_entry.data.reg;
                    let y_type = match r.reg_type {
                        0 => MotionType::kMTCharWise,
                        1 => MotionType::kMTLineWise,
                        2 => MotionType::kMTBlockWise,
                        _ => {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    };
                    let reg_ts = if (flags & kShaDaNanos) != 0 {
                        cur_entry.timestamp
                    } else {
                        cur_entry.timestamp.wrapping_mul(NS_PER_SEC)
                    };
                    if !force {
                        let reg = op_reg_get(r.name);
                        if reg.is_null() || (*reg).timestamp >= reg_ts {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    }
                    if !op_reg_set(
                        r.name,
                        yankreg_T {
                            y_array: r.contents,
                            y_size: r.contents_size,
                            y_type,
                            y_width: r.width as colnr_T,
                            timestamp: reg_ts,
                            additional_data: cur_entry.additional_data,
                        },
                        r.is_unnamed,
                    ) {
                        shada_free_shada_entry(&mut cur_entry);
                    }
                }
                ShadaEntryType::kSDItemVariable => {
                    var_set_global(
                        cur_entry.data.global_var.name,
                        cur_entry.data.global_var.value,
                    );
                    cur_entry.data.global_var.value.v_type = VarType::VAR_UNKNOWN;
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::kSDItemJump | ShadaEntryType::kSDItemGlobalMark => {
                    let buf = find_buffer(&mut fname_bufs, cur_entry.data.filemark.fname);
                    if !buf.is_null() {
                        xfree(cur_entry.data.filemark.fname.cast::<c_void>());
                        cur_entry.data.filemark.fname = ptr::null_mut();
                    }
                    let fm = xfmark_T {
                        fname: if buf.is_null() {
                            cur_entry.data.filemark.fname
                        } else {
                            ptr::null_mut()
                        },
                        fmark: fmark_T {
                            mark: cur_entry.data.filemark.mark,
                            fnum: if buf.is_null() { 0 } else { (*buf).b_fnum },
                            timestamp: cur_entry.timestamp,
                            view: INIT_FMARKV,
                            additional_data: cur_entry.additional_data,
                        },
                    };
                    if cur_entry.entry_type == ShadaEntryType::kSDItemGlobalMark {
                        if !mark_set_global(cur_entry.data.filemark.name, fm, !force) {
                            shada_free_shada_entry(&mut cur_entry);
                        }
                    } else {
                        let mut i = (*curwin).w_jumplistlen;
                        while i > 0 {
                            let jl_entry = (*curwin).w_jumplist[(i - 1) as usize];
                            if jl_entry.fmark.timestamp <= cur_entry.timestamp {
                                if marks_equal(jl_entry.fmark.mark, cur_entry.data.filemark.mark)
                                    && (if buf.is_null() {
                                        !jl_entry.fname.is_null()
                                            && strequal(fm.fname, jl_entry.fname)
                                    } else {
                                        fm.fmark.fnum == jl_entry.fmark.fnum
                                    })
                                {
                                    i = -1;
                                }
                                break;
                            }
                            i -= 1;
                        }
                        if i > 0 && (*curwin).w_jumplistlen == (JUMPLISTSIZE as c_int) {
                            free_xfmark((*curwin).w_jumplist[0]);
                        }
                        i = marklist_insert(
                            (&raw mut (*curwin).w_jumplist).cast::<c_void>(),
                            std::mem::size_of::<xfmark_T>(),
                            (*curwin).w_jumplistlen,
                            i,
                        );
                        if i != -1 {
                            (*curwin).w_jumplist[i as usize] = fm;
                            if (*curwin).w_jumplistlen < (JUMPLISTSIZE as c_int) {
                                (*curwin).w_jumplistlen += 1;
                            }
                            if (*curwin).w_jumplistidx >= i
                                && (*curwin).w_jumplistidx < (*curwin).w_jumplistlen
                            {
                                (*curwin).w_jumplistidx += 1;
                            }
                        } else {
                            shada_free_shada_entry(&mut cur_entry);
                        }
                    }
                }
                ShadaEntryType::kSDItemBufferList => {
                    let bl = cur_entry.data.buffer_list;
                    for i in 0..bl.size {
                        let item = &mut *bl.buffers.add(i);
                        let sfname = path_try_shorten_fname(item.fname);
                        let buf = buflist_new(item.fname, sfname, 0, BLN_LISTED);
                        if !buf.is_null() {
                            free_fmark((*buf).b_last_cursor);
                            (*buf).b_last_cursor = fmark_T {
                                mark: item.pos,
                                fnum: 0,
                                timestamp: os_time(),
                                view: INIT_FMARKV,
                                additional_data: ptr::null_mut(),
                            };
                            buflist_setfpos(
                                buf,
                                curwin,
                                (*buf).b_last_cursor.mark.lnum,
                                (*buf).b_last_cursor.mark.col,
                                false,
                            );
                            xfree((*buf).additional_data);
                            (*buf).additional_data = item.additional_data;
                            item.additional_data = ptr::null_mut();
                        }
                    }
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::kSDItemChange | ShadaEntryType::kSDItemLocalMark => {
                    if get_old_files
                        && !set_has_path(&mut oldfiles_set, cur_entry.data.filemark.fname)
                    {
                        let mut fname = cur_entry.data.filemark.fname;
                        if want_marks {
                            fname = xstrdup(fname);
                        }
                        set_put_path(&mut oldfiles_set, fname);
                        tv_list_append_allocated_string(oldfiles_list, fname);
                        if !want_marks {
                            cur_entry.data.filemark.fname = ptr::null_mut();
                        }
                    }
                    if !want_marks {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    let buf = find_buffer(&mut fname_bufs, cur_entry.data.filemark.fname);
                    if buf.is_null() {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    let fm = fmark_T {
                        mark: cur_entry.data.filemark.mark,
                        fnum: (*buf).b_fnum,
                        timestamp: cur_entry.timestamp,
                        view: INIT_FMARKV,
                        additional_data: cur_entry.additional_data,
                    };
                    if cur_entry.entry_type == ShadaEntryType::kSDItemLocalMark {
                        if !mark_set_local(cur_entry.data.filemark.name, buf, fm, !force) {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    } else {
                        set_put_ptr(&mut cl_bufs, buf.cast::<c_void>());
                        let mut i = (*buf).b_changelistlen;
                        while i > 0 {
                            let jl_entry = (*buf).b_changelist[(i - 1) as usize];
                            if jl_entry.timestamp <= cur_entry.timestamp {
                                if marks_equal(jl_entry.mark, cur_entry.data.filemark.mark) {
                                    i = -1;
                                }
                                break;
                            }
                            i -= 1;
                        }
                        if i > 0 && (*buf).b_changelistlen == (JUMPLISTSIZE as c_int) {
                            free_fmark((*buf).b_changelist[0]);
                        }
                        i = marklist_insert(
                            (&raw mut (*buf).b_changelist).cast::<c_void>(),
                            std::mem::size_of::<fmark_T>(),
                            (*buf).b_changelistlen,
                            i,
                        );
                        if i != -1 {
                            (*buf).b_changelist[i as usize] = fm;
                            if (*buf).b_changelistlen < (JUMPLISTSIZE as c_int) {
                                (*buf).b_changelistlen += 1;
                            }
                        } else {
                            xfree(fm.additional_data);
                        }
                    }
                    xfree(cur_entry.data.filemark.fname.cast::<c_void>());
                }
            }
        }

        if (srni_flags & kSDReadHistory) != 0 {
            for i in 0..HIST_COUNT {
                hms_insert_whole_neovim_history(&mut hms[i]);
                clr_history(i as c_int);
                let mut new_hisidx: *mut c_int = ptr::null_mut();
                let mut new_hisnum: *mut c_int = ptr::null_mut();
                let hist = hist_get_array(i as u8, &mut new_hisidx, &mut new_hisnum);
                if !hist.is_null() {
                    hms_to_he_array(&hms[i], hist, new_hisidx, new_hisnum);
                }
                hms_dealloc(&mut hms[i]);
            }
        }
        if cl_bufs.h.n_occupied != 0 {
            let mut tp = first_tabpage;
            while !tp.is_null() {
                let mut wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
                while !wp.is_null() {
                    if set_has_ptr(&mut cl_bufs, (*wp).w_buffer.cast::<c_void>()) {
                        (*wp).w_changelistidx = (*(*wp).w_buffer).b_changelistlen;
                    }
                    wp = (*wp).w_next;
                }
                tp = (*tp).tp_next;
            }
        }
        set_destroy(&mut cl_bufs);
        for i in 0..fname_bufs.set.h.n_keys {
            let key = *fname_bufs.set.keys.add(i as usize);
            xfree(key.cast_mut().cast::<c_void>());
        }
        map_destroy(&mut fname_bufs);
        set_destroy(&mut oldfiles_set);
    }
}

unsafe fn shada_get_default_file() -> *const c_char {
    unsafe {
        if default_shada_file.is_null() {
            let shada_dir = stdpaths_user_state_subpath(c"shada".as_ptr(), 0, false);
            default_shada_file = concat_fnames_realloc(shada_dir, c"main.shada".as_ptr(), true);
        }
        default_shada_file
    }
}

unsafe fn shada_filename(mut file: *const c_char) -> *mut c_char {
    unsafe {
        if file.is_null() || *file == NUL {
            if !p_shadafile.is_null() && *p_shadafile != NUL {
                if !strequal(p_shadafile, c"NONE".as_ptr()) {
                    file = p_shadafile;
                } else {
                    return ptr::null_mut();
                }
            } else {
                file = find_shada_parameter(b'n' as c_int);
                if file.is_null() || *file == NUL {
                    file = shada_get_default_file();
                }
                let name_buff_ptr = (&raw mut NameBuff).cast::<c_char>();
                let len = expand_env(file, name_buff_ptr, MAXPATHL as c_int);
                return xmemdupz_cstr(name_buff_ptr, len);
            }
        }
        xstrdup(file)
    }
}

#[inline]
unsafe fn shada_check_buffer(packer: *mut PackerBuffer) {
    unsafe {
        let p = &mut *packer;
        let remaining = (p.endptr as usize).saturating_sub(p.ptr as usize);
        if remaining < SHADA_MPACK_FREE_SPACE
            && let Some(flush) = p.packer_flush
        {
            flush(packer);
        }
    }
}

#[inline]
unsafe fn additional_data_len(src: *mut AdditionalData) -> u32 {
    if src.is_null() {
        0
    } else {
        unsafe { (*src.cast::<AdditionalDataHeader>()).nitems }
    }
}

#[inline]
unsafe fn dump_additional_data(src: *mut AdditionalData, sbuf: *mut PackerBuffer) {
    if !src.is_null() {
        unsafe {
            let hdr = src.cast::<AdditionalDataHeader>();
            let data = src
                .cast::<c_char>()
                .add(std::mem::size_of::<AdditionalDataHeader>());
            mpack_raw(data, (*hdr).nbytes as usize, sbuf);
        }
    }
}

unsafe fn shada_pack_entry(
    packer: *mut PackerBuffer,
    entry: ShadaEntry,
    max_kbyte: usize,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::kSDWriteFailed;
        let mut sbuf = packer_string_buffer();
        let def = sd_default_value(entry.entry_type);

        shada_check_buffer(&mut sbuf);
        match entry.entry_type {
            ShadaEntryType::kSDItemMissing => std::process::abort(),
            ShadaEntryType::kSDItemUnknown => {
                mpack_raw(
                    entry.data.unknown_item.contents,
                    entry.data.unknown_item.size,
                    &mut sbuf,
                );
            }
            ShadaEntryType::kSDItemHistoryEntry => {
                let is_hist_search = (entry.data.history_item.histtype as c_int) == HIST_SEARCH;
                let arr_size =
                    2 + u32::from(is_hist_search) + additional_data_len(entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                mpack_uint(&mut sbuf.ptr, u32::from(entry.data.history_item.histtype));
                mpack_bin(cstr_as_string(entry.data.history_item.string), &mut sbuf);
                if is_hist_search {
                    mpack_uint(&mut sbuf.ptr, u32::from(entry.data.history_item.sep as u8));
                }
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemVariable => {
                let is_blob = entry.data.global_var.value.v_type == VarType::VAR_BLOB;
                let arr_size = 2 + u32::from(is_blob) + additional_data_len(entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                let varname = cstr_as_string(entry.data.global_var.name);
                mpack_bin(varname, &mut sbuf);
                let mut vardesc = [0 as c_char; 256];
                let prefix = b"variable g:";
                ptr::copy_nonoverlapping(
                    prefix.as_ptr().cast::<c_char>(),
                    vardesc.as_mut_ptr(),
                    prefix.len(),
                );
                let copy_len = varname.size.min(vardesc.len() - prefix.len() - 1);
                ptr::copy_nonoverlapping(
                    varname.data,
                    vardesc.as_mut_ptr().add(prefix.len()),
                    copy_len,
                );
                vardesc[prefix.len() + copy_len] = 0;
                if encode_vim_to_msgpack(
                    &mut sbuf,
                    &raw const entry.data.global_var.value as *mut typval_T,
                    vardesc.as_ptr(),
                ) == FAIL
                {
                    ret = ShaDaWriteResult::kSDWriteIgnError;
                    semsg(
                        gettext(c"E574: Failed to write variable %s".as_ptr()),
                        entry.data.global_var.name,
                    );
                    xfree(sbuf.startptr.cast::<c_void>());
                    return ret;
                }
                if is_blob {
                    mpack_check_buffer(&mut sbuf);
                    mpack_integer(&mut sbuf.ptr, VAR_TYPE_BLOB);
                }
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemSubString => {
                let arr_size = 1 + additional_data_len(entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                mpack_bin(cstr_as_string(entry.data.sub_string.sub), &mut sbuf);
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemSearchPattern => {
                let sp = entry.data.search_pattern;
                let dsp = def.data.search_pattern;
                let entry_map_size = 1
                    + u32::from(sp.magic != dsp.magic)
                    + u32::from(sp.is_last_used != dsp.is_last_used)
                    + u32::from(sp.smartcase != dsp.smartcase)
                    + u32::from(sp.has_line_offset != dsp.has_line_offset)
                    + u32::from(sp.place_cursor_at_end != dsp.place_cursor_at_end)
                    + u32::from(sp.is_substitute_pattern != dsp.is_substitute_pattern)
                    + u32::from(sp.highlighted != dsp.highlighted)
                    + u32::from(sp.offset != dsp.offset)
                    + u32::from(sp.search_backward != dsp.search_backward)
                    + additional_data_len(entry.additional_data);
                mpack_map(&mut sbuf.ptr, entry_map_size);
                mpack_str(static_cstr_as_string(c"sp"), &mut sbuf);
                mpack_bin(sp.pat, &mut sbuf);
                if sp.magic != dsp.magic {
                    mpack_str(static_cstr_as_string(c"sm"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.magic);
                }
                if sp.is_last_used != dsp.is_last_used {
                    mpack_str(static_cstr_as_string(c"su"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.is_last_used);
                }
                if sp.smartcase != dsp.smartcase {
                    mpack_str(static_cstr_as_string(c"sc"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.smartcase);
                }
                if sp.has_line_offset != dsp.has_line_offset {
                    mpack_str(static_cstr_as_string(c"sl"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.has_line_offset);
                }
                if sp.place_cursor_at_end != dsp.place_cursor_at_end {
                    mpack_str(static_cstr_as_string(c"se"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.place_cursor_at_end);
                }
                if sp.is_substitute_pattern != dsp.is_substitute_pattern {
                    mpack_str(static_cstr_as_string(c"ss"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.is_substitute_pattern);
                }
                if sp.highlighted != dsp.highlighted {
                    mpack_str(static_cstr_as_string(c"sh"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.highlighted);
                }
                if sp.search_backward != dsp.search_backward {
                    mpack_str(static_cstr_as_string(c"sb"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, !dsp.search_backward);
                }
                if sp.offset != dsp.offset {
                    mpack_str(static_cstr_as_string(c"so"), &mut sbuf);
                    mpack_integer(&mut sbuf.ptr, sp.offset);
                }
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemChange
            | ShadaEntryType::kSDItemGlobalMark
            | ShadaEntryType::kSDItemLocalMark
            | ShadaEntryType::kSDItemJump => {
                let fm = entry.data.filemark;
                let dfm = def.data.filemark;
                let entry_map_size = 1
                    + u32::from(fm.mark.lnum != dfm.mark.lnum)
                    + u32::from(fm.mark.col != dfm.mark.col)
                    + u32::from(fm.name != dfm.name)
                    + additional_data_len(entry.additional_data);
                mpack_map(&mut sbuf.ptr, entry_map_size);
                mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
                mpack_bin(cstr_as_string(fm.fname), &mut sbuf);
                if fm.mark.lnum != dfm.mark.lnum {
                    mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                    mpack_integer(&mut sbuf.ptr, i64::from(fm.mark.lnum));
                }
                if fm.mark.col != dfm.mark.col {
                    mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                    mpack_integer(&mut sbuf.ptr, i64::from(fm.mark.col));
                }
                assert!(if entry.entry_type == ShadaEntryType::kSDItemJump
                    || entry.entry_type == ShadaEntryType::kSDItemChange
                {
                    fm.name == dfm.name
                } else {
                    true
                });
                if fm.name != dfm.name {
                    mpack_str(static_cstr_as_string(c"n"), &mut sbuf);
                    mpack_uint(&mut sbuf.ptr, u32::from(fm.name as u8));
                }
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemRegister => {
                let r = entry.data.reg;
                let dr = def.data.reg;
                let entry_map_size = 2
                    + u32::from(r.reg_type != dr.reg_type)
                    + u32::from(r.width != dr.width)
                    + u32::from(r.is_unnamed != dr.is_unnamed)
                    + additional_data_len(entry.additional_data);
                mpack_map(&mut sbuf.ptr, entry_map_size);
                mpack_str(static_cstr_as_string(c"rc"), &mut sbuf);
                mpack_array(&mut sbuf.ptr, r.contents_size as u32);
                for i in 0..r.contents_size {
                    mpack_bin(*r.contents.add(i), &mut sbuf);
                }
                mpack_str(static_cstr_as_string(c"n"), &mut sbuf);
                mpack_uint(&mut sbuf.ptr, u32::from(r.name as u8));
                if r.reg_type != dr.reg_type {
                    mpack_str(static_cstr_as_string(c"rt"), &mut sbuf);
                    mpack_uint(&mut sbuf.ptr, u32::from(r.reg_type));
                }
                if r.width != dr.width {
                    mpack_str(static_cstr_as_string(c"rw"), &mut sbuf);
                    mpack_uint64(&mut sbuf.ptr, r.width as u64);
                }
                if r.is_unnamed != dr.is_unnamed {
                    mpack_str(static_cstr_as_string(c"ru"), &mut sbuf);
                    mpack_bool(&mut sbuf.ptr, r.is_unnamed);
                }
                dump_additional_data(entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::kSDItemBufferList => {
                let bl = entry.data.buffer_list;
                mpack_array(&mut sbuf.ptr, bl.size as u32);
                for i in 0..bl.size {
                    let b = *bl.buffers.add(i);
                    let entry_map_size = 1
                        + u32::from(b.pos.lnum != DEFAULT_POS.lnum)
                        + u32::from(b.pos.col != DEFAULT_POS.col)
                        + additional_data_len(b.additional_data);
                    mpack_map(&mut sbuf.ptr, entry_map_size);
                    mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
                    mpack_bin(cstr_as_string(b.fname), &mut sbuf);
                    if b.pos.lnum != 1 {
                        mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                        mpack_uint64(&mut sbuf.ptr, b.pos.lnum as u64);
                    }
                    if b.pos.col != 0 {
                        mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                        mpack_uint64(&mut sbuf.ptr, b.pos.col as u64);
                    }
                    dump_additional_data(b.additional_data, &mut sbuf);
                }
            }
            ShadaEntryType::kSDItemHeader => {
                let hdr = entry.data.header;
                mpack_map(&mut sbuf.ptr, hdr.size as u32);
                for i in 0..hdr.size {
                    let kv = *hdr.items.add(i);
                    mpack_str(kv.key, &mut sbuf);
                    match kv.value.obj_type {
                        ObjectType::kObjectTypeString => {
                            mpack_bin(kv.value.data.string, &mut sbuf);
                        }
                        ObjectType::kObjectTypeInteger => {
                            mpack_integer(&mut sbuf.ptr, kv.value.data.integer);
                        }
                        _ => std::process::abort(),
                    }
                }
            }
        }

        let packed = packer_take_string(&mut sbuf);
        if max_kbyte == 0 || packed.size <= max_kbyte * 1024 {
            shada_check_buffer(packer);
            if entry.entry_type == ShadaEntryType::kSDItemUnknown {
                mpack_uint64(&mut (*packer).ptr, entry.data.unknown_item.item_type);
            } else {
                mpack_uint64(&mut (*packer).ptr, entry.entry_type as i32 as u64);
            }
            mpack_uint64(&mut (*packer).ptr, entry.timestamp);
            if packed.size > 0 {
                mpack_uint64(&mut (*packer).ptr, packed.size as u64);
                mpack_raw(packed.data, packed.size, packer);
            }
            if (*packer).anyint != 0 {
                xfree(sbuf.startptr.cast::<c_void>());
                return ret;
            }
        }
        ret = ShaDaWriteResult::kSDWriteSuccessful;
        xfree(sbuf.startptr.cast::<c_void>());
        ret
    }
}

#[inline]
unsafe fn shada_pack_pfreed_entry(
    packer: *mut PackerBuffer,
    mut entry: ShadaEntry,
    max_kbyte: usize,
) -> ShaDaWriteResult {
    unsafe {
        let ret = shada_pack_entry(packer, entry, max_kbyte);
        shada_free_shada_entry(&mut entry);
        ret
    }
}

unsafe fn shada_check_status(
    initial_fpos: u64,
    status: c_int,
    remaining: usize,
) -> ShaDaReadResult {
    unsafe {
        match status {
            MPACK_OK => {
                if remaining != 0 {
                    semsg(
                        gettext(
                            c"E576: Failed to parse ShaDa file: extra bytes in msgpack string at position %llu"
                                .as_ptr(),
                        ),
                        initial_fpos,
                    );
                    return ShaDaReadResult::kSDReadStatusNotShaDa;
                }
                ShaDaReadResult::kSDReadStatusSuccess
            }
            MPACK_EOF => {
                semsg(
                    gettext(
                        c"E576: Failed to parse ShaDa file: incomplete msgpack string at position %llu"
                            .as_ptr(),
                    ),
                    initial_fpos,
                );
                ShaDaReadResult::kSDReadStatusNotShaDa
            }
            _ => {
                semsg(
                    gettext(
                        c"E576: Failed to parse ShaDa file due to a msgpack parser error at position %llu"
                            .as_ptr(),
                    ),
                    initial_fpos,
                );
                ShaDaReadResult::kSDReadStatusNotShaDa
            }
        }
    }
}

unsafe fn shada_read_when_writing(
    sd_reader: *mut FileDescriptor,
    srni_flags: c_uint,
    max_kbyte: usize,
    wms: *mut WriteMergerState,
    packer: *mut PackerBuffer,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::kSDWriteSuccessful;
        let mut entry = MISSING_ENTRY;

        loop {
            let srni_ret = shada_read_next_item(sd_reader, &mut entry, srni_flags, max_kbyte);
            if srni_ret == ShaDaReadResult::kSDReadStatusFinished {
                break;
            }
            match srni_ret {
                ShaDaReadResult::kSDReadStatusSuccess => {}
                ShaDaReadResult::kSDReadStatusFinished => std::process::abort(),
                ShaDaReadResult::kSDReadStatusNotShaDa => {
                    return ShaDaWriteResult::kSDWriteReadNotShada;
                }
                ShaDaReadResult::kSDReadStatusReadError => {
                    return ret;
                }
                ShaDaReadResult::kSDReadStatusMalformed => continue,
            }

            let compare_with_entry = |wms_entry: *mut ShadaEntry, mut e: ShadaEntry| {
                if (*wms_entry).entry_type != ShadaEntryType::kSDItemMissing {
                    if (*wms_entry).timestamp >= e.timestamp {
                        shada_free_shada_entry(&mut e);
                        return;
                    }
                    shada_free_shada_entry(wms_entry);
                }
                *wms_entry = e;
            };

            match entry.entry_type {
                ShadaEntryType::kSDItemMissing => {}
                ShadaEntryType::kSDItemHeader | ShadaEntryType::kSDItemBufferList => {
                    std::process::abort();
                }
                ShadaEntryType::kSDItemUnknown => {
                    ret = shada_pack_entry(packer, entry, 0);
                    shada_free_shada_entry(&mut entry);
                }
                ShadaEntryType::kSDItemSearchPattern => {
                    let is_sub = entry.data.search_pattern.is_substitute_pattern;
                    let wms_pat = if is_sub {
                        &raw mut (*wms).sub_search_pattern
                    } else {
                        &raw mut (*wms).search_pattern
                    };
                    if (*wms_pat).entry_type == ShadaEntryType::kSDItemMissing
                        && search_pattern_cleared(is_sub)
                        && get_search_pattern_timestamp(is_sub) >= entry.timestamp
                    {
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    compare_with_entry(wms_pat, entry);
                }
                ShadaEntryType::kSDItemSubString => {
                    compare_with_entry(&raw mut (*wms).replacement, entry);
                }
                ShadaEntryType::kSDItemHistoryEntry => {
                    let ht = entry.data.history_item.histtype as usize;
                    if ht >= HIST_COUNT {
                        ret = shada_pack_entry(packer, entry, 0);
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    if (*wms).hms[ht].hmll.size != 0 {
                        hms_insert(&mut (*wms).hms[ht], entry, true);
                    } else {
                        shada_free_shada_entry(&mut entry);
                    }
                }
                ShadaEntryType::kSDItemRegister => {
                    let idx = op_reg_index(c_int::from(entry.data.reg.name));
                    if idx < 0 {
                        ret = shada_pack_entry(packer, entry, 0);
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    let u_idx = idx as usize;
                    if (*wms).registers[u_idx].entry_type == ShadaEntryType::kSDItemMissing {
                        let reg = op_reg_get(entry.data.reg.name);
                        if !reg.is_null()
                            && reg_empty(reg)
                            && (*reg).timestamp / NS_PER_SEC >= entry.timestamp
                        {
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                    }
                    compare_with_entry(&raw mut (*wms).registers[u_idx], entry);
                }
                ShadaEntryType::kSDItemVariable => {
                    if !set_has_cstr(&mut (*wms).dumped_variables, entry.data.global_var.name) {
                        ret = shada_pack_entry(packer, entry, 0);
                    }
                    shada_free_shada_entry(&mut entry);
                }
                ShadaEntryType::kSDItemGlobalMark => {
                    let name = entry.data.filemark.name;
                    if (name as u8).is_ascii_digit() {
                        let mut processed_mark = false;
                        for i in (1..=EXTRA_MARKS).rev() {
                            let wms_entry = (*wms).numbered_marks[i - 1];
                            if wms_entry.entry_type != ShadaEntryType::kSDItemGlobalMark {
                                continue;
                            }
                            if wms_entry.timestamp == entry.timestamp
                                && wms_entry.additional_data.is_null()
                                && entry.additional_data.is_null()
                                && marks_equal(
                                    wms_entry.data.filemark.mark,
                                    entry.data.filemark.mark,
                                )
                                && strequal(
                                    wms_entry.data.filemark.fname,
                                    entry.data.filemark.fname,
                                )
                            {
                                shada_free_shada_entry(&mut entry);
                                processed_mark = true;
                                break;
                            }
                            if wms_entry.timestamp >= entry.timestamp {
                                processed_mark = true;
                                if i < EXTRA_MARKS {
                                    replace_numbered_mark(wms, i, entry);
                                } else {
                                    shada_free_shada_entry(&mut entry);
                                }
                                break;
                            }
                        }
                        if !processed_mark {
                            replace_numbered_mark(wms, 0, entry);
                        }
                    } else {
                        let idx = mark_global_index(name);
                        if idx < 0 {
                            ret = shada_pack_entry(packer, entry, 0);
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                        let u_idx = idx as usize;
                        let mark = if u_idx < 26 {
                            &raw mut (*wms).global_marks[u_idx]
                        } else {
                            &raw mut (*wms).numbered_marks[u_idx - 26]
                        };
                        if (*mark).entry_type == ShadaEntryType::kSDItemMissing
                            && namedfm[u_idx].fmark.timestamp >= entry.timestamp
                        {
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                        compare_with_entry(mark, entry);
                    }
                }
                ShadaEntryType::kSDItemChange | ShadaEntryType::kSDItemLocalMark => {
                    if shada_removable(entry.data.filemark.fname) {
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    let fname = entry.data.filemark.fname;
                    let mut key: *mut cstr_t = ptr::null_mut();
                    let mut new_item = false;
                    let val = map_put_ref_cstr_t_ptr_t(
                        &mut (*wms).file_marks,
                        fname,
                        &mut key,
                        &mut new_item,
                    );
                    if new_item {
                        *key = xstrdup(fname);
                    }
                    if (*val).is_null() {
                        *val = xcalloc(1, std::mem::size_of::<FileMarks>());
                    }
                    let filemarks = (*val).cast::<FileMarks>();
                    if entry.timestamp > (*filemarks).greatest_timestamp {
                        (*filemarks).greatest_timestamp = entry.timestamp;
                    }
                    if entry.entry_type == ShadaEntryType::kSDItemLocalMark {
                        let idx = mark_local_index(entry.data.filemark.name);
                        if idx < 0 {
                            (*filemarks).additional_marks_size += 1;
                            (*filemarks).additional_marks = xrealloc(
                                (*filemarks).additional_marks.cast::<c_void>(),
                                (*filemarks).additional_marks_size
                                    * std::mem::size_of::<ShadaEntry>(),
                            )
                            .cast::<ShadaEntry>();
                            *(*filemarks)
                                .additional_marks
                                .add((*filemarks).additional_marks_size - 1) = entry;
                        } else {
                            let wms_entry = &raw mut (*filemarks).marks[idx as usize];
                            let mut set_wms = true;
                            if (*wms_entry).entry_type != ShadaEntryType::kSDItemMissing {
                                if (*wms_entry).timestamp >= entry.timestamp {
                                    shada_free_shada_entry(&mut entry);
                                    continue;
                                }
                                if (*wms_entry).can_free_entry {
                                    if *key == (*wms_entry).data.filemark.fname.cast_const() {
                                        *key = entry.data.filemark.fname;
                                    }
                                    shada_free_shada_entry(wms_entry);
                                }
                            } else {
                                let mut buf = firstbuf;
                                while !buf.is_null() {
                                    if !(*buf).b_ffname.is_null()
                                        && path_equal(
                                            entry.data.filemark.fname,
                                            (*buf).b_ffname,
                                            kPathCmpLiteral,
                                        )
                                    {
                                        let mut fm = std::mem::zeroed::<fmark_T>();
                                        mark_get(
                                            buf,
                                            curwin,
                                            &mut fm,
                                            kMarkBufLocal,
                                            c_int::from(entry.data.filemark.name),
                                        );
                                        if fm.timestamp >= entry.timestamp {
                                            set_wms = false;
                                            shada_free_shada_entry(&mut entry);
                                            break;
                                        }
                                    }
                                    buf = (*buf).b_next;
                                }
                            }
                            if set_wms {
                                *wms_entry = entry;
                            }
                        }
                    } else {
                        let mut i = (*filemarks).changes_size as c_int;
                        while i > 0 {
                            let jl_entry = (*filemarks).changes[(i - 1) as usize];
                            if jl_entry.timestamp <= entry.timestamp {
                                if marks_equal(
                                    jl_entry.data.filemark.mark,
                                    entry.data.filemark.mark,
                                ) {
                                    i = -1;
                                }
                                break;
                            }
                            i -= 1;
                        }
                        if i > 0 && (*filemarks).changes_size == JUMPLISTSIZE {
                            shada_free_shada_entry(&mut (*filemarks).changes[0]);
                        }
                        i = marklist_insert(
                            (&raw mut (*filemarks).changes).cast::<c_void>(),
                            std::mem::size_of::<ShadaEntry>(),
                            (*filemarks).changes_size as c_int,
                            i,
                        );
                        if i != -1 {
                            (*filemarks).changes[i as usize] = entry;
                            if (*filemarks).changes_size < JUMPLISTSIZE {
                                (*filemarks).changes_size += 1;
                            }
                        } else {
                            shada_free_shada_entry(&mut entry);
                        }
                    }
                }
                ShadaEntryType::kSDItemJump => {
                    let mut i = (*wms).jumps_size as c_int;
                    while i > 0 {
                        let jl_entry = (*wms).jumps[(i - 1) as usize];
                        if jl_entry.timestamp <= entry.timestamp {
                            if marks_equal(jl_entry.data.filemark.mark, entry.data.filemark.mark)
                                && strequal(jl_entry.data.filemark.fname, entry.data.filemark.fname)
                            {
                                i = -1;
                            }
                            break;
                        }
                        i -= 1;
                    }
                    if i > 0 && (*wms).jumps_size == JUMPLISTSIZE {
                        shada_free_shada_entry(&mut (*wms).jumps[0]);
                    }
                    i = marklist_insert(
                        (&raw mut (*wms).jumps).cast::<c_void>(),
                        std::mem::size_of::<ShadaEntry>(),
                        (*wms).jumps_size as c_int,
                        i,
                    );
                    if i != -1 {
                        (*wms).jumps[i as usize] = entry;
                        if (*wms).jumps_size < JUMPLISTSIZE {
                            (*wms).jumps_size += 1;
                        }
                    } else {
                        shada_free_shada_entry(&mut entry);
                    }
                }
            }
        }
        ret
    }
}

#[inline]
unsafe fn ignore_buf(buf: *const buf_T, removable_bufs: *mut Set<ptr_t>) -> bool {
    unsafe {
        buf.is_null()
            || (*buf).b_ffname.is_null()
            || ((*buf).b_p_bl == 0 && (*buf).b_p_initialized)
            || bt_quickfix(buf)
            || bt_terminal(buf)
            || set_has_ptr(removable_bufs, buf.cast_mut().cast::<c_void>())
    }
}

#[inline]
unsafe fn shada_get_buflist(removable_bufs: *mut Set<ptr_t>) -> ShadaEntry {
    unsafe {
        let max_bufs = get_shada_parameter(b'%' as c_int);
        let mut buf_count = 0usize;
        let mut buf = firstbuf;
        while !buf.is_null() {
            if !ignore_buf(buf, removable_bufs)
                && (*buf).b_p_bl != 0
                && (max_bufs < 0 || buf_count < (max_bufs as usize))
            {
                buf_count += 1;
            }
            buf = (*buf).b_next;
        }

        let buffers =
            xmalloc(buf_count * std::mem::size_of::<BufferListBuffer>()).cast::<BufferListBuffer>();
        let mut i = 0usize;
        buf = firstbuf;
        while !buf.is_null() {
            if !ignore_buf(buf, removable_bufs) && (*buf).b_p_bl != 0 {
                if i >= buf_count {
                    break;
                }
                *buffers.add(i) = BufferListBuffer {
                    pos: (*buf).b_last_cursor.mark,
                    fname: (*buf).b_ffname,
                    additional_data: (*buf).additional_data,
                };
                i += 1;
            }
            buf = (*buf).b_next;
        }

        ShadaEntry {
            entry_type: ShadaEntryType::kSDItemBufferList,
            can_free_entry: false,
            timestamp: os_time(),
            data: ShadaEntryData {
                buffer_list: ShadaBufferList {
                    size: buf_count,
                    buffers,
                },
            },
            additional_data: ptr::null_mut(),
        }
    }
}

#[inline]
unsafe fn add_search_pattern(
    ret_pse: *mut ShadaEntry,
    get_pattern: unsafe extern "C" fn(*mut SearchPattern),
    is_substitute_pattern: bool,
    search_last_used: bool,
    search_highlighted: bool,
) {
    unsafe {
        let mut pat = std::mem::zeroed::<SearchPattern>();
        get_pattern(&mut pat);
        if !pat.pat.is_null() {
            let is_last_used = is_substitute_pattern ^ search_last_used;
            *ret_pse = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::kSDItemSearchPattern,
                timestamp: pat.timestamp,
                data: ShadaEntryData {
                    search_pattern: KeyDict__shada_search_pat {
                        is_set___shada_search_pat_: 0,
                        magic: pat.magic,
                        smartcase: !pat.no_scs,
                        has_line_offset: if is_substitute_pattern {
                            DEFAULT_SEARCH_PAT.has_line_offset
                        } else {
                            pat.off.line
                        },
                        place_cursor_at_end: if is_substitute_pattern {
                            DEFAULT_SEARCH_PAT.place_cursor_at_end
                        } else {
                            pat.off.end
                        },
                        offset: if is_substitute_pattern {
                            DEFAULT_SEARCH_PAT.offset
                        } else {
                            pat.off.off
                        },
                        is_last_used,
                        is_substitute_pattern,
                        highlighted: is_last_used && search_highlighted,
                        pat: cstr_as_string(pat.pat),
                        search_backward: !is_substitute_pattern && pat.off.dir == (b'?' as c_char),
                    },
                },
                additional_data: pat.additional_data,
            };
        }
    }
}

#[inline]
unsafe fn shada_initialize_registers(
    wms: *mut WriteMergerState,
    max_reg_lines: c_int,
    scale_ts: bool,
) {
    unsafe {
        let mut reg_iter: *const c_void = ptr::null();
        let limit_reg_lines = max_reg_lines >= 0;
        loop {
            let mut reg = std::mem::zeroed::<yankreg_T>();
            let mut name: c_char = NUL;
            let mut is_unnamed = false;
            reg_iter = op_global_reg_iter(reg_iter, &mut name, &mut reg, &mut is_unnamed);
            if name == NUL {
                break;
            }
            if limit_reg_lines && reg.y_size > (max_reg_lines as usize) {
                if reg_iter.is_null() {
                    break;
                }
                continue;
            }
            let idx = op_reg_index(c_int::from(name)) as usize;
            (*wms).registers[idx] = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::kSDItemRegister,
                timestamp: if scale_ts {
                    reg.timestamp / NS_PER_SEC
                } else {
                    reg.timestamp
                },
                data: ShadaEntryData {
                    reg: ShadaReg {
                        contents: reg.y_array,
                        contents_size: reg.y_size,
                        reg_type: reg.y_type as u8,
                        width: if reg.y_type == MotionType::kMTBlockWise {
                            reg.y_width as usize
                        } else {
                            0
                        },
                        name,
                        is_unnamed,
                    },
                },
                additional_data: reg.additional_data,
            };
            if reg_iter.is_null() {
                break;
            }
        }
    }
}

#[inline]
unsafe fn replace_numbered_mark(wms: *mut WriteMergerState, idx: usize, entry: ShadaEntry) {
    unsafe {
        shada_free_shada_entry(&mut (*wms).numbered_marks[EXTRA_MARKS - 1]);
        for i in idx..(EXTRA_MARKS - 1) {
            if (*wms).numbered_marks[i].entry_type == ShadaEntryType::kSDItemGlobalMark {
                (*wms).numbered_marks[i].data.filemark.name = (b'0' + (i as u8) + 1) as c_char;
            }
        }
        let base = (&raw mut (*wms).numbered_marks).cast::<ShadaEntry>();
        ptr::copy(base.add(idx), base.add(idx + 1), EXTRA_MARKS - 1 - idx);
        (*wms).numbered_marks[idx] = entry;
        (*wms).numbered_marks[idx].data.filemark.name = (b'0' + (idx as u8)) as c_char;
    }
}

#[inline]
unsafe fn find_removable_bufs(removable_bufs: *mut Set<ptr_t>) {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            if !(*buf).b_ffname.is_null() && shada_removable((*buf).b_ffname) {
                set_put_ptr(removable_bufs, buf.cast::<c_void>());
            }
            buf = (*buf).b_next;
        }
    }
}

const fn hist_type2char(hist_type: c_int) -> c_int {
    match hist_type {
        HIST_CMD => b':' as c_int,
        HIST_SEARCH => b'/' as c_int,
        HIST_EXPR => b'=' as c_int,
        HIST_INPUT => b'@' as c_int,
        HIST_DEBUG => b'>' as c_int,
        _ => 0,
    }
}

unsafe extern "C" fn flush_file_buffer(buffer: *mut PackerBuffer) {
    unsafe {
        let fd = (*buffer).anydata.cast::<FileDescriptor>();
        (*fd).write_pos = (*buffer).ptr;
        (*buffer).anyint = i64::from(file_flush(fd));
        (*buffer).ptr = (*fd).write_pos;
    }
}

unsafe fn packer_buffer_for_file(file: *mut FileDescriptor) -> PackerBuffer {
    unsafe {
        if file_space(file) < SHADA_MPACK_FREE_SPACE {
            file_flush(file);
        }
        PackerBuffer {
            startptr: (*file).buffer,
            ptr: (*file).write_pos,
            endptr: (*file).buffer.add(ARENA_BLOCK_SIZE),
            anydata: file.cast::<c_void>(),
            anyint: 0,
            packer_flush: Some(flush_file_buffer),
        }
    }
}

unsafe fn shada_write(
    sd_writer: *mut FileDescriptor,
    sd_reader: *mut FileDescriptor,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::kSDWriteSuccessful;
        let mut max_kbyte_i = get_shada_parameter(b's' as c_int);
        if max_kbyte_i < 0 {
            max_kbyte_i = 10;
        }
        if max_kbyte_i == 0 {
            return ret;
        }

        let wms = xcalloc(1, std::mem::size_of::<WriteMergerState>()).cast::<WriteMergerState>();
        let mut dump_one_history = [false; HIST_COUNT];
        let dump_global_vars = !find_shada_parameter(b'!' as c_int).is_null();
        let mut max_reg_lines = get_shada_parameter(b'<' as c_int);
        if max_reg_lines < 0 {
            max_reg_lines = get_shada_parameter(b'"' as c_int);
        }
        let dump_registers = max_reg_lines != 0;
        let mut removable_bufs: Set<ptr_t> = set_init();
        let max_kbyte = max_kbyte_i as usize;
        let num_marked_files = get_shada_parameter(b'\'' as c_int) as usize;
        let dump_global_marks = get_shada_parameter(b'f' as c_int) != 0;
        let mut dump_history = false;

        for i in 0..HIST_COUNT {
            let mut num_saved = get_shada_parameter(hist_type2char(i as c_int));
            if num_saved == -1 {
                num_saved = p_hi as c_int;
            }
            if num_saved > 0 {
                dump_history = true;
                dump_one_history[i] = true;
                hms_init(
                    &mut (*wms).hms[i],
                    i as u8,
                    num_saved as usize,
                    !sd_reader.is_null(),
                    false,
                );
            } else {
                dump_one_history[i] = false;
            }
        }

        let srni_flags: c_uint = kSDReadUndisableableData
            | kSDReadUnknown
            | if dump_history { kSDReadHistory } else { 0 }
            | if dump_registers { kSDReadRegisters } else { 0 }
            | if dump_global_vars {
                kSDReadVariables
            } else {
                0
            }
            | if dump_global_marks {
                kSDReadGlobalMarks
            } else {
                0
            }
            | if num_marked_files != 0 {
                kSDReadLocalMarks | kSDReadChanges
            } else {
                0
            };

        let mut packer = packer_buffer_for_file(sd_writer);

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                set_last_cursor(wp);
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        find_removable_bufs(&mut removable_bufs);

        let mut header_items = [
            KeyValuePair {
                key: static_cstr_as_string(c"generator"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: static_cstr_as_string(c"nvim"),
                    },
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"version"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string(longVersion),
                    },
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"max_kbyte"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: max_kbyte as Integer,
                    },
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"pid"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: os_get_pid() as Integer,
                    },
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"encoding"),
                value: Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string(p_enc),
                    },
                },
            },
        ];

        let write_body: ShaDaWriteResult = 'shada_write_exit: {
            if shada_pack_entry(
                &mut packer,
                ShadaEntry {
                    entry_type: ShadaEntryType::kSDItemHeader,
                    can_free_entry: false,
                    timestamp: os_time(),
                    data: ShadaEntryData {
                        header: Dict {
                            items: header_items.as_mut_ptr(),
                            size: 5,
                            capacity: 5,
                        },
                    },
                    additional_data: ptr::null_mut(),
                },
                0,
            ) == ShaDaWriteResult::kSDWriteFailed
            {
                break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
            }

            if !find_shada_parameter(b'%' as c_int).is_null() {
                let buflist_entry = shada_get_buflist(&mut removable_bufs);
                if shada_pack_entry(&mut packer, buflist_entry, 0)
                    == ShaDaWriteResult::kSDWriteFailed
                {
                    xfree(buflist_entry.data.buffer_list.buffers.cast::<c_void>());
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
                xfree(buflist_entry.data.buffer_list.buffers.cast::<c_void>());
            }

            if dump_global_vars {
                let mut var_iter: *const c_void = ptr::null();
                let cur_timestamp = os_time();
                loop {
                    let mut vartv = std::mem::zeroed::<typval_T>();
                    let mut name: *const c_char = ptr::null();
                    var_iter = var_shada_iter(var_iter, &mut name, &mut vartv, VAR_FLAVOUR_SHADA);
                    if name.is_null() {
                        break;
                    }
                    let mut skip = false;
                    match vartv.v_type {
                        VarType::VAR_FUNC | VarType::VAR_PARTIAL => {
                            tv_clear(&mut vartv);
                            skip = true;
                        }
                        VarType::VAR_DICT => {
                            let di = vartv.vval.v_dict;
                            let copy_id = get_copyID();
                            if !set_ref_in_ht(&mut (*di).dv_hashtab, copy_id, ptr::null_mut())
                                && copy_id == (*di).dv_copyID
                            {
                                tv_clear(&mut vartv);
                                skip = true;
                            }
                        }
                        VarType::VAR_LIST => {
                            let l = vartv.vval.v_list;
                            let copy_id = get_copyID();
                            if !set_ref_in_list_items(l, copy_id, ptr::null_mut())
                                && copy_id == (*l).lv_copyID
                            {
                                tv_clear(&mut vartv);
                                skip = true;
                            }
                        }
                        _ => {}
                    }
                    if !skip {
                        let mut tgttv = std::mem::zeroed::<typval_T>();
                        tv_copy(&vartv, &mut tgttv);
                        let spe_ret = shada_pack_entry(
                            &mut packer,
                            ShadaEntry {
                                entry_type: ShadaEntryType::kSDItemVariable,
                                can_free_entry: false,
                                timestamp: cur_timestamp,
                                data: ShadaEntryData {
                                    global_var: ShadaGlobalVar {
                                        name: name.cast_mut(),
                                        value: tgttv,
                                    },
                                },
                                additional_data: ptr::null_mut(),
                            },
                            max_kbyte,
                        );
                        if spe_ret == ShaDaWriteResult::kSDWriteFailed {
                            tv_clear(&mut vartv);
                            tv_clear(&mut tgttv);
                            break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                        }
                        tv_clear(&mut vartv);
                        tv_clear(&mut tgttv);
                        if spe_ret == ShaDaWriteResult::kSDWriteSuccessful {
                            set_put_cstr(&mut (*wms).dumped_variables, name);
                        }
                    }
                    if var_iter.is_null() {
                        break;
                    }
                }
            }

            if num_marked_files > 0 {
                (*wms).jumps_size = shada_init_jumps(
                    (&raw mut (*wms).jumps).cast::<ShadaEntry>(),
                    &mut removable_bufs,
                );
            }

            if dump_one_history[HIST_SEARCH as usize] {
                let search_highlighted =
                    !(Search.no_hlsearch || !find_shada_parameter(b'h' as c_int).is_null());
                let search_last_used = search_was_last_used();

                add_search_pattern(
                    &mut (*wms).search_pattern,
                    get_search_pattern,
                    false,
                    search_last_used,
                    search_highlighted,
                );
                add_search_pattern(
                    &mut (*wms).sub_search_pattern,
                    get_substitute_pattern,
                    true,
                    search_last_used,
                    search_highlighted,
                );

                let mut sub = std::mem::zeroed::<SubReplacementString>();
                sub_get_replacement(&mut sub);
                if !sub.sub.is_null() {
                    (*wms).replacement = ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::kSDItemSubString,
                        timestamp: sub.timestamp,
                        data: ShadaEntryData {
                            sub_string: ShadaSubString { sub: sub.sub },
                        },
                        additional_data: sub.additional_data,
                    };
                }
            }

            if dump_global_marks {
                let mut global_mark_iter: *const c_void = ptr::null();
                let mut digit_mark_idx = 0usize;
                loop {
                    let mut name: c_char = NUL;
                    let mut fm = std::mem::zeroed::<xfmark_T>();
                    global_mark_iter = mark_global_iter(global_mark_iter, &mut name, &mut fm);
                    if name == NUL {
                        break;
                    }
                    let fname: *const c_char;
                    if fm.fmark.fnum == 0 {
                        assert!(!fm.fname.is_null());
                        if shada_removable(fm.fname) {
                            if global_mark_iter.is_null() {
                                break;
                            }
                            continue;
                        }
                        fname = fm.fname;
                    } else {
                        let buf = buflist_findnr(fm.fmark.fnum);
                        if buf.is_null()
                            || (*buf).b_ffname.is_null()
                            || set_has_ptr(&mut removable_bufs, buf.cast::<c_void>())
                        {
                            if global_mark_iter.is_null() {
                                break;
                            }
                            continue;
                        }
                        fname = (*buf).b_ffname;
                    }
                    let entry = ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::kSDItemGlobalMark,
                        timestamp: fm.fmark.timestamp,
                        data: ShadaEntryData {
                            filemark: ShadaFilemark {
                                mark: fm.fmark.mark,
                                name,
                                fname: fname.cast_mut(),
                            },
                        },
                        additional_data: fm.fmark.additional_data,
                    };
                    if (name as u8).is_ascii_digit() {
                        replace_numbered_mark(wms, digit_mark_idx, entry);
                        digit_mark_idx += 1;
                    } else {
                        (*wms).global_marks[mark_global_index(name) as usize] = entry;
                    }
                    if global_mark_iter.is_null() {
                        break;
                    }
                }
            }

            if dump_registers {
                shada_initialize_registers(wms, max_reg_lines, true);
            }

            if num_marked_files > 0 {
                let mut buf = firstbuf;
                while !buf.is_null() {
                    if !ignore_buf(buf, &mut removable_bufs) {
                        let mut local_marks_iter: *const c_void = ptr::null();
                        let fname = (*buf).b_ffname;
                        let mut map_key: *mut cstr_t = ptr::null_mut();
                        let mut new_item = false;
                        let val = map_put_ref_cstr_t_ptr_t(
                            &mut (*wms).file_marks,
                            fname,
                            &mut map_key,
                            &mut new_item,
                        );
                        if new_item {
                            *map_key = xstrdup(fname);
                        }
                        if (*val).is_null() {
                            *val = xcalloc(1, std::mem::size_of::<FileMarks>());
                        }
                        let filemarks = (*val).cast::<FileMarks>();
                        loop {
                            let mut fm = std::mem::zeroed::<fmark_T>();
                            let mut name: c_char = NUL;
                            local_marks_iter =
                                mark_buffer_iter(local_marks_iter, buf, &mut name, &mut fm);
                            if name == NUL {
                                break;
                            }
                            (*filemarks).marks[mark_local_index(name) as usize] = ShadaEntry {
                                can_free_entry: false,
                                entry_type: ShadaEntryType::kSDItemLocalMark,
                                timestamp: fm.timestamp,
                                data: ShadaEntryData {
                                    filemark: ShadaFilemark {
                                        mark: fm.mark,
                                        name,
                                        fname,
                                    },
                                },
                                additional_data: fm.additional_data,
                            };
                            if fm.timestamp > (*filemarks).greatest_timestamp {
                                (*filemarks).greatest_timestamp = fm.timestamp;
                            }
                            if local_marks_iter.is_null() {
                                break;
                            }
                        }
                        for i in 0..((*buf).b_changelistlen as usize) {
                            let fm = (*buf).b_changelist[i];
                            (*filemarks).changes[i] = ShadaEntry {
                                can_free_entry: false,
                                entry_type: ShadaEntryType::kSDItemChange,
                                timestamp: fm.timestamp,
                                data: ShadaEntryData {
                                    filemark: ShadaFilemark {
                                        name: NUL,
                                        mark: fm.mark,
                                        fname,
                                    },
                                },
                                additional_data: fm.additional_data,
                            };
                            if fm.timestamp > (*filemarks).greatest_timestamp {
                                (*filemarks).greatest_timestamp = fm.timestamp;
                            }
                        }
                        (*filemarks).changes_size = (*buf).b_changelistlen as usize;
                    }
                    buf = (*buf).b_next;
                }
            }

            if !sd_reader.is_null() {
                let srww_ret =
                    shada_read_when_writing(sd_reader, srni_flags, max_kbyte, wms, &mut packer);
                if srww_ret != ShaDaWriteResult::kSDWriteSuccessful {
                    ret = srww_ret;
                }
            }

            if dump_global_marks
                && !ignore_buf(curbuf, &mut removable_bufs)
                && (*curwin).w_cursor.lnum != 0
            {
                replace_numbered_mark(
                    wms,
                    0,
                    ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::kSDItemGlobalMark,
                        timestamp: os_time(),
                        data: ShadaEntryData {
                            filemark: ShadaFilemark {
                                mark: (*curwin).w_cursor,
                                name: b'0' as c_char,
                                fname: (*curbuf).b_ffname,
                            },
                        },
                        additional_data: ptr::null_mut(),
                    },
                );
            }

            for entry in (*wms).global_marks {
                if entry.entry_type != ShadaEntryType::kSDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::kSDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
            }
            for entry in (*wms).numbered_marks {
                if entry.entry_type != ShadaEntryType::kSDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::kSDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
            }
            for entry in (*wms).registers {
                if entry.entry_type != ShadaEntryType::kSDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::kSDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
            }
            for i in 0..(*wms).jumps_size {
                if shada_pack_pfreed_entry(&mut packer, (*wms).jumps[i], max_kbyte)
                    == ShaDaWriteResult::kSDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
            }
            for entry in [
                (*wms).search_pattern,
                (*wms).sub_search_pattern,
                (*wms).replacement,
            ] {
                if entry.entry_type != ShadaEntryType::kSDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::kSDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                }
            }

            let file_markss_size = (*wms).file_marks.set.h.size as usize;
            let mut all_file_markss: Vec<*mut FileMarks> = Vec::with_capacity(file_markss_size);
            for i in 0..(*wms).file_marks.set.h.n_keys {
                let val = *(*wms).file_marks.values.add(i as usize);
                all_file_markss.push(val.cast::<FileMarks>());
            }
            all_file_markss.sort_by(|&a, &b| {
                let ta = (*a).greatest_timestamp;
                let tb = (*b).greatest_timestamp;
                if ta == tb {
                    Ordering::Equal
                } else if ta > tb {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            });
            let file_markss_to_dump = num_marked_files.min(all_file_markss.len());
            for &fm_ptr in &all_file_markss[..file_markss_to_dump] {
                for entry in (*fm_ptr).marks {
                    if entry.entry_type != ShadaEntryType::kSDItemMissing
                        && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                            == ShaDaWriteResult::kSDWriteFailed
                    {
                        break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                    }
                }
                for j in 0..(*fm_ptr).changes_size {
                    if shada_pack_pfreed_entry(&mut packer, (*fm_ptr).changes[j], max_kbyte)
                        == ShaDaWriteResult::kSDWriteFailed
                    {
                        break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                    }
                }
                for j in 0..(*fm_ptr).additional_marks_size {
                    let add_ptr = (*fm_ptr).additional_marks.add(j);
                    if shada_pack_entry(&mut packer, *add_ptr, 0)
                        == ShaDaWriteResult::kSDWriteFailed
                    {
                        shada_free_shada_entry(add_ptr);
                        break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                    }
                    shada_free_shada_entry(add_ptr);
                }
                xfree((*fm_ptr).additional_marks.cast::<c_void>());
            }

            if dump_history {
                for i in 0..HIST_COUNT {
                    if dump_one_history[i] {
                        hms_insert_whole_neovim_history(&mut (*wms).hms[i]);
                        let mut cur_entry = (*wms).hms[i].hmll.first;
                        while !cur_entry.is_null() {
                            if shada_pack_pfreed_entry(&mut packer, (*cur_entry).data, max_kbyte)
                                == ShaDaWriteResult::kSDWriteFailed
                            {
                                break 'shada_write_exit ShaDaWriteResult::kSDWriteFailed;
                            }
                            cur_entry = (*cur_entry).next;
                        }
                    }
                }
            }

            ret
        };
        ret = write_body;

        for i in 0..HIST_COUNT {
            if dump_one_history[i] {
                hms_dealloc(&mut (*wms).hms[i]);
            }
        }
        for i in 0..(*wms).file_marks.set.h.n_keys {
            let stored_key = *(*wms).file_marks.set.keys.add(i as usize);
            let val = *(*wms).file_marks.values.add(i as usize);
            xfree(stored_key.cast_mut().cast::<c_void>());
            xfree(val);
        }
        map_destroy(&mut (*wms).file_marks);
        set_destroy(&mut removable_bufs);
        if let Some(flush) = packer.packer_flush {
            flush(&mut packer);
        }
        set_destroy(&mut (*wms).dumped_variables);
        xfree(wms.cast::<c_void>());
        ret
    }
}

/// Write ShaDa file to a given location (`shada_write_file`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_write_file(file: *const c_char, mut nomerge: bool) -> c_int {
    unsafe {
        let fname = shada_filename(file);
        if fname.is_null() {
            return FAIL;
        }

        let mut tempname: *mut c_char = ptr::null_mut();
        let mut sd_writer = std::mem::zeroed::<FileDescriptor>();
        let mut sd_reader = std::mem::zeroed::<FileDescriptor>();
        let mut did_open_writer = false;
        let mut did_open_reader = false;

        if !nomerge {
            let mut error = file_open(&mut sd_reader, fname, K_FILE_READ_ONLY, 0);
            if error != 0 {
                if error != UV_ENOENT {
                    semsg(
                        gettext(
                            c"E886: System error while opening ShaDa file %s for reading to merge before writing it: %s"
                                .as_ptr(),
                        ),
                        fname,
                        os_strerror(error),
                    );
                }
                nomerge = true;
            } else {
                did_open_reader = true;
                tempname = modname(fname, c".tmp.a".as_ptr(), false);
                if tempname.is_null() {
                    nomerge = true;
                } else {
                    let mut perm = os_getperm(fname) as c_int;
                    perm = if perm >= 0 {
                        (perm & 0o777) | 0o600
                    } else {
                        0o600
                    };
                    loop {
                        error = file_open(
                            &mut sd_writer,
                            tempname,
                            K_FILE_CREATE_ONLY | K_FILE_NO_SYMLINK,
                            perm,
                        );
                        if error != 0 {
                            if error == UV_EEXIST || error == UV_ELOOP {
                                let wp =
                                    tempname.add(CStr::from_ptr(tempname).to_bytes().len() - 1);
                                if *wp == (b'z' as c_char) {
                                    semsg(
                                        gettext(
                                            c"E138: All %s.tmp.X files exist, cannot write ShaDa file!"
                                                .as_ptr(),
                                        ),
                                        fname,
                                    );
                                    xfree(fname.cast::<c_void>());
                                    xfree(tempname.cast::<c_void>());
                                    if did_open_reader {
                                        close_file(&mut sd_reader);
                                    }
                                    return FAIL;
                                }
                                *wp += 1;
                                continue;
                            } else {
                                semsg(
                                    gettext(
                                        c"E886: System error while opening temporary ShaDa file %s for writing: %s"
                                            .as_ptr(),
                                    ),
                                    tempname,
                                    os_strerror(error),
                                );
                            }
                        } else {
                            did_open_writer = true;
                        }
                        break;
                    }
                }
            }
        }

        if nomerge {
            let tail = path_tail_with_sep(fname);
            if tail != fname {
                let tail_save = *tail;
                *tail = NUL;
                if !os_isdir(fname) {
                    let mut failed_dir: *mut c_char = ptr::null_mut();
                    let ret = os_mkdir_recurse(fname, 0o700, &mut failed_dir, ptr::null_mut());
                    if ret != 0 {
                        semsg(
                            gettext(
                                c"E886: Failed to create directory %s for writing ShaDa file: %s"
                                    .as_ptr(),
                            ),
                            failed_dir,
                            os_strerror(ret),
                        );
                        xfree(fname.cast::<c_void>());
                        xfree(failed_dir.cast::<c_void>());
                        return FAIL;
                    }
                }
                *tail = tail_save;
            }
            let error = file_open(
                &mut sd_writer,
                fname,
                K_FILE_CREATE | K_FILE_TRUNCATE,
                0o600,
            );
            if error != 0 {
                semsg(
                    gettext(
                        c"E886: System error while opening ShaDa file %s for writing: %s".as_ptr(),
                    ),
                    fname,
                    os_strerror(error),
                );
            } else {
                did_open_writer = true;
            }
        }

        if !did_open_writer {
            xfree(fname.cast::<c_void>());
            xfree(tempname.cast::<c_void>());
            if did_open_reader {
                close_file(&mut sd_reader);
            }
            return FAIL;
        }

        if p_verbose > 1 {
            verbose_enter();
            smsg(0, gettext(c"Writing ShaDa file \"%s\"".as_ptr()), fname);
            verbose_leave();
        }

        let sw_ret = shada_write(
            &mut sd_writer,
            if nomerge {
                ptr::null_mut()
            } else {
                &mut sd_reader
            },
        );
        assert!(sw_ret != ShaDaWriteResult::kSDWriteIgnError);
        if !nomerge {
            if did_open_reader {
                close_file(&mut sd_reader);
            }
            let mut did_remove = false;
            if sw_ret == ShaDaWriteResult::kSDWriteSuccessful {
                let mut old_info = std::mem::zeroed::<FileInfo>();
                let is_dir = (old_info.stat.st_mode & S_IFMT) == S_IFDIR;
                #[cfg(unix)]
                let unix_not_writable = |info: &FileInfo| -> bool {
                    let uid = getuid();
                    let gid = getgid();
                    uid != ROOT_UID
                        && !(if info.stat.st_uid == u64::from(uid) {
                            (info.stat.st_mode & 0o200) != 0
                        } else if info.stat.st_gid == u64::from(gid) {
                            (info.stat.st_mode & 0o020) != 0
                        } else {
                            (info.stat.st_mode & 0o002) != 0
                        })
                };
                #[cfg(not(unix))]
                let unix_not_writable = |_info: &FileInfo| -> bool { false };

                let has_info = os_fileinfo(fname, &mut old_info);
                if !has_info
                    || (old_info.stat.st_mode & S_IFMT) == S_IFDIR
                    || is_dir
                    || unix_not_writable(&old_info)
                {
                    semsg(
                        gettext(c"E137: ShaDa file is not writable: %s".as_ptr()),
                        fname,
                    );
                } else {
                    #[allow(unused_mut)]
                    let mut chown_ok = true;
                    #[cfg(unix)]
                    {
                        if getuid() == ROOT_UID
                            && (old_info.stat.st_uid != u64::from(ROOT_UID)
                                || old_info.stat.st_gid != u64::from(getgid()))
                        {
                            let old_uid = old_info.stat.st_uid as u32;
                            let old_gid = old_info.stat.st_gid as u32;
                            let fchown_ret = os_fchown(file_fd(&sd_writer), old_uid, old_gid);
                            if fchown_ret != 0 {
                                semsg(
                                    gettext(
                                        c"E136: Failed setting uid and gid for file %s: %s"
                                            .as_ptr(),
                                    ),
                                    tempname,
                                    os_strerror(fchown_ret),
                                );
                                chown_ok = false;
                            }
                        }
                    }
                    if chown_ok {
                        if vim_rename(tempname, fname) == -1 {
                            semsg(
                                gettext(c"E136: Can't rename ShaDa file from %s to %s!".as_ptr()),
                                tempname,
                                fname,
                            );
                        } else {
                            did_remove = true;
                            os_remove(tempname);
                        }
                    }
                }
            } else if sw_ret == ShaDaWriteResult::kSDWriteReadNotShada {
                semsg(
                    gettext(
                        c"E136: Did not rename %s because %s does not look like a ShaDa file"
                            .as_ptr(),
                    ),
                    tempname,
                    fname,
                );
            } else {
                semsg(
                    gettext(
                        c"E136: Did not rename %s to %s because there were errors during writing it"
                            .as_ptr(),
                    ),
                    tempname,
                    fname,
                );
            }
            if !did_remove {
                semsg(
                    gettext(
                        c"E136: Do not forget to remove %s or rename it manually to %s.".as_ptr(),
                    ),
                    tempname,
                    fname,
                );
            }
            xfree(tempname.cast::<c_void>());
        }
        close_file(&mut sd_writer);

        xfree(fname.cast::<c_void>());
        OK
    }
}

/// Read marks information from ShaDa file (`shada_read_marks`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_read_marks() -> c_int {
    unsafe { shada_read_file(ptr::null(), kShaDaWantMarks) }
}

/// Read all information from ShaDa file (`shada_read_everything`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_read_everything(
    fname: *const c_char,
    forceit: bool,
    missing_ok: bool,
) -> c_int {
    unsafe {
        shada_read_file(
            fname,
            kShaDaWantInfo
                | kShaDaWantMarks
                | kShaDaGetOldfiles
                | if forceit { kShaDaForceit } else { 0 }
                | if missing_ok { 0 } else { kShaDaMissingError },
        )
    }
}

unsafe fn shada_free_shada_entry(entry: *mut ShadaEntry) {
    if entry.is_null() {
        return;
    }
    unsafe {
        if !(*entry).can_free_entry {
            return;
        }
        match (*entry).entry_type {
            ShadaEntryType::kSDItemMissing => {}
            ShadaEntryType::kSDItemUnknown => {
                xfree((*entry).data.unknown_item.contents.cast::<c_void>());
            }
            ShadaEntryType::kSDItemHeader => {
                api_free_dict((*entry).data.header);
            }
            ShadaEntryType::kSDItemChange
            | ShadaEntryType::kSDItemJump
            | ShadaEntryType::kSDItemGlobalMark
            | ShadaEntryType::kSDItemLocalMark => {
                xfree((*entry).data.filemark.fname.cast::<c_void>());
            }
            ShadaEntryType::kSDItemSearchPattern => {
                api_free_string((*entry).data.search_pattern.pat);
            }
            ShadaEntryType::kSDItemRegister => {
                for i in 0..(*entry).data.reg.contents_size {
                    api_free_string(*(*entry).data.reg.contents.add(i));
                }
                xfree((*entry).data.reg.contents.cast::<c_void>());
            }
            ShadaEntryType::kSDItemHistoryEntry => {
                xfree((*entry).data.history_item.string.cast::<c_void>());
            }
            ShadaEntryType::kSDItemVariable => {
                xfree((*entry).data.global_var.name.cast::<c_void>());
                tv_clear(&mut (*entry).data.global_var.value);
            }
            ShadaEntryType::kSDItemSubString => {
                xfree((*entry).data.sub_string.sub.cast::<c_void>());
            }
            ShadaEntryType::kSDItemBufferList => {
                for i in 0..(*entry).data.buffer_list.size {
                    let b = *(*entry).data.buffer_list.buffers.add(i);
                    xfree(b.fname.cast::<c_void>());
                    xfree(b.additional_data);
                }
                xfree((*entry).data.buffer_list.buffers.cast::<c_void>());
            }
        }
        xfree((*entry).additional_data);
        (*entry).additional_data = ptr::null_mut();
    }
}

unsafe fn fread_len(
    sd_reader: *mut FileDescriptor,
    buffer: *mut c_char,
    length: usize,
) -> ShaDaReadResult {
    unsafe {
        let read_bytes = file_read(sd_reader, buffer, length);
        if read_bytes < 0 {
            semsg(
                gettext(c"E886: System error while reading ShaDa file: %s".as_ptr()),
                os_strerror(read_bytes as c_int),
            );
            return ShaDaReadResult::kSDReadStatusReadError;
        }
        if read_bytes != (length as isize) {
            semsg(
                gettext(
                    c"E576: Error while reading ShaDa file: last entry specified that it occupies %llu bytes, but file ended earlier"
                        .as_ptr(),
                ),
                length as u64,
            );
            return ShaDaReadResult::kSDReadStatusNotShaDa;
        }
        ShaDaReadResult::kSDReadStatusSuccess
    }
}

unsafe fn msgpack_read_uint64(
    sd_reader: *mut FileDescriptor,
    allow_eof: bool,
    result: *mut u64,
) -> ShaDaReadResult {
    unsafe {
        let fpos = (*sd_reader).bytes_read;
        let mut ret: u8 = 0;
        let read_bytes = file_read(sd_reader, (&raw mut ret).cast::<c_char>(), 1);
        if read_bytes < 0 {
            semsg(
                gettext(c"E886: System error while reading integer from ShaDa file: %s".as_ptr()),
                os_strerror(read_bytes as c_int),
            );
            return ShaDaReadResult::kSDReadStatusReadError;
        } else if read_bytes == 0 {
            if allow_eof && file_eof(sd_reader) {
                return ShaDaReadResult::kSDReadStatusFinished;
            }
            semsg(
                gettext(
                    c"E576: Error while reading ShaDa file: expected positive integer at position %llu, but got nothing"
                        .as_ptr(),
                ),
                fpos,
            );
            return ShaDaReadResult::kSDReadStatusNotShaDa;
        }

        if (ret & 0x80) == 0 {
            *result = u64::from(ret);
        } else {
            let length = match ret {
                0xCC => 1usize,
                0xCD => 2usize,
                0xCE => 4usize,
                0xCF => 8usize,
                _ => {
                    semsg(
                        gettext(
                            c"E576: Error while reading ShaDa file: expected positive integer at position %llu"
                                .as_ptr(),
                        ),
                        fpos,
                    );
                    return ShaDaReadResult::kSDReadStatusNotShaDa;
                }
            };
            let mut buf = [0u8; 8];
            let fl_ret = fread_len(
                sd_reader,
                buf.as_mut_ptr().add(8 - length).cast::<c_char>(),
                length,
            );
            if fl_ret != ShaDaReadResult::kSDReadStatusSuccess {
                return fl_ret;
            }
            *result = u64::from_be_bytes(buf);
        }
        ShaDaReadResult::kSDReadStatusSuccess
    }
}

unsafe fn shada_read_next_item(
    sd_reader: *mut FileDescriptor,
    entry: *mut ShadaEntry,
    flags: c_uint,
    max_kbyte: usize,
) -> ShaDaReadResult {
    unsafe {
        let mut ret = ShaDaReadResult::kSDReadStatusMalformed;
        loop {
            *entry = MISSING_ENTRY;
            if file_eof(sd_reader) {
                return ShaDaReadResult::kSDReadStatusFinished;
            }

            let mut verify_but_ignore = false;
            let mut type_u64: u64 = 0;
            let mut timestamp_u64: u64 = 0;
            let mut length_u64: u64 = 0;

            let initial_fpos = (*sd_reader).bytes_read;
            let mut ad = AdditionalDataBuilder::new();
            let mut read_additional_array_elements: u32 = 0;
            let mut error_alloc: *mut c_char = ptr::null_mut();

            let mut mru_ret = msgpack_read_uint64(sd_reader, true, &mut type_u64);
            if mru_ret != ShaDaReadResult::kSDReadStatusSuccess {
                return mru_ret;
            }
            mru_ret = msgpack_read_uint64(sd_reader, false, &mut timestamp_u64);
            if mru_ret != ShaDaReadResult::kSDReadStatusSuccess {
                return mru_ret;
            }
            mru_ret = msgpack_read_uint64(sd_reader, false, &mut length_u64);
            if mru_ret != ShaDaReadResult::kSDReadStatusSuccess {
                return mru_ret;
            }

            if length_u64 > (isize::MAX as u64) {
                semsg(
                    gettext(
                        c"E576: Error while reading ShaDa file: there is an item at position %llu that is stated to be too long"
                            .as_ptr(),
                    ),
                    initial_fpos,
                );
                return ShaDaReadResult::kSDReadStatusNotShaDa;
            }

            let length = length_u64 as usize;
            (*entry).timestamp = timestamp_u64;
            (*entry).can_free_entry = true;

            if type_u64 == 0 {
                semsg(
                    gettext(
                        c"E576: Error while reading ShaDa file: there is an item at position %llu that must not be there: Missing items are for internal uses only"
                            .as_ptr(),
                    ),
                    initial_fpos,
                );
                return ShaDaReadResult::kSDReadStatusNotShaDa;
            }

            let skip_by_flags = if type_u64 > SHADA_LAST_ENTRY {
                (flags & kSDReadUnknown) == 0
            } else {
                ((1u32 << (type_u64 as u32)) & flags) == 0
            };
            if skip_by_flags || (max_kbyte != 0 && length > max_kbyte * 1024) {
                if initial_fpos == 0
                    && (type_u64 == u64::from(b'\n') || type_u64 > SHADA_LAST_ENTRY)
                {
                    verify_but_ignore = true;
                } else {
                    let srs_ret = sd_reader_skip(sd_reader, length);
                    if srs_ret != ShaDaReadResult::kSDReadStatusSuccess {
                        return srs_ret;
                    }
                    continue;
                }
            }

            let parse_pos = (*sd_reader).bytes_read;
            let mut buf_allocated = false;
            let mut buf = file_try_read_buffered(sd_reader, length);
            if buf.is_null() {
                buf_allocated = true;
                buf = xmalloc(length).cast::<c_char>();
                let fl_ret = fread_len(sd_reader, buf, length);
                if fl_ret != ShaDaReadResult::kSDReadStatusSuccess {
                    xfree(buf.cast::<c_void>());
                    return fl_ret;
                }
            }

            let mut read_ptr: *const c_char = buf;
            let mut read_size: usize = length;

            if verify_but_ignore {
                let status = unpack_skip(&mut read_ptr, &mut read_size);
                let spm_ret = shada_check_status(parse_pos, status, read_size);
                if buf_allocated {
                    xfree(buf.cast::<c_void>());
                }
                if spm_ret != ShaDaReadResult::kSDReadStatusSuccess {
                    return spm_ret;
                }
                continue;
            }

            if type_u64 > SHADA_LAST_ENTRY {
                (*entry).entry_type = ShadaEntryType::kSDItemUnknown;
                (*entry).data.unknown_item.size = length;
                (*entry).data.unknown_item.item_type = type_u64;
                if initial_fpos == 0 {
                    let status = unpack_skip(&mut read_ptr, &mut read_size);
                    let spm_ret = shada_check_status(parse_pos, status, read_size);
                    if spm_ret != ShaDaReadResult::kSDReadStatusSuccess {
                        if buf_allocated {
                            xfree(buf.cast::<c_void>());
                        }
                        (*entry).entry_type = ShadaEntryType::kSDItemMissing;
                        return spm_ret;
                    }
                }
                (*entry).data.unknown_item.contents = if buf_allocated {
                    buf
                } else {
                    xmemdup(buf.cast::<c_void>(), length).cast::<c_char>()
                };
                return ShaDaReadResult::kSDReadStatusSuccess;
            }

            let entry_type = match type_u64 {
                1 => ShadaEntryType::kSDItemHeader,
                2 => ShadaEntryType::kSDItemSearchPattern,
                3 => ShadaEntryType::kSDItemSubString,
                4 => ShadaEntryType::kSDItemHistoryEntry,
                5 => ShadaEntryType::kSDItemRegister,
                6 => ShadaEntryType::kSDItemVariable,
                7 => ShadaEntryType::kSDItemGlobalMark,
                8 => ShadaEntryType::kSDItemJump,
                9 => ShadaEntryType::kSDItemBufferList,
                10 => ShadaEntryType::kSDItemLocalMark,
                11 => ShadaEntryType::kSDItemChange,
                _ => ShadaEntryType::kSDItemUnknown,
            };
            (*entry).data = sd_default_value(entry_type).data;

            let parse_ok: bool = 'parse_item: {
                match entry_type {
                    ShadaEntryType::kSDItemHeader => {}
                    ShadaEntryType::kSDItemSearchPattern => {
                        let it = &raw mut (*entry).data.search_pattern;
                        if !unpack_keydict(
                            it.cast::<c_void>(),
                            Some(shada_search_pat_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: search pattern entry at position %llu %s"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                                error_alloc,
                            );
                            (*it).pat = NvimString {
                                data: ptr::null_mut(),
                                size: 0,
                            };
                            break 'parse_item false;
                        }
                        if ((*it).is_set___shada_search_pat_
                            & (1u64 << KEYSET_OPTIDX__shada_search_pat__sp))
                            == 0
                        {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: search pattern entry at position %llu has no pattern"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        (*entry).data.search_pattern.pat =
                            copy_string((*entry).data.search_pattern.pat, ptr::null_mut());
                    }
                    ShadaEntryType::kSDItemChange
                    | ShadaEntryType::kSDItemJump
                    | ShadaEntryType::kSDItemGlobalMark
                    | ShadaEntryType::kSDItemLocalMark => {
                        let mut it = std::mem::zeroed::<KeyDict__shada_mark>();
                        if !unpack_keydict(
                            (&raw mut it).cast::<c_void>(),
                            Some(shada_mark_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu %s"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                                error_alloc,
                            );
                            break 'parse_item false;
                        }
                        if (it.is_set___shada_mark_ & (1u64 << KEYSET_OPTIDX__shada_mark__n)) != 0 {
                            if entry_type == ShadaEntryType::kSDItemJump
                                || entry_type == ShadaEntryType::kSDItemChange
                            {
                                semsg(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: mark entry at position %llu has n key which is only valid for local and global mark entries"
                                            .as_ptr(),
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            (*entry).data.filemark.name = it.n as c_char;
                        }
                        if (it.is_set___shada_mark_ & (1u64 << KEYSET_OPTIDX__shada_mark__l)) != 0 {
                            (*entry).data.filemark.mark.lnum = it.l as linenr_T;
                        }
                        if (it.is_set___shada_mark_ & (1u64 << KEYSET_OPTIDX__shada_mark__c)) != 0 {
                            (*entry).data.filemark.mark.col = it.c as colnr_T;
                        }
                        if (it.is_set___shada_mark_ & (1u64 << KEYSET_OPTIDX__shada_mark__f)) != 0 {
                            (*entry).data.filemark.fname = xmemdupz_cstr(it.f.data, it.f.size);
                            #[cfg(windows)]
                            path_to_slash((*entry).data.filemark.fname);
                        }
                        if (*entry).data.filemark.fname.is_null() {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu is missing file name"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if (*entry).data.filemark.mark.lnum <= 0 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu has invalid line number"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if (*entry).data.filemark.mark.col < 0 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu has invalid column number"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                    }
                    ShadaEntryType::kSDItemRegister => {
                        let mut it = std::mem::zeroed::<KeyDict__shada_register>();
                        if !unpack_keydict(
                            (&raw mut it).cast::<c_void>(),
                            Some(shada_register_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: register entry at position %llu %s"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                                error_alloc,
                            );
                            it.rc.destroy();
                            break 'parse_item false;
                        }
                        if it.rc.size == 0 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: register entry at position %llu has rc key with missing or empty array"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        (*entry).data.reg.contents_size = it.rc.size;
                        (*entry).data.reg.contents =
                            xmalloc(it.rc.size * std::mem::size_of::<NvimString>())
                                .cast::<NvimString>();
                        for j in 0..it.rc.size {
                            *(*entry).data.reg.contents.add(j) =
                                copy_string(*it.rc.items.add(j), ptr::null_mut());
                        }
                        it.rc.destroy();

                        if (it.is_set___shada_register_
                            & (1u64 << KEYSET_OPTIDX__shada_register__ru))
                            != 0
                        {
                            (*entry).data.reg.is_unnamed = it.ru;
                        }
                        if (it.is_set___shada_register_
                            & (1u64 << KEYSET_OPTIDX__shada_register__rt))
                            != 0
                        {
                            (*entry).data.reg.reg_type = it.rt as u8;
                        }
                        if (it.is_set___shada_register_
                            & (1u64 << KEYSET_OPTIDX__shada_register__n))
                            != 0
                        {
                            (*entry).data.reg.name = it.n as c_char;
                        }
                        if (it.is_set___shada_register_
                            & (1u64 << KEYSET_OPTIDX__shada_register__rw))
                            != 0
                        {
                            (*entry).data.reg.width = it.rw as usize;
                        }
                    }
                    ShadaEntryType::kSDItemHistoryEntry => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 2 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu is not an array with enough elements"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let mut hist_type: Integer = 0;
                        if !unpack_integer(&mut read_ptr, &mut read_size, &mut hist_type) {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history type type"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let item = unpack_string(&mut read_ptr, &mut read_size);
                        if item.data.is_null() {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history string type"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let item_slice =
                            std::slice::from_raw_parts(item.data.cast::<u8>(), item.size);
                        if item_slice.contains(&0) {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu contains string with zero byte inside"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        (*entry).data.history_item.histtype = hist_type as u8;
                        let is_hist_search =
                            ((*entry).data.history_item.histtype as c_int) == HIST_SEARCH;
                        if is_hist_search {
                            if len < 3 {
                                semsg(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: search history entry at position %llu does not have separator character"
                                            .as_ptr(),
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            let mut sep_type: Integer = 0;
                            if !unpack_integer(&mut read_ptr, &mut read_size, &mut sep_type) {
                                semsg(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: search history entry at position %llu has wrong history separator type"
                                            .as_ptr(),
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            (*entry).data.history_item.sep = sep_type as c_char;
                        }
                        let strsize = item.size + 2;
                        let s = xmalloc(strsize).cast::<c_char>();
                        ptr::copy_nonoverlapping(item.data, s, item.size);
                        *s.add(strsize - 2) = 0;
                        *s.add(strsize - 1) = (*entry).data.history_item.sep;
                        (*entry).data.history_item.string = s;
                        read_additional_array_elements =
                            (len as u32) - (2 + u32::from(is_hist_search));
                    }
                    ShadaEntryType::kSDItemVariable => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 2 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: variable entry at position %llu is not an array with enough elements"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let name = unpack_string(&mut read_ptr, &mut read_size);
                        if name.data.is_null() {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: variable entry at position %llu has wrong variable name type"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        (*entry).data.global_var.name = xmemdupz_cstr(name.data, name.size);
                        let binval = unpack_string(&mut read_ptr, &mut read_size);
                        let mut is_blob = false;
                        if !binval.data.is_null() {
                            if len > 2 {
                                let mut vtype: Integer = 0;
                                if !unpack_integer(&mut read_ptr, &mut read_size, &mut vtype)
                                    || vtype != VAR_TYPE_BLOB
                                {
                                    semsg(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: variable entry at position %llu has wrong variable type"
                                                .as_ptr(),
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                is_blob = true;
                            }
                            (*entry).data.global_var.value =
                                decode_string(binval.data, binval.size, is_blob, false);
                        } else {
                            let status = unpack_typval(
                                &mut read_ptr,
                                &mut read_size,
                                &raw mut (*entry).data.global_var.value,
                            );
                            if status != MPACK_OK {
                                semsg(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: variable entry at position %llu has value that cannot be converted to the Vimscript value"
                                            .as_ptr(),
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                        }
                        read_additional_array_elements = (len as u32) - 2 - u32::from(is_blob);
                    }
                    ShadaEntryType::kSDItemSubString => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 1 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu is not an array with enough elements"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let sub = unpack_string(&mut read_ptr, &mut read_size);
                        if sub.data.is_null() {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu has wrong sub string type"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        (*entry).data.sub_string.sub = xmemdupz_cstr(sub.data, sub.size);
                        read_additional_array_elements = (len as u32) - 1;
                    }
                    ShadaEntryType::kSDItemBufferList => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 0 {
                            semsg(
                                gettext(
                                    c"E575: Error while reading ShaDa file: buffer list entry at position %llu is not an array"
                                        .as_ptr(),
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if len > 0 {
                            let ulen = len as usize;
                            (*entry).data.buffer_list.buffers =
                                xcalloc(ulen, std::mem::size_of::<BufferListBuffer>())
                                    .cast::<BufferListBuffer>();
                            for i in 0..ulen {
                                (*entry).data.buffer_list.size += 1;
                                let mut it = std::mem::zeroed::<KeyDict__shada_buflist_item>();
                                let mut it_ad = AdditionalDataBuilder::new();
                                if !unpack_keydict(
                                    (&raw mut it).cast::<c_void>(),
                                    Some(shada_buflist_item_get_field_cb),
                                    &mut it_ad,
                                    &mut read_ptr,
                                    &mut read_size,
                                    &mut error_alloc,
                                ) {
                                    semsg(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry that %s"
                                                .as_ptr(),
                                        ),
                                        initial_fpos,
                                        error_alloc,
                                    );
                                    it_ad.destroy();
                                    break 'parse_item false;
                                }
                                let e = &mut *(*entry).data.buffer_list.buffers.add(i);
                                e.additional_data = it_ad.items.cast::<AdditionalData>();
                                e.pos = DEFAULT_POS;
                                if (it.is_set___shada_buflist_item_
                                    & (1u64 << KEYSET_OPTIDX__shada_buflist_item__l))
                                    != 0
                                {
                                    e.pos.lnum = it.l as linenr_T;
                                }
                                if (it.is_set___shada_buflist_item_
                                    & (1u64 << KEYSET_OPTIDX__shada_buflist_item__c))
                                    != 0
                                {
                                    e.pos.col = it.c as colnr_T;
                                }
                                if (it.is_set___shada_buflist_item_
                                    & (1u64 << KEYSET_OPTIDX__shada_buflist_item__f))
                                    != 0
                                {
                                    e.fname = xmemdupz_cstr(it.f.data, it.f.size);
                                    #[cfg(windows)]
                                    path_to_slash(e.fname);
                                }
                                if e.pos.lnum <= 0 {
                                    semsg(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry with invalid line number"
                                                .as_ptr(),
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                if e.pos.col < 0 {
                                    semsg(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry with invalid column number"
                                                .as_ptr(),
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                if e.fname.is_null() {
                                    semsg(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry that does not have a file name"
                                                .as_ptr(),
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                            }
                        }
                    }
                    ShadaEntryType::kSDItemMissing | ShadaEntryType::kSDItemUnknown => {
                        std::process::abort();
                    }
                }

                for _ in 0..read_additional_array_elements {
                    let item_start = read_ptr;
                    let status = unpack_skip(&mut read_ptr, &mut read_size);
                    if status != 0 {
                        break 'parse_item false;
                    }
                    push_additional_data(
                        &mut ad,
                        item_start,
                        (read_ptr as usize) - (item_start as usize),
                    );
                }

                if read_size != 0 {
                    semsg(
                        gettext(
                            c"E575: Error while reading ShaDa file: item entry at position %llu additional bytes"
                                .as_ptr(),
                        ),
                        initial_fpos,
                    );
                    break 'parse_item false;
                }

                true
            };

            if parse_ok {
                (*entry).entry_type = entry_type;
                (*entry).additional_data = ad.items.cast::<AdditionalData>();
                ret = ShaDaReadResult::kSDReadStatusSuccess;
            } else {
                (*entry).entry_type = entry_type;
                shada_free_shada_entry(entry);
                (*entry).entry_type = ShadaEntryType::kSDItemMissing;
                xfree(error_alloc.cast::<c_void>());
                ad.destroy();
            }

            if buf_allocated {
                xfree(buf.cast::<c_void>());
            }
            return ret;
        }
    }
}

unsafe fn shada_removable(name: *const c_char) -> bool {
    unsafe {
        let mut part = [0 as c_char; MAXPATHL + 1];
        let mut retval = false;
        let new_name = home_replace_save(ptr::null_mut(), name);
        let mut p = p_shada;
        while *p != 0 {
            copy_option_part(&mut p, part.as_mut_ptr(), part.len(), c", ".as_ptr());
            if part[0] == (b'r' as c_char) {
                let name_buff_ptr = (&raw mut NameBuff).cast::<c_char>();
                let n = home_replace(
                    ptr::null_mut(),
                    part.as_ptr().add(1),
                    name_buff_ptr,
                    MAXPATHL,
                    true,
                );
                if mb_strnicmp(name_buff_ptr, new_name, n) == 0 {
                    retval = true;
                    break;
                }
            }
        }
        xfree(new_name.cast::<c_void>());
        retval
    }
}

#[inline]
unsafe fn shada_init_jumps(jumps: *mut ShadaEntry, removable_bufs: *mut Set<ptr_t>) -> usize {
    unsafe {
        let mut jumps_size = 0usize;
        let mut jump_iter: *const c_void = ptr::null();
        setpcmark();
        cleanup_jumplist(curwin, false);
        loop {
            let mut fm = std::mem::zeroed::<xfmark_T>();
            jump_iter = mark_jumplist_iter(jump_iter, curwin, &mut fm);
            if fm.fmark.mark.lnum == 0 {
                siemsg(
                    c"ShaDa: mark lnum zero (ji:%p, js:%p, len:%i)".as_ptr(),
                    jump_iter,
                    (&raw mut (*curwin).w_jumplist).cast::<c_void>(),
                    (*curwin).w_jumplistlen,
                );
                if jump_iter.is_null() {
                    break;
                }
                continue;
            }
            let buf = if fm.fmark.fnum == 0 {
                ptr::null_mut()
            } else {
                buflist_findnr(fm.fmark.fnum)
            };
            let skip = if !buf.is_null() {
                ignore_buf(buf, removable_bufs)
            } else {
                fm.fmark.fnum != 0
            };
            if skip {
                if jump_iter.is_null() {
                    break;
                }
                continue;
            }
            let fname = if fm.fmark.fnum == 0 {
                fm.fname
            } else if !buf.is_null() {
                (*buf).b_ffname
            } else {
                ptr::null_mut()
            };
            if fname.is_null() {
                if jump_iter.is_null() {
                    break;
                }
                continue;
            }
            *jumps.add(jumps_size) = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::kSDItemJump,
                timestamp: fm.fmark.timestamp,
                data: ShadaEntryData {
                    filemark: ShadaFilemark {
                        name: NUL,
                        mark: fm.fmark.mark,
                        fname,
                    },
                },
                additional_data: fm.fmark.additional_data,
            };
            jumps_size += 1;
            if jump_iter.is_null() {
                break;
            }
        }
        jumps_size
    }
}

/// Gets registers as a msgpack-encoded string, in shada format (`shada_encode_regs`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_encode_regs(scale_ts: bool, since: Timestamp) -> NvimString {
    unsafe {
        let wms = xcalloc(1, std::mem::size_of::<WriteMergerState>()).cast::<WriteMergerState>();
        shada_initialize_registers(wms, -1, scale_ts);
        let mut packer = packer_string_buffer();
        for entry in (*wms).registers {
            if entry.entry_type == ShadaEntryType::kSDItemRegister
                && entry.timestamp >= since
                && shada_pack_pfreed_entry(&mut packer, entry, 0)
                    == ShaDaWriteResult::kSDWriteFailed
            {
                std::process::abort();
            }
        }
        xfree(wms.cast::<c_void>());
        packer_take_string(&mut packer)
    }
}

/// Gets the jumplist as a msgpack-encoded string, in shada format (`shada_encode_jumps`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_encode_jumps() -> NvimString {
    unsafe {
        let mut removable_bufs: Set<ptr_t> = set_init();
        find_removable_bufs(&mut removable_bufs);
        let mut jumps = [MISSING_ENTRY; JUMPLISTSIZE];
        let jumps_size = shada_init_jumps(jumps.as_mut_ptr(), &mut removable_bufs);
        let mut packer = packer_string_buffer();
        for entry in &jumps[..jumps_size] {
            if shada_pack_pfreed_entry(&mut packer, *entry, 0) == ShaDaWriteResult::kSDWriteFailed {
                std::process::abort();
            }
        }
        packer_take_string(&mut packer)
    }
}

/// Gets the buffer-list as a msgpack-encoded string, in shada format (`shada_encode_buflist`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_encode_buflist() -> NvimString {
    unsafe {
        let mut removable_bufs: Set<ptr_t> = set_init();
        find_removable_bufs(&mut removable_bufs);
        let buflist_entry = shada_get_buflist(&mut removable_bufs);

        let mut packer = packer_string_buffer();
        if shada_pack_entry(&mut packer, buflist_entry, 0) == ShaDaWriteResult::kSDWriteFailed {
            std::process::abort();
        }
        xfree(buflist_entry.data.buffer_list.buffers.cast::<c_void>());
        packer_take_string(&mut packer)
    }
}

/// Gets global Vimscript variables as a msgpack-encoded string, in shada format (`shada_encode_gvars`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_encode_gvars() -> NvimString {
    unsafe {
        let mut packer = packer_string_buffer();
        let mut var_iter: *const c_void = ptr::null();
        let cur_timestamp = os_time();
        loop {
            let mut vartv = std::mem::zeroed::<typval_T>();
            let mut name: *const c_char = ptr::null();
            var_iter = var_shada_iter(
                var_iter,
                &mut name,
                &mut vartv,
                VAR_FLAVOUR_DEFAULT | VAR_FLAVOUR_SESSION | VAR_FLAVOUR_SHADA,
            );
            if name.is_null() {
                break;
            }
            if vartv.v_type != VarType::VAR_FUNC && vartv.v_type != VarType::VAR_PARTIAL {
                let mut tgttv = std::mem::zeroed::<typval_T>();
                tv_copy(&vartv, &mut tgttv);
                let r = shada_pack_entry(
                    &mut packer,
                    ShadaEntry {
                        entry_type: ShadaEntryType::kSDItemVariable,
                        can_free_entry: false,
                        timestamp: cur_timestamp,
                        data: ShadaEntryData {
                            global_var: ShadaGlobalVar {
                                name: name.cast_mut(),
                                value: tgttv,
                            },
                        },
                        additional_data: ptr::null_mut(),
                    },
                    0,
                );
                if r == ShaDaWriteResult::kSDWriteFailed {
                    std::process::abort();
                }
                tv_clear(&mut tgttv);
            }
            tv_clear(&mut vartv);
            if var_iter.is_null() {
                break;
            }
        }
        packer_take_string(&mut packer)
    }
}

/// Read ShaDa from String (`shada_read_string`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shada_read_string(string: NvimString, flags: c_int) {
    if string.size == 0 {
        return;
    }
    unsafe {
        let mut sd_reader = std::mem::zeroed::<FileDescriptor>();
        file_open_buffer(&mut sd_reader, string.data, string.size);
        shada_read(&mut sd_reader, flags);
        close_file(&mut sd_reader);
    }
}

/// Find the parameter represented by the given character in 'shada' (`get_shada_parameter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_shada_parameter(param_type: c_int) -> c_int {
    unsafe {
        let p = find_shada_parameter(param_type);
        if !p.is_null() && (*p as u8).is_ascii_digit() {
            let mut val: c_int = 0;
            let mut cur = p;
            while (*cur as u8).is_ascii_digit() {
                val = val
                    .wrapping_mul(10)
                    .wrapping_add(c_int::from((*cur as u8) - b'0'));
                cur = cur.add(1);
            }
            return val;
        }
        -1
    }
}

/// Find the parameter represented by the given character in 'shada' (`find_shada_parameter`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_shada_parameter(param_type: c_int) -> *mut c_char {
    unsafe {
        let mut p = p_shada;
        while *p != 0 {
            if c_int::from(*p as u8) == param_type {
                return p.add(1);
            }
            if *p == (b'n' as c_char) {
                break;
            }
            p = vim_strchr(p, b',' as c_int);
            if p.is_null() {
                break;
            }
            p = p.add(1);
        }
        ptr::null_mut()
    }
}

/// Read marks for the current buffer from the ShaDa file (`check_marks_read`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_marks_read() {
    unsafe {
        if !(*curbuf).b_marks_read
            && get_shada_parameter(b'\'' as c_int) > 0
            && !(*curbuf).b_ffname.is_null()
        {
            shada_read_marks();
        }
        (*curbuf).b_marks_read = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_indices_and_hist_type2char_map_correctly() {
        assert_eq!(mark_global_index(b'A' as c_char), 0);
        assert_eq!(mark_global_index(b'Z' as c_char), 25);
        assert_eq!(mark_global_index(b'0' as c_char), 26);
        assert_eq!(mark_global_index(b'9' as c_char), 35);
        assert_eq!(mark_global_index(b'a' as c_char), -1);

        assert_eq!(mark_local_index(b'a' as c_char), 0);
        assert_eq!(mark_local_index(b'z' as c_char), 25);
        assert_eq!(mark_local_index(b'"' as c_char), 26);
        assert_eq!(mark_local_index(b'^' as c_char), 27);
        assert_eq!(mark_local_index(b'.' as c_char), 28);
        assert_eq!(mark_local_index(b'A' as c_char), -1);

        assert_eq!(hist_type2char(HIST_CMD), b':' as c_int);
        assert_eq!(hist_type2char(HIST_SEARCH), b'/' as c_int);
        assert_eq!(hist_type2char(HIST_EXPR), b'=' as c_int);
        assert_eq!(hist_type2char(HIST_INPUT), b'@' as c_int);
        assert_eq!(hist_type2char(HIST_DEBUG), b'>' as c_int);
    }
}
