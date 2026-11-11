#![forbid(unsafe_code)]

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
    hist_iter, hist_with_array_mut,
};
use crate::context::INIT_FMARKV;
use crate::cursor::{Colnr, CursorPos, Linenr};
use crate::eval::decode::{decode_string, unpack_typval};
use crate::eval::encode::encode_vim_to_msgpack;
use crate::eval::typval::{
    TypVal, TypvalVval, VarType, tv_clear, tv_copy, tv_list_alloc, tv_list_len,
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
    FileDescriptor, K_FILE_CREATE_ONLY, K_FILE_NO_SYMLINK, K_FILE_READ_ONLY, file_close,
    file_flush, file_open, file_open_buffer, file_read, file_skip, file_try_read_buffered,
};
use crate::os::fs::{
    CASE_INSENSITIVE_FILENAME, FileInfo, UV_ENOENT, os_file_mkdir, os_fileinfo, os_fileinfo_link,
    os_getperm, os_remove,
};
use crate::os::stdpaths::stdpaths_user_state_subpath;
use crate::os::time::{Timestamp, os_time};
use crate::path::{
    K_PATH_CMP_LITERAL, concat_fnames_cstr, path_cmp, path_equal, path_fold_char,
    path_try_shorten_fname, vim_ispathsep, vim_ispathsep_nocolon,
};
use crate::regexp::{K_MARK_BUF_LOCAL, regtilde};
use crate::register::{
    MotionType, NUM_SAVED_REGISTERS, Yankreg, op_reg_get, op_reg_index, op_reg_set, reg_empty,
    with_global_reg_iter,
};
use crate::search::{
    AdditionalData, SearchOffset, SearchPattern, get_search_pattern, get_search_pattern_timestamp,
    get_substitute_pattern, search_pattern_cleared, search_was_last_used, set_last_used_pattern,
    set_search_pattern, set_substitute_pattern,
};
use crate::strings::{gettext, vim_strchr};
use std::cmp::Ordering;
use std::ffi::CStr;
use std::ptr;

#[cfg(unix)]
use crate::os::fs::os_fchown;
#[cfg(windows)]
use crate::path::path_to_slash;

pub const K_SHA_DA_WANT_INFO: i32 = 1;
pub const K_SHA_DA_WANT_MARKS: i32 = 2;
pub const K_SHA_DA_FORCEIT: i32 = 4;
pub const K_SHA_DA_GET_OLDFILES: i32 = 8;
pub const K_SHA_DA_MISSING_ERROR: i32 = 16;
pub const K_SHA_DA_NANOS: i32 = 32;
pub const K_SHA_DA_NO_HISTORY: i32 = 64;
pub const K_SHA_DA_NO_OPT: i32 = 128;

const NLOCALMARKS: usize = NMARKS + 3;
const SHADA_MPACK_FREE_SPACE: usize = 4 * MPACK_ITEM_SIZE;

#[cfg(windows)]
const UV_EEXIST: i32 = -4075;
#[cfg(windows)]
const UV_ELOOP: i32 = -4067;
#[cfg(not(windows))]
const UV_EEXIST: i32 = -17;
#[cfg(target_os = "linux")]
const UV_ELOOP: i32 = -40;
#[cfg(all(not(windows), not(target_os = "linux")))]
const UV_ELOOP: i32 = -62;

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

const K_SDREAD_HEADER: u32 = 1 << (ShadaEntryType::SDItemHeader as u32);
const K_SDREAD_UNDISABLEABLE_DATA: u32 = (1 << (ShadaEntryType::SDItemSearchPattern as u32))
    | (1 << (ShadaEntryType::SDItemSubString as u32))
    | (1 << (ShadaEntryType::SDItemJump as u32));
const K_SDREAD_REGISTERS: u32 = 1 << (ShadaEntryType::SDItemRegister as u32);
const K_SDREAD_HISTORY: u32 = 1 << (ShadaEntryType::SDItemHistoryEntry as u32);
const K_SDREAD_VARIABLES: u32 = 1 << (ShadaEntryType::SDItemVariable as u32);
const K_SDREAD_BUFFER_LIST: u32 = 1 << (ShadaEntryType::SDItemBufferList as u32);
const K_SDREAD_UNKNOWN: u32 = 1 << ((SHADA_LAST_ENTRY as u32) + 1);
const K_SDREAD_GLOBAL_MARKS: u32 = 1 << (ShadaEntryType::SDItemGlobalMark as u32);
const K_SDREAD_LOCAL_MARKS: u32 = 1 << (ShadaEntryType::SDItemLocalMark as u32);
const K_SDREAD_CHANGES: u32 = 1 << (ShadaEntryType::SDItemChange as u32);

#[derive(Clone)]
struct ShadaFilemark {
    name: u8,
    mark: CursorPos,
    fname: Option<std::ffi::CString>,
}

#[derive(Clone)]
struct ShadaHistoryItem {
    histtype: u8,
    string: Option<std::ffi::CString>,
    sep: u8,
}

#[repr(C)]
#[derive(Clone)]
struct ShadaReg {
    name: u8,
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
    contents: Vec<u8>,
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
    pat: NvimString::new(),
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
            data: ShadaEntryData::Header(Dict::new()),
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
                sep: 0,
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemRegister => ShadaEntry {
            entry_type: ShadaEntryType::SDItemRegister,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Reg(ShadaReg {
                name: 0,
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
                    vval: TypvalVval::NULL_STRING,
                },
            }),
            additional_data: None,
        },
        ShadaEntryType::SDItemGlobalMark => ShadaEntry {
            entry_type: ShadaEntryType::SDItemGlobalMark,
            can_free_entry: false,
            timestamp: 0,
            data: ShadaEntryData::Filemark(ShadaFilemark {
                name: b'"',
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
                name: 0,
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
                name: b'"',
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
                name: 0,
                mark: DEFAULT_POS,
                fname: None,
            }),
            additional_data: None,
        },
    }
}

static DEFAULT_SHADA_FILE: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();

use crate::api::private::dispatch::{
    key_dict_shada_buflist_item_get_field as shada_buflist_item_get_field_cb,
    key_dict_shada_mark_get_field as shada_mark_get_field_cb,
    key_dict_shada_register_get_field as shada_register_get_field_cb,
    key_dict_shada_search_pat_get_field as shada_search_pat_get_field_cb,
};

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
fn file_fd(fp: &FileDescriptor) -> i32 {
    fp.fd
}

use crate::os::fileio::file_space;

#[inline]
const fn mark_local_index(name: u8) -> i32 {
    if name.is_ascii_lowercase() {
        (name - b'a') as i32
    } else if name == b'"' {
        NMARKS as i32
    } else if name == b'^' {
        (NMARKS + 1) as i32
    } else if name == b'.' {
        (NMARKS + 2) as i32
    } else {
        -1
    }
}

#[derive(Clone)]
struct PathKey(std::ffi::CString);

impl PartialEq for PathKey {
    fn eq(&self, other: &Self) -> bool {
        path_cmp(CASE_INSENSITIVE_FILENAME, &self.0, &other.0, MAXPATHL) == 0
    }
}

impl Eq for PathKey {}

impl std::hash::Hash for PathKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut bytes = self.0.to_bytes();
        let mut h: u32 = 0;
        if cfg!(target_os = "windows")
            && bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(1) == Some(&b':')
        {
            bytes = &bytes[2..];
        }
        let ic = CASE_INSENSITIVE_FILENAME;
        let mut idx = 0usize;
        let mut len: i32 = 0;
        while idx < bytes.len() {
            let cur = bytes[idx];
            let next = bytes.get(idx + 1).copied().unwrap_or(0);
            if vim_ispathsep_nocolon(i32::from(cur))
                && next == 0
                && idx > 0
                && !vim_ispathsep(i32::from(bytes[idx - 1]))
            {
                break;
            }
            let c = path_fold_char(ic, &bytes[idx..], &mut len);
            h = (h << 5).wrapping_sub(h).wrapping_add(c as u32);
            idx += (len as usize).max(1);
        }
        state.write_u32(h);
    }
}

