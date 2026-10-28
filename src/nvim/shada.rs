//! ShaDa (Shared Data) file reader and writer (`src/nvim/shada.c`).

use crate::api::private::helpers::{
    api_free_dict, api_free_string, copy_string, cstr_as_string, static_cstr_as_string,
};
use crate::api::private::validate::{
    Dict, KeyValuePair, NvimString, Object, ObjectData, ObjectType,
};
use crate::buffer::{
    BLN_LISTED, FileBuf, JUMPLISTSIZE, NMARKS, bt_quickfix, bt_terminal, buflist_findnr,
    buflist_new, buflist_setfpos,
};
use crate::cmdhist::{
    HIST_CMD, HIST_COUNT, HIST_DEBUG, HIST_EXPR, HIST_INPUT, HIST_SEARCH, Histentry, clr_history,
    hist_get_array, hist_iter,
};
use crate::context::INIT_FMARKV;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::eval::decode::{decode_string, unpack_typval};
use crate::eval::encode::encode_vim_to_msgpack;
use crate::eval::typval::{
    TypVal, TypvalVval, VarType, tv_clear, tv_copy, tv_dict_hi2di, tv_list_alloc, tv_list_len,
};
use crate::eval::vars::{
    VAR_TYPE_BLOB, VV_OLDFILES, get_globvar_ht, get_vim_var_list, set_vim_var_list,
};
use crate::eval::{
    K_LIST_LEN_UNKNOWN_CONST, VAR_FLAVOUR_DEFAULT, VAR_FLAVOUR_SESSION, VAR_FLAVOUR_SHADA,
    VarFlavour, get_copy_id, set_ref_in_ht, set_ref_in_list_items, var_flavour, var_set_global,
};
use crate::ex_cmds::{sub_get_replacement, sub_set_replacement};
use crate::ex_docmd::set_no_hlsearch;
use crate::fileio::{S_IFDIR, S_IFMT, modname, vim_rename};
#[cfg(unix)]
use crate::fileio::{getgid, getuid};
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::highlight::OptionalKeys;
use crate::lua::converter::Boolean;
use crate::mark::{
    EXTRA_MARKS, Fmark, Xfmark, cleanup_jumplist, free_xfmark, mark_buffer_iter, mark_get,
    mark_global_index, mark_global_iter, mark_jumplist_iter, mark_set_global, mark_set_local,
    set_last_cursor, setpcmark,
};
use crate::mbyte::mb_strnicmp;
use crate::memory::ARENA_BLOCK_SIZE;
use crate::message::{semsg, siemsg, smsg, verbose_enter, verbose_leave};
use crate::mpack::mpack_core::MPACK_OK;
use crate::msgpack_rpc::packer::{
    Integer, MPACK_ITEM_SIZE, PackerBuffer, mpack_array, mpack_bin, mpack_bool, mpack_check_buffer,
    mpack_integer, mpack_map, mpack_raw, mpack_str, mpack_uint, mpack_uint64, packer_string_buffer,
    packer_take_string,
};
use crate::msgpack_rpc::unpacker::{
    AdditionalDataBuilder, StringArray, additional_data_take, push_additional_data, unpack_array,
    unpack_integer, unpack_keydict, unpack_skip, unpack_string,
};
#[cfg(unix)]
use crate::option::ROOT_UID;
use crate::option::copy_option_part;
use crate::option_vars::*;
use crate::os::env::{expand_env, home_replace, home_replace_save, os_get_pid};
use crate::os::fileio::{
    FileDescriptor, K_FILE_CREATE, K_FILE_CREATE_ONLY, K_FILE_NO_SYMLINK, K_FILE_READ_ONLY,
    K_FILE_TRUNCATE, file_close, file_flush, file_open, file_open_buffer, file_read, file_skip,
    file_try_read_buffered,
};
use crate::os::fs::{
    CASE_INSENSITIVE_FILENAME, FileInfo, UV_ENOENT, os_fileinfo, os_getperm, os_isdir,
    os_mkdir_recurse, os_remove,
};
use crate::os::stdpaths::stdpaths_user_state_subpath;
use crate::os::time::{Timestamp, os_time};
use crate::path::{
    K_PATH_CMP_LITERAL, concat_fnames_cstr, path_cmp, path_equal, path_fold_char,
    path_tail_with_sep, path_try_shorten_fname, vim_ispathsep, vim_ispathsep_nocolon,
};
use crate::regexp::{K_MARK_BUF_LOCAL, regtilde};
use crate::register::{
    MotionType, NUM_SAVED_REGISTERS, Yankreg, op_global_reg_iter, op_reg_get, op_reg_index,
    op_reg_set, reg_empty,
};
use crate::search::{
    AdditionalData, SearchOffset, SearchPattern, get_search_pattern, get_search_pattern_ptr,
    get_search_pattern_timestamp, get_substitute_pattern, search_pattern_cleared,
    search_was_last_used, set_last_used_pattern, set_search_pattern, set_substitute_pattern,
};
use crate::strings::{gettext, vim_strchr};
use std::cmp::Ordering;
use std::ffi::{CStr, c_char, c_int, c_uint};
use std::ptr;

#[cfg(unix)]
use crate::os::fs::os_fchown;
#[cfg(windows)]
use crate::path::path_to_slash;

pub const K_SHA_DA_WANT_INFO: c_int = 1;
pub const K_SHA_DA_WANT_MARKS: c_int = 2;
pub const K_SHA_DA_FORCEIT: c_int = 4;
pub const K_SHA_DA_GET_OLDFILES: c_int = 8;
pub const K_SHA_DA_MISSING_ERROR: c_int = 16;
pub const K_SHA_DA_NANOS: c_int = 32;
pub const K_SHA_DA_NO_HISTORY: c_int = 64;
pub const K_SHA_DA_NO_OPT: c_int = 128;

const NLOCALMARKS: usize = NMARKS + 3;
const SHADA_MPACK_FREE_SPACE: usize = 4 * MPACK_ITEM_SIZE;

#[cfg(windows)]
const UV_EEXIST: c_int = -4075;
#[cfg(windows)]
const UV_ELOOP: c_int = -4067;
#[cfg(not(windows))]
const UV_EEXIST: c_int = -17;
#[cfg(target_os = "linux")]
const UV_ELOOP: c_int = -40;
#[cfg(all(not(windows), not(target_os = "linux")))]
const UV_ELOOP: c_int = -62;

const DEFAULT_POS: CursorPos = CursorPos {
    lnum: 1,
    col: 0,
    coladd: 0,
};

const KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_C: u32 = 1;
const KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_F: u32 = 2;
const KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_L: u32 = 3;

const KEYSET_OPTIDX_SHADA_MARK_C: u32 = 1;
const KEYSET_OPTIDX_SHADA_MARK_F: u32 = 2;
const KEYSET_OPTIDX_SHADA_MARK_L: u32 = 3;
const KEYSET_OPTIDX_SHADA_MARK_N: u32 = 4;

const KEYSET_OPTIDX_SHADA_REGISTER_N: u32 = 1;
const KEYSET_OPTIDX_SHADA_REGISTER_RT: u32 = 3;
const KEYSET_OPTIDX_SHADA_REGISTER_RU: u32 = 4;
const KEYSET_OPTIDX_SHADA_REGISTER_RW: u32 = 5;

