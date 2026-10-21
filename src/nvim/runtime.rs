//! Management of runtime files (including packages) (`src/nvim/runtime.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::command::exarg_T;
use crate::api::options::{IOSIZE, optset_T};
use crate::api::private::converter::{sctx_T, ufunc_T};
use crate::api::private::helpers::cstr_literal;
use crate::api::private::helpers::{
    SID_API_CLIENT, SID_LUA, add_c, api_clear_error, arena_array, arena_dict, arena_string,
    boolean_obj, cstr_as_obj, cstr_as_string, cstr_to_string, dict_obj, integer_obj, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectType,
};
use crate::autocmd::{
    AutoPatCmd, DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_SOURCECMD, EVENT_SOURCEPOST,
    EVENT_SOURCEPRE, PROF_YES, apply_autocmds, funccal_entry_T, has_autocmd,
};
use crate::charset::{skip_to_newline, skiptowhite, skiptowhite_esc, skipwhite, skipwhite_len};
use crate::clipboard::{TypvalVval, VarType, list_T, typval_T};
use crate::cmdexpand::{EXPAND_RUNTIME, WILD_ADD_SLASH, globpath};
use crate::cursor::linenr_T;
use crate::debugger::{dbg_breakpoint, dbg_find_breakpoint, has_profiling};
use crate::errors::*;
use crate::eval::decode::dict_T;
use crate::eval::deprecated::{EvalFuncData, ScopeDictDictItem};
use crate::eval::typval::{
    VAR_LOCKED, tv_check_for_opt_dict_arg, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_func,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_copy, tv_dict_find, tv_dict_get_string, tv_get_number_chk, tv_get_string,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_append_string,
    tv_list_append_tv,
};
use crate::eval::userfunc::{func_tbl_get, restore_funccal, save_funccal};
use crate::eval::vars::{SID_STR, new_script_vars};
use crate::eval::{eval_to_number, get_copyID};
use crate::ex_docmd::{do_cmdline, do_cmdline_cmd, do_exedit, getline_cookie, getline_equal};
use crate::ex_eval::{CSTP_FINISH, aborting, cleanup_conditionals, cstack_T, report_make_pending};
use crate::ex_session::FILE;
#[cfg(not(windows))]
use crate::fileio::fdopen;
#[cfg(windows)]
use crate::fileio::{_fdopen, EOL_DOS, EOL_UNIX, EOL_UNKNOWN};
use crate::fileio::{EINTR, O_RDONLY, fclose, fgets};
use crate::garray::{
    ga_append, ga_append_via_ptr, ga_clear, ga_clear_strings, ga_concat, ga_concat_len, ga_grow,
    ga_init, ga_remove_duplicate_strings, ga_set_growsize, garray_T,
};
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::input::openscript;
use crate::kvec::KVec;
use crate::lua::executor::{
    kRetNilBool, nlua_exec, nlua_exec_file, nlua_exec_ga, nlua_is_deferred_safe,
};
use crate::lua::stdlib::regmatch_T;
use crate::map::{
    Map, Set, map_put_ref_String_int, map_ref_String_int, mh_get_String, mh_put_String,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, MHPutStatus, MapHash, NvimString};
use crate::math::{FAIL, OK};
use crate::mbyte::{
    convert_setup, enc_canonize, mb_prevptr, string_convert, utfc_ptr2len, vimconv_T,
};
use crate::memline::ml_get;
use crate::memory::{
    Arena, memcnt, strequal, try_malloc, xcalloc, xfree, xmalloc, xmallocz, xmemcpyz, xmemdupz,
    xstrdup, xstrlcat, xstrlcpy,
};
#[cfg(windows)]
use crate::message::msg_source;
use crate::message::{
    emsg, message_filtered, msg_ext_set_kind, msg_ext_ui_flush, msg_outtrans, msg_putchar, semsg,
    smsg, verbose_enter, verbose_leave,
};
use crate::msgpack_rpc::packer::Integer;
use crate::option::{copy_option_part, kOptRuntimepath, set_option_value_give_err, vimrc_found};
use crate::option_vars::*;
use crate::os::env::{
    expand_env, expand_env_save, home_replace, home_replace_save, os_setenv, vim_env_iter,
    vim_env_iter_rev, vim_get_prefix_from_exepath, vim_getenv,
};
use crate::os::fs::errno_ptr;
use crate::os::fs::{os_file_is_readable, os_isdir, os_open, os_set_cloexec};
use crate::os::input::line_breakcheck;
use crate::os::stdpaths::{XDGVarType, get_appname, stdpaths_get_xdg_var};
use crate::path::{
    EW_DIR, EW_FILE, EW_NOBREAK, FreeWild, add_pathsep, after_pathsep, append_path, concat_fnames,
    fix_fname, gen_expand_wildcards, get_past_head, kPathCmpLiteral, path_cmp, path_equal,
    path_tail, path_with_extension, to_slash, vim_ispathsep, vim_ispathsep_nocolon,
};
use crate::profile::{
    estack_T, prof_child_enter, prof_child_exit, profile_add, profile_end, profile_init,
    profile_reset, profile_self, profile_start, profile_sub_wait, profile_zero, proftime_T,
    script_line_end, script_line_start, scriptitem_T, time_msg, time_pop, time_push,
};
use crate::regexp::{RE_MAGIC, RE_STRING, vim_regcomp, vim_regexec, vim_regfree};
use crate::sign::expand_T;
use crate::strings::{gettext, strstr, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::usercmd::add_win_cmd_modifiers;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::Mutex;

pub use crate::eval::vars::scid_T;
pub type etype_T = c_int;
pub type estack_arg_T = c_int;
pub use crate::cmdexpand::LineGetter;
pub use crate::eval::buffer::varnumber_T;
pub use crate::extmark::TriState;
pub type DoInRuntimepathCB = Option<
    unsafe extern "C" fn(
        num_fnames: c_int,
        fnames: *mut *mut c_char,
        all: bool,
        cookie: *mut c_void,
    ) -> bool,
>;

const Ctrl_V: c_char = 22;
#[cfg(windows)]
const Ctrl_Z: c_char = 26;

#[cfg(windows)]
const ENV_SEPCHAR: c_char = b';' as c_char;
#[cfg(not(windows))]
const ENV_SEPCHAR: c_char = b':' as c_char;

const PATHSEP: c_char = b'/' as c_char;

pub const ETYPE_TOP: etype_T = 0;
pub const ETYPE_SCRIPT: etype_T = 1;
pub const ETYPE_UFUNC: etype_T = 2;
pub const ETYPE_AUCMD: etype_T = 3;
pub const ETYPE_MODELINE: etype_T = 4;
pub const ETYPE_EXCEPT: etype_T = 5;
pub const ETYPE_ARGS: etype_T = 6;
pub const ETYPE_ENV: etype_T = 7;
pub const ETYPE_INTERNAL: etype_T = 8;
pub const ETYPE_SPELL: etype_T = 9;

pub const ESTACK_NONE: estack_arg_T = 0;
pub const ESTACK_SFILE: estack_arg_T = 1;
pub const ESTACK_STACK: estack_arg_T = 2;
pub const ESTACK_SCRIPT: estack_arg_T = 3;

pub const DOSO_NONE: c_int = 0;
pub const DOSO_VIMRC: c_int = 1;

pub const DIP_ALL: c_int = 0x01;
pub const DIP_DIR: c_int = 0x02;
pub const DIP_ERR: c_int = 0x04;
pub const DIP_START: c_int = 0x08;
pub const DIP_OPT: c_int = 0x10;
pub const DIP_NORTP: c_int = 0x20;
pub const DIP_NOAFTER: c_int = 0x40;
pub const DIP_AFTER: c_int = 0x80;
pub const DIP_DIRFILE: c_int = 0x200;

const kCpoConcat: c_int = b'C' as c_int;

pub(crate) const SID_CMDARG: scid_T = -2;
pub(crate) const SID_CARG: scid_T = -3;
pub(crate) const SID_ENV: scid_T = -4;
pub(crate) const SID_ERROR: scid_T = -5;
pub(crate) const SID_WINLAYOUT: scid_T = -7;

const AUTOLOAD_CHAR: u8 = b'#';

#[cfg(windows)]
const HLF_W: usize = 26;

#[cfg(windows)]
const O_BINARY: c_int = 0x8000;
#[cfg(windows)]
const O_NOINHERIT: c_int = 0x0080;

#[repr(C)]
pub struct scriptvar_T {
    pub sv_var: ScopeDictDictItem,
    pub sv_dict: dict_T,
}

#[repr(C)]
struct source_cookie_T {
    fp: *mut FILE,
    nextline: *mut c_char,
    sourcing_lnum: linenr_T,
    finished: bool,
    source_from_buf_or_str: bool,
    buf_lnum: c_int,
    buflines: garray_T,
    #[cfg(windows)]
    fileformat: c_int,
    #[cfg(windows)]
    error: bool,
    breakpoint: linenr_T,
    fname: *mut c_char,
    dbg_tick: c_int,
    level: c_int,
    conv: vimconv_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SearchPathItem {
    path: *mut c_char,
    after: bool,
    pack_inserted: bool,
    has_lua: TriState,
    pos_in_rtp: usize,
}

type RuntimeSearchPath = KVec<SearchPathItem>;
type CharVec = KVec<*mut c_char>;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut exestack: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: std::mem::size_of::<estack_T>() as c_int,
    ga_growsize: 50,
    ga_data: ptr::null_mut(),
};

#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut script_items: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: std::mem::size_of::<*mut scriptitem_T>() as c_int,
    ga_growsize: 20,
    ga_data: ptr::null_mut(),
};

static mut ga_loaded: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: std::mem::size_of::<*mut c_char>() as c_int,
    ga_growsize: 4,
    ga_data: ptr::null_mut(),
};

static mut last_current_SID_seq: c_int = 0;
static mut last_current_SID: scid_T = 0;

static mut runtime_search_path_valid: bool = false;
static mut runtime_search_path_valid_thread: bool = false;
static mut runtime_search_path_ref: *mut c_int = ptr::null_mut();
static mut runtime_search_path: RuntimeSearchPath = RuntimeSearchPath {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};
static mut runtime_search_path_thread: RuntimeSearchPath = RuntimeSearchPath {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};
static RUNTIME_SEARCH_PATH_MUTEX: Mutex<()> = Mutex::new(());

static mut runtime_expand_flags: c_int = 0;
static mut get_named_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
static mut get_named_thread_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
static mut pack_entry_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
static mut build_rtp_buf: [c_char; MAXPATHL] = [0; MAXPATHL];
static mut pack_start_buf: [c_char; MAXPATHL] = [0; MAXPATHL];

static mut APP_ADD_DIR: c_int = 0;
static mut APP_LOAD: c_int = 0;
static mut APP_BOTH: c_int = 0;

use crate::api::buffer::NUL;
use crate::api::vim::{MAXPATHL, kFalse, kNone, kTrue};
use crate::buffer::{SID_MODELINE, VAR_FIXED};
use crate::version::default_lib_dir;
use crate::viml::parser::parser::CONV_NONE;

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
pub unsafe fn script_item(id: scid_T) -> *mut scriptitem_T {
    unsafe { *(script_items.ga_data as *mut *mut scriptitem_T).add((id - 1) as usize) }
}

#[inline]
unsafe fn set_script_item(id: scid_T, si: *mut scriptitem_T) {
    unsafe {
        *(script_items.ga_data as *mut *mut scriptitem_T).add((id - 1) as usize) = si;
    }
}

#[inline]
pub unsafe fn script_id_valid(id: scid_T) -> bool {
    unsafe { id > 0 && id <= script_items.ga_len }
}

#[inline]
pub unsafe fn sourcing_name() -> *mut c_char {
    unsafe { (*(exestack.ga_data as *const estack_T).add((exestack.ga_len - 1) as usize)).es_name }
}

#[inline]
pub unsafe fn sourcing_lnum() -> linenr_T {
    unsafe { (*(exestack.ga_data as *const estack_T).add((exestack.ga_len - 1) as usize)).es_lnum }
}

#[inline]
pub unsafe fn set_sourcing_lnum(lnum: linenr_T) {
    unsafe {
        (*(exestack.ga_data as *mut estack_T).add((exestack.ga_len - 1) as usize)).es_lnum = lnum;
    }
}

const fn empty_set<K>() -> Set<K> {
    Set {
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
    }
}

const fn empty_map<K, V>() -> Map<K, V> {
    Map {
        set: empty_set(),
        values: ptr::null_mut(),
    }
}

unsafe fn set_put_ref_string(
    set: *mut Set<NvimString>,
    key: NvimString,
    key_alloc: *mut *mut NvimString,
) -> bool {
    unsafe {
        let mut status = MHPutStatus::kMHExisting;
        let k = mh_put_String(set, key, &mut status);
        if !key_alloc.is_null() {
            *key_alloc = (*set).keys.add(k as usize);
        }
        status != MHPutStatus::kMHExisting
    }
}

unsafe fn set_has_string(set: *mut Set<NvimString>, key: NvimString) -> bool {
    unsafe { mh_get_String(set, key) != MH_TOMBSTONE }
}

unsafe fn set_destroy_string(set: *mut Set<NvimString>) {
    unsafe {
        xfree((*set).keys.cast::<c_void>());
        xfree((*set).h.hash.cast::<c_void>());
        *set = empty_set();
    }
}

unsafe fn map_put_string_int(map: *mut Map<NvimString, c_int>, key: NvimString, value: c_int) {
    unsafe {
        let val = map_put_ref_String_int(map, key, ptr::null_mut(), ptr::null_mut());
        *val = value;
    }
}

unsafe fn map_get_string_int(map: *mut Map<NvimString, c_int>, key: NvimString) -> c_int {
    unsafe {
        let k = mh_get_String(&mut (*map).set, key);
        if k == MH_TOMBSTONE {
            0
        } else {
            *(*map).values.add(k as usize)
        }
    }
}

unsafe fn map_destroy_string_int(map: *mut Map<NvimString, c_int>) {
    unsafe {
        set_destroy_string(&mut (*map).set);
        xfree((*map).values.cast::<c_void>());
        (*map).values = ptr::null_mut();
    }
}

/// Initialize the execution stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn estack_init() {
    unsafe {
        ga_grow(&raw mut exestack, 10);
        let entry = (exestack.ga_data as *mut estack_T).add(exestack.ga_len as usize);
        (*entry).es_type = ETYPE_TOP;
        (*entry).es_name = ptr::null_mut();
        (*entry).es_lnum = 0;
        (*entry).es_info = ptr::null_mut();
        exestack.ga_len += 1;
    }
}

/// Add an item to the execution stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn estack_push(
    type_: etype_T,
    name: *mut c_char,
    lnum: linenr_T,
) -> *mut estack_T {
    unsafe {
        ga_grow(&raw mut exestack, 1);
        let entry = (exestack.ga_data as *mut estack_T).add(exestack.ga_len as usize);
        (*entry).es_type = type_;
        (*entry).es_name = name;
        (*entry).es_lnum = lnum;
        (*entry).es_info = ptr::null_mut();
        exestack.ga_len += 1;
        entry
    }
}

