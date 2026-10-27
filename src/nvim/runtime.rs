//! Management of runtime files (including packages) (`src/nvim/runtime.c`).

use crate::api::private::helpers::{
    SID_API_CLIENT, SID_LUA, add_c, api_clear_error, arena_array, arena_dict, arena_string,
    boolean_obj, cstr_as_obj, cstr_as_string, dict_obj, integer_obj, put_c, static_cstr_as_string,
    string_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString, ObjectType};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_SOURCECMD, EVENT_SOURCEPOST, EVENT_SOURCEPRE,
    PROF_YES, apply_autocmds, has_autocmd, has_autocmd_fullname,
};
use crate::charset::{
    Ctrl_V, skip_to_newline, skiptowhite, skiptowhite_esc, skipwhite, skipwhite_len,
};
use crate::cmdexpand::{EXPAND_RUNTIME, LineGetter, WILD_ADD_SLASH, globpath};
use crate::cursor::linenr_T;
use crate::debugger::{dbg_breakpoint, dbg_find_breakpoint, has_profiling};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, ScopeDictDictItem, TypvalVval, VAR_LOCKED, VarType, dict_T, list_T,
    tv_check_for_opt_dict_arg, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_func,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_copy, tv_dict_find, tv_get_number_chk, tv_get_string,
    tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict, tv_list_append_string,
    tv_list_append_tv, typval_T, varnumber_T,
};
use crate::eval::userfunc::{func_tbl_get, restore_funccal, save_funccal, ufunc_T};
use crate::eval::vars::{SID_STR, new_script_vars, scid_T};
use crate::eval::{eval_to_number, get_copyID};
#[cfg(windows)]
use crate::ex_docmd::Ctrl_Z;
use crate::ex_docmd::{do_cmdline, do_cmdline_cmd, do_exedit, exarg_T};
use crate::ex_eval::{
    CSTP_FINISH, PendingVal, aborting, cleanup_conditionals, cstack_T, report_make_pending,
};
use crate::fileio::{EINTR, FILE, O_RDONLY, fdopen, fgets};
#[cfg(windows)]
use crate::fileio::{EOL_DOS, EOL_UNIX, EOL_UNKNOWN};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::input::openscript;
use crate::lua::executor::{
    kRetNilBool, nlua_exec, nlua_exec_file, nlua_exec_lines, nlua_is_deferred_safe,
};
use crate::math::{FAIL, OK};
use crate::mbyte::{
    convert_setup, enc_canonize, mb_prevptr, string_convert, utfc_ptr2len, vimconv_T,
};
use crate::memline::ml_get;
use crate::memory::{
    Arena, cstr_from_bytes, cstr_to_cstring, xstrlcat, xstrlcpy,
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
use crate::optionstr::optset_T;
use crate::os::env::{
    expand_T, expand_env, expand_env_save, home_replace, home_replace_save, os_setenv,
    vim_env_iter, vim_env_iter_rev, vim_get_prefix_from_exepath, vim_getenv,
};
#[cfg(windows)]
use crate::os::fs::O_BINARY;
use crate::os::fs::{errno_ptr, os_file_is_readable, os_isdir, os_open, os_set_cloexec};
use crate::os::input::line_breakcheck;
use crate::os::stdpaths::{XDGVarType, get_appname, stdpaths_get_xdg_var};
use crate::path::{
    ENV_SEPCHAR, EW_DIR, EW_FILE, EW_NOBREAK, PATHSEP, add_pathsep, after_pathsep, append_path,
    fix_fname, gen_expand_wildcards_vec, get_past_head, kPathCmpLiteral, path_cmp, path_equal,
    path_tail, path_with_extension, to_slash, vim_ispathsep, vim_ispathsep_nocolon,
};
use crate::profile::{
    estack_T, prof_child_enter, prof_child_exit, profile_add, profile_end, profile_init,
    profile_reset, profile_self, profile_start, profile_sub_wait, profile_zero, proftime_T,
    script_line_end, script_line_start, scriptitem_T, time_msg, time_pop, time_push,
};
use crate::regexp::{RE_MAGIC, RE_STRING, regmatch_T, vim_regcomp, vim_regexec, vim_regfree};
use crate::strings::{gettext, vim_snprintf, vim_snprintf_safelen, vim_strchr};
use crate::usercmd::add_win_cmd_modifiers;
use crate::viml::parser::expressions::AUTOLOAD_CHAR;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;
use std::sync::Mutex;

pub type etype_T = c_int;
pub type estack_arg_T = c_int;
pub(crate) const SID_NONE: scid_T = -6;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct sctx_T {
    pub sc_sid: c_int,
    pub sc_seq: c_int,
    pub sc_lnum: i32,
    pub sc_chan: u64,
}

impl sctx_T {
    pub const fn new() -> Self {
        Self {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        }
    }
}
use crate::extmark::TriState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PackAddAction {
    AddDir,
    Load,
    Both,
}

pub enum RuntimeCallback<'a> {
    Source { skip_if_loaded: bool },
    SourceVimLua,
    AddPackStartDir,
    AddPackPlugins { opt: bool, action: PackAddAction },
    FindRuntime(&'a mut Vec<std::ffi::CString>),
    SpellLoad(&'a mut crate::spell::spelload_T),
    FoundTagfile,
}

impl RuntimeCallback<'_> {
    unsafe fn invoke(&mut self, fnames: &[std::ffi::CString], all: bool) {
        unsafe {
            match self {
                Self::Source { skip_if_loaded } => {
                    let mut sid: c_int = 0;
                    let cookie = if *skip_if_loaded {
                        &raw mut sid
                    } else {
                        ptr::null_mut()
                    };
                    let _ = source_callback(fnames, all, cookie);
                }
                Self::SourceVimLua => {
                    let _ = source_callback_vim_lua(fnames, all, ptr::null_mut());
                }
                Self::AddPackStartDir => {
                    let _ = add_pack_start_dir(fnames, all);
                }
                Self::AddPackPlugins { opt, action } => {
                    add_pack_plugins(*opt, fnames, all, *action);
                }
                Self::FindRuntime(out) => {
                    for fname in fnames {
                        out.push(fname.clone());
                        if !all {
                            break;
                        }
                    }
                }
                Self::SpellLoad(slp) => {
                    crate::spell::spell_load_cb(fnames, all, slp);
                }
                Self::FoundTagfile => {
                    crate::tag::found_tagfile_cb(fnames, all);
                }
            }
        }
    }
}

pub const ETYPE_TOP: etype_T = 0;
pub const ETYPE_SCRIPT: etype_T = 1;
pub const ETYPE_UFUNC: etype_T = 2;
pub const ETYPE_AUCMD: etype_T = 3;
pub const ETYPE_MODELINE: etype_T = 4;
pub const ETYPE_EXCEPT: etype_T = 5;
pub const ETYPE_ARGS: etype_T = 6;
pub const ETYPE_ENV: etype_T = 7;
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

#[cfg(windows)]
const HLF_W: usize = 26;

#[cfg(windows)]
const O_NOINHERIT: c_int = 0x0080;

#[repr(C)]
pub struct scriptvar_T {
    pub sv_var: ScopeDictDictItem,
    pub sv_dict: dict_T,
}

impl scriptvar_T {
    pub const fn new() -> Self {
        Self {
            sv_var: ScopeDictDictItem::new(),
            sv_dict: dict_T::new(),
        }
    }
}

pub struct source_cookie_T {
    fp: Option<crate::os::fs::CFile>,
    nextline: Option<Vec<c_char>>,
    sourcing_lnum: linenr_T,
    finished: bool,
    source_from_buf_or_str: bool,
    buf_lnum: c_int,
    buflines: Vec<std::ffi::CString>,
    #[cfg(windows)]
    fileformat: c_int,
    #[cfg(windows)]
    error: bool,
    breakpoint: linenr_T,
    fname: Option<std::ffi::CString>,
    dbg_tick: c_int,
    level: c_int,
    conv: vimconv_T,
}

impl source_cookie_T {
    const fn new() -> Self {
        Self {
            fp: None,
            nextline: None,
            sourcing_lnum: 0,
            finished: false,
            source_from_buf_or_str: false,
            buf_lnum: 0,
            buflines: Vec::new(),
            #[cfg(windows)]
            fileformat: 0,
            #[cfg(windows)]
            error: false,
            breakpoint: 0,
            fname: None,
            dbg_tick: 0,
            level: 0,
            conv: vimconv_T {
                vc_type: 0,
                vc_factor: 0,
                vc_fd: None,
                vc_fail: false,
            },
        }
    }
}

#[repr(C)]
#[derive(Clone)]
struct SearchPathItem {
    path: std::ffi::CString,
    after: bool,
    pack_inserted: bool,
    has_lua: TriState,
    pos_in_rtp: usize,
}

type RuntimeSearchPath = Vec<SearchPathItem>;

static RUNTIME_SEARCH_PATH_MUTEX: Mutex<()> = Mutex::new(());

pub(crate) struct RuntimeModuleState {
    pub(crate) exestack: Vec<estack_T>,
    pub(crate) script_items: Vec<scriptitem_T>,
    ga_loaded: Vec<std::ffi::CString>,
    last_current_SID_seq: c_int,
    last_current_SID: scid_T,
    runtime_search_path_valid: bool,
    runtime_search_path_valid_thread: bool,
    runtime_search_path_refcount: usize,
    runtime_search_path_old: Vec<RuntimeSearchPath>,
    runtime_search_path: RuntimeSearchPath,
    runtime_search_path_thread: RuntimeSearchPath,
    runtime_expand_flags: c_int,
    get_named_buf: [c_char; MAXPATHL],
    get_named_thread_buf: [c_char; MAXPATHL],
    pack_entry_buf: [c_char; MAXPATHL],
    build_rtp_buf: [c_char; MAXPATHL],
    pack_start_buf: [c_char; MAXPATHL],
}

impl RuntimeModuleState {
    const fn new() -> Self {
        Self {
            exestack: Vec::new(),
            script_items: Vec::new(),
            ga_loaded: Vec::new(),
            last_current_SID_seq: 0,
            last_current_SID: 0,
            runtime_search_path_valid: false,
            runtime_search_path_valid_thread: false,
            runtime_search_path_refcount: 0,
            runtime_search_path_old: Vec::new(),
            runtime_search_path: Vec::new(),
            runtime_search_path_thread: Vec::new(),
            runtime_expand_flags: 0,
            get_named_buf: [0; MAXPATHL],
            get_named_thread_buf: [0; MAXPATHL],
            pack_entry_buf: [0; MAXPATHL],
            build_rtp_buf: [0; MAXPATHL],
            pack_start_buf: [0; MAXPATHL],
        }
    }
}

static RT_STATE: GlobalState<RuntimeModuleState> = GlobalState::new(RuntimeModuleState::new());

#[inline]
pub(crate) unsafe fn rt_st() -> &'static mut RuntimeModuleState {
    unsafe { &mut *RT_STATE.as_ptr() }
}

use crate::buffer::{SID_MODELINE, VAR_FIXED};
use crate::charset::NUL;
use crate::extmark::{kFalse, kNone, kTrue};
use crate::os::fs::MAXPATHL;
use crate::version::default_lib_dir;
use crate::viml::parser::parser::CONV_NONE;

#[inline]
pub unsafe fn script_item(id: scid_T) -> *mut scriptitem_T {
    unsafe {
        let items = &raw mut rt_st().script_items;
        &raw mut (*items).as_mut_slice()[(id - 1) as usize]
    }
}

#[inline]
pub unsafe fn script_items_len() -> scid_T {
    unsafe {
        let items = &raw const rt_st().script_items;
        (*items).len() as scid_T
    }
}