const KEYSET_OPTIDX_SHADA_SEARCH_PAT_SP: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict__shada_buflist_item {
    pub is_set_shada_buflist_item: OptionalKeys,
    pub l: Integer,
    pub c: Integer,
    pub f: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict__shada_mark {
    pub is_set_shada_mark: OptionalKeys,
    pub n: Integer,
    pub l: Integer,
    pub c: Integer,
    pub f: NvimString,
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct KeyDict__shada_register {
    pub is_set_shada_register: OptionalKeys,
    pub rc: StringArray,
    pub ru: Boolean,
    pub rt: Integer,
    pub n: Integer,
    pub rw: Integer,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict__shada_search_pat {
    pub is_set_shada_search_pat: OptionalKeys,
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

#[derive(Clone)]
pub struct SubReplacementString {
    pub sub: Option<std::ffi::CString>,
    pub timestamp: Timestamp,
    pub additional_data: Option<AdditionalData>,
}

impl SubReplacementString {
    pub const fn new() -> Self {
        Self {
            sub: None,
            timestamp: 0,
            additional_data: None,
        }
    }
}

impl Default for SubReplacementString {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShadaEntryType {
    SDItemUnknown = -1,
    SDItemMissing = 0,
    SDItemHeader = 1,
    SDItemSearchPattern = 2,
    SDItemSubString = 3,
    SDItemHistoryEntry = 4,
    SDItemRegister = 5,
    SDItemVariable = 6,
    SDItemGlobalMark = 7,
    SDItemJump = 8,
    SDItemBufferList = 9,
    SDItemLocalMark = 10,
    SDItemChange = 11,
}

const SHADA_LAST_ENTRY: u64 = ShadaEntryType::SDItemChange as i32 as u64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShaDaReadResult {
    SDReadStatusSuccess,
    SDReadStatusFinished,
    SDReadStatusReadError,
    SDReadStatusNotShaDa,
    SDReadStatusMalformed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShaDaWriteResult {
    SDWriteSuccessful,
    SDWriteReadNotShada,
    SDWriteFailed,
    SDWriteIgnError,
}

const K_SDREAD_UNDISABLEABLE_DATA: c_uint = (1 << (ShadaEntryType::SDItemSearchPattern as u32))
    | (1 << (ShadaEntryType::SDItemSubString as u32))
    | (1 << (ShadaEntryType::SDItemJump as u32));
const K_SDREAD_REGISTERS: c_uint = 1 << (ShadaEntryType::SDItemRegister as u32);
const K_SDREAD_HISTORY: c_uint = 1 << (ShadaEntryType::SDItemHistoryEntry as u32);
const K_SDREAD_VARIABLES: c_uint = 1 << (ShadaEntryType::SDItemVariable as u32);
const K_SDREAD_BUFFER_LIST: c_uint = 1 << (ShadaEntryType::SDItemBufferList as u32);
const K_SDREAD_UNKNOWN: c_uint = 1 << ((SHADA_LAST_ENTRY as u32) + 1);
const K_SDREAD_GLOBAL_MARKS: c_uint = 1 << (ShadaEntryType::SDItemGlobalMark as u32);
const K_SDREAD_LOCAL_MARKS: c_uint = 1 << (ShadaEntryType::SDItemLocalMark as u32);
const K_SDREAD_CHANGES: c_uint = 1 << (ShadaEntryType::SDItemChange as u32);

#[derive(Clone)]
struct ShadaFilemark {
    name: c_char,
    mark: CursorPos,
    fname: Option<std::ffi::CString>,
}

#[derive(Clone)]
struct ShadaHistoryItem {
    histtype: u8,
    string: Option<std::ffi::CString>,
    sep: c_char,
}

#[repr(C)]
#[derive(Clone)]
struct ShadaReg {
    name: c_char,
    reg_type: u8,
    contents: Vec<NvimString>,
    is_unnamed: bool,
    width: usize,
}

#[derive(Clone)]
struct ShadaGlobalVar {
    name: Option<std::ffi::CString>,
    value: TypVal,
}

#[derive(Clone)]
struct ShadaUnknownItem {
    item_type: u64,
    contents: Vec<c_char>,
}

#[derive(Clone)]
struct ShadaSubString {
    sub: Option<std::ffi::CString>,
}

#[derive(Clone)]
struct BufferListBuffer {
    pos: CursorPos,
    fname: Option<std::ffi::CString>,
    additional_data: Option<AdditionalData>,
}

#[repr(C)]
#[derive(Clone)]
struct ShadaBufferList {
    buffers: Vec<BufferListBuffer>,
}

#[derive(Clone)]
enum ShadaEntryData {
    Missing,
    Header(Dict),
    Filemark(ShadaFilemark),
    SearchPattern(KeyDict__shada_search_pat),
    HistoryItem(ShadaHistoryItem),
    Reg(ShadaReg),
    GlobalVar(ShadaGlobalVar),
    UnknownItem(ShadaUnknownItem),
    SubString(ShadaSubString),
    BufferList(ShadaBufferList),
}

impl ShadaEntryData {
    #[inline]
    const fn header(&self) -> Dict {
        match self {
            Self::Header(v) => *v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn filemark(&self) -> &ShadaFilemark {
        match self {
            Self::Filemark(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn filemark_mut(&mut self) -> &mut ShadaFilemark {
        match self {
            Self::Filemark(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn search_pattern(&self) -> KeyDict__shada_search_pat {
        match self {
            Self::SearchPattern(v) => *v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn search_pattern_mut(&mut self) -> &mut KeyDict__shada_search_pat {
        match self {
            Self::SearchPattern(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn history_item(&self) -> &ShadaHistoryItem {
        match self {
            Self::HistoryItem(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn history_item_mut(&mut self) -> &mut ShadaHistoryItem {
        match self {
            Self::HistoryItem(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn reg(&self) -> &ShadaReg {
        match self {
            Self::Reg(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn reg_mut(&mut self) -> &mut ShadaReg {
        match self {
            Self::Reg(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn global_var(&self) -> &ShadaGlobalVar {
        match self {
            Self::GlobalVar(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn global_var_mut(&mut self) -> &mut ShadaGlobalVar {
        match self {
            Self::GlobalVar(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn unknown_item(&self) -> &ShadaUnknownItem {
        match self {
            Self::UnknownItem(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn sub_string(&self) -> &ShadaSubString {
        match self {
            Self::SubString(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn sub_string_mut(&mut self) -> &mut ShadaSubString {
        match self {
            Self::SubString(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    const fn buffer_list(&self) -> &ShadaBufferList {
        match self {
            Self::BufferList(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn buffer_list_mut(&mut self) -> &mut ShadaBufferList {
        match self {
            Self::BufferList(v) => v,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone)]
struct ShadaEntry {
    entry_type: ShadaEntryType,
    can_free_entry: bool,
    timestamp: Timestamp,
    data: ShadaEntryData,
    additional_data: Option<AdditionalData>,
}

#[repr(C)]
#[derive(Clone)]
struct HMLListEntry {
    data: ShadaEntry,
    next: Option<usize>,
    prev: Option<usize>,
}

struct HMLList {
    entries: Vec<HMLListEntry>,
    first: Option<usize>,
    last: Option<usize>,
    free_entry: Option<usize>,
    last_free_entry: usize,
    size: usize,
    num_entries: usize,
    contained_entries: std::collections::HashMap<Vec<u8>, usize>,
}

#[repr(C)]
struct HistoryMergerState {
    hmll: HMLList,
    do_merge: bool,
    reading: bool,
    iter: Option<usize>,
    last_hist_entry: ShadaEntry,
    history_type: u8,
}

#[repr(C)]
struct FileMarks {
    marks: [ShadaEntry; NLOCALMARKS],
    changes: [ShadaEntry; JUMPLISTSIZE],
    changes_size: usize,
    additional_marks: Vec<ShadaEntry>,
    greatest_timestamp: Timestamp,
}

impl FileMarks {
    fn new() -> Self {
        Self {
            marks: [const { MISSING_ENTRY }; NLOCALMARKS],
            changes: [const { MISSING_ENTRY }; JUMPLISTSIZE],
            changes_size: 0,
            additional_marks: Vec::new(),
            greatest_timestamp: 0,
        }
    }
}

impl Drop for FileMarks {
    fn drop(&mut self) {
        for mut add_entry in self.additional_marks.drain(..) {
            shada_free_shada_entry(&mut add_entry);
        }
    }
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
    dumped_variables: std::collections::HashSet<Vec<u8>>,
    file_marks: std::collections::HashMap<Vec<u8>, FileMarks>,
}

impl WriteMergerState {
    fn new() -> Box<Self> {
        Box::new(Self {
            hms: std::array::from_fn(|_| HistoryMergerState {
                hmll: HMLList {
                    entries: Vec::new(),
                    first: None,
                    last: None,
                    free_entry: None,
                    last_free_entry: 0,
                    size: 0,
                    num_entries: 0,
                    contained_entries: std::collections::HashMap::new(),
                },
                do_merge: false,
                reading: false,
                iter: None,
                last_hist_entry: MISSING_ENTRY,
                history_type: 0,
            }),
            global_marks: [const { MISSING_ENTRY }; NMARKS],
            numbered_marks: [const { MISSING_ENTRY }; EXTRA_MARKS],
            registers: [const { MISSING_ENTRY }; NUM_SAVED_REGISTERS],
            jumps: [const { MISSING_ENTRY }; JUMPLISTSIZE],
            jumps_size: 0,
            search_pattern: MISSING_ENTRY,
            sub_search_pattern: MISSING_ENTRY,
            replacement: MISSING_ENTRY,
            dumped_variables: std::collections::HashSet::new(),
            file_marks: std::collections::HashMap::new(),
        })
    }
}

const MISSING_ENTRY: ShadaEntry = ShadaEntry {
    entry_type: ShadaEntryType::SDItemMissing,
    can_free_entry: false,
    timestamp: 0,
    data: ShadaEntryData::Missing,
    additional_data: None,
};

const DEFAULT_SEARCH_PAT: KeyDict__shada_search_pat = KeyDict__shada_search_pat {
    is_set_shada_search_pat: 0,
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
        ShadaEntryType::SDItemUnknown => ShadaEntry {
            entry_type: ShadaEntryType::SDItemUnknown,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::UnknownItem(ShadaUnknownItem {
                item_type: 0,
                contents: Vec::new(),
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemMissing => MISSING_ENTRY,
        ShadaEntryType::SDItemHeader => ShadaEntry {
            entry_type: ShadaEntryType::SDItemHeader,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Header(Dict {
                items: ptr::null_mut(),
                size: 0,
                capacity: 0,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemSearchPattern => ShadaEntry {
            entry_type: ShadaEntryType::SDItemSearchPattern,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::SearchPattern(DEFAULT_SEARCH_PAT),
            additional_data: None,
        },
        ShadaEntryType::SDItemSubString => ShadaEntry {
            entry_type: ShadaEntryType::SDItemSubString,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::SubString(ShadaSubString { sub: None }),
            additional_data: None,
        },
        ShadaEntryType::SDItemHistoryEntry => ShadaEntry {
            entry_type: ShadaEntryType::SDItemHistoryEntry,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::HistoryItem(ShadaHistoryItem {
                histtype: HIST_CMD as u8,
                string: None,
                sep: NUL,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemRegister => ShadaEntry {
            entry_type: ShadaEntryType::SDItemRegister,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Reg(ShadaReg {
                name: NUL,
                reg_type: MotionType::MtCharWise as u8,
                contents: Vec::new(),
                is_unnamed: false,
                width: 0,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemVariable => ShadaEntry {
            entry_type: ShadaEntryType::SDItemVariable,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::GlobalVar(ShadaGlobalVar {
                name: None,
                value: TypVal {
                    v_type: VarType::VarUnknown,
                    v_lock: 0,
                    vval: TypvalVval::String(ptr::null_mut()),
                },
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemGlobalMark => ShadaEntry {
            entry_type: ShadaEntryType::SDItemGlobalMark,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Filemark(ShadaFilemark {
                name: b'"' as c_char,
                mark: DEFAULT_POS,
                fname: None,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemJump => ShadaEntry {
            entry_type: ShadaEntryType::SDItemJump,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Filemark(ShadaFilemark {
                name: NUL,
                mark: DEFAULT_POS,
                fname: None,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemBufferList => ShadaEntry {
            entry_type: ShadaEntryType::SDItemBufferList,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::BufferList(ShadaBufferList {
                buffers: Vec::new(),
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemLocalMark => ShadaEntry {
            entry_type: ShadaEntryType::SDItemLocalMark,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Filemark(ShadaFilemark {
                name: b'"' as c_char,
                mark: DEFAULT_POS,
                fname: None,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemChange => ShadaEntry {
            entry_type: ShadaEntryType::SDItemChange,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Filemark(ShadaFilemark {
                name: NUL,
                mark: DEFAULT_POS,
                fname: None,
            }),
            additional_data: None,
        },
    }
}

struct ShadaModuleState {
    default_shada_file: Option<std::ffi::CString>,
}

impl ShadaModuleState {
    const fn new() -> Self {
        Self {
            default_shada_file: None,
        }
    }
}

static SHADA_STATE: crate::globals::GlobalState<ShadaModuleState> =
    crate::globals::GlobalState::new(ShadaModuleState::new());

#[inline]
fn shada_st() -> &'static mut ShadaModuleState {
    unsafe { &mut *SHADA_STATE.as_ptr() }
}

use crate::api::private::dispatch::{
    key_dict_shada_buflist_item_get_field as shada_buflist_item_get_field_cb,
    key_dict_shada_mark_get_field as shada_mark_get_field_cb,
    key_dict_shada_register_get_field as shada_register_get_field_cb,
    key_dict_shada_search_pat_get_field as shada_search_pat_get_field_cb,
};

use crate::charset::NUL;
use crate::eval::funcs::MPACK_EOF;
use crate::event::r#loop::{NS_PER_SEC, uv_strerror};
use crate::os::fs::MAXPATHL;
use crate::version::long_version;

#[inline]
fn file_eof(fp: &FileDescriptor) -> bool {
    fp.eof && fp.read_pos == fp.write_pos
}

#[inline]
#[cfg(unix)]
fn file_fd(fp: &FileDescriptor) -> c_int {
    fp.fd
}

use crate::os::fileio::file_space;

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

#[derive(Clone, Copy)]
struct PathKey(*const c_char);

impl PartialEq for PathKey {
    fn eq(&self, other: &Self) -> bool {
        if ptr::eq(self.0, other.0) {
            return true;
        }
        if self.0.is_null() || other.0.is_null() {
            return false;
        }
        path_cmp(CASE_INSENSITIVE_FILENAME, self.0, other.0, MAXPATHL) == 0
    }
}

impl Eq for PathKey {}

impl std::hash::Hash for PathKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut p = self.0;
        let mut h: u32 = 0;
        if cfg!(target_os = "windows") {
            let c0 = { crate::memory::cstr_first_byte(p) };
            if c0.is_ascii_alphabetic() && { crate::memory::cstr_byte_at(p, 1) } == b':' {
                p = unsafe { p.add(2) };
            }
        }
        let ic = CASE_INSENSITIVE_FILENAME;
        let start = p;
        let mut len: c_int = 0;
        while unsafe { *p != 0 } {
            let cur = unsafe { *p };
            let next = unsafe { *p.add(1) };
            if vim_ispathsep_nocolon(c_int::from(cur as u8))
                && next == 0
                && !ptr::eq(start, p)
                && unsafe { !vim_ispathsep(c_int::from(*p.sub(1) as u8)) }
            {
                break;
            }
            let c = path_fold_char(ic, p, &mut len);
            h = (h << 5).wrapping_sub(h).wrapping_add(c as u32);
            p = unsafe { p.add(len as usize) };
        }
        state.write_u32(h);
    }
}

#[inline]
fn hmll_init(hmll: &mut HMLList, size: usize) {
    unsafe {
        ptr::write(
            hmll,
            HMLList {
                entries: vec![
                    HMLListEntry {
                        data: MISSING_ENTRY,
                        next: None,
                        prev: None,
                    };
                    size
                ],
                first: None,
                last: None,
                free_entry: None,
                last_free_entry: 0,
                size,
                num_entries: 0,
                contained_entries: std::collections::HashMap::new(),
            },
        );
    }
}

#[inline]
fn hmll_remove(hmll: &mut HMLList, entry_idx: usize) {
    if entry_idx + 1 == hmll.last_free_entry {
        hmll.last_free_entry -= 1;
    } else {
        assert!(hmll.free_entry.is_none());
        hmll.free_entry = Some(entry_idx);
    }
    let next_idx = hmll.entries[entry_idx].next;
    let prev_idx = hmll.entries[entry_idx].prev;
    let key = hmll.entries[entry_idx]
        .data
        .data
        .history_item()
        .string
        .as_ref()
        .map_or(&b""[..], |s| s.to_bytes());
    let val = hmll.contained_entries.remove(key);
    assert!(val.is_some());
    if let Some(n) = next_idx {
        hmll.entries[n].prev = prev_idx;
    } else {
        hmll.last = prev_idx;
    }
    if let Some(p) = prev_idx {
        hmll.entries[p].next = next_idx;
    } else {
        hmll.first = next_idx;
    }
    hmll.num_entries -= 1;
    shada_free_shada_entry(&mut hmll.entries[entry_idx].data);
}

#[inline]
fn hmll_insert(hmll: &mut HMLList, mut after_idx: Option<usize>, data: ShadaEntry) {
    if hmll.num_entries == hmll.size {
        if after_idx == hmll.first {
            after_idx = None;
        }
        let first_idx = hmll.first.expect("hmll.first must be Some when full");
        hmll_remove(hmll, first_idx);
    }
    let target_idx = if let Some(free_idx) = hmll.free_entry.take() {
        assert!(hmll.last_free_entry - 1 == hmll.num_entries);
        free_idx
    } else {
        assert!(hmll.last_free_entry == hmll.num_entries);
        let idx = hmll.last_free_entry;
        hmll.last_free_entry += 1;
        idx
    };
    let key = data
        .data
        .history_item()
        .string
        .as_ref()
        .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
    hmll.entries[target_idx].data = data;
    hmll.contained_entries.entry(key).or_insert(target_idx);
    hmll.num_entries += 1;
    hmll.entries[target_idx].prev = after_idx;
    let next_idx = if let Some(after) = after_idx {
        let next = hmll.entries[after].next;
        hmll.entries[target_idx].next = next;
        hmll.entries[after].next = Some(target_idx);
        next
    } else {
        let next = hmll.first;
        hmll.entries[target_idx].next = next;
        hmll.first = Some(target_idx);
        next
    };
    if let Some(n) = next_idx {
        hmll.entries[n].prev = Some(target_idx);
    } else {
        hmll.last = Some(target_idx);
    }
}

#[inline]
fn hmll_dealloc(hmll: &mut HMLList) {
    hmll.contained_entries = std::collections::HashMap::new();
    hmll.entries = Vec::new();
}

fn sd_reader_skip(sd_reader: &mut FileDescriptor, offset: usize) -> ShaDaReadResult {
    unsafe {
        let skip_bytes = file_skip((&raw mut *sd_reader), offset);
        if skip_bytes < 0 {
            semsg!(
                gettext(c"E886: System error while skipping in ShaDa file: %s"),
                uv_strerror(skip_bytes as c_int),
            );
            return ShaDaReadResult::SDReadStatusReadError;
        } else if skip_bytes != (offset as isize) {
            assert!(skip_bytes < (offset as isize));
            if file_eof(sd_reader) {
                semsg!(
                    gettext(
                        c"E576: Reading ShaDa file: last entry specified that it occupies %llu bytes, but file ended earlier",
                    ),
                    offset as u64,
                );
            } else {
                semsg!(
                    gettext(c"E886: System error while skipping in ShaDa file: %s"),
                    gettext(c"too few bytes read"),
                );
            }
            return ShaDaReadResult::SDReadStatusNotShaDa;
        }
        ShaDaReadResult::SDReadStatusSuccess
    }
}

fn close_file(cookie: &mut FileDescriptor) {
    unsafe {
        let error = file_close(cookie, opt_vars().p_fs != 0);
        if error != 0 {
            semsg!(
                gettext(c"E886: System error while closing ShaDa file: %s"),
                uv_strerror(error),
            );
        }
    }
}

fn shada_read_file(file: impl crate::strings::IntoCCharPtr, flags: c_int) -> bool {
    let file = file.into_c_char_ptr();
    unsafe {
        let Some(fname_cstr) = shada_filename(file) else {
            return false;
        };
        let fname = fname_cstr.as_ptr();

        let mut sd_reader = FileDescriptor::new();
        let of_ret = file_open(&mut sd_reader, fname, K_FILE_READ_ONLY, 0);

        if opt_vars().p_verbose > 1 {
            verbose_enter();
            let empty = c"".as_ptr();
            smsg!(
                0,
                gettext(c"Reading ShaDa file \"%s\"%s%s%s%s"),
                fname,
                if (flags & K_SHA_DA_WANT_INFO) != 0 {
                    gettext(c" info").cast_const()
                } else {
                    empty
                },
                if (flags & K_SHA_DA_WANT_MARKS) != 0 {
                    gettext(c" marks").cast_const()
                } else {
                    empty
                },
                if (flags & K_SHA_DA_GET_OLDFILES) != 0 {
                    gettext(c" oldfiles").cast_const()
                } else {
                    empty
                },
                if of_ret != 0 {
                    gettext(c" FAILED").cast_const()
                } else {
                    empty
                },
            );
            verbose_leave();
        }

        if of_ret != 0 {
            if of_ret != UV_ENOENT || (flags & K_SHA_DA_MISSING_ERROR) != 0 {
                semsg!(
                    gettext(c"E886: System error while opening ShaDa file %s for reading: %s",),
                    fname,
                    uv_strerror(of_ret),
                );
            }
            return false;
        }
        drop(fname_cstr);

        shada_read(&mut sd_reader, flags);
        close_file(&mut sd_reader);

        true
    }
}

fn shada_hist_iter(
    iter: Option<usize>,
    history_type: u8,
    zero: bool,
    hist: &mut ShadaEntry,
) -> Option<usize> {
    let mut hist_he = Histentry::new();
    let ret = hist_iter(iter, history_type, zero, &mut hist_he);
    if hist_he.hisstr.is_none() {
        *hist = MISSING_ENTRY;
    } else {
        let sep = if (history_type as c_int) == HIST_SEARCH {
            hist_he.sep
        } else {
            0
        };
        *hist = ShadaEntry {
            can_free_entry: zero,
            entry_type: ShadaEntryType::SDItemHistoryEntry,
            timestamp: hist_he.timestamp,
            data: ShadaEntryData::HistoryItem(ShadaHistoryItem {
                histtype: history_type,
                string: hist_he.hisstr,
                sep,
            }),
            additional_data: hist_he.additional_data,
        };
    }
    ret
}

fn hms_insert(hms_p: &mut HistoryMergerState, entry: ShadaEntry, do_iter: bool) {
    if do_iter {
        while hms_p.last_hist_entry.entry_type != ShadaEntryType::SDItemMissing
            && hms_p.last_hist_entry.timestamp < entry.timestamp
        {
            let last_entry = std::mem::replace(&mut hms_p.last_hist_entry, MISSING_ENTRY);
            hms_insert(hms_p, last_entry, false);
            if hms_p.iter.is_none() {
                hms_p.last_hist_entry.entry_type = ShadaEntryType::SDItemMissing;
                break;
            }
            hms_p.iter = shada_hist_iter(
                hms_p.iter,
                hms_p.history_type,
                hms_p.reading,
                &mut hms_p.last_hist_entry,
            );
        }
    }
    let hmll = &mut hms_p.hmll;
    let key = entry
        .data
        .history_item()
        .string
        .as_ref()
        .map_or(&b""[..], |s| s.to_bytes());
    if let Some(&existing_idx) = hmll.contained_entries.get(key) {
        let existing_ts = hmll.entries[existing_idx].data.timestamp;
        if entry.timestamp > existing_ts {
            hmll_remove(hmll, existing_idx);
        } else if !do_iter && entry.timestamp == existing_ts {
            shada_free_shada_entry(&mut hmll.entries[existing_idx].data);
            hmll.entries[existing_idx].data = entry;
            return;
        } else {
            return;
        }
    }
    let mut insert_after = hmll.last;
    while let Some(idx) = insert_after {
        if hmll.entries[idx].data.timestamp <= entry.timestamp {
            break;
        }
        insert_after = hmll.entries[idx].prev;
    }
    hmll_insert(hmll, insert_after, entry);
}

#[inline]
fn hms_init(
    hms_p: &mut HistoryMergerState,
    history_type: u8,
    num_elements: usize,
    do_merge: bool,
    reading: bool,
) {
    hmll_init(&mut hms_p.hmll, num_elements);
    hms_p.do_merge = do_merge;
    hms_p.reading = reading;
    hms_p.iter = shada_hist_iter(
        None,
        history_type,
        hms_p.reading,
        &mut hms_p.last_hist_entry,
    );
    hms_p.history_type = history_type;
}

#[inline]
fn hms_insert_whole_neovim_history(hms_p: &mut HistoryMergerState) {
    while hms_p.last_hist_entry.entry_type != ShadaEntryType::SDItemMissing {
        let last_entry = std::mem::replace(&mut hms_p.last_hist_entry, MISSING_ENTRY);
        hms_insert(hms_p, last_entry, false);
        if hms_p.iter.is_none() {
            break;
        }
        hms_p.iter = shada_hist_iter(
            hms_p.iter,
            hms_p.history_type,
            hms_p.reading,
            &mut hms_p.last_hist_entry,
        );
    }
}

#[inline]
fn hms_to_he_array(
    hms_p: &HistoryMergerState,
    hist_array: &mut [Histentry],
    new_hisidx: &mut c_int,
    new_hisnum: &mut c_int,
) {
    let mut count = 0usize;
    let hmll = &hms_p.hmll;
    let mut cur_idx = hmll.first;
    while let Some(idx) = cur_idx {
        let cur_entry = &hmll.entries[idx];
        let hi = cur_entry.data.data.history_item();
        hist_array[count] = Histentry {
            timestamp: cur_entry.data.timestamp,
            hisnum: (count as c_int) + 1,
            hisstr: hi.string.clone(),
            sep: hi.sep,
            additional_data: cur_entry.data.additional_data.clone(),
        };
        count += 1;
        cur_idx = cur_entry.next;
    }
    *new_hisnum = count as c_int;
    *new_hisidx = *new_hisnum - 1;
}

#[inline]
fn hms_dealloc(hms_p: &mut HistoryMergerState) {
    hmll_dealloc(&mut hms_p.hmll);
}

fn var_shada_iter(
    iter: Option<usize>,
    name: &mut *const c_char,
    rettv: &mut TypVal,
    flavour: VarFlavour,
) -> Option<usize> {
    unsafe {
        let globvarht = get_globvar_ht();
        let hinum = globvarht.ht_mask + 1;
        let harray = std::slice::from_raw_parts(globvarht.array_ptr(), hinum);
        *name = ptr::null();
        let mut idx = if let Some(i) = iter {
            i
        } else {
            let mut i = 0;
            while i < hinum
                && (hashitem_empty(&harray[i]) || (var_flavour(harray[i].key_ptr()) & flavour) == 0)
            {
                i += 1;
            }
            if i == hinum {
                return None;
            }
            i
        };
        let di = tv_dict_hi2di(&raw const harray[idx]);
        *name = di.di_key.as_ptr();
        tv_copy(&di.di_tv, rettv);
        idx += 1;
        while idx < hinum {
            if !hashitem_empty(&harray[idx]) && (var_flavour(harray[idx].key_ptr()) & flavour) != 0
            {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }
}

fn find_buffer(
    fname_bufs: &mut std::collections::HashMap<Vec<u8>, *mut FileBuf>,
    fname: *const c_char,
) -> *mut FileBuf {
    {
        let key = crate::memory::cstr_as_bytes(fname);
        if let Some(&buf) = fname_bufs.get(key) {
            return buf;
        }

        for buf in for_all_buffers_ptr() {
            if !buf.b_ffname.is_null() && path_equal(fname, buf.b_ffname, K_PATH_CMP_LITERAL) {
                fname_bufs.insert(key.to_vec(), buf);
                return buf;
            }
        }
        fname_bufs.insert(key.to_vec(), ptr::null_mut());
        ptr::null_mut()
    }
}

#[inline]
const fn marks_equal(a: CursorPos, b: CursorPos) -> bool {
    a.lnum == b.lnum && a.col == b.col
}

fn marklist_insert<T>(jumps: &mut [T; JUMPLISTSIZE], jl_len: c_int, mut i: c_int) -> c_int {
    if i > 0 {
        if jl_len == (JUMPLISTSIZE as c_int) {
            i -= 1;
            if i > 0 {
                jumps[..=i as usize].rotate_left(1);
            }
        } else if i != jl_len {
            jumps[i as usize..=jl_len as usize].rotate_right(1);
        }
    } else if i == 0 {
        if jl_len == (JUMPLISTSIZE as c_int) {
            return -1;
        } else if jl_len > 0 {
            jumps[..=jl_len as usize].rotate_right(1);
        }
    }
    i
}

fn shada_read(sd_reader: &mut FileDescriptor, flags: c_int) {
    unsafe {
        let mut oldfiles_list = get_vim_var_list(VV_OLDFILES);
        let force = (flags & K_SHA_DA_FORCEIT) != 0;
        let get_old_files = (flags & K_SHA_DA_GET_OLDFILES) != 0
            && (force || tv_list_len(oldfiles_list.as_ref()) == 0);
        let want_marks = (flags & K_SHA_DA_WANT_MARKS) != 0;
        let no_opt = (flags & K_SHA_DA_NO_OPT) != 0;
        let argcount = crate::arglist::argcount();
        let srni_flags: c_uint = (if (flags & K_SHA_DA_WANT_INFO) != 0 {
            K_SDREAD_UNDISABLEABLE_DATA
                | K_SDREAD_REGISTERS
                | K_SDREAD_GLOBAL_MARKS
                | if opt_vars().p_hi != 0 && (flags & K_SHA_DA_NO_HISTORY) == 0 {
                    K_SDREAD_HISTORY
                } else {
                    0
                }
                | if no_opt || !find_shada_parameter(b'!' as c_int).is_null() {
                    K_SDREAD_VARIABLES
                } else {
                    0
                }
                | if (no_opt || !find_shada_parameter(b'%' as c_int).is_null()) && argcount == 0 {
                    K_SDREAD_BUFFER_LIST
                } else {
                    0
                }
        } else {
            0
        }) | if want_marks && get_shada_parameter(b'\'' as c_int) > 0 {
            K_SDREAD_LOCAL_MARKS | K_SDREAD_CHANGES
        } else {
            0
        } | if get_old_files {
            K_SDREAD_LOCAL_MARKS
        } else {
            0
        };

        if srni_flags == 0 {
            return;
        }
        let mut hms: [HistoryMergerState; HIST_COUNT] =
            std::array::from_fn(|_| HistoryMergerState {
                hmll: HMLList {
                    entries: Vec::new(),
                    first: None,
                    last: None,
                    free_entry: None,
                    last_free_entry: 0,
                    size: 0,
                    num_entries: 0,
                    contained_entries: std::collections::HashMap::new(),
                },
                do_merge: false,
                reading: false,
                iter: None,
                last_hist_entry: MISSING_ENTRY,
                history_type: 0,
            });
        if (srni_flags & K_SDREAD_HISTORY) != 0 {
            for (i, hm) in hms.iter_mut().enumerate().take(HIST_COUNT) {
                hms_init(hm, i as u8, opt_vars().p_hi as usize, true, true);
            }
        }
        let mut cur_entry = MISSING_ENTRY;
        let mut cl_bufs: std::collections::HashSet<*mut FileBuf> = std::collections::HashSet::new();
        let mut fname_bufs: std::collections::HashMap<Vec<u8>, *mut FileBuf> =
            std::collections::HashMap::new();
        let mut oldfiles_set: std::collections::HashSet<PathKey> = std::collections::HashSet::new();
        if get_old_files && (oldfiles_list.is_null() || force) {
            oldfiles_list = tv_list_alloc(K_LIST_LEN_UNKNOWN_CONST);
            set_vim_var_list(VV_OLDFILES, oldfiles_list);
        }

        loop {
            let srni_ret = shada_read_next_item(sd_reader, &mut cur_entry, srni_flags, 0);
            if srni_ret == ShaDaReadResult::SDReadStatusFinished {
                break;
            }
            match srni_ret {
                ShaDaReadResult::SDReadStatusSuccess => {}
                ShaDaReadResult::SDReadStatusFinished => std::process::abort(),
                ShaDaReadResult::SDReadStatusNotShaDa | ShaDaReadResult::SDReadStatusReadError => {
                    break;
                }
                ShaDaReadResult::SDReadStatusMalformed => continue,
            }
            match cur_entry.entry_type {
                ShadaEntryType::SDItemMissing => std::process::abort(),
                ShadaEntryType::SDItemUnknown => {}
                ShadaEntryType::SDItemHeader => {
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::SDItemSearchPattern => {
                    let sp = cur_entry.data.search_pattern();
                    if !force {
                        let mut pat = SearchPattern::new();
                        if sp.is_substitute_pattern {
                            get_substitute_pattern(&mut pat);
                        } else {
                            get_search_pattern(&mut pat);
                        }
                        if pat.pat.is_some() && pat.timestamp >= cur_entry.timestamp {
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
                        pat: (!sp.pat.data.is_null()).then(|| {
                            let s = crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                                sp.pat.data.cast::<u8>(),
                                sp.pat.size,
                            ));
                            api_free_string(sp.pat);
                            s
                        }),
                        additional_data: cur_entry.additional_data.take(),
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
                ShadaEntryType::SDItemSubString => {
                    if !force {
                        let mut sub = SubReplacementString::new();
                        sub_get_replacement(&mut sub);
                        if sub.sub.is_some() && sub.timestamp >= cur_entry.timestamp {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    }
                    let mut sub_bytes = cur_entry
                        .data
                        .sub_string_mut()
                        .sub
                        .take()
                        .unwrap_or_default()
                        .into_bytes_with_nul();
                    let final_sub =
                        regtilde(sub_bytes.as_mut_ptr().cast(), opt_vars().p_magic, false).or_else(
                            || Some(crate::memory::cstr_to_cstring(sub_bytes.as_ptr().cast())),
                        );
                    sub_set_replacement(SubReplacementString {
                        sub: final_sub,
                        timestamp: cur_entry.timestamp,
                        additional_data: cur_entry.additional_data.take(),
                    });
                }
                ShadaEntryType::SDItemHistoryEntry => {
                    let ht = cur_entry.data.history_item().histtype as usize;
                    if ht >= HIST_COUNT {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    let entry = std::mem::replace(&mut cur_entry, MISSING_ENTRY);
                    hms_insert(&mut hms[ht], entry, true);
                }
                ShadaEntryType::SDItemRegister => {
                    let r_name = cur_entry.data.reg().name;
                    let r_reg_type = cur_entry.data.reg().reg_type;
                    let r_width = cur_entry.data.reg().width;
                    let r_is_unnamed = cur_entry.data.reg().is_unnamed;
                    let y_type = match r_reg_type {
                        0 => MotionType::MtCharWise,
                        1 => MotionType::MtLineWise,
                        2 => MotionType::MtBlockWise,
                        _ => {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    };
                    let reg_ts = if (flags & K_SHA_DA_NANOS) != 0 {
                        cur_entry.timestamp
                    } else {
                        cur_entry.timestamp.wrapping_mul(NS_PER_SEC)
                    };
                    if !force {
                        let reg = op_reg_get(r_name);
                        if reg.is_null() || (*reg).timestamp >= reg_ts {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    }
                    let y_array = std::mem::take(&mut cur_entry.data.reg_mut().contents);
                    if !op_reg_set(
                        r_name,
                        Yankreg {
                            y_array,
                            y_allocated: true,
                            y_type,
                            y_width: r_width as Colnr,
                            timestamp: reg_ts,
                            additional_data: cur_entry.additional_data.take(),
                        },
                        r_is_unnamed,
                    ) {
                        shada_free_shada_entry(&mut cur_entry);
                    }
                }
                ShadaEntryType::SDItemVariable => {
                    let gv = cur_entry.data.global_var();
                    var_set_global(
                        gv.name.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                        gv.value,
                    );
                    cur_entry.data.global_var_mut().value.v_type = VarType::VarUnknown;
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::SDItemJump | ShadaEntryType::SDItemGlobalMark => {
                    let fm_mark = cur_entry.data.filemark().mark;
                    let fm_name = cur_entry.data.filemark().name;
                    let fname_ptr = cur_entry
                        .data
                        .filemark()
                        .fname
                        .as_ref()
                        .map_or(ptr::null(), |s| s.as_ptr());
                    let buf = find_buffer(&mut fname_bufs, fname_ptr);
                    if !buf.is_null() {
                        cur_entry.data.filemark_mut().fname = None;
                    }
                    let fm = Xfmark {
                        fname: if buf.is_null() {
                            cur_entry.data.filemark_mut().fname.take()
                        } else {
                            None
                        },
                        fmark: Fmark {
                            mark: fm_mark,
                            fnum: if buf.is_null() { 0 } else { (*buf).b_fnum },
                            timestamp: cur_entry.timestamp,
                            view: INIT_FMARKV,
                            additional_data: cur_entry.additional_data.take(),
                        },
                    };
                    if cur_entry.entry_type == ShadaEntryType::SDItemGlobalMark {
                        if !mark_set_global(fm_name, fm, !force) {
                            shada_free_shada_entry(&mut cur_entry);
                        }
                    } else {
                        let mut i = curwin_mut().w_jumplistlen;
                        while i > 0 {
                            let jl_entry = &curwin_mut().w_jumplist[(i - 1) as usize];
                            if jl_entry.fmark.timestamp <= cur_entry.timestamp {
                                if marks_equal(jl_entry.fmark.mark, fm_mark)
                                    && (if buf.is_null() {
                                        jl_entry.fname.is_some() && fm.fname == jl_entry.fname
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
                        if i > 0 && curwin_mut().w_jumplistlen == (JUMPLISTSIZE as c_int) {
                            free_xfmark(&mut curwin_mut().w_jumplist[0]);
                        }
                        i = marklist_insert(
                            &mut curwin_mut().w_jumplist,
                            curwin_mut().w_jumplistlen,
                            i,
                        );
                        if i != -1 {
                            curwin_mut().w_jumplist[i as usize] = fm;
                            if curwin_mut().w_jumplistlen < (JUMPLISTSIZE as c_int) {
                                curwin_mut().w_jumplistlen += 1;
                            }
                            if curwin_mut().w_jumplistidx >= i
                                && curwin_mut().w_jumplistidx < curwin_mut().w_jumplistlen
                            {
                                curwin_mut().w_jumplistidx += 1;
                            }
                        } else {
                            shada_free_shada_entry(&mut cur_entry);
                        }
                    }
                }
                ShadaEntryType::SDItemBufferList => {
                    for item in &mut cur_entry.data.buffer_list_mut().buffers {
                        let fname_ptr = item
                            .fname
                            .as_ref()
                            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                        let sfname = path_try_shorten_fname(fname_ptr);
                        let buf = buflist_new(fname_ptr, sfname, 0, BLN_LISTED);
                        if !buf.is_null() {
                            let buf = { &mut *buf };
                            buf.b_last_cursor = Fmark {
                                mark: item.pos,
                                fnum: 0,
                                timestamp: os_time(),
                                view: INIT_FMARKV,
                                additional_data: None,
                            };
                            buflist_setfpos(
                                buf,
                                curwin_ptr(),
                                buf.b_last_cursor.mark.lnum,
                                buf.b_last_cursor.mark.col,
                                false,
                            );
                            buf.additional_data = item.additional_data.take();
                        }
                    }
                    shada_free_shada_entry(&mut cur_entry);
                }
                ShadaEntryType::SDItemChange | ShadaEntryType::SDItemLocalMark => {
                    let fname_ptr = cur_entry
                        .data
                        .filemark()
                        .fname
                        .as_ref()
                        .map_or(ptr::null(), |s| s.as_ptr());
                    if get_old_files && !oldfiles_set.contains(&PathKey(fname_ptr)) {
                        crate::eval::typval::tv_list_append_string(
                            &mut *oldfiles_list,
                            fname_ptr,
                            -1,
                        );
                        oldfiles_set
                            .insert(PathKey((*(*oldfiles_list).lv_last).li_tv.vval.v_string()));
                    }
                    if !want_marks {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    let fm_mark = cur_entry.data.filemark().mark;
                    let fm_name = cur_entry.data.filemark().name;
                    let buf = find_buffer(&mut fname_bufs, fname_ptr);
                    if buf.is_null() {
                        shada_free_shada_entry(&mut cur_entry);
                        continue;
                    }
                    let buf = { &mut *buf };
                    let fm = Fmark {
                        mark: fm_mark,
                        fnum: buf.b_fnum,
                        timestamp: cur_entry.timestamp,
                        view: INIT_FMARKV,
                        additional_data: cur_entry.additional_data.take(),
                    };
                    if cur_entry.entry_type == ShadaEntryType::SDItemLocalMark {
                        if !mark_set_local(fm_name, &mut *buf, fm, !force) {
                            shada_free_shada_entry(&mut cur_entry);
                            continue;
                        }
                    } else {
                        cl_bufs.insert(buf);
                        let mut i = buf.b_changelistlen;
                        while i > 0 {
                            let jl_entry = &buf.b_changelist[(i - 1) as usize];
                            if jl_entry.timestamp <= cur_entry.timestamp {
                                if marks_equal(jl_entry.mark, fm_mark) {
                                    i = -1;
                                }
                                break;
                            }
                            i -= 1;
                        }
                        i = marklist_insert(&mut buf.b_changelist, buf.b_changelistlen, i);
                        if i != -1 {
                            buf.b_changelist[i as usize] = fm;
                            if buf.b_changelistlen < (JUMPLISTSIZE as c_int) {
                                buf.b_changelistlen += 1;
                            }
                        }
                    }
                    cur_entry.data.filemark_mut().fname = None;
                }
            }
        }

        if (srni_flags & K_SDREAD_HISTORY) != 0 {
            for (i, hm) in hms.iter_mut().enumerate().take(HIST_COUNT) {
                hms_insert_whole_neovim_history(hm);
                clr_history(i as c_int);
                let (hist, new_hisidx, new_hisnum) = hist_get_array(i as u8);
                if !hist.is_empty() {
                    hms_to_he_array(hm, hist, new_hisidx, new_hisnum);
                }
                hms_dealloc(hm);
            }
        }
        if !cl_bufs.is_empty() {
            for tp in for_all_tabpages_ptr() {
                for wp in for_all_windows_in_tab_mut(&*tp) {
                    if cl_bufs.contains(&wp.w_buffer) {
                        wp.w_changelistidx = wp.buf_mut().b_changelistlen;
                    }
                }
            }
        }
    }
}

fn shada_get_default_file() -> *const c_char {
    unsafe {
        if shada_st().default_shada_file.is_none() {
            let shada_dir = stdpaths_user_state_subpath(c"shada", 0, false);
            shada_st().default_shada_file =
                Some(concat_fnames_cstr(shada_dir.as_ptr(), c"main.shada", true));
        }
        (&raw const shada_st().default_shada_file)
            .as_ref()
            .unwrap()
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr())
    }
}

fn shada_filename(file: impl crate::strings::IntoCCharPtr) -> Option<std::ffi::CString> {
    let mut file = file.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_is_empty(file) {
            if !opt_vars().p_shadafile.is_null()
                && !crate::memory::cstr_is_empty(opt_vars().p_shadafile)
            {
                if !(crate::memory::cstr_as_opt_bytes(opt_vars().p_shadafile) == Some(b"NONE")) {
                    file = opt_vars().p_shadafile;
                } else {
                    return None;
                }
            } else {
                file = find_shada_parameter(b'n' as c_int);
                if crate::memory::cstr_is_empty(file) {
                    file = shada_get_default_file();
                }
                let name_buff_ptr = g_st().name_buff.as_mut_ptr();
                let len = expand_env(file.cast_mut(), name_buff_ptr, MAXPATHL as c_int);
                return Some(
                    std::ffi::CString::new(std::slice::from_raw_parts(
                        name_buff_ptr.cast::<u8>(),
                        len,
                    ))
                    .unwrap_or_default(),
                );
            }
        }
        Some(crate::memory::cstr_to_cstring(file))
    }
}

#[inline]
fn shada_check_buffer(packer: &mut PackerBuffer) {
    let remaining = (packer.endptr as usize).saturating_sub(packer.ptr as usize);
    if remaining < SHADA_MPACK_FREE_SPACE {
        crate::msgpack_rpc::packer::packer_flush_buffer(packer);
    }
}

#[inline]
fn additional_data_len(src: &Option<AdditionalData>) -> u32 {
    src.as_ref().map_or(0, |ad| ad.nitems)
}

#[inline]
fn dump_additional_data(src: &Option<AdditionalData>, sbuf: &mut PackerBuffer) {
    if let Some(ad) = src {
        mpack_raw(ad.bytes.as_ptr(), ad.bytes.len(), sbuf);
    }
}

fn shada_pack_entry(
    packer: &mut PackerBuffer,
    entry: &ShadaEntry,
    max_kbyte: usize,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::SDWriteFailed;
        let mut sbuf = packer_string_buffer();
        let def = sd_default_value(entry.entry_type);

        shada_check_buffer(&mut sbuf);
        match entry.entry_type {
            ShadaEntryType::SDItemMissing => std::process::abort(),
            ShadaEntryType::SDItemUnknown => {
                let ui = entry.data.unknown_item();
                mpack_raw(ui.contents.as_ptr(), ui.contents.len(), &mut sbuf);
            }
            ShadaEntryType::SDItemHistoryEntry => {
                let hi = entry.data.history_item();
                let is_hist_search = (hi.histtype as c_int) == HIST_SEARCH;
                let arr_size =
                    2 + u32::from(is_hist_search) + additional_data_len(&entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                mpack_uint(&mut sbuf.ptr, u32::from(hi.histtype));
                mpack_bin(
                    cstr_as_string(hi.string.as_ref().map_or(ptr::null(), |s| s.as_ptr())),
                    &mut sbuf,
                );
                if is_hist_search {
                    mpack_uint(&mut sbuf.ptr, u32::from(hi.sep as u8));
                }
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemVariable => {
                let gv_name_ptr = entry
                    .data
                    .global_var()
                    .name
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr());
                let mut gv_val = entry.data.global_var().value;
                let is_blob = gv_val.v_type == VarType::VarBlob;
                let arr_size = 2 + u32::from(is_blob) + additional_data_len(&entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                let varname = cstr_as_string(gv_name_ptr);
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
                if !encode_vim_to_msgpack(&mut sbuf, &mut gv_val, vardesc.as_ptr()) {
                    ret = ShaDaWriteResult::SDWriteIgnError;
                    semsg!(gettext(c"E574: Failed to write variable %s"), gv_name_ptr);
                    return ret;
                }
                if is_blob {
                    mpack_check_buffer(&mut sbuf);
                    mpack_integer(&mut sbuf.ptr, VAR_TYPE_BLOB);
                }
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemSubString => {
                let arr_size = 1 + additional_data_len(&entry.additional_data);
                mpack_array(&mut sbuf.ptr, arr_size);
                mpack_bin(
                    cstr_as_string(
                        entry
                            .data
                            .sub_string()
                            .sub
                            .as_ref()
                            .map_or(ptr::null(), |s| s.as_ptr()),
                    ),
                    &mut sbuf,
                );
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemSearchPattern => {
                let sp = entry.data.search_pattern();
                let dsp = def.data.search_pattern();
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
                    + additional_data_len(&entry.additional_data);
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
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemChange
            | ShadaEntryType::SDItemGlobalMark
            | ShadaEntryType::SDItemLocalMark
            | ShadaEntryType::SDItemJump => {
                let fm = entry.data.filemark();
                let dfm = def.data.filemark();
                let entry_map_size = 1
                    + u32::from(fm.mark.lnum != dfm.mark.lnum)
                    + u32::from(fm.mark.col != dfm.mark.col)
                    + u32::from(fm.name != dfm.name)
                    + additional_data_len(&entry.additional_data);
                mpack_map(&mut sbuf.ptr, entry_map_size);
                mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
                mpack_bin(
                    cstr_as_string(fm.fname.as_ref().map_or(ptr::null(), |s| s.as_ptr())),
                    &mut sbuf,
                );
                if fm.mark.lnum != dfm.mark.lnum {
                    mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                    mpack_integer(&mut sbuf.ptr, i64::from(fm.mark.lnum));
                }
                if fm.mark.col != dfm.mark.col {
                    mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                    mpack_integer(&mut sbuf.ptr, i64::from(fm.mark.col));
                }
                assert!(if entry.entry_type == ShadaEntryType::SDItemJump
                    || entry.entry_type == ShadaEntryType::SDItemChange
                {
                    fm.name == dfm.name
                } else {
                    true
                });
                if fm.name != dfm.name {
                    mpack_str(static_cstr_as_string(c"n"), &mut sbuf);
                    mpack_uint(&mut sbuf.ptr, u32::from(fm.name as u8));
                }
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemRegister => {
                let r = entry.data.reg();
                let dr = def.data.reg();
                let entry_map_size = 2
                    + u32::from(r.reg_type != dr.reg_type)
                    + u32::from(r.width != dr.width)
                    + u32::from(r.is_unnamed != dr.is_unnamed)
                    + additional_data_len(&entry.additional_data);
                mpack_map(&mut sbuf.ptr, entry_map_size);
                mpack_str(static_cstr_as_string(c"rc"), &mut sbuf);
                mpack_array(&mut sbuf.ptr, r.contents.len() as u32);
                for &s in &r.contents {
                    mpack_bin(s, &mut sbuf);
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
                dump_additional_data(&entry.additional_data, &mut sbuf);
            }
            ShadaEntryType::SDItemBufferList => {
                let bl = entry.data.buffer_list();
                mpack_array(&mut sbuf.ptr, bl.buffers.len() as u32);
                for b in &bl.buffers {
                    let entry_map_size = 1
                        + u32::from(b.pos.lnum != DEFAULT_POS.lnum)
                        + u32::from(b.pos.col != DEFAULT_POS.col)
                        + additional_data_len(&b.additional_data);
                    mpack_map(&mut sbuf.ptr, entry_map_size);
                    mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
                    mpack_bin(
                        cstr_as_string(b.fname.as_ref().map_or(ptr::null(), |s| s.as_ptr())),
                        &mut sbuf,
                    );
                    if b.pos.lnum != 1 {
                        mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                        mpack_uint64(&mut sbuf.ptr, b.pos.lnum as u64);
                    }
                    if b.pos.col != 0 {
                        mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                        mpack_uint64(&mut sbuf.ptr, b.pos.col as u64);
                    }
                    dump_additional_data(&b.additional_data, &mut sbuf);
                }
            }
            ShadaEntryType::SDItemHeader => {
                let hdr = entry.data.header();
                mpack_map(&mut sbuf.ptr, hdr.size as u32);
                for i in 0..hdr.size {
                    let kv = hdr[i];
                    mpack_str(kv.key, &mut sbuf);
                    match kv.value.obj_type {
                        ObjectType::ObjectTypeString => {
                            mpack_bin(kv.value.data.string(), &mut sbuf);
                        }
                        ObjectType::ObjectTypeInteger => {
                            mpack_integer(&mut sbuf.ptr, kv.value.data.integer());
                        }
                        _ => std::process::abort(),
                    }
                }
            }
        }

        let packed = packer_take_string(&mut sbuf);
        if max_kbyte == 0 || packed.len() <= max_kbyte * 1024 {
            shada_check_buffer(packer);
            if entry.entry_type == ShadaEntryType::SDItemUnknown {
                mpack_uint64(&mut packer.ptr, entry.data.unknown_item().item_type);
            } else {
                mpack_uint64(&mut packer.ptr, entry.entry_type as i32 as u64);
            }
            mpack_uint64(&mut packer.ptr, entry.timestamp);
            if !packed.is_empty() {
                mpack_uint64(&mut packer.ptr, packed.len() as u64);
                mpack_raw(packed.as_ptr().cast::<c_char>(), packed.len(), packer);
            }
            if packer.anyint != 0 {
                return ret;
            }
        }
        ret = ShaDaWriteResult::SDWriteSuccessful;
        ret
    }
}

#[inline]
fn shada_pack_pfreed_entry(
    packer: &mut PackerBuffer,
    mut entry: ShadaEntry,
    max_kbyte: usize,
) -> ShaDaWriteResult {
    let ret = shada_pack_entry(packer, &entry, max_kbyte);
    shada_free_shada_entry(&mut entry);
    ret
}

fn shada_check_status(initial_fpos: u64, status: c_int, remaining: usize) -> ShaDaReadResult {
    match status {
        MPACK_OK => {
            if remaining != 0 {
                semsg!(
                        gettext(
                            c"E576: Failed to parse ShaDa file: extra bytes in msgpack string at position %llu",
                        ),
                        initial_fpos,
                    );
                return ShaDaReadResult::SDReadStatusNotShaDa;
            }
            ShaDaReadResult::SDReadStatusSuccess
        }
        MPACK_EOF => {
            semsg!(
                gettext(
                    c"E576: Failed to parse ShaDa file: incomplete msgpack string at position %llu",
                ),
                initial_fpos,
            );
            ShaDaReadResult::SDReadStatusNotShaDa
        }
        _ => {
            semsg!(
                    gettext(
                        c"E576: Failed to parse ShaDa file due to a msgpack parser error at position %llu",
                    ),
                    initial_fpos,
                );
            ShaDaReadResult::SDReadStatusNotShaDa
        }
    }
}

fn shada_read_when_writing(
    sd_reader: &mut FileDescriptor,
    srni_flags: c_uint,
    max_kbyte: usize,
    wms: &mut WriteMergerState,
    packer: &mut PackerBuffer,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::SDWriteSuccessful;
        let mut entry = MISSING_ENTRY;

        loop {
            let srni_ret = shada_read_next_item(sd_reader, &mut entry, srni_flags, max_kbyte);
            if srni_ret == ShaDaReadResult::SDReadStatusFinished {
                break;
            }
            match srni_ret {
                ShaDaReadResult::SDReadStatusSuccess => {}
                ShaDaReadResult::SDReadStatusFinished => std::process::abort(),
                ShaDaReadResult::SDReadStatusNotShaDa => {
                    return ShaDaWriteResult::SDWriteReadNotShada;
                }
                ShaDaReadResult::SDReadStatusReadError => {
                    return ret;
                }
                ShaDaReadResult::SDReadStatusMalformed => continue,
            }

            let compare_with_entry = |wms_entry: *mut ShadaEntry, mut e: ShadaEntry| {
                if (*wms_entry).entry_type != ShadaEntryType::SDItemMissing {
                    if (*wms_entry).timestamp >= e.timestamp {
                        shada_free_shada_entry(&mut e);
                        return;
                    }
                    shada_free_shada_entry(&mut *wms_entry);
                }
                *wms_entry = e;
            };

            match entry.entry_type {
                ShadaEntryType::SDItemMissing => {}
                ShadaEntryType::SDItemHeader | ShadaEntryType::SDItemBufferList => {
                    std::process::abort();
                }
                ShadaEntryType::SDItemUnknown => {
                    ret = shada_pack_entry(packer, &entry, 0);
                    shada_free_shada_entry(&mut entry);
                }
                ShadaEntryType::SDItemSearchPattern => {
                    let is_sub = entry.data.search_pattern().is_substitute_pattern;
                    let wms_pat = if is_sub {
                        &raw mut wms.sub_search_pattern
                    } else {
                        &raw mut wms.search_pattern
                    };
                    if (*wms_pat).entry_type == ShadaEntryType::SDItemMissing
                        && search_pattern_cleared(is_sub)
                        && get_search_pattern_timestamp(is_sub) >= entry.timestamp
                    {
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                    compare_with_entry(wms_pat, e);
                }
                ShadaEntryType::SDItemSubString => {
                    let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                    compare_with_entry(&raw mut wms.replacement, e);
                }
                ShadaEntryType::SDItemHistoryEntry => {
                    let ht = entry.data.history_item().histtype as usize;
                    if ht >= HIST_COUNT {
                        ret = shada_pack_entry(packer, &entry, 0);
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    if wms.hms[ht].hmll.size != 0 {
                        let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                        hms_insert(&mut wms.hms[ht], e, true);
                    } else {
                        shada_free_shada_entry(&mut entry);
                    }
                }
                ShadaEntryType::SDItemRegister => {
                    let reg_name = entry.data.reg().name;
                    let idx = op_reg_index(c_int::from(reg_name));
                    if idx < 0 {
                        ret = shada_pack_entry(packer, &entry, 0);
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    let u_idx = idx as usize;
                    if wms.registers[u_idx].entry_type == ShadaEntryType::SDItemMissing {
                        let reg = op_reg_get(reg_name);
                        if !reg.is_null()
                            && reg_empty(&*reg)
                            && (*reg).timestamp / NS_PER_SEC >= entry.timestamp
                        {
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                    }
                    let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                    compare_with_entry(&raw mut wms.registers[u_idx], e);
                }
                ShadaEntryType::SDItemVariable => {
                    let var_name = entry
                        .data
                        .global_var()
                        .name
                        .as_ref()
                        .map_or(&b""[..], |s| s.to_bytes());
                    if !wms.dumped_variables.contains(var_name) {
                        ret = shada_pack_entry(packer, &entry, 0);
                    }
                    shada_free_shada_entry(&mut entry);
                }
                ShadaEntryType::SDItemGlobalMark => {
                    let fm_mark = entry.data.filemark().mark;
                    let name = entry.data.filemark().name;
                    if (name as u8).is_ascii_digit() {
                        let mut processed_mark = false;
                        for i in (1..=EXTRA_MARKS).rev() {
                            let wms_entry = &wms.numbered_marks[i - 1];
                            if wms_entry.entry_type != ShadaEntryType::SDItemGlobalMark {
                                continue;
                            }
                            if wms_entry.timestamp == entry.timestamp
                                && wms_entry.additional_data.is_none()
                                && entry.additional_data.is_none()
                                && marks_equal(wms_entry.data.filemark().mark, fm_mark)
                                && wms_entry.data.filemark().fname == entry.data.filemark().fname
                            {
                                shada_free_shada_entry(&mut entry);
                                processed_mark = true;
                                break;
                            }
                            if wms_entry.timestamp >= entry.timestamp {
                                processed_mark = true;
                                if i < EXTRA_MARKS {
                                    let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                                    replace_numbered_mark(wms, i, e);
                                } else {
                                    shada_free_shada_entry(&mut entry);
                                }
                                break;
                            }
                        }
                        if !processed_mark {
                            let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                            replace_numbered_mark(wms, 0, e);
                        }
                    } else {
                        let idx = mark_global_index(name);
                        if idx < 0 {
                            ret = shada_pack_entry(packer, &entry, 0);
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                        let u_idx = idx as usize;
                        let mark = if u_idx < 26 {
                            &raw mut wms.global_marks[u_idx]
                        } else {
                            &raw mut wms.numbered_marks[u_idx - 26]
                        };
                        if (*mark).entry_type == ShadaEntryType::SDItemMissing
                            && g_st().namedfm[u_idx].fmark.timestamp >= entry.timestamp
                        {
                            shada_free_shada_entry(&mut entry);
                            continue;
                        }
                        let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                        compare_with_entry(mark, e);
                    }
                }
                ShadaEntryType::SDItemChange | ShadaEntryType::SDItemLocalMark => {
                    let fm_mark = entry.data.filemark().mark;
                    let fm_name = entry.data.filemark().name;
                    let fname_ptr = entry
                        .data
                        .filemark()
                        .fname
                        .as_ref()
                        .map_or(ptr::null(), |s| s.as_ptr());
                    if shada_removable(fname_ptr) {
                        shada_free_shada_entry(&mut entry);
                        continue;
                    }
                    let fname_bytes = entry
                        .data
                        .filemark()
                        .fname
                        .as_ref()
                        .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
                    let filemarks: *mut FileMarks = wms
                        .file_marks
                        .entry(fname_bytes)
                        .or_insert_with(FileMarks::new);
                    let filemarks = &mut *filemarks;
                    if entry.timestamp > filemarks.greatest_timestamp {
                        filemarks.greatest_timestamp = entry.timestamp;
                    }
                    if entry.entry_type == ShadaEntryType::SDItemLocalMark {
                        let idx = mark_local_index(fm_name);
                        if idx < 0 {
                            let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                            filemarks.additional_marks.push(e);
                        } else {
                            let wms_entry = &raw mut filemarks.marks[idx as usize];
                            let mut set_wms = true;
                            if (*wms_entry).entry_type != ShadaEntryType::SDItemMissing {
                                if (*wms_entry).timestamp >= entry.timestamp {
                                    shada_free_shada_entry(&mut entry);
                                    continue;
                                }
                                if (*wms_entry).can_free_entry {
                                    shada_free_shada_entry(&mut *wms_entry);
                                }
                            } else {
                                for buf in for_all_buffers_mut() {
                                    if !buf.b_ffname.is_null()
                                        && path_equal(fname_ptr, buf.b_ffname, K_PATH_CMP_LITERAL)
                                    {
                                        let mut fm = Fmark::new();
                                        mark_get(
                                            buf,
                                            curwin_mut(),
                                            Some(&mut fm),
                                            K_MARK_BUF_LOCAL,
                                            c_int::from(fm_name),
                                        );
                                        if fm.timestamp >= entry.timestamp {
                                            set_wms = false;
                                            shada_free_shada_entry(&mut entry);
                                            break;
                                        }
                                    }
                                }
                            }
                            if set_wms {
                                *wms_entry = std::mem::replace(&mut entry, MISSING_ENTRY);
                            }
                        }
                    } else {
                        let mut i = filemarks.changes_size as c_int;
                        while i > 0 {
                            let jl_entry = &filemarks.changes[(i - 1) as usize];
                            if jl_entry.timestamp <= entry.timestamp {
                                if marks_equal(jl_entry.data.filemark().mark, fm_mark) {
                                    i = -1;
                                }
                                break;
                            }
                            i -= 1;
                        }
                        if i > 0 && filemarks.changes_size == JUMPLISTSIZE {
                            shada_free_shada_entry(&mut filemarks.changes[0]);
                        }
                        i = marklist_insert(
                            &mut filemarks.changes,
                            filemarks.changes_size as c_int,
                            i,
                        );
                        if i != -1 {
                            filemarks.changes[i as usize] =
                                std::mem::replace(&mut entry, MISSING_ENTRY);
                            if filemarks.changes_size < JUMPLISTSIZE {
                                filemarks.changes_size += 1;
                            }
                        } else {
                            shada_free_shada_entry(&mut entry);
                        }
                    }
                }
                ShadaEntryType::SDItemJump => {
                    let fm_mark = entry.data.filemark().mark;
                    let mut i = wms.jumps_size as c_int;
                    while i > 0 {
                        let jl_entry = &wms.jumps[(i - 1) as usize];
                        if jl_entry.timestamp <= entry.timestamp {
                            if marks_equal(jl_entry.data.filemark().mark, fm_mark)
                                && jl_entry.data.filemark().fname == entry.data.filemark().fname
                            {
                                i = -1;
                            }
                            break;
                        }
                        i -= 1;
                    }
                    if i > 0 && wms.jumps_size == JUMPLISTSIZE {
                        shada_free_shada_entry(&mut wms.jumps[0]);
                    }
                    i = marklist_insert(&mut wms.jumps, wms.jumps_size as c_int, i);
                    if i != -1 {
                        wms.jumps[i as usize] = std::mem::replace(&mut entry, MISSING_ENTRY);
                        if wms.jumps_size < JUMPLISTSIZE {
                            wms.jumps_size += 1;
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
fn ignore_buf(
    buf: *const FileBuf,
    removable_bufs: &std::collections::HashSet<*mut FileBuf>,
) -> bool {
    unsafe {
        buf.is_null()
            || (*buf).b_ffname.is_null()
            || ((*buf).b_p_bl == 0 && (*buf).b_p_initialized)
            || bt_quickfix(buf.as_ref())
            || bt_terminal(buf.as_ref())
            || removable_bufs.contains(&buf.cast_mut())
    }
}

#[inline]
fn shada_get_buflist(removable_bufs: &std::collections::HashSet<*mut FileBuf>) -> ShadaEntry {
    {
        let max_bufs = get_shada_parameter(b'%' as c_int);
        let mut buffers = Vec::new();
        for buf in for_all_buffers_ptr() {
            if !ignore_buf(buf, removable_bufs) && buf.b_p_bl != 0 {
                if max_bufs >= 0 && buffers.len() >= (max_bufs as usize) {
                    break;
                }
                buffers.push(BufferListBuffer {
                    pos: buf.b_last_cursor.mark,
                    fname: Some(crate::memory::cstr_to_cstring(buf.b_ffname)),
                    additional_data: buf.additional_data.clone(),
                });
            }
        }

        ShadaEntry {
            entry_type: ShadaEntryType::SDItemBufferList,
            can_free_entry: false,
            timestamp: os_time(),
            data: ShadaEntryData::BufferList(ShadaBufferList { buffers }),
            additional_data: None,
        }
    }
}

#[inline]
fn add_search_pattern(
    ret_pse: &mut ShadaEntry,
    get_pattern: fn(&mut SearchPattern),
    is_substitute_pattern: bool,
    search_last_used: bool,
    search_highlighted: bool,
) {
    {
        let mut pat = SearchPattern::new();
        get_pattern(&mut pat);
        if pat.pat.is_some() {
            let is_last_used = is_substitute_pattern ^ search_last_used;
            *ret_pse = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::SDItemSearchPattern,
                timestamp: pat.timestamp,
                data: ShadaEntryData::SearchPattern(KeyDict__shada_search_pat {
                    is_set_shada_search_pat: 0,
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
                    pat: cstr_as_string(get_search_pattern_ptr(is_substitute_pattern)),
                    search_backward: !is_substitute_pattern && pat.off.dir == (b'?' as c_char),
                }),
                additional_data: pat.additional_data,
            };
        }
    }
}

#[inline]
fn shada_initialize_registers(wms: &mut WriteMergerState, max_reg_lines: c_int, scale_ts: bool) {
    unsafe {
        let mut reg_iter: Option<usize> = None;
        let limit_reg_lines = max_reg_lines >= 0;
        loop {
            let mut reg: *const Yankreg = ptr::null();
            let mut name: c_char = NUL;
            let mut is_unnamed = false;
            reg_iter = op_global_reg_iter(reg_iter, &mut name, &mut reg, &mut is_unnamed);
            if name == NUL {
                break;
            }
            let reg_ref = &*reg;
            if limit_reg_lines && reg_ref.y_array.len() > (max_reg_lines as usize) {
                if reg_iter.is_none() {
                    break;
                }
                continue;
            }
            let idx = op_reg_index(c_int::from(name)) as usize;
            wms.registers[idx] = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::SDItemRegister,
                timestamp: if scale_ts {
                    reg_ref.timestamp / NS_PER_SEC
                } else {
                    reg_ref.timestamp
                },
                data: ShadaEntryData::Reg(ShadaReg {
                    contents: reg_ref.y_array.clone(),
                    reg_type: reg_ref.y_type as u8,
                    width: if reg_ref.y_type == MotionType::MtBlockWise {
                        reg_ref.y_width as usize
                    } else {
                        0
                    },
                    name,
                    is_unnamed,
                }),
                additional_data: reg_ref.additional_data.clone(),
            };
            if reg_iter.is_none() {
                break;
            }
        }
    }
}

#[inline]
fn replace_numbered_mark(wms: &mut WriteMergerState, idx: usize, entry: ShadaEntry) {
    unsafe {
        shada_free_shada_entry(&mut wms.numbered_marks[EXTRA_MARKS - 1]);
        for i in idx..(EXTRA_MARKS - 1) {
            if wms.numbered_marks[i].entry_type == ShadaEntryType::SDItemGlobalMark {
                wms.numbered_marks[i].data.filemark_mut().name = (b'0' + (i as u8) + 1) as c_char;
            }
        }
        let base = (&raw mut wms.numbered_marks).cast::<ShadaEntry>();
        ptr::copy(base.add(idx), base.add(idx + 1), EXTRA_MARKS - 1 - idx);
        ptr::write(base.add(idx), entry);
        wms.numbered_marks[idx].data.filemark_mut().name = (b'0' + (idx as u8)) as c_char;
    }
}

#[inline]
fn find_removable_bufs(removable_bufs: &mut std::collections::HashSet<*mut FileBuf>) {
    {
        for buf in for_all_buffers_ptr() {
            if !buf.b_ffname.is_null() && shada_removable(buf.b_ffname) {
                removable_bufs.insert(buf);
            }
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

pub(crate) fn flush_file_buffer(buffer: &mut PackerBuffer) {
    unsafe {
        let fd = buffer.anydata.as_file();
        let base = fd.buffer.as_ptr();
        fd.write_pos = (buffer.ptr as usize).saturating_sub(base as usize);
        buffer.anyint = i64::from(file_flush(fd));
        buffer.ptr = fd.buffer.as_mut_ptr().add(fd.write_pos);
    }
}

fn packer_buffer_for_file(file: &mut FileDescriptor) -> PackerBuffer {
    unsafe {
        if file_space(&*file) < SHADA_MPACK_FREE_SPACE {
            file_flush(&raw mut *file);
        }
        let base = file.buffer.as_mut_ptr();
        PackerBuffer {
            startptr: base,
            ptr: base.add(file.write_pos),
            endptr: base.add(ARENA_BLOCK_SIZE),
            anydata: crate::msgpack_rpc::packer::PackerData::File(file),
            anyint: 0,
            owned_buf: Vec::new(),
        }
    }
}

fn shada_write(
    sd_writer: &mut FileDescriptor,
    sd_reader: Option<&mut FileDescriptor>,
) -> ShaDaWriteResult {
    unsafe {
        let mut ret = ShaDaWriteResult::SDWriteSuccessful;
        let mut max_kbyte_i = get_shada_parameter(b's' as c_int);
        if max_kbyte_i < 0 {
            max_kbyte_i = 10;
        }
        if max_kbyte_i == 0 {
            return ret;
        }

        let mut wms_box = WriteMergerState::new();
        let wms: *mut WriteMergerState = &raw mut *wms_box;
        let wms = &mut *wms;
        let mut dump_one_history = [false; HIST_COUNT];
        let dump_global_vars = !find_shada_parameter(b'!' as c_int).is_null();
        let mut max_reg_lines = get_shada_parameter(b'<' as c_int);
        if max_reg_lines < 0 {
            max_reg_lines = get_shada_parameter(b'"' as c_int);
        }
        let dump_registers = max_reg_lines != 0;
        let mut removable_bufs: std::collections::HashSet<*mut FileBuf> =
            std::collections::HashSet::new();
        let max_kbyte = max_kbyte_i as usize;
        let num_marked_files = get_shada_parameter(b'\'' as c_int) as usize;
        let dump_global_marks = get_shada_parameter(b'f' as c_int) != 0;
        let mut dump_history = false;

        for (i, dump_one) in dump_one_history.iter_mut().enumerate().take(HIST_COUNT) {
            let mut num_saved = get_shada_parameter(hist_type2char(i as c_int));
            if num_saved == -1 {
                num_saved = opt_vars().p_hi as c_int;
            }
            if num_saved > 0 {
                dump_history = true;
                *dump_one = true;
                hms_init(
                    &mut wms.hms[i],
                    i as u8,
                    num_saved as usize,
                    sd_reader.is_some(),
                    false,
                );
            } else {
                *dump_one = false;
            }
        }

        let srni_flags: c_uint = K_SDREAD_UNDISABLEABLE_DATA
            | K_SDREAD_UNKNOWN
            | if dump_history { K_SDREAD_HISTORY } else { 0 }
            | if dump_registers {
                K_SDREAD_REGISTERS
            } else {
                0
            }
            | if dump_global_vars {
                K_SDREAD_VARIABLES
            } else {
                0
            }
            | if dump_global_marks {
                K_SDREAD_GLOBAL_MARKS
            } else {
                0
            }
            | if num_marked_files != 0 {
                K_SDREAD_LOCAL_MARKS | K_SDREAD_CHANGES
            } else {
                0
            };

        let mut packer = packer_buffer_for_file(&mut *sd_writer);

        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_ptr(&*tp) {
                set_last_cursor(&mut *wp);
            }
        }

        find_removable_bufs(&mut removable_bufs);

        let mut header_items = [
            KeyValuePair {
                key: static_cstr_as_string(c"generator"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(static_cstr_as_string(c"nvim")),
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"version"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(long_version())),
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"max_kbyte"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(max_kbyte as Integer),
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"pid"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeInteger,
                    data: ObjectData::Integer(os_get_pid() as Integer),
                },
            },
            KeyValuePair {
                key: static_cstr_as_string(c"encoding"),
                value: Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(opt_vars().p_enc)),
                },
            },
        ];

        let write_body: ShaDaWriteResult = 'shada_write_exit: {
            if shada_pack_entry(
                &mut packer,
                &ShadaEntry {
                    entry_type: ShadaEntryType::SDItemHeader,
                    can_free_entry: false,
                    timestamp: os_time(),
                    data: ShadaEntryData::Header(Dict {
                        items: header_items.as_mut_ptr(),
                        size: 5,
                        capacity: 5,
                    }),
                    additional_data: None,
                },
                0,
            ) == ShaDaWriteResult::SDWriteFailed
            {
                break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
            }

            if !find_shada_parameter(b'%' as c_int).is_null() {
                let buflist_entry = shada_get_buflist(&removable_bufs);
                let pack_res = shada_pack_entry(&mut packer, &buflist_entry, 0);
                if pack_res == ShaDaWriteResult::SDWriteFailed {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }

            if dump_global_vars {
                let mut var_iter: Option<usize> = None;
                let cur_timestamp = os_time();
                loop {
                    let mut vartv = TypVal::new();
                    let mut name: *const c_char = ptr::null();
                    var_iter = var_shada_iter(var_iter, &mut name, &mut vartv, VAR_FLAVOUR_SHADA);
                    if name.is_null() {
                        break;
                    }
                    let mut skip = false;
                    match vartv.v_type {
                        VarType::VarFunc | VarType::VarPartial => {
                            tv_clear(&mut vartv);
                            skip = true;
                        }
                        VarType::VarDict => {
                            let di = vartv.vval.v_dict();
                            let copy_id = get_copy_id();
                            if !set_ref_in_ht(&mut (*di).dv_hashtab, copy_id, ptr::null_mut())
                                && copy_id == (*di).dv_copy_id
                            {
                                tv_clear(&mut vartv);
                                skip = true;
                            }
                        }
                        VarType::VarList => {
                            let l = vartv.vval.v_list();
                            let copy_id = get_copy_id();
                            if !set_ref_in_list_items(l, copy_id, ptr::null_mut())
                                && copy_id == (*l).lv_copy_id
                            {
                                tv_clear(&mut vartv);
                                skip = true;
                            }
                        }
                        _ => {}
                    }
                    if !skip {
                        let mut tgttv = TypVal::new();
                        tv_copy(&vartv, &mut tgttv);
                        let spe_ret = shada_pack_entry(
                            &mut packer,
                            &ShadaEntry {
                                entry_type: ShadaEntryType::SDItemVariable,
                                can_free_entry: false,
                                timestamp: cur_timestamp,
                                data: ShadaEntryData::GlobalVar(ShadaGlobalVar {
                                    name: Some(crate::memory::cstr_to_cstring(name)),
                                    value: tgttv,
                                }),
                                additional_data: None,
                            },
                            max_kbyte,
                        );
                        if spe_ret == ShaDaWriteResult::SDWriteFailed {
                            tv_clear(&mut vartv);
                            tv_clear(&mut tgttv);
                            break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                        }
                        tv_clear(&mut vartv);
                        tv_clear(&mut tgttv);
                        if spe_ret == ShaDaWriteResult::SDWriteSuccessful {
                            wms.dumped_variables
                                .insert(crate::memory::cstr_as_bytes(name).to_vec());
                        }
                    }
                    if var_iter.is_none() {
                        break;
                    }
                }
            }

            if num_marked_files > 0 {
                wms.jumps_size = shada_init_jumps(&mut wms.jumps, &removable_bufs);
            }

            if dump_one_history[HIST_SEARCH as usize] {
                let search_highlighted = !(g_st().search_global.no_hlsearch
                    || !find_shada_parameter(b'h' as c_int).is_null());
                let search_last_used = search_was_last_used();

                add_search_pattern(
                    &mut wms.search_pattern,
                    get_search_pattern,
                    false,
                    search_last_used,
                    search_highlighted,
                );
                add_search_pattern(
                    &mut wms.sub_search_pattern,
                    get_substitute_pattern,
                    true,
                    search_last_used,
                    search_highlighted,
                );

                let mut sub = SubReplacementString::new();
                sub_get_replacement(&mut sub);
                if sub.sub.is_some() {
                    wms.replacement = ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::SDItemSubString,
                        timestamp: sub.timestamp,
                        data: ShadaEntryData::SubString(ShadaSubString { sub: sub.sub }),
                        additional_data: sub.additional_data,
                    };
                }
            }

            if dump_global_marks {
                let mut global_mark_iter: Option<usize> = None;
                let mut digit_mark_idx = 0usize;
                loop {
                    let mut name: c_char = NUL;
                    let mut fm = Xfmark::new();
                    global_mark_iter = mark_global_iter(global_mark_iter, &mut name, &mut fm);
                    if name == NUL {
                        break;
                    }
                    let fname: std::ffi::CString = if fm.fmark.fnum == 0 {
                        let Some(f) = fm.fname.take() else {
                            if global_mark_iter.is_none() {
                                break;
                            }
                            continue;
                        };
                        if shada_removable(f.as_ptr()) {
                            if global_mark_iter.is_none() {
                                break;
                            }
                            continue;
                        }
                        f
                    } else {
                        let buf = buflist_findnr(fm.fmark.fnum);
                        if buf.is_null()
                            || (*buf).b_ffname.is_null()
                            || removable_bufs.contains(&buf)
                        {
                            if global_mark_iter.is_none() {
                                break;
                            }
                            continue;
                        }
                        crate::memory::cstr_to_cstring((*buf).b_ffname)
                    };
                    let entry = ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::SDItemGlobalMark,
                        timestamp: fm.fmark.timestamp,
                        data: ShadaEntryData::Filemark(ShadaFilemark {
                            mark: fm.fmark.mark,
                            name,
                            fname: Some(fname),
                        }),
                        additional_data: fm.fmark.additional_data,
                    };
                    if (name as u8).is_ascii_digit() {
                        replace_numbered_mark(&mut *wms, digit_mark_idx, entry);
                        digit_mark_idx += 1;
                    } else {
                        wms.global_marks[mark_global_index(name) as usize] = entry;
                    }
                    if global_mark_iter.is_none() {
                        break;
                    }
                }
            }

            if dump_registers {
                shada_initialize_registers(&mut *wms, max_reg_lines, true);
            }

            if num_marked_files > 0 {
                for buf in for_all_buffers_ptr() {
                    if !ignore_buf(buf, &removable_bufs) {
                        let mut local_marks_iter: Option<c_char> = None;
                        let fname = buf.b_ffname;
                        let fname_cstr = crate::memory::cstr_to_cstring(fname);
                        let fname_bytes = fname_cstr.to_bytes().to_vec();
                        let filemarks: *mut FileMarks = wms
                            .file_marks
                            .entry(fname_bytes)
                            .or_insert_with(FileMarks::new);
                        let filemarks = &mut *filemarks;
                        loop {
                            let mut fm = Fmark::new();
                            let mut name: c_char = NUL;
                            local_marks_iter =
                                mark_buffer_iter(local_marks_iter, &*buf, &mut name, &mut fm);
                            if name == NUL {
                                break;
                            }
                            filemarks.marks[mark_local_index(name) as usize] = ShadaEntry {
                                can_free_entry: false,
                                entry_type: ShadaEntryType::SDItemLocalMark,
                                timestamp: fm.timestamp,
                                data: ShadaEntryData::Filemark(ShadaFilemark {
                                    mark: fm.mark,
                                    name,
                                    fname: Some(fname_cstr.clone()),
                                }),
                                additional_data: fm.additional_data,
                            };
                            if fm.timestamp > filemarks.greatest_timestamp {
                                filemarks.greatest_timestamp = fm.timestamp;
                            }
                            if local_marks_iter.is_none() {
                                break;
                            }
                        }
                        for i in 0..(buf.b_changelistlen as usize) {
                            let fm = &buf.b_changelist[i];
                            filemarks.changes[i] = ShadaEntry {
                                can_free_entry: false,
                                entry_type: ShadaEntryType::SDItemChange,
                                timestamp: fm.timestamp,
                                data: ShadaEntryData::Filemark(ShadaFilemark {
                                    name: NUL,
                                    mark: fm.mark,
                                    fname: Some(fname_cstr.clone()),
                                }),
                                additional_data: fm.additional_data.clone(),
                            };
                            if fm.timestamp > filemarks.greatest_timestamp {
                                filemarks.greatest_timestamp = fm.timestamp;
                            }
                        }
                        filemarks.changes_size = buf.b_changelistlen as usize;
                    }
                }
            }

            if let Some(sd_reader) = sd_reader {
                let srww_ret = shada_read_when_writing(
                    sd_reader,
                    srni_flags,
                    max_kbyte,
                    &mut *wms,
                    &mut packer,
                );
                if srww_ret != ShaDaWriteResult::SDWriteSuccessful {
                    ret = srww_ret;
                }
            }

            if dump_global_marks
                && !ignore_buf(g_st().curbuf, &removable_bufs)
                && curwin_mut().w_cursor.lnum != 0
            {
                replace_numbered_mark(
                    &mut *wms,
                    0,
                    ShadaEntry {
                        can_free_entry: false,
                        entry_type: ShadaEntryType::SDItemGlobalMark,
                        timestamp: os_time(),
                        data: ShadaEntryData::Filemark(ShadaFilemark {
                            mark: curwin_mut().w_cursor,
                            name: b'0' as c_char,
                            fname: Some(crate::memory::cstr_to_cstring(curbuf_mut().b_ffname)),
                        }),
                        additional_data: None,
                    },
                );
            }

            for entry in &mut wms.global_marks {
                let entry = std::mem::replace(entry, MISSING_ENTRY);
                if entry.entry_type != ShadaEntryType::SDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for entry in &mut wms.numbered_marks {
                let entry = std::mem::replace(entry, MISSING_ENTRY);
                if entry.entry_type != ShadaEntryType::SDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for entry in &mut wms.registers {
                let entry = std::mem::replace(entry, MISSING_ENTRY);
                if entry.entry_type != ShadaEntryType::SDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for i in 0..wms.jumps_size {
                let entry = std::mem::replace(&mut wms.jumps[i], MISSING_ENTRY);
                if shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                    == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for entry in [
                std::mem::replace(&mut wms.search_pattern, MISSING_ENTRY),
                std::mem::replace(&mut wms.sub_search_pattern, MISSING_ENTRY),
                std::mem::replace(&mut wms.replacement, MISSING_ENTRY),
            ] {
                if entry.entry_type != ShadaEntryType::SDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }

            let mut all_file_markss: Vec<*mut FileMarks> = wms
                .file_marks
                .values_mut()
                .map(|fm| fm as *mut FileMarks)
                .collect();
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
                for entry in &mut (*fm_ptr).marks {
                    let entry = std::mem::replace(entry, MISSING_ENTRY);
                    if entry.entry_type != ShadaEntryType::SDItemMissing
                        && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                            == ShaDaWriteResult::SDWriteFailed
                    {
                        break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                    }
                }
                for j in 0..(*fm_ptr).changes_size {
                    let entry = std::mem::replace(&mut (*fm_ptr).changes[j], MISSING_ENTRY);
                    if shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                    {
                        break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                    }
                }
                for mut add_entry in (*fm_ptr).additional_marks.drain(..) {
                    if shada_pack_entry(&mut packer, &add_entry, 0)
                        == ShaDaWriteResult::SDWriteFailed
                    {
                        shada_free_shada_entry(&mut add_entry);
                        break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                    }
                    shada_free_shada_entry(&mut add_entry);
                }
            }

            if dump_history {
                for (i, &dump_one) in dump_one_history.iter().enumerate().take(HIST_COUNT) {
                    if dump_one {
                        hms_insert_whole_neovim_history(&mut wms.hms[i]);
                        let mut cur_idx = wms.hms[i].hmll.first;
                        while let Some(idx) = cur_idx {
                            let entries = wms.hms[i].hmll.entries.as_mut_slice();
                            let next_idx = entries[idx].next;
                            let entry = std::mem::replace(&mut entries[idx].data, MISSING_ENTRY);
                            if shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                                == ShaDaWriteResult::SDWriteFailed
                            {
                                break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                            }
                            cur_idx = next_idx;
                        }
                    }
                }
            }

            ret
        };
        ret = write_body;

        for (i, &dump_one) in dump_one_history.iter().enumerate().take(HIST_COUNT) {
            if dump_one {
                hms_dealloc(&mut wms.hms[i]);
            }
        }
        wms.file_marks.clear();
        crate::msgpack_rpc::packer::packer_flush_buffer(&mut packer);
        ret
    }
}

/// Write ShaDa file to a given location (`shada_write_file`).
pub fn shada_write_file(file: impl crate::strings::IntoCCharPtr, mut nomerge: bool) -> bool {
    let file = file.into_c_char_ptr();
    unsafe {
        let Some(fname_cstr) = shada_filename(file) else {
            return false;
        };
        let mut fname_bytes = fname_cstr.into_bytes_with_nul();
        let fname = fname_bytes.as_mut_ptr().cast::<c_char>();

        let mut tempname_bytes: Vec<u8> = Vec::new();
        let mut sd_writer = FileDescriptor::new();
        let mut sd_reader = FileDescriptor::new();
        let mut did_open_writer = false;
        let mut did_open_reader = false;

        if !nomerge {
            let mut error = file_open(&mut sd_reader, fname, K_FILE_READ_ONLY, 0);
            if error != 0 {
                if error != UV_ENOENT {
                    semsg!(
                        gettext(
                            c"E886: System error while opening ShaDa file %s for reading to merge before writing it: %s",
                        ),
                        fname,
                        uv_strerror(error),
                    );
                }
                nomerge = true;
            } else {
                did_open_reader = true;
                if let Some(tn) = modname(fname, c".tmp.a", false) {
                    tempname_bytes = tn.into_bytes_with_nul();
                    let tempname = tempname_bytes.as_mut_ptr().cast::<c_char>();
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
                                let last_idx = tempname_bytes.len() - 2;
                                if tempname_bytes[last_idx] == b'z' {
                                    semsg!(
                                        gettext(
                                            c"E138: All %s.tmp.X files exist, cannot write ShaDa file!",
                                        ),
                                        fname,
                                    );
                                    if did_open_reader {
                                        close_file(&mut sd_reader);
                                    }
                                    return false;
                                }
                                tempname_bytes[last_idx] += 1;
                                continue;
                            } else {
                                semsg!(
                                    gettext(
                                        c"E886: System error while opening temporary ShaDa file %s for writing: %s",
                                    ),
                                    tempname,
                                    uv_strerror(error),
                                );
                            }
                        } else {
                            did_open_writer = true;
                        }
                        break;
                    }
                } else {
                    nomerge = true;
                }
            }
        }

        if nomerge {
            let tail = path_tail_with_sep(fname);
            if tail != fname {
                let tail_save = *tail;
                *tail = NUL;
                if !os_isdir(fname) {
                    let mut failed_dir: Option<std::ffi::CString> = None;
                    let ret = os_mkdir_recurse(fname, 0o700, &mut failed_dir, None);
                    if ret != 0 {
                        semsg!(
                            gettext(
                                c"E886: Failed to create directory %s for writing ShaDa file: %s",
                            ),
                            failed_dir.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                            uv_strerror(ret),
                        );
                        return false;
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
                semsg!(
                    gettext(c"E886: System error while opening ShaDa file %s for writing: %s",),
                    fname,
                    uv_strerror(error),
                );
            } else {
                did_open_writer = true;
            }
        }

        if !did_open_writer {
            if did_open_reader {
                close_file(&mut sd_reader);
            }
            return false;
        }

        if opt_vars().p_verbose > 1 {
            verbose_enter();
            smsg!(0, gettext(c"Writing ShaDa file \"%s\""), fname);
            verbose_leave();
        }

        let sw_ret = shada_write(
            &mut sd_writer,
            (if nomerge {
                ptr::null_mut()
            } else {
                &mut sd_reader
            })
            .as_mut(),
        );
        assert!(sw_ret != ShaDaWriteResult::SDWriteIgnError);
        if !nomerge {
            let tempname = tempname_bytes.as_ptr().cast::<c_char>();
            if did_open_reader {
                close_file(&mut sd_reader);
            }
            let mut did_remove = false;
            if sw_ret == ShaDaWriteResult::SDWriteSuccessful {
                let mut old_info = FileInfo::new();
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
                    semsg!(gettext(c"E137: ShaDa file is not writable: %s"), fname,);
                } else {
                    #[cfg(unix)]
                    let mut chown_ok = true;
                    #[cfg(not(unix))]
                    let chown_ok = true;
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
                                semsg!(
                                    gettext(c"E136: Failed setting uid and gid for file %s: %s",),
                                    tempname,
                                    uv_strerror(fchown_ret),
                                );
                                chown_ok = false;
                            }
                        }
                    }
                    if chown_ok {
                        if vim_rename(tempname, fname) == -1 {
                            semsg!(
                                gettext(c"E136: Can't rename ShaDa file from %s to %s!"),
                                tempname,
                                fname,
                            );
                        } else {
                            did_remove = true;
                            os_remove(tempname);
                        }
                    }
                }
            } else if sw_ret == ShaDaWriteResult::SDWriteReadNotShada {
                semsg!(
                    gettext(c"E136: Did not rename %s because %s does not look like a ShaDa file",),
                    tempname,
                    fname,
                );
            } else {
                semsg!(
                    gettext(
                        c"E136: Did not rename %s to %s because there were errors during writing it",
                    ),
                    tempname,
                    fname,
                );
            }
            if !did_remove {
                semsg!(
                    gettext(c"E136: Do not forget to remove %s or rename it manually to %s.",),
                    tempname,
                    fname,
                );
            }
        }
        close_file(&mut sd_writer);

        true
    }
}

/// Read marks information from ShaDa file (`shada_read_marks`).
pub fn shada_read_marks() -> bool {
    shada_read_file(ptr::null(), K_SHA_DA_WANT_MARKS)
}

/// Read all information from ShaDa file (`shada_read_everything`).
pub fn shada_read_everything(
    fname: impl crate::strings::IntoCCharPtr,
    forceit: bool,
    missing_ok: bool,
) -> bool {
    let fname = fname.into_c_char_ptr();
    shada_read_file(
        fname,
        K_SHA_DA_WANT_INFO
            | K_SHA_DA_WANT_MARKS
            | K_SHA_DA_GET_OLDFILES
            | if forceit { K_SHA_DA_FORCEIT } else { 0 }
            | if missing_ok {
                0
            } else {
                K_SHA_DA_MISSING_ERROR
            },
    )
}

fn shada_free_shada_entry(entry: &mut ShadaEntry) {
    {
        if !entry.can_free_entry {
            return;
        }
        match entry.entry_type {
            ShadaEntryType::SDItemMissing => {}
            ShadaEntryType::SDItemUnknown => {}
            ShadaEntryType::SDItemHeader => {
                api_free_dict(entry.data.header());
            }
            ShadaEntryType::SDItemChange
            | ShadaEntryType::SDItemJump
            | ShadaEntryType::SDItemGlobalMark
            | ShadaEntryType::SDItemLocalMark => {
                entry.data.filemark_mut().fname = None;
            }
            ShadaEntryType::SDItemSearchPattern => {
                api_free_string(entry.data.search_pattern().pat);
            }
            ShadaEntryType::SDItemRegister => {
                for s in entry.data.reg_mut().contents.drain(..) {
                    api_free_string(s);
                }
            }
            ShadaEntryType::SDItemHistoryEntry => {
                entry.data.history_item_mut().string = None;
            }
            ShadaEntryType::SDItemVariable => {
                entry.data.global_var_mut().name = None;
                tv_clear(&mut entry.data.global_var_mut().value);
            }
            ShadaEntryType::SDItemSubString => {
                entry.data.sub_string_mut().sub = None;
            }
            ShadaEntryType::SDItemBufferList => {
                entry.data.buffer_list_mut().buffers.clear();
            }
        }
        entry.additional_data = None;
    }
}

fn fread_len(
    sd_reader: *mut FileDescriptor,
    buffer: impl crate::strings::IntoCCharPtr,
    length: usize,
) -> ShaDaReadResult {
    let buffer = buffer.into_c_char_ptr().cast_mut();
    unsafe {
        let read_bytes = file_read(sd_reader, buffer, length);
        if read_bytes < 0 {
            semsg!(
                gettext(c"E886: System error while reading ShaDa file: %s"),
                uv_strerror(read_bytes as c_int),
            );
            return ShaDaReadResult::SDReadStatusReadError;
        }
        if read_bytes != (length as isize) {
            semsg!(
                gettext(
                    c"E576: Error while reading ShaDa file: last entry specified that it occupies %llu bytes, but file ended earlier",
                ),
                length as u64,
            );
            return ShaDaReadResult::SDReadStatusNotShaDa;
        }
        ShaDaReadResult::SDReadStatusSuccess
    }
}

fn msgpack_read_uint64(
    sd_reader: &mut FileDescriptor,
    allow_eof: bool,
    result: &mut u64,
) -> ShaDaReadResult {
    unsafe {
        let fpos = sd_reader.bytes_read;
        let mut ret: u8 = 0;
        let read_bytes = file_read((&raw mut *sd_reader), (&raw mut ret).cast::<c_char>(), 1);
        if read_bytes < 0 {
            semsg!(
                gettext(c"E886: System error while reading integer from ShaDa file: %s"),
                uv_strerror(read_bytes as c_int),
            );
            return ShaDaReadResult::SDReadStatusReadError;
        } else if read_bytes == 0 {
            if allow_eof && file_eof(sd_reader) {
                return ShaDaReadResult::SDReadStatusFinished;
            }
            semsg!(
                gettext(
                    c"E576: Error while reading ShaDa file: expected positive integer at position %llu, but got nothing",
                ),
                fpos,
            );
            return ShaDaReadResult::SDReadStatusNotShaDa;
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
                    semsg!(
                        gettext(
                            c"E576: Error while reading ShaDa file: expected positive integer at position %llu",
                        ),
                        fpos,
                    );
                    return ShaDaReadResult::SDReadStatusNotShaDa;
                }
            };
            let mut buf = [0u8; 8];
            let fl_ret = fread_len(
                (&raw mut *sd_reader),
                buf.as_mut_ptr().add(8 - length).cast::<c_char>(),
                length,
            );
            if fl_ret != ShaDaReadResult::SDReadStatusSuccess {
                return fl_ret;
            }
            *result = u64::from_be_bytes(buf);
        }
        ShaDaReadResult::SDReadStatusSuccess
    }
}

fn shada_read_next_item(
    sd_reader: &mut FileDescriptor,
    entry: &mut ShadaEntry,
    flags: c_uint,
    max_kbyte: usize,
) -> ShaDaReadResult {
    unsafe {
        let mut ret = ShaDaReadResult::SDReadStatusMalformed;
        loop {
            *entry = MISSING_ENTRY;
            if file_eof(sd_reader) {
                return ShaDaReadResult::SDReadStatusFinished;
            }

            let mut verify_but_ignore = false;
            let mut type_u64: u64 = 0;
            let mut timestamp_u64: u64 = 0;
            let mut length_u64: u64 = 0;

            let initial_fpos = sd_reader.bytes_read;
            let mut ad = AdditionalDataBuilder::default();
            let mut read_additional_array_elements: u32 = 0;
            let mut error_alloc: Option<std::ffi::CString> = None;

            let mut mru_ret = msgpack_read_uint64(sd_reader, true, &mut type_u64);
            if mru_ret != ShaDaReadResult::SDReadStatusSuccess {
                return mru_ret;
            }
            mru_ret = msgpack_read_uint64(sd_reader, false, &mut timestamp_u64);
            if mru_ret != ShaDaReadResult::SDReadStatusSuccess {
                return mru_ret;
            }
            mru_ret = msgpack_read_uint64(sd_reader, false, &mut length_u64);
            if mru_ret != ShaDaReadResult::SDReadStatusSuccess {
                return mru_ret;
            }

            if length_u64 > (isize::MAX as u64) {
                semsg!(
                    gettext(
                        c"E576: Error while reading ShaDa file: there is an item at position %llu that is stated to be too long",
                    ),
                    initial_fpos,
                );
                return ShaDaReadResult::SDReadStatusNotShaDa;
            }

            let length = length_u64 as usize;
            entry.timestamp = timestamp_u64;
            entry.can_free_entry = true;

            if type_u64 == 0 {
                semsg!(
                    gettext(
                        c"E576: Error while reading ShaDa file: there is an item at position %llu that must not be there: Missing items are for internal uses only",
                    ),
                    initial_fpos,
                );
                return ShaDaReadResult::SDReadStatusNotShaDa;
            }

            let skip_by_flags = if type_u64 > SHADA_LAST_ENTRY {
                (flags & K_SDREAD_UNKNOWN) == 0
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
                    if srs_ret != ShaDaReadResult::SDReadStatusSuccess {
                        return srs_ret;
                    }
                    continue;
                }
            }

            let parse_pos = sd_reader.bytes_read;
            let mut owned_buf: Vec<c_char> = Vec::new();
            let mut buf = file_try_read_buffered((&raw mut *sd_reader), length);
            if buf.is_null() {
                owned_buf = vec![0 as c_char; length];
                buf = owned_buf.as_mut_ptr();
                let fl_ret = fread_len((&raw mut *sd_reader), buf, length);
                if fl_ret != ShaDaReadResult::SDReadStatusSuccess {
                    return fl_ret;
                }
            }

            let mut read_ptr: *const c_char = buf;
            let mut read_size: usize = length;

            if verify_but_ignore {
                let status = unpack_skip(&mut read_ptr, &mut read_size);
                let spm_ret = shada_check_status(parse_pos, status, read_size);
                if spm_ret != ShaDaReadResult::SDReadStatusSuccess {
                    return spm_ret;
                }
                continue;
            }

            if type_u64 > SHADA_LAST_ENTRY {
                entry.entry_type = ShadaEntryType::SDItemUnknown;
                if initial_fpos == 0 {
                    let status = unpack_skip(&mut read_ptr, &mut read_size);
                    let spm_ret = shada_check_status(parse_pos, status, read_size);
                    if spm_ret != ShaDaReadResult::SDReadStatusSuccess {
                        entry.entry_type = ShadaEntryType::SDItemMissing;
                        return spm_ret;
                    }
                }
                let contents = if !owned_buf.is_empty() {
                    owned_buf
                } else {
                    std::slice::from_raw_parts(buf, length).to_vec()
                };
                entry.data = ShadaEntryData::UnknownItem(ShadaUnknownItem {
                    item_type: type_u64,
                    contents,
                });
                return ShaDaReadResult::SDReadStatusSuccess;
            }

            let entry_type = match type_u64 {
                1 => ShadaEntryType::SDItemHeader,
                2 => ShadaEntryType::SDItemSearchPattern,
                3 => ShadaEntryType::SDItemSubString,
                4 => ShadaEntryType::SDItemHistoryEntry,
                5 => ShadaEntryType::SDItemRegister,
                6 => ShadaEntryType::SDItemVariable,
                7 => ShadaEntryType::SDItemGlobalMark,
                8 => ShadaEntryType::SDItemJump,
                9 => ShadaEntryType::SDItemBufferList,
                10 => ShadaEntryType::SDItemLocalMark,
                11 => ShadaEntryType::SDItemChange,
                _ => ShadaEntryType::SDItemUnknown,
            };
            entry.data = sd_default_value(entry_type).data;

            let parse_ok: bool = 'parse_item: {
                match entry_type {
                    ShadaEntryType::SDItemHeader => {}
                    ShadaEntryType::SDItemSearchPattern => {
                        let it = entry.data.search_pattern_mut();
                        if !unpack_keydict(
                            (&raw mut *it).cast(),
                            Some(shada_search_pat_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: search pattern entry at position %llu %s",
                                ),
                                initial_fpos,
                                error_alloc.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                            );
                            it.pat = NvimString {
                                data: ptr::null_mut(),
                                size: 0,
                            };
                            break 'parse_item false;
                        }
                        if (it.is_set_shada_search_pat
                            & (1u64 << KEYSET_OPTIDX_SHADA_SEARCH_PAT_SP))
                            == 0
                        {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: search pattern entry at position %llu has no pattern",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        it.pat = copy_string(it.pat, None);
                    }
                    ShadaEntryType::SDItemChange
                    | ShadaEntryType::SDItemJump
                    | ShadaEntryType::SDItemGlobalMark
                    | ShadaEntryType::SDItemLocalMark => {
                        let mut it = KeyDict__shada_mark::default();
                        if !unpack_keydict(
                            (&raw mut it).cast(),
                            Some(shada_mark_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu %s",
                                ),
                                initial_fpos,
                                error_alloc.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                            );
                            break 'parse_item false;
                        }
                        let fm = entry.data.filemark_mut();
                        if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_N)) != 0 {
                            if entry_type == ShadaEntryType::SDItemJump
                                || entry_type == ShadaEntryType::SDItemChange
                            {
                                semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: mark entry at position %llu has n key which is only valid for local and global mark entries",
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            fm.name = it.n as c_char;
                        }
                        if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_L)) != 0 {
                            fm.mark.lnum = it.l as Linenr;
                        }
                        if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_C)) != 0 {
                            fm.mark.col = it.c as Colnr;
                        }
                        if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_F)) != 0 {
                            let mut bytes =
                                crate::memory::cstr_slice(it.f.data, it.f.size).to_vec();
                            bytes.push(0);
                            #[cfg(windows)]
                            path_to_slash(bytes.as_mut_ptr().cast::<c_char>());
                            fm.fname = Some(
                                CStr::from_bytes_with_nul(&bytes)
                                    .unwrap_or_default()
                                    .to_owned(),
                            );
                        }
                        if fm.fname.is_none() {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu is missing file name",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if fm.mark.lnum <= 0 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu has invalid line number",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if fm.mark.col < 0 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: mark entry at position %llu has invalid column number",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                    }
                    ShadaEntryType::SDItemRegister => {
                        let mut it = KeyDict__shada_register {
                            is_set_shada_register: 0,
                            rc: Vec::new(),
                            ru: false,
                            rt: 0,
                            n: 0,
                            rw: 0,
                        };
                        if !unpack_keydict(
                            (&raw mut it).cast(),
                            Some(shada_register_get_field_cb),
                            &mut ad,
                            &mut read_ptr,
                            &mut read_size,
                            &mut error_alloc,
                        ) {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: register entry at position %llu %s",
                                ),
                                initial_fpos,
                                error_alloc.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                            );
                            break 'parse_item false;
                        }
                        if it.rc.is_empty() {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: register entry at position %llu has rc key with missing or empty array",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        for s in &mut it.rc {
                            *s = copy_string(*s, None);
                        }
                        let reg = entry.data.reg_mut();
                        reg.contents = it.rc;

                        if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RU))
                            != 0
                        {
                            reg.is_unnamed = it.ru;
                        }
                        if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RT))
                            != 0
                        {
                            reg.reg_type = it.rt as u8;
                        }
                        if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_N))
                            != 0
                        {
                            reg.name = it.n as c_char;
                        }
                        if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RW))
                            != 0
                        {
                            reg.width = it.rw as usize;
                        }
                    }
                    ShadaEntryType::SDItemHistoryEntry => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 2 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu is not an array with enough elements",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let mut hist_type: Integer = 0;
                        if !unpack_integer(&mut read_ptr, &mut read_size, &mut hist_type) {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history type type",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let item = unpack_string(&mut read_ptr, &mut read_size);
                        if item.data.is_null() {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history string type",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let item_slice = crate::memory::cstr_slice(item.data, item.size);
                        if item_slice.contains(&0) {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu contains string with zero byte inside",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let hi = entry.data.history_item_mut();
                        hi.histtype = hist_type as u8;
                        let is_hist_search = (hi.histtype as c_int) == HIST_SEARCH;
                        if is_hist_search {
                            if len < 3 {
                                semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: search history entry at position %llu does not have separator character",
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            let mut sep_type: Integer = 0;
                            if !unpack_integer(&mut read_ptr, &mut read_size, &mut sep_type) {
                                semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: search history entry at position %llu has wrong history separator type",
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                            hi.sep = sep_type as c_char;
                        }
                        hi.string = Some(
                            std::ffi::CString::new(std::slice::from_raw_parts(
                                item.data.cast::<u8>(),
                                item.size,
                            ))
                            .unwrap_or_default(),
                        );
                        read_additional_array_elements =
                            (len as u32) - (2 + u32::from(is_hist_search));
                    }
                    ShadaEntryType::SDItemVariable => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 2 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: variable entry at position %llu is not an array with enough elements",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let name = unpack_string(&mut read_ptr, &mut read_size);
                        if name.data.is_null() {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: variable entry at position %llu has wrong variable name type",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let gv = entry.data.global_var_mut();
                        gv.name = Some(
                            std::ffi::CString::new(std::slice::from_raw_parts(
                                name.data.cast::<u8>(),
                                name.size,
                            ))
                            .unwrap_or_default(),
                        );
                        let binval = unpack_string(&mut read_ptr, &mut read_size);
                        let mut is_blob = false;
                        if !binval.data.is_null() {
                            if len > 2 {
                                let mut vtype: Integer = 0;
                                if !unpack_integer(&mut read_ptr, &mut read_size, &mut vtype)
                                    || vtype != VAR_TYPE_BLOB
                                {
                                    semsg!(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: variable entry at position %llu has wrong variable type",
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                is_blob = true;
                            }
                            gv.value = decode_string(binval.data, binval.size, is_blob, false);
                        } else {
                            let status =
                                unpack_typval(&mut read_ptr, &mut read_size, &mut gv.value);
                            if status != MPACK_OK {
                                semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: variable entry at position %llu has value that cannot be converted to the Vimscript value",
                                    ),
                                    initial_fpos,
                                );
                                break 'parse_item false;
                            }
                        }
                        read_additional_array_elements = (len as u32) - 2 - u32::from(is_blob);
                    }
                    ShadaEntryType::SDItemSubString => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 1 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu is not an array with enough elements",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        let sub = unpack_string(&mut read_ptr, &mut read_size);
                        if sub.data.is_null() {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu has wrong sub string type",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        entry.data.sub_string_mut().sub = Some(
                            std::ffi::CString::new(std::slice::from_raw_parts(
                                sub.data.cast::<u8>(),
                                sub.size,
                            ))
                            .unwrap_or_default(),
                        );
                        read_additional_array_elements = (len as u32) - 1;
                    }
                    ShadaEntryType::SDItemBufferList => {
                        let len = unpack_array(&mut read_ptr, &mut read_size);
                        if len < 0 {
                            semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: buffer list entry at position %llu is not an array",
                                ),
                                initial_fpos,
                            );
                            break 'parse_item false;
                        }
                        if len > 0 {
                            let ulen = len as usize;
                            let buffers = &mut entry.data.buffer_list_mut().buffers;
                            buffers.reserve(ulen);
                            for _ in 0..ulen {
                                let mut it = KeyDict__shada_buflist_item::default();
                                let mut it_ad = AdditionalDataBuilder::default();
                                if !unpack_keydict(
                                    (&raw mut it).cast(),
                                    Some(shada_buflist_item_get_field_cb),
                                    &mut it_ad,
                                    &mut read_ptr,
                                    &mut read_size,
                                    &mut error_alloc,
                                ) {
                                    semsg!(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry that %s",
                                        ),
                                        initial_fpos,
                                        error_alloc.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                                    );
                                    break 'parse_item false;
                                }
                                let mut e = BufferListBuffer {
                                    pos: DEFAULT_POS,
                                    fname: None,
                                    additional_data: additional_data_take(&mut it_ad),
                                };
                                if (it.is_set_shada_buflist_item
                                    & (1u64 << KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_L))
                                    != 0
                                {
                                    e.pos.lnum = it.l as Linenr;
                                }
                                if (it.is_set_shada_buflist_item
                                    & (1u64 << KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_C))
                                    != 0
                                {
                                    e.pos.col = it.c as Colnr;
                                }
                                if (it.is_set_shada_buflist_item
                                    & (1u64 << KEYSET_OPTIDX_SHADA_BUFLIST_ITEM_F))
                                    != 0
                                {
                                    let mut bytes = std::slice::from_raw_parts(
                                        it.f.data.cast::<u8>(),
                                        it.f.size,
                                    )
                                    .to_vec();
                                    bytes.push(0);
                                    #[cfg(windows)]
                                    path_to_slash(bytes.as_mut_ptr().cast::<c_char>());
                                    e.fname = Some(
                                        CStr::from_bytes_with_nul(&bytes)
                                            .unwrap_or_default()
                                            .to_owned(),
                                    );
                                }
                                let has_fname = e.fname.is_some();
                                let pos = e.pos;
                                buffers.push(e);
                                if pos.lnum <= 0 {
                                    semsg!(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry with invalid line number",
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                if pos.col < 0 {
                                    semsg!(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry with invalid column number",
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                                if !has_fname {
                                    semsg!(
                                        gettext(
                                            c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry that does not have a file name",
                                        ),
                                        initial_fpos,
                                    );
                                    break 'parse_item false;
                                }
                            }
                        }
                    }
                    ShadaEntryType::SDItemMissing | ShadaEntryType::SDItemUnknown => {
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
                    semsg!(
                        gettext(
                            c"E575: Error while reading ShaDa file: item entry at position %llu additional bytes",
                        ),
                        initial_fpos,
                    );
                    break 'parse_item false;
                }

                true
            };

            if parse_ok {
                entry.entry_type = entry_type;
                entry.additional_data = additional_data_take(&mut ad);
                ret = ShaDaReadResult::SDReadStatusSuccess;
            } else {
                entry.entry_type = entry_type;
                shada_free_shada_entry(entry);
                entry.entry_type = ShadaEntryType::SDItemMissing;
            }

            drop(owned_buf);
            return ret;
        }
    }
}

fn shada_removable(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    unsafe {
        let mut part = [0 as c_char; MAXPATHL + 1];
        let mut retval = false;
        let new_name = home_replace_save(None, name);
        let mut p = opt_vars().p_shada;
        while *p != 0 {
            copy_option_part(&mut p, part.as_mut_ptr(), part.len(), c", ");
            if part[0] == (b'r' as c_char) {
                let name_buff_ptr = g_st().name_buff.as_mut_ptr();
                let n = home_replace(None, part.as_ptr().add(1), name_buff_ptr, MAXPATHL, true);
                if mb_strnicmp(name_buff_ptr, new_name.as_ptr(), n) == 0 {
                    retval = true;
                    break;
                }
            }
        }
        retval
    }
}

#[inline]
fn shada_init_jumps(
    jumps: &mut [ShadaEntry; JUMPLISTSIZE],
    removable_bufs: &std::collections::HashSet<*mut FileBuf>,
) -> usize {
    unsafe {
        let mut jumps_size = 0usize;
        let mut jump_iter: Option<usize> = None;
        setpcmark();
        cleanup_jumplist(curwin_mut(), false);
        loop {
            let mut fm = Xfmark::new();
            jump_iter = mark_jumplist_iter(jump_iter, curwin_mut(), &mut fm);
            if fm.fmark.mark.lnum == 0 {
                siemsg!(
                    c"ShaDa: mark lnum zero (ji:%zu, js:%p, len:%i)",
                    jump_iter.unwrap_or(0),
                    &raw const curwin_mut().w_jumplist,
                    curwin_mut().w_jumplistlen,
                );
                if jump_iter.is_none() {
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
                if jump_iter.is_none() {
                    break;
                }
                continue;
            }
            let fname = if fm.fmark.fnum == 0 {
                fm.fname
            } else if !buf.is_null() && !(*buf).b_ffname.is_null() {
                Some(crate::memory::cstr_to_cstring((*buf).b_ffname))
            } else {
                None
            };
            if fname.is_none() {
                if jump_iter.is_none() {
                    break;
                }
                continue;
            }
            jumps[jumps_size] = ShadaEntry {
                can_free_entry: false,
                entry_type: ShadaEntryType::SDItemJump,
                timestamp: fm.fmark.timestamp,
                data: ShadaEntryData::Filemark(ShadaFilemark {
                    name: NUL,
                    mark: fm.fmark.mark,
                    fname,
                }),
                additional_data: fm.fmark.additional_data,
            };
            jumps_size += 1;
            if jump_iter.is_none() {
                break;
            }
        }
        jumps_size
    }
}

/// Gets registers as a msgpack-encoded byte buffer, in shada format (`shada_encode_regs`).
pub fn shada_encode_regs(scale_ts: bool, since: Timestamp) -> Vec<u8> {
    let mut wms = WriteMergerState::new();
    shada_initialize_registers(&mut wms, -1, scale_ts);
    let mut packer = packer_string_buffer();
    for entry in wms.registers {
        if entry.entry_type == ShadaEntryType::SDItemRegister
            && entry.timestamp >= since
            && shada_pack_pfreed_entry(&mut packer, entry, 0) == ShaDaWriteResult::SDWriteFailed
        {
            std::process::abort();
        }
    }
    packer_take_string(&mut packer)
}

/// Gets the jumplist as a msgpack-encoded byte buffer, in shada format (`shada_encode_jumps`).
pub fn shada_encode_jumps() -> Vec<u8> {
    let mut removable_bufs: std::collections::HashSet<*mut FileBuf> =
        std::collections::HashSet::new();
    find_removable_bufs(&mut removable_bufs);
    let mut jumps = [const { MISSING_ENTRY }; JUMPLISTSIZE];
    let jumps_size = shada_init_jumps(&mut jumps, &removable_bufs);
    let mut packer = packer_string_buffer();
    for entry in &mut jumps[..jumps_size] {
        let entry = std::mem::replace(entry, MISSING_ENTRY);
        if shada_pack_pfreed_entry(&mut packer, entry, 0) == ShaDaWriteResult::SDWriteFailed {
            std::process::abort();
        }
    }
    packer_take_string(&mut packer)
}

/// Gets the buffer-list as a msgpack-encoded byte buffer, in shada format (`shada_encode_buflist`).
pub fn shada_encode_buflist() -> Vec<u8> {
    let mut removable_bufs: std::collections::HashSet<*mut FileBuf> =
        std::collections::HashSet::new();
    find_removable_bufs(&mut removable_bufs);
    let buflist_entry = shada_get_buflist(&removable_bufs);

    let mut packer = packer_string_buffer();
    if shada_pack_entry(&mut packer, &buflist_entry, 0) == ShaDaWriteResult::SDWriteFailed {
        std::process::abort();
    }
    packer_take_string(&mut packer)
}

/// Gets global Vimscript variables as a msgpack-encoded byte buffer, in shada format (`shada_encode_gvars`).
pub fn shada_encode_gvars() -> Vec<u8> {
    {
        let mut packer = packer_string_buffer();
        let mut var_iter: Option<usize> = None;
        let cur_timestamp = os_time();
        loop {
            let mut vartv = TypVal::new();
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
            if vartv.v_type != VarType::VarFunc && vartv.v_type != VarType::VarPartial {
                let mut tgttv = TypVal::new();
                tv_copy(&vartv, &mut tgttv);
                let r = shada_pack_entry(
                    &mut packer,
                    &ShadaEntry {
                        entry_type: ShadaEntryType::SDItemVariable,
                        can_free_entry: false,
                        timestamp: cur_timestamp,
                        data: ShadaEntryData::GlobalVar(ShadaGlobalVar {
                            name: Some(crate::memory::cstr_to_cstring(name)),
                            value: tgttv,
                        }),
                        additional_data: None,
                    },
                    0,
                );
                if r == ShaDaWriteResult::SDWriteFailed {
                    std::process::abort();
                }
                tv_clear(&mut tgttv);
            }
            tv_clear(&mut vartv);
            if var_iter.is_none() {
                break;
            }
        }
        packer_take_string(&mut packer)
    }
}

/// Read ShaDa from byte slice (`shada_read_string`).
pub fn shada_read_string(bytes: &[u8], flags: c_int) {
    if bytes.is_empty() {
        return;
    }
    let mut sd_reader = FileDescriptor::new();
    file_open_buffer(
        &mut sd_reader,
        bytes.as_ptr().cast::<c_char>().cast_mut(),
        bytes.len(),
    );
    shada_read(&mut sd_reader, flags);
    close_file(&mut sd_reader);
}

/// Find the parameter represented by the given character in 'shada' (`get_shada_parameter`).
pub fn get_shada_parameter(param_type: c_int) -> c_int {
    unsafe {
        let p = find_shada_parameter(param_type);
        if !p.is_null() && (crate::memory::cstr_first_byte(p)).is_ascii_digit() {
            let mut val: c_int = 0;
            let mut cur = p;
            while (crate::memory::cstr_first_byte(cur)).is_ascii_digit() {
                val = val
                    .wrapping_mul(10)
                    .wrapping_add(c_int::from((crate::memory::cstr_first_byte(cur)) - b'0'));
                cur = cur.add(1);
            }
            return val;
        }
        -1
    }
}

/// Find the parameter represented by the given character in 'shada' (`find_shada_parameter`).
pub fn find_shada_parameter(param_type: c_int) -> *mut c_char {
    unsafe {
        let mut p = opt_vars().p_shada;
        while *p != 0 {
            if c_int::from(crate::memory::cstr_first_byte(p)) == param_type {
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
pub fn check_marks_read() {
    {
        if !curbuf_mut().b_marks_read
            && get_shada_parameter(b'\'' as c_int) > 0
            && !curbuf_mut().b_ffname.is_null()
        {
            shada_read_marks();
        }
        curbuf_mut().b_marks_read = true;
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