/// Add a user function to the execution stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn estack_push_ufunc(ufunc: *mut ufunc_T, lnum: linenr_T) {
    unsafe {
        let name = if !(*ufunc).uf_name_exp.is_null() {
            (*ufunc).uf_name_exp
        } else {
            (*ufunc).uf_name.as_mut_ptr()
        };
        let entry = estack_push(ETYPE_UFUNC, name, lnum);
        if !entry.is_null() {
            (*entry).es_info = ufunc.cast::<c_void>();
        }
    }
}

/// Take an item off of the execution stack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn estack_pop() {
    unsafe {
        if exestack.ga_len > 1 {
            exestack.ga_len -= 1;
        }
    }
}

/// Get the current value for <sfile> in allocated memory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn estack_sfile(which: estack_arg_T) -> *mut c_char {
    unsafe {
        let mut entry = (exestack.ga_data as *const estack_T).add((exestack.ga_len - 1) as usize);
        if which == ESTACK_SFILE && (*entry).es_type != ETYPE_UFUNC {
            return if !(*entry).es_name.is_null() {
                xstrdup((*entry).es_name)
            } else {
                ptr::null_mut()
            };
        }

        if which == ESTACK_SCRIPT {
            let mut idx = exestack.ga_len - 1;
            while idx >= 0 {
                if (*entry).es_type == ETYPE_UFUNC || (*entry).es_type == ETYPE_AUCMD {
                    let def_ctx = if (*entry).es_type == ETYPE_UFUNC {
                        &(*(*entry).es_info.cast::<ufunc_T>()).uf_script_ctx
                    } else {
                        &(*(*entry).es_info.cast::<AutoPatCmd>()).script_ctx
                    };
                    return if def_ctx.sc_sid > 0 {
                        xstrdup((*script_item(def_ctx.sc_sid)).sn_name)
                    } else {
                        ptr::null_mut()
                    };
                } else if (*entry).es_type == ETYPE_SCRIPT {
                    return xstrdup((*entry).es_name);
                }
                idx -= 1;
                if idx >= 0 {
                    entry = entry.sub(1);
                }
            }
            return ptr::null_mut();
        }

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&mut ga, 1, 100);
        let mut last_type = ETYPE_SCRIPT;
        for idx in 0..exestack.ga_len {
            entry = (exestack.ga_data as *const estack_T).add(idx as usize);
            if !(*entry).es_name.is_null() {
                let mut type_name = cstr_literal(c"");
                let es_name = cstr_as_string((*entry).es_name);
                if (*entry).es_type != last_type {
                    match (*entry).es_type {
                        ETYPE_SCRIPT => type_name = cstr_literal(c"script "),
                        ETYPE_UFUNC => type_name = cstr_literal(c"function "),
                        _ => {}
                    }
                    last_type = (*entry).es_type;
                }
                let lnum = if idx == exestack.ga_len - 1 {
                    if which == ESTACK_STACK {
                        sourcing_lnum()
                    } else {
                        0
                    }
                } else {
                    (*entry).es_lnum
                };

                let len = es_name.size + type_name.size + 26;
                ga_grow(&mut ga, len as c_int);
                ga_concat_len(&mut ga, type_name.data, type_name.size);
                ga_concat_len(&mut ga, es_name.data, es_name.size);
                if lnum != 0 {
                    ga.ga_len += vim_snprintf_safelen!(
                        (ga.ga_data as *mut c_char).add(ga.ga_len as usize),
                        (ga.ga_maxlen - ga.ga_len) as usize,
                        c"[%d]".as_ptr(),
                        lnum,
                    ) as c_int;
                }
                if idx != exestack.ga_len - 1 {
                    ga_concat_len(&mut ga, c"..".as_ptr(), 2);
                }
            }
        }

        if !ga.ga_data.is_null() {
            ga_append(&mut ga, 0);
        }
        ga.ga_data.cast::<c_char>()
    }
}

unsafe fn stacktrace_push_item(
    l: *mut list_T,
    fp: *mut ufunc_T,
    event: *const c_char,
    lnum: linenr_T,
    filepath: *mut c_char,
) {
    unsafe {
        let d = tv_dict_alloc_lock(VAR_FIXED);
        let mut tv = typval_T {
            v_type: VarType::VAR_DICT,
            v_lock: VAR_LOCKED,
            vval: TypvalVval {
                v_ptr: d.cast::<c_void>(),
            },
        };

        if !fp.is_null() {
            tv_dict_add_func(d, c"funcref".as_ptr(), 7, fp);
        }
        if !event.is_null() {
            tv_dict_add_str(d, c"event".as_ptr(), 5, event);
        }
        tv_dict_add_nr(d, c"lnum".as_ptr(), 4, varnumber_T::from(lnum));
        tv_dict_add_str(d, c"filepath".as_ptr(), 8, filepath);

        tv_list_append_tv(l, &mut tv);
    }
}

/// Create the stacktrace from exestack.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn stacktrace_create() -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(exestack.ga_len as isize);

        for i in 0..exestack.ga_len {
            let entry = &*(exestack.ga_data as *const estack_T).add(i as usize);
            let mut lnum = entry.es_lnum;

            if entry.es_type == ETYPE_SCRIPT {
                stacktrace_push_item(l, ptr::null_mut(), ptr::null(), lnum, entry.es_name);
            } else if entry.es_type == ETYPE_UFUNC {
                let fp = entry.es_info.cast::<ufunc_T>();
                let sctx = (*fp).uf_script_ctx;
                let filepath = if sctx.sc_sid > 0 {
                    get_scriptname(sctx, ptr::null_mut())
                } else {
                    c"".as_ptr() as *mut c_char
                };
                lnum += sctx.sc_lnum;
                stacktrace_push_item(l, fp, ptr::null(), lnum, filepath);
            } else if entry.es_type == ETYPE_AUCMD {
                let sctx = (*entry.es_info.cast::<AutoPatCmd>()).script_ctx;
                let filepath = if sctx.sc_sid > 0 {
                    get_scriptname(sctx, ptr::null_mut())
                } else {
                    c"".as_ptr() as *mut c_char
                };
                lnum += sctx.sc_lnum;
                stacktrace_push_item(l, ptr::null_mut(), entry.es_name, lnum, filepath);
            }
        }
        l
    }
}

/// getstacktrace() function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getstacktrace(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let l = stacktrace_create();
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_ptr = l.cast::<c_void>();
        if !l.is_null() {
            (*l).lv_refcount += 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtime_init() {}

unsafe fn get_runtime_cmd_flags(argp: *mut *mut c_char, where_len: usize) -> c_int {
    unsafe {
        let arg = *argp;
        if where_len == 0 {
            return 0;
        }
        let slice = std::slice::from_raw_parts(arg.cast::<u8>(), where_len);
        if b"START".starts_with(slice) {
            *argp = skipwhite(arg.add(where_len));
            return DIP_START + DIP_NORTP;
        }
        if b"OPT".starts_with(slice) {
            *argp = skipwhite(arg.add(where_len));
            return DIP_OPT + DIP_NORTP;
        }
        if b"PACK".starts_with(slice) {
            *argp = skipwhite(arg.add(where_len));
            return DIP_START + DIP_OPT + DIP_NORTP;
        }
        if b"ALL".starts_with(slice) {
            *argp = skipwhite(arg.add(where_len));
            return DIP_START + DIP_OPT;
        }
        0
    }
}

/// ":runtime [where] {name}"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_runtime(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;
        let mut flags = if (*eap).forceit != 0 { DIP_ALL } else { 0 };
        let p = skiptowhite(arg);
        flags += get_runtime_cmd_flags(&mut arg, (p as usize) - (arg as usize));
        debug_assert!(!arg.is_null());
        source_runtime(arg, flags);
    }
}

/// Set the completion context for the :runtime command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn set_context_in_runtime_cmd(xp: *mut expand_T, mut arg: *const c_char) {
    unsafe {
        let mut p = skiptowhite(arg);
        let mut arg_mut = arg as *mut c_char;
        runtime_expand_flags = if *p != NUL {
            get_runtime_cmd_flags(&mut arg_mut, (p as usize) - (arg as usize))
        } else {
            0
        };
        arg = arg_mut;
        loop {
            p = skiptowhite_esc(arg);
            if *p == NUL {
                break;
            }
            if runtime_expand_flags == 0 {
                runtime_expand_flags = DIP_ALL;
            }
            arg = skipwhite(p);
        }
        (*xp).xp_context = EXPAND_RUNTIME;
        (*xp).xp_pattern = arg as *mut c_char;
    }
}