#[inline]
pub unsafe fn script_id_valid(id: scid_T) -> bool {
    unsafe { id > 0 && id <= script_items_len() }
}

#[inline]
pub unsafe fn sourcing_name() -> *mut c_char {
    unsafe {
        let es = &raw const rt_st().exestack;
        (*es).last().unwrap().name_ptr()
    }
}

#[inline]
pub unsafe fn sourcing_lnum() -> linenr_T {
    unsafe {
        let es = &raw const rt_st().exestack;
        (*es).last().unwrap().es_lnum
    }
}

#[inline]
pub unsafe fn set_sourcing_lnum(lnum: linenr_T) {
    unsafe {
        let es = &raw mut rt_st().exestack;
        (*es).last_mut().unwrap().es_lnum = lnum;
    }
}

/// Initialize the execution stack.
pub unsafe fn estack_init() {
    unsafe {
        let es = &raw mut rt_st().exestack;
        (*es).reserve(10);
        (*es).push(estack_T {
            es_type: ETYPE_TOP,
            es_name: None,
            es_lnum: 0,
            es_info: crate::profile::EStackInfo::None,
        });
    }
}

/// Add an item to the execution stack.
pub unsafe fn estack_push(type_: etype_T, name: *const c_char, lnum: linenr_T) -> *mut estack_T {
    unsafe {
        let es = &raw mut rt_st().exestack;
        (*es).push(estack_T {
            es_type: type_,
            es_name: if name.is_null() {
                None
            } else {
                Some(CStr::from_ptr(name).to_owned())
            },
            es_lnum: lnum,
            es_info: crate::profile::EStackInfo::None,
        });
        (*es).last_mut().map_or(ptr::null_mut(), ptr::from_mut)
    }
}

/// Add a user function to the execution stack.
pub unsafe fn estack_push_ufunc(ufunc: *mut ufunc_T, lnum: linenr_T) {
    unsafe {
        let name = if let Some(ref exp) = (*ufunc).uf_name_exp {
            exp.as_ptr()
        } else {
            (*ufunc).uf_name.as_ptr()
        };
        let entry = estack_push(ETYPE_UFUNC, name, lnum);
        if !entry.is_null() {
            (*entry).es_info = crate::profile::EStackInfo::Ufunc(ufunc);
        }
    }
}

/// Take an item off of the execution stack.
pub unsafe fn estack_pop() {
    unsafe {
        let es = &raw mut rt_st().exestack;
        if (*es).len() > 1 {
            (*es).pop();
        }
    }
}

/// Get the current value for <sfile> as an owned `CString`.
pub unsafe fn estack_sfile(which: estack_arg_T) -> Option<std::ffi::CString> {
    unsafe {
        let es = &raw const rt_st().exestack;
        let last_entry = (*es).last().unwrap();
        if which == ESTACK_SFILE && last_entry.es_type != ETYPE_UFUNC {
            return last_entry.es_name.clone();
        }

        if which == ESTACK_SCRIPT {
            for entry in (*es).iter().rev() {
                if entry.es_type == ETYPE_UFUNC || entry.es_type == ETYPE_AUCMD {
                    let def_ctx = if entry.es_type == ETYPE_UFUNC {
                        (*entry.es_info.ufunc()).uf_script_ctx
                    } else {
                        entry.es_info.aucmd_sctx()
                    };
                    return if def_ctx.sc_sid > 0 {
                        Some(cstr_to_cstring((*script_item(def_ctx.sc_sid)).name_ptr()))
                    } else {
                        None
                    };
                } else if entry.es_type == ETYPE_SCRIPT {
                    return entry.es_name.clone();
                }
            }
            return None;
        }

        let mut buf: Vec<c_char> = Vec::new();
        let mut last_type = ETYPE_SCRIPT;
        let es_len = (*es).len();
        for (idx, entry) in (*es).iter().enumerate() {
            if let Some(ref entry_name) = entry.es_name {
                let mut type_name = static_cstr_as_string(c"");
                let es_name = cstr_as_string(entry_name.as_ptr());
                if entry.es_type != last_type {
                    match entry.es_type {
                        ETYPE_SCRIPT => type_name = static_cstr_as_string(c"script "),
                        ETYPE_UFUNC => type_name = static_cstr_as_string(c"function "),
                        _ => {}
                    }
                    last_type = entry.es_type;
                }
                let lnum = if idx == es_len - 1 {
                    if which == ESTACK_STACK {
                        sourcing_lnum()
                    } else {
                        0
                    }
                } else {
                    entry.es_lnum
                };

                if !type_name.data.is_null() && type_name.size > 0 {
                    buf.extend_from_slice(std::slice::from_raw_parts(
                        type_name.data,
                        type_name.size,
                    ));
                }
                if !es_name.data.is_null() && es_name.size > 0 {
                    buf.extend_from_slice(std::slice::from_raw_parts(es_name.data, es_name.size));
                }
                if lnum != 0 {
                    let mut lnum_buf = [0 as c_char; 32];
                    let n = vim_snprintf_safelen!(
                        lnum_buf.as_mut_ptr(),
                        lnum_buf.len(),
                        c"[%d]".as_ptr(),
                        lnum,
                    );
                    buf.extend_from_slice(&lnum_buf[..n]);
                }
                if idx != es_len - 1 {
                    buf.extend_from_slice(std::slice::from_raw_parts(c"..".as_ptr(), 2));
                }
            }
        }

        if buf.is_empty() {
            None
        } else {
            let bytes = std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), buf.len());
            Some(cstr_from_bytes(bytes))
        }
    }
}

