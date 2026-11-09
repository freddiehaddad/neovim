//! Management of runtime files (including packages) (`src/nvim/runtime.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    SID_API_CLIENT, SID_LUA, add_c, api_clear_error, arena_array, arena_array_opt, arena_dict,
    arena_string_opt, boolean_obj, cstr_as_obj, cstr_as_string, dict_obj, integer_obj, put_c,
    static_cstr_as_string, string_obj,
};
use crate::api::private::validate::{Array, Error, ErrorType, NvimString, ObjectType};
use crate::autocmd::{
    DOCMD_NOWAIT, DOCMD_REPEAT, DOCMD_VERBOSE, EVENT_SOURCECMD, EVENT_SOURCEPOST, EVENT_SOURCEPRE,
    PROF_YES, apply_autocmds, has_autocmd, has_autocmd_fullname,
};
use crate::charset::CTRL_V_CONST;
use crate::cmdexpand::{EXPAND_RUNTIME, LineGetter, WILD_ADD_SLASH, globpath};
use crate::cursor::Linenr;
use crate::debugger::{dbg_breakpoint, dbg_find_breakpoint, has_profiling};
use crate::errors::*;
use crate::eval::typval::{
    EvalFuncData, ScopeDictDictItem, TvDict, TvList, TypVal, TypvalVval, VAR_LOCKED, VarType,
    Varnumber, tv_check_for_opt_dict_arg, tv_dict_add_bool, tv_dict_add_dict, tv_dict_add_func,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_alloc, tv_dict_alloc_lock,
    tv_dict_copy, tv_get_number_chk, tv_get_string, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_dict, tv_list_append_string, tv_list_append_tv,
};
use crate::eval::userfunc::{UserFunc, func_tbl_get, restore_funccal, save_funccal};
use crate::eval::vars::{SID_STR, Scid, new_script_vars};
use crate::eval::{eval_to_number, get_copy_id};
#[cfg(windows)]
use crate::ex_docmd::CTRL_Z_CONST;
use crate::ex_docmd::{Exarg, do_cmdline, do_cmdline_cmd, do_exedit};
use crate::ex_eval::{
    CSTP_FINISH, PendingVal, aborting, cleanup_conditionals, report_make_pending,
};
use crate::fileio::{EINTR, O_RDONLY};
#[cfg(windows)]
use crate::fileio::{EOL_DOS, EOL_UNIX, EOL_UNKNOWN};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::input::openscript;
use crate::lua::executor::{
    K_RET_NIL_BOOL, nlua_exec, nlua_exec_file, nlua_exec_lines, nlua_is_deferred_safe,
};
use crate::mbyte::{Vimconv, convert_setup, enc_canonize, string_convert, utfc_ptr2len};
use crate::memory::{Arena, bytes_to_cstring};
#[cfg(windows)]
use crate::message::msg_source;
use crate::message::{
    emsg, message_filtered, msg_ext_set_kind, msg_ext_ui_flush, msg_outtrans, msg_putchar, semsg,
    smsg, verbose_enter, verbose_leave,
};
use crate::msgpack_rpc::packer::Integer;
use crate::option::{K_OPT_RUNTIMEPATH, copy_option_part, set_option_value_give_err, vimrc_found};
use crate::option_vars::*;
use crate::optionstr::Optset;
use crate::os::env::{
    ExpandCtx, expand_env, expand_env_save, home_replace, home_replace_save, os_setenv,
    vim_env_iter, vim_env_iter_rev, vim_get_prefix_from_exepath, vim_getenv,
};
#[cfg(windows)]
use crate::os::fs::O_BINARY;
use crate::os::fs::{PATHSEP, os_file_is_readable, os_isdir, os_open, os_set_cloexec};
use crate::os::input::line_breakcheck;
use crate::os::stdpaths::{XDGVarType, get_appname, stdpaths_get_xdg_var};
use crate::path::{
    ENV_SEPCHAR, EW_DIR, EW_FILE, EW_NOBREAK, K_PATH_CMP_LITERAL, add_pathsep, after_pathsep,
    append_path, fix_fname, gen_expand_wildcards_vec, get_past_head, path_cmp, path_equal,
    path_tail, path_with_extension, to_slash, vim_ispathsep, vim_ispathsep_nocolon,
};
use crate::profile::{
    Estack, Proftime, Scriptitem, prof_child_enter, prof_child_exit, profile_add, profile_end,
    profile_init, profile_reset, profile_self, profile_start, profile_sub_wait, profile_zero,
    script_line_end, script_line_start, time_msg, time_pop, time_push,
};
use crate::regexp::{RE_MAGIC, RE_STRING, Regmatch, vim_regcomp, vim_regexec};
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext};
use crate::usercmd::add_win_cmd_modifiers;
use crate::viml::parser::expressions::AUTOLOAD_CHAR;
use std::ffi::CStr;
use std::ptr;
use std::sync::Mutex;

pub type Etype = i32;
pub type EstackArg = i32;
pub(crate) const SID_NONE: Scid = -6;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Sctx {
    pub sc_sid: i32,
    pub sc_seq: i32,
    pub sc_lnum: i32,
    pub sc_chan: u64,
}

impl Sctx {
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
    SpellLoad(&'a mut crate::spell::Spelload),
    FoundTagfile,
}

impl RuntimeCallback<'_> {
    fn invoke(&mut self, fnames: &[std::ffi::CString], all: bool) {
        match self {
            Self::Source { skip_if_loaded } => {
                let mut sid: i32 = 0;
                let cookie = (*skip_if_loaded).then_some(&mut sid);
                let _ = source_callback(fnames, all, cookie);
            }
            Self::SourceVimLua => {
                let _ = source_callback_vim_lua(fnames, all, None);
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

pub const ETYPE_TOP: Etype = 0;
pub const ETYPE_SCRIPT: Etype = 1;
pub const ETYPE_UFUNC: Etype = 2;
pub const ETYPE_AUCMD: Etype = 3;
pub const ETYPE_MODELINE: Etype = 4;
pub const ETYPE_EXCEPT: Etype = 5;
pub const ETYPE_ARGS: Etype = 6;
pub const ETYPE_ENV: Etype = 7;
pub const ETYPE_SPELL: Etype = 9;

pub const ESTACK_NONE: EstackArg = 0;
pub const ESTACK_SFILE: EstackArg = 1;
pub const ESTACK_STACK: EstackArg = 2;
pub const ESTACK_SCRIPT: EstackArg = 3;

pub const DOSO_NONE: i32 = 0;
pub const DOSO_VIMRC: i32 = 1;

pub const DIP_ALL: i32 = 0x01;
pub const DIP_DIR: i32 = 0x02;
pub const DIP_ERR: i32 = 0x04;
pub const DIP_START: i32 = 0x08;
pub const DIP_OPT: i32 = 0x10;
pub const DIP_NORTP: i32 = 0x20;
pub const DIP_NOAFTER: i32 = 0x40;
pub const DIP_AFTER: i32 = 0x80;
pub const DIP_DIRFILE: i32 = 0x200;

const K_CPO_CONCAT: i32 = b'C' as i32;

pub(crate) const SID_CMDARG: Scid = -2;
pub(crate) const SID_CARG: Scid = -3;
pub(crate) const SID_ENV: Scid = -4;
pub(crate) const SID_ERROR: Scid = -5;
pub(crate) const SID_WINLAYOUT: Scid = -7;

#[cfg(windows)]
const HLF_W: usize = 26;

#[cfg(windows)]
const O_NOINHERIT: i32 = 0x0080;

#[repr(C)]
pub struct Scriptvar {
    pub sv_var: ScopeDictDictItem,
    pub sv_dict: TvDict,
}

impl Scriptvar {
    pub const fn new() -> Self {
        Self {
            sv_var: ScopeDictDictItem::new(),
            sv_dict: TvDict::new(),
        }
    }
}

pub struct SourceCookie {
    fp: Option<crate::os::fs::CFile>,
    nextline: Option<Vec<u8>>,
    sourcing_lnum: Linenr,
    finished: bool,
    source_from_buf_or_str: bool,
    buf_lnum: i32,
    buflines: Vec<std::ffi::CString>,
    #[cfg(windows)]
    fileformat: i32,
    #[cfg(windows)]
    error: bool,
    breakpoint: Linenr,
    fname: Option<std::ffi::CString>,
    dbg_tick: i32,
    level: i32,
    conv: Vimconv,
}

impl SourceCookie {
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
            conv: Vimconv {
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
    pub(crate) exestack: Vec<Estack>,
    pub(crate) script_items: Vec<Scriptitem>,
    ga_loaded: Vec<std::ffi::CString>,
    last_current_sid_seq: i32,
    last_current_sid: Scid,
    runtime_search_path_valid: bool,
    runtime_search_path_valid_thread: bool,
    runtime_search_path_refcount: usize,
    runtime_search_path_old: Vec<RuntimeSearchPath>,
    runtime_search_path: RuntimeSearchPath,
    runtime_search_path_thread: RuntimeSearchPath,
    runtime_expand_flags: i32,
    get_named_buf: [u8; MAXPATHL],
    get_named_thread_buf: [u8; MAXPATHL],
    pack_entry_buf: [u8; MAXPATHL],
    build_rtp_buf: [u8; MAXPATHL],
    pack_start_buf: [u8; MAXPATHL],
}

impl RuntimeModuleState {
    const fn new() -> Self {
        Self {
            exestack: Vec::new(),
            script_items: Vec::new(),
            ga_loaded: Vec::new(),
            last_current_sid_seq: 0,
            last_current_sid: 0,
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
pub(crate) fn rt_st() -> crate::globals::GraphPtr<RuntimeModuleState> {
    RT_STATE.handle()
}

use crate::buffer::{SID_MODELINE, VAR_FIXED};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE};
use crate::os::fs::MAXPATHL;
use crate::version::default_lib_dir;
use crate::viml::parser::parser::CONV_NONE;

#[inline]
pub fn script_item(id: Scid) -> &'static mut Scriptitem {
    &mut rt_st().borrow_mut().script_items[(id - 1) as usize]
}

#[inline]
pub fn script_items_len() -> Scid {
    rt_st().script_items.len() as Scid
}

#[inline]
pub fn script_id_valid(id: Scid) -> bool {
    id > 0 && id <= script_items_len()
}

#[inline]
pub fn sourcing_name() -> Option<&'static CStr> {
    rt_st().borrow_mut().exestack.last().unwrap().name_cstr()
}

#[inline]
pub fn sourcing_lnum() -> Linenr {
    rt_st().exestack.last().unwrap().es_lnum
}

#[inline]
pub fn set_sourcing_lnum(lnum: Linenr) {
    rt_st().exestack.last_mut().unwrap().es_lnum = lnum;
}

/// Initialize the execution stack.
pub fn estack_init() {
    rt_st().exestack.reserve(10);
    rt_st().exestack.push(Estack {
        es_type: ETYPE_TOP,
        es_name: None,
        es_lnum: 0,
        es_info: crate::profile::EStackInfo::None,
    });
}

/// Add an item to the execution stack.
pub fn estack_push(type_: Etype, name: Option<&CStr>, lnum: Linenr) -> &'static mut Estack {
    rt_st().exestack.push(Estack {
        es_type: type_,
        es_name: name.map(CStr::to_owned),
        es_lnum: lnum,
        es_info: crate::profile::EStackInfo::None,
    });
    rt_st().borrow_mut().exestack.last_mut().unwrap()
}

/// Add a user function to the execution stack.
pub fn estack_push_ufunc(ufunc: &mut UserFunc, lnum: Linenr) {
    let name: &CStr = if let Some(ref exp) = ufunc.uf_name_exp {
        exp
    } else {
        &ufunc.uf_name
    };
    let entry = estack_push(ETYPE_UFUNC, Some(name), lnum);
    entry.es_info = crate::profile::EStackInfo::Ufunc(ufunc.into());
}

/// Take an item off of the execution stack.
pub fn estack_pop() {
    if rt_st().exestack.len() > 1 {
        rt_st().exestack.pop();
    }
}

/// Get the current value for <sfile> as an owned `CString`.
pub fn estack_sfile(which: EstackArg) -> Option<std::ffi::CString> {
    let last_entry = rt_st().borrow_mut().exestack.last().unwrap();
    if which == ESTACK_SFILE && last_entry.es_type != ETYPE_UFUNC {
        return last_entry.es_name.clone();
    }

    if which == ESTACK_SCRIPT {
        for entry in rt_st().exestack.iter().rev() {
            if entry.es_type == ETYPE_UFUNC || entry.es_type == ETYPE_AUCMD {
                let def_ctx = if entry.es_type == ETYPE_UFUNC {
                    entry.es_info.ufunc().unwrap().uf_script_ctx
                } else {
                    entry.es_info.aucmd_sctx()
                };
                return if def_ctx.sc_sid > 0 {
                    script_item(def_ctx.sc_sid).name_cstr().map(CStr::to_owned)
                } else {
                    None
                };
            } else if entry.es_type == ETYPE_SCRIPT {
                return entry.es_name.clone();
            }
        }
        return None;
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut last_type = ETYPE_SCRIPT;
    let es_len = rt_st().exestack.len();
    for (idx, entry) in rt_st().exestack.iter().enumerate() {
        if let Some(ref entry_name) = entry.es_name {
            let mut type_name = static_cstr_as_string(c"");
            let es_name = cstr_as_string(Some(entry_name));
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
                buf.extend_from_slice(type_name.as_bytes());
            }
            if !es_name.data.is_null() && es_name.size > 0 {
                buf.extend_from_slice(es_name.as_bytes());
            }
            if lnum != 0 {
                let mut lnum_buf = [0u8; 32];
                let n = format_to_buf_safelen!(&mut lnum_buf, c"[%d]", lnum);
                buf.extend_from_slice(&lnum_buf[..n]);
            }
            if idx != es_len - 1 {
                buf.extend_from_slice(b"..");
            }
        }
    }

    if buf.is_empty() {
        None
    } else {
        Some(bytes_to_cstring(&buf))
    }
}

fn stacktrace_push_item(
    l: &mut TvList,
    fp: Option<&mut UserFunc>,
    event: Option<&CStr>,
    lnum: Linenr,
    filepath: Option<&CStr>,
) {
    let d = tv_dict_alloc_lock(VAR_FIXED);
    let mut tv = TypVal {
        v_type: VarType::VarDict,
        v_lock: VAR_LOCKED,
        vval: TypvalVval::Dict(crate::globals::GraphPtr::new(d)),
    };

    if let Some(fp) = fp {
        tv_dict_add_func(&mut *d, c"funcref", fp);
    }
    if let Some(ev) = event {
        tv_dict_add_str(&mut *d, c"event", Some(ev));
    }
    tv_dict_add_nr(&mut *d, c"lnum", Varnumber::from(lnum));
    tv_dict_add_str(&mut *d, c"filepath", filepath);

    tv_list_append_tv(l, &mut tv);
}

/// Create the stacktrace from rt_st().exestack.
pub fn stacktrace_create() -> &'static mut TvList {
    let l = tv_list_alloc(rt_st().exestack.len() as isize);

    for entry in &rt_st().exestack {
        let mut lnum = entry.es_lnum;

        if entry.es_type == ETYPE_SCRIPT {
            stacktrace_push_item(&mut *l, None, None, lnum, entry.name_cstr());
        } else if entry.es_type == ETYPE_UFUNC {
            let fp = entry.es_info.ufunc();
            let sctx = fp.as_deref().unwrap().uf_script_ctx;
            let filepath = if sctx.sc_sid > 0 {
                get_scriptname(sctx, false)
            } else {
                std::borrow::Cow::Borrowed(c"")
            };
            lnum += sctx.sc_lnum;
            stacktrace_push_item(&mut *l, fp, None, lnum, Some(&filepath));
        } else if entry.es_type == ETYPE_AUCMD {
            let sctx = entry.es_info.aucmd_sctx();
            let filepath = if sctx.sc_sid > 0 {
                get_scriptname(sctx, false)
            } else {
                std::borrow::Cow::Borrowed(c"")
            };
            lnum += sctx.sc_lnum;
            stacktrace_push_item(&mut *l, None, entry.name_cstr(), lnum, Some(&filepath));
        }
    }
    l
}

/// getstacktrace() function
pub fn f_getstacktrace(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let l = stacktrace_create();
    rettv.v_type = VarType::VarList;
    rettv.vval = crate::eval::typval::TypvalVval::List(crate::globals::GraphPtr::new(l));
    l.lv_refcount += 1;
}

fn get_runtime_cmd_flags(argp: &mut &CStr, where_len: usize) -> i32 {
    let arg = *argp;
    if where_len == 0 {
        return 0;
    }
    let arg_bytes = arg.to_bytes();
    let slice = &arg_bytes[..where_len.min(arg_bytes.len())];
    if b"START".starts_with(slice) {
        *argp = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(arg, where_len));
        return DIP_START + DIP_NORTP;
    }
    if b"OPT".starts_with(slice) {
        *argp = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(arg, where_len));
        return DIP_OPT + DIP_NORTP;
    }
    if b"PACK".starts_with(slice) {
        *argp = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(arg, where_len));
        return DIP_START + DIP_OPT + DIP_NORTP;
    }
    if b"ALL".starts_with(slice) {
        *argp = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(arg, where_len));
        return DIP_START + DIP_OPT;
    }
    0
}