unsafe extern "C" fn source_callback_vim_lua(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) -> bool {
    unsafe {
        let mut did_one = false;

        for i in 0..num_fnames {
            let fname = *fnames.add(i as usize);
            if path_with_extension(fname, c"vim".as_ptr()) {
                do_source(fname, false, DOSO_NONE, cookie.cast::<c_int>());
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        for i in 0..num_fnames {
            let fname = *fnames.add(i as usize);
            if path_with_extension(fname, c"lua".as_ptr()) {
                do_source(fname, false, DOSO_NONE, cookie.cast::<c_int>());
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        did_one
    }
}

unsafe extern "C" fn source_callback(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) -> bool {
    unsafe {
        let mut did_one = source_callback_vim_lua(num_fnames, fnames, all, cookie);

        if !all && did_one {
            return true;
        }

        for i in 0..num_fnames {
            let fname = *fnames.add(i as usize);
            if !path_with_extension(fname, c"vim".as_ptr())
                && !path_with_extension(fname, c"lua".as_ptr())
            {
                do_source(fname, false, DOSO_NONE, cookie.cast::<c_int>());
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        did_one
    }
}

/// Find the patterns in "name" in all directories in "path" and invoke
/// "callback(fname, cookie)".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_in_path(
    path: *const c_char,
    prefix: *const c_char,
    name: *mut c_char,
    flags: c_int,
    callback: DoInRuntimepathCB,
    cookie: *mut c_void,
) -> c_int {
    unsafe {
        let mut did_one = false;

        let rtp_copy = xstrdup(path);
        let mut buf = xmallocz(MAXPATHL).cast::<c_char>();
        {
            if p_verbose > 10 && !name.is_null() {
                verbose_enter();
                if *prefix != NUL {
                    smsg!(
                        0,
                        gettext(c"Searching for \"%s\" under \"%s\" in \"%s\"".as_ptr()),
                        name,
                        prefix,
                        path,
                    );
                } else {
                    smsg!(
                        0,
                        gettext(c"Searching for \"%s\" in \"%s\"".as_ptr()),
                        name,
                        path,
                    );
                }
                verbose_leave();
            }

            let do_all = (flags & DIP_ALL) != 0;

            let mut rtp = rtp_copy;
            while *rtp != NUL && (do_all || !did_one) {
                let buflen = copy_option_part(&mut rtp, buf, MAXPATHL, c",".as_ptr());

                if (flags & (DIP_NOAFTER | DIP_AFTER)) != 0 {
                    let is_after = path_is_after(buf, buflen);
                    if (is_after && (flags & DIP_NOAFTER) != 0)
                        || (!is_after && (flags & DIP_AFTER) != 0)
                    {
                        continue;
                    }
                }

                if name.is_null() {
                    if let Some(cb) = callback {
                        cb(1, &mut buf, do_all, cookie);
                    }
                    did_one = true;
                } else {
                    let prefix_len = CStr::from_ptr(prefix).to_bytes().len();
                    let name_len = CStr::from_ptr(name).to_bytes().len();
                    if buflen + 2 + prefix_len + name_len < MAXPATHL {
                        add_pathsep(buf);
                        let cur_len = CStr::from_ptr(buf).to_bytes().len();
                        ptr::copy_nonoverlapping(prefix, buf.add(cur_len), prefix_len + 1);
                        let tail = buf.add(CStr::from_ptr(buf).to_bytes().len());

                        let mut np = name;
                        while *np != NUL && (do_all || !did_one) {
                            let tail_off = (tail as usize) - (buf as usize);
                            debug_assert!(MAXPATHL >= tail_off);
                            copy_option_part(&mut np, tail, MAXPATHL - tail_off, c"\t ".as_ptr());

                            if p_verbose > 10 {
                                verbose_enter();
                                smsg!(0, gettext(c"Searching for \"%s\"".as_ptr()), buf);
                                verbose_leave();
                            }

                            let ew_flags = (if (flags & DIP_DIR) != 0 {
                                EW_DIR
                            } else {
                                EW_FILE
                            }) | (if (flags & DIP_DIRFILE) != 0 {
                                EW_DIR | EW_FILE
                            } else {
                                0
                            });

                            did_one |= gen_expand_wildcards_and_cb(
                                1, &mut buf, ew_flags, do_all, callback, cookie,
                            ) == OK;
                        }
                    }
                }
            }
        }
        xfree(buf.cast::<c_void>());
        xfree(rtp_copy.cast::<c_void>());
        if !did_one && !name.is_null() {
            let basepath = if ptr::eq(path, p_rtp) {
                c"runtimepath".as_ptr()
            } else {
                c"packpath".as_ptr()
            };

            if (flags & DIP_ERR) != 0 {
                semsg!(gettext(e_dirnotf.as_ptr()), basepath, name);
            } else if p_verbose > 1 {
                verbose_enter();
                smsg!(
                    0,
                    gettext(c"not found in '%s': \"%s\"".as_ptr()),
                    basepath,
                    name,
                );
                verbose_leave();
            }
        }

        if did_one { OK } else { FAIL }
    }
}

unsafe fn runtime_search_path_get_cached(ref_ptr: *mut c_int) -> RuntimeSearchPath {
    unsafe {
        runtime_search_path_validate();

        *ref_ptr = 0;
        if runtime_search_path_ref.is_null() {
            *ref_ptr += 1;
            runtime_search_path_ref = ref_ptr;
        }
        RuntimeSearchPath {
            size: runtime_search_path.size,
            capacity: runtime_search_path.capacity,
            items: runtime_search_path.items,
        }
    }
}

unsafe fn copy_runtime_search_path(src: &RuntimeSearchPath) -> RuntimeSearchPath {
    unsafe {
        let mut dst = RuntimeSearchPath::new();
        for j in 0..src.size {
            let item = *src.items.add(j);
            dst.push(SearchPathItem {
                path: xstrdup(item.path),
                after: item.after,
                pack_inserted: item.pack_inserted,
                has_lua: item.has_lua,
                pos_in_rtp: item.pos_in_rtp,
            });
        }
        dst
    }
}

unsafe fn runtime_search_path_unref(mut path: RuntimeSearchPath, ref_ptr: *const c_int) {
    unsafe {
        if *ref_ptr != 0 {
            if ptr::eq(runtime_search_path_ref, ref_ptr) {
                runtime_search_path_ref = ptr::null_mut();
            } else {
                runtime_search_path_free(&mut path);
            }
        }
    }
}

unsafe fn do_in_cached_path(
    name: *mut c_char,
    flags: c_int,
    callback: DoInRuntimepathCB,
    cookie: *mut c_void,
) -> c_int {
    unsafe {
        let mut did_one = false;
        let mut buf = [0 as c_char; MAXPATHL];

        if p_verbose > 10 && !name.is_null() {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Searching for \"%s\" in runtime path".as_ptr()),
                name,
            );
            verbose_leave();
        }

        let mut ref_val: c_int = 0;
        let path = runtime_search_path_get_cached(&mut ref_val);

        let do_all = (flags & DIP_ALL) != 0;

        for j in 0..path.size {
            let mut item = *path.items.add(j);
            let buflen = CStr::from_ptr(item.path).to_bytes().len();

            if (flags & (DIP_NOAFTER | DIP_AFTER)) != 0
                && ((item.after && (flags & DIP_NOAFTER) != 0)
                    || (!item.after && (flags & DIP_AFTER) != 0))
            {
                continue;
            }

            if name.is_null() {
                if let Some(cb) = callback {
                    cb(1, &mut item.path, do_all, cookie);
                }
            } else if buflen + CStr::from_ptr(name).to_bytes().len() + 2 < MAXPATHL {
                ptr::copy_nonoverlapping(item.path, buf.as_mut_ptr(), buflen + 1);
                add_pathsep(buf.as_mut_ptr());
                let tail = buf
                    .as_mut_ptr()
                    .add(CStr::from_ptr(buf.as_ptr()).to_bytes().len());

                let mut np = name;
                while *np != NUL && (do_all || !did_one) {
                    let tail_off = (tail as usize) - (buf.as_ptr() as usize);
                    debug_assert!(MAXPATHL >= tail_off);
                    copy_option_part(&mut np, tail, MAXPATHL - tail_off, c"\t ".as_ptr());

                    if p_verbose > 10 {
                        verbose_enter();
                        smsg!(0, gettext(c"Searching for \"%s\"".as_ptr()), buf.as_ptr());
                        verbose_leave();
                    }

                    let ew_flags = (if (flags & DIP_DIR) != 0 {
                        EW_DIR
                    } else {
                        EW_FILE
                    }) | (if (flags & DIP_DIRFILE) != 0 {
                        EW_DIR | EW_FILE
                    } else {
                        0
                    }) | EW_NOBREAK;

                    let mut pat = [buf.as_mut_ptr()];
                    did_one |= gen_expand_wildcards_and_cb(
                        1,
                        pat.as_mut_ptr(),
                        ew_flags,
                        do_all,
                        callback,
                        cookie,
                    ) == OK;
                }
            }
        }

        if !did_one && !name.is_null() {
            if (flags & DIP_ERR) != 0 {
                semsg!(gettext(e_dirnotf.as_ptr()), c"runtime path".as_ptr(), name);
            } else if p_verbose > 1 {
                verbose_enter();
                smsg!(
                    0,
                    gettext(c"not found in runtime path: \"%s\"".as_ptr()),
                    name,
                );
                verbose_leave();
            }
        }

        runtime_search_path_unref(path, &ref_val);

        if did_one { OK } else { FAIL }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtime_inspect(arena: *mut Arena) -> Array {
    unsafe {
        let mut rv = arena_array(arena, runtime_search_path.size);

        for i in 0..runtime_search_path.size {
            let item = &*runtime_search_path.items.add(i);
            let mut entry = arena_dict(arena, 5);
            put_c(&mut entry, c"path", cstr_as_obj(item.path));
            if item.after {
                put_c(&mut entry, c"after", boolean_obj(true));
            }
            if item.pack_inserted {
                put_c(&mut entry, c"pack_inserted", boolean_obj(true));
            }
            if item.has_lua != kNone {
                put_c(&mut entry, c"has_lua", boolean_obj(item.has_lua == kTrue));
            }
            put_c(
                &mut entry,
                c"pos_in_rtp",
                integer_obj(item.pos_in_rtp as Integer),
            );

            add_c(&mut rv, dict_obj(entry));
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtime_get_named(
    lua: bool,
    pat: Array,
    all: bool,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut ref_val: c_int = 0;
        let path = runtime_search_path_get_cached(&mut ref_val);
        let rv = runtime_get_named_common(
            lua,
            pat,
            all,
            &path,
            (&raw mut get_named_buf).cast::<c_char>(),
            MAXPATHL,
            arena,
        );
        runtime_search_path_unref(path, &ref_val);
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtime_get_named_thread(lua: bool, pat: Array, all: bool) -> Array {
    unsafe {
        let _guard = RUNTIME_SEARCH_PATH_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path = RuntimeSearchPath {
            size: runtime_search_path_thread.size,
            capacity: runtime_search_path_thread.capacity,
            items: runtime_search_path_thread.items,
        };
        runtime_get_named_common(
            lua,
            pat,
            all,
            &path,
            (&raw mut get_named_thread_buf).cast::<c_char>(),
            MAXPATHL,
            ptr::null_mut(),
        )
    }
}

unsafe fn runtime_get_named_common(
    lua: bool,
    pat: Array,
    all: bool,
    path: &RuntimeSearchPath,
    buf: *mut c_char,
    buf_len: usize,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut rv = arena_array(arena, path.size * pat.size);
        for i in 0..path.size {
            let item = &mut *path.items.add(i);
            if lua {
                if item.has_lua == kNone {
                    let size = vim_snprintf!(buf, buf_len, c"%s/lua/".as_ptr(), item.path) as usize;
                    item.has_lua = TriState::from(size < buf_len && os_isdir(buf));
                }
                if item.has_lua == kFalse {
                    continue;
                }
            }

            for j in 0..pat.size {
                let pat_item = *pat.items.add(j);
                if pat_item.obj_type == ObjectType::kObjectTypeString {
                    let size = vim_snprintf!(
                        buf,
                        buf_len,
                        c"%s/%s".as_ptr(),
                        item.path,
                        pat_item.data.string.data,
                    ) as usize;
                    if size < buf_len && os_file_is_readable(buf) {
                        to_slash(buf);
                        let astr = arena_string(arena, cstr_as_string(buf));
                        add_c(&mut rv, string_obj(astr));
                        if !all {
                            return rv;
                        }
                    }
                }
            }
        }
        rv
    }
}

/// Find "name" in "path".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_in_path_and_pp(
    path: *mut c_char,
    name: *mut c_char,
    flags: c_int,
    callback: DoInRuntimepathCB,
    cookie: *mut c_void,
) -> c_int {
    unsafe {
        let mut done = FAIL;

        if (flags & DIP_NORTP) == 0 {
            let name_arg = if !name.is_null() && *name == NUL {
                ptr::null_mut()
            } else {
                name
            };
            done |= do_in_path(path, c"".as_ptr(), name_arg, flags, callback, cookie);
        }

        if (done == FAIL || (flags & DIP_ALL) != 0) && (flags & DIP_START) != 0 {
            let mut prefix = if (flags & DIP_AFTER) != 0 {
                c"pack/*/start/*/after/".as_ptr()
            } else {
                c"pack/*/start/*/".as_ptr()
            };
            done |= do_in_path(p_pp, prefix, name, flags & !DIP_AFTER, callback, cookie);

            if done == FAIL || (flags & DIP_ALL) != 0 {
                prefix = if (flags & DIP_AFTER) != 0 {
                    c"start/*/after/".as_ptr()
                } else {
                    c"start/*/".as_ptr()
                };
                done |= do_in_path(p_pp, prefix, name, flags & !DIP_AFTER, callback, cookie);
            }
        }

        if (done == FAIL || (flags & DIP_ALL) != 0) && (flags & DIP_OPT) != 0 {
            done |= do_in_path(
                p_pp,
                c"pack/*/opt/*/".as_ptr(),
                name,
                flags,
                callback,
                cookie,
            );

            if done == FAIL || (flags & DIP_ALL) != 0 {
                done |= do_in_path(p_pp, c"opt/*/".as_ptr(), name, flags, callback, cookie);
            }
        }

        done
    }
}

unsafe fn push_path(
    search_path: &mut RuntimeSearchPath,
    rtp_used: *mut Set<NvimString>,
    entry: *mut c_char,
    after: bool,
    pos_in_rtp: usize,
) -> bool {
    unsafe {
        let mut key_alloc: *mut NvimString = ptr::null_mut();
        if set_put_ref_string(rtp_used, cstr_as_string(entry), &mut key_alloc) {
            *key_alloc = cstr_to_string(entry);
            search_path.push(SearchPathItem {
                path: (*key_alloc).data,
                after,
                pack_inserted: false,
                has_lua: kNone,
                pos_in_rtp,
            });
            return true;
        }
        false
    }
}

unsafe fn expand_rtp_entry(
    search_path: &mut RuntimeSearchPath,
    rtp_used: *mut Set<NvimString>,
    entry: *mut c_char,
    after: bool,
    pos_in_rtp: usize,
) {
    unsafe {
        if set_has_string(rtp_used, cstr_as_string(entry)) {
            return;
        }

        if *entry == NUL {
            push_path(search_path, rtp_used, entry, after, pos_in_rtp);
        }

        let mut num_files: c_int = 0;
        let mut files: *mut *mut c_char = ptr::null_mut();
        let mut pat = [entry];
        if gen_expand_wildcards(
            1,
            pat.as_mut_ptr(),
            &mut num_files,
            &mut files,
            EW_DIR | EW_NOBREAK,
        ) == OK
        {
            for i in 0..num_files {
                push_path(
                    search_path,
                    rtp_used,
                    *files.add(i as usize),
                    after,
                    pos_in_rtp,
                );
            }
            FreeWild(num_files, files);
        }
    }
}

unsafe fn expand_pack_entry(
    search_path: &mut RuntimeSearchPath,
    rtp_used: *mut Set<NvimString>,
    after_path: &mut CharVec,
    pack_entry: *const c_char,
    pack_entry_len: usize,
    pos_in_rtp: usize,
) {
    unsafe {
        let buf = (&raw mut pack_entry_buf).cast::<c_char>();
        let start_pats = [c"/pack/*/start/*", c"/start/*"];
        for pat in &start_pats {
            let pat_len = pat.to_bytes().len();
            if pack_entry_len + pat_len + 1 > MAXPATHL {
                continue;
            }
            xstrlcpy(buf, pack_entry, MAXPATHL);
            xstrlcpy(
                buf.add(pack_entry_len),
                pat.as_ptr(),
                MAXPATHL - pack_entry_len,
            );
            expand_rtp_entry(search_path, rtp_used, buf, false, pos_in_rtp);
            let after_size = CStr::from_ptr(buf).to_bytes().len() + 7;
            let after = xmallocz(after_size).cast::<c_char>();
            xstrlcpy(after, buf, after_size);
            xstrlcat(after, c"/after".as_ptr(), after_size);
            after_path.push(after);
        }
    }
}

unsafe fn path_is_after(buf: *const c_char, buflen: usize) -> bool {
    unsafe {
        buflen >= 5
            && (buflen < 6 || vim_ispathsep(*buf.add(buflen - 6) as u8 as c_int))
            && CStr::from_ptr(buf.add(buflen - 5)).to_bytes() == b"after"
    }
}

unsafe fn runtime_search_path_build() -> RuntimeSearchPath {
    unsafe {
        let mut pack_entries: KVec<NvimString> = KVec::new();
        let mut pack_used: Map<NvimString, c_int> = empty_map();
        let mut rtp_used: Set<NvimString> = empty_set();
        let mut search_path = RuntimeSearchPath::new();
        let mut after_path = CharVec::new();

        let buf = (&raw mut build_rtp_buf).cast::<c_char>();
        let mut entry = p_pp;
        while *entry != NUL {
            let cur_entry = entry;
            let buflen = copy_option_part(&mut entry, buf, MAXPATHL, c",".as_ptr());

            let the_entry = NvimString {
                data: cur_entry,
                size: buflen,
            };

            pack_entries.push(the_entry);
            map_put_string_int(&mut pack_used, the_entry, 0);
        }

        let mut rtp_entry = p_rtp;
        while *rtp_entry != NUL {
            let cur_entry = rtp_entry;
            let buflen = copy_option_part(&mut rtp_entry, buf, MAXPATHL, c",".as_ptr());

            if path_is_after(buf, buflen) {
                rtp_entry = cur_entry;
                break;
            }

            let pos_in_rtp = (cur_entry as usize) - (p_rtp as usize);

            expand_rtp_entry(&mut search_path, &mut rtp_used, buf, false, pos_in_rtp);

            let h = map_ref_String_int(&mut pack_used, cstr_as_string(buf), ptr::null_mut());
            if !h.is_null() {
                *h += 1;
                expand_pack_entry(
                    &mut search_path,
                    &mut rtp_used,
                    &mut after_path,
                    buf,
                    buflen,
                    pos_in_rtp,
                );
            }
        }

        let sentinel_pos_in_rtp = ((rtp_entry as usize) - (p_rtp as usize)).saturating_sub(1);

        for i in 0..pack_entries.size {
            let item = *pack_entries.items.add(i);
            let h = map_get_string_int(&mut pack_used, item);
            if h == 0 {
                expand_pack_entry(
                    &mut search_path,
                    &mut rtp_used,
                    &mut after_path,
                    item.data,
                    item.size,
                    sentinel_pos_in_rtp,
                );
            }
        }

        for i in 0..after_path.size {
            let ap = *after_path.items.add(i);
            expand_rtp_entry(
                &mut search_path,
                &mut rtp_used,
                ap,
                true,
                sentinel_pos_in_rtp,
            );
            xfree(ap.cast::<c_void>());
        }

        while *rtp_entry != NUL {
            let cur_entry = rtp_entry;
            let buflen = copy_option_part(&mut rtp_entry, buf, MAXPATHL, c",".as_ptr());
            let pos_in_rtp = (cur_entry as usize) - (p_rtp as usize);
            expand_rtp_entry(
                &mut search_path,
                &mut rtp_used,
                buf,
                path_is_after(buf, buflen),
                pos_in_rtp,
            );
        }

        pack_entries.destroy();
        after_path.destroy();
        map_destroy_string_int(&mut pack_used);
        set_destroy_string(&mut rtp_used);

        search_path
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn did_set_runtimepackpath(_args: *mut optset_T) -> *const c_char {
    unsafe {
        runtime_search_path_valid = false;
        ptr::null()
    }
}

unsafe fn runtime_search_path_free(path: &mut RuntimeSearchPath) {
    unsafe {
        for j in 0..path.size {
            let item = *path.items.add(j);
            xfree(item.path.cast::<c_void>());
        }
        path.destroy();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtime_search_path_validate() {
    unsafe {
        if !nlua_is_deferred_safe() {
            return;
        }
        if !runtime_search_path_valid {
            if runtime_search_path_ref.is_null() {
                msg_ext_ui_flush();
                let mut old_path = RuntimeSearchPath {
                    size: runtime_search_path.size,
                    capacity: runtime_search_path.capacity,
                    items: runtime_search_path.items,
                };
                runtime_search_path_free(&mut old_path);
            }
            runtime_search_path = runtime_search_path_build();
            runtime_search_path_valid = true;
            runtime_search_path_ref = ptr::null_mut();
            update_runtime_search_path_thread(true);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn update_runtime_search_path_thread(force: bool) {
    unsafe {
        if !force && !(runtime_search_path_valid && !runtime_search_path_valid_thread) {
            return;
        }

        let _guard = RUNTIME_SEARCH_PATH_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut old_thread = RuntimeSearchPath {
            size: runtime_search_path_thread.size,
            capacity: runtime_search_path_thread.capacity,
            items: runtime_search_path_thread.items,
        };
        runtime_search_path_free(&mut old_thread);
        let src = RuntimeSearchPath {
            size: runtime_search_path.size,
            capacity: runtime_search_path.capacity,
            items: runtime_search_path.items,
        };
        runtime_search_path_thread = copy_runtime_search_path(&src);
        drop(_guard);
        runtime_search_path_valid_thread = true;
    }
}

/// Just like do_in_path_and_pp(), using 'runtimepath' for "path".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_in_runtimepath(
    name: *mut c_char,
    mut flags: c_int,
    callback: DoInRuntimepathCB,
    cookie: *mut c_void,
) -> c_int {
    unsafe {
        let mut success = FAIL;
        if (flags & DIP_NORTP) == 0 {
            let name_arg = if !name.is_null() && *name == NUL {
                ptr::null_mut()
            } else {
                name
            };
            success |= do_in_cached_path(name_arg, flags, callback, cookie);
            flags = (flags & !DIP_START) | DIP_NORTP;
        }
        if (flags & (DIP_START | DIP_OPT)) != 0 && (success == FAIL || (flags & DIP_ALL) != 0) {
            success |= do_in_path_and_pp(p_rtp, name, flags, callback, cookie);
        }
        success
    }
}

/// Source the file "name" from all directories in 'runtimepath'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_runtime(name: *mut c_char, flags: c_int) -> c_int {
    unsafe { do_in_runtimepath(name, flags, Some(source_callback), ptr::null_mut()) }
}

/// Just like source_runtime(), but only source vim and lua files.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_runtime_vim_lua(name: *mut c_char, flags: c_int) -> c_int {
    unsafe { do_in_runtimepath(name, flags, Some(source_callback_vim_lua), ptr::null_mut()) }
}

/// Just like source_runtime(), but use "path" instead of 'runtimepath' and only source .vim and .lua files.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_in_path_vim_lua(
    path: *mut c_char,
    name: *mut c_char,
    flags: c_int,
) -> c_int {
    unsafe {
        do_in_path_and_pp(
            path,
            name,
            flags,
            Some(source_callback_vim_lua),
            ptr::null_mut(),
        )
    }
}

unsafe fn gen_expand_wildcards_and_cb(
    num_pat: c_int,
    pats: *mut *mut c_char,
    flags: c_int,
    all: bool,
    callback: DoInRuntimepathCB,
    cookie: *mut c_void,
) -> c_int {
    unsafe {
        let mut num_files: c_int = 0;
        let mut files: *mut *mut c_char = ptr::null_mut();

        if gen_expand_wildcards(num_pat, pats, &mut num_files, &mut files, flags) != OK {
            return FAIL;
        }

        if let Some(cb) = callback {
            cb(num_files, files, all, cookie);
        }

        FreeWild(num_files, files);

        OK
    }
}

unsafe fn add_pack_dir_to_rtp(fname: *mut c_char, is_pack: bool) -> c_int {
    unsafe {
        let mut afterdir = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let mut retval = FAIL;

        let mut p1 = get_past_head(fname);
        let mut p2 = p1;
        let mut p3 = p1;
        let mut p4 = p1;
        let mut p = p1;
        while *p != NUL {
            if vim_ispathsep_nocolon(*p as u8 as c_int) {
                p4 = p3;
                p3 = p2;
                p2 = p1;
                p1 = p;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        p4 = p4.add(1);
        let c = *p4;
        *p4 = NUL;
        let ffname = fix_fname(fname);
        *p4 = c;

        if ffname.is_null() {
            return FAIL;
        }

        'block: {
            let mut fname_len = CStr::from_ptr(ffname).to_bytes().len();
            let mut buf_data = [0 as c_char; MAXPATHL];
            let mut buf = NvimString {
                data: buf_data.as_mut_ptr(),
                size: 0,
            };

            let mut insp: *const c_char = ptr::null();
            let mut after_insp: *const c_char = ptr::null();
            let mut p_rtp_len: usize = 0;
            let mut entry: *mut c_char = p_rtp;
            while *entry != NUL {
                let cur_entry = entry;
                buf.size = copy_option_part(&mut entry, buf.data, MAXPATHL, c",".as_ptr());

                p_rtp_len += if *cur_entry.add(buf.size) == b',' as c_char {
                    buf.size + 1
                } else {
                    buf.size
                };

                let p_after = strstr(buf.data, c"after".as_ptr());
                let is_after = !p_after.is_null()
                    && (p_after as usize) > (buf.data as usize)
                    && vim_ispathsep(*p_after.sub(1) as u8 as c_int)
                    && (vim_ispathsep(*p_after.add(5) as u8 as c_int)
                        || *p_after.add(5) == NUL
                        || *p_after.add(5) == b',' as c_char);

                if is_after {
                    if insp.is_null() {
                        insp = cur_entry;
                    }
                    after_insp = cur_entry;
                    break;
                }

                if insp.is_null() {
                    add_pathsep(buf.data);
                    let rtp_ffname = fix_fname(buf.data);
                    if rtp_ffname.is_null() {
                        break 'block;
                    }
                    if path_cmp(p_fic != 0, rtp_ffname, ffname, fname_len) == 0 {
                        insp = entry;
                    }
                    xfree(rtp_ffname.cast::<c_void>());
                }
            }

            p_rtp_len += CStr::from_ptr(p_rtp.add(p_rtp_len)).to_bytes().len();
            if insp.is_null() {
                insp = p_rtp.add(p_rtp_len);
            }

            fname_len = CStr::from_ptr(fname).to_bytes().len();
            afterdir = concat_fnames(
                NvimString {
                    data: fname,
                    size: fname_len,
                },
                cstr_literal(c"after"),
                true,
            );
            if if is_pack {
                !pack_has_entries(afterdir.data)
            } else {
                !os_isdir(afterdir.data)
            } {
                afterdir.size = 0;
            }

            let new_rtp_capacity = p_rtp_len + fname_len + afterdir.size + 3;
            let new_rtp_data = try_malloc(new_rtp_capacity).cast::<c_char>();
            if new_rtp_data.is_null() {
                break 'block;
            }
            let mut new_rtp = NvimString {
                data: new_rtp_data,
                size: 0,
            };

            let mut keep = (insp as usize) - (p_rtp as usize);
            let mut first_pos = keep;
            ptr::copy(p_rtp, new_rtp.data, keep);
            new_rtp.size = keep;
            if *insp == NUL {
                *new_rtp.data.add(new_rtp.size) = b',' as c_char;
                new_rtp.size += 1;
                first_pos += 1;
            }
            ptr::copy(fname, new_rtp.data.add(new_rtp.size), fname_len);
            new_rtp.size += fname_len;
            if *insp != NUL {
                *new_rtp.data.add(new_rtp.size) = b',' as c_char;
                new_rtp.size += 1;
            }

            let mut after_pos: usize = 0;

            if afterdir.size > 0 && !after_insp.is_null() {
                let keep_after = (after_insp as usize) - (p_rtp as usize);
                let append_len = keep_after - keep;

                ptr::copy(p_rtp.add(keep), new_rtp.data.add(new_rtp.size), append_len);
                new_rtp.size += append_len;
                ptr::copy(afterdir.data, new_rtp.data.add(new_rtp.size), afterdir.size);
                new_rtp.size += afterdir.size;
                *new_rtp.data.add(new_rtp.size) = b',' as c_char;
                new_rtp.size += 1;
                keep = keep_after;
                after_pos = keep_after;
            }

            if *p_rtp.add(keep) != NUL {
                let append_len = p_rtp_len - keep;
                ptr::copy(
                    p_rtp.add(keep),
                    new_rtp.data.add(new_rtp.size),
                    append_len + 1,
                );
                new_rtp.size += append_len;
            } else {
                *new_rtp.data.add(new_rtp.size) = NUL;
            }

            if afterdir.size > 0 && after_insp.is_null() {
                *new_rtp.data.add(new_rtp.size) = b',' as c_char;
                new_rtp.size += 1;
                after_pos = new_rtp.size;
                ptr::copy(
                    afterdir.data,
                    new_rtp.data.add(new_rtp.size),
                    afterdir.size + 1,
                );
                new_rtp.size += afterdir.size;
            }

            let was_valid = runtime_search_path_valid;
            set_option_value_give_err(kOptRuntimepath, string_obj(new_rtp), 0);

            debug_assert!(!runtime_search_path_valid);
            if was_valid && !is_pack && runtime_search_path_ref.is_null() {
                runtime_search_path_valid = true;
                runtime_search_path_valid_thread = false;
                let rsp_ptr = &raw mut runtime_search_path;
                let rsp = &mut *rsp_ptr;
                rsp.push(SearchPathItem {
                    path: ptr::null_mut(),
                    after: false,
                    pack_inserted: false,
                    has_lua: kNone,
                    pos_in_rtp: 0,
                });
                let mut i = (rsp.size as isize) - 1;

                if afterdir.size > 0 {
                    rsp.push(SearchPathItem {
                        path: ptr::null_mut(),
                        after: false,
                        pack_inserted: false,
                        has_lua: kNone,
                        pos_in_rtp: 0,
                    });
                    i += 1;
                    while i >= 1 {
                        if i > 1
                            && (*runtime_search_path.items.add((i - 2) as usize)).pos_in_rtp
                                >= after_pos
                        {
                            let mut prev = *runtime_search_path.items.add((i - 2) as usize);
                            prev.pos_in_rtp += fname_len + afterdir.size + 2;
                            *runtime_search_path.items.add(i as usize) = prev;
                        } else {
                            *runtime_search_path.items.add(i as usize) = SearchPathItem {
                                path: xmemdupz(afterdir.data.cast::<c_void>(), afterdir.size)
                                    .cast::<c_char>(),
                                after: true,
                                pack_inserted: true,
                                has_lua: kNone,
                                pos_in_rtp: after_pos + fname_len + 1,
                            };
                            i -= 1;
                            break;
                        }
                        i -= 1;
                    }
                }

                while i >= 0 {
                    if i > 0
                        && (*runtime_search_path.items.add((i - 1) as usize)).pos_in_rtp
                            >= first_pos
                    {
                        let mut prev = *runtime_search_path.items.add((i - 1) as usize);
                        prev.pos_in_rtp += fname_len + 1;
                        *runtime_search_path.items.add(i as usize) = prev;
                    } else {
                        *runtime_search_path.items.add(i as usize) = SearchPathItem {
                            path: xmemdupz(fname.cast::<c_void>(), fname_len).cast::<c_char>(),
                            after: false,
                            pack_inserted: true,
                            has_lua: kNone,
                            pos_in_rtp: first_pos,
                        };
                        break;
                    }
                    i -= 1;
                }
            }
            xfree(new_rtp.data.cast::<c_void>());
            retval = OK;
        }

        xfree(ffname.cast::<c_void>());
        xfree(afterdir.data.cast::<c_void>());
        retval
    }
}

unsafe fn load_pack_plugin(opt: bool, fname: *mut c_char) -> c_int {
    unsafe {
        let ffname = fix_fname(fname);
        let len = CStr::from_ptr(ffname).to_bytes().len() + 14;
        let mut pat = xmallocz(len).cast::<c_char>();

        vim_snprintf!(pat, len, c"%s/plugin/**/*".as_ptr(), ffname);
        gen_expand_wildcards_and_cb(
            1,
            &mut pat,
            EW_FILE,
            true,
            Some(source_callback_vim_lua),
            ptr::null_mut(),
        );

        let cmd = xstrdup(c"g:did_load_filetypes".as_ptr());

        if opt && eval_to_number(cmd, false) > 0 {
            do_cmdline_cmd(c"augroup filetypedetect".as_ptr());
            vim_snprintf!(pat, len, c"%s/ftdetect/*".as_ptr(), ffname);
            gen_expand_wildcards_and_cb(
                1,
                &mut pat,
                EW_FILE,
                true,
                Some(source_callback_vim_lua),
                ptr::null_mut(),
            );
            do_cmdline_cmd(c"augroup END".as_ptr());
        }
        xfree(cmd.cast::<c_void>());
        xfree(pat.cast::<c_void>());
        xfree(ffname.cast::<c_void>());

        OK
    }
}

unsafe fn add_pack_plugins(
    opt: bool,
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) {
    unsafe {
        let mut did_one = false;

        if !ptr::eq(cookie, (&raw mut APP_LOAD).cast::<c_void>()) {
            let mut buf = [0 as c_char; MAXPATHL];
            for i in 0..num_fnames {
                let mut found = false;
                let mut p: *mut c_char = p_rtp;
                while *p != NUL {
                    copy_option_part(&mut p, buf.as_mut_ptr(), MAXPATHL, c",".as_ptr());
                    if path_equal(buf.as_ptr(), *fnames.add(i as usize), kPathCmpLiteral) {
                        found = true;
                        break;
                    }
                }
                if !found && add_pack_dir_to_rtp(*fnames.add(i as usize), false) == FAIL {
                    return;
                }
                did_one = true;
                if !all {
                    break;
                }
            }
        }

        if !all && did_one {
            return;
        }

        if !ptr::eq(cookie, (&raw mut APP_ADD_DIR).cast::<c_void>()) {
            for i in 0..num_fnames {
                load_pack_plugin(opt, *fnames.add(i as usize));
                if !all {
                    break;
                }
            }
        }
    }
}

unsafe extern "C" fn add_start_pack_plugins(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) -> bool {
    unsafe {
        add_pack_plugins(false, num_fnames, fnames, all, cookie);
        num_fnames > 0
    }
}

unsafe extern "C" fn add_opt_pack_plugins(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) -> bool {
    unsafe {
        add_pack_plugins(true, num_fnames, fnames, all, cookie);
        num_fnames > 0
    }
}

/// Add all packages in the "start" directory to 'runtimepath'.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn add_pack_start_dirs() {
    unsafe {
        do_in_path(
            p_pp,
            c"".as_ptr(),
            ptr::null_mut(),
            DIP_ALL + DIP_DIR,
            Some(add_pack_start_dir),
            ptr::null_mut(),
        );
    }
}

unsafe fn pack_has_entries(buf: *mut c_char) -> bool {
    unsafe {
        let mut num_files: c_int = 0;
        let mut files: *mut *mut c_char = ptr::null_mut();
        let mut pat = [buf];
        if gen_expand_wildcards(1, pat.as_mut_ptr(), &mut num_files, &mut files, EW_DIR) == OK {
            FreeWild(num_files, files);
        }
        num_files > 0
    }
}

unsafe extern "C" fn add_pack_start_dir(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    _cookie: *mut c_void,
) -> bool {
    unsafe {
        let buf = (&raw mut pack_start_buf).cast::<c_char>();
        let start_pats = [c"/start/*", c"/pack/*/start/*"];
        for i in 0..num_fnames {
            let fname = *fnames.add(i as usize);
            let fname_len = CStr::from_ptr(fname).to_bytes().len();
            for pat in &start_pats {
                if fname_len + pat.to_bytes().len() + 1 > MAXPATHL {
                    continue;
                }
                xstrlcpy(buf, fname, MAXPATHL);
                xstrlcat(buf, pat.as_ptr(), MAXPATHL);
                if pack_has_entries(buf) {
                    add_pack_dir_to_rtp(buf, true);
                }
            }

            if !all {
                break;
            }
        }

        num_fnames > 1
    }
}

/// Load plugins from all packages in the "start" directory.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn load_start_packages() {
    unsafe {
        did_source_packages = true;
        do_in_path(
            p_pp,
            c"".as_ptr(),
            c"pack/*/start/*".as_ptr() as *mut c_char,
            DIP_ALL + DIP_DIR,
            Some(add_start_pack_plugins),
            (&raw mut APP_LOAD).cast::<c_void>(),
        );
        do_in_path(
            p_pp,
            c"".as_ptr(),
            c"start/*".as_ptr() as *mut c_char,
            DIP_ALL + DIP_DIR,
            Some(add_start_pack_plugins),
            (&raw mut APP_LOAD).cast::<c_void>(),
        );

        update_runtime_search_path_thread(false);
    }
}

/// ":packloadall"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_packloadall(eap: *mut exarg_T) {
    unsafe {
        if !did_source_packages || (*eap).forceit != 0 {
            add_pack_start_dirs();
            load_start_packages();
        }
    }
}

/// Read all the plugin files at startup.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn load_plugins() {
    unsafe {
        if p_lpl != 0 {
            let mut rtp_copy = p_rtp;
            let plugin_pattern = c"plugin/**/*".as_ptr() as *mut c_char;

            if !did_source_packages {
                rtp_copy = xstrdup(p_rtp);
                add_pack_start_dirs();
            }

            source_in_path_vim_lua(rtp_copy, plugin_pattern, DIP_ALL | DIP_NOAFTER);
            if !time_fd.is_null() {
                time_msg(c"loading rtp plugins".as_ptr(), ptr::null());
            }

            if !did_source_packages {
                xfree(rtp_copy.cast::<c_void>());
                load_start_packages();
            }
            if !time_fd.is_null() {
                time_msg(c"loading packages".as_ptr(), ptr::null());
            }

            source_runtime_vim_lua(plugin_pattern, DIP_ALL | DIP_AFTER);
            if !time_fd.is_null() {
                time_msg(c"loading after plugins".as_ptr(), ptr::null());
            }
        }
    }
}

/// ":packadd[!] {name}"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_packadd(eap: *mut exarg_T) {
    unsafe {
        let mut res = OK;
        let len = 11 + CStr::from_ptr((*eap).arg).to_bytes().len() + 5;
        let pat = xmallocz(len).cast::<c_char>();
        let cookie = if (*eap).forceit != 0 {
            (&raw mut APP_ADD_DIR).cast::<c_void>()
        } else {
            (&raw mut APP_BOTH).cast::<c_void>()
        };

        if !did_source_packages {
            vim_snprintf!(
                pat,
                len,
                c"pack/*/%s/%s".as_ptr(),
                c"start".as_ptr(),
                (*eap).arg,
            );
            res = do_in_path(
                p_pp,
                c"".as_ptr(),
                pat,
                DIP_ALL + DIP_DIR,
                Some(add_start_pack_plugins),
                cookie,
            );
        }

        vim_snprintf!(
            pat,
            len,
            c"pack/*/%s/%s".as_ptr(),
            c"opt".as_ptr(),
            (*eap).arg,
        );
        do_in_path(
            p_pp,
            c"".as_ptr(),
            pat,
            DIP_ALL + DIP_DIR + if res == FAIL { DIP_ERR } else { 0 },
            Some(add_opt_pack_plugins),
            cookie,
        );

        update_runtime_search_path_thread(false);

        xfree(pat.cast::<c_void>());
    }
}

unsafe fn ExpandRTDir_int(
    pat: *mut c_char,
    pat_len: usize,
    flags: c_int,
    keep_ext: bool,
    gap: *mut garray_T,
    dirnames: *mut *mut c_char,
) {
    unsafe {
        let mut i = 0usize;
        while !(*dirnames.add(i)).is_null() {
            let dir = *dirnames.add(i);
            let buf_len = CStr::from_ptr(dir).to_bytes().len() + pat_len + 64;
            let buf = xmalloc(buf_len).cast::<c_char>();
            let mut glob_flags: c_int = 0;
            let mut expand_dirs = false;
            let dir_str = if *dir != NUL { dir } else { c"".as_ptr() };
            let slash_str = if *dir != NUL {
                c"/".as_ptr()
            } else {
                c"".as_ptr()
            };
            vim_snprintf!(
                buf,
                buf_len,
                c"%s%s%s%s".as_ptr(),
                dir_str,
                slash_str,
                pat,
                c"*.{vim,lua}".as_ptr(),
            );

            loop {
                if (flags & DIP_NORTP) == 0 {
                    globpath(p_rtp, buf, gap, glob_flags, expand_dirs);
                }

                let ext_str = if expand_dirs {
                    c"*".as_ptr()
                } else {
                    c"*.{vim,lua}".as_ptr()
                };

                if (flags & DIP_START) != 0 {
                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"pack/*/start/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(p_pp, buf, gap, glob_flags, expand_dirs);

                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"start/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(p_pp, buf, gap, glob_flags, expand_dirs);
                }

                if (flags & DIP_OPT) != 0 {
                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"pack/*/opt/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(p_pp, buf, gap, glob_flags, expand_dirs);

                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"opt/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(p_pp, buf, gap, glob_flags, expand_dirs);
                }

                if *dir == NUL && !expand_dirs {
                    vim_snprintf!(buf, buf_len, c"%s*".as_ptr(), pat);
                    glob_flags = WILD_ADD_SLASH;
                    expand_dirs = true;
                    continue;
                }
                break;
            }

            xfree(buf.cast::<c_void>());
            i += 1;
        }

        let mut pat_pathsep_cnt: c_int = 0;
        for idx in 0..pat_len {
            if vim_ispathsep(*pat.add(idx) as u8 as c_int) {
                pat_pathsep_cnt += 1;
            }
        }

        for idx in 0..(*gap).ga_len {
            let match_ptr = *((*gap).ga_data as *mut *mut c_char).add(idx as usize);
            let mut s = match_ptr;
            let mut e = s.add(CStr::from_ptr(s).to_bytes().len());
            if (e as usize) - (s as usize) > 4 && !keep_ext {
                let tail4 = std::slice::from_raw_parts(e.sub(4).cast::<u8>(), 4);
                if tail4.eq_ignore_ascii_case(b".vim") || tail4.eq_ignore_ascii_case(b".lua") {
                    e = e.sub(4);
                    *e = NUL;
                }
            }

            let mut match_pathsep_cnt =
                if (e as usize) > (s as usize) && *e.sub(1) == b'/' as c_char {
                    -1
                } else {
                    0
                };
            s = e;
            while (s as usize) > (match_ptr as usize) {
                if vim_ispathsep(*s as u8 as c_int) {
                    match_pathsep_cnt += 1;
                    if match_pathsep_cnt > pat_pathsep_cnt {
                        break;
                    }
                }
                s = mb_prevptr(match_ptr, s);
            }
            s = s.add(1);
            if !ptr::eq(s, match_ptr) {
                let copy_len = (e as usize) - (s as usize) + 1;
                ptr::copy(s, match_ptr, copy_len);
            }
        }

        if (*gap).ga_len == 0 {
            return;
        }

        ga_remove_duplicate_strings(gap);
    }
}

/// Expand color scheme, compiler or filetype names.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ExpandRTDir(
    pat: *mut c_char,
    flags: c_int,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
    dirnames: *mut *mut c_char,
) -> c_int {
    unsafe {
        *num_file = 0;
        *file = ptr::null_mut();

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&mut ga, std::mem::size_of::<*mut c_char>() as c_int, 10);

        ExpandRTDir_int(
            pat,
            CStr::from_ptr(pat).to_bytes().len(),
            flags,
            false,
            &mut ga,
            dirnames,
        );

        if ga.ga_len == 0 {
            return FAIL;
        }

        *file = ga.ga_data.cast::<*mut c_char>();
        *num_file = ga.ga_len;
        OK
    }
}

/// Handle command line completion for :runtime command.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn expand_runtime_cmd(
    pat: *mut c_char,
    numMatches: *mut c_int,
    matches: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        *numMatches = 0;
        *matches = ptr::null_mut();

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&mut ga, std::mem::size_of::<*mut c_char>() as c_int, 10);

        let pat_len = CStr::from_ptr(pat).to_bytes().len();
        let mut dirnames = [c"".as_ptr() as *mut c_char, ptr::null_mut()];
        ExpandRTDir_int(
            pat,
            pat_len,
            runtime_expand_flags,
            true,
            &mut ga,
            dirnames.as_mut_ptr(),
        );

        if runtime_expand_flags == 0 {
            let where_values = [c"START", c"OPT", c"PACK", c"ALL"];
            let pat_slice = std::slice::from_raw_parts(pat.cast::<u8>(), pat_len);
            for wv in &where_values {
                if wv.to_bytes().starts_with(pat_slice) {
                    let slot = ga_append_via_ptr(&mut ga, std::mem::size_of::<*mut c_char>())
                        .cast::<*mut c_char>();
                    *slot = xstrdup(wv.as_ptr());
                }
            }
        }

        if ga.ga_len == 0 {
            return FAIL;
        }

        *matches = ga.ga_data.cast::<*mut c_char>();
        *numMatches = ga.ga_len;
        OK
    }
}

/// Expand loadplugin names: 'packpath'/pack/*/opt/{pat}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ExpandPackAddDir(
    pat: *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();

        *num_file = 0;
        *file = ptr::null_mut();
        let pat_len = CStr::from_ptr(pat).to_bytes().len();
        ga_init(&mut ga, std::mem::size_of::<*mut c_char>() as c_int, 10);

        let buflen = pat_len + 26;
        let s = xmalloc(buflen).cast::<c_char>();
        vim_snprintf!(s, buflen, c"pack/*/opt/%s*".as_ptr(), pat);
        globpath(p_pp, s, &mut ga, 0, true);
        vim_snprintf!(s, buflen, c"opt/%s*".as_ptr(), pat);
        globpath(p_pp, s, &mut ga, 0, true);
        xfree(s.cast::<c_void>());

        for i in 0..ga.ga_len {
            let match_ptr = *(ga.ga_data as *mut *mut c_char).add(i as usize);
            let tail = path_tail(match_ptr);
            let tail_len = CStr::from_ptr(tail).to_bytes().len();
            ptr::copy(tail, match_ptr, tail_len + 1);
        }

        if ga.ga_len == 0 {
            return FAIL;
        }

        ga_remove_duplicate_strings(&mut ga);

        *file = ga.ga_data.cast::<*mut c_char>();
        *num_file = ga.ga_len;
        OK
    }
}