unsafe fn stacktrace_push_item(
    l: *mut list_T,
    fp: *mut ufunc_T,
    event: *const c_char,
    lnum: linenr_T,
    filepath: *const c_char,
) {
    unsafe {
        let d = tv_dict_alloc_lock(VAR_FIXED);
        let mut tv = typval_T {
            v_type: VarType::VAR_DICT,
            v_lock: VAR_LOCKED,
            vval: TypvalVval::Dict(d),
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

/// Create the stacktrace from rt_st().exestack.
pub unsafe fn stacktrace_create() -> *mut list_T {
    unsafe {
        let es = &raw const rt_st().exestack;
        let l = tv_list_alloc((*es).len() as isize);

        for entry in &*es {
            let mut lnum = entry.es_lnum;

            if entry.es_type == ETYPE_SCRIPT {
                stacktrace_push_item(l, ptr::null_mut(), ptr::null(), lnum, entry.name_ptr());
            } else if entry.es_type == ETYPE_UFUNC {
                let fp = entry.es_info.ufunc();
                let sctx = (*fp).uf_script_ctx;
                let filepath = if sctx.sc_sid > 0 {
                    get_scriptname(sctx, false)
                } else {
                    std::borrow::Cow::Borrowed(c"")
                };
                lnum += sctx.sc_lnum;
                stacktrace_push_item(l, fp, ptr::null(), lnum, filepath.as_ptr());
            } else if entry.es_type == ETYPE_AUCMD {
                let sctx = entry.es_info.aucmd_sctx();
                let filepath = if sctx.sc_sid > 0 {
                    get_scriptname(sctx, false)
                } else {
                    std::borrow::Cow::Borrowed(c"")
                };
                lnum += sctx.sc_lnum;
                stacktrace_push_item(
                    l,
                    ptr::null_mut(),
                    entry.name_ptr(),
                    lnum,
                    filepath.as_ptr(),
                );
            }
        }
        l
    }
}

/// getstacktrace() function
pub unsafe fn f_getstacktrace(_argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let l = stacktrace_create();
        rettv.v_type = VarType::VAR_LIST;
        rettv.vval = crate::eval::typval::TypvalVval::List(l);
        if !l.is_null() {
            (*l).lv_refcount += 1;
        }
    }
}

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
pub unsafe fn ex_runtime(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;
        let mut flags = if eap.forceit != 0 { DIP_ALL } else { 0 };
        let p = skiptowhite(arg);
        flags += get_runtime_cmd_flags(&mut arg, (p as usize) - (arg as usize));
        debug_assert!(!arg.is_null());
        source_runtime(arg, flags);
    }
}

/// Set the completion context for the :runtime command.
pub unsafe fn set_context_in_runtime_cmd(xp: *mut expand_T, mut arg: *const c_char) {
    unsafe {
        let mut p = skiptowhite(arg);
        let mut arg_mut = arg as *mut c_char;
        rt_st().runtime_expand_flags = if *p != NUL {
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
            if rt_st().runtime_expand_flags == 0 {
                rt_st().runtime_expand_flags = DIP_ALL;
            }
            arg = skipwhite(p);
        }
        (*xp).xp_context = EXPAND_RUNTIME;
        (*xp).xp_pattern = arg as *mut c_char;
    }
}

unsafe fn source_callback_vim_lua(
    fnames: &[std::ffi::CString],
    all: bool,
    cookie: *mut c_int,
) -> bool {
    unsafe {
        let mut did_one = false;

        for fname_cstr in fnames {
            let fname = fname_cstr.as_ptr().cast_mut();
            if path_with_extension(fname, c"vim".as_ptr()) {
                do_source(fname, false, DOSO_NONE, cookie);
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        for fname_cstr in fnames {
            let fname = fname_cstr.as_ptr().cast_mut();
            if path_with_extension(fname, c"lua".as_ptr()) {
                do_source(fname, false, DOSO_NONE, cookie);
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        did_one
    }
}

unsafe fn source_callback(fnames: &[std::ffi::CString], all: bool, cookie: *mut c_int) -> bool {
    unsafe {
        let mut did_one = source_callback_vim_lua(fnames, all, cookie);

        if !all && did_one {
            return true;
        }

        for fname_cstr in fnames {
            let fname = fname_cstr.as_ptr().cast_mut();
            if !path_with_extension(fname, c"vim".as_ptr())
                && !path_with_extension(fname, c"lua".as_ptr())
            {
                do_source(fname, false, DOSO_NONE, cookie);
                did_one = true;
                if !all {
                    return true;
                }
            }
        }

        did_one
    }
}

/// Find the patterns in "name" in all directories in "path" and invoke `callback`.
pub unsafe fn do_in_path(
    path: *const c_char,
    prefix: *const c_char,
    name: *mut c_char,
    flags: c_int,
    callback: &mut RuntimeCallback<'_>,
) -> c_int {
    unsafe {
        let mut did_one = false;

        let rtp_copy = cstr_to_cstring(path);
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let mut buf = buf_vec.as_mut_ptr();
        {
            if opt_vars().p_verbose > 10 && !name.is_null() {
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

            let mut rtp = rtp_copy.as_ptr().cast_mut();
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
                    callback.invoke(&[CStr::from_ptr(buf).to_owned()], do_all);
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

                            if opt_vars().p_verbose > 10 {
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
                                1, &mut buf, ew_flags, do_all, callback,
                            ) == OK;
                        }
                    }
                }
            }
        }
        drop(buf_vec);
        drop(rtp_copy);
        if !did_one && !name.is_null() {
            let basepath = if ptr::eq(path, opt_vars().p_rtp) {
                c"runtimepath".as_ptr()
            } else {
                c"packpath".as_ptr()
            };

            if (flags & DIP_ERR) != 0 {
                semsg!(gettext(e_dirnotf.as_ptr()), basepath, name);
            } else if opt_vars().p_verbose > 1 {
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

unsafe fn runtime_search_path_get_cached() -> *mut [SearchPathItem] {
    unsafe {
        runtime_search_path_validate();
        rt_st().runtime_search_path_refcount += 1;
        let rsp_ptr = &raw mut rt_st().runtime_search_path;
        let rsp = &mut *rsp_ptr;
        ptr::slice_from_raw_parts_mut(rsp.as_mut_ptr(), rsp.len())
    }
}

fn copy_runtime_search_path(src: &[SearchPathItem]) -> RuntimeSearchPath {
    src.to_vec()
}

unsafe fn runtime_search_path_unref() {
    unsafe {
        rt_st().runtime_search_path_refcount -= 1;
        if rt_st().runtime_search_path_refcount == 0 {
            let old_ptr = &raw mut rt_st().runtime_search_path_old;
            (*old_ptr).clear();
        }
    }
}

unsafe fn do_in_cached_path(
    name: *mut c_char,
    flags: c_int,
    callback: &mut RuntimeCallback<'_>,
) -> c_int {
    unsafe {
        let mut did_one = false;
        let mut buf = [0 as c_char; MAXPATHL];

        if opt_vars().p_verbose > 10 && !name.is_null() {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Searching for \"%s\" in runtime path".as_ptr()),
                name,
            );
            verbose_leave();
        }

        let path = &*runtime_search_path_get_cached();

        let do_all = (flags & DIP_ALL) != 0;

        for item in path {
            let item_path = item.path.as_ptr().cast_mut();
            let buflen = item.path.as_bytes().len();

            if (flags & (DIP_NOAFTER | DIP_AFTER)) != 0
                && ((item.after && (flags & DIP_NOAFTER) != 0)
                    || (!item.after && (flags & DIP_AFTER) != 0))
            {
                continue;
            }

            if name.is_null() {
                callback.invoke(std::slice::from_ref(&item.path), do_all);
            } else if buflen + CStr::from_ptr(name).to_bytes().len() + 2 < MAXPATHL {
                ptr::copy_nonoverlapping(item_path, buf.as_mut_ptr(), buflen + 1);
                add_pathsep(buf.as_mut_ptr());
                let tail = buf
                    .as_mut_ptr()
                    .add(CStr::from_ptr(buf.as_ptr()).to_bytes().len());

                let mut np = name;
                while *np != NUL && (do_all || !did_one) {
                    let tail_off = (tail as usize) - (buf.as_ptr() as usize);
                    debug_assert!(MAXPATHL >= tail_off);
                    copy_option_part(&mut np, tail, MAXPATHL - tail_off, c"\t ".as_ptr());

                    if opt_vars().p_verbose > 10 {
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
                    ) == OK;
                }
            }
        }

        if !did_one && !name.is_null() {
            if (flags & DIP_ERR) != 0 {
                semsg!(gettext(e_dirnotf.as_ptr()), c"runtime path".as_ptr(), name);
            } else if opt_vars().p_verbose > 1 {
                verbose_enter();
                smsg!(
                    0,
                    gettext(c"not found in runtime path: \"%s\"".as_ptr()),
                    name,
                );
                verbose_leave();
            }
        }

        runtime_search_path_unref();

        if did_one { OK } else { FAIL }
    }
}

pub unsafe fn runtime_inspect(arena: *mut Arena) -> Array {
    unsafe {
        let rsp_ptr = &raw const rt_st().runtime_search_path;
        let rsp = &*rsp_ptr;
        let mut rv = arena_array(arena, rsp.len());

        for item in rsp {
            let mut entry = arena_dict(arena, 5);
            put_c(&mut entry, c"path", cstr_as_obj(item.path.as_ptr()));
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

pub unsafe fn runtime_get_named(lua: bool, pat: Array, all: bool, arena: *mut Arena) -> Array {
    unsafe {
        let path = &mut *runtime_search_path_get_cached();
        let rv = runtime_get_named_common(
            lua,
            pat,
            all,
            path,
            (&raw mut rt_st().get_named_buf).cast::<c_char>(),
            MAXPATHL,
            arena,
        );
        runtime_search_path_unref();
        rv
    }
}

pub unsafe fn runtime_get_named_thread(lua: bool, pat: Array, all: bool) -> Array {
    unsafe {
        let _guard = RUNTIME_SEARCH_PATH_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path_ptr = &raw mut rt_st().runtime_search_path_thread;
        let path = &mut *path_ptr;
        runtime_get_named_common(
            lua,
            pat,
            all,
            path,
            (&raw mut rt_st().get_named_thread_buf).cast::<c_char>(),
            MAXPATHL,
            ptr::null_mut(),
        )
    }
}

unsafe fn runtime_get_named_common(
    lua: bool,
    pat: Array,
    all: bool,
    path: &mut [SearchPathItem],
    buf: *mut c_char,
    buf_len: usize,
    arena: *mut Arena,
) -> Array {
    unsafe {
        let mut rv = arena_array(arena, path.len() * pat.size);
        for item in path.iter_mut() {
            if lua {
                if item.has_lua == kNone {
                    let size = vim_snprintf!(buf, buf_len, c"%s/lua/".as_ptr(), item.path.as_ptr())
                        as usize;
                    item.has_lua = TriState::from(size < buf_len && os_isdir(buf));
                }
                if item.has_lua == kFalse {
                    continue;
                }
            }

            for j in 0..pat.size {
                let pat_item = pat[j];
                if pat_item.obj_type == ObjectType::kObjectTypeString {
                    let size = vim_snprintf!(
                        buf,
                        buf_len,
                        c"%s/%s".as_ptr(),
                        item.path.as_ptr(),
                        pat_item.data.string().data,
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
pub unsafe fn do_in_path_and_pp(
    path: *mut c_char,
    name: *mut c_char,
    flags: c_int,
    callback: &mut RuntimeCallback<'_>,
) -> c_int {
    unsafe {
        let mut done = FAIL;

        if (flags & DIP_NORTP) == 0 {
            let name_arg = if !name.is_null() && *name == NUL {
                ptr::null_mut()
            } else {
                name
            };
            done |= do_in_path(path, c"".as_ptr(), name_arg, flags, callback);
        }

        if (done == FAIL || (flags & DIP_ALL) != 0) && (flags & DIP_START) != 0 {
            let mut prefix = if (flags & DIP_AFTER) != 0 {
                c"pack/*/start/*/after/".as_ptr()
            } else {
                c"pack/*/start/*/".as_ptr()
            };
            done |= do_in_path(opt_vars().p_pp, prefix, name, flags & !DIP_AFTER, callback);

            if done == FAIL || (flags & DIP_ALL) != 0 {
                prefix = if (flags & DIP_AFTER) != 0 {
                    c"start/*/after/".as_ptr()
                } else {
                    c"start/*/".as_ptr()
                };
                done |= do_in_path(opt_vars().p_pp, prefix, name, flags & !DIP_AFTER, callback);
            }
        }

        if (done == FAIL || (flags & DIP_ALL) != 0) && (flags & DIP_OPT) != 0 {
            done |= do_in_path(opt_vars().p_pp, c"pack/*/opt/*/".as_ptr(), name, flags, callback);

            if done == FAIL || (flags & DIP_ALL) != 0 {
                done |= do_in_path(opt_vars().p_pp, c"opt/*/".as_ptr(), name, flags, callback);
            }
        }

        done
    }
}

unsafe fn push_path(
    search_path: &mut RuntimeSearchPath,
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    entry: *mut c_char,
    after: bool,
    pos_in_rtp: usize,
) -> bool {
    unsafe {
        if rtp_used.insert(CStr::from_ptr(entry).to_bytes().to_vec()) {
            search_path.push(SearchPathItem {
                path: CStr::from_ptr(entry).to_owned(),
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
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    entry: *mut c_char,
    after: bool,
    pos_in_rtp: usize,
) {
    unsafe {
        if rtp_used.contains(CStr::from_ptr(entry).to_bytes()) {
            return;
        }

        if *entry == NUL {
            push_path(search_path, rtp_used, entry, after, pos_in_rtp);
        }

        let mut files: Vec<std::ffi::CString> = Vec::new();
        let mut pat = [entry];
        if gen_expand_wildcards_vec(1, pat.as_mut_ptr(), &mut files, EW_DIR | EW_NOBREAK) == OK {
            for file in &files {
                push_path(search_path, rtp_used, file.as_ptr().cast_mut(), after, pos_in_rtp);
            }
        }
    }
}

unsafe fn expand_pack_entry(
    search_path: &mut RuntimeSearchPath,
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    after_path: &mut Vec<std::ffi::CString>,
    pack_entry: *const c_char,
    pack_entry_len: usize,
    pos_in_rtp: usize,
) {
    unsafe {
        let buf = (&raw mut rt_st().pack_entry_buf).cast::<c_char>();
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
            let mut after_bytes = Vec::from(CStr::from_ptr(buf).to_bytes());
            after_bytes.extend_from_slice(b"/after");
            after_path.push(cstr_from_bytes(&after_bytes));
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
        let mut pack_entries: Vec<NvimString> = Vec::new();
        let mut pack_used: std::collections::HashMap<Vec<u8>, c_int> =
            std::collections::HashMap::new();
        let mut rtp_used: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        let mut search_path = RuntimeSearchPath::new();
        let mut after_path: Vec<std::ffi::CString> = Vec::new();

        let buf = (&raw mut rt_st().build_rtp_buf).cast::<c_char>();
        let mut entry = opt_vars().p_pp;
        while *entry != NUL {
            let cur_entry = entry;
            let buflen = copy_option_part(&mut entry, buf, MAXPATHL, c",".as_ptr());

            let the_entry = NvimString {
                data: cur_entry,
                size: buflen,
            };

            pack_entries.push(the_entry);
            pack_used.insert(
                std::slice::from_raw_parts(cur_entry.cast::<u8>(), buflen).to_vec(),
                0,
            );
        }

        let mut rtp_entry = opt_vars().p_rtp;
        while *rtp_entry != NUL {
            let cur_entry = rtp_entry;
            let buflen = copy_option_part(&mut rtp_entry, buf, MAXPATHL, c",".as_ptr());

            if path_is_after(buf, buflen) {
                rtp_entry = cur_entry;
                break;
            }

            let pos_in_rtp = (cur_entry as usize) - (opt_vars().p_rtp as usize);

            expand_rtp_entry(&mut search_path, &mut rtp_used, buf, false, pos_in_rtp);

            if let Some(h) = pack_used.get_mut(CStr::from_ptr(buf).to_bytes()) {
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

        let sentinel_pos_in_rtp = ((rtp_entry as usize) - (opt_vars().p_rtp as usize)).saturating_sub(1);

        for item in pack_entries {
            let item_bytes = std::slice::from_raw_parts(item.data.cast::<u8>(), item.size);
            let h = pack_used.get(item_bytes).copied().unwrap_or(0);
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

        for ap in after_path {
            expand_rtp_entry(
                &mut search_path,
                &mut rtp_used,
                ap.as_ptr().cast_mut(),
                true,
                sentinel_pos_in_rtp,
            );
        }

        while *rtp_entry != NUL {
            let cur_entry = rtp_entry;
            let buflen = copy_option_part(&mut rtp_entry, buf, MAXPATHL, c",".as_ptr());
            let pos_in_rtp = (cur_entry as usize) - (opt_vars().p_rtp as usize);
            expand_rtp_entry(
                &mut search_path,
                &mut rtp_used,
                buf,
                path_is_after(buf, buflen),
                pos_in_rtp,
            );
        }

        search_path
    }
}

pub unsafe fn did_set_runtimepackpath(_args: &mut optset_T) -> *const c_char {
    unsafe {
        rt_st().runtime_search_path_valid = false;
        ptr::null()
    }
}

pub unsafe fn runtime_search_path_validate() {
    unsafe {
        if !nlua_is_deferred_safe() {
            return;
        }
        if !rt_st().runtime_search_path_valid {
            let new_path = runtime_search_path_build();
            let rsp_ptr = &raw mut rt_st().runtime_search_path;
            let old_path = std::mem::replace(&mut *rsp_ptr, new_path);
            if rt_st().runtime_search_path_refcount == 0 {
                msg_ext_ui_flush();
                drop(old_path);
            } else {
                let old_ptr = &raw mut rt_st().runtime_search_path_old;
                (*old_ptr).push(old_path);
            }
            rt_st().runtime_search_path_valid = true;
            update_runtime_search_path_thread(true);
        }
    }
}

pub unsafe fn update_runtime_search_path_thread(force: bool) {
    unsafe {
        if !force && !(rt_st().runtime_search_path_valid && !rt_st().runtime_search_path_valid_thread) {
            return;
        }

        let _guard = RUNTIME_SEARCH_PATH_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let rsp_ptr = &raw const rt_st().runtime_search_path;
        rt_st().runtime_search_path_thread = copy_runtime_search_path(&*rsp_ptr);
        drop(_guard);
        rt_st().runtime_search_path_valid_thread = true;
    }
}

/// Just like do_in_path_and_pp(), using 'runtimepath' for "path".
pub unsafe fn do_in_runtimepath(
    name: *mut c_char,
    mut flags: c_int,
    callback: &mut RuntimeCallback<'_>,
) -> c_int {
    unsafe {
        let mut success = FAIL;
        if (flags & DIP_NORTP) == 0 {
            let name_arg = if !name.is_null() && *name == NUL {
                ptr::null_mut()
            } else {
                name
            };
            success |= do_in_cached_path(name_arg, flags, callback);
            flags = (flags & !DIP_START) | DIP_NORTP;
        }
        if (flags & (DIP_START | DIP_OPT)) != 0 && (success == FAIL || (flags & DIP_ALL) != 0) {
            success |= do_in_path_and_pp(opt_vars().p_rtp, name, flags, callback);
        }
        success
    }
}

/// Source the file "name" from all directories in 'runtimepath'.
pub unsafe fn source_runtime(name: *mut c_char, flags: c_int) -> c_int {
    unsafe {
        do_in_runtimepath(
            name,
            flags,
            &mut RuntimeCallback::Source {
                skip_if_loaded: false,
            },
        )
    }
}

/// Just like source_runtime(), but only source vim and lua files.
pub unsafe fn source_runtime_vim_lua(name: *mut c_char, flags: c_int) -> c_int {
    unsafe { do_in_runtimepath(name, flags, &mut RuntimeCallback::SourceVimLua) }
}

/// Just like source_runtime(), but use "path" instead of 'runtimepath' and only source .vim and .lua files.
pub unsafe fn source_in_path_vim_lua(path: *mut c_char, name: *mut c_char, flags: c_int) -> c_int {
    unsafe { do_in_path_and_pp(path, name, flags, &mut RuntimeCallback::SourceVimLua) }
}

unsafe fn gen_expand_wildcards_and_cb(
    num_pat: c_int,
    pats: *mut *mut c_char,
    flags: c_int,
    all: bool,
    callback: &mut RuntimeCallback<'_>,
) -> c_int {
    unsafe {
        let mut files: Vec<std::ffi::CString> = Vec::new();

        if gen_expand_wildcards_vec(num_pat, pats, &mut files, flags) != OK {
            return FAIL;
        }

        callback.invoke(&files, all);

        OK
    }
}

unsafe fn add_pack_dir_to_rtp(fname: *mut c_char, is_pack: bool) -> c_int {
    unsafe {
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
        let ffname_cstr = fix_fname(fname);
        *p4 = c;

        let Some(ref ffname_owned) = ffname_cstr else {
            return FAIL;
        };
        let ffname = ffname_owned.as_ptr();

        let mut afterdir_cstr: Option<std::ffi::CString> = None;

        'block: {
            let mut fname_len = ffname_owned.as_bytes().len();
            let mut buf_data = [0 as c_char; MAXPATHL];
            let mut buf = NvimString {
                data: buf_data.as_mut_ptr(),
                size: 0,
            };

            let mut insp: *const c_char = ptr::null();
            let mut after_insp: *const c_char = ptr::null();
            let mut p_rtp_len: usize = 0;
            let mut entry: *mut c_char = opt_vars().p_rtp;
            while *entry != NUL {
                let cur_entry = entry;
                buf.size = copy_option_part(&mut entry, buf.data, MAXPATHL, c",".as_ptr());

                p_rtp_len += if *cur_entry.add(buf.size) == b',' as c_char {
                    buf.size + 1
                } else {
                    buf.size
                };

                let p_after = CStr::from_ptr(buf.data).to_bytes().windows(5).position(|w| w == b"after").map_or(ptr::null_mut(), |i| buf.data.add(i));
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
                    let Some(rtp_ffname) = fix_fname(buf.data) else {
                        break 'block;
                    };
                    if path_cmp(opt_vars().p_fic != 0, rtp_ffname.as_ptr(), ffname, fname_len) == 0 {
                        insp = entry;
                    }
                }
            }

            p_rtp_len += CStr::from_ptr(opt_vars().p_rtp.add(p_rtp_len)).to_bytes().len();
            if insp.is_null() {
                insp = opt_vars().p_rtp.add(p_rtp_len);
            }

            fname_len = CStr::from_ptr(fname).to_bytes().len();
            let ad = crate::path::concat_fnames_cstr(fname, c"after".as_ptr(), true);
            let mut afterdir = NvimString {
                data: ad.as_ptr().cast_mut(),
                size: ad.as_bytes().len(),
            };
            afterdir_cstr = Some(ad);
            if if is_pack {
                !pack_has_entries(afterdir.data)
            } else {
                !os_isdir(afterdir.data)
            } {
                afterdir.size = 0;
            }

            let new_rtp_capacity = p_rtp_len + fname_len + afterdir.size + 3;
            let mut new_rtp_vec = vec![0 as c_char; new_rtp_capacity];
            let mut new_rtp = NvimString {
                data: new_rtp_vec.as_mut_ptr(),
                size: 0,
            };

            let mut keep = (insp as usize) - (opt_vars().p_rtp as usize);
            let mut first_pos = keep;
            ptr::copy(opt_vars().p_rtp, new_rtp.data, keep);
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
                let keep_after = (after_insp as usize) - (opt_vars().p_rtp as usize);
                let append_len = keep_after - keep;

                ptr::copy(opt_vars().p_rtp.add(keep), new_rtp.data.add(new_rtp.size), append_len);
                new_rtp.size += append_len;
                ptr::copy(afterdir.data, new_rtp.data.add(new_rtp.size), afterdir.size);
                new_rtp.size += afterdir.size;
                *new_rtp.data.add(new_rtp.size) = b',' as c_char;
                new_rtp.size += 1;
                keep = keep_after;
                after_pos = keep_after;
            }

            if *opt_vars().p_rtp.add(keep) != NUL {
                let append_len = p_rtp_len - keep;
                ptr::copy(
                    opt_vars().p_rtp.add(keep),
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

            let was_valid = rt_st().runtime_search_path_valid;
            set_option_value_give_err(kOptRuntimepath, string_obj(new_rtp), 0);

            debug_assert!(!rt_st().runtime_search_path_valid);
            if was_valid && !is_pack && rt_st().runtime_search_path_refcount == 0 {
                rt_st().runtime_search_path_valid = true;
                rt_st().runtime_search_path_valid_thread = false;
                let rsp_ptr = &raw mut rt_st().runtime_search_path;
                let rsp = &mut *rsp_ptr;

                let idx1 = rsp
                    .iter()
                    .position(|item| item.pos_in_rtp >= first_pos)
                    .unwrap_or(rsp.len());
                for item in &mut rsp[idx1..] {
                    item.pos_in_rtp += fname_len + 1;
                }
                let fname_bytes = std::slice::from_raw_parts(fname.cast::<u8>(), fname_len);
                let fname_end = fname_bytes
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(fname_bytes.len());
                rsp.insert(
                    idx1,
                    SearchPathItem {
                        path: std::ffi::CString::new(&fname_bytes[..fname_end]).unwrap(),
                        after: false,
                        pack_inserted: true,
                        has_lua: kNone,
                        pos_in_rtp: first_pos,
                    },
                );

                if afterdir.size > 0 {
                    let target_after_pos = after_pos + fname_len + 1;
                    let idx2 = rsp
                        .iter()
                        .position(|item| item.pos_in_rtp >= target_after_pos)
                        .unwrap_or(rsp.len());
                    for item in &mut rsp[idx2..] {
                        item.pos_in_rtp += afterdir.size + 1;
                    }
                    let after_bytes =
                        std::slice::from_raw_parts(afterdir.data.cast::<u8>(), afterdir.size);
                    let after_end = after_bytes
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(after_bytes.len());
                    rsp.insert(
                        idx2,
                        SearchPathItem {
                            path: std::ffi::CString::new(&after_bytes[..after_end]).unwrap(),
                            after: true,
                            pack_inserted: true,
                            has_lua: kNone,
                            pos_in_rtp: target_after_pos,
                        },
                    );
                }
            }
            drop(new_rtp_vec);
            retval = OK;
        }

        drop(afterdir_cstr);
        drop(ffname_cstr);
        retval
    }
}

unsafe fn load_pack_plugin(opt: bool, fname: *mut c_char) -> c_int {
    unsafe {
        let ffname_cstr = fix_fname(fname).unwrap_or_default();
        let ffname = ffname_cstr.as_ptr();
        let len = ffname_cstr.as_bytes().len() + 14;
        let mut pat_vec = vec![0 as c_char; len + 1];
        let mut pat = pat_vec.as_mut_ptr();

        vim_snprintf!(pat, len, c"%s/plugin/**/*".as_ptr(), ffname);
        gen_expand_wildcards_and_cb(1, &mut pat, EW_FILE, true, &mut RuntimeCallback::SourceVimLua);

        let mut cmd = *b"g:did_load_filetypes\0";

        if opt && eval_to_number(cmd.as_mut_ptr().cast(), false) > 0 {
            do_cmdline_cmd(c"augroup filetypedetect".as_ptr());
            vim_snprintf!(pat, len, c"%s/ftdetect/*".as_ptr(), ffname);
            gen_expand_wildcards_and_cb(
                1,
                &mut pat,
                EW_FILE,
                true,
                &mut RuntimeCallback::SourceVimLua,
            );
            do_cmdline_cmd(c"augroup END".as_ptr());
        }

        OK
    }
}

unsafe fn add_pack_plugins(
    opt: bool,
    fnames: &[std::ffi::CString],
    all: bool,
    action: PackAddAction,
) {
    unsafe {
        let mut did_one = false;

        if action != PackAddAction::Load {
            let mut buf = [0 as c_char; MAXPATHL];
            for fname_cstr in fnames {
                let fname = fname_cstr.as_ptr().cast_mut();
                let mut found = false;
                let mut p: *mut c_char = opt_vars().p_rtp;
                while *p != NUL {
                    copy_option_part(&mut p, buf.as_mut_ptr(), MAXPATHL, c",".as_ptr());
                    if path_equal(buf.as_ptr(), fname, kPathCmpLiteral) {
                        found = true;
                        break;
                    }
                }
                if !found && add_pack_dir_to_rtp(fname, false) == FAIL {
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

        if action != PackAddAction::AddDir {
            for fname_cstr in fnames {
                load_pack_plugin(opt, fname_cstr.as_ptr().cast_mut());
                if !all {
                    break;
                }
            }
        }
    }
}

/// Add all packages in the "start" directory to 'runtimepath'.
pub unsafe fn add_pack_start_dirs() {
    unsafe {
        do_in_path(
            opt_vars().p_pp,
            c"".as_ptr(),
            ptr::null_mut(),
            DIP_ALL + DIP_DIR,
            &mut RuntimeCallback::AddPackStartDir,
        );
    }
}

unsafe fn pack_has_entries(buf: *mut c_char) -> bool {
    unsafe {
        let mut files: Vec<std::ffi::CString> = Vec::new();
        let mut pat = [buf];
        gen_expand_wildcards_vec(1, pat.as_mut_ptr(), &mut files, EW_DIR) == OK && !files.is_empty()
    }
}

unsafe fn add_pack_start_dir(fnames: &[std::ffi::CString], all: bool) -> bool {
    unsafe {
        let buf = (&raw mut rt_st().pack_start_buf).cast::<c_char>();
        let start_pats = [c"/start/*", c"/pack/*/start/*"];
        for fname_cstr in fnames {
            let fname_len = fname_cstr.as_bytes().len();
            for pat in &start_pats {
                if fname_len + pat.to_bytes().len() + 1 > MAXPATHL {
                    continue;
                }
                xstrlcpy(buf, fname_cstr.as_ptr(), MAXPATHL);
                xstrlcat(buf, pat.as_ptr(), MAXPATHL);
                if pack_has_entries(buf) {
                    add_pack_dir_to_rtp(buf, true);
                }
            }

            if !all {
                break;
            }
        }

        fnames.len() > 1
    }
}

/// Load plugins from all packages in the "start" directory.
pub unsafe fn load_start_packages() {
    unsafe {
        g_st().did_source_packages = true;
        do_in_path(
            opt_vars().p_pp,
            c"".as_ptr(),
            c"pack/*/start/*".as_ptr() as *mut c_char,
            DIP_ALL + DIP_DIR,
            &mut RuntimeCallback::AddPackPlugins {
                opt: false,
                action: PackAddAction::Load,
            },
        );
        do_in_path(
            opt_vars().p_pp,
            c"".as_ptr(),
            c"start/*".as_ptr() as *mut c_char,
            DIP_ALL + DIP_DIR,
            &mut RuntimeCallback::AddPackPlugins {
                opt: false,
                action: PackAddAction::Load,
            },
        );

        update_runtime_search_path_thread(false);
    }
}

/// ":packloadall"
pub unsafe fn ex_packloadall(eap: &mut exarg_T) {
    unsafe {
        if !g_st().did_source_packages || eap.forceit != 0 {
            add_pack_start_dirs();
            load_start_packages();
        }
    }
}

/// Read all the plugin files at startup.
pub unsafe fn load_plugins() {
    unsafe {
        if opt_vars().p_lpl != 0 {
            let mut rtp_copy_owned: Option<std::ffi::CString> = None;
            let mut rtp_copy = opt_vars().p_rtp;
            let plugin_pattern = c"plugin/**/*".as_ptr() as *mut c_char;

            if !g_st().did_source_packages {
                let copy = cstr_to_cstring(opt_vars().p_rtp);
                rtp_copy = copy.as_ptr().cast_mut();
                rtp_copy_owned = Some(copy);
                add_pack_start_dirs();
            }

            source_in_path_vim_lua(rtp_copy, plugin_pattern, DIP_ALL | DIP_NOAFTER);
            if has_time_fd() {
                time_msg(c"loading rtp plugins".as_ptr(), ptr::null());
            }

            if !g_st().did_source_packages {
                drop(rtp_copy_owned);
                load_start_packages();
            }
            if has_time_fd() {
                time_msg(c"loading packages".as_ptr(), ptr::null());
            }

            source_runtime_vim_lua(plugin_pattern, DIP_ALL | DIP_AFTER);
            if has_time_fd() {
                time_msg(c"loading after plugins".as_ptr(), ptr::null());
            }
        }
    }
}

/// ":packadd[!] {name}"
pub unsafe fn ex_packadd(eap: &mut exarg_T) {
    unsafe {
        let mut res = OK;
        let len = 11 + CStr::from_ptr(eap.arg).to_bytes().len() + 5;
        let mut pat_vec = vec![0 as c_char; len];
        let pat = pat_vec.as_mut_ptr();
        let action = if eap.forceit != 0 {
            PackAddAction::AddDir
        } else {
            PackAddAction::Both
        };

        if !g_st().did_source_packages {
            vim_snprintf!(
                pat,
                len,
                c"pack/*/%s/%s".as_ptr(),
                c"start".as_ptr(),
                eap.arg,
            );
            res = do_in_path(
                opt_vars().p_pp,
                c"".as_ptr(),
                pat,
                DIP_ALL + DIP_DIR,
                &mut RuntimeCallback::AddPackPlugins { opt: false, action },
            );
        }

        vim_snprintf!(
            pat,
            len,
            c"pack/*/%s/%s".as_ptr(),
            c"opt".as_ptr(),
            eap.arg,
        );
        do_in_path(
            opt_vars().p_pp,
            c"".as_ptr(),
            pat,
            DIP_ALL + DIP_DIR + if res == FAIL { DIP_ERR } else { 0 },
            &mut RuntimeCallback::AddPackPlugins { opt: true, action },
        );

        update_runtime_search_path_thread(false);
    }
}

unsafe fn ExpandRTDir_int(
    pat: *mut c_char,
    pat_len: usize,
    flags: c_int,
    keep_ext: bool,
    gap: &mut Vec<std::ffi::CString>,
    dirnames: *mut *mut c_char,
) {
    unsafe {
        let mut i = 0usize;
        while !(*dirnames.add(i)).is_null() {
            let dir = *dirnames.add(i);
            let buf_len = CStr::from_ptr(dir).to_bytes().len() + pat_len + 64;
            let mut buf_vec = vec![0 as c_char; buf_len];
            let buf = buf_vec.as_mut_ptr();
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
                    globpath(opt_vars().p_rtp, buf, gap, glob_flags, expand_dirs);
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
                    globpath(opt_vars().p_pp, buf, gap, glob_flags, expand_dirs);

                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"start/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(opt_vars().p_pp, buf, gap, glob_flags, expand_dirs);
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
                    globpath(opt_vars().p_pp, buf, gap, glob_flags, expand_dirs);

                    vim_snprintf!(
                        buf,
                        buf_len,
                        c"opt/*/%s%s%s%s".as_ptr(),
                        dir_str,
                        slash_str,
                        pat,
                        ext_str,
                    );
                    globpath(opt_vars().p_pp, buf, gap, glob_flags, expand_dirs);
                }

                if *dir == NUL && !expand_dirs {
                    vim_snprintf!(buf, buf_len, c"%s*".as_ptr(), pat);
                    glob_flags = WILD_ADD_SLASH;
                    expand_dirs = true;
                    continue;
                }
                break;
            }

            i += 1;
        }

        let mut pat_pathsep_cnt: c_int = 0;
        for idx in 0..pat_len {
            if vim_ispathsep(*pat.add(idx) as u8 as c_int) {
                pat_pathsep_cnt += 1;
            }
        }

        for slot in gap.iter_mut() {
            let match_ptr = slot.as_ptr().cast_mut();
            let mut s = match_ptr;
            let mut e = s.add(slot.as_bytes().len());
            if (e as usize) - (s as usize) > 4 && !keep_ext {
                let tail4 = std::slice::from_raw_parts(e.sub(4).cast::<u8>(), 4);
                if tail4.eq_ignore_ascii_case(b".vim") || tail4.eq_ignore_ascii_case(b".lua") {
                    e = e.sub(4);
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
            let copy_len = (e as usize) - (s as usize);
            *slot =
                crate::memory::cstr_from_bytes(std::slice::from_raw_parts(s.cast::<u8>(), copy_len));
        }

        if gap.is_empty() {
            return;
        }

        crate::path::remove_duplicate_strings(gap);
    }
}

/// Expand color scheme, compiler or filetype names.
pub unsafe fn ExpandRTDir(
    pat: *mut c_char,
    flags: c_int,
    file: &mut Vec<std::ffi::CString>,
    dirnames: *mut *mut c_char,
) -> c_int {
    unsafe {
        file.clear();

        ExpandRTDir_int(
            pat,
            CStr::from_ptr(pat).to_bytes().len(),
            flags,
            false,
            file,
            dirnames,
        );

        if file.is_empty() {
            return FAIL;
        }

        OK
    }
}

/// Handle command line completion for :runtime command.
pub unsafe fn expand_runtime_cmd(
    pat: *mut c_char,
    matches: &mut Vec<std::ffi::CString>,
) -> c_int {
    unsafe {
        matches.clear();

        let pat_len = CStr::from_ptr(pat).to_bytes().len();
        let mut dirnames = [c"".as_ptr() as *mut c_char, ptr::null_mut()];
        ExpandRTDir_int(
            pat,
            pat_len,
            rt_st().runtime_expand_flags,
            true,
            matches,
            dirnames.as_mut_ptr(),
        );

        if rt_st().runtime_expand_flags == 0 {
            let where_values = [c"START", c"OPT", c"PACK", c"ALL"];
            let pat_slice = std::slice::from_raw_parts(pat.cast::<u8>(), pat_len);
            for wv in &where_values {
                if wv.to_bytes().starts_with(pat_slice) {
                    matches.push((*wv).to_owned());
                }
            }
        }

        if matches.is_empty() {
            return FAIL;
        }

        OK
    }
}

/// Expand loadplugin names: 'packpath'/pack/*/opt/{pat}
pub unsafe fn ExpandPackAddDir(pat: *mut c_char, file: &mut Vec<std::ffi::CString>) -> c_int {
    unsafe {
        file.clear();
        let pat_len = CStr::from_ptr(pat).to_bytes().len();

        let buflen = pat_len + 26;
        let mut s_vec = vec![0 as c_char; buflen];
        let s = s_vec.as_mut_ptr();
        vim_snprintf!(s, buflen, c"pack/*/opt/%s*".as_ptr(), pat);
        globpath(opt_vars().p_pp, s, file, 0, true);
        vim_snprintf!(s, buflen, c"opt/%s*".as_ptr(), pat);
        globpath(opt_vars().p_pp, s, file, 0, true);
        drop(s_vec);

        for slot in file.iter_mut() {
            let tail = path_tail(slot.as_ptr());
            *slot = CStr::from_ptr(tail).to_owned();
        }

        if file.is_empty() {
            return FAIL;
        }

        crate::path::remove_duplicate_strings(file);
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
        let mut iter: *const c_char = ptr::null();
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = vim_env_iter(ENV_SEPCHAR, val, iter, &mut dir, &mut dir_len);
            if !dir.is_null() && dir_len > 0 {
                ret += (dir_len
                    + std::slice::from_raw_parts(dir.cast::<u8>(), dir_len).iter().filter(|&&b| b == b',').count()
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
        let mut iter: *const c_char = ptr::null();
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
            let iobuff = (&raw mut g_st().IObuff).cast::<c_char>();
            { let _d = (iobuff).cast::<u8>(); let _n = appname_len; if _n > 0 { std::ptr::copy_nonoverlapping((appname).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
            #[cfg(windows)]
            if matches!(type_, XDGVarType::kXDGDataHome | XDGVarType::kXDGStateHome) {
                xstrlcat(iobuff, c"-data".as_ptr(), IOSIZE);
                appname_len += 5;
            }
            { let _d = (dest).cast::<u8>(); let _n = appname_len; if _n > 0 { std::ptr::copy_nonoverlapping((iobuff).cast::<u8>(), _d, _n); } *_d.add(_n) = 0; };
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

pub unsafe fn get_lib_dir() -> Option<std::ffi::CString> {
    unsafe {
        if !default_lib_dir().is_null()
            && *default_lib_dir() != NUL
            && os_isdir(default_lib_dir().cast_mut())
        {
            return Some(crate::memory::cstr_to_cstring(default_lib_dir()));
        }

        let mut exe_name = [0 as c_char; MAXPATHL];
        vim_get_prefix_from_exepath(exe_name.as_mut_ptr());
        if append_path(exe_name.as_mut_ptr(), c"lib/nvim".as_ptr(), MAXPATHL) == OK {
            return Some(crate::memory::cstr_to_cstring(exe_name.as_ptr()));
        }
        None
    }
}

/// Determine the startup value for &runtimepath.
pub unsafe fn runtimepath_default(clean_arg: bool) -> Option<std::ffi::CString> {
    unsafe {
        const SITE_SIZE: usize = 4;
        const AFTER_SIZE: usize = 5;

        let mut rtp_size: usize = 0;
        let data_home_cstr = if clean_arg {
            None
        } else {
            stdpaths_get_xdg_var(XDGVarType::kXDGDataHome)
        };
        let config_home_cstr = if clean_arg {
            None
        } else {
            stdpaths_get_xdg_var(XDGVarType::kXDGConfigHome)
        };
        let libdir_cstr = get_lib_dir();
        let data_dirs_cstr = stdpaths_get_xdg_var(XDGVarType::kXDGDataDirs);
        let config_dirs_cstr = stdpaths_get_xdg_var(XDGVarType::kXDGConfigDirs);
        let vimruntime_cstr = vim_getenv(c"VIMRUNTIME".as_ptr());

        let data_home = data_home_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let config_home = config_home_cstr
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        let libdir = libdir_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let data_dirs = data_dirs_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let config_dirs = config_dirs_cstr
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr());
        let vimruntime = vimruntime_cstr.as_ref().map_or(ptr::null(), |s| s.as_ptr());

        let mut data_len: usize = 0;
        let mut config_len: usize = 0;
        let mut vimruntime_len: usize = 0;
        let mut libdir_len: usize = 0;
        let appname_len = CStr::from_ptr(get_appname(false)).to_bytes().len();

        if let Some(ref dh) = data_home_cstr {
            data_len = dh.as_bytes().len();
            let nvim_data_size = if cfg!(windows) {
                appname_len + 5
            } else {
                appname_len
            };
            if data_len != 0 {
                rtp_size += (data_len
                    + std::slice::from_raw_parts(data_home.cast::<u8>(), data_len).iter().filter(|&&b| b == b',').count()
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
        if let Some(ref ch) = config_home_cstr {
            config_len = ch.as_bytes().len();
            if config_len != 0 {
                rtp_size += (config_len
                    +  ch.as_bytes().iter().filter(|&&b| b == b',').count()
                    + appname_len
                    + 1
                    + usize::from(after_pathsep(config_home, config_home.add(config_len)) == 0))
                    * 2
                    + AFTER_SIZE
                    + 1;
            }
        }
        if let Some(ref vr) = vimruntime_cstr {
            vimruntime_len = vr.as_bytes().len();
            if vimruntime_len != 0 {
                rtp_size +=
                    vimruntime_len + vr.as_bytes().iter().filter(|&&b| b == b',').count() + 1;
            }
        }
        if let Some(ref ld) = libdir_cstr {
            libdir_len = ld.as_bytes().len();
            if libdir_len != 0 {
                rtp_size += libdir_len + ld.as_bytes().iter().filter(|&&b| b == b',').count() + 1;
            }
        }
        rtp_size +=
            compute_double_env_sep_len(data_dirs, appname_len + 1 + SITE_SIZE + 1, AFTER_SIZE + 1);
        rtp_size += compute_double_env_sep_len(config_dirs, appname_len + 1, AFTER_SIZE + 1);

        if rtp_size != 0 {
            let mut rtp_vec = vec![0 as c_char; rtp_size];
            let rtp = rtp_vec.as_mut_ptr();
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
            Some(crate::memory::cstr_to_cstring(rtp))
        } else {
            None
        }
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
                g_st().global_busy != 0
                    || g_st().listcmd_busy
                    || !(*eap).nextcmd.is_null()
                    || (*(*eap).cstack.cast::<cstack_T>()).cs_idx >= 0,
            );
        } else if do_source(fname, false, DOSO_NONE, ptr::null_mut()) == FAIL {
            semsg!(gettext(e_notopen.as_ptr()), fname);
        }
    }
}

/// ":source [{fname}]"
pub unsafe fn ex_source(eap: &mut exarg_T) {
    unsafe {
        cmd_source(eap.arg, eap);
    }
}

/// ":options"
pub unsafe fn ex_options(_eap: &mut exarg_T) {
    unsafe {
        let mut buf = [0 as c_char; 500];
        let mut multi_mods = false;

        add_win_cmd_modifiers(buf.as_mut_ptr(), &raw mut g_st().cmdmod, &mut multi_mods);

        os_setenv(c"OPTWIN_CMD".as_ptr(), buf.as_ptr(), 1);
        cmd_source(
            c"$VIMRUNTIME/scripts/optwin.lua".as_ptr() as *mut c_char,
            ptr::null_mut(),
        );
    }
}

pub unsafe fn source_breakpoint(sp: *mut source_cookie_T) -> *mut linenr_T {
    unsafe { &raw mut (*sp).breakpoint }
}

pub unsafe fn source_dbg_tick(sp: *mut source_cookie_T) -> *mut c_int {
    unsafe { &raw mut (*sp).dbg_tick }
}

pub unsafe fn source_level(sp: *mut source_cookie_T) -> c_int {
    unsafe { (*sp).level }
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

        fdopen(fd_tmp, c"rb".as_ptr())
    }
}

unsafe fn concat_continued_line(ga: &mut Vec<c_char>, p: *const c_char, mut len: usize) -> bool {
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
        if len > 1 {
            ga.extend_from_slice(std::slice::from_raw_parts(line.add(1), len - 1));
        }
        true
    }
}

/// Create a new script item and allocate script-local vars.
pub unsafe fn new_script_item(
    name: Option<std::ffi::CString>,
    sid_out: *mut scid_T,
) -> *mut scriptitem_T {
    unsafe {
        rt_st().last_current_SID += 1;
        let sid = rt_st().last_current_SID;
        if !sid_out.is_null() {
            *sid_out = sid;
        }
        let items = &raw mut rt_st().script_items;
        while ((*items).len() as scid_T) < sid {
            (*items).push(scriptitem_T {
                sn_vars: new_script_vars(),
                sn_name: None,
                sn_lua: false,
                sn_prof_on: false,
                sn_pr_force: false,
                sn_pr_child: 0,
                sn_pr_nest: 0,
                sn_pr_count: 0,
                sn_pr_total: 0,
                sn_pr_self: 0,
                sn_pr_start: 0,
                sn_pr_children: 0,
                sn_prl_ga: Vec::new(),
                sn_prl_start: 0,
                sn_prl_children: 0,
                sn_prl_wait: 0,
                sn_prl_idx: 0,
                sn_prl_execed: 0,
            });
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
) -> Option<std::ffi::CString> {
    unsafe {
        if g_st().curbuf.is_null() {
            return None;
        }

        let iobuff = (&raw mut g_st().IObuff).cast::<c_char>();
        let fname = if !(*g_st().curbuf).b_ffname.is_null() {
            cstr_to_cstring((*g_st().curbuf).b_ffname)
        } else {
            if ex_lua {
                vim_snprintf!(
                    iobuff,
                    IOSIZE,
                    c":{range}lua buffer=%d".as_ptr(),
                    (*g_st().curbuf).b_fnum,
                );
            } else {
                vim_snprintf!(
                    iobuff,
                    IOSIZE,
                    c":source buffer=%d".as_ptr(),
                    (*g_st().curbuf).b_fnum,
                );
            }
            cstr_to_cstring(iobuff)
        };

        (*sp).buflines.clear();
        for curr_lnum in (*eap).line1..=(*eap).line2 {
            (*sp)
                .buflines
                .push(CStr::from_ptr(ml_get(curr_lnum)).to_owned());
        }
        (*sp).buf_lnum = 0;
        (*sp).source_from_buf_or_str = true;
        (*sp).sourcing_lnum = (*eap).line1 - 1;

        Some(fname)
    }
}

unsafe fn do_source_str_init(sp: *mut source_cookie_T, mut str_ptr: *const c_char) {
    unsafe {
        (*sp).buflines.clear();
        while *str_ptr != NUL {
            let eol = skip_to_newline(str_ptr);
            let slice = std::slice::from_raw_parts(
                str_ptr.cast::<u8>(),
                (eol as usize) - (str_ptr as usize),
            );
            (*sp)
                .buflines
                .push(std::ffi::CString::new(slice).unwrap_or_default());
            str_ptr = eol.add(usize::from(*eol != NUL));
        }
        (*sp).buf_lnum = 0;
        (*sp).source_from_buf_or_str = true;
    }
}

pub unsafe fn cmd_source_buffer(eap: *const exarg_T, ex_lua: bool) {
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
pub unsafe fn do_source_str(str_ptr: *const c_char, mut traceback_name: *const c_char) -> c_int {
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
        let mut cookie = source_cookie_T::new();
        let mut firstline: Option<std::ffi::CString> = None;
        let mut retval = FAIL;
        let save_debug_break_level = g_st().debug_break_level;
        let mut si: *mut scriptitem_T = ptr::null_mut();
        let mut wait_start: proftime_T = 0;
        let mut trigger_source_post = false;

        let fname_exp_owned: std::ffi::CString = if fname.is_null() {
            debug_assert!(str_ptr.is_null());
            let Some(fe) = do_source_buffer_init(&mut cookie, eap, ex_lua) else {
                return FAIL;
            };
            fe
        } else if !str_ptr.is_null() {
            do_source_str_init(&mut cookie, str_ptr);
            cstr_to_cstring(fname)
        } else {
            let p = expand_env_save(fname);
            let Some(fe) = fix_fname(p.as_ptr()) else {
                return retval;
            };
            if os_isdir(fe.as_ptr()) {
                smsg!(
                    0,
                    gettext(c"Cannot source a directory: \"%s\"".as_ptr()),
                    fname,
                );
                return retval;
            }
            fe
        };
        let fname_exp = fname_exp_owned.as_ptr().cast_mut();

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
                let has_sourcecmd = if fname.is_null() {
                    has_autocmd(EVENT_SOURCECMD, fname_exp, ptr::null_mut())
                } else {
                    has_autocmd_fullname(EVENT_SOURCECMD, fname_exp, ptr::null_mut())
                };
                if has_sourcecmd
                    && apply_autocmds(EVENT_SOURCECMD, fname_exp, fname_exp, false, g_st().curbuf)
                {
                    retval = if aborting() { FAIL } else { OK };
                    if retval == OK {
                        apply_autocmds(EVENT_SOURCEPOST, fname_exp, fname_exp, false, g_st().curbuf);
                    }
                    break 'block;
                }

                apply_autocmds(EVENT_SOURCEPRE, fname_exp, fname_exp, false, g_st().curbuf);
            }

            if !cookie.source_from_buf_or_str {
                cookie.fp = crate::os::fs::CFile::from_raw(fopen_noinh_readbin(fname_exp));
            }
            if cookie.fp.is_none() && check_other {
                let p = path_tail(fname_exp);
                if *p == b'.' as c_char || *p == b'_' as c_char {
                    let rest = CStr::from_ptr(p.add(1)).to_bytes();
                    if rest.eq_ignore_ascii_case(b"nvimrc") || rest.eq_ignore_ascii_case(b"exrc") {
                        *p = if *p == b'_' as c_char {
                            b'.' as c_char
                        } else {
                            b'_' as c_char
                        };
                        cookie.fp = crate::os::fs::CFile::from_raw(fopen_noinh_readbin(fname_exp));
                    }
                }
            }

            if cookie.fp.is_none() && !cookie.source_from_buf_or_str {
                if opt_vars().p_verbose > 1 {
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

            if opt_vars().p_verbose > 1 {
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
                if *opt_vars().p_ffs == NUL {
                    cookie.fileformat = EOL_DOS;
                } else {
                    cookie.fileformat = EOL_UNKNOWN;
                }
            }

            cookie.breakpoint = dbg_find_breakpoint(true, fname_exp, 0);
            cookie.fname = if fname_exp.is_null() {
                None
            } else {
                Some(CStr::from_ptr(fname_exp).to_owned())
            };
            cookie.dbg_tick = g_st().debug_tick;
            cookie.level = g_st().ex_nesting_level;

            let mut rel_time: proftime_T = 0;
            let mut start_time: proftime_T = 0;
            let l_has_time_fd = has_time_fd();
            if l_has_time_fd {
                time_push(&mut rel_time, &mut start_time);
            }

            let l_do_profiling = g_st().do_profiling;
            if l_do_profiling == PROF_YES {
                prof_child_enter(&mut wait_start);
            }

            save_funccal();

            let save_current_sctx = g_st().current_sctx;

            rt_st().last_current_SID_seq += 1;
            g_st().current_sctx.sc_seq = rt_st().last_current_SID_seq;

            if sid > 0 {
                si = script_item(sid);
            } else if str_ptr.is_null() {
                si = new_script_item(Some(CStr::from_ptr(fname_exp).to_owned()), &mut sid);
                (*si).sn_lua = path_with_extension(fname_exp, c"lua".as_ptr());
                if !ret_sid.is_null() {
                    *ret_sid = sid;
                }
            }
            debug_assert_eq!(!si.is_null(), str_ptr.is_null());

            if str_ptr.is_null() || !script_is_lua(g_st().current_sctx.sc_sid) {
                g_st().current_sctx.sc_sid = sid;
                g_st().current_sctx.sc_lnum = 0;
            }

            estack_push(
                ETYPE_SCRIPT,
                if !si.is_null() {
                    (*si).name_ptr()
                } else {
                    fname_exp
                },
                0,
            );

            if l_do_profiling == PROF_YES && !si.is_null() {
                let mut forceit = false;
                if !(*si).sn_prof_on && has_profiling(true, (*si).name_ptr(), &mut forceit) {
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
                && !{ let _a = (*g_st().curbuf).b_p_ft; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"lua" }
                && !(!(*g_st().curbuf).b_fname.is_null()
                    && path_with_extension((*g_st().curbuf).b_fname, c"lua".as_ptr()))
            {
                let mut args_items = [
                    integer_obj(Integer::from((*g_st().curbuf).b_fnum)),
                    integer_obj(Integer::from((*eap).line1)),
                    integer_obj(Integer::from((*eap).line2)),
                ];
                let args = Array {
                    items: args_items.as_mut_ptr(),
                    size: 3,
                    capacity: 3,
                };
                let mut err = Error::new();
                let result = nlua_exec(
                    static_cstr_as_string(c"return require('vim._core.util').source_is_lua(...)"),
                    ptr::null(),
                    args,
                    kRetNilBool,
                    ptr::null_mut(),
                    &mut err,
                );
                if err.err_type == ErrorType::kErrorTypeNone
                    && result.obj_type == ObjectType::kObjectTypeBoolean
                    && result.data.boolean()
                {
                    ts_lua = true;
                }
                api_clear_error(&mut err);
            }

            if fname.is_null()
                && (ex_lua
                    || ts_lua
                    || { let _a = (*g_st().curbuf).b_p_ft; !_a.is_null() && std::ffi::CStr::from_ptr(_a).to_bytes() == b"lua" }
                    || (!(*g_st().curbuf).b_fname.is_null()
                        && path_with_extension((*g_st().curbuf).b_fname, c"lua".as_ptr())))
            {
                nlua_exec_lines(&cookie.buflines, fname_exp);
            } else if !si.is_null() && (*si).sn_lua {
                nlua_exec_file(fname_exp);
            } else {
                firstline = getsourceline(0, &raw mut cookie, 0, true);
                if let Some(ref fl) = firstline
                    && fl.to_bytes().starts_with(&[0xef, 0xbb, 0xbf])
                {
                    convert_setup(&mut cookie.conv, c"utf-8".as_ptr().cast_mut(), opt_vars().p_enc);
                    let after_bom = fl.as_ptr().add(3).cast_mut();
                    firstline = Some(
                        string_convert(&cookie.conv, after_bom, ptr::null_mut()).map_or_else(
                            || cstr_to_cstring(after_bom),
                            |v| cstr_to_cstring(v.as_ptr().cast()),
                        ),
                    );
                }
                do_cmdline(
                    firstline
                        .as_ref()
                        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                    LineGetter::Source(&raw mut cookie),
                    DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_REPEAT,
                );
            }
            retval = OK;

            if l_do_profiling == PROF_YES && !si.is_null() {
                si = script_item(g_st().current_sctx.sc_sid);
                if (*si).sn_prof_on {
                    (*si).sn_pr_start = profile_end((*si).sn_pr_start);
                    (*si).sn_pr_start = profile_sub_wait(wait_start, (*si).sn_pr_start);
                    (*si).sn_pr_total = profile_add((*si).sn_pr_total, (*si).sn_pr_start);
                    (*si).sn_pr_self =
                        profile_self((*si).sn_pr_self, (*si).sn_pr_start, (*si).sn_pr_children);
                }
            }

            if g_st().got_int {
                emsg(gettext(e_interr.as_ptr()));
            }
            estack_pop();
            if opt_vars().p_verbose > 1 {
                verbose_enter();
                smsg!(0, gettext(c"finished sourcing %s".as_ptr()), fname);
                let sname = sourcing_name();
                if !sname.is_null() {
                    smsg!(0, gettext(c"continuing in %s".as_ptr()), sname);
                }
                verbose_leave();
            }

            if l_has_time_fd {
                let iobuff = (&raw mut g_st().IObuff).cast::<c_char>();
                vim_snprintf!(iobuff, IOSIZE, c"sourcing %s".as_ptr(), fname);
                time_msg(iobuff, &start_time);
                time_pop(rel_time);
            }

            if !g_st().got_int {
                trigger_source_post = true;
            }

            if save_debug_break_level > g_st().ex_nesting_level && g_st().debug_break_level == g_st().ex_nesting_level {
                g_st().debug_break_level += 1;
            }

            g_st().current_sctx = save_current_sctx;
            restore_funccal();
            if l_do_profiling == PROF_YES {
                prof_child_exit(&mut wait_start);
            }
            drop(firstline);
            drop(cookie);

            if str_ptr.is_null() && trigger_source_post {
                apply_autocmds(EVENT_SOURCEPOST, fname_exp, fname_exp, false, g_st().curbuf);
            }
        }

        drop(fname_exp_owned);
        retval
    }
}

pub unsafe fn do_source(
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
pub unsafe fn script_is_lua(sid: scid_T) -> bool {
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
pub unsafe fn find_script_by_name(name: *mut c_char) -> c_int {
    unsafe {
        let mut sid = script_items_len();
        while sid > 0 {
            let si = script_item(sid);
            if let Some(ref sn) = (*si).sn_name
                && path_equal(sn.as_ptr().cast_mut(), name, kPathCmpLiteral)
            {
                return sid;
            }
            sid -= 1;
        }
        -1
    }
}

/// ":scriptnames"
pub unsafe fn ex_scriptnames(eap: &mut exarg_T) {
    unsafe {
        let namebuff = (&raw mut g_st().NameBuff).cast::<c_char>();
        let iobuff = (&raw mut g_st().IObuff).cast::<c_char>();
        if eap.addr_count > 0 || *eap.arg != NUL {
            if eap.addr_count > 0 && !script_id_valid(eap.line2) {
                emsg(gettext(e_invarg.as_ptr()));
            } else {
                if eap.addr_count > 0 {
                    eap.arg = (*script_item(eap.line2)).name_ptr();
                } else {
                    expand_env(eap.arg, namebuff, MAXPATHL as c_int);
                    eap.arg = namebuff;
                    to_slash(eap.arg);
                }
                do_exedit(eap, ptr::null_mut());
            }
            return;
        }

        msg_ext_set_kind(c"list_cmd".as_ptr());
        let mut i = 1;
        while i <= script_items_len() && !g_st().got_int {
            let si = script_item(i);
            if let Some(ref sn) = (*si).sn_name {
                home_replace(ptr::null_mut(), sn.as_ptr(), namebuff, MAXPATHL, true);
                vim_snprintf!(iobuff, IOSIZE, c"%3d: %s".as_ptr(), i, namebuff);
                if !message_filtered(iobuff) {
                    if g_st().msg_col > 0 {
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

/// Get a script name (`get_scriptname`).
pub unsafe fn get_scriptname(
    script_ctx: sctx_T,
    home_replace: bool,
) -> std::borrow::Cow<'static, CStr> {
    use std::borrow::Cow;
    unsafe {
        let iobuff = (&raw mut g_st().IObuff).cast::<c_char>();
        let ptr = match script_ctx.sc_sid {
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
                let sname = (*script_item(script_ctx.sc_sid)).name_ptr();
                if sname.is_null() {
                    vim_snprintf!(
                        iobuff,
                        IOSIZE,
                        gettext(c"anonymous :source (script id %d)".as_ptr()),
                        script_ctx.sc_sid,
                    );
                    iobuff
                } else if home_replace {
                    return Cow::Owned(home_replace_save(ptr::null_mut(), sname));
                } else {
                    sname
                }
            }
        };
        Cow::Borrowed(CStr::from_ptr(ptr))
    }
}

pub unsafe fn free_scriptnames() {
    unsafe {
        profile_reset();
        rt_st().script_items = Vec::new();
    }
}

pub unsafe fn free_autoload_scriptnames() {
    unsafe {
        let loaded = &raw mut rt_st().ga_loaded;
        *loaded = Vec::new();
    }
}

pub unsafe fn get_sourced_lnum(fgetline: LineGetter) -> linenr_T {
    unsafe {
        if let LineGetter::Source(sp) = fgetline {
            (*sp).sourcing_lnum
        } else {
            sourcing_lnum()
        }
    }
}

unsafe fn get_script_local_funcs(sid: scid_T) -> *mut list_T {
    unsafe {
        let functbl = func_tbl_get();
        let l = tv_list_alloc(functbl.len() as isize);

        for &fp in functbl.values() {
            if (*fp).uf_script_ctx.sc_sid == sid {
                let name = if let Some(ref exp) = (*fp).uf_name_exp {
                    exp.as_ptr()
                } else {
                    (*fp).uf_name.as_ptr()
                };
                tv_list_append_string(l, name, -1);
            }
        }

        l
    }
}

/// "getscriptinfo()" function
pub unsafe fn f_getscriptinfo(argvars: *mut typval_T, rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, script_items_len() as isize);

        if tv_check_for_opt_dict_arg(argvars, 0) == FAIL {
            return;
        }

        let l = rettv.vval.v_list();

        let mut regmatch: regmatch_T = regmatch_T::new();
        regmatch.rm_ic = opt_vars().p_ic != 0;
        let mut filterpat = false;
        let mut sid: varnumber_T = -1;

        let mut pat_owned: Option<std::ffi::CString> = None;
        if (*argvars).v_type == VarType::VAR_DICT {
            let arg_dict = (*argvars).vval.v_dict();
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
                pat_owned = crate::eval::typval::tv_dict_get_cstring(arg_dict, c"name".as_ptr());
                if let Some(ref pat) = pat_owned {
                    regmatch.regprog = vim_regcomp(pat.as_ptr(), RE_MAGIC + RE_STRING);
                }
                if !regmatch.regprog.is_null() {
                    filterpat = true;
                }
            }
        }

        let mut i = if sid > 0 { sid } else { 1 };
        while (i == sid || sid <= 0) && i <= varnumber_T::from(script_items_len()) {
            let si = script_item(i as scid_T);

            if let Some(ref sn) = (*si).sn_name
                && (!filterpat || vim_regexec(&mut regmatch, sn.as_ptr(), 0))
            {
                let d = tv_dict_alloc();
                tv_list_append_dict(l, d);
                tv_dict_add_str(d, c"name".as_ptr(), 4, sn.as_ptr());
                tv_dict_add_nr(d, c"sid".as_ptr(), 3, i);
                tv_dict_add_nr(d, c"version".as_ptr(), 7, 1);
                tv_dict_add_bool(d, c"autoload".as_ptr(), 8, kFalse);

                if sid > 0 {
                    let var_dict =
                        tv_dict_copy(None, &mut (*si).sn_vars.sv_dict, true, get_copyID());
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
        drop(pat_owned);
    }
}

/// Get one full line from a sourced file.
pub unsafe fn getsourceline(
    _c: c_int,
    sp: *mut source_cookie_T,
    _indent: c_int,
    do_concat: bool,
) -> Option<std::ffi::CString> {
    unsafe {
        if (*sp).dbg_tick < g_st().debug_tick && !(*sp).source_from_buf_or_str {
            let fname_ptr = (*sp)
                .fname
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            (*sp).breakpoint = dbg_find_breakpoint(true, fname_ptr, sourcing_lnum());
            (*sp).dbg_tick = g_st().debug_tick;
        }
        if g_st().do_profiling == PROF_YES {
            script_line_end();
        }
        set_sourcing_lnum((*sp).sourcing_lnum + 1);
        let mut line_vec =
            if (*sp).finished || (!(*sp).source_from_buf_or_str && (*sp).fp.is_none()) {
                None
            } else if let Some(next) = (*sp).nextline.take() {
                (*sp).sourcing_lnum += 1;
                Some(next)
            } else {
                get_one_sourceline(sp)
            };
        if line_vec.is_some() && g_st().do_profiling == PROF_YES {
            script_line_start();
        }

        if let Some(ref mut buf) = line_vec
            && do_concat
            && vim_strchr(opt_vars().p_cpo, kCpoConcat).is_null()
        {
            (*sp).sourcing_lnum -= 1;

            (*sp).nextline = get_one_sourceline(sp);
            if let Some(ref next) = (*sp).nextline {
                let p = skipwhite(next.as_ptr());
                if *p == b'\\' as c_char
                    || (*p == b'"' as c_char
                        && *p.add(1) == b'\\' as c_char
                        && *p.add(2) == b' ' as c_char)
                {
                    if buf.last() == Some(&NUL) {
                        buf.pop();
                    }
                    buf.reserve(400);
                    while let Some(ref next) = (*sp).nextline
                        && concat_continued_line(
                            buf,
                            next.as_ptr(),
                            CStr::from_ptr(next.as_ptr()).to_bytes().len(),
                        )
                    {
                        (*sp).nextline = get_one_sourceline(sp);
                    }
                    if buf.last() != Some(&NUL) {
                        buf.push(NUL);
                    }
                }
            }
        }

        let line = line_vec.map(|b| {
            let len = b.len().saturating_sub(usize::from(b.last() == Some(&NUL)));
            if (*sp).conv.vc_type != CONV_NONE
                && let Some(s) =
                    string_convert(&(*sp).conv, b.as_ptr().cast_mut(), ptr::null_mut())
            {
                cstr_to_cstring(s.as_ptr().cast())
            } else {
                cstr_from_bytes(std::slice::from_raw_parts(b.as_ptr().cast::<u8>(), len))
            }
        });

        if !(*sp).source_from_buf_or_str
            && (*sp).breakpoint != 0
            && (*sp).breakpoint <= sourcing_lnum()
        {
            let fname_ptr = (*sp)
                .fname
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            dbg_breakpoint(fname_ptr, sourcing_lnum());
            (*sp).breakpoint = dbg_find_breakpoint(true, fname_ptr, sourcing_lnum());
            (*sp).dbg_tick = g_st().debug_tick;
        }

        line
    }
}

unsafe fn get_one_sourceline(sp: *mut source_cookie_T) -> Option<Vec<c_char>> {
    unsafe {
        let mut buf: Vec<c_char> = Vec::with_capacity(250);
        let mut have_read = false;

        (*sp).sourcing_lnum += 1;
        loop {
            #[cfg(windows)]
            let mut len: usize;
            #[cfg(not(windows))]
            let len: usize;
            let mut filled_chunk = false;
            if (*sp).source_from_buf_or_str {
                if ((*sp).buf_lnum as usize) >= (*sp).buflines.len() {
                    break;
                }
                let line_bytes = (*sp).buflines.as_slice()[(*sp).buf_lnum as usize].to_bytes();
                buf.extend_from_slice(std::slice::from_raw_parts(
                    line_bytes.as_ptr().cast::<c_char>(),
                    line_bytes.len(),
                ));
                (*sp).buf_lnum += 1;
                buf.push(NUL);
                len = buf.len();
            } else {
                let start = buf.len();
                buf.resize(start + 120, 0);
                let mut eof = false;
                let fp = (&*sp).fp.as_ref().unwrap().as_ptr();
                loop {
                    *errno_ptr() = 0;
                    if fgets(buf.as_mut_ptr().add(start), 120, fp).is_null() {
                        if *errno_ptr() == EINTR {
                            continue;
                        }
                        eof = true;
                    }
                    break;
                }
                if eof {
                    buf.truncate(start);
                    break;
                }
                let read_len = CStr::from_ptr(buf.as_ptr().add(start)).to_bytes().len();
                filled_chunk = read_len == 119;
                len = start + read_len;
                buf.truncate(len);
            }

            #[cfg(windows)]
            {
                if (len == 1 || (len >= 2 && buf[len - 2] == b'\n' as c_char))
                    && (*sp).fileformat == EOL_DOS
                    && buf[len - 1] == Ctrl_Z
                {
                    buf.truncate(len - 1);
                    break;
                }
            }

            have_read = true;

            if !(*sp).source_from_buf_or_str && filled_chunk && buf[len - 1] != b'\n' as c_char {
                continue;
            }

            if len >= 1 && buf[len - 1] == b'\n' as c_char {
                #[cfg(windows)]
                {
                    let has_cr = len >= 2 && buf[len - 2] == b'\r' as c_char;
                    if (*sp).fileformat == EOL_UNKNOWN {
                        (*sp).fileformat = if has_cr { EOL_DOS } else { EOL_UNIX };
                    }

                    if (*sp).fileformat == EOL_DOS {
                        if has_cr {
                            buf[len - 2] = b'\n' as c_char;
                            len -= 1;
                            buf.truncate(len);
                        } else {
                            if !(*sp).error {
                                msg_source(crate::highlight::hl_attr(HLF_W as c_int));
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

                let mut c = (len as isize) - 2;
                while c >= 0 && buf[c as usize] == Ctrl_V {
                    c -= 1;
                }
                if ((len as isize) & 1) != (c & 1) {
                    (*sp).sourcing_lnum += 1;
                    continue;
                }

                buf.truncate(len - 1);
            }

            line_breakcheck();
            break;
        }

        if have_read {
            if buf.last() != Some(&NUL) {
                buf.push(NUL);
            }
            return Some(buf);
        }

        None
    }
}

/// Returns true if sourcing a script either from a file or a buffer or a string.
pub unsafe fn sourcing_a_script(eap: *mut exarg_T) -> c_int {
    unsafe { c_int::from((*eap).ea_getline.is_source()) }
}

/// ":scriptencoding": Set encoding conversion for a sourced script.
pub unsafe fn ex_scriptencoding(eap: &mut exarg_T) {
    unsafe {
        let Some(sp) = eap.ea_getline.as_source() else {
            emsg(gettext(
                c"E167: :scriptencoding used outside of a sourced file".as_ptr(),
            ));
            return;
        };

        let name_owned: Option<std::ffi::CString>;
        let name = if *eap.arg != NUL {
            name_owned = Some(enc_canonize(eap.arg));
            name_owned.as_ref().unwrap().as_ptr().cast_mut()
        } else {
            eap.arg
        };

        convert_setup(&mut (*sp).conv, name, opt_vars().p_enc);
    }
}

/// ":finish": Mark a sourced file as finished.
pub unsafe fn ex_finish(eap: &mut exarg_T) {
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
pub unsafe fn do_finish(eap: *mut exarg_T, reanimate: bool) {
    unsafe {
        let sp = (*eap).ea_getline.as_source().unwrap();
        if reanimate {
            (*sp).finished = false;
        }

        let cstack = (*eap).cstack.cast::<cstack_T>();
        let idx = cleanup_conditionals(cstack, 0, 1);
        if idx >= 0 {
            (*cstack).cs_pending[idx as usize] = CSTP_FINISH as c_char;
            report_make_pending(CSTP_FINISH, PendingVal::None);
        } else {
            (*sp).finished = true;
        }
    }
}

pub unsafe fn source_finished(fgetline: LineGetter) -> bool {
    unsafe { fgetline.as_source().is_some_and(|sp| (*sp).finished) }
}

/// Return the autoload script name for a function or variable name.
pub unsafe fn autoload_name(name: *const c_char, name_len: usize) -> std::ffi::CString {
    unsafe {
        const PREFIX: &[u8] = b"autoload/";
        const SUFFIX: &[u8] = b".vim";
        let mut scriptname = Vec::with_capacity(name_len + PREFIX.len() + SUFFIX.len());
        scriptname.extend_from_slice(PREFIX);
        scriptname.extend_from_slice(std::slice::from_raw_parts(name.cast::<u8>(), name_len));
        let mut auchar_idx: usize = 0;
        let mut i = PREFIX.len();
        while i - PREFIX.len() < name_len {
            if scriptname[i] == AUTOLOAD_CHAR {
                scriptname[i] = b'/';
                auchar_idx = i;
            }
            i += 1;
        }
        scriptname.truncate(auchar_idx);
        scriptname.extend_from_slice(SUFFIX);

        cstr_from_bytes(&scriptname)
    }
}

/// If name has a package name try autoloading the script for it.
pub unsafe fn script_autoload(name: *const c_char, name_len: usize, reload: bool) -> bool {
    unsafe {
        let slice = std::slice::from_raw_parts(name.cast::<u8>(), name_len);
        let Some(pos) = slice.iter().position(|&b| b == AUTOLOAD_CHAR) else {
            return false;
        };
        if pos == 0 {
            return false;
        }

        let mut ret = false;
        let scriptname_cstr = autoload_name(name, name_len);
        let scriptname = scriptname_cstr.as_ptr().cast_mut();

        let target_tail = &scriptname_cstr.as_bytes()[9..];
        let loaded = &raw mut rt_st().ga_loaded;
        let already_loaded = (*loaded)
            .iter()
            .any(|entry| &entry.as_bytes()[9..] == target_tail);
        if reload || !already_loaded {
            if !already_loaded {
                (*loaded).push(scriptname_cstr.clone());
            }

            if do_in_runtimepath(
                scriptname,
                DIP_START,
                &mut RuntimeCallback::Source {
                    skip_if_loaded: true,
                },
            ) == OK
            {
                ret = true;
            }
        }

        ret
    }
}