#[inline]
fn hmll_init(hmll: &mut HMLList, size: usize) {
    *hmll = HMLList {
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
    };
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
    let skip_bytes = file_skip(sd_reader, offset);
    if skip_bytes < 0 {
        semsg!(
            gettext(c"E886: System error while skipping in ShaDa file: %s"),
            uv_strerror(skip_bytes as i32),
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

fn close_file(cookie: &mut FileDescriptor) -> i32 {
    let error = file_close(cookie, opt_vars().p_fs);
    if error != 0 {
        semsg!(
            gettext(c"E886: System error while closing ShaDa file: %s"),
            uv_strerror(error),
        );
    }
    error
}

fn shada_read_file(file: Option<&CStr>, flags: i32) -> bool {
    let Some(fname_cstr) = shada_filename(file) else {
        return false;
    };

    let mut sd_reader = FileDescriptor::new();
    let of_ret = file_open(
        &mut sd_reader,
        Some(fname_cstr.as_c_str()),
        K_FILE_READ_ONLY,
        0,
    );

    if opt_vars().p_verbose > 1 {
        verbose_enter();
        let empty = c"";
        smsg!(
            0,
            gettext(c"Reading ShaDa file \"%s\"%s%s%s%s"),
            &fname_cstr,
            if (flags & K_SHA_DA_WANT_INFO) != 0 {
                gettext(c" info")
            } else {
                empty
            },
            if (flags & K_SHA_DA_WANT_MARKS) != 0 {
                gettext(c" marks")
            } else {
                empty
            },
            if (flags & K_SHA_DA_GET_OLDFILES) != 0 {
                gettext(c" oldfiles")
            } else {
                empty
            },
            if of_ret != 0 {
                gettext(c" FAILED")
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
                &fname_cstr,
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
        let sep = if (history_type as i32) == HIST_SEARCH {
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
    new_hisidx: &mut i32,
    new_hisnum: &mut i32,
) {
    let mut count = 0usize;
    let hmll = &hms_p.hmll;
    let mut cur_idx = hmll.first;
    while let Some(idx) = cur_idx {
        let cur_entry = &hmll.entries[idx];
        let hi = cur_entry.data.data.history_item();
        hist_array[count] = Histentry {
            timestamp: cur_entry.data.timestamp,
            hisnum: (count as i32) + 1,
            hisstr: hi.string.clone(),
            sep: hi.sep,
            additional_data: cur_entry.data.additional_data.clone(),
        };
        count += 1;
        cur_idx = cur_entry.next;
    }
    *new_hisnum = count as i32;
    *new_hisidx = *new_hisnum - 1;
}

#[inline]
fn hms_dealloc(hms_p: &mut HistoryMergerState) {
    hmll_dealloc(&mut hms_p.hmll);
}

fn var_shada_iter(
    iter: Option<usize>,
    name: &mut Option<&'static CStr>,
    rettv: &mut TypVal,
    flavour: VarFlavour,
) -> Option<usize> {
    let globvarht = get_globvar_ht();
    let harray = &globvarht.ht_array;
    let hinum = harray.len();
    *name = None;
    let mut idx = if let Some(i) = iter {
        i
    } else {
        let mut i = 0;
        while i < hinum
            && (hashitem_empty(&harray[i])
                || (var_flavour(harray[i].key_cstr().unwrap_or(c"").to_bytes()) & flavour) == 0)
        {
            i += 1;
        }
        if i == hinum {
            return None;
        }
        i
    };
    let di = harray[idx].di_opt().unwrap();
    *name = Some(di.key_cstr());
    tv_copy(&di.di_tv, rettv);
    idx += 1;
    while idx < hinum {
        if !hashitem_empty(&harray[idx])
            && (var_flavour(harray[idx].key_cstr().unwrap_or(c"").to_bytes()) & flavour) != 0
        {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

fn find_buffer(
    fname_bufs: &mut std::collections::HashMap<Vec<u8>, crate::globals::BufPtr>,
    fname: &CStr,
) -> Option<&'static mut FileBuf> {
    let key = fname.to_bytes();
    if let Some(&buf_ptr) = fname_bufs.get(key) {
        return buf_ptr.opt_mut();
    }

    for buf in for_all_buffers_ptr() {
        if buf.b_ffname.is_some() && path_equal(fname, buf.ffname_cstr(), K_PATH_CMP_LITERAL) {
            fname_bufs.insert(key.to_vec(), crate::globals::BufPtr::from(&mut *buf));
            return Some(buf);
        }
    }
    fname_bufs.insert(key.to_vec(), crate::globals::BufPtr::NULL);
    None
}

#[inline]
const fn marks_equal(a: CursorPos, b: CursorPos) -> bool {
    a.lnum == b.lnum && a.col == b.col
}

fn marklist_insert<T>(jumps: &mut [T; JUMPLISTSIZE], jl_len: i32, mut i: i32) -> i32 {
    if i > 0 {
        if jl_len == (JUMPLISTSIZE as i32) {
            i -= 1;
            if i > 0 {
                jumps[..=i as usize].rotate_left(1);
            }
        } else if i != jl_len {
            jumps[i as usize..=jl_len as usize].rotate_right(1);
        }
    } else if i == 0 {
        if jl_len == (JUMPLISTSIZE as i32) {
            return -1;
        } else if jl_len > 0 {
            jumps[..=jl_len as usize].rotate_right(1);
        }
    }
    i
}

fn shada_read(sd_reader: &mut FileDescriptor, flags: i32) {
    let mut oldfiles_list = get_vim_var_list(VV_OLDFILES);
    let force = (flags & K_SHA_DA_FORCEIT) != 0;
    let get_old_files = (flags & K_SHA_DA_GET_OLDFILES) != 0
        && (force || tv_list_len(oldfiles_list.as_deref()) == 0);
    let want_marks = (flags & K_SHA_DA_WANT_MARKS) != 0;
    let no_opt = (flags & K_SHA_DA_NO_OPT) != 0;
    let argcount = crate::arglist::argcount();
    let srni_flags: u32 = (if (flags & K_SHA_DA_WANT_INFO) != 0 {
        K_SDREAD_UNDISABLEABLE_DATA
            | K_SDREAD_REGISTERS
            | K_SDREAD_GLOBAL_MARKS
            | if opt_vars().p_hi != 0 && (flags & K_SHA_DA_NO_HISTORY) == 0 {
                K_SDREAD_HISTORY
            } else {
                0
            }
            | if no_opt || find_shada_parameter(b'!' as i32).is_some() {
                K_SDREAD_VARIABLES
            } else {
                0
            }
            | if (no_opt || find_shada_parameter(b'%' as i32).is_some()) && argcount == 0 {
                K_SDREAD_BUFFER_LIST
            } else {
                0
            }
    } else {
        0
    }) | if want_marks && get_shada_parameter(b'\'' as i32) > 0 {
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
    let mut hms: [HistoryMergerState; HIST_COUNT] = std::array::from_fn(|_| HistoryMergerState {
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
    let mut cl_bufs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut fname_bufs: std::collections::HashMap<Vec<u8>, crate::globals::BufPtr> =
        std::collections::HashMap::new();
    let mut oldfiles_set: std::collections::HashSet<PathKey> = std::collections::HashSet::new();
    if get_old_files && (oldfiles_list.is_none() || force) {
        let new_list = tv_list_alloc(K_LIST_LEN_UNKNOWN_CONST);
        let new_list_id = new_list.lv_id;
        set_vim_var_list(VV_OLDFILES, Some(new_list));
        oldfiles_list = new_list_id.opt_mut();
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
                        dir: if sp.search_backward { b'?' } else { b'/' },
                        line: sp.has_line_offset,
                        end: sp.place_cursor_at_end,
                        off: sp.offset,
                    },
                    pat: (!sp.pat.data.is_null()).then(|| {
                        let s = crate::memory::bytes_to_cstring(sp.pat.as_bytes());
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
                let final_sub = regtilde(&mut sub_bytes, i32::from(opt_vars().p_magic), false)
                    .or_else(|| Some(crate::memory::cstring_until_nul(&sub_bytes)));
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
                    if reg.is_none_or(|r| r.with(|reg| reg.timestamp >= reg_ts)) {
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
                let gv = cur_entry.data.global_var_mut();
                let gv_val = std::mem::take(&mut gv.value);
                var_set_global(gv.name.as_ref().map_or(&[], |s| s.as_bytes()), gv_val);
                shada_free_shada_entry(&mut cur_entry);
            }
            ShadaEntryType::SDItemJump | ShadaEntryType::SDItemGlobalMark => {
                let fm_mark = cur_entry.data.filemark().mark;
                let fm_name = cur_entry.data.filemark().name;
                let fname_cstr = cur_entry.data.filemark().fname.as_deref().unwrap_or(c"");
                let buf_fnum = find_buffer(&mut fname_bufs, fname_cstr).map_or(0, |b| b.b_fnum);
                if buf_fnum != 0 {
                    cur_entry.data.filemark_mut().fname = None;
                }
                let fm = Xfmark {
                    fname: if buf_fnum == 0 {
                        cur_entry.data.filemark_mut().fname.take()
                    } else {
                        None
                    },
                    fmark: Fmark {
                        mark: fm_mark,
                        fnum: buf_fnum,
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
                                && (if buf_fnum == 0 {
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
                    if i > 0 && curwin_mut().w_jumplistlen == (JUMPLISTSIZE as i32) {
                        free_xfmark(&mut curwin_mut().w_jumplist[0]);
                    }
                    i = marklist_insert(
                        &mut curwin_mut().w_jumplist,
                        curwin_mut().w_jumplistlen,
                        i,
                    );
                    if i != -1 {
                        curwin_mut().w_jumplist[i as usize] = fm;
                        if curwin_mut().w_jumplistlen < (JUMPLISTSIZE as i32) {
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
                    let fname_opt = item.fname.as_deref();
                    let sfname = path_try_shorten_fname(fname_opt);
                    if let Some(buf) = buflist_new(fname_opt, sfname, 0, BLN_LISTED) {
                        buf.b_last_cursor = Fmark {
                            mark: item.pos,
                            fnum: 0,
                            timestamp: os_time(),
                            view: INIT_FMARKV,
                            additional_data: None,
                        };
                        let lnum = buf.b_last_cursor.mark.lnum;
                        let col = buf.b_last_cursor.mark.col;
                        buflist_setfpos(buf, curwin_ptr().opt_mut(), lnum, col, false);
                        buf.additional_data = item.additional_data.take();
                    }
                }
                shada_free_shada_entry(&mut cur_entry);
            }
            ShadaEntryType::SDItemChange | ShadaEntryType::SDItemLocalMark => {
                let fname_cstr = cur_entry.data.filemark().fname.as_deref().unwrap_or(c"");
                if get_old_files && oldfiles_set.insert(PathKey(fname_cstr.to_owned())) {
                    let ol = oldfiles_list.as_deref_mut().unwrap();
                    crate::eval::typval::tv_list_append_string(ol, Some(fname_cstr), -1);
                }
                if !want_marks {
                    shada_free_shada_entry(&mut cur_entry);
                    continue;
                }
                let fm_mark = cur_entry.data.filemark().mark;
                let fm_name = cur_entry.data.filemark().name;
                let Some(buf) = find_buffer(&mut fname_bufs, fname_cstr) else {
                    shada_free_shada_entry(&mut cur_entry);
                    continue;
                };
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
                    cl_bufs.insert(ptr::from_mut(buf) as usize);
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
                        if buf.b_changelistlen < (JUMPLISTSIZE as i32) {
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
            clr_history(i as i32);
            hist_with_array_mut(i as u8, |hist, new_hisidx, new_hisnum| {
                if !hist.is_empty() {
                    hms_to_he_array(hm, hist, new_hisidx, new_hisnum);
                }
            });
            hms_dealloc(hm);
        }
    }
    if !cl_bufs.is_empty() {
        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_mut(&*tp) {
                if cl_bufs.contains(&(wp.w_buffer.as_ptr() as usize)) {
                    wp.w_changelistidx = wp.buf_mut().b_changelistlen;
                }
            }
        }
    }
}

fn shada_get_default_file() -> &'static CStr {
    DEFAULT_SHADA_FILE.get_or_init(|| {
        let shada_dir = stdpaths_user_state_subpath(c"shada", 0, false);
        concat_fnames_cstr(&shada_dir, c"main.shada", true)
    })
}

fn shada_filename(file_arg: Option<&CStr>) -> Option<std::ffi::CString> {
    let shadafile_owned;
    let param_owned;
    let mut file = file_arg;
    if file.is_none_or(|s| s.is_empty()) {
        shadafile_owned = opt_vars().p_shadafile.to_owned();
        if !shadafile_owned.is_empty() {
            if shadafile_owned.to_bytes() != b"NONE" {
                file = Some(&shadafile_owned);
            } else {
                return None;
            }
        } else {
            param_owned = find_shada_parameter(b'n' as i32);
            file = param_owned.as_deref();
            if file.is_none_or(|s| s.is_empty()) {
                file = Some(shada_get_default_file());
            }
            let len = expand_env(file.unwrap_or(c""), g_st().name_buff_bytes_mut());
            return Some(
                std::ffi::CString::new(&g_st().name_buff_bytes_mut()[..len]).unwrap_or_default(),
            );
        }
    }
    Some(file.unwrap_or(c"").to_owned())
}

#[inline]
fn shada_check_buffer(packer: &mut PackerBuffer) {
    let remaining = packer.owned_buf.len().saturating_sub(packer.pos);
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
        mpack_raw(&ad.bytes, sbuf);
    }
}

fn shada_pack_entry(
    packer: &mut PackerBuffer,
    entry: &ShadaEntry,
    max_kbyte: usize,
) -> ShaDaWriteResult {
    let mut ret = ShaDaWriteResult::SDWriteFailed;
    let mut sbuf = packer_string_buffer();
    let def = sd_default_value(entry.entry_type);

    shada_check_buffer(&mut sbuf);
    match entry.entry_type {
        ShadaEntryType::SDItemMissing => std::process::abort(),
        ShadaEntryType::SDItemUnknown => {
            let ui = entry.data.unknown_item();
            mpack_raw(&ui.contents, &mut sbuf);
        }
        ShadaEntryType::SDItemHistoryEntry => {
            let hi = entry.data.history_item();
            let is_hist_search = (hi.histtype as i32) == HIST_SEARCH;
            let arr_size =
                2 + u32::from(is_hist_search) + additional_data_len(&entry.additional_data);
            mpack_array(&mut sbuf, arr_size);
            mpack_uint(&mut sbuf, u32::from(hi.histtype));
            mpack_bin(cstr_as_string(hi.string.as_deref()), &mut sbuf);
            if is_hist_search {
                mpack_uint(&mut sbuf, u32::from(hi.sep));
            }
            dump_additional_data(&entry.additional_data, &mut sbuf);
        }
        ShadaEntryType::SDItemVariable => {
            let gv_name_opt = entry.data.global_var().name.as_deref();
            let mut gv_val = entry.data.global_var().value.clone();
            let is_blob = gv_val.v_type == VarType::VarBlob;
            let arr_size = 2 + u32::from(is_blob) + additional_data_len(&entry.additional_data);
            mpack_array(&mut sbuf, arr_size);
            let varname = cstr_as_string(gv_name_opt);
            mpack_bin(varname, &mut sbuf);
            let mut vardesc = [0u8; 256];
            let prefix = b"variable g:";
            vardesc[..prefix.len()].copy_from_slice(prefix);
            let copy_len = varname.size.min(vardesc.len() - prefix.len() - 1);
            vardesc[prefix.len()..prefix.len() + copy_len]
                .copy_from_slice(&varname.as_bytes()[..copy_len]);
            vardesc[prefix.len() + copy_len] = 0;
            let vardesc_cstr =
                CStr::from_bytes_with_nul(&vardesc[..=prefix.len() + copy_len]).unwrap_or_default();
            if !encode_vim_to_msgpack(&mut sbuf, &mut gv_val, vardesc_cstr) {
                ret = ShaDaWriteResult::SDWriteIgnError;
                semsg!(
                    gettext(c"E574: Failed to write variable %s"),
                    gv_name_opt.unwrap_or(c"")
                );
                return ret;
            }
            if is_blob {
                mpack_check_buffer(&mut sbuf);
                mpack_integer(&mut sbuf, VAR_TYPE_BLOB);
            }
            dump_additional_data(&entry.additional_data, &mut sbuf);
        }
        ShadaEntryType::SDItemSubString => {
            let arr_size = 1 + additional_data_len(&entry.additional_data);
            mpack_array(&mut sbuf, arr_size);
            mpack_bin(
                cstr_as_string(entry.data.sub_string().sub.as_deref()),
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
            mpack_map(&mut sbuf, entry_map_size);
            mpack_str(static_cstr_as_string(c"sp"), &mut sbuf);
            mpack_bin(sp.pat, &mut sbuf);
            if sp.magic != dsp.magic {
                mpack_str(static_cstr_as_string(c"sm"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.magic);
            }
            if sp.is_last_used != dsp.is_last_used {
                mpack_str(static_cstr_as_string(c"su"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.is_last_used);
            }
            if sp.smartcase != dsp.smartcase {
                mpack_str(static_cstr_as_string(c"sc"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.smartcase);
            }
            if sp.has_line_offset != dsp.has_line_offset {
                mpack_str(static_cstr_as_string(c"sl"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.has_line_offset);
            }
            if sp.place_cursor_at_end != dsp.place_cursor_at_end {
                mpack_str(static_cstr_as_string(c"se"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.place_cursor_at_end);
            }
            if sp.is_substitute_pattern != dsp.is_substitute_pattern {
                mpack_str(static_cstr_as_string(c"ss"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.is_substitute_pattern);
            }
            if sp.highlighted != dsp.highlighted {
                mpack_str(static_cstr_as_string(c"sh"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.highlighted);
            }
            if sp.search_backward != dsp.search_backward {
                mpack_str(static_cstr_as_string(c"sb"), &mut sbuf);
                mpack_bool(&mut sbuf, !dsp.search_backward);
            }
            if sp.offset != dsp.offset {
                mpack_str(static_cstr_as_string(c"so"), &mut sbuf);
                mpack_integer(&mut sbuf, sp.offset);
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
            mpack_map(&mut sbuf, entry_map_size);
            mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
            mpack_bin(cstr_as_string(fm.fname.as_deref()), &mut sbuf);
            if fm.mark.lnum != dfm.mark.lnum {
                mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                mpack_integer(&mut sbuf, i64::from(fm.mark.lnum));
            }
            if fm.mark.col != dfm.mark.col {
                mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                mpack_integer(&mut sbuf, i64::from(fm.mark.col));
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
                mpack_uint(&mut sbuf, u32::from(fm.name));
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
            mpack_map(&mut sbuf, entry_map_size);
            mpack_str(static_cstr_as_string(c"rc"), &mut sbuf);
            mpack_array(&mut sbuf, r.contents.len() as u32);
            for &s in &r.contents {
                mpack_bin(s, &mut sbuf);
            }
            mpack_str(static_cstr_as_string(c"n"), &mut sbuf);
            mpack_uint(&mut sbuf, u32::from(r.name));
            if r.reg_type != dr.reg_type {
                mpack_str(static_cstr_as_string(c"rt"), &mut sbuf);
                mpack_uint(&mut sbuf, u32::from(r.reg_type));
            }
            if r.width != dr.width {
                mpack_str(static_cstr_as_string(c"rw"), &mut sbuf);
                mpack_uint64(&mut sbuf, r.width as u64);
            }
            if r.is_unnamed != dr.is_unnamed {
                mpack_str(static_cstr_as_string(c"ru"), &mut sbuf);
                mpack_bool(&mut sbuf, r.is_unnamed);
            }
            dump_additional_data(&entry.additional_data, &mut sbuf);
        }
        ShadaEntryType::SDItemBufferList => {
            let bl = entry.data.buffer_list();
            mpack_array(&mut sbuf, bl.buffers.len() as u32);
            for b in &bl.buffers {
                let entry_map_size = 1
                    + u32::from(b.pos.lnum != DEFAULT_POS.lnum)
                    + u32::from(b.pos.col != DEFAULT_POS.col)
                    + additional_data_len(&b.additional_data);
                mpack_map(&mut sbuf, entry_map_size);
                mpack_str(static_cstr_as_string(c"f"), &mut sbuf);
                mpack_bin(cstr_as_string(b.fname.as_deref()), &mut sbuf);
                if b.pos.lnum != 1 {
                    mpack_str(static_cstr_as_string(c"l"), &mut sbuf);
                    mpack_uint64(&mut sbuf, b.pos.lnum as u64);
                }
                if b.pos.col != 0 {
                    mpack_str(static_cstr_as_string(c"c"), &mut sbuf);
                    mpack_uint64(&mut sbuf, b.pos.col as u64);
                }
                dump_additional_data(&b.additional_data, &mut sbuf);
            }
        }
        ShadaEntryType::SDItemHeader => {
            let hdr = entry.data.header();
            mpack_map(&mut sbuf, hdr.size as u32);
            for i in 0..hdr.size {
                let kv = hdr[i];
                mpack_str(kv.key, &mut sbuf);
                match kv.value.obj_type {
                    ObjectType::ObjectTypeString => {
                        mpack_bin(kv.value.data.string(), &mut sbuf);
                    }
                    ObjectType::ObjectTypeInteger => {
                        mpack_integer(&mut sbuf, kv.value.data.integer());
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
            mpack_uint64(packer, entry.data.unknown_item().item_type);
        } else {
            mpack_uint64(packer, entry.entry_type as i32 as u64);
        }
        mpack_uint64(packer, entry.timestamp);
        if !packed.is_empty() {
            mpack_uint64(packer, packed.len() as u64);
            mpack_raw(&packed, packer);
        }
        if packer.anyint != 0 {
            return ret;
        }
    }
    ret = ShaDaWriteResult::SDWriteSuccessful;
    ret
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

fn shada_check_status(initial_fpos: u64, status: i32, remaining: usize) -> ShaDaReadResult {
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
    srni_flags: u32,
    max_kbyte: usize,
    wms: &mut WriteMergerState,
    packer: &mut PackerBuffer,
) -> ShaDaWriteResult {
    let mut ret = ShaDaWriteResult::SDWriteSuccessful;
    let mut entry = MISSING_ENTRY;
    let mut has_header = false;

    loop {
        let srni_ret = shada_read_next_item(sd_reader, &mut entry, srni_flags, max_kbyte);
        if srni_ret == ShaDaReadResult::SDReadStatusFinished {
            break;
        }
        match srni_ret {
            ShaDaReadResult::SDReadStatusSuccess => {}
            ShaDaReadResult::SDReadStatusFinished => std::process::abort(),
            ShaDaReadResult::SDReadStatusNotShaDa => {
                // Damage after a valid header (e.g. truncated by a crash), so we know it is a
                // (partial) ShaDa file. Keep the entries gathered so far and replace the file.
                return if has_header {
                    ret
                } else {
                    ShaDaWriteResult::SDWriteReadNotShada
                };
            }
            ShaDaReadResult::SDReadStatusReadError => {
                return ret;
            }
            ShaDaReadResult::SDReadStatusMalformed => continue,
        }

        let compare_with_entry = |wms_entry: &mut ShadaEntry, mut e: ShadaEntry| {
            if wms_entry.entry_type != ShadaEntryType::SDItemMissing {
                if wms_entry.timestamp >= e.timestamp {
                    shada_free_shada_entry(&mut e);
                    return;
                }
                shada_free_shada_entry(wms_entry);
            }
            *wms_entry = e;
        };

        match entry.entry_type {
            ShadaEntryType::SDItemMissing => {}
            ShadaEntryType::SDItemHeader => {
                has_header = true;
                shada_free_shada_entry(&mut entry);
            }
            ShadaEntryType::SDItemBufferList => {
                std::process::abort();
            }
            ShadaEntryType::SDItemUnknown => {
                ret = shada_pack_entry(packer, &entry, 0);
                shada_free_shada_entry(&mut entry);
            }
            ShadaEntryType::SDItemSearchPattern => {
                let is_sub = entry.data.search_pattern().is_substitute_pattern;
                let wms_pat = if is_sub {
                    &mut wms.sub_search_pattern
                } else {
                    &mut wms.search_pattern
                };
                if wms_pat.entry_type == ShadaEntryType::SDItemMissing
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
                compare_with_entry(&mut wms.replacement, e);
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
                let idx = op_reg_index(i32::from(reg_name));
                if idx < 0 {
                    ret = shada_pack_entry(packer, &entry, 0);
                    shada_free_shada_entry(&mut entry);
                    continue;
                }
                let u_idx = idx as usize;
                if wms.registers[u_idx].entry_type == ShadaEntryType::SDItemMissing
                    && let Some(reg) = op_reg_get(reg_name)
                    && reg.with(|r| reg_empty(r) && r.timestamp / NS_PER_SEC >= entry.timestamp)
                {
                    shada_free_shada_entry(&mut entry);
                    continue;
                }
                let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                compare_with_entry(&mut wms.registers[u_idx], e);
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
                if name.is_ascii_digit() {
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
                        &mut wms.global_marks[u_idx]
                    } else {
                        &mut wms.numbered_marks[u_idx - 26]
                    };
                    if mark.entry_type == ShadaEntryType::SDItemMissing
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
                let fname_cstr = entry.data.filemark().fname.as_deref().unwrap_or(c"");
                if shada_removable(fname_cstr) {
                    shada_free_shada_entry(&mut entry);
                    continue;
                }
                let fname_bytes = entry
                    .data
                    .filemark()
                    .fname
                    .as_ref()
                    .map_or_else(Vec::new, |s| s.to_bytes().to_vec());
                let filemarks = wms
                    .file_marks
                    .entry(fname_bytes)
                    .or_insert_with(FileMarks::new);
                if entry.timestamp > filemarks.greatest_timestamp {
                    filemarks.greatest_timestamp = entry.timestamp;
                }
                if entry.entry_type == ShadaEntryType::SDItemLocalMark {
                    let idx = mark_local_index(fm_name);
                    if idx < 0 {
                        let e = std::mem::replace(&mut entry, MISSING_ENTRY);
                        filemarks.additional_marks.push(e);
                    } else {
                        let wms_entry = &mut filemarks.marks[idx as usize];
                        let mut set_wms = true;
                        if wms_entry.entry_type != ShadaEntryType::SDItemMissing {
                            if wms_entry.timestamp >= entry.timestamp {
                                shada_free_shada_entry(&mut entry);
                                continue;
                            }
                            if wms_entry.can_free_entry {
                                shada_free_shada_entry(wms_entry);
                            }
                        } else {
                            for buf in for_all_buffers_mut() {
                                if buf.b_ffname.is_some()
                                    && path_equal(fname_cstr, buf.ffname_cstr(), K_PATH_CMP_LITERAL)
                                {
                                    let mut fm = Fmark::new();
                                    mark_get(
                                        buf,
                                        curwin_mut(),
                                        Some(&mut fm),
                                        K_MARK_BUF_LOCAL,
                                        i32::from(fm_name),
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
                    let mut i = filemarks.changes_size as i32;
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
                    i = marklist_insert(&mut filemarks.changes, filemarks.changes_size as i32, i);
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
                let mut i = wms.jumps_size as i32;
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
                i = marklist_insert(&mut wms.jumps, wms.jumps_size as i32, i);
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

#[inline]
fn ignore_buf(buf: Option<&FileBuf>, removable_bufs: &std::collections::HashSet<usize>) -> bool {
    let Some(buf) = buf else {
        return true;
    };
    buf.b_ffname.is_none()
        || (!buf.b_p_bl && buf.b_p_initialized)
        || bt_quickfix(Some(buf))
        || bt_terminal(Some(buf))
        || removable_bufs.contains(&(ptr::from_ref(buf) as usize))
}

#[inline]
fn shada_get_buflist(removable_bufs: &std::collections::HashSet<usize>) -> ShadaEntry {
    let max_bufs = get_shada_parameter(b'%' as i32);
    let mut buffers = Vec::new();
    for buf in for_all_buffers_ptr() {
        if !ignore_buf(Some(&*buf), removable_bufs) && buf.b_p_bl {
            if max_bufs >= 0 && buffers.len() >= (max_bufs as usize) {
                break;
            }
            buffers.push(BufferListBuffer {
                pos: buf.b_last_cursor.mark,
                fname: Some(buf.ffname_cstr().to_owned()),
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

#[inline]
fn add_search_pattern(
    ret_pse: &mut ShadaEntry,
    get_pattern: fn(&mut SearchPattern),
    is_substitute_pattern: bool,
    search_last_used: bool,
    search_highlighted: bool,
) {
    let mut pat = SearchPattern::new();
    get_pattern(&mut pat);
    if pat.pat.is_some() {
        let is_last_used = is_substitute_pattern ^ search_last_used;
        *ret_pse = ShadaEntry {
            can_free_entry: true,
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
                pat: crate::api::private::helpers::cstr_to_string(pat.pat.as_deref()),
                search_backward: !is_substitute_pattern && pat.off.dir == b'?',
            }),
            additional_data: pat.additional_data,
        };
    }
}

#[inline]
fn shada_initialize_registers(wms: &mut WriteMergerState, max_reg_lines: i32, scale_ts: bool) {
    let mut reg_iter: Option<usize> = None;
    let limit_reg_lines = max_reg_lines >= 0;
    loop {
        let mut got_one = false;
        reg_iter = with_global_reg_iter(reg_iter, |name, reg_ref, is_unnamed| {
            got_one = true;
            if limit_reg_lines && reg_ref.y_array.len() > (max_reg_lines as usize) {
                return;
            }
            let idx = op_reg_index(i32::from(name)) as usize;
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
        });
        if !got_one || reg_iter.is_none() {
            break;
        }
    }
}

#[inline]
fn replace_numbered_mark(wms: &mut WriteMergerState, idx: usize, entry: ShadaEntry) {
    shada_free_shada_entry(&mut wms.numbered_marks[EXTRA_MARKS - 1]);
    for i in idx..(EXTRA_MARKS - 1) {
        if wms.numbered_marks[i].entry_type == ShadaEntryType::SDItemGlobalMark {
            wms.numbered_marks[i].data.filemark_mut().name = b'0' + (i as u8) + 1;
        }
    }
    wms.numbered_marks[idx..].rotate_right(1);
    wms.numbered_marks[idx] = entry;
    wms.numbered_marks[idx].data.filemark_mut().name = b'0' + (idx as u8);
}

#[inline]
fn find_removable_bufs(removable_bufs: &mut std::collections::HashSet<usize>) {
    for buf in for_all_buffers_ptr() {
        if buf.b_ffname.is_some() && shada_removable(buf.ffname_cstr()) {
            removable_bufs.insert(ptr::from_mut(buf) as usize);
        }
    }
}

const fn hist_type2char(hist_type: i32) -> i32 {
    match hist_type {
        HIST_CMD => b':' as i32,
        HIST_SEARCH => b'/' as i32,
        HIST_EXPR => b'=' as i32,
        HIST_INPUT => b'@' as i32,
        HIST_DEBUG => b'>' as i32,
        _ => 0,
    }
}

pub(crate) fn flush_file_buffer(buffer: &mut PackerBuffer) {
    let mut owned_buf = std::mem::take(&mut buffer.owned_buf);
    let pos = buffer.pos;
    let fd = buffer.anydata.as_file();
    std::mem::swap(&mut fd.buffer, &mut owned_buf);
    fd.write_pos = pos;
    let flush_res = i64::from(file_flush(fd));
    let new_pos = fd.write_pos;
    std::mem::swap(&mut fd.buffer, &mut owned_buf);
    buffer.owned_buf = owned_buf;
    buffer.pos = new_pos;
    buffer.anyint = flush_res;
}

fn packer_buffer_for_file(file: &mut FileDescriptor) -> PackerBuffer {
    if file_space(&*file) < SHADA_MPACK_FREE_SPACE {
        file_flush(file);
    }
    let pos = file.write_pos;
    let owned_buf = std::mem::take(&mut file.buffer);
    let fd = std::mem::take(file);
    PackerBuffer {
        pos,
        owned_buf,
        anydata: crate::msgpack_rpc::packer::PackerData::File(Box::new(fd)),
        ..PackerBuffer::new()
    }
}

fn shada_write(
    sd_writer: &mut FileDescriptor,
    sd_reader: Option<&mut FileDescriptor>,
) -> ShaDaWriteResult {
    let mut ret = ShaDaWriteResult::SDWriteSuccessful;
    let mut max_kbyte_i = get_shada_parameter(b's' as i32);
    if max_kbyte_i < 0 {
        max_kbyte_i = 10;
    }
    if max_kbyte_i == 0 {
        return ret;
    }

    let mut wms = WriteMergerState::new();
    let mut dump_one_history = [false; HIST_COUNT];
    let dump_global_vars = find_shada_parameter(b'!' as i32).is_some();
    let mut max_reg_lines = get_shada_parameter(b'<' as i32);
    if max_reg_lines < 0 {
        max_reg_lines = get_shada_parameter(b'"' as i32);
    }
    let dump_registers = max_reg_lines != 0;
    let mut removable_bufs: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let max_kbyte = max_kbyte_i as usize;
    let num_marked_files = get_shada_parameter(b'\'' as i32) as usize;
    let dump_global_marks = get_shada_parameter(b'f' as i32) != 0;
    let mut dump_history = false;

    for (i, dump_one) in dump_one_history.iter_mut().enumerate().take(HIST_COUNT) {
        let mut num_saved = get_shada_parameter(hist_type2char(i as i32));
        if num_saved == -1 {
            num_saved = opt_vars().p_hi as i32;
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

    let srni_flags: u32 = K_SDREAD_HEADER
        | K_SDREAD_UNDISABLEABLE_DATA
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
                data: ObjectData::String(cstr_as_string(Some(long_version()))),
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
                data: ObjectData::String(cstr_as_string(Some(opt_vars().p_enc.as_cstr()))),
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
                data: ShadaEntryData::Header(Dict::from_slice(&mut header_items[..5])),
                additional_data: None,
            },
            0,
        ) == ShaDaWriteResult::SDWriteFailed
        {
            break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
        }

        if find_shada_parameter(b'%' as i32).is_some() {
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
                let mut name: Option<&'static CStr> = None;
                var_iter = var_shada_iter(var_iter, &mut name, &mut vartv, VAR_FLAVOUR_SHADA);
                let Some(name) = name else {
                    break;
                };
                let mut skip = false;
                match vartv.v_type {
                    VarType::VarFunc | VarType::VarPartial => {
                        tv_clear(&mut vartv);
                        skip = true;
                    }
                    VarType::VarDict => {
                        let di = vartv.vval.v_dict().borrow_mut();
                        let copy_id = get_copy_id();
                        if !set_ref_in_ht(Some(&mut di.dv_hashtab), copy_id, None)
                            && copy_id == di.dv_copy_id
                        {
                            tv_clear(&mut vartv);
                            skip = true;
                        }
                    }
                    VarType::VarList => {
                        let l = vartv.vval.v_list();
                        let copy_id = get_copy_id();
                        if !set_ref_in_list_items(l.opt_mut(), copy_id, None)
                            && copy_id == l.lv_copy_id
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
                    let mut entry = ShadaEntry {
                        entry_type: ShadaEntryType::SDItemVariable,
                        can_free_entry: false,
                        timestamp: cur_timestamp,
                        data: ShadaEntryData::GlobalVar(ShadaGlobalVar {
                            name: Some(name.to_owned()),
                            value: tgttv,
                        }),
                        additional_data: None,
                    };
                    let spe_ret = shada_pack_entry(&mut packer, &entry, max_kbyte);
                    tv_clear(&mut entry.data.global_var_mut().value);
                    if spe_ret == ShaDaWriteResult::SDWriteFailed {
                        tv_clear(&mut vartv);
                        break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                    }
                    tv_clear(&mut vartv);
                    if spe_ret == ShaDaWriteResult::SDWriteSuccessful {
                        wms.dumped_variables.insert(name.to_bytes().to_vec());
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
            let search_highlighted =
                !(g_st().search_global.no_hlsearch || find_shada_parameter(b'h' as i32).is_some());
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
                let mut name: u8 = 0;
                let mut fm = Xfmark::new();
                global_mark_iter = mark_global_iter(global_mark_iter, &mut name, &mut fm);
                if name == 0 {
                    break;
                }
                let fname: std::ffi::CString = if fm.fmark.fnum == 0 {
                    let Some(f) = fm.fname.take() else {
                        if global_mark_iter.is_none() {
                            break;
                        }
                        continue;
                    };
                    if shada_removable(f.as_c_str()) {
                        if global_mark_iter.is_none() {
                            break;
                        }
                        continue;
                    }
                    f
                } else {
                    let Some(buf) = buflist_findnr(fm.fmark.fnum) else {
                        if global_mark_iter.is_none() {
                            break;
                        }
                        continue;
                    };
                    if buf.b_ffname.is_none()
                        || removable_bufs.contains(&(ptr::from_ref(&*buf) as usize))
                    {
                        if global_mark_iter.is_none() {
                            break;
                        }
                        continue;
                    }
                    buf.ffname_cstr().to_owned()
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
                if name.is_ascii_digit() {
                    replace_numbered_mark(&mut wms, digit_mark_idx, entry);
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
            shada_initialize_registers(&mut wms, max_reg_lines, true);
        }

        if num_marked_files > 0 {
            for buf in for_all_buffers_ptr() {
                if !ignore_buf(Some(&*buf), &removable_bufs) {
                    let mut local_marks_iter: Option<u8> = None;
                    let fname_cstr = buf.ffname_cstr().to_owned();
                    let fname_bytes = fname_cstr.to_bytes().to_vec();
                    let filemarks = wms
                        .file_marks
                        .entry(fname_bytes)
                        .or_insert_with(FileMarks::new);
                    loop {
                        let mut fm = Fmark::new();
                        let mut name: u8 = 0;
                        local_marks_iter =
                            mark_buffer_iter(local_marks_iter, &*buf, &mut name, &mut fm);
                        if name == 0 {
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
                                name: 0,
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
            let srww_ret =
                shada_read_when_writing(sd_reader, srni_flags, max_kbyte, &mut wms, &mut packer);
            if srww_ret != ShaDaWriteResult::SDWriteSuccessful {
                ret = srww_ret;
            }
        }

        if dump_global_marks
            && !ignore_buf(g_st().curbuf.opt(), &removable_bufs)
            && curwin_mut().w_cursor.lnum != 0
        {
            replace_numbered_mark(
                &mut wms,
                0,
                ShadaEntry {
                    can_free_entry: false,
                    entry_type: ShadaEntryType::SDItemGlobalMark,
                    timestamp: os_time(),
                    data: ShadaEntryData::Filemark(ShadaFilemark {
                        mark: curwin_mut().w_cursor,
                        name: b'0',
                        fname: Some(curbuf_mut().ffname_cstr().to_owned()),
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

        let mut all_file_markss: Vec<&mut FileMarks> = wms.file_marks.values_mut().collect();
        all_file_markss.sort_by(|a, b| {
            let ta = a.greatest_timestamp;
            let tb = b.greatest_timestamp;
            if ta == tb {
                Ordering::Equal
            } else if ta > tb {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        });
        let file_markss_to_dump = num_marked_files.min(all_file_markss.len());
        for fm in &mut all_file_markss[..file_markss_to_dump] {
            for entry in &mut fm.marks {
                let entry = std::mem::replace(entry, MISSING_ENTRY);
                if entry.entry_type != ShadaEntryType::SDItemMissing
                    && shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                        == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for j in 0..fm.changes_size {
                let entry = std::mem::replace(&mut fm.changes[j], MISSING_ENTRY);
                if shada_pack_pfreed_entry(&mut packer, entry, max_kbyte)
                    == ShaDaWriteResult::SDWriteFailed
                {
                    break 'shada_write_exit ShaDaWriteResult::SDWriteFailed;
                }
            }
            for mut add_entry in fm.additional_marks.drain(..) {
                if shada_pack_entry(&mut packer, &add_entry, 0) == ShaDaWriteResult::SDWriteFailed {
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
    *sd_writer = packer.anydata.take_file();
    std::mem::swap(&mut sd_writer.buffer, &mut packer.owned_buf);
    if packer.anyint != 0 {
        ret = ShaDaWriteResult::SDWriteFailed;
    }
    ret
}

/// Creates the first available `{fname}.tmp.a…z` file. If all exist (old failed writes), deletes
/// the oldest and reuses its name.
fn shada_open_tmp(
    fname_bytes: &[u8],
    sd_writer: &mut FileDescriptor,
    perm: i32,
) -> Option<Vec<u8>> {
    let fname = crate::memory::bytes_as_cstr(fname_bytes);
    let tn = modname(Some(fname), c".tmp.a", false)?;
    let mut tempname_bytes = tn.into_bytes_with_nul();
    let flags = K_FILE_CREATE_ONLY | K_FILE_NO_SYMLINK;
    let last_idx = tempname_bytes.len() - 2;
    let mut oldest: u8 = 0;
    let mut oldest_mtime: std::ffi::c_long = 0;
    for c in b'a'..=b'z' {
        tempname_bytes[last_idx] = c;
        let error = file_open(
            sd_writer,
            CStr::from_bytes_with_nul(&tempname_bytes).ok(),
            flags,
            perm,
        );
        if error == 0 {
            return Some(tempname_bytes);
        } else if error != UV_EEXIST && error != UV_ELOOP {
            semsg!(
                gettext(
                    c"E886: System error while opening temporary ShaDa file %s for writing: %s",
                ),
                CStr::from_bytes_with_nul(&tempname_bytes).unwrap_or_default(),
                uv_strerror(error),
            );
            return None;
        }
        let mut info = FileInfo::new();
        if os_fileinfo_link(crate::memory::bytes_as_cstr(&tempname_bytes), &mut info)
            && (oldest == 0 || info.stat.st_mtim.tv_sec < oldest_mtime)
        {
            oldest = c;
            oldest_mtime = info.stat.st_mtim.tv_sec;
        }
    }
    if oldest != 0 {
        tempname_bytes[last_idx] = oldest;
        if os_remove(crate::memory::bytes_as_cstr(&tempname_bytes)) == 0
            && file_open(
                sd_writer,
                CStr::from_bytes_with_nul(&tempname_bytes).ok(),
                flags,
                perm,
            ) == 0
        {
            return Some(tempname_bytes);
        }
    }
    semsg!(
        gettext(c"E138: All %s.tmp.X files exist, cannot write ShaDa file!"),
        CStr::from_bytes_with_nul(fname_bytes).unwrap_or_default(),
    );
    None
}

/// Write ShaDa file to a given location (`shada_write_file`).
pub fn shada_write_file(file: Option<&CStr>, nomerge: bool) -> bool {
    let Some(fname_cstr) = shada_filename(file) else {
        return false;
    };
    let fname_bytes = fname_cstr.into_bytes_with_nul();
    let fname_cstr = CStr::from_bytes_with_nul(&fname_bytes).unwrap_or_default();
    let fname_opt = Some(fname_cstr);

    let mut sd_writer = FileDescriptor::new();
    let mut sd_reader = FileDescriptor::new();
    let mut did_open_reader = false;

    if !nomerge {
        let error = file_open(&mut sd_reader, fname_opt, K_FILE_READ_ONLY, 0);
        if error == 0 {
            did_open_reader = true;
        } else if error != UV_ENOENT {
            semsg!(
                gettext(
                    c"E886: System error while opening ShaDa file %s for reading to merge before writing it: %s",
                ),
                fname_cstr,
                uv_strerror(error),
            );
        }
    }

    let mut perm = os_getperm(fname_cstr);
    perm = if perm >= 0 {
        (perm & 0o777) | 0o600
    } else {
        0o600
    };
    let tempname_opt = if os_file_mkdir(fname_cstr, 0o700) == 0 {
        shada_open_tmp(&fname_bytes, &mut sd_writer, perm)
    } else {
        None
    };
    let Some(tempname_bytes) = tempname_opt else {
        if did_open_reader {
            close_file(&mut sd_reader);
        }
        return false;
    };

    if opt_vars().p_verbose > 1 {
        verbose_enter();
        smsg!(0, gettext(c"Writing ShaDa file \"%s\""), fname_cstr);
        verbose_leave();
    }

    let sw_ret = shada_write(&mut sd_writer, did_open_reader.then_some(&mut sd_reader));
    assert!(sw_ret != ShaDaWriteResult::SDWriteIgnError);
    if did_open_reader {
        close_file(&mut sd_reader);
    }
    let tempname = CStr::from_bytes_with_nul(&tempname_bytes).unwrap_or_default();
    let fname = fname_cstr;
    let mut did_remove = false;
    if sw_ret == ShaDaWriteResult::SDWriteSuccessful {
        let mut can_rename = true;
        let mut old_info = FileInfo::new();
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

        if os_fileinfo(fname, &mut old_info) {
            if (old_info.stat.st_mode & S_IFMT) == S_IFDIR || unix_not_writable(&old_info) {
                semsg!(gettext(c"E137: ShaDa file is not writable: %s"), fname,);
                can_rename = false;
            } else {
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
                            can_rename = false;
                        }
                    }
                }
            }
        }
        if can_rename && close_file(&mut sd_writer) == 0 {
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
    } else if sw_ret == ShaDaWriteResult::SDWriteReadNotShada {
        semsg!(
            gettext(c"E136: Did not rename %s because %s does not look like a ShaDa file",),
            tempname,
            fname,
        );
    } else {
        semsg!(
            gettext(c"E136: Did not rename %s to %s because there were errors during writing it",),
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
    close_file(&mut sd_writer);

    true
}

/// Read marks information from ShaDa file (`shada_read_marks`).
pub fn shada_read_marks() -> bool {
    shada_read_file(None, K_SHA_DA_WANT_MARKS)
}

/// Read all information from ShaDa file (`shada_read_everything`).
pub fn shada_read_everything(fname: Option<&CStr>, forceit: bool, missing_ok: bool) -> bool {
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

fn fread_len(sd_reader: &mut FileDescriptor, buffer: &mut [u8]) -> ShaDaReadResult {
    let length = buffer.len();
    let read_bytes = file_read(sd_reader, buffer);
    if read_bytes < 0 {
        semsg!(
            gettext(c"E886: System error while reading ShaDa file: %s"),
            uv_strerror(read_bytes as i32),
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

fn msgpack_read_uint64(
    sd_reader: &mut FileDescriptor,
    allow_eof: bool,
    result: &mut u64,
) -> ShaDaReadResult {
    let fpos = sd_reader.bytes_read;
    let mut ret = [0u8; 1];
    let read_bytes = file_read(sd_reader, &mut ret);
    let ret = ret[0];
    if read_bytes < 0 {
        semsg!(
            gettext(c"E886: System error while reading integer from ShaDa file: %s"),
            uv_strerror(read_bytes as i32),
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
        let fl_ret = fread_len(sd_reader, &mut buf[8 - length..]);
        if fl_ret != ShaDaReadResult::SDReadStatusSuccess {
            return fl_ret;
        }
        *result = u64::from_be_bytes(buf);
    }
    ShaDaReadResult::SDReadStatusSuccess
}

fn shada_read_next_item(
    sd_reader: &mut FileDescriptor,
    entry: &mut ShadaEntry,
    flags: u32,
    max_kbyte: usize,
) -> ShaDaReadResult {
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
            if initial_fpos == 0 && (type_u64 == u64::from(b'\n') || type_u64 > SHADA_LAST_ENTRY) {
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
        let mut owned_buf: Vec<u8> = Vec::new();
        let buf_slice: &[u8] = if sd_reader.write_pos.saturating_sub(sd_reader.read_pos) >= length {
            file_try_read_buffered(sd_reader, length).unwrap()
        } else {
            owned_buf = vec![0u8; length];
            let fl_ret = fread_len(sd_reader, &mut owned_buf);
            if fl_ret != ShaDaReadResult::SDReadStatusSuccess {
                return fl_ret;
            }
            &owned_buf
        };

        let mut read_buf: &[u8] = buf_slice;

        if verify_but_ignore {
            let status = unpack_skip(&mut read_buf);
            let spm_ret = shada_check_status(parse_pos, status, read_buf.len());
            if spm_ret != ShaDaReadResult::SDReadStatusSuccess {
                return spm_ret;
            }
            continue;
        }

        if type_u64 > SHADA_LAST_ENTRY {
            entry.entry_type = ShadaEntryType::SDItemUnknown;
            if initial_fpos == 0 {
                let status = unpack_skip(&mut read_buf);
                let spm_ret = shada_check_status(parse_pos, status, read_buf.len());
                if spm_ret != ShaDaReadResult::SDReadStatusSuccess {
                    entry.entry_type = ShadaEntryType::SDItemMissing;
                    return spm_ret;
                }
            }
            let contents = if !owned_buf.is_empty() {
                owned_buf
            } else {
                buf_slice.to_vec()
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
                ShadaEntryType::SDItemHeader => {
                    if unpack_skip(&mut read_buf) != MPACK_OK {
                        break 'parse_item false;
                    }
                }
                ShadaEntryType::SDItemSearchPattern => {
                    let it = entry.data.search_pattern_mut();
                    if !unpack_keydict(
                        it,
                        Some(shada_search_pat_get_field_cb),
                        &mut ad,
                        &mut read_buf,
                        &mut error_alloc,
                    ) {
                        semsg!(
                            gettext(
                                c"E575: Error while reading ShaDa file: search pattern entry at position %llu %s",
                            ),
                            initial_fpos,
                            error_alloc.as_deref().unwrap_or(c""),
                        );
                        it.pat = NvimString::new();
                        break 'parse_item false;
                    }
                    if (it.is_set_shada_search_pat & (1u64 << KEYSET_OPTIDX_SHADA_SEARCH_PAT_SP))
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
                        &mut it,
                        Some(shada_mark_get_field_cb),
                        &mut ad,
                        &mut read_buf,
                        &mut error_alloc,
                    ) {
                        semsg!(
                            gettext(
                                c"E575: Error while reading ShaDa file: mark entry at position %llu %s",
                            ),
                            initial_fpos,
                            error_alloc.as_deref().unwrap_or(c""),
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
                        fm.name = it.n as u8;
                    }
                    if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_L)) != 0 {
                        fm.mark.lnum = it.l as Linenr;
                    }
                    if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_C)) != 0 {
                        fm.mark.col = it.c as Colnr;
                    }
                    if (it.is_set_shada_mark & (1u64 << KEYSET_OPTIDX_SHADA_MARK_F)) != 0 {
                        let mut bytes = it.f.as_bytes().to_vec();
                        bytes.push(0);
                        #[cfg(windows)]
                        path_to_slash(&mut bytes);
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
                        &mut it,
                        Some(shada_register_get_field_cb),
                        &mut ad,
                        &mut read_buf,
                        &mut error_alloc,
                    ) {
                        semsg!(
                            gettext(
                                c"E575: Error while reading ShaDa file: register entry at position %llu %s",
                            ),
                            initial_fpos,
                            error_alloc.as_deref().unwrap_or(c""),
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

                    if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RU)) != 0 {
                        reg.is_unnamed = it.ru;
                    }
                    if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RT)) != 0 {
                        reg.reg_type = it.rt as u8;
                    }
                    if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_N)) != 0 {
                        reg.name = it.n as u8;
                    }
                    if (it.is_set_shada_register & (1u64 << KEYSET_OPTIDX_SHADA_REGISTER_RW)) != 0 {
                        reg.width = it.rw as usize;
                    }
                }
                ShadaEntryType::SDItemHistoryEntry => {
                    let len = unpack_array(&mut read_buf);
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
                    if !unpack_integer(&mut read_buf, &mut hist_type) {
                        semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history type type",
                                ),
                                initial_fpos,
                            );
                        break 'parse_item false;
                    }
                    let item = unpack_string(&mut read_buf);
                    if item.data.is_null() {
                        semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: history entry at position %llu has wrong history string type",
                                ),
                                initial_fpos,
                            );
                        break 'parse_item false;
                    }
                    let item_slice = item.as_bytes();
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
                    let is_hist_search = (hi.histtype as i32) == HIST_SEARCH;
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
                        if !unpack_integer(&mut read_buf, &mut sep_type) {
                            semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: search history entry at position %llu has wrong history separator type",
                                    ),
                                    initial_fpos,
                                );
                            break 'parse_item false;
                        }
                        hi.sep = sep_type as u8;
                    }
                    hi.string = Some(std::ffi::CString::new(item.as_bytes()).unwrap_or_default());
                    read_additional_array_elements = (len as u32) - (2 + u32::from(is_hist_search));
                }
                ShadaEntryType::SDItemVariable => {
                    let len = unpack_array(&mut read_buf);
                    if len < 2 {
                        semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: variable entry at position %llu is not an array with enough elements",
                                ),
                                initial_fpos,
                            );
                        break 'parse_item false;
                    }
                    let name = unpack_string(&mut read_buf);
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
                    gv.name = Some(std::ffi::CString::new(name.as_bytes()).unwrap_or_default());
                    let binval = unpack_string(&mut read_buf);
                    let mut is_blob = false;
                    if !binval.data.is_null() {
                        if len > 2 {
                            let mut vtype: Integer = 0;
                            if !unpack_integer(&mut read_buf, &mut vtype) || vtype != VAR_TYPE_BLOB
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
                        gv.value = decode_string(binval.as_bytes(), is_blob);
                    } else {
                        let status = unpack_typval(&mut read_buf, &mut gv.value);
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
                    let len = unpack_array(&mut read_buf);
                    if len < 1 {
                        semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu is not an array with enough elements",
                                ),
                                initial_fpos,
                            );
                        break 'parse_item false;
                    }
                    let sub = unpack_string(&mut read_buf);
                    if sub.data.is_null() {
                        semsg!(
                                gettext(
                                    c"E575: Error while reading ShaDa file: sub string entry at position %llu has wrong sub string type",
                                ),
                                initial_fpos,
                            );
                        break 'parse_item false;
                    }
                    entry.data.sub_string_mut().sub =
                        Some(std::ffi::CString::new(sub.as_bytes()).unwrap_or_default());
                    read_additional_array_elements = (len as u32) - 1;
                }
                ShadaEntryType::SDItemBufferList => {
                    let len = unpack_array(&mut read_buf);
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
                                &mut it,
                                Some(shada_buflist_item_get_field_cb),
                                &mut it_ad,
                                &mut read_buf,
                                &mut error_alloc,
                            ) {
                                semsg!(
                                    gettext(
                                        c"E575: Error while reading ShaDa file: buffer list at position %llu contains entry that %s",
                                    ),
                                    initial_fpos,
                                    error_alloc.as_deref().unwrap_or(c""),
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
                                let mut bytes = it.f.as_bytes().to_vec();
                                bytes.push(0);
                                #[cfg(windows)]
                                path_to_slash(&mut bytes);
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
                let item_start = read_buf;
                let status = unpack_skip(&mut read_buf);
                if status != 0 {
                    break 'parse_item false;
                }
                let skipped_len = item_start.len() - read_buf.len();
                push_additional_data(&mut ad, &item_start[..skipped_len]);
            }

            if !read_buf.is_empty() {
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

fn shada_removable(name: &CStr) -> bool {
    let mut part = [0u8; MAXPATHL + 1];
    let mut retval = false;
    let new_name = home_replace_save(None, Some(name));
    let p_shada = opt_vars().p_shada.to_owned();
    let mut p = p_shada.as_bytes();
    while !p.is_empty() {
        copy_option_part(&mut p, &mut part, c", ");
        if part[0] == b'r' {
            let n = home_replace(
                None,
                Some(crate::memory::bytes_as_cstr(&part[1..])),
                g_st().name_buff_bytes_mut(),
                true,
            );
            if mb_strnicmp(g_st().name_buff_cstr().to_bytes(), new_name.as_bytes(), n) == 0 {
                retval = true;
                break;
            }
        }
    }
    retval
}

#[inline]
fn shada_init_jumps(
    jumps: &mut [ShadaEntry; JUMPLISTSIZE],
    removable_bufs: &std::collections::HashSet<usize>,
) -> usize {
    let mut jumps_size = 0usize;
    let mut jump_iter: Option<usize> = None;
    setpcmark();
    cleanup_jumplist(crate::globals::curwin_handle(), false);
    loop {
        let mut fm = Xfmark::new();
        jump_iter = mark_jumplist_iter(jump_iter, curwin_mut(), &mut fm);
        if fm.fmark.mark.lnum == 0 {
            siemsg!(
                c"ShaDa: mark lnum zero (ji:%zu, js:%p, len:%i)",
                jump_iter.unwrap_or(0),
                curwin_mut().w_jumplist.as_ptr().addr(),
                curwin_mut().w_jumplistlen,
            );
            if jump_iter.is_none() {
                break;
            }
            continue;
        }
        let buf = if fm.fmark.fnum == 0 {
            None
        } else {
            buflist_findnr(fm.fmark.fnum).map(|b| &*b)
        };
        let skip = if buf.is_some() {
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
        } else if let Some(buf) = buf
            && buf.b_ffname.is_some()
        {
            Some(buf.ffname_cstr().to_owned())
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
                name: 0,
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
    let mut removable_bufs: std::collections::HashSet<usize> = std::collections::HashSet::new();
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
    let mut removable_bufs: std::collections::HashSet<usize> = std::collections::HashSet::new();
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
    let mut packer = packer_string_buffer();
    let mut var_iter: Option<usize> = None;
    let cur_timestamp = os_time();
    loop {
        let mut vartv = TypVal::new();
        let mut name: Option<&'static CStr> = None;
        var_iter = var_shada_iter(
            var_iter,
            &mut name,
            &mut vartv,
            VAR_FLAVOUR_DEFAULT | VAR_FLAVOUR_SESSION | VAR_FLAVOUR_SHADA,
        );
        let Some(name) = name else {
            break;
        };
        if vartv.v_type != VarType::VarFunc && vartv.v_type != VarType::VarPartial {
            let mut tgttv = TypVal::new();
            tv_copy(&vartv, &mut tgttv);
            let mut entry = ShadaEntry {
                entry_type: ShadaEntryType::SDItemVariable,
                can_free_entry: false,
                timestamp: cur_timestamp,
                data: ShadaEntryData::GlobalVar(ShadaGlobalVar {
                    name: Some(name.to_owned()),
                    value: tgttv,
                }),
                additional_data: None,
            };
            let r = shada_pack_entry(&mut packer, &entry, 0);
            if r == ShaDaWriteResult::SDWriteFailed {
                std::process::abort();
            }
            tv_clear(&mut entry.data.global_var_mut().value);
        }
        tv_clear(&mut vartv);
        if var_iter.is_none() {
            break;
        }
    }
    packer_take_string(&mut packer)
}

/// Read ShaDa from byte slice (`shada_read_string`).
pub fn shada_read_string(bytes: &[u8], flags: i32) {
    if bytes.is_empty() {
        return;
    }
    let mut sd_reader = FileDescriptor::new();
    file_open_buffer(&mut sd_reader, bytes);
    shada_read(&mut sd_reader, flags);
    close_file(&mut sd_reader);
}

/// Find the parameter represented by the given character in 'shada' (`get_shada_parameter`).
pub fn get_shada_parameter(param_type: i32) -> i32 {
    if let Some(p) = find_shada_parameter(param_type) {
        let bytes = p.to_bytes();
        if bytes.first().is_some_and(u8::is_ascii_digit) {
            let mut val: i32 = 0;
            for &b in bytes {
                if !b.is_ascii_digit() {
                    break;
                }
                val = val.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
            }
            return val;
        }
    }
    -1
}

/// Find the parameter represented by the given character in 'shada' (`find_shada_parameter`).
pub fn find_shada_parameter(param_type: i32) -> Option<std::ffi::CString> {
    let p_shada = opt_vars().p_shada.to_owned();
    let mut p = p_shada.as_c_str();
    while !p.to_bytes().is_empty() {
        if i32::from(p.to_bytes()[0]) == param_type {
            return Some(p[1..].to_owned());
        }
        if p.to_bytes()[0] == b'n' {
            break;
        }
        let found = vim_strchr(p, b',' as i32)?;
        p = &found[1..];
    }
    None
}

/// Read marks for the current buffer from the ShaDa file (`check_marks_read`).
pub fn check_marks_read() {
    if !curbuf_mut().b_marks_read
        && get_shada_parameter(b'\'' as i32) > 0
        && curbuf_mut().b_ffname.is_some()
    {
        shada_read_marks();
    }
    curbuf_mut().b_marks_read = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_indices_and_hist_type2char_map_correctly() {
        assert_eq!(mark_global_index(b'A'), 0);
        assert_eq!(mark_global_index(b'Z'), 25);
        assert_eq!(mark_global_index(b'0'), 26);
        assert_eq!(mark_global_index(b'9'), 35);
        assert_eq!(mark_global_index(b'a'), -1);

        assert_eq!(mark_local_index(b'a'), 0);
        assert_eq!(mark_local_index(b'z'), 25);
        assert_eq!(mark_local_index(b'"'), 26);
        assert_eq!(mark_local_index(b'^'), 27);
        assert_eq!(mark_local_index(b'.'), 28);
        assert_eq!(mark_local_index(b'A'), -1);

        assert_eq!(hist_type2char(HIST_CMD), b':' as i32);
        assert_eq!(hist_type2char(HIST_SEARCH), b'/' as i32);
        assert_eq!(hist_type2char(HIST_EXPR), b'=' as i32);
        assert_eq!(hist_type2char(HIST_INPUT), b'@' as i32);
        assert_eq!(hist_type2char(HIST_DEBUG), b'>' as i32);
    }
}