unsafe fn strcpy_comma_escaped(dest: *mut c_char, src: *const c_char, len: usize) -> *mut c_char {
    unsafe {
        let mut shift: usize = 0;
        for i in 0..len {
            if *src.add(i) == b',' as c_char {
                *dest.add(i + shift) = b'\\' as c_char;
                shift += 1;
            }
            *dest.add(i + shift) = *src.add(i);
        }
        dest.add(len + shift)
    }
}

unsafe fn compute_double_env_sep_len(
    val: *const c_char,
    common_suf_len: usize,
    single_suf_len: usize,
) -> usize {
    unsafe {
        if val.is_null() || *val == NUL {
            return 0;
        }
        let mut ret: usize = 0;
        let mut iter: *const c_void = ptr::null();
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = vim_env_iter(ENV_SEPCHAR, val, iter, &mut dir, &mut dir_len);
            if !dir.is_null() && dir_len > 0 {
                ret += (dir_len
                    + memcnt(dir.cast::<c_void>(), b',' as c_char, dir_len)
                    + common_suf_len
                    + usize::from(after_pathsep(dir, dir.add(dir_len)) == 0))
                    * 2
                    + single_suf_len;
            }
            if iter.is_null() {
                break;
            }
        }
        ret
    }
}

unsafe fn add_env_sep_dirs(
    mut dest: *mut c_char,
    val: *const c_char,
    suf1: *const c_char,
    len1: usize,
    suf2: *const c_char,
    len2: usize,
    forward: bool,
) -> *mut c_char {
    unsafe {
        if val.is_null() || *val == NUL {
            return dest;
        }
        let mut iter: *const c_void = ptr::null();
        let appname = get_appname(false);
        let appname_len = CStr::from_ptr(appname).to_bytes().len();
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = if forward {
                vim_env_iter(ENV_SEPCHAR, val, iter, &mut dir, &mut dir_len)
            } else {
                vim_env_iter_rev(ENV_SEPCHAR, val, iter, &mut dir, &mut dir_len)
            };
            if !dir.is_null() && dir_len > 0 {
                dest = strcpy_comma_escaped(dest, dir, dir_len);
                if after_pathsep(dest.sub(1), dest) == 0 {
                    *dest = PATHSEP;
                    dest = dest.add(1);
                }
                ptr::copy(appname, dest, appname_len);
                dest = dest.add(appname_len);
                if !suf1.is_null() {
                    *dest = PATHSEP;
                    dest = dest.add(1);
                    ptr::copy(suf1, dest, len1);
                    dest = dest.add(len1);
                    if !suf2.is_null() {
                        *dest = PATHSEP;
                        dest = dest.add(1);
                        ptr::copy(suf2, dest, len2);
                        dest = dest.add(len2);
                    }
                }
                *dest = b',' as c_char;
                dest = dest.add(1);
            }
            if iter.is_null() {
                break;
            }
        }
        dest
    }
}