/// ":runtime [where] {name}"
pub fn ex_runtime(eap: &mut Exarg) {
    let mut arg = eap.arg.as_cstr();
    let mut flags = if eap.forceit { DIP_ALL } else { 0 };
    let where_len = crate::charset::skiptowhite_len(arg.to_bytes());
    flags += get_runtime_cmd_flags(&mut arg, where_len);
    source_runtime(arg, flags);
}

/// Set the completion context for the :runtime command.
pub fn set_context_in_runtime_cmd(xp: &mut ExpandCtx, mut arg: &CStr) {
    let p_rem = crate::charset::skiptowhite_slice(arg.to_bytes());
    rt_st().runtime_expand_flags = if !p_rem.is_empty() {
        let len = arg.to_bytes().len() - p_rem.len();
        get_runtime_cmd_flags(&mut arg, len)
    } else {
        0
    };
    let mut arg_cstr = arg;
    loop {
        let step = crate::charset::skiptowhite_esc_len(arg_cstr.to_bytes());
        let p = crate::memory::CStrSubslice::advance(arg_cstr, step);
        if p.to_bytes().is_empty() {
            break;
        }
        if rt_st().runtime_expand_flags == 0 {
            rt_st().runtime_expand_flags = DIP_ALL;
        }
        arg_cstr = crate::charset::skipwhite(p);
    }
    xp.xp_context = EXPAND_RUNTIME;
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg_cstr);
}

fn source_callback_vim_lua(
    fnames: &[std::ffi::CString],
    all: bool,
    mut cookie: Option<&mut i32>,
) -> bool {
    let mut did_one = false;

    for fname_cstr in fnames {
        if path_with_extension(fname_cstr, c"vim") {
            do_source(fname_cstr, false, DOSO_NONE, cookie.as_deref_mut());
            did_one = true;
            if !all {
                return true;
            }
        }
    }

    for fname_cstr in fnames {
        if path_with_extension(fname_cstr, c"lua") {
            do_source(fname_cstr, false, DOSO_NONE, cookie.as_deref_mut());
            did_one = true;
            if !all {
                return true;
            }
        }
    }

    did_one
}

fn source_callback(fnames: &[std::ffi::CString], all: bool, mut cookie: Option<&mut i32>) -> bool {
    let mut did_one = source_callback_vim_lua(fnames, all, cookie.as_deref_mut());

    if !all && did_one {
        return true;
    }

    for fname_cstr in fnames {
        if !path_with_extension(fname_cstr, c"vim") && !path_with_extension(fname_cstr, c"lua") {
            do_source(fname_cstr, false, DOSO_NONE, cookie.as_deref_mut());
            did_one = true;
            if !all {
                return true;
            }
        }
    }

    did_one
}

/// Find the patterns in "name" in all directories in "path" and invoke `callback`.
pub fn do_in_path(
    path_cstr: &std::ffi::CStr,
    prefix: &std::ffi::CStr,
    name_opt: Option<&std::ffi::CStr>,
    flags: i32,
    callback: &mut RuntimeCallback<'_>,
) -> bool {
    let path = path_cstr.as_ptr();
    let mut did_one = false;

    let rtp_copy = path_cstr.to_owned();
    let mut buf_vec = vec![0u8; MAXPATHL];
    {
        if opt_vars().p_verbose > 10
            && let Some(name) = name_opt
        {
            verbose_enter();
            if !prefix.to_bytes().is_empty() {
                smsg!(
                    0,
                    gettext(c"Searching for \"%s\" under \"%s\" in \"%s\""),
                    name,
                    prefix,
                    &rtp_copy,
                );
            } else {
                smsg!(
                    0,
                    gettext(c"Searching for \"%s\" in \"%s\""),
                    name,
                    &rtp_copy,
                );
            }
            verbose_leave();
        }

        let do_all = (flags & DIP_ALL) != 0;

        let mut rtp = rtp_copy.as_bytes();
        while !rtp.is_empty() && (do_all || !did_one) {
            let buflen = copy_option_part(&mut rtp, &mut buf_vec[..MAXPATHL], c",");

            if (flags & (DIP_NOAFTER | DIP_AFTER)) != 0 {
                let is_after = path_is_after(&buf_vec[..buflen]);
                if (is_after && (flags & DIP_NOAFTER) != 0)
                    || (!is_after && (flags & DIP_AFTER) != 0)
                {
                    continue;
                }
            }

            if let Some(name) = name_opt {
                let prefix_len = prefix.to_bytes().len();
                let name_len = name.to_bytes().len();
                if buflen + 2 + prefix_len + name_len < MAXPATHL {
                    add_pathsep(&mut buf_vec);
                    let cur_len = crate::memory::bytes_until_nul(&buf_vec).len();
                    buf_vec[cur_len..(cur_len + prefix_len + 1)]
                        .copy_from_slice(prefix.to_bytes_with_nul());

                    let mut np = name.to_bytes();
                    while !np.is_empty() && (do_all || !did_one) {
                        let tail_off = cur_len + prefix_len;
                        debug_assert!(MAXPATHL >= tail_off);
                        copy_option_part(&mut np, &mut buf_vec[tail_off..MAXPATHL], c"\t ");

                        if opt_vars().p_verbose > 10 {
                            verbose_enter();
                            smsg!(0, gettext(c"Searching for \"%s\""), &buf_vec);
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
                            crate::memory::bytes_as_cstr(&buf_vec),
                            ew_flags,
                            do_all,
                            callback,
                        );
                    }
                }
            } else {
                callback.invoke(&[crate::memory::cstring_until_nul(&buf_vec)], do_all);
                did_one = true;
            }
        }
    }
    drop(buf_vec);
    drop(rtp_copy);
    if !did_one && let Some(name) = name_opt {
        let basepath = if ptr::eq(path, opt_vars().p_rtp.as_ptr()) {
            c"runtimepath"
        } else {
            c"packpath"
        };

        if (flags & DIP_ERR) != 0 {
            semsg!(gettext(E_DIRNOTF), basepath, name);
        } else if opt_vars().p_verbose > 1 {
            verbose_enter();
            smsg!(0, gettext(c"not found in '%s': \"%s\""), basepath, name);
            verbose_leave();
        }
    }

    did_one
}

fn runtime_search_path_get_cached() -> &'static mut [SearchPathItem] {
    runtime_search_path_validate();
    rt_st().runtime_search_path_refcount += 1;
    rt_st().borrow_mut().runtime_search_path.as_mut_slice()
}

fn copy_runtime_search_path(src: &[SearchPathItem]) -> RuntimeSearchPath {
    src.to_vec()
}

fn runtime_search_path_unref() {
    rt_st().runtime_search_path_refcount -= 1;
    if rt_st().runtime_search_path_refcount == 0 {
        rt_st().runtime_search_path_old.clear();
    }
}