unsafe fn add_dir(
    mut dest: *mut c_char,
    dir: *const c_char,
    dir_len: usize,
    type_: XDGVarType,
    suf1: *const c_char,
    len1: usize,
    suf2: *const c_char,
    len2: usize,
) -> *mut c_char {
    unsafe {
        if dir.is_null() || dir_len == 0 {
            return dest;
        }
        dest = strcpy_comma_escaped(dest, dir, dir_len);
        let append_nvim = matches!(type_, XDGVarType::kXDGDataHome | XDGVarType::kXDGConfigHome);
        if append_nvim {
            if after_pathsep(dest.sub(1), dest) == 0 {
                *dest = PATHSEP;
                dest = dest.add(1);
            }
            let appname = get_appname(false);
            #[cfg(windows)]
            let mut appname_len = CStr::from_ptr(appname).to_bytes().len();
            #[cfg(not(windows))]
            let appname_len = CStr::from_ptr(appname).to_bytes().len();
            debug_assert!(appname_len < IOSIZE - 6);
            let iobuff = (&raw mut IObuff).cast::<c_char>();
            xmemcpyz(
                iobuff.cast::<c_void>(),
                appname.cast::<c_void>(),
                appname_len,
            );
            #[cfg(windows)]
            if matches!(type_, XDGVarType::kXDGDataHome | XDGVarType::kXDGStateHome) {
                xstrlcat(iobuff, c"-data".as_ptr(), IOSIZE);
                appname_len += 5;
            }
            xmemcpyz(dest.cast::<c_void>(), iobuff.cast::<c_void>(), appname_len);
            dest = dest.add(appname_len);
            if !suf1.is_null() {
                *dest = PATHSEP;
                dest = dest.add(1);
                ptr::copy(suf1, dest, len1);
                dest = dest.add(len1);
                if !suf2.is_null() {
                    *dest = PATHSEP;
                    dest = dest.add(1);
                    ptr::copy(suf2, dest, len2);
                    dest = dest.add(len2);
                }
            }
        }
        *dest = b',' as c_char;
        dest.add(1)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_lib_dir() -> *mut c_char {
    unsafe {
        if !default_lib_dir.is_null() && *default_lib_dir != NUL && os_isdir(default_lib_dir) {
            return xstrdup(default_lib_dir);
        }

        let mut exe_name = [0 as c_char; MAXPATHL];
        vim_get_prefix_from_exepath(exe_name.as_mut_ptr());
        if append_path(exe_name.as_mut_ptr(), c"lib/nvim".as_ptr(), MAXPATHL) == OK {
            return xstrdup(exe_name.as_ptr());
        }
        ptr::null_mut()
    }
}

/// Determine the startup value for &runtimepath.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn runtimepath_default(clean_arg: bool) -> *mut c_char {
    unsafe {
        const SITE_SIZE: usize = 4;
        const AFTER_SIZE: usize = 5;

        let mut rtp_size: usize = 0;
        let data_home = if clean_arg {
            ptr::null_mut()
        } else {
            stdpaths_get_xdg_var(XDGVarType::kXDGDataHome)
        };
        let config_home = if clean_arg {
            ptr::null_mut()
        } else {
            stdpaths_get_xdg_var(XDGVarType::kXDGConfigHome)
        };
        let libdir = get_lib_dir();
        let data_dirs = stdpaths_get_xdg_var(XDGVarType::kXDGDataDirs);
        let config_dirs = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs);
        let vimruntime = vim_getenv(c"VIMRUNTIME".as_ptr());

        let mut data_len: usize = 0;
        let mut config_len: usize = 0;
        let mut vimruntime_len: usize = 0;
        let mut libdir_len: usize = 0;
        let appname_len = CStr::from_ptr(get_appname(false)).to_bytes().len();

        if !data_home.is_null() {
            data_len = CStr::from_ptr(data_home).to_bytes().len();
            let nvim_data_size = if cfg!(windows) {
                appname_len + 5
            } else {
                appname_len
            };
            if data_len != 0 {
                rtp_size += (data_len
                    + memcnt(data_home.cast::<c_void>(), b',' as c_char, data_len)
                    + nvim_data_size
                    + 1
                    + SITE_SIZE
                    + 1
                    + usize::from(after_pathsep(data_home, data_home.add(data_len)) == 0))
                    * 2
                    + AFTER_SIZE
                    + 1;
            }
        }
        if !config_home.is_null() {
            config_len = CStr::from_ptr(config_home).to_bytes().len();
            if config_len != 0 {
                rtp_size += (config_len
                    + memcnt(config_home.cast::<c_void>(), b',' as c_char, config_len)
                    + appname_len
                    + 1
                    + usize::from(after_pathsep(config_home, config_home.add(config_len)) == 0))
                    * 2
                    + AFTER_SIZE
                    + 1;
            }
        }
        if !vimruntime.is_null() {
            vimruntime_len = CStr::from_ptr(vimruntime).to_bytes().len();
            if vimruntime_len != 0 {
                rtp_size += vimruntime_len
                    + memcnt(vimruntime.cast::<c_void>(), b',' as c_char, vimruntime_len)
                    + 1;
            }
        }
        if !libdir.is_null() {
            libdir_len = CStr::from_ptr(libdir).to_bytes().len();
            if libdir_len != 0 {
                rtp_size +=
                    libdir_len + memcnt(libdir.cast::<c_void>(), b',' as c_char, libdir_len) + 1;
            }
        }
        rtp_size +=
            compute_double_env_sep_len(data_dirs, appname_len + 1 + SITE_SIZE + 1, AFTER_SIZE + 1);
        rtp_size += compute_double_env_sep_len(config_dirs, appname_len + 1, AFTER_SIZE + 1);

        let mut rtp: *mut c_char = ptr::null_mut();
        if rtp_size != 0 {
            rtp = xmalloc(rtp_size).cast::<c_char>();
            let mut rtp_cur = rtp;
            rtp_cur = add_dir(
                rtp_cur,
                config_home,
                config_len,
                XDGVarType::kXDGConfigHome,
                ptr::null(),
                0,
                ptr::null(),
                0,
            );
            rtp_cur = add_env_sep_dirs(rtp_cur, config_dirs, ptr::null(), 0, ptr::null(), 0, true);
            rtp_cur = add_dir(
                rtp_cur,
                data_home,
                data_len,
                XDGVarType::kXDGDataHome,
                c"site".as_ptr(),
                SITE_SIZE,
                ptr::null(),
                0,
            );
            rtp_cur = add_env_sep_dirs(
                rtp_cur,
                data_dirs,
                c"site".as_ptr(),
                SITE_SIZE,
                ptr::null(),
                0,
                true,
            );
            rtp_cur = add_dir(
                rtp_cur,
                vimruntime,
                vimruntime_len,
                XDGVarType::kXDGNone,
                ptr::null(),
                0,
                ptr::null(),
                0,
            );
            rtp_cur = add_dir(
                rtp_cur,
                libdir,
                libdir_len,
                XDGVarType::kXDGNone,
                ptr::null(),
                0,
                ptr::null(),
                0,
            );
            rtp_cur = add_env_sep_dirs(
                rtp_cur,
                data_dirs,
                c"site".as_ptr(),
                SITE_SIZE,
                c"after".as_ptr(),
                AFTER_SIZE,
                false,
            );
            rtp_cur = add_dir(
                rtp_cur,
                data_home,
                data_len,
                XDGVarType::kXDGDataHome,
                c"site".as_ptr(),
                SITE_SIZE,
                c"after".as_ptr(),
                AFTER_SIZE,
            );
            rtp_cur = add_env_sep_dirs(
                rtp_cur,
                config_dirs,
                c"after".as_ptr(),
                AFTER_SIZE,
                ptr::null(),
                0,
                false,
            );
            rtp_cur = add_dir(
                rtp_cur,
                config_home,
                config_len,
                XDGVarType::kXDGConfigHome,
                c"after".as_ptr(),
                AFTER_SIZE,
                ptr::null(),
                0,
            );
            *rtp_cur.sub(1) = NUL;
            debug_assert_eq!((rtp_cur as usize) - (rtp as usize), rtp_size);
        }

        xfree(data_dirs.cast::<c_void>());
        xfree(config_dirs.cast::<c_void>());
        xfree(data_home.cast::<c_void>());
        xfree(config_home.cast::<c_void>());
        xfree(vimruntime.cast::<c_void>());
        xfree(libdir.cast::<c_void>());

        rtp
    }
}

unsafe fn cmd_source(fname: *mut c_char, eap: *mut exarg_T) {
    unsafe {
        if *fname != NUL && !eap.is_null() && (*eap).addr_count > 0 {
            emsg(gettext(e_norange.as_ptr()));
            return;
        }

        if !eap.is_null() && *fname == NUL {
            if (*eap).forceit != 0 {
                emsg(gettext(e_argreq.as_ptr()));
            } else {
                cmd_source_buffer(eap, false);
            }
        } else if !eap.is_null() && (*eap).forceit != 0 {
            openscript(
                fname,
                global_busy != 0
                    || listcmd_busy
                    || !(*eap).nextcmd.is_null()
                    || (*(*eap).cstack.cast::<cstack_T>()).cs_idx >= 0,
            );
        } else if do_source(fname, false, DOSO_NONE, ptr::null_mut()) == FAIL {
            semsg!(gettext(e_notopen.as_ptr()), fname);
        }
    }
}

/// ":source [{fname}]"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_source(eap: *mut exarg_T) {
    unsafe {
        cmd_source((*eap).arg, eap);
    }
}

/// ":options"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_options(_eap: *mut exarg_T) {
    unsafe {
        let mut buf = [0 as c_char; 500];
        let mut multi_mods = false;

        add_win_cmd_modifiers(buf.as_mut_ptr(), &raw mut cmdmod, &mut multi_mods);

        os_setenv(c"OPTWIN_CMD".as_ptr(), buf.as_ptr(), 1);
        cmd_source(
            c"$VIMRUNTIME/scripts/optwin.lua".as_ptr() as *mut c_char,
            ptr::null_mut(),
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_breakpoint(cookie: *mut c_void) -> *mut linenr_T {
    unsafe { &raw mut (*cookie.cast::<source_cookie_T>()).breakpoint }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_dbg_tick(cookie: *mut c_void) -> *mut c_int {
    unsafe { &raw mut (*cookie.cast::<source_cookie_T>()).dbg_tick }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_level(cookie: *mut c_void) -> c_int {
    unsafe { (*cookie.cast::<source_cookie_T>()).level }
}

unsafe fn fopen_noinh_readbin(filename: *const c_char) -> *mut FILE {
    unsafe {
        #[cfg(windows)]
        let fd_tmp = os_open(filename, O_RDONLY | O_BINARY | O_NOINHERIT, 0);
        #[cfg(not(windows))]
        let fd_tmp = os_open(filename, O_RDONLY, 0);

        if fd_tmp < 0 {
            return ptr::null_mut();
        }

        os_set_cloexec(fd_tmp);

        #[cfg(windows)]
        {
            _fdopen(fd_tmp, c"rb".as_ptr())
        }
        #[cfg(not(windows))]
        {
            fdopen(fd_tmp, c"rb".as_ptr())
        }
    }
}

unsafe fn concat_continued_line(
    ga: *mut garray_T,
    init_growsize: c_int,
    p: *const c_char,
    mut len: usize,
) -> bool {
    unsafe {
        let line = skipwhite_len(p, len);
        len -= (line as usize) - (p as usize);
        let slice = std::slice::from_raw_parts(line.cast::<u8>(), len);
        if slice.starts_with(b"\"\\ ") {
            return true;
        }
        if slice.is_empty() || slice[0] != b'\\' {
            return false;
        }
        if (*ga).ga_len > init_growsize {
            ga_set_growsize(ga, (*ga).ga_len.min(8000));
        }
        ga_concat_len(ga, line.add(1), len - 1);
        true
    }
}

/// Create a new script item and allocate script-local vars.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn new_script_item(
    name: *mut c_char,
    sid_out: *mut scid_T,
) -> *mut scriptitem_T {
    unsafe {
        last_current_SID += 1;
        let sid = last_current_SID;
        if !sid_out.is_null() {
            *sid_out = sid;
        }
        ga_grow(&raw mut script_items, sid - script_items.ga_len);
        while script_items.ga_len < sid {
            let si = xcalloc(1, std::mem::size_of::<scriptitem_T>()).cast::<scriptitem_T>();
            script_items.ga_len += 1;
            set_script_item(script_items.ga_len, si);
            (*si).sn_name = ptr::null_mut();

            new_script_vars(script_items.ga_len);

            (*si).sn_prof_on = false;
        }
        let item = script_item(sid);
        (*item).sn_name = name;
        item
    }
}

unsafe fn do_source_buffer_init(
    sp: *mut source_cookie_T,
    eap: *const exarg_T,
    ex_lua: bool,
) -> *mut c_char {
    unsafe {
        if curbuf.is_null() {
            return ptr::null_mut();
        }

        let iobuff = (&raw mut IObuff).cast::<c_char>();
        let fname = if !(*curbuf).b_ffname.is_null() {
            xstrdup((*curbuf).b_ffname)
        } else {
            if ex_lua {
                vim_snprintf!(
                    iobuff,
                    IOSIZE,
                    c":{range}lua buffer=%d".as_ptr(),
                    (*curbuf).b_fnum,
                );
            } else {
                vim_snprintf!(
                    iobuff,
                    IOSIZE,
                    c":source buffer=%d".as_ptr(),
                    (*curbuf).b_fnum,
                );
            }
            xstrdup(iobuff)
        };

        ga_init(
            &mut (*sp).buflines,
            std::mem::size_of::<*mut c_char>() as c_int,
            100,
        );
        for curr_lnum in (*eap).line1..=(*eap).line2 {
            let slot = ga_append_via_ptr(&mut (*sp).buflines, std::mem::size_of::<*mut c_char>())
                .cast::<*mut c_char>();
            *slot = xstrdup(ml_get(curr_lnum));
        }
        (*sp).buf_lnum = 0;
        (*sp).source_from_buf_or_str = true;
        (*sp).sourcing_lnum = (*eap).line1 - 1;

        fname
    }
}

unsafe fn do_source_str_init(sp: *mut source_cookie_T, mut str_ptr: *const c_char) {
    unsafe {
        ga_init(
            &mut (*sp).buflines,
            std::mem::size_of::<*mut c_char>() as c_int,
            100,
        );
        while *str_ptr != NUL {
            let eol = skip_to_newline(str_ptr);
            let slot = ga_append_via_ptr(&mut (*sp).buflines, std::mem::size_of::<*mut c_char>())
                .cast::<*mut c_char>();
            *slot = xmemdupz(
                str_ptr.cast::<c_void>(),
                (eol as usize) - (str_ptr as usize),
            )
            .cast::<c_char>();
            str_ptr = eol.add(usize::from(*eol != NUL));
        }
        (*sp).buf_lnum = 0;
        (*sp).source_from_buf_or_str = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cmd_source_buffer(eap: *const exarg_T, ex_lua: bool) {
    unsafe {
        do_source_ext(
            ptr::null_mut(),
            false,
            DOSO_NONE,
            ptr::null_mut(),
            eap,
            ex_lua,
            ptr::null(),
        );
    }
}

/// Executes lines in `str` as Ex commands.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_source_str(
    str_ptr: *const c_char,
    mut traceback_name: *const c_char,
) -> c_int {
    unsafe {
        let sname = sourcing_name();
        let slnum = sourcing_lnum();
        let mut sname_buf = [0 as c_char; 256];
        if !sname.is_null() {
            vim_snprintf!(
                sname_buf.as_mut_ptr(),
                sname_buf.len(),
                c"%s called at %s:%d".as_ptr(),
                traceback_name,
                sname,
                slnum,
            );
            traceback_name = sname_buf.as_ptr();
        }
        do_source_ext(
            traceback_name as *mut c_char,
            false,
            DOSO_NONE,
            ptr::null_mut(),
            ptr::null(),
            false,
            str_ptr,
        )
    }
}

unsafe fn do_source_ext(
    fname: *mut c_char,
    check_other: bool,
    is_vimrc: c_int,
    ret_sid: *mut c_int,
    eap: *const exarg_T,
    ex_lua: bool,
    str_ptr: *const c_char,
) -> c_int {
    unsafe {
        let mut cookie: source_cookie_T = std::mem::zeroed();
        let mut firstline: *mut u8 = ptr::null_mut();
        let mut retval = FAIL;
        let save_debug_break_level = debug_break_level;
        let mut si: *mut scriptitem_T = ptr::null_mut();
        let mut wait_start: proftime_T = 0;
        let mut trigger_source_post = false;

        let mut fname_exp: *mut c_char;
        if fname.is_null() {
            debug_assert!(str_ptr.is_null());
            fname_exp = do_source_buffer_init(&mut cookie, eap, ex_lua);
            if fname_exp.is_null() {
                return FAIL;
            }
        } else if !str_ptr.is_null() {
            do_source_str_init(&mut cookie, str_ptr);
            fname_exp = xstrdup(fname);
        } else {
            let p = expand_env_save(fname);
            if p.is_null() {
                return retval;
            }
            fname_exp = fix_fname(p);
            xfree(p.cast::<c_void>());
            if fname_exp.is_null() {
                return retval;
            }
            if os_isdir(fname_exp) {
                smsg!(
                    0,
                    gettext(c"Cannot source a directory: \"%s\"".as_ptr()),
                    fname,
                );
                xfree(fname_exp.cast::<c_void>());
                return retval;
            }
        }

        'block: {
            let mut sid = if !str_ptr.is_null() {
                SID_STR
            } else {
                find_script_by_name(fname_exp)
            };
            if sid > 0 && !ret_sid.is_null() {
                *ret_sid = sid;
                retval = OK;
                break 'block;
            }

            if str_ptr.is_null() {
                if has_autocmd(EVENT_SOURCECMD, fname_exp, ptr::null_mut())
                    && apply_autocmds(EVENT_SOURCECMD, fname_exp, fname_exp, false, curbuf)
                {
                    retval = if aborting() { FAIL } else { OK };
                    if retval == OK {
                        apply_autocmds(EVENT_SOURCEPOST, fname_exp, fname_exp, false, curbuf);
                    }
                    break 'block;
                }

                apply_autocmds(EVENT_SOURCEPRE, fname_exp, fname_exp, false, curbuf);
            }

            if !cookie.source_from_buf_or_str {
                cookie.fp = fopen_noinh_readbin(fname_exp);
            }
            if cookie.fp.is_null() && check_other {
                let p = path_tail(fname_exp);
                if *p == b'.' as c_char || *p == b'_' as c_char {
                    let rest = CStr::from_ptr(p.add(1)).to_bytes();
                    if rest.eq_ignore_ascii_case(b"nvimrc") || rest.eq_ignore_ascii_case(b"exrc") {
                        *p = if *p == b'_' as c_char {
                            b'.' as c_char
                        } else {
                            b'_' as c_char
                        };
                        cookie.fp = fopen_noinh_readbin(fname_exp);
                    }
                }
            }

            if cookie.fp.is_null() && !cookie.source_from_buf_or_str {
                if p_verbose > 1 {
                    verbose_enter();
                    if sourcing_name().is_null() {
                        smsg!(0, gettext(c"could not source \"%s\"".as_ptr()), fname);
                    } else {
                        smsg!(
                            0,
                            gettext(c"line %lld: could not source \"%s\"".as_ptr()),
                            i64::from(sourcing_lnum()),
                            fname,
                        );
                    }
                    verbose_leave();
                }
                break 'block;
            }

            if p_verbose > 1 {
                verbose_enter();
                if sourcing_name().is_null() {
                    smsg!(0, gettext(c"sourcing \"%s\"".as_ptr()), fname);
                } else {
                    smsg!(
                        0,
                        gettext(c"line %lld: sourcing \"%s\"".as_ptr()),
                        i64::from(sourcing_lnum()),
                        fname,
                    );
                }
                verbose_leave();
            }
            if is_vimrc == DOSO_VIMRC {
                vimrc_found(fname_exp, c"MYVIMRC".as_ptr().cast_mut());
            }

            #[cfg(windows)]
            {
                if *p_ffs == NUL {
                    cookie.fileformat = EOL_DOS;
                } else {
                    cookie.fileformat = EOL_UNKNOWN;
                }
            }

            cookie.breakpoint = dbg_find_breakpoint(true, fname_exp, 0);
            cookie.fname = fname_exp;
            cookie.dbg_tick = debug_tick;
            cookie.level = ex_nesting_level;

            let mut rel_time: proftime_T = 0;
            let mut start_time: proftime_T = 0;
            let l_time_fd = time_fd;
            if !l_time_fd.is_null() {
                time_push(&mut rel_time, &mut start_time);
            }

            let l_do_profiling = do_profiling;
            if l_do_profiling == PROF_YES {
                prof_child_enter(&mut wait_start);
            }

            let mut funccalp_entry = funccal_entry_T {
                top_funccal: ptr::null_mut(),
                next: ptr::null_mut(),
            };
            save_funccal(&mut funccalp_entry);

            let save_current_sctx = current_sctx;

            last_current_SID_seq += 1;
            current_sctx.sc_seq = last_current_SID_seq;

            if sid > 0 {
                si = script_item(sid);
            } else if str_ptr.is_null() {
                si = new_script_item(fname_exp, &mut sid);
                (*si).sn_lua = path_with_extension(fname_exp, c"lua".as_ptr());
                fname_exp = xstrdup((*si).sn_name);
                if !ret_sid.is_null() {
                    *ret_sid = sid;
                }
            }
            debug_assert_eq!(!si.is_null(), str_ptr.is_null());

            if str_ptr.is_null() || !script_is_lua(current_sctx.sc_sid) {
                current_sctx.sc_sid = sid;
                current_sctx.sc_lnum = 0;
            }

            estack_push(
                ETYPE_SCRIPT,
                if !si.is_null() {
                    (*si).sn_name
                } else {
                    fname_exp
                },
                0,
            );

            if l_do_profiling == PROF_YES && !si.is_null() {
                let mut forceit = false;
                if !(*si).sn_prof_on && has_profiling(true, (*si).sn_name, &mut forceit) {
                    profile_init(si);
                    (*si).sn_pr_force = forceit;
                }
                if (*si).sn_prof_on {
                    (*si).sn_pr_count += 1;
                    (*si).sn_pr_start = profile_start();
                    (*si).sn_pr_children = profile_zero();
                }
            }

            cookie.conv.vc_type = CONV_NONE;

            let mut ts_lua = false;
            if fname.is_null()
                && !eap.is_null()
                && !ex_lua
                && !strequal((*curbuf).b_p_ft, c"lua".as_ptr())
                && !(!(*curbuf).b_fname.is_null()
                    && path_with_extension((*curbuf).b_fname, c"lua".as_ptr()))
            {
                let mut args_items = [
                    integer_obj(Integer::from((*curbuf).b_fnum)),
                    integer_obj(Integer::from((*eap).line1)),
                    integer_obj(Integer::from((*eap).line2)),
                ];
                let args = Array {
                    items: args_items.as_mut_ptr(),
                    size: 3,
                    capacity: 3,
                };
                let mut err = Error {
                    err_type: ErrorType::kErrorTypeNone,
                    msg: ptr::null_mut(),
                };
                let result = nlua_exec(
                    cstr_literal(c"return require('vim._core.util').source_is_lua(...)"),
                    ptr::null(),
                    args,
                    kRetNilBool,
                    ptr::null_mut(),
                    &mut err,
                );
                if err.err_type == ErrorType::kErrorTypeNone
                    && result.obj_type == ObjectType::kObjectTypeBoolean
                    && result.data.boolean
                {
                    ts_lua = true;
                }
                api_clear_error(&mut err);
            }

            if fname.is_null()
                && (ex_lua
                    || ts_lua
                    || strequal((*curbuf).b_p_ft, c"lua".as_ptr())
                    || (!(*curbuf).b_fname.is_null()
                        && path_with_extension((*curbuf).b_fname, c"lua".as_ptr())))
            {
                nlua_exec_ga(&mut cookie.buflines, fname_exp);
            } else if !si.is_null() && (*si).sn_lua {
                nlua_exec_file(fname_exp);
            } else {
                firstline =
                    getsourceline(0, (&raw mut cookie).cast::<c_void>(), 0, true).cast::<u8>();
                if !firstline.is_null()
                    && CStr::from_ptr(firstline.cast::<c_char>()).to_bytes().len() >= 3
                    && *firstline == 0xef
                    && *firstline.add(1) == 0xbb
                    && *firstline.add(2) == 0xbf
                {
                    convert_setup(&mut cookie.conv, c"utf-8".as_ptr() as *mut c_char, p_enc);
                    let mut p = string_convert(
                        &cookie.conv,
                        firstline.add(3).cast::<c_char>(),
                        ptr::null_mut(),
                    );
                    if p.is_null() {
                        p = xstrdup(firstline.add(3).cast::<c_char>());
                    }
                    xfree(firstline.cast::<c_void>());
                    firstline = p.cast::<u8>();
                }
                do_cmdline(
                    firstline.cast::<c_char>(),
                    Some(getsourceline),
                    (&raw mut cookie).cast::<c_void>(),
                    DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_REPEAT,
                );
            }
            retval = OK;

            if l_do_profiling == PROF_YES && !si.is_null() {
                si = script_item(current_sctx.sc_sid);
                if (*si).sn_prof_on {
                    (*si).sn_pr_start = profile_end((*si).sn_pr_start);
                    (*si).sn_pr_start = profile_sub_wait(wait_start, (*si).sn_pr_start);
                    (*si).sn_pr_total = profile_add((*si).sn_pr_total, (*si).sn_pr_start);
                    (*si).sn_pr_self =
                        profile_self((*si).sn_pr_self, (*si).sn_pr_start, (*si).sn_pr_children);
                }
            }

            if got_int {
                emsg(gettext(e_interr.as_ptr()));
            }
            estack_pop();
            if p_verbose > 1 {
                verbose_enter();
                smsg!(0, gettext(c"finished sourcing %s".as_ptr()), fname);
                let sname = sourcing_name();
                if !sname.is_null() {
                    smsg!(0, gettext(c"continuing in %s".as_ptr()), sname);
                }
                verbose_leave();
            }

            if !l_time_fd.is_null() {
                let iobuff = (&raw mut IObuff).cast::<c_char>();
                vim_snprintf!(iobuff, IOSIZE, c"sourcing %s".as_ptr(), fname);
                time_msg(iobuff, &start_time);
                time_pop(rel_time);
            }

            if !got_int {
                trigger_source_post = true;
            }

            if save_debug_break_level > ex_nesting_level && debug_break_level == ex_nesting_level {
                debug_break_level += 1;
            }

            current_sctx = save_current_sctx;
            restore_funccal();
            if l_do_profiling == PROF_YES {
                prof_child_exit(&mut wait_start);
            }
            if !cookie.fp.is_null() {
                fclose(cookie.fp);
            }
            if cookie.source_from_buf_or_str {
                ga_clear_strings(&mut cookie.buflines);
            }
            xfree(cookie.nextline.cast::<c_void>());
            xfree(firstline.cast::<c_void>());
            convert_setup(&mut cookie.conv, ptr::null_mut(), ptr::null_mut());

            if str_ptr.is_null() && trigger_source_post {
                apply_autocmds(EVENT_SOURCEPOST, fname_exp, fname_exp, false, curbuf);
            }
        }

        xfree(fname_exp.cast::<c_void>());
        retval
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_source(
    fname: *mut c_char,
    check_other: bool,
    is_vimrc: c_int,
    ret_sid: *mut c_int,
) -> c_int {
    unsafe {
        do_source_ext(
            fname,
            check_other,
            is_vimrc,
            ret_sid,
            ptr::null(),
            false,
            ptr::null(),
        )
    }
}

/// Checks if the script with the given script ID is a Lua script.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn script_is_lua(sid: scid_T) -> bool {
    unsafe {
        if sid == SID_LUA {
            return true;
        }
        if !script_id_valid(sid) {
            return false;
        }
        (*script_item(sid)).sn_lua
    }
}

/// Find an already loaded script "name".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_script_by_name(name: *mut c_char) -> c_int {
    unsafe {
        debug_assert!(script_items.ga_len >= 0);
        let mut sid = script_items.ga_len;
        while sid > 0 {
            let si = script_item(sid);
            if !(*si).sn_name.is_null() && path_equal((*si).sn_name, name, kPathCmpLiteral) {
                return sid;
            }
            sid -= 1;
        }
        -1
    }
}

/// ":scriptnames"
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_scriptnames(eap: *mut exarg_T) {
    unsafe {
        let namebuff = (&raw mut NameBuff).cast::<c_char>();
        let iobuff = (&raw mut IObuff).cast::<c_char>();
        if (*eap).addr_count > 0 || *(*eap).arg != NUL {
            if (*eap).addr_count > 0 && !script_id_valid((*eap).line2) {
                emsg(gettext(e_invarg.as_ptr()));
            } else {
                if (*eap).addr_count > 0 {
                    (*eap).arg = (*script_item((*eap).line2)).sn_name;
                } else {
                    expand_env((*eap).arg, namebuff, MAXPATHL as c_int);
                    (*eap).arg = namebuff;
                    to_slash((*eap).arg);
                }
                do_exedit(eap, ptr::null_mut());
            }
            return;
        }

        msg_ext_set_kind(c"list_cmd".as_ptr());
        let mut i = 1;
        while i <= script_items.ga_len && !got_int {
            let si = script_item(i);
            if !(*si).sn_name.is_null() {
                home_replace(ptr::null_mut(), (*si).sn_name, namebuff, MAXPATHL, true);
                vim_snprintf!(iobuff, IOSIZE, c"%3d: %s".as_ptr(), i, namebuff);
                if !message_filtered(iobuff) {
                    if msg_col > 0 {
                        msg_putchar(b'\n' as c_int);
                    }
                    msg_outtrans(iobuff, 0, false);
                    line_breakcheck();
                }
            }
            i += 1;
        }
    }
}

/// Get a pointer to a script name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_scriptname(script_ctx: sctx_T, should_free: *mut bool) -> *mut c_char {
    unsafe {
        if !should_free.is_null() {
            *should_free = false;
        }

        let iobuff = (&raw mut IObuff).cast::<c_char>();
        match script_ctx.sc_sid {
            SID_MODELINE => gettext(c"modeline".as_ptr()),
            SID_CMDARG => gettext(c"--cmd argument".as_ptr()),
            SID_CARG => gettext(c"-c argument".as_ptr()),
            SID_ENV => gettext(c"environment variable".as_ptr()),
            SID_ERROR => gettext(c"error handler".as_ptr()),
            SID_WINLAYOUT => gettext(c"changed window size".as_ptr()),
            SID_LUA => gettext(c"Lua".as_ptr()),
            SID_API_CLIENT => {
                vim_snprintf!(
                    iobuff,
                    IOSIZE,
                    gettext(c"API client (channel id %llu)".as_ptr()),
                    script_ctx.sc_chan,
                );
                iobuff
            }
            SID_STR => gettext(c"anonymous :source".as_ptr()),
            _ => {
                let sname = (*script_item(script_ctx.sc_sid)).sn_name;
                if sname.is_null() {
                    vim_snprintf!(
                        iobuff,
                        IOSIZE,
                        gettext(c"anonymous :source (script id %d)".as_ptr()),
                        script_ctx.sc_sid,
                    );
                    return iobuff;
                }
                if !should_free.is_null() {
                    *should_free = true;
                    home_replace_save(ptr::null_mut(), sname)
                } else {
                    sname
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_scriptnames() {
    unsafe {
        profile_reset();

        for i in 1..=script_items.ga_len {
            let si = script_item(i);
            xfree((*si).sn_vars);
            xfree((*si).sn_name.cast::<c_void>());
            ga_clear(&mut (*si).sn_prl_ga);
            xfree(si.cast::<c_void>());
        }
        ga_clear(&raw mut script_items);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_autoload_scriptnames() {
    unsafe {
        ga_clear_strings(&raw mut ga_loaded);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_sourced_lnum(fgetline: LineGetter, cookie: *mut c_void) -> linenr_T {
    unsafe {
        if fgetline.is_some_and(|f| {
            std::ptr::fn_addr_eq(
                f,
                getsourceline
                    as unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char,
            )
        }) {
            (*cookie.cast::<source_cookie_T>()).sourcing_lnum
        } else {
            sourcing_lnum()
        }
    }
}

unsafe fn get_script_local_funcs(sid: scid_T) -> *mut list_T {
    unsafe {
        let functbl = func_tbl_get();
        let l = tv_list_alloc((*functbl).ht_used as isize);

        let mut todo = (*functbl).ht_used;
        let mut hi = (*functbl).ht_array;
        while todo > 0 {
            if !hashitem_empty(&*hi) {
                todo -= 1;
                let fp = ((*hi).hi_key as *const u8)
                    .sub(std::mem::offset_of!(ufunc_T, uf_name))
                    .cast::<ufunc_T>();
                if (*fp).uf_script_ctx.sc_sid == sid {
                    let name = if !(*fp).uf_name_exp.is_null() {
                        (*fp).uf_name_exp
                    } else {
                        (*fp).uf_name.as_ptr()
                    };
                    tv_list_append_string(l, name, -1);
                }
            }
            hi = hi.add(1);
        }

        l
    }
}

/// "getscriptinfo()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getscriptinfo(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, script_items.ga_len as isize);

        if tv_check_for_opt_dict_arg(argvars, 0) == FAIL {
            return;
        }

        let l = (*rettv).vval.v_ptr.cast::<list_T>();

        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.rm_ic = p_ic != 0;
        let mut filterpat = false;
        let mut sid: varnumber_T = -1;

        let mut pat: *mut c_char = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_DICT {
            let arg_dict = (*argvars).vval.v_ptr.cast::<dict_T>();
            let sid_di = tv_dict_find(arg_dict, c"sid".as_ptr(), 3);
            if !sid_di.is_null() {
                let mut error = false;
                sid = tv_get_number_chk(&(*sid_di).di_tv, &mut error);
                if error {
                    return;
                }
                if sid <= 0 {
                    semsg!(
                        gettext(e_invargNval.as_ptr()),
                        c"sid".as_ptr(),
                        tv_get_string(&(*sid_di).di_tv),
                    );
                    return;
                }
            } else {
                pat = tv_dict_get_string(arg_dict, c"name".as_ptr(), true);
                if !pat.is_null() {
                    regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
                }
                if !regmatch.regprog.is_null() {
                    filterpat = true;
                }
            }
        }

        let mut i = if sid > 0 { sid } else { 1 };
        while (i == sid || sid <= 0) && i <= varnumber_T::from(script_items.ga_len) {
            let si = script_item(i as scid_T);

            if !(*si).sn_name.is_null()
                && (!filterpat || vim_regexec(&mut regmatch, (*si).sn_name, 0))
            {
                let d = tv_dict_alloc();
                tv_list_append_dict(l, d);
                tv_dict_add_str(d, c"name".as_ptr(), 4, (*si).sn_name);
                tv_dict_add_nr(d, c"sid".as_ptr(), 3, i);
                tv_dict_add_nr(d, c"version".as_ptr(), 7, 1);
                tv_dict_add_bool(d, c"autoload".as_ptr(), 8, kFalse);

                if sid > 0 {
                    let var_dict = tv_dict_copy(
                        ptr::null(),
                        &mut (*(*si).sn_vars.cast::<scriptvar_T>()).sv_dict,
                        true,
                        get_copyID(),
                    );
                    tv_dict_add_dict(d, c"variables".as_ptr(), 9, var_dict);
                    tv_dict_add_list(
                        d,
                        c"functions".as_ptr(),
                        9,
                        get_script_local_funcs(sid as scid_T),
                    );
                }
            }

            i += 1;
        }

        vim_regfree(regmatch.regprog);
        xfree(pat.cast::<c_void>());
    }
}

/// Get one full line from a sourced file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn getsourceline(
    _c: c_int,
    cookie: *mut c_void,
    _indent: c_int,
    do_concat: bool,
) -> *mut c_char {
    unsafe {
        let sp = cookie.cast::<source_cookie_T>();

        if (*sp).dbg_tick < debug_tick && !(*sp).source_from_buf_or_str {
            (*sp).breakpoint = dbg_find_breakpoint(true, (*sp).fname, sourcing_lnum());
            (*sp).dbg_tick = debug_tick;
        }
        if do_profiling == PROF_YES {
            script_line_end();
        }
        set_sourcing_lnum((*sp).sourcing_lnum + 1);
        let mut line = if (*sp).finished || (!(*sp).source_from_buf_or_str && (*sp).fp.is_null()) {
            ptr::null_mut()
        } else if (*sp).nextline.is_null() {
            get_one_sourceline(sp)
        } else {
            let l = (*sp).nextline;
            (*sp).nextline = ptr::null_mut();
            (*sp).sourcing_lnum += 1;
            l
        };
        if !line.is_null() && do_profiling == PROF_YES {
            script_line_start();
        }

        if !line.is_null() && do_concat && vim_strchr(p_cpo, kCpoConcat).is_null() {
            (*sp).sourcing_lnum -= 1;

            (*sp).nextline = get_one_sourceline(sp);
            if !(*sp).nextline.is_null() {
                let p = skipwhite((*sp).nextline);
                if *p == b'\\' as c_char
                    || (*p == b'"' as c_char
                        && *p.add(1) == b'\\' as c_char
                        && *p.add(2) == b' ' as c_char)
                {
                    let mut ga: garray_T = std::mem::zeroed();
                    ga_init(&mut ga, 1, 400);
                    ga_concat(&mut ga, line);
                    while !(*sp).nextline.is_null()
                        && concat_continued_line(
                            &mut ga,
                            400,
                            (*sp).nextline,
                            CStr::from_ptr((*sp).nextline).to_bytes().len(),
                        )
                    {
                        xfree((*sp).nextline.cast::<c_void>());
                        (*sp).nextline = get_one_sourceline(sp);
                    }
                    ga_append(&mut ga, 0);
                    xfree(line.cast::<c_void>());
                    line = ga.ga_data.cast::<c_char>();
                }
            }
        }

        if !line.is_null() && (*sp).conv.vc_type != CONV_NONE {
            let s = string_convert(&(*sp).conv, line, ptr::null_mut());
            if !s.is_null() {
                xfree(line.cast::<c_void>());
                line = s;
            }
        }

        if !(*sp).source_from_buf_or_str
            && (*sp).breakpoint != 0
            && (*sp).breakpoint <= sourcing_lnum()
        {
            dbg_breakpoint((*sp).fname, sourcing_lnum());
            (*sp).breakpoint = dbg_find_breakpoint(true, (*sp).fname, sourcing_lnum());
            (*sp).dbg_tick = debug_tick;
        }

        line
    }
}

unsafe fn get_one_sourceline(sp: *mut source_cookie_T) -> *mut c_char {
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();
        let mut have_read = false;

        ga_init(&mut ga, 1, 250);

        (*sp).sourcing_lnum += 1;
        loop {
            ga_grow(&mut ga, 120);
            #[cfg(windows)]
            let mut len: c_int;
            #[cfg(not(windows))]
            let len: c_int;
            let buf: *mut c_char;
            if (*sp).source_from_buf_or_str {
                if (*sp).buf_lnum >= (*sp).buflines.ga_len {
                    break;
                }
                ga_concat(
                    &mut ga,
                    *((*sp).buflines.ga_data as *mut *mut c_char).add((*sp).buf_lnum as usize),
                );
                (*sp).buf_lnum += 1;
                ga_grow(&mut ga, 1);
                buf = ga.ga_data.cast::<c_char>();
                *buf.add(ga.ga_len as usize) = NUL;
                ga.ga_len += 1;
                len = ga.ga_len;
            } else {
                buf = ga.ga_data.cast::<c_char>();
                let mut eof = false;
                loop {
                    *errno_ptr() = 0;
                    if fgets(
                        buf.add(ga.ga_len as usize),
                        ga.ga_maxlen - ga.ga_len,
                        (*sp).fp,
                    )
                    .is_null()
                    {
                        if *errno_ptr() == EINTR {
                            continue;
                        }
                        eof = true;
                    }
                    break;
                }
                if eof {
                    break;
                }
                len = ga.ga_len
                    + CStr::from_ptr(buf.add(ga.ga_len as usize)).to_bytes().len() as c_int;
            }

            #[cfg(windows)]
            {
                if (len == 1 || (len >= 2 && *buf.add((len - 2) as usize) == b'\n' as c_char))
                    && (*sp).fileformat == EOL_DOS
                    && *buf.add((len - 1) as usize) == Ctrl_Z
                {
                    *buf.add((len - 1) as usize) = NUL;
                    break;
                }
            }

            have_read = true;
            ga.ga_len = len;

            if !(*sp).source_from_buf_or_str
                && ga.ga_maxlen - ga.ga_len == 1
                && *buf.add((len - 1) as usize) != b'\n' as c_char
            {
                continue;
            }

            if len >= 1 && *buf.add((len - 1) as usize) == b'\n' as c_char {
                #[cfg(windows)]
                {
                    let has_cr = len >= 2 && *buf.add((len - 2) as usize) == b'\r' as c_char;
                    if (*sp).fileformat == EOL_UNKNOWN {
                        (*sp).fileformat = if has_cr { EOL_DOS } else { EOL_UNIX };
                    }

                    if (*sp).fileformat == EOL_DOS {
                        if has_cr {
                            *buf.add((len - 2) as usize) = b'\n' as c_char;
                            len -= 1;
                            ga.ga_len -= 1;
                        } else {
                            if !(*sp).error {
                                msg_source(*hl_attr_active.add(HLF_W));
                                emsg(gettext(
                                    c"W15: Warning: Wrong line separator, ^M may be missing"
                                        .as_ptr(),
                                ));
                            }
                            (*sp).error = true;
                            (*sp).fileformat = EOL_UNIX;
                        }
                    }
                }

                let mut c = len - 2;
                while c >= 0 && *buf.add(c as usize) == Ctrl_V {
                    c -= 1;
                }
                if (len & 1) != (c & 1) {
                    (*sp).sourcing_lnum += 1;
                    continue;
                }

                *buf.add((len - 1) as usize) = NUL;
            }

            line_breakcheck();
            break;
        }

        if have_read {
            return ga.ga_data.cast::<c_char>();
        }

        xfree(ga.ga_data);
        ptr::null_mut()
    }
}

/// Returns true if sourcing a script either from a file or a buffer or a string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn sourcing_a_script(eap: *mut exarg_T) -> c_int {
    unsafe {
        let fgetline: LineGetter = (*eap).ea_getline;
        c_int::from(getline_equal(fgetline, (*eap).cookie, Some(getsourceline)))
    }
}

/// ":scriptencoding": Set encoding conversion for a sourced script.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_scriptencoding(eap: *mut exarg_T) {
    unsafe {
        if sourcing_a_script(eap) == 0 {
            emsg(gettext(
                c"E167: :scriptencoding used outside of a sourced file".as_ptr(),
            ));
            return;
        }

        let name = if *(*eap).arg != NUL {
            enc_canonize((*eap).arg)
        } else {
            (*eap).arg
        };

        let fgetline: LineGetter = (*eap).ea_getline;
        let sp = getline_cookie(fgetline, (*eap).cookie).cast::<source_cookie_T>();
        convert_setup(&mut (*sp).conv, name, p_enc);

        if !ptr::eq(name, (*eap).arg) {
            xfree(name.cast::<c_void>());
        }
    }
}

/// ":finish": Mark a sourced file as finished.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_finish(eap: *mut exarg_T) {
    unsafe {
        if sourcing_a_script(eap) != 0 {
            do_finish(eap, false);
        } else {
            emsg(gettext(
                c"E168: :finish used outside of a sourced file".as_ptr(),
            ));
        }
    }
}

/// Mark a sourced file as finished.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_finish(eap: *mut exarg_T, reanimate: bool) {
    unsafe {
        let fgetline: LineGetter = (*eap).ea_getline;
        if reanimate {
            (*getline_cookie(fgetline, (*eap).cookie).cast::<source_cookie_T>()).finished = false;
        }

        let cstack = (*eap).cstack.cast::<cstack_T>();
        let idx = cleanup_conditionals(cstack, 0, 1);
        if idx >= 0 {
            (*cstack).cs_pending[idx as usize] = CSTP_FINISH as c_char;
            report_make_pending(CSTP_FINISH, ptr::null_mut());
        } else {
            (*getline_cookie(fgetline, (*eap).cookie).cast::<source_cookie_T>()).finished = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn source_finished(fgetline: LineGetter, cookie: *mut c_void) -> bool {
    unsafe {
        getline_equal(fgetline, cookie, Some(getsourceline))
            && (*getline_cookie(fgetline, cookie).cast::<source_cookie_T>()).finished
    }
}

/// Return the autoload script name for a function or variable name.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn autoload_name(name: *const c_char, name_len: usize) -> *mut c_char {
    unsafe {
        const PREFIX: &[u8] = b"autoload/";
        const SUFFIX: &[u8] = b".vim\0";
        let scriptname = xmalloc(name_len + PREFIX.len() + SUFFIX.len()).cast::<u8>();
        ptr::copy_nonoverlapping(PREFIX.as_ptr(), scriptname, PREFIX.len());
        ptr::copy_nonoverlapping(name.cast::<u8>(), scriptname.add(PREFIX.len()), name_len);
        let mut auchar_idx: usize = 0;
        let mut i = PREFIX.len();
        while i - PREFIX.len() < name_len {
            if *scriptname.add(i) == AUTOLOAD_CHAR {
                *scriptname.add(i) = b'/';
                auchar_idx = i;
            }
            i += 1;
        }
        ptr::copy_nonoverlapping(SUFFIX.as_ptr(), scriptname.add(auchar_idx), SUFFIX.len());

        scriptname.cast::<c_char>()
    }
}

/// If name has a package name try autoloading the script for it.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn script_autoload(
    name: *const c_char,
    name_len: usize,
    reload: bool,
) -> bool {
    unsafe {
        let slice = std::slice::from_raw_parts(name.cast::<u8>(), name_len);
        let Some(pos) = slice.iter().position(|&b| b == AUTOLOAD_CHAR) else {
            return false;
        };
        if pos == 0 {
            return false;
        }

        let mut ret = false;
        let mut tofree = autoload_name(name, name_len);
        let scriptname = tofree;

        let mut i: c_int = 0;
        let target_tail = CStr::from_ptr(scriptname.add(9)).to_bytes();
        while i < ga_loaded.ga_len {
            let loaded_entry = *(ga_loaded.ga_data as *mut *mut c_char).add(i as usize);
            if CStr::from_ptr(loaded_entry.add(9)).to_bytes() == target_tail {
                break;
            }
            i += 1;
        }
        if !reload && i < ga_loaded.ga_len {
            ret = false;
        } else {
            if i == ga_loaded.ga_len {
                let slot =
                    ga_append_via_ptr(&raw mut ga_loaded, std::mem::size_of::<*mut c_char>())
                        .cast::<*mut c_char>();
                *slot = scriptname;
                tofree = ptr::null_mut();
            }

            let mut ret_sid: c_int = 0;
            if do_in_runtimepath(
                scriptname,
                DIP_START,
                Some(source_callback),
                (&raw mut ret_sid).cast::<c_void>(),
            ) == OK
            {
                ret = true;
            }
        }

        xfree(tofree.cast::<c_void>());
        ret
    }
}