fn do_in_cached_path(
    name_opt: Option<&std::ffi::CStr>,
    flags: i32,
    callback: &mut RuntimeCallback<'_>,
) -> bool {
    let mut did_one = false;
    let mut buf = [0u8; MAXPATHL];

    if opt_vars().p_verbose > 10
        && let Some(name) = name_opt
    {
        verbose_enter();
        smsg!(0, gettext(c"Searching for \"%s\" in runtime path"), name);
        verbose_leave();
    }

    let path: &[SearchPathItem] = runtime_search_path_get_cached();

    let do_all = (flags & DIP_ALL) != 0;

    for item in path {
        let buflen = item.path.as_bytes().len();

        if (flags & (DIP_NOAFTER | DIP_AFTER)) != 0
            && ((item.after && (flags & DIP_NOAFTER) != 0)
                || (!item.after && (flags & DIP_AFTER) != 0))
        {
            continue;
        }

        let Some(name) = name_opt else {
            callback.invoke(std::slice::from_ref(&item.path), do_all);
            continue;
        };
        if buflen + name.to_bytes().len() + 2 < MAXPATHL {
            buf[..=buflen].copy_from_slice(item.path.to_bytes_with_nul());
            add_pathsep(&mut buf);
            let tail_off = crate::memory::bytes_until_nul(&buf).len();

            let mut np = name.to_bytes();
            while !np.is_empty() && (do_all || !did_one) {
                debug_assert!(MAXPATHL >= tail_off);
                copy_option_part(&mut np, &mut buf[tail_off..MAXPATHL], c"\t ");

                if opt_vars().p_verbose > 10 {
                    verbose_enter();
                    smsg!(0, gettext(c"Searching for \"%s\""), &buf);
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

                did_one |= gen_expand_wildcards_and_cb(
                    crate::memory::bytes_as_cstr(&buf),
                    ew_flags,
                    do_all,
                    callback,
                );
            }
        }
    }

    if !did_one && let Some(name) = name_opt {
        if (flags & DIP_ERR) != 0 {
            semsg!(gettext(E_DIRNOTF), c"runtime path", name);
        } else if opt_vars().p_verbose > 1 {
            verbose_enter();
            smsg!(0, gettext(c"not found in runtime path: \"%s\""), name);
            verbose_leave();
        }
    }

    runtime_search_path_unref();

    did_one
}

pub fn runtime_inspect(arena: &mut Arena) -> Array {
    let rsp = &rt_st().runtime_search_path;
    let mut rv = arena_array(arena, rsp.len());

    for item in rsp {
        let mut entry = arena_dict(arena, 5);
        put_c(&mut entry, c"path", cstr_as_obj(Some(&*item.path)));
        if item.after {
            put_c(&mut entry, c"after", boolean_obj(true));
        }
        if item.pack_inserted {
            put_c(&mut entry, c"pack_inserted", boolean_obj(true));
        }
        if item.has_lua != K_NONE {
            put_c(&mut entry, c"has_lua", boolean_obj(item.has_lua == K_TRUE));
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

pub fn runtime_get_named(lua: bool, pat: Array, all: bool, arena: &mut Arena) -> Array {
    let path = runtime_search_path_get_cached();
    let rv = runtime_get_named_common(lua, pat, all, path, &mut rt_st().get_named_buf, Some(arena));
    runtime_search_path_unref();
    rv
}

pub fn runtime_get_named_thread(lua: bool, pat: Array, all: bool) -> Array {
    let _guard = RUNTIME_SEARCH_PATH_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let path = &mut rt_st().runtime_search_path_thread;
    let buf: &mut [u8] = &mut rt_st().get_named_thread_buf;
    runtime_get_named_common(lua, pat, all, path, buf, None)
}

fn runtime_get_named_common(
    lua: bool,
    pat: Array,
    all: bool,
    path: &mut [SearchPathItem],
    buf_slice: &mut [u8],
    mut arena: Option<&mut Arena>,
) -> Array {
    let buf_len = buf_slice.len();
    let mut rv = arena_array_opt(arena.as_deref_mut(), path.len() * pat.size);
    for item in path.iter_mut() {
        if lua {
            if item.has_lua == K_NONE {
                let size = format_to_buf!(&mut *buf_slice, c"%s/lua/", &item.path) as usize;
                item.has_lua = TriState::from(
                    size < buf_len && os_isdir(crate::memory::bytes_as_cstr(buf_slice)),
                );
            }
            if item.has_lua == K_FALSE {
                continue;
            }
        }

        for j in 0..pat.size {
            let pat_item = pat[j];
            if pat_item.obj_type == ObjectType::ObjectTypeString {
                let size = format_to_buf!(
                    &mut *buf_slice,
                    c"%s/%s",
                    &item.path,
                    pat_item.data.string(),
                ) as usize;
                if size < buf_len && os_file_is_readable(crate::memory::bytes_as_cstr(buf_slice)) {
                    to_slash(buf_slice);
                    let astr = arena_string_opt(
                        arena.as_deref_mut(),
                        cstr_as_string(Some(crate::memory::bytes_as_cstr(buf_slice))),
                    );
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

/// Find "name" in "path".
pub fn do_in_path_and_pp(
    path: &std::ffi::CStr,
    name: Option<&std::ffi::CStr>,
    flags: i32,
    callback: &mut RuntimeCallback<'_>,
) -> bool {
    let mut done = false;

    if (flags & DIP_NORTP) == 0 {
        let name_arg = name.filter(|s| !s.to_bytes().is_empty());
        done |= do_in_path(path, c"", name_arg, flags, callback);
    }

    let p_pp_owned = opt_vars().p_pp.to_owned();
    let p_pp = p_pp_owned.as_c_str();
    if (!done || (flags & DIP_ALL) != 0) && (flags & DIP_START) != 0 {
        let mut prefix = if (flags & DIP_AFTER) != 0 {
            c"pack/*/start/*/after/"
        } else {
            c"pack/*/start/*/"
        };
        done |= do_in_path(p_pp, prefix, name, flags & !DIP_AFTER, callback);

        if !done || (flags & DIP_ALL) != 0 {
            prefix = if (flags & DIP_AFTER) != 0 {
                c"start/*/after/"
            } else {
                c"start/*/"
            };
            done |= do_in_path(p_pp, prefix, name, flags & !DIP_AFTER, callback);
        }
    }

    if (!done || (flags & DIP_ALL) != 0) && (flags & DIP_OPT) != 0 {
        done |= do_in_path(p_pp, c"pack/*/opt/*/", name, flags, callback);

        if !done || (flags & DIP_ALL) != 0 {
            done |= do_in_path(p_pp, c"opt/*/", name, flags, callback);
        }
    }

    done
}

fn push_path(
    search_path: &mut RuntimeSearchPath,
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    entry: &std::ffi::CStr,
    after: bool,
    pos_in_rtp: usize,
) -> bool {
    if rtp_used.insert(entry.to_bytes().to_vec()) {
        search_path.push(SearchPathItem {
            path: entry.to_owned(),
            after,
            pack_inserted: false,
            has_lua: K_NONE,
            pos_in_rtp,
        });
        return true;
    }
    false
}

fn expand_rtp_entry(
    search_path: &mut RuntimeSearchPath,
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    entry: &std::ffi::CStr,
    after: bool,
    pos_in_rtp: usize,
) {
    if rtp_used.contains(entry.to_bytes()) {
        return;
    }

    if entry.to_bytes().is_empty() {
        push_path(search_path, rtp_used, entry, after, pos_in_rtp);
    }

    let mut files: Vec<std::ffi::CString> = Vec::new();
    if gen_expand_wildcards_vec(&[entry], &mut files, EW_DIR | EW_NOBREAK) {
        for file in &files {
            push_path(search_path, rtp_used, file, after, pos_in_rtp);
        }
    }
}

fn expand_pack_entry(
    search_path: &mut RuntimeSearchPath,
    rtp_used: &mut std::collections::HashSet<Vec<u8>>,
    after_path: &mut Vec<std::ffi::CString>,
    pack_entry_bytes: &[u8],
    pos_in_rtp: usize,
) {
    let pack_entry_len = pack_entry_bytes.len();
    let start_pats = [c"/pack/*/start/*", c"/start/*"];
    for pat in &start_pats {
        let pat_len = pat.to_bytes().len();
        if pack_entry_len + pat_len + 1 > MAXPATHL {
            continue;
        }
        crate::memory::copy_bytes_to_c_buf(&mut rt_st().pack_entry_buf, pack_entry_bytes);
        crate::memory::copy_bytes_to_c_buf(
            &mut rt_st().pack_entry_buf[pack_entry_len..],
            pat.to_bytes(),
        );
        let entry_cstr = crate::memory::bytes_as_cstr(&rt_st().borrow_mut().pack_entry_buf);
        expand_rtp_entry(search_path, rtp_used, entry_cstr, false, pos_in_rtp);
        let mut after_bytes = Vec::from(entry_cstr.to_bytes());
        after_bytes.extend_from_slice(b"/after");
        after_path.push(bytes_to_cstring(&after_bytes));
    }
}

fn path_is_after(buf_slice: &[u8]) -> bool {
    let buflen = buf_slice.len();
    buflen >= 5
        && (buflen < 6 || vim_ispathsep(i32::from(buf_slice[buflen - 6])))
        && &buf_slice[buflen - 5..] == b"after"
}

fn runtime_search_path_build() -> RuntimeSearchPath {
    let mut pack_entries: Vec<NvimString> = Vec::new();
    let mut pack_used: std::collections::HashMap<Vec<u8>, i32> = std::collections::HashMap::new();
    let mut rtp_used: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut search_path = RuntimeSearchPath::new();
    let mut after_path: Vec<std::ffi::CString> = Vec::new();

    let p_pp = opt_vars().p_pp.to_owned();
    let mut entry = p_pp.as_bytes();
    while !entry.is_empty() {
        let cur_entry_slice = entry;
        let buflen = copy_option_part(&mut entry, &mut rt_st().build_rtp_buf[..MAXPATHL], c",");

        let the_entry = NvimString::from_bytes(&cur_entry_slice[..buflen]);

        pack_entries.push(the_entry);
        pack_used.insert(cur_entry_slice[..buflen].to_vec(), 0);
    }

    let p_rtp = opt_vars().p_rtp.to_owned();
    let mut rtp_entry = p_rtp.as_bytes();
    while !rtp_entry.is_empty() {
        let cur_entry = rtp_entry;
        let buflen = copy_option_part(&mut rtp_entry, &mut rt_st().build_rtp_buf[..MAXPATHL], c",");
        let buf_slice = &rt_st().build_rtp_buf[..buflen];

        if path_is_after(buf_slice) {
            rtp_entry = cur_entry;
            break;
        }

        let pos_in_rtp = p_rtp.as_bytes().len() - cur_entry.len();

        expand_rtp_entry(
            &mut search_path,
            &mut rtp_used,
            crate::memory::bytes_as_cstr(&rt_st().build_rtp_buf),
            false,
            pos_in_rtp,
        );

        let buf_slice = &rt_st().build_rtp_buf[..buflen];
        if let Some(h) = pack_used.get_mut(buf_slice) {
            *h += 1;
            expand_pack_entry(
                &mut search_path,
                &mut rtp_used,
                &mut after_path,
                buf_slice,
                pos_in_rtp,
            );
        }
    }

    let sentinel_pos_in_rtp = (p_rtp.as_bytes().len() - rtp_entry.len()).saturating_sub(1);

    for item in pack_entries {
        let item_bytes = item.as_bytes();
        let h = pack_used.get(item_bytes).copied().unwrap_or(0);
        if h == 0 {
            expand_pack_entry(
                &mut search_path,
                &mut rtp_used,
                &mut after_path,
                item_bytes,
                sentinel_pos_in_rtp,
            );
        }
    }

    for ap in after_path {
        expand_rtp_entry(
            &mut search_path,
            &mut rtp_used,
            &ap,
            true,
            sentinel_pos_in_rtp,
        );
    }

    while !rtp_entry.is_empty() {
        let cur_entry = rtp_entry;
        let buflen = copy_option_part(&mut rtp_entry, &mut rt_st().build_rtp_buf[..MAXPATHL], c",");
        let pos_in_rtp = p_rtp.as_bytes().len() - cur_entry.len();
        expand_rtp_entry(
            &mut search_path,
            &mut rtp_used,
            crate::memory::bytes_as_cstr(&rt_st().build_rtp_buf),
            path_is_after(&rt_st().build_rtp_buf[..buflen]),
            pos_in_rtp,
        );
    }

    search_path
}

pub fn did_set_runtimepackpath(_args: &mut Optset) -> Option<&'static CStr> {
    rt_st().runtime_search_path_valid = false;
    None
}

pub fn runtime_search_path_validate() {
    if !nlua_is_deferred_safe() {
        return;
    }
    if !rt_st().runtime_search_path_valid {
        let new_path = runtime_search_path_build();
        let old_path = std::mem::replace(&mut rt_st().runtime_search_path, new_path);
        if rt_st().runtime_search_path_refcount == 0 {
            msg_ext_ui_flush();
            drop(old_path);
        } else {
            rt_st().runtime_search_path_old.push(old_path);
        }
        rt_st().runtime_search_path_valid = true;
        update_runtime_search_path_thread(true);
    }
}

pub fn update_runtime_search_path_thread(force: bool) {
    if !force && !(rt_st().runtime_search_path_valid && !rt_st().runtime_search_path_valid_thread) {
        return;
    }

    let _guard = RUNTIME_SEARCH_PATH_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    rt_st().runtime_search_path_thread = copy_runtime_search_path(&rt_st().runtime_search_path);
    drop(_guard);
    rt_st().runtime_search_path_valid_thread = true;
}

/// Just like do_in_path_and_pp(), using 'runtimepath' for "path".
pub fn do_in_runtimepath(
    name: &std::ffi::CStr,
    mut flags: i32,
    callback: &mut RuntimeCallback<'_>,
) -> bool {
    let mut success = false;
    if (flags & DIP_NORTP) == 0 {
        let name_arg = (!name.to_bytes().is_empty()).then_some(name);
        success |= do_in_cached_path(name_arg, flags, callback);
        flags = (flags & !DIP_START) | DIP_NORTP;
    }
    if (flags & (DIP_START | DIP_OPT)) != 0 && (!success || (flags & DIP_ALL) != 0) {
        success |= do_in_path_and_pp(opt_vars().p_rtp.as_cstr(), Some(name), flags, callback);
    }
    success
}

/// Source the file "name" from all directories in 'runtimepath'.
pub fn source_runtime(name: &std::ffi::CStr, flags: i32) -> bool {
    do_in_runtimepath(
        name,
        flags,
        &mut RuntimeCallback::Source {
            skip_if_loaded: false,
        },
    )
}

/// Just like source_runtime(), but only source vim and lua files.
pub fn source_runtime_vim_lua(name: &std::ffi::CStr, flags: i32) -> bool {
    do_in_runtimepath(name, flags, &mut RuntimeCallback::SourceVimLua)
}

/// Just like source_runtime(), but use "path" instead of 'runtimepath' and only source .vim and .lua files.
pub fn source_in_path_vim_lua(path: &std::ffi::CStr, name: &std::ffi::CStr, flags: i32) -> bool {
    do_in_path_and_pp(path, Some(name), flags, &mut RuntimeCallback::SourceVimLua)
}

fn gen_expand_wildcards_and_cb(
    pat: &std::ffi::CStr,
    flags: i32,
    all: bool,
    callback: &mut RuntimeCallback<'_>,
) -> bool {
    let mut files: Vec<std::ffi::CString> = Vec::new();

    if !gen_expand_wildcards_vec(&[pat], &mut files, flags) {
        return false;
    }

    callback.invoke(&files, all);

    true
}

fn add_pack_dir_to_rtp(fname_cstr: &std::ffi::CStr, is_pack: bool) -> bool {
    let fname = fname_cstr.as_ptr().cast_mut();
    let mut retval = false;

    let past_head_cstr = get_past_head(fname_cstr);
    let past_head = past_head_cstr.as_ptr();
    let mut p1 = past_head;
    let mut p2 = p1;
    let mut p3 = p1;
    let mut p4 = p1;
    let rem_bytes = past_head_cstr.to_bytes();
    let mut idx = 0usize;
    while idx < rem_bytes.len() {
        if vim_ispathsep_nocolon(rem_bytes[idx] as i32) {
            p4 = p3;
            p3 = p2;
            p2 = p1;
            p1 = past_head.wrapping_add(idx);
        }
        idx += utfc_ptr2len(&rem_bytes[idx..]) as usize;
    }

    p4 = p4.wrapping_add(1);
    let prefix_len = (p4 as usize) - (fname as usize);
    let prefix_cstr = bytes_to_cstring(&fname_cstr.to_bytes()[..prefix_len]);
    let ffname_cstr = fix_fname(Some(&prefix_cstr));

    let Some(ref ffname_owned) = ffname_cstr else {
        return false;
    };

    let mut afterdir_cstr: Option<std::ffi::CString> = None;

    'block: {
        let mut fname_len = ffname_owned.as_bytes().len();
        let mut buf_data = [0u8; MAXPATHL];

        let mut insp_off: Option<usize> = None;
        let mut after_insp_off: Option<usize> = None;
        let mut p_rtp_len: usize = 0;
        let p_rtp = opt_vars().p_rtp.to_owned();
        let rtp_bytes = p_rtp.as_bytes();
        let mut entry_off = 0usize;
        while entry_off < rtp_bytes.len() {
            let cur_entry_off = entry_off;
            let mut entry_bytes = &rtp_bytes[entry_off..];
            let old_len = entry_bytes.len();
            let buf_size = copy_option_part(&mut entry_bytes, &mut buf_data, c",");
            entry_off += old_len - entry_bytes.len();

            p_rtp_len += if rtp_bytes.get(cur_entry_off + buf_size) == Some(&b',') {
                buf_size + 1
            } else {
                buf_size
            };

            let is_after = buf_data[..buf_size]
                .windows(5)
                .position(|w| w == b"after")
                .is_some_and(|i| {
                    i > 0
                        && vim_ispathsep(buf_data[i - 1] as i32)
                        && (vim_ispathsep(buf_data[i + 5] as i32)
                            || buf_data[i + 5] == 0
                            || buf_data[i + 5] == b',')
                });

            if is_after {
                if insp_off.is_none() {
                    insp_off = Some(cur_entry_off);
                }
                after_insp_off = Some(cur_entry_off);
                break;
            }

            if insp_off.is_none() {
                add_pathsep(&mut buf_data[..buf_size + 3]);
                let Some(rtp_ffname) = fix_fname(Some(crate::memory::bytes_as_cstr(&buf_data)))
                else {
                    break 'block;
                };
                if path_cmp(
                    opt_vars().p_fic,
                    &rtp_ffname,
                    ffname_owned.as_c_str(),
                    fname_len,
                ) == 0
                {
                    insp_off = Some(entry_off);
                }
            }
        }

        p_rtp_len += rtp_bytes[p_rtp_len..].len();
        let insp_idx = insp_off.unwrap_or(p_rtp_len);

        fname_len = fname_cstr.to_bytes().len();
        let ad = crate::path::concat_fnames_cstr(fname_cstr, c"after", true);
        let mut afterdir = NvimString::from_cstr(&ad);
        let afterdir_has_entries = if is_pack {
            pack_has_entries(&ad)
        } else {
            os_isdir(afterdir.as_cstr())
        };
        afterdir_cstr = Some(ad);
        if !afterdir_has_entries {
            afterdir.size = 0;
        }

        let new_rtp_capacity = p_rtp_len + fname_len + afterdir.size + 3;
        let mut new_rtp_vec = Vec::with_capacity(new_rtp_capacity);
        let p_rtp_slice = &rtp_bytes[..p_rtp_len];
        let fname_slice = fname_cstr.to_bytes();

        let mut keep = insp_idx;
        let mut first_pos = keep;
        new_rtp_vec.extend_from_slice(&p_rtp_slice[..keep]);
        let insp_empty = insp_idx >= rtp_bytes.len();
        if insp_empty {
            new_rtp_vec.push(b',');
            first_pos += 1;
        }
        new_rtp_vec.extend_from_slice(fname_slice);
        if !insp_empty {
            new_rtp_vec.push(b',');
        }

        let mut after_pos: usize = 0;

        if afterdir.size > 0
            && let Some(keep_after) = after_insp_off
        {
            new_rtp_vec.extend_from_slice(&p_rtp_slice[keep..keep_after]);
            new_rtp_vec.extend_from_slice(afterdir.as_bytes());
            new_rtp_vec.push(b',');
            keep = keep_after;
            after_pos = keep_after;
        }

        if keep < p_rtp_len {
            new_rtp_vec.extend_from_slice(&p_rtp_slice[keep..p_rtp_len]);
        }

        if afterdir.size > 0 && after_insp_off.is_none() {
            new_rtp_vec.push(b',');
            after_pos = new_rtp_vec.len();
            new_rtp_vec.extend_from_slice(afterdir.as_bytes());
        }
        let new_rtp_size = new_rtp_vec.len();
        new_rtp_vec.push(0);
        let new_rtp = NvimString::from_bytes_len(&new_rtp_vec, new_rtp_size);

        let was_valid = rt_st().runtime_search_path_valid;
        set_option_value_give_err(K_OPT_RUNTIMEPATH, string_obj(new_rtp), 0);

        debug_assert!(!rt_st().runtime_search_path_valid);
        if was_valid && !is_pack && rt_st().runtime_search_path_refcount == 0 {
            rt_st().runtime_search_path_valid = true;
            rt_st().runtime_search_path_valid_thread = false;
            let rsp = &mut rt_st().runtime_search_path;

            let idx1 = rsp
                .iter()
                .position(|item| item.pos_in_rtp >= first_pos)
                .unwrap_or(rsp.len());
            for item in &mut rsp[idx1..] {
                item.pos_in_rtp += fname_len + 1;
            }
            let fname_bytes = &fname_cstr.to_bytes()[..fname_len];
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
                    has_lua: K_NONE,
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
                let after_bytes = afterdir.as_bytes();
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
                        has_lua: K_NONE,
                        pos_in_rtp: target_after_pos,
                    },
                );
            }
        }
        drop(new_rtp_vec);
        retval = true;
    }

    drop(afterdir_cstr);
    drop(ffname_cstr);
    retval
}

fn load_pack_plugin(opt: bool, fname: &std::ffi::CStr) -> bool {
    let ffname_cstr = fix_fname(Some(fname)).unwrap_or_default();
    let len = ffname_cstr.as_bytes().len() + 14;
    let mut pat_vec = vec![0u8; len + 1];

    format_to_buf!(&mut pat_vec[..len], c"%s/plugin/**/*", &ffname_cstr);
    gen_expand_wildcards_and_cb(
        crate::memory::bytes_as_cstr(&pat_vec),
        EW_FILE,
        true,
        &mut RuntimeCallback::SourceVimLua,
    );

    if opt && eval_to_number(c"g:did_load_filetypes", false) > 0 {
        do_cmdline_cmd(c"augroup filetypedetect");
        format_to_buf!(&mut pat_vec[..len], c"%s/ftdetect/*", &ffname_cstr);
        gen_expand_wildcards_and_cb(
            crate::memory::bytes_as_cstr(&pat_vec),
            EW_FILE,
            true,
            &mut RuntimeCallback::SourceVimLua,
        );
        do_cmdline_cmd(c"augroup END");
    }

    true
}

fn add_pack_plugins(opt: bool, fnames: &[std::ffi::CString], all: bool, action: PackAddAction) {
    let mut did_one = false;

    if action != PackAddAction::Load {
        let mut buf = [0u8; MAXPATHL];
        let p_rtp = opt_vars().p_rtp.to_owned();
        for fname_cstr in fnames {
            let mut found = false;
            let mut p = p_rtp.as_bytes();
            while !p.is_empty() {
                copy_option_part(&mut p, &mut buf[..MAXPATHL], c",");
                if path_equal(
                    crate::memory::bytes_as_cstr(&buf),
                    fname_cstr.as_c_str(),
                    K_PATH_CMP_LITERAL,
                ) {
                    found = true;
                    break;
                }
            }
            if !found && !add_pack_dir_to_rtp(fname_cstr, false) {
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
            load_pack_plugin(opt, fname_cstr);
            if !all {
                break;
            }
        }
    }
}

/// Add all packages in the "start" directory to 'runtimepath'.
pub fn add_pack_start_dirs() {
    do_in_path(
        opt_vars().p_pp.as_cstr(),
        c"",
        None,
        DIP_ALL + DIP_DIR,
        &mut RuntimeCallback::AddPackStartDir,
    );
}

fn pack_has_entries(buf: &std::ffi::CStr) -> bool {
    let mut files: Vec<std::ffi::CString> = Vec::new();
    gen_expand_wildcards_vec(&[buf], &mut files, EW_DIR) && !files.is_empty()
}

fn add_pack_start_dir(fnames: &[std::ffi::CString], all: bool) -> bool {
    let start_pats = [c"/start/*", c"/pack/*/start/*"];
    for fname_cstr in fnames {
        let fname_len = fname_cstr.as_bytes().len();
        for pat in &start_pats {
            if fname_len + pat.to_bytes().len() + 1 > MAXPATHL {
                continue;
            }
            crate::memory::copy_bytes_to_c_buf(&mut rt_st().pack_start_buf, fname_cstr.as_bytes());
            crate::memory::append_bytes_to_c_buf(&mut rt_st().pack_start_buf, pat.to_bytes());
            let buf_cstr = crate::memory::bytes_as_cstr(&rt_st().borrow_mut().pack_start_buf);
            if pack_has_entries(buf_cstr) {
                add_pack_dir_to_rtp(buf_cstr, true);
            }
        }

        if !all {
            break;
        }
    }

    fnames.len() > 1
}

/// Load plugins from all packages in the "start" directory.
pub fn load_start_packages() {
    g_st().did_source_packages = true;
    let p_pp = opt_vars().p_pp.to_owned();
    do_in_path(
        &p_pp,
        c"",
        Some(c"pack/*/start/*"),
        DIP_ALL + DIP_DIR,
        &mut RuntimeCallback::AddPackPlugins {
            opt: false,
            action: PackAddAction::Load,
        },
    );
    do_in_path(
        &p_pp,
        c"",
        Some(c"start/*"),
        DIP_ALL + DIP_DIR,
        &mut RuntimeCallback::AddPackPlugins {
            opt: false,
            action: PackAddAction::Load,
        },
    );

    update_runtime_search_path_thread(false);
}

/// ":packloadall"
pub fn ex_packloadall(eap: &mut Exarg) {
    if !g_st().did_source_packages || eap.forceit {
        add_pack_start_dirs();
        load_start_packages();
    }
}

/// Read all the plugin files at startup.
pub fn load_plugins() {
    if opt_vars().p_lpl {
        let mut rtp_copy_owned: Option<std::ffi::CString> = None;
        let plugin_pattern = c"plugin/**/*";

        if !g_st().did_source_packages {
            let copy = opt_vars().p_rtp.to_owned();
            rtp_copy_owned = Some(copy);
            add_pack_start_dirs();
        }

        let rtp_cstr: &CStr = rtp_copy_owned
            .as_deref()
            .unwrap_or_else(|| opt_vars().borrow().p_rtp.as_cstr());
        source_in_path_vim_lua(rtp_cstr, plugin_pattern, DIP_ALL | DIP_NOAFTER);
        if has_time_fd() {
            time_msg(c"loading rtp plugins", None);
        }

        if !g_st().did_source_packages {
            drop(rtp_copy_owned);
            load_start_packages();
        }
        if has_time_fd() {
            time_msg(c"loading packages", None);
        }

        source_runtime_vim_lua(plugin_pattern, DIP_ALL | DIP_AFTER);
        if has_time_fd() {
            time_msg(c"loading after plugins", None);
        }
    }
}

/// ":packadd[!] {name}"
pub fn ex_packadd(eap: &mut Exarg) {
    let mut res = true;
    let len = 11 + eap.arg.len() + 5;
    let mut pat_vec = vec![0u8; len];
    let action = if eap.forceit {
        PackAddAction::AddDir
    } else {
        PackAddAction::Both
    };

    if !g_st().did_source_packages {
        format_to_buf!(&mut pat_vec[..len], c"pack/*/%s/%s", c"start", eap.arg);
        res = do_in_path(
            opt_vars().p_pp.as_cstr(),
            c"",
            Some(crate::memory::bytes_as_cstr(&pat_vec)),
            DIP_ALL + DIP_DIR,
            &mut RuntimeCallback::AddPackPlugins { opt: false, action },
        );
    }

    format_to_buf!(&mut pat_vec[..len], c"pack/*/%s/%s", c"opt", eap.arg);
    do_in_path(
        opt_vars().p_pp.as_cstr(),
        c"",
        Some(crate::memory::bytes_as_cstr(&pat_vec)),
        DIP_ALL + DIP_DIR + if !res { DIP_ERR } else { 0 },
        &mut RuntimeCallback::AddPackPlugins { opt: true, action },
    );

    update_runtime_search_path_thread(false);
}

fn expand_rtdir_int(
    pat_cstr: &std::ffi::CStr,
    pat_len: usize,
    flags: i32,
    keep_ext: bool,
    gap: &mut Vec<std::ffi::CString>,
    dirnames: &[&CStr],
) {
    for &dir in dirnames {
        let buf_len = dir.to_bytes().len() + pat_len + 64;
        let mut buf_vec = vec![0u8; buf_len];
        let mut glob_flags: i32 = 0;
        let mut expand_dirs = false;
        let dir_str = if !dir.is_empty() { dir } else { c"" };
        let slash_str = if !dir.is_empty() { c"/" } else { c"" };
        format_to_buf!(
            &mut buf_vec,
            c"%s%s%s%s",
            dir_str,
            slash_str,
            pat_cstr,
            c"*.{vim,lua}",
        );

        loop {
            if (flags & DIP_NORTP) == 0 {
                globpath(
                    opt_vars().p_rtp.as_cstr(),
                    crate::memory::bytes_as_cstr(&buf_vec),
                    gap,
                    glob_flags,
                    expand_dirs,
                );
            }

            let ext_str = if expand_dirs { c"*" } else { c"*.{vim,lua}" };

            if (flags & DIP_START) != 0 {
                format_to_buf!(
                    &mut buf_vec,
                    c"pack/*/start/*/%s%s%s%s",
                    dir_str,
                    slash_str,
                    pat_cstr,
                    ext_str,
                );
                globpath(
                    opt_vars().p_pp.as_cstr(),
                    crate::memory::bytes_as_cstr(&buf_vec),
                    gap,
                    glob_flags,
                    expand_dirs,
                );

                format_to_buf!(
                    &mut buf_vec,
                    c"start/*/%s%s%s%s",
                    dir_str,
                    slash_str,
                    pat_cstr,
                    ext_str,
                );
                globpath(
                    opt_vars().p_pp.as_cstr(),
                    crate::memory::bytes_as_cstr(&buf_vec),
                    gap,
                    glob_flags,
                    expand_dirs,
                );
            }

            if (flags & DIP_OPT) != 0 {
                format_to_buf!(
                    &mut buf_vec,
                    c"pack/*/opt/*/%s%s%s%s",
                    dir_str,
                    slash_str,
                    pat_cstr,
                    ext_str,
                );
                globpath(
                    opt_vars().p_pp.as_cstr(),
                    crate::memory::bytes_as_cstr(&buf_vec),
                    gap,
                    glob_flags,
                    expand_dirs,
                );

                format_to_buf!(
                    &mut buf_vec,
                    c"opt/*/%s%s%s%s",
                    dir_str,
                    slash_str,
                    pat_cstr,
                    ext_str,
                );
                globpath(
                    opt_vars().p_pp.as_cstr(),
                    crate::memory::bytes_as_cstr(&buf_vec),
                    gap,
                    glob_flags,
                    expand_dirs,
                );
            }

            if dir.is_empty() && !expand_dirs {
                format_to_buf!(&mut buf_vec, c"%s*", pat_cstr);
                glob_flags = WILD_ADD_SLASH;
                expand_dirs = true;
                continue;
            }
            break;
        }
    }

    let mut pat_pathsep_cnt: i32 = 0;
    let pat_bytes = pat_cstr.to_bytes();
    for &b in &pat_bytes[..pat_len.min(pat_bytes.len())] {
        if vim_ispathsep(i32::from(b)) {
            pat_pathsep_cnt += 1;
        }
    }

    for slot in gap.iter_mut() {
        let bytes = slot.to_bytes_with_nul();

        let mut end_idx = bytes.len() - 1;
        if end_idx > 4 && !keep_ext {
            let tail4 = &bytes[(end_idx - 4)..end_idx];
            if tail4.eq_ignore_ascii_case(b".vim") || tail4.eq_ignore_ascii_case(b".lua") {
                end_idx -= 4;
            }
        }

        let mut match_pathsep_cnt = if end_idx > 0 && bytes[end_idx - 1] == b'/' {
            -1
        } else {
            0
        };
        let mut s_idx = end_idx;
        while s_idx > 0 {
            if vim_ispathsep(bytes[s_idx] as i32) {
                match_pathsep_cnt += 1;
                if match_pathsep_cnt > pat_pathsep_cnt {
                    break;
                }
            }
            s_idx = s_idx - 1 - crate::mbyte::utf_head_off_slice(bytes, s_idx - 1) as usize;
        }
        let start_idx = s_idx + 1;
        *slot = crate::memory::bytes_to_cstring(&bytes[start_idx..end_idx]);
    }

    if gap.is_empty() {
        return;
    }

    crate::path::remove_duplicate_strings(gap);
}

/// Expand color scheme, compiler or filetype names.
pub fn expand_rtdir(
    pat: &std::ffi::CStr,
    flags: i32,
    file: &mut Vec<std::ffi::CString>,
    dirnames: &[&CStr],
) -> bool {
    file.clear();

    expand_rtdir_int(pat, pat.to_bytes().len(), flags, false, file, dirnames);

    !file.is_empty()
}

/// Handle command line completion for :runtime command.
pub fn expand_runtime_cmd(pat: &std::ffi::CStr, matches: &mut Vec<std::ffi::CString>) -> bool {
    matches.clear();

    let pat_len = pat.to_bytes().len();
    expand_rtdir_int(
        pat,
        pat_len,
        rt_st().runtime_expand_flags,
        true,
        matches,
        &[c""],
    );

    if rt_st().runtime_expand_flags == 0 {
        let where_values = [c"START", c"OPT", c"PACK", c"ALL"];
        let pat_slice = pat.to_bytes();
        for wv in &where_values {
            if wv.to_bytes().starts_with(pat_slice) {
                matches.push((*wv).to_owned());
            }
        }
    }

    !matches.is_empty()
}

/// Expand loadplugin names: 'packpath'/pack/*/opt/{pat}
#[allow(non_snake_case)]
pub fn ExpandPackAddDir(pat_cstr: &std::ffi::CStr, file: &mut Vec<std::ffi::CString>) -> bool {
    file.clear();
    let pat_len = pat_cstr.to_bytes().len();

    let buflen = pat_len + 26;
    let mut s_vec = vec![0u8; buflen];
    format_to_buf!(&mut s_vec, c"pack/*/opt/%s*", pat_cstr);
    globpath(
        opt_vars().p_pp.as_cstr(),
        crate::memory::bytes_as_cstr(&s_vec),
        file,
        0,
        true,
    );
    format_to_buf!(&mut s_vec, c"opt/%s*", pat_cstr);
    globpath(
        opt_vars().p_pp.as_cstr(),
        crate::memory::bytes_as_cstr(&s_vec),
        file,
        0,
        true,
    );
    drop(s_vec);

    for slot in file.iter_mut() {
        let tail = path_tail(slot);
        *slot = (*tail).to_owned();
    }

    if file.is_empty() {
        return false;
    }

    crate::path::remove_duplicate_strings(file);
    true
}

fn strcpy_comma_escaped(dest: &mut Vec<u8>, src: &[u8]) {
    for &b in src {
        if b == b',' {
            dest.push(b'\\');
        }
        dest.push(b);
    }
}

fn compute_double_env_sep_len(
    val_opt: Option<&std::ffi::CStr>,
    common_suf_len: usize,
    single_suf_len: usize,
) -> usize {
    let val_bytes = val_opt.map_or(&[][..], CStr::to_bytes);
    if val_bytes.is_empty() {
        return 0;
    }
    let mut ret: usize = 0;
    let mut iter = None;
    loop {
        let mut dir: &[u8] = &[];
        iter = vim_env_iter(ENV_SEPCHAR, val_bytes, iter, &mut dir);
        if !dir.is_empty() {
            let has_trailing_sep = after_pathsep(dir) != 0;
            ret += (dir.len()
                + dir.iter().filter(|&&b| b == b',').count()
                + common_suf_len
                + usize::from(!has_trailing_sep))
                * 2
                + single_suf_len;
        }
        if iter.is_none() {
            break;
        }
    }
    ret
}

fn add_env_sep_dirs(
    dest: &mut Vec<u8>,
    val_opt: Option<&std::ffi::CStr>,
    suf1: Option<&std::ffi::CStr>,
    len1: usize,
    suf2: Option<&std::ffi::CStr>,
    len2: usize,
    forward: bool,
) {
    let val_bytes = val_opt.map_or(&[][..], CStr::to_bytes);
    if val_bytes.is_empty() {
        return;
    }
    let mut iter = None;
    let appname_bytes = get_appname(false).to_bytes();
    loop {
        let mut dir: &[u8] = &[];
        iter = if forward {
            vim_env_iter(ENV_SEPCHAR, val_bytes, iter, &mut dir)
        } else {
            vim_env_iter_rev(ENV_SEPCHAR, val_bytes, iter, &mut dir)
        };
        if !dir.is_empty() {
            strcpy_comma_escaped(dest, dir);
            if !dest.last().is_some_and(|&b| vim_ispathsep(b as i32)) {
                dest.push(PATHSEP);
            }
            dest.extend_from_slice(appname_bytes);
            if let Some(s1) = suf1 {
                dest.push(PATHSEP);
                dest.extend_from_slice(&s1.to_bytes()[..len1]);
                if let Some(s2) = suf2 {
                    dest.push(PATHSEP);
                    dest.extend_from_slice(&s2.to_bytes()[..len2]);
                }
            }
            dest.push(b',');
        }
        if iter.is_none() {
            break;
        }
    }
}

fn add_dir(
    dest: &mut Vec<u8>,
    dir_opt: Option<&std::ffi::CStr>,
    dir_len: usize,
    type_: XDGVarType,
    suf1: Option<&std::ffi::CStr>,
    len1: usize,
    suf2: Option<&std::ffi::CStr>,
    len2: usize,
) {
    let Some(dir) = dir_opt else {
        return;
    };
    if dir_len == 0 {
        return;
    }
    strcpy_comma_escaped(dest, &dir.to_bytes()[..dir_len]);
    let append_nvim = matches!(type_, XDGVarType::XDGDataHome | XDGVarType::XDGConfigHome);
    if append_nvim {
        if !dest.last().is_some_and(|&b| vim_ispathsep(b as i32)) {
            dest.push(PATHSEP);
        }
        let appname = get_appname(false);
        let appname_bytes = appname.to_bytes();
        debug_assert!(appname_bytes.len() < IOSIZE - 6);
        dest.extend_from_slice(appname_bytes);
        #[cfg(windows)]
        if matches!(type_, XDGVarType::XDGDataHome | XDGVarType::XDGStateHome) {
            dest.extend_from_slice(b"-data");
        }
        if let Some(s1) = suf1 {
            dest.push(PATHSEP);
            dest.extend_from_slice(&s1.to_bytes()[..len1]);
            if let Some(s2) = suf2 {
                dest.push(PATHSEP);
                dest.extend_from_slice(&s2.to_bytes()[..len2]);
            }
        }
    }
    dest.push(b',');
}

pub fn get_lib_dir() -> Option<std::ffi::CString> {
    let dld = default_lib_dir();
    if !dld.to_bytes().is_empty() && os_isdir(dld) {
        return Some(dld.to_owned());
    }

    let mut exe_name = [0u8; MAXPATHL];
    vim_get_prefix_from_exepath(&mut exe_name);
    if append_path(&mut exe_name, c"lib/nvim") {
        return Some(crate::memory::cstring_until_nul(&exe_name));
    }
    None
}

/// Determine the startup value for &runtimepath.
pub fn runtimepath_default(clean_arg: bool) -> Option<std::ffi::CString> {
    const SITE_SIZE: usize = 4;
    const AFTER_SIZE: usize = 5;

    let mut rtp_size: usize = 0;
    let data_home_cstr = if clean_arg {
        None
    } else {
        stdpaths_get_xdg_var(XDGVarType::XDGDataHome)
    };
    let config_home_cstr = if clean_arg {
        None
    } else {
        stdpaths_get_xdg_var(XDGVarType::XDGConfigHome)
    };
    let libdir_cstr = get_lib_dir();
    let data_dirs_cstr = stdpaths_get_xdg_var(XDGVarType::XDGDataDirs);
    let config_dirs_cstr = stdpaths_get_xdg_var(XDGVarType::XDGConfigDirs);
    let vimruntime_cstr = vim_getenv(c"VIMRUNTIME");

    let data_dirs = data_dirs_cstr.as_deref();
    let config_dirs = config_dirs_cstr.as_deref();
    let vimruntime = vimruntime_cstr.as_deref();
    let libdir = libdir_cstr.as_deref();

    let mut data_len: usize = 0;
    let mut config_len: usize = 0;
    let mut vimruntime_len: usize = 0;
    let mut libdir_len: usize = 0;
    let appname_len = get_appname(false).to_bytes().len();

    if let Some(ref dh) = data_home_cstr {
        let dh_bytes = dh.as_bytes();
        data_len = dh_bytes.len();
        let nvim_data_size = if cfg!(windows) {
            appname_len + 5
        } else {
            appname_len
        };
        if data_len != 0 {
            rtp_size += (data_len
                + dh_bytes.iter().filter(|&&b| b == b',').count()
                + nvim_data_size
                + 1
                + SITE_SIZE
                + 1
                + usize::from(after_pathsep(dh_bytes) == 0))
                * 2
                + AFTER_SIZE
                + 1;
        }
    }
    if let Some(ref ch) = config_home_cstr {
        let ch_bytes = ch.as_bytes();
        config_len = ch_bytes.len();
        if config_len != 0 {
            rtp_size += (config_len
                + ch_bytes.iter().filter(|&&b| b == b',').count()
                + appname_len
                + 1
                + usize::from(after_pathsep(ch_bytes) == 0))
                * 2
                + AFTER_SIZE
                + 1;
        }
    }
    if let Some(ref vr) = vimruntime_cstr {
        vimruntime_len = vr.as_bytes().len();
        if vimruntime_len != 0 {
            rtp_size += vimruntime_len + vr.as_bytes().iter().filter(|&&b| b == b',').count() + 1;
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
        let mut rtp_vec = Vec::with_capacity(rtp_size);
        add_dir(
            &mut rtp_vec,
            config_home_cstr.as_deref(),
            config_len,
            XDGVarType::XDGConfigHome,
            None,
            0,
            None,
            0,
        );
        add_env_sep_dirs(&mut rtp_vec, config_dirs, None, 0, None, 0, true);
        add_dir(
            &mut rtp_vec,
            data_home_cstr.as_deref(),
            data_len,
            XDGVarType::XDGDataHome,
            Some(c"site"),
            SITE_SIZE,
            None,
            0,
        );
        add_env_sep_dirs(
            &mut rtp_vec,
            data_dirs,
            Some(c"site"),
            SITE_SIZE,
            None,
            0,
            true,
        );
        add_dir(
            &mut rtp_vec,
            vimruntime,
            vimruntime_len,
            XDGVarType::XDGNone,
            None,
            0,
            None,
            0,
        );
        add_dir(
            &mut rtp_vec,
            libdir,
            libdir_len,
            XDGVarType::XDGNone,
            None,
            0,
            None,
            0,
        );
        add_env_sep_dirs(
            &mut rtp_vec,
            data_dirs,
            Some(c"site"),
            SITE_SIZE,
            Some(c"after"),
            AFTER_SIZE,
            false,
        );
        add_dir(
            &mut rtp_vec,
            data_home_cstr.as_deref(),
            data_len,
            XDGVarType::XDGDataHome,
            Some(c"site"),
            SITE_SIZE,
            Some(c"after"),
            AFTER_SIZE,
        );
        add_env_sep_dirs(
            &mut rtp_vec,
            config_dirs,
            Some(c"after"),
            AFTER_SIZE,
            None,
            0,
            false,
        );
        add_dir(
            &mut rtp_vec,
            config_home_cstr.as_deref(),
            config_len,
            XDGVarType::XDGConfigHome,
            Some(c"after"),
            AFTER_SIZE,
            None,
            0,
        );
        debug_assert_eq!(rtp_vec.len(), rtp_size);
        if let Some(last) = rtp_vec.last_mut() {
            *last = 0;
        }
        Some(crate::memory::cstring_until_nul(&rtp_vec))
    } else {
        None
    }
}

fn cmd_source(fname: &std::ffi::CStr, mut eap: Option<&mut Exarg>) {
    if !fname.to_bytes().is_empty() && eap.is_some() && eap.as_deref_mut().unwrap().addr_count > 0 {
        emsg(gettext(E_NORANGE));
        return;
    }

    if eap.is_some() && fname.to_bytes().is_empty() {
        if eap.as_deref_mut().unwrap().forceit {
            emsg(gettext(E_ARGREQ));
        } else {
            cmd_source_buffer(eap.as_deref(), false);
        }
    } else if let Some(eap) = eap
        && eap.forceit
    {
        openscript(
            fname,
            g_st().global_busy != 0
                || g_st().listcmd_busy
                || !eap.nextcmd.is_null()
                || eap.cstack_mut().cs_idx >= 0,
        );
    } else if !do_source(fname, false, DOSO_NONE, None) {
        semsg!(gettext(E_NOTOPEN), fname);
    }
}

/// ":source [{fname}]"
pub fn ex_source(eap: &mut Exarg) {
    let arg = eap.arg.to_cstring();
    cmd_source(&arg, Some(&mut *eap));
}

/// ":options"
pub fn ex_options(_eap: &mut Exarg) {
    let mut buf = Vec::new();
    let mut multi_mods = false;

    add_win_cmd_modifiers(&mut buf, &g_st().cmdmod, &mut multi_mods);
    buf.push(0);

    os_setenv(c"OPTWIN_CMD", crate::memory::bytes_as_cstr(&buf), 1);
    cmd_source(c"$VIMRUNTIME/scripts/optwin.lua", None);
}

pub fn source_breakpoint(sp: &mut SourceCookie) -> &mut Linenr {
    &mut sp.breakpoint
}

pub fn source_dbg_tick(sp: &mut SourceCookie) -> &mut i32 {
    &mut sp.dbg_tick
}

pub fn source_level(sp: &mut SourceCookie) -> i32 {
    sp.level
}

fn fopen_noinh_readbin(filename: &std::ffi::CStr) -> Option<crate::os::fs::CFile> {
    #[cfg(windows)]
    let fd_tmp = os_open(filename, O_RDONLY | O_BINARY | O_NOINHERIT, 0);
    #[cfg(not(windows))]
    let fd_tmp = os_open(filename, O_RDONLY, 0);

    if fd_tmp < 0 {
        return None;
    }

    os_set_cloexec(fd_tmp);

    crate::os::fs::CFile::from_fd(fd_tmp, c"rb")
}

fn concat_continued_line(ga: &mut Vec<u8>, slice: &[u8]) -> bool {
    let slice = crate::charset::skipwhite_slice(slice);
    if slice.starts_with(b"\"\\ ") {
        return true;
    }
    if slice.is_empty() || slice[0] != b'\\' {
        return false;
    }
    if slice.len() > 1 {
        ga.extend_from_slice(&slice[1..]);
    }
    true
}

/// Create a new script item and allocate script-local vars.
pub fn new_script_item(
    name: Option<std::ffi::CString>,
    sid_out: &mut Scid,
) -> &'static mut Scriptitem {
    rt_st().last_current_sid += 1;
    let sid = rt_st().last_current_sid;
    {
        *sid_out = sid;
    }
    while (rt_st().script_items.len() as Scid) < sid {
        rt_st().script_items.push(Scriptitem {
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
    item.sn_name = name;
    item
}

fn do_source_buffer_init(
    sp: &mut SourceCookie,
    eap: Option<&Exarg>,
    ex_lua: bool,
) -> Option<std::ffi::CString> {
    if g_st().curbuf.is_null() {
        return None;
    }

    let fname = if curbuf_mut().b_ffname.is_some() {
        curbuf_mut().ffname_cstr().to_owned()
    } else {
        if ex_lua {
            format_to_buf!(
                &mut g_st().iobuff,
                c":{range}lua buffer=%d",
                curbuf_mut().b_fnum,
            );
        } else {
            format_to_buf!(
                &mut g_st().iobuff,
                c":source buffer=%d",
                curbuf_mut().b_fnum,
            );
        }
        g_st().iobuff_cstr().to_owned()
    };

    sp.buflines.clear();
    for curr_lnum in eap.unwrap().line1..=eap.unwrap().line2 {
        sp.buflines.push(crate::memory::bytes_to_cstring(
            crate::memline::ml_get_slice(curr_lnum),
        ));
    }
    sp.buf_lnum = 0;
    sp.source_from_buf_or_str = true;
    sp.sourcing_lnum = eap.unwrap().line1 - 1;

    Some(fname)
}

fn do_source_str_init(sp: &mut SourceCookie, str_cstr: &std::ffi::CStr) {
    let mut rem = str_cstr.to_bytes();
    sp.buflines.clear();
    while !rem.is_empty() {
        let off = rem.iter().position(|&b| b == b'\n').unwrap_or(rem.len());
        sp.buflines
            .push(std::ffi::CString::new(&rem[..off]).unwrap_or_default());
        rem = &rem[(off + 1).min(rem.len())..];
    }
    sp.buf_lnum = 0;
    sp.source_from_buf_or_str = true;
}

pub fn cmd_source_buffer(eap: Option<&Exarg>, ex_lua: bool) {
    do_source_ext(None, false, DOSO_NONE, None, eap, ex_lua, None);
}

/// Executes lines in `str` as Ex commands.
pub fn do_source_str(str_ptr: &std::ffi::CStr, traceback_name_in: &std::ffi::CStr) -> bool {
    let mut traceback_name = traceback_name_in;
    let slnum = sourcing_lnum();
    let mut sname_buf = [0u8; 256];
    if let Some(sname) = sourcing_name() {
        format_to_buf!(
            &mut sname_buf,
            c"%s called at %s:%d",
            traceback_name,
            sname,
            slnum,
        );
        traceback_name = crate::memory::bytes_as_cstr(&sname_buf);
    }
    do_source_ext(
        Some(traceback_name),
        false,
        DOSO_NONE,
        None,
        None,
        false,
        Some(str_ptr),
    )
}

fn do_source_ext(
    fname_opt: Option<&std::ffi::CStr>,
    check_other: bool,
    is_vimrc: i32,
    mut ret_sid: Option<&mut i32>,
    eap: Option<&Exarg>,
    ex_lua: bool,
    str_opt: Option<&std::ffi::CStr>,
) -> bool {
    let fname_cstr_or_empty = fname_opt.unwrap_or(c"");
    let mut cookie = SourceCookie::new();
    let mut firstline: Option<std::ffi::CString> = None;
    let mut retval = false;
    let save_debug_break_level = g_st().debug_break_level;
    let mut si = crate::globals::GraphPtr::<Scriptitem>::NULL;
    let mut wait_start: Proftime = 0;
    let mut trigger_source_post = false;

    let mut fname_exp_owned: std::ffi::CString = if let Some(fname_cstr) = fname_opt {
        if let Some(str_cstr) = str_opt {
            do_source_str_init(&mut cookie, str_cstr);
            fname_cstr.to_owned()
        } else {
            let p = expand_env_save(fname_cstr);
            let Some(fe) = fix_fname(Some(&p)) else {
                return retval;
            };
            if os_isdir(&fe) {
                smsg!(
                    0,
                    gettext(c"Cannot source a directory: \"%s\""),
                    fname_cstr_or_empty,
                );
                return retval;
            }
            fe
        }
    } else {
        debug_assert!(str_opt.is_none());
        let Some(fe) = do_source_buffer_init(&mut cookie, eap, ex_lua) else {
            return false;
        };
        fe
    };

    'block: {
        let mut sid = if str_opt.is_some() {
            SID_STR
        } else {
            find_script_by_name(&fname_exp_owned)
        };
        if sid > 0
            && let Some(ref mut ret_sid_ref) = ret_sid
        {
            **ret_sid_ref = sid;
            retval = true;
            break 'block;
        }

        if str_opt.is_none() {
            let fname_exp_opt = Some(fname_exp_owned.as_c_str());
            let has_sourcecmd = if fname_opt.is_none() {
                has_autocmd(EVENT_SOURCECMD, fname_exp_opt, None)
            } else {
                has_autocmd_fullname(EVENT_SOURCECMD, fname_exp_opt, None)
            };
            if has_sourcecmd
                && apply_autocmds(
                    EVENT_SOURCECMD,
                    fname_exp_opt,
                    fname_exp_opt,
                    false,
                    curbuf_opt_mut(),
                )
            {
                retval = !aborting();
                if retval {
                    apply_autocmds(
                        EVENT_SOURCEPOST,
                        fname_exp_opt,
                        fname_exp_opt,
                        false,
                        curbuf_opt_mut(),
                    );
                }
                break 'block;
            }

            apply_autocmds(
                EVENT_SOURCEPRE,
                fname_exp_opt,
                fname_exp_opt,
                false,
                curbuf_opt_mut(),
            );
        }

        if !cookie.source_from_buf_or_str {
            cookie.fp = fopen_noinh_readbin(&fname_exp_owned);
        }
        if cookie.fp.is_none() && check_other {
            let tail_ptr = path_tail(fname_exp_owned.as_c_str()).as_ptr();
            let tail_off = (tail_ptr as usize) - (fname_exp_owned.as_ptr() as usize);
            let first_b = fname_exp_owned
                .as_bytes()
                .get(tail_off)
                .copied()
                .unwrap_or(0);
            if first_b == b'.' || first_b == b'_' {
                let rest = &fname_exp_owned.as_bytes()[tail_off + 1..];
                if rest.eq_ignore_ascii_case(b"nvimrc") || rest.eq_ignore_ascii_case(b"exrc") {
                    let mut bytes = fname_exp_owned.into_bytes();
                    bytes[tail_off] = if first_b == b'_' { b'.' } else { b'_' };
                    fname_exp_owned = std::ffi::CString::new(bytes).unwrap();
                    cookie.fp = fopen_noinh_readbin(&fname_exp_owned);
                }
            }
        }

        if cookie.fp.is_none() && !cookie.source_from_buf_or_str {
            if opt_vars().p_verbose > 1 {
                verbose_enter();
                if sourcing_name().is_none() {
                    smsg!(0, gettext(c"could not source \"%s\""), fname_cstr_or_empty);
                } else {
                    smsg!(
                        0,
                        gettext(c"line %lld: could not source \"%s\""),
                        i64::from(sourcing_lnum()),
                        fname_cstr_or_empty,
                    );
                }
                verbose_leave();
            }
            break 'block;
        }

        if opt_vars().p_verbose > 1 {
            verbose_enter();
            if sourcing_name().is_none() {
                smsg!(0, gettext(c"sourcing \"%s\""), fname_cstr_or_empty);
            } else {
                smsg!(
                    0,
                    gettext(c"line %lld: sourcing \"%s\""),
                    i64::from(sourcing_lnum()),
                    fname_cstr_or_empty,
                );
            }
            verbose_leave();
        }
        if is_vimrc == DOSO_VIMRC {
            vimrc_found(Some(fname_exp_owned.as_c_str()), c"MYVIMRC");
        }

        #[cfg(windows)]
        {
            if opt_vars().p_ffs.is_empty() {
                cookie.fileformat = EOL_DOS;
            } else {
                cookie.fileformat = EOL_UNKNOWN;
            }
        }

        cookie.breakpoint = dbg_find_breakpoint(true, &fname_exp_owned, 0);
        cookie.fname = Some(fname_exp_owned.clone());
        cookie.dbg_tick = g_st().debug_tick;
        cookie.level = g_st().ex_nesting_level;

        let mut rel_time: Proftime = 0;
        let mut start_time: Proftime = 0;
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

        rt_st().last_current_sid_seq += 1;
        g_st().current_sctx.sc_seq = rt_st().last_current_sid_seq;

        if sid > 0 {
            si = crate::globals::GraphPtr::<Scriptitem>::of(script_item(sid));
        } else if str_opt.is_none() {
            si = crate::globals::GraphPtr::<Scriptitem>::of(new_script_item(
                Some(fname_exp_owned.clone()),
                &mut sid,
            ));
            si.sn_lua = path_with_extension(fname_exp_owned.as_c_str(), c"lua");
            if let Some(ref mut ret_sid_ref) = ret_sid {
                **ret_sid_ref = sid;
            }
        }
        debug_assert_eq!(!si.is_null(), str_opt.is_none());

        if str_opt.is_none() || !script_is_lua(g_st().current_sctx.sc_sid) {
            g_st().current_sctx.sc_sid = sid;
            g_st().current_sctx.sc_lnum = 0;
        }

        estack_push(
            ETYPE_SCRIPT,
            if !si.is_null() {
                si.name_cstr()
            } else {
                Some(&fname_exp_owned)
            },
            0,
        );

        if l_do_profiling == PROF_YES && !si.is_null() {
            let si_mut = si.borrow_mut();
            let mut forceit = false;
            if !si_mut.sn_prof_on
                && has_profiling(true, si_mut.name_cstr().unwrap_or(c""), Some(&mut forceit))
            {
                profile_init(si_mut);
                si_mut.sn_pr_force = forceit;
            }
            if si_mut.sn_prof_on {
                si_mut.sn_pr_count += 1;
                si_mut.sn_pr_start = profile_start();
                si_mut.sn_pr_children = profile_zero();
            }
        }

        cookie.conv.vc_type = CONV_NONE;

        let mut ts_lua = false;
        if fname_opt.is_none()
            && let Some(eap) = eap
            && !ex_lua
            && !(Some(curbuf_mut().b_p_ft.as_bytes()) == Some(b"lua"))
            && !curbuf_mut()
                .fname_opt()
                .is_some_and(|f| path_with_extension(f, c"lua"))
        {
            let mut args_items = [
                integer_obj(Integer::from(curbuf_mut().b_fnum)),
                integer_obj(Integer::from(eap.line1)),
                integer_obj(Integer::from(eap.line2)),
            ];
            let args = Array {
                items: crate::globals::GraphPtr::new(args_items.as_mut_ptr()),
                size: 3,
                capacity: 3,
            };
            let mut err = Error::new();
            let result = nlua_exec(
                static_cstr_as_string(c"return require('vim._core.util').source_is_lua(...)"),
                None,
                args,
                K_RET_NIL_BOOL,
                None,
                &mut err,
            );
            if err.err_type == ErrorType::ErrorTypeNone
                && result.obj_type == ObjectType::ObjectTypeBoolean
                && result.data.boolean()
            {
                ts_lua = true;
            }
            api_clear_error(&mut err);
        }

        if fname_opt.is_none()
            && (ex_lua
                || ts_lua
                || (Some(curbuf_mut().b_p_ft.as_bytes()) == Some(b"lua"))
                || curbuf_mut()
                    .fname_opt()
                    .is_some_and(|f| path_with_extension(f, c"lua")))
        {
            nlua_exec_lines(&cookie.buflines, &fname_exp_owned);
        } else if !si.is_null() && si.sn_lua {
            nlua_exec_file(&fname_exp_owned);
        } else {
            firstline = getsourceline(0, &mut cookie, 0, true);
            if let Some(ref fl) = firstline
                && fl.to_bytes().starts_with(&[0xef, 0xbb, 0xbf])
            {
                convert_setup(
                    &mut cookie.conv,
                    Some(c"utf-8"),
                    Some(opt_vars().p_enc.as_cstr()),
                );
                let after_bom = &fl.to_bytes()[3..];
                firstline = Some(string_convert(&cookie.conv, after_bom, None).map_or_else(
                    || crate::memory::bytes_to_cstring(after_bom),
                    |v| crate::memory::cstring_until_nul(&v),
                ));
            }
            do_cmdline(
                firstline.as_deref(),
                LineGetter::Source((&mut cookie).into()),
                DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_REPEAT,
            );
        }
        retval = true;

        if l_do_profiling == PROF_YES && !si.is_null() {
            let si_mut = script_item(g_st().current_sctx.sc_sid);
            if si_mut.sn_prof_on {
                si_mut.sn_pr_start = profile_end(si_mut.sn_pr_start);
                si_mut.sn_pr_start = profile_sub_wait(wait_start, si_mut.sn_pr_start);
                si_mut.sn_pr_total = profile_add(si_mut.sn_pr_total, si_mut.sn_pr_start);
                si_mut.sn_pr_self =
                    profile_self(si_mut.sn_pr_self, si_mut.sn_pr_start, si_mut.sn_pr_children);
            }
        }

        if g_st().got_int {
            emsg(gettext(E_INTERR));
        }
        estack_pop();
        if opt_vars().p_verbose > 1 {
            verbose_enter();
            smsg!(0, gettext(c"finished sourcing %s"), fname_cstr_or_empty);
            if let Some(sname) = sourcing_name() {
                smsg!(0, gettext(c"continuing in %s"), sname);
            }
            verbose_leave();
        }

        if l_has_time_fd {
            format_to_buf!(&mut g_st().iobuff, c"sourcing %s", fname_cstr_or_empty);
            time_msg(g_st().iobuff_cstr(), Some(&start_time));
            time_pop(rel_time);
        }

        if !g_st().got_int {
            trigger_source_post = true;
        }

        if save_debug_break_level > g_st().ex_nesting_level
            && g_st().debug_break_level == g_st().ex_nesting_level
        {
            g_st().debug_break_level += 1;
        }

        g_st().current_sctx = save_current_sctx;
        restore_funccal();
        if l_do_profiling == PROF_YES {
            prof_child_exit(&mut wait_start);
        }
        drop(firstline);
        drop(cookie);

        if str_opt.is_none() && trigger_source_post {
            let fname_exp_opt = Some(fname_exp_owned.as_c_str());
            apply_autocmds(
                EVENT_SOURCEPOST,
                fname_exp_opt,
                fname_exp_opt,
                false,
                curbuf_opt_mut(),
            );
        }
    }

    drop(fname_exp_owned);
    retval
}

pub fn do_source(
    fname: &std::ffi::CStr,
    check_other: bool,
    is_vimrc: i32,
    ret_sid: Option<&mut i32>,
) -> bool {
    do_source_ext(
        Some(fname),
        check_other,
        is_vimrc,
        ret_sid,
        None,
        false,
        None,
    )
}

/// Checks if the script with the given script ID is a Lua script.
pub fn script_is_lua(sid: Scid) -> bool {
    if sid == SID_LUA {
        return true;
    }
    if !script_id_valid(sid) {
        return false;
    }
    script_item(sid).sn_lua
}

/// Find an already loaded script "name".
pub fn find_script_by_name(name: &std::ffi::CStr) -> i32 {
    let mut sid = script_items_len();
    while sid > 0 {
        let si = script_item(sid);
        if let Some(ref sn) = si.sn_name
            && path_equal(sn, name, K_PATH_CMP_LITERAL)
        {
            return sid;
        }
        sid -= 1;
    }
    -1
}

/// ":scriptnames"
pub fn ex_scriptnames(eap: &mut Exarg) {
    if eap.addr_count > 0 || !eap.arg.is_empty() {
        if eap.addr_count > 0 && !script_id_valid(eap.line2) {
            emsg(gettext(E_INVARG));
        } else {
            if eap.addr_count > 0 {
                eap.arg =
                    crate::strings::OwnedSlice::from_opt_cstr(script_item(eap.line2).name_cstr());
            } else {
                expand_env(eap.arg.as_cstr(), g_st().name_buff_bytes_mut());
                to_slash(g_st().name_buff_bytes_mut());
                eap.arg = crate::strings::OwnedSlice::from_bytes(g_st().name_buff_bytes_mut());
            }
            do_exedit(eap, None);
        }
        return;
    }

    msg_ext_set_kind(c"list_cmd");
    let mut i = 1;
    while i <= script_items_len() && !g_st().got_int {
        let si = script_item(i);
        if let Some(ref sn) = si.sn_name {
            home_replace(
                None,
                Some(sn.as_c_str()),
                g_st().name_buff_bytes_mut(),
                true,
            );
            format_to_buf!(&mut g_st().iobuff, c"%3d: %s", i, &g_st().name_buff);
            if !message_filtered(crate::memory::bytes_as_cstr(g_st().iobuff_bytes())) {
                if g_st().msg_col > 0 {
                    msg_putchar(b'\n' as i32);
                }
                msg_outtrans(
                    crate::memory::bytes_as_cstr(g_st().iobuff_bytes()),
                    0,
                    false,
                );
                line_breakcheck();
            }
        }
        i += 1;
    }
}

/// Get a script name (`get_scriptname`).
pub fn get_scriptname(script_ctx: Sctx, home_replace: bool) -> std::borrow::Cow<'static, CStr> {
    use std::borrow::Cow;
    {
        let cstr = match script_ctx.sc_sid {
            SID_MODELINE => gettext(c"modeline"),
            SID_CMDARG => gettext(c"--cmd argument"),
            SID_CARG => gettext(c"-c argument"),
            SID_ENV => gettext(c"environment variable"),
            SID_ERROR => gettext(c"error handler"),
            SID_WINLAYOUT => gettext(c"changed window size"),
            SID_LUA => gettext(c"Lua"),
            SID_API_CLIENT => {
                format_to_buf!(
                    &mut g_st().iobuff,
                    gettext(c"API client (channel id %llu)"),
                    script_ctx.sc_chan,
                );
                g_st().iobuff_cstr()
            }
            SID_STR => gettext(c"anonymous :source"),
            _ => {
                if let Some(sname) = script_item(script_ctx.sc_sid).name_cstr() {
                    if home_replace {
                        return Cow::Owned(home_replace_save(None, Some(sname)));
                    } else {
                        return Cow::Borrowed(sname);
                    }
                } else {
                    format_to_buf!(
                        &mut g_st().iobuff,
                        gettext(c"anonymous :source (script id %d)"),
                        script_ctx.sc_sid,
                    );
                    g_st().iobuff_cstr()
                }
            }
        };
        Cow::Borrowed(cstr)
    }
}

pub fn free_scriptnames() {
    profile_reset();
    rt_st().script_items = Vec::new();
}

pub fn free_autoload_scriptnames() {
    rt_st().ga_loaded = Vec::new();
}

pub fn get_sourced_lnum(fgetline: LineGetter) -> Linenr {
    if let Some(sp) = fgetline.as_source_mut() {
        sp.sourcing_lnum
    } else {
        sourcing_lnum()
    }
}

fn get_script_local_funcs(sid: Scid) -> &'static mut TvList {
    let functbl = func_tbl_get();
    let l = tv_list_alloc(functbl.len() as isize);

    for &fp in functbl.values() {
        let fp_ref = crate::globals::GraphPtr::of(fp).borrow();
        if fp_ref.uf_script_ctx.sc_sid == sid {
            let name = if let Some(ref exp) = fp_ref.uf_name_exp {
                exp.as_c_str()
            } else {
                fp_ref.uf_name.as_c_str()
            };
            tv_list_append_string(&mut *l, Some(name), -1);
        }
    }

    l
}

/// "getscriptinfo()" function
pub fn f_getscriptinfo(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, script_items_len() as isize);

    if !tv_check_for_opt_dict_arg(argvars, 0) {
        return;
    }

    let l = rettv.vval.v_list_opt().unwrap();

    let mut regmatch: Regmatch = Regmatch::new();
    regmatch.rm_ic = opt_vars().p_ic;
    let mut filterpat = false;
    let mut sid: Varnumber = -1;

    let mut pat_owned: Option<std::ffi::CString> = None;
    if argvars[0].v_type == VarType::VarDict {
        let arg_dict = argvars[0].vval.v_dict_opt();
        if let Some(sid_di) = crate::eval::typval::tv_dict_find_opt(arg_dict.as_deref(), c"sid", 3)
        {
            let mut error = false;
            sid = tv_get_number_chk(&sid_di.di_tv, Some(&mut error));
            if error {
                return;
            }
            if sid <= 0 {
                semsg!(gettext(E_INVARG_NVAL), c"sid", tv_get_string(&sid_di.di_tv),);
                return;
            }
        } else {
            pat_owned = crate::eval::typval::tv_dict_get_cstring(arg_dict.as_deref(), c"name");
            if let Some(ref pat) = pat_owned {
                regmatch.set_prog(vim_regcomp(pat, RE_MAGIC + RE_STRING));
            }
            if !regmatch.regprog.is_null() {
                filterpat = true;
            }
        }
    }

    let mut i = if sid > 0 { sid } else { 1 };
    while (i == sid || sid <= 0) && i <= Varnumber::from(script_items_len()) {
        let si = script_item(i as Scid);

        if let Some(ref sn) = si.sn_name
            && (!filterpat || vim_regexec(&mut regmatch, sn, 0))
        {
            let d = tv_dict_alloc();
            tv_list_append_dict(l, Some(d));
            tv_dict_add_str(&mut *d, c"name", Some(sn.as_c_str()));
            tv_dict_add_nr(&mut *d, c"sid", i);
            tv_dict_add_nr(&mut *d, c"version", 1);
            tv_dict_add_bool(&mut *d, c"autoload", K_FALSE);

            if sid > 0 {
                let var_dict =
                    tv_dict_copy(None, Some(&mut si.sn_vars.sv_dict), true, get_copy_id());
                tv_dict_add_dict(&mut *d, c"variables", var_dict);
                tv_dict_add_list(
                    &mut *d,
                    c"functions",
                    Some(get_script_local_funcs(sid as Scid)),
                );
            }
        }

        i += 1;
    }

    regmatch.free_prog();
    drop(pat_owned);
}

/// Get one full line from a sourced file.
pub fn getsourceline(
    _c: i32,
    sp: &mut SourceCookie,
    _indent: i32,
    do_concat: bool,
) -> Option<std::ffi::CString> {
    if sp.dbg_tick < g_st().debug_tick && !sp.source_from_buf_or_str {
        let fname_cstr = sp.fname.as_deref().unwrap_or(c"");
        sp.breakpoint = dbg_find_breakpoint(true, fname_cstr, sourcing_lnum());
        sp.dbg_tick = g_st().debug_tick;
    }
    if g_st().do_profiling == PROF_YES {
        script_line_end();
    }
    set_sourcing_lnum(sp.sourcing_lnum + 1);
    let mut line_vec = if sp.finished || (!sp.source_from_buf_or_str && sp.fp.is_none()) {
        None
    } else if let Some(next) = sp.nextline.take() {
        sp.sourcing_lnum += 1;
        Some(next)
    } else {
        get_one_sourceline(sp)
    };
    if line_vec.is_some() && g_st().do_profiling == PROF_YES {
        script_line_start();
    }

    if let Some(ref mut buf) = line_vec
        && do_concat
        && !crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_CONCAT)
    {
        sp.sourcing_lnum -= 1;

        sp.nextline = get_one_sourceline(sp);
        if let Some(ref next) = sp.nextline {
            let p_bytes = crate::charset::skipwhite_slice(crate::memory::bytes_until_nul(next));
            if p_bytes.first() == Some(&b'\\') || p_bytes.starts_with(b"\"\\ ") {
                if buf.last() == Some(&0) {
                    buf.pop();
                }
                buf.reserve(400);
                while let Some(ref next) = sp.nextline
                    && concat_continued_line(buf, crate::memory::bytes_until_nul(next))
                {
                    sp.nextline = get_one_sourceline(sp);
                }
                if buf.last() != Some(&0) {
                    buf.push(0);
                }
            }
        }
    }

    let line = line_vec.map(|b| {
        let len = b.len().saturating_sub(usize::from(b.last() == Some(&0)));
        if sp.conv.vc_type != CONV_NONE
            && let Some(s) = string_convert(&sp.conv, crate::memory::bytes_until_nul(&b), None)
        {
            crate::memory::cstring_until_nul(&s)
        } else {
            bytes_to_cstring(&b[..len])
        }
    });

    if !sp.source_from_buf_or_str && sp.breakpoint != 0 && sp.breakpoint <= sourcing_lnum() {
        let fname_cstr = sp.fname.as_deref();
        dbg_breakpoint(fname_cstr, sourcing_lnum());
        sp.breakpoint = dbg_find_breakpoint(true, fname_cstr.unwrap_or(c""), sourcing_lnum());
        sp.dbg_tick = g_st().debug_tick;
    }

    line
}

fn get_one_sourceline(sp: &mut SourceCookie) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(250);
    let mut have_read = false;

    sp.sourcing_lnum += 1;
    loop {
        #[cfg(windows)]
        let mut len: usize;
        #[cfg(not(windows))]
        let len: usize;
        let mut filled_chunk = false;
        if sp.source_from_buf_or_str {
            if (sp.buf_lnum as usize) >= sp.buflines.len() {
                break;
            }
            let line_bytes = sp.buflines.as_slice()[sp.buf_lnum as usize].to_bytes();
            buf.extend_from_slice(line_bytes);
            sp.buf_lnum += 1;
            buf.push(0);
            len = buf.len();
        } else {
            let start = buf.len();
            buf.resize(start + 120, 0);
            let mut eof = false;
            let fp = sp.fp.as_ref().unwrap();
            loop {
                crate::os::fs::os_set_errno(0);
                if !fp.gets(&mut buf[start..start + 120]) {
                    if crate::os::fs::os_errno() == EINTR {
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
            let read_len = crate::memory::bytes_as_cstr(&buf[start..]).to_bytes().len();
            filled_chunk = read_len == 119;
            len = start + read_len;
            buf.truncate(len);
        }

        #[cfg(windows)]
        {
            if (len == 1 || (len >= 2 && buf[len - 2] == b'\n'))
                && sp.fileformat == EOL_DOS
                && buf[len - 1] == CTRL_Z_CONST
            {
                buf.truncate(len - 1);
                break;
            }
        }

        have_read = true;

        if !sp.source_from_buf_or_str && filled_chunk && buf[len - 1] != b'\n' {
            continue;
        }

        if len >= 1 && buf[len - 1] == b'\n' {
            #[cfg(windows)]
            {
                let has_cr = len >= 2 && buf[len - 2] == b'\r';
                if sp.fileformat == EOL_UNKNOWN {
                    sp.fileformat = if has_cr { EOL_DOS } else { EOL_UNIX };
                }

                if sp.fileformat == EOL_DOS {
                    if has_cr {
                        buf[len - 2] = b'\n';
                        len -= 1;
                        buf.truncate(len);
                    } else {
                        if !sp.error {
                            msg_source(crate::highlight::hl_attr(HLF_W as i32));
                            emsg(gettext(
                                c"W15: Warning: Wrong line separator, ^M may be missing",
                            ));
                        }
                        sp.error = true;
                        sp.fileformat = EOL_UNIX;
                    }
                }
            }

            let mut c = (len as isize) - 2;
            while c >= 0 && buf[c as usize] == CTRL_V_CONST as u8 {
                c -= 1;
            }
            if ((len as isize) & 1) != (c & 1) {
                sp.sourcing_lnum += 1;
                continue;
            }

            buf.truncate(len - 1);
        }

        line_breakcheck();
        break;
    }

    if have_read {
        if buf.last() != Some(&0) {
            buf.push(0);
        }
        return Some(buf);
    }

    None
}

/// Returns true if sourcing a script either from a file or a buffer or a string.
pub fn sourcing_a_script(eap: &mut Exarg) -> i32 {
    i32::from(eap.ea_getline.is_source())
}

/// ":scriptencoding": Set encoding conversion for a sourced script.
pub fn ex_scriptencoding(eap: &mut Exarg) {
    let Some(sp) = eap.ea_getline.as_source_mut() else {
        emsg(gettext(
            c"E167: :scriptencoding used outside of a sourced file",
        ));
        return;
    };

    let name_owned = (!eap.arg.is_empty()).then(|| enc_canonize(eap.arg.as_cstr()));
    let name_opt = if let Some(ref n) = name_owned {
        Some(n.as_c_str())
    } else {
        eap.arg.as_opt_cstr()
    };

    convert_setup(&mut sp.conv, name_opt, Some(opt_vars().p_enc.as_cstr()));
}

/// ":finish": Mark a sourced file as finished.
pub fn ex_finish(eap: &mut Exarg) {
    if sourcing_a_script(eap) != 0 {
        do_finish(eap, false);
    } else {
        emsg(gettext(c"E168: :finish used outside of a sourced file"));
    }
}

/// Mark a sourced file as finished.
pub fn do_finish(eap: &mut Exarg, reanimate: bool) {
    let sp = eap.ea_getline.as_source_mut().unwrap();
    if reanimate {
        sp.finished = false;
    }

    let cstack = eap.cstack_mut();
    let idx = cleanup_conditionals(&mut *cstack, 0, true);
    if idx >= 0 {
        cstack.cs_pending[idx as usize] = CSTP_FINISH as u8;
        report_make_pending(CSTP_FINISH, PendingVal::None);
    } else {
        sp.finished = true;
    }
}

pub fn source_finished(fgetline: LineGetter) -> bool {
    fgetline.as_source_mut().is_some_and(|sp| sp.finished)
}

/// Return the autoload script name for a function or variable name.
pub fn autoload_name(name: &[u8]) -> std::ffi::CString {
    const PREFIX: &[u8] = b"autoload/";
    const SUFFIX: &[u8] = b".vim";
    let name_len = name.len();
    let mut scriptname = Vec::with_capacity(name_len + PREFIX.len() + SUFFIX.len());
    scriptname.extend_from_slice(PREFIX);
    scriptname.extend_from_slice(name);
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

    bytes_to_cstring(&scriptname)
}

/// If name has a package name try autoloading the script for it.
pub fn script_autoload(slice: &[u8], reload: bool) -> bool {
    let Some(pos) = slice.iter().position(|&b| b == AUTOLOAD_CHAR) else {
        return false;
    };
    if pos == 0 {
        return false;
    }

    let mut ret = false;
    let scriptname_cstr = autoload_name(slice);

    let target_tail = &scriptname_cstr.as_bytes()[9..];
    let already_loaded = rt_st()
        .ga_loaded
        .iter()
        .any(|entry| &entry.as_bytes()[9..] == target_tail);
    if reload || !already_loaded {
        if !already_loaded {
            rt_st().ga_loaded.push(scriptname_cstr.clone());
        }

        if do_in_runtimepath(
            &scriptname_cstr,
            DIP_START,
            &mut RuntimeCallback::Source {
                skip_if_loaded: true,
            },
        ) {
            ret = true;
        }
    }

    ret
}
