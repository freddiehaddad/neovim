//! Autocommand registration, matching, and execution (`src/nvim/autocmd.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{
    HLF_E, api_clear_error, api_free_object, api_free_string, boolean_obj, copy_object,
    cstr_as_obj, cstr_as_string, cstr_to_string, dict_obj, find_buffer_by_handle, integer_obj,
    static_cstr_as_string, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType,
};
use crate::api::tabpage::Buffer;
use crate::api::ui::NO_SCREEN;
use crate::buffer::{
    bt_nofile, buf_T, buflist_findnr, bufref_T, bufref_valid, do_modelines, set_bufref,
};
use crate::charset::{NUL, ascii_iswhite, getdigits_int, skipdigits, skipwhite};
use crate::cmdexpand::{EXPAND_EVENTS, EXPAND_FILES, EXPAND_NOTHING};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::errors::*;
use crate::eval::typval::{
    Callback, callback_copy, callback_free, callback_to_string, kCallbackLua, tv_clear,
    tv_dict_add_nr, tv_dict_add_tv, tv_dict_set_keys_readonly, typval_T, varnumber_T,
};
use crate::eval::userfunc::{restore_funccal, save_funccal};
use crate::eval::vars::{
    VV_CMDBANG, VV_OPTION_TYPE, VV_TERMRESPONSE, get_vim_var_nr, get_vim_var_str, set_cmdarg,
    set_vim_var_nr,
};
use crate::eval::{callback_call, get_v_event, last_set_msg, restore_v_event};
use crate::event::r#loop::{Loop, loop_queues};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, multiqueue_free, multiqueue_new_child, multiqueue_put_event,
};
use crate::ex_docmd::{do_cmdline, ends_excmd, exarg_T, expand_sfile};
use crate::ex_eval::{aborting, should_abort};
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::fileio::{check_timestamps, file_pat_to_reg_pat, match_file_pat};
use crate::globals::*;
use crate::input::{RedoState, restore_redobuff, save_redobuff};
use crate::insexpand::ins_compl_active;
use crate::lua::converter::LuaRef;
use crate::lua::executor::{kRetNilBool, nlua_call_ref, nlua_set_sctx};
use crate::map::{
    Map, MapHash, Set, map_del_String_int, map_del_int_String, map_put_ref_String_int,
    map_put_ref_int_String, map_ref_String_int, map_ref_int_String,
};
use crate::math::{FAIL, OK};
use crate::mbyte::{mb_strcmp_ic, vim_strchr, vim_strnicmp_asc};
use crate::memory::{xcalloc, xfree, xmalloc, xmallocz, xmemdupz, xrealloc, xstrdup, xstrnsave};
use crate::message::{
    emsg, give_warning, msg_advance, msg_clr_eos, msg_end, msg_ext_set_kind, msg_outtrans,
    msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, msg_start, semsg, smsg, verbose_enter,
    verbose_enter_scroll, verbose_leave, verbose_leave_scroll,
};
use crate::msgpack_rpc::packer::{Integer, handle_T};
use crate::option::{OPT_LOCAL, apply_optionset_autocmd_now, set_option_direct};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::env::{expand_T, expand_env_save};
use crate::os::fs::MAXPATHL;
use crate::os::input::line_breakcheck;
use crate::os::time::{Timestamp, os_now};
use crate::path::{FullName_save, path_tail, to_slash_save};
use crate::profile::{estack_T, prof_child_enter, prof_child_exit, proftime_T};
use crate::regexp::{RE_MAGIC, RE_NOBREAK, vim_regcomp, vim_regfree};
use crate::runtime::{ETYPE_AUCMD, SID_NONE, estack_pop, estack_push, exestack, sctx_T};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::state::{MODE_INSERT, MODE_NORMAL_BUSY, get_mode, get_real_state, save_v_event_T};
use crate::strings::tr;
use crate::window::{
    check_lnums, check_lnums_nested, reset_lnums, snapshot_windows_scroll_size, unuse_tabpage,
    use_tabpage, win_T,
};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

pub type event_T = c_int;
pub(crate) const BUFLOCAL_PAT_LEN: usize = 25;

#[repr(C)]
pub struct AutoPat {
    pub refcount: usize,
    pub pat: *mut c_char,
    pub reg_prog: *mut crate::regexp::regprog_T,
    pub group: c_int,
    pub patlen: c_int,
    pub buflocal_nr: c_int,
    pub allow_dirs: c_char,
}

#[repr(C)]
pub struct AutoCmd {
    pub pat: *mut AutoPat,
    pub id: i64,
    pub desc: *mut c_char,
    pub handler_cmd: *mut c_char,
    pub handler_fn: Callback,
    pub script_ctx: crate::runtime::sctx_T,
    pub once: bool,
    pub nested: bool,
}

#[repr(C)]
pub struct AutoCmdVec {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut AutoCmd,
}

pub const AUGROUP_DEFAULT: c_int = -1;
pub const AUGROUP_ERROR: c_int = -2;
pub const AUGROUP_ALL: c_int = -3;
pub const AUGROUP_DELETED: c_int = -4;

pub(crate) const OPT_NOWIN: c_int = 0x10;
pub(crate) const kOptEventignore: OptIndex = 88;
pub(crate) const kOptModified: OptIndex = 196;

pub(crate) const DOCMD_VERBOSE: c_int = 0x01;
pub(crate) const DOCMD_NOWAIT: c_int = 0x02;
pub(crate) const DOCMD_REPEAT: c_int = 0x04;

pub(crate) const PROF_YES: c_int = 1;

pub(crate) const HLF_8: c_int = 1;
pub(crate) const HLF_T: c_int = 23;

pub const EVENT_BUFADD: event_T = 0;
pub const EVENT_BUFCREATE: event_T = 1;
pub const EVENT_BUFDELETE: event_T = 2;
pub const EVENT_BUFENTER: event_T = 3;
pub const EVENT_BUFFILEPOST: event_T = 4;
pub const EVENT_BUFFILEPRE: event_T = 5;
pub const EVENT_BUFHIDDEN: event_T = 6;
pub const EVENT_BUFLEAVE: event_T = 7;
pub const EVENT_BUFNEW: event_T = 8;
pub const EVENT_BUFNEWFILE: event_T = 9;
pub const EVENT_BUFREAD: event_T = 10;
pub const EVENT_BUFREADCMD: event_T = 11;
pub const EVENT_BUFREADPOST: event_T = 12;
pub const EVENT_BUFREADPRE: event_T = 13;
pub const EVENT_BUFUNLOAD: event_T = 14;
pub const EVENT_BUFWINENTER: event_T = 15;
pub const EVENT_BUFWINLEAVE: event_T = 16;
pub const EVENT_BUFWIPEOUT: event_T = 17;
pub const EVENT_BUFWRITE: event_T = 18;
pub const EVENT_BUFWRITECMD: event_T = 19;
pub const EVENT_BUFWRITEPOST: event_T = 20;
pub const EVENT_BUFWRITEPRE: event_T = 21;
pub const EVENT_CHANCLOSE: event_T = 22;
pub const EVENT_CHANINFO: event_T = 23;
pub const EVENT_CHANOPEN: event_T = 24;
pub const EVENT_CMDATOM: event_T = 25;
pub const EVENT_CMDLINECHANGED: event_T = 26;
pub const EVENT_CMDLINEENTER: event_T = 27;
pub const EVENT_CMDLINELEAVE: event_T = 28;
pub const EVENT_CMDLINELEAVEPRE: event_T = 29;
pub const EVENT_CMDUNDEFINED: event_T = 30;
pub const EVENT_CMDWINENTER: event_T = 31;
pub const EVENT_CMDWINLEAVE: event_T = 32;
pub const EVENT_COLORSCHEME: event_T = 33;
pub const EVENT_COLORSCHEMEPRE: event_T = 34;
pub const EVENT_COMPLETECHANGED: event_T = 35;
pub const EVENT_COMPLETEDONE: event_T = 36;
pub const EVENT_COMPLETEDONEPRE: event_T = 37;
pub const EVENT_CURSORHOLD: event_T = 38;
pub const EVENT_CURSORHOLDI: event_T = 39;
pub const EVENT_CURSORMOVED: event_T = 40;
pub const EVENT_CURSORMOVEDC: event_T = 41;
pub const EVENT_CURSORMOVEDI: event_T = 42;
pub const EVENT_DIAGNOSTICCHANGED: event_T = 43;
pub const EVENT_DIFFUPDATED: event_T = 44;
pub const EVENT_DIRCHANGED: event_T = 45;
pub const EVENT_DIRCHANGEDPRE: event_T = 46;
pub const EVENT_ENCODINGCHANGED: event_T = 47;
pub const EVENT_EXITPRE: event_T = 48;
pub const EVENT_FILEAPPENDCMD: event_T = 49;
pub const EVENT_FILEAPPENDPOST: event_T = 50;
pub const EVENT_FILEAPPENDPRE: event_T = 51;
pub const EVENT_FILECHANGEDRO: event_T = 52;
pub const EVENT_FILECHANGEDSHELL: event_T = 53;
pub const EVENT_FILECHANGEDSHELLPOST: event_T = 54;
pub const EVENT_FILEENCODING: event_T = 55;
pub const EVENT_FILEREADCMD: event_T = 56;
pub const EVENT_FILEREADPOST: event_T = 57;
pub const EVENT_FILEREADPRE: event_T = 58;
pub const EVENT_FILETYPE: event_T = 59;
pub const EVENT_FILEWRITECMD: event_T = 60;
pub const EVENT_FILEWRITEPOST: event_T = 61;
pub const EVENT_FILEWRITEPRE: event_T = 62;
pub const EVENT_FILTERREADPOST: event_T = 63;
pub const EVENT_FILTERREADPRE: event_T = 64;
pub const EVENT_FILTERWRITEPOST: event_T = 65;
pub const EVENT_FILTERWRITEPRE: event_T = 66;
pub const EVENT_FOCUSGAINED: event_T = 67;
pub const EVENT_FOCUSLOST: event_T = 68;
pub const EVENT_FUNCUNDEFINED: event_T = 69;
pub const EVENT_GUIENTER: event_T = 70;
pub const EVENT_GUIFAILED: event_T = 71;
pub const EVENT_INSERTCHANGE: event_T = 72;
pub const EVENT_INSERTCHARPRE: event_T = 73;
pub const EVENT_INSERTENTER: event_T = 74;
pub const EVENT_INSERTLEAVE: event_T = 75;
pub const EVENT_INSERTLEAVEPRE: event_T = 76;
pub const EVENT_LSPATTACH: event_T = 77;
pub const EVENT_LSPDETACH: event_T = 78;
pub const EVENT_LSPNOTIFY: event_T = 79;
pub const EVENT_LSPPROGRESS: event_T = 80;
pub const EVENT_LSPREQUEST: event_T = 81;
pub const EVENT_LSPTOKENUPDATE: event_T = 82;
pub const EVENT_MARKSET: event_T = 83;
pub const EVENT_MENUPOPUP: event_T = 84;
pub const EVENT_MODECHANGED: event_T = 85;
pub const EVENT_OPTIONSET: event_T = 86;
pub const EVENT_PACKCHANGED: event_T = 87;
pub const EVENT_PACKCHANGEDPRE: event_T = 88;
pub const EVENT_PROGRESS: event_T = 89;
pub const EVENT_QUICKFIXCMDPOST: event_T = 90;
pub const EVENT_QUICKFIXCMDPRE: event_T = 91;
pub const EVENT_QUITPRE: event_T = 92;
pub const EVENT_RECORDINGENTER: event_T = 93;
pub const EVENT_RECORDINGLEAVE: event_T = 94;
pub const EVENT_REMOTEREPLY: event_T = 95;
pub const EVENT_SAFESTATE: event_T = 96;
pub const EVENT_SEARCHWRAPPED: event_T = 97;
pub const EVENT_SESSIONLOADPOST: event_T = 98;
pub const EVENT_SESSIONLOADPRE: event_T = 99;
pub const EVENT_SESSIONWRITEPOST: event_T = 100;
pub const EVENT_SESSIONWRITEPRE: event_T = 101;
pub const EVENT_SHELLCMDPOST: event_T = 102;
pub const EVENT_SHELLFILTERPOST: event_T = 103;
pub const EVENT_SIGNAL: event_T = 104;
pub const EVENT_SOURCECMD: event_T = 105;
pub const EVENT_SOURCEPOST: event_T = 106;
pub const EVENT_SOURCEPRE: event_T = 107;
pub const EVENT_SPELLFILEMISSING: event_T = 108;
pub const EVENT_STDINREADPOST: event_T = 109;
pub const EVENT_STDINREADPRE: event_T = 110;
pub const EVENT_SWAPEXISTS: event_T = 111;
pub const EVENT_SYNTAX: event_T = 112;
pub const EVENT_TABCLOSED: event_T = 113;
pub const EVENT_TABCLOSEDPRE: event_T = 114;
pub const EVENT_TABENTER: event_T = 115;
pub const EVENT_TABLEAVE: event_T = 116;
pub const EVENT_TABMOVED: event_T = 117;
pub const EVENT_TABNEW: event_T = 118;
pub const EVENT_TABNEWENTERED: event_T = 119;
pub const EVENT_TERMCHANGED: event_T = 120;
pub const EVENT_TERMCLOSE: event_T = 121;
pub const EVENT_TERMENTER: event_T = 122;
pub const EVENT_TERMLEAVE: event_T = 123;
pub const EVENT_TERMOPEN: event_T = 124;
pub const EVENT_TERMREQUEST: event_T = 125;
pub const EVENT_TERMRESPONSE: event_T = 126;
pub const EVENT_TEXTCHANGED: event_T = 127;
pub const EVENT_TEXTCHANGEDI: event_T = 128;
pub const EVENT_TEXTCHANGEDP: event_T = 129;
pub const EVENT_TEXTCHANGEDT: event_T = 130;
pub const EVENT_TEXTPUTPOST: event_T = 131;
pub const EVENT_TEXTPUTPRE: event_T = 132;
pub const EVENT_TEXTYANKPOST: event_T = 133;
pub const EVENT_UIENTER: event_T = 134;
pub const EVENT_UILEAVE: event_T = 135;
pub const EVENT_USER: event_T = 136;
pub const EVENT_VIMENTER: event_T = 137;
pub const EVENT_VIMLEAVE: event_T = 138;
pub const EVENT_VIMLEAVEPRE: event_T = 139;
pub const EVENT_VIMRESIZED: event_T = 140;
pub const EVENT_VIMRESUME: event_T = 141;
pub const EVENT_VIMSUSPEND: event_T = 142;
pub const EVENT_WINCLOSED: event_T = 143;
pub const EVENT_WINENTER: event_T = 144;
pub const EVENT_WINLEAVE: event_T = 145;
pub const EVENT_WINNEW: event_T = 146;
pub const EVENT_WINNEWPRE: event_T = 147;
pub const EVENT_WINRESIZED: event_T = 148;
pub const EVENT_WINSCROLLED: event_T = 149;
pub const NUM_EVENTS: usize = 150;

const E_AUTOCOMMAND_NESTING_TOO_DEEP: &CStr = c"E218: Autocommand nesting too deep";

#[repr(C)]
pub struct AutoPatCmd {
    pub lastpat: *mut AutoPat,
    pub auidx: usize,
    pub ausize: usize,
    pub afile_orig: *mut c_char,
    pub fname: *mut c_char,
    pub sfname: *mut c_char,
    pub tail: *mut c_char,
    pub group: c_int,
    pub event: event_T,
    pub script_ctx: sctx_T,
    pub arg_bufnr: c_int,
    pub data: *mut Object,
    pub next: *mut AutoPatCmd,
}

#[repr(C)]
pub struct AutoCmdEvent {
    pub event: event_T,
    pub fname: *mut c_char,
    pub fname_io: *mut c_char,
    pub buf: Buffer,
    pub group: c_int,
    pub eap: *mut exarg_T,
    pub data: *mut Object,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct funccal_entry_T {
    pub top_funccal: *mut c_void,
    pub next: *mut funccal_entry_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ExpandPrefix {
    xp_pattern: *mut c_char,
    xp_context: c_int,
}

#[derive(Clone, Copy)]
struct EventNameEntry {
    _len: usize,
    name: &'static CStr,
    event: c_int,
}

const EMPTY_AUCMD_VEC: AutoCmdVec = AutoCmdVec {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};

pub(crate) const EMPTY_MAP_HASH: MapHash = MapHash {
    hash: ptr::null_mut(),
    n_buckets: 0,
    size: 0,
    n_occupied: 0,
    upper_bound: 0,
    n_keys: 0,
    keys_capacity: 0,
};

const EMPTY_NAME_TO_ID_MAP: Map<NvimString, c_int> = Map {
    set: Set {
        h: EMPTY_MAP_HASH,
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

const EMPTY_ID_TO_NAME_MAP: Map<c_int, NvimString> = Map {
    set: Set {
        h: EMPTY_MAP_HASH,
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

static EVENT_NAMES: [EventNameEntry; NUM_EVENTS] = [
    EventNameEntry {
        _len: 6,
        name: c"BufAdd",
        event: -EVENT_BUFADD,
    },
    EventNameEntry {
        _len: 9,
        name: c"BufCreate",
        event: -EVENT_BUFADD,
    },
    EventNameEntry {
        _len: 9,
        name: c"BufDelete",
        event: -EVENT_BUFDELETE,
    },
    EventNameEntry {
        _len: 8,
        name: c"BufEnter",
        event: -EVENT_BUFENTER,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufFilePost",
        event: -EVENT_BUFFILEPOST,
    },
    EventNameEntry {
        _len: 10,
        name: c"BufFilePre",
        event: -EVENT_BUFFILEPRE,
    },
    EventNameEntry {
        _len: 9,
        name: c"BufHidden",
        event: -EVENT_BUFHIDDEN,
    },
    EventNameEntry {
        _len: 8,
        name: c"BufLeave",
        event: -EVENT_BUFLEAVE,
    },
    EventNameEntry {
        _len: 6,
        name: c"BufNew",
        event: -EVENT_BUFNEW,
    },
    EventNameEntry {
        _len: 10,
        name: c"BufNewFile",
        event: -EVENT_BUFNEWFILE,
    },
    EventNameEntry {
        _len: 7,
        name: c"BufRead",
        event: -EVENT_BUFREADPOST,
    },
    EventNameEntry {
        _len: 10,
        name: c"BufReadCmd",
        event: -EVENT_BUFREADCMD,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufReadPost",
        event: -EVENT_BUFREADPOST,
    },
    EventNameEntry {
        _len: 10,
        name: c"BufReadPre",
        event: -EVENT_BUFREADPRE,
    },
    EventNameEntry {
        _len: 9,
        name: c"BufUnload",
        event: -EVENT_BUFUNLOAD,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufWinEnter",
        event: -EVENT_BUFWINENTER,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufWinLeave",
        event: -EVENT_BUFWINLEAVE,
    },
    EventNameEntry {
        _len: 10,
        name: c"BufWipeout",
        event: -EVENT_BUFWIPEOUT,
    },
    EventNameEntry {
        _len: 8,
        name: c"BufWrite",
        event: -EVENT_BUFWRITEPRE,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufWriteCmd",
        event: -EVENT_BUFWRITECMD,
    },
    EventNameEntry {
        _len: 12,
        name: c"BufWritePost",
        event: -EVENT_BUFWRITEPOST,
    },
    EventNameEntry {
        _len: 11,
        name: c"BufWritePre",
        event: -EVENT_BUFWRITEPRE,
    },
    EventNameEntry {
        _len: 9,
        name: c"ChanClose",
        event: EVENT_CHANCLOSE,
    },
    EventNameEntry {
        _len: 8,
        name: c"ChanInfo",
        event: EVENT_CHANINFO,
    },
    EventNameEntry {
        _len: 8,
        name: c"ChanOpen",
        event: EVENT_CHANOPEN,
    },
    EventNameEntry {
        _len: 7,
        name: c"CmdAtom",
        event: EVENT_CMDATOM,
    },
    EventNameEntry {
        _len: 14,
        name: c"CmdlineChanged",
        event: EVENT_CMDLINECHANGED,
    },
    EventNameEntry {
        _len: 12,
        name: c"CmdlineEnter",
        event: EVENT_CMDLINEENTER,
    },
    EventNameEntry {
        _len: 12,
        name: c"CmdlineLeave",
        event: EVENT_CMDLINELEAVE,
    },
    EventNameEntry {
        _len: 15,
        name: c"CmdlineLeavePre",
        event: EVENT_CMDLINELEAVEPRE,
    },
    EventNameEntry {
        _len: 12,
        name: c"CmdUndefined",
        event: EVENT_CMDUNDEFINED,
    },
    EventNameEntry {
        _len: 11,
        name: c"CmdwinEnter",
        event: EVENT_CMDWINENTER,
    },
    EventNameEntry {
        _len: 11,
        name: c"CmdwinLeave",
        event: EVENT_CMDWINLEAVE,
    },
    EventNameEntry {
        _len: 11,
        name: c"ColorScheme",
        event: EVENT_COLORSCHEME,
    },
    EventNameEntry {
        _len: 14,
        name: c"ColorSchemePre",
        event: EVENT_COLORSCHEMEPRE,
    },
    EventNameEntry {
        _len: 15,
        name: c"CompleteChanged",
        event: EVENT_COMPLETECHANGED,
    },
    EventNameEntry {
        _len: 12,
        name: c"CompleteDone",
        event: EVENT_COMPLETEDONE,
    },
    EventNameEntry {
        _len: 15,
        name: c"CompleteDonePre",
        event: EVENT_COMPLETEDONEPRE,
    },
    EventNameEntry {
        _len: 10,
        name: c"CursorHold",
        event: -EVENT_CURSORHOLD,
    },
    EventNameEntry {
        _len: 11,
        name: c"CursorHoldI",
        event: -EVENT_CURSORHOLDI,
    },
    EventNameEntry {
        _len: 11,
        name: c"CursorMoved",
        event: -EVENT_CURSORMOVED,
    },
    EventNameEntry {
        _len: 12,
        name: c"CursorMovedC",
        event: -EVENT_CURSORMOVEDC,
    },
    EventNameEntry {
        _len: 12,
        name: c"CursorMovedI",
        event: -EVENT_CURSORMOVEDI,
    },
    EventNameEntry {
        _len: 17,
        name: c"DiagnosticChanged",
        event: EVENT_DIAGNOSTICCHANGED,
    },
    EventNameEntry {
        _len: 11,
        name: c"DiffUpdated",
        event: EVENT_DIFFUPDATED,
    },
    EventNameEntry {
        _len: 10,
        name: c"DirChanged",
        event: EVENT_DIRCHANGED,
    },
    EventNameEntry {
        _len: 13,
        name: c"DirChangedPre",
        event: EVENT_DIRCHANGEDPRE,
    },
    EventNameEntry {
        _len: 15,
        name: c"EncodingChanged",
        event: EVENT_ENCODINGCHANGED,
    },
    EventNameEntry {
        _len: 7,
        name: c"ExitPre",
        event: EVENT_EXITPRE,
    },
    EventNameEntry {
        _len: 13,
        name: c"FileAppendCmd",
        event: -EVENT_FILEAPPENDCMD,
    },
    EventNameEntry {
        _len: 14,
        name: c"FileAppendPost",
        event: -EVENT_FILEAPPENDPOST,
    },
    EventNameEntry {
        _len: 13,
        name: c"FileAppendPre",
        event: -EVENT_FILEAPPENDPRE,
    },
    EventNameEntry {
        _len: 13,
        name: c"FileChangedRO",
        event: -EVENT_FILECHANGEDRO,
    },
    EventNameEntry {
        _len: 16,
        name: c"FileChangedShell",
        event: -EVENT_FILECHANGEDSHELL,
    },
    EventNameEntry {
        _len: 20,
        name: c"FileChangedShellPost",
        event: -EVENT_FILECHANGEDSHELLPOST,
    },
    EventNameEntry {
        _len: 12,
        name: c"FileEncoding",
        event: EVENT_ENCODINGCHANGED,
    },
    EventNameEntry {
        _len: 11,
        name: c"FileReadCmd",
        event: -EVENT_FILEREADCMD,
    },
    EventNameEntry {
        _len: 12,
        name: c"FileReadPost",
        event: -EVENT_FILEREADPOST,
    },
    EventNameEntry {
        _len: 11,
        name: c"FileReadPre",
        event: -EVENT_FILEREADPRE,
    },
    EventNameEntry {
        _len: 8,
        name: c"FileType",
        event: -EVENT_FILETYPE,
    },
    EventNameEntry {
        _len: 12,
        name: c"FileWriteCmd",
        event: -EVENT_FILEWRITECMD,
    },
    EventNameEntry {
        _len: 13,
        name: c"FileWritePost",
        event: -EVENT_FILEWRITEPOST,
    },
    EventNameEntry {
        _len: 12,
        name: c"FileWritePre",
        event: -EVENT_FILEWRITEPRE,
    },
    EventNameEntry {
        _len: 14,
        name: c"FilterReadPost",
        event: -EVENT_FILTERREADPOST,
    },
    EventNameEntry {
        _len: 13,
        name: c"FilterReadPre",
        event: -EVENT_FILTERREADPRE,
    },
    EventNameEntry {
        _len: 15,
        name: c"FilterWritePost",
        event: -EVENT_FILTERWRITEPOST,
    },
    EventNameEntry {
        _len: 14,
        name: c"FilterWritePre",
        event: -EVENT_FILTERWRITEPRE,
    },
    EventNameEntry {
        _len: 11,
        name: c"FocusGained",
        event: EVENT_FOCUSGAINED,
    },
    EventNameEntry {
        _len: 9,
        name: c"FocusLost",
        event: EVENT_FOCUSLOST,
    },
    EventNameEntry {
        _len: 13,
        name: c"FuncUndefined",
        event: EVENT_FUNCUNDEFINED,
    },
    EventNameEntry {
        _len: 8,
        name: c"GUIEnter",
        event: EVENT_GUIENTER,
    },
    EventNameEntry {
        _len: 9,
        name: c"GUIFailed",
        event: EVENT_GUIFAILED,
    },
    EventNameEntry {
        _len: 12,
        name: c"InsertChange",
        event: -EVENT_INSERTCHANGE,
    },
    EventNameEntry {
        _len: 13,
        name: c"InsertCharPre",
        event: -EVENT_INSERTCHARPRE,
    },
    EventNameEntry {
        _len: 11,
        name: c"InsertEnter",
        event: -EVENT_INSERTENTER,
    },
    EventNameEntry {
        _len: 11,
        name: c"InsertLeave",
        event: -EVENT_INSERTLEAVE,
    },
    EventNameEntry {
        _len: 14,
        name: c"InsertLeavePre",
        event: -EVENT_INSERTLEAVEPRE,
    },
    EventNameEntry {
        _len: 9,
        name: c"LspAttach",
        event: EVENT_LSPATTACH,
    },
    EventNameEntry {
        _len: 9,
        name: c"LspDetach",
        event: EVENT_LSPDETACH,
    },
    EventNameEntry {
        _len: 9,
        name: c"LspNotify",
        event: EVENT_LSPNOTIFY,
    },
    EventNameEntry {
        _len: 11,
        name: c"LspProgress",
        event: EVENT_LSPPROGRESS,
    },
    EventNameEntry {
        _len: 10,
        name: c"LspRequest",
        event: EVENT_LSPREQUEST,
    },
    EventNameEntry {
        _len: 14,
        name: c"LspTokenUpdate",
        event: EVENT_LSPTOKENUPDATE,
    },
    EventNameEntry {
        _len: 7,
        name: c"MarkSet",
        event: EVENT_MARKSET,
    },
    EventNameEntry {
        _len: 9,
        name: c"MenuPopup",
        event: EVENT_MENUPOPUP,
    },
    EventNameEntry {
        _len: 11,
        name: c"ModeChanged",
        event: EVENT_MODECHANGED,
    },
    EventNameEntry {
        _len: 9,
        name: c"OptionSet",
        event: EVENT_OPTIONSET,
    },
    EventNameEntry {
        _len: 11,
        name: c"PackChanged",
        event: EVENT_PACKCHANGED,
    },
    EventNameEntry {
        _len: 14,
        name: c"PackChangedPre",
        event: EVENT_PACKCHANGEDPRE,
    },
    EventNameEntry {
        _len: 8,
        name: c"Progress",
        event: EVENT_PROGRESS,
    },
    EventNameEntry {
        _len: 15,
        name: c"QuickFixCmdPost",
        event: EVENT_QUICKFIXCMDPOST,
    },
    EventNameEntry {
        _len: 14,
        name: c"QuickFixCmdPre",
        event: EVENT_QUICKFIXCMDPRE,
    },
    EventNameEntry {
        _len: 7,
        name: c"QuitPre",
        event: EVENT_QUITPRE,
    },
    EventNameEntry {
        _len: 14,
        name: c"RecordingEnter",
        event: -EVENT_RECORDINGENTER,
    },
    EventNameEntry {
        _len: 14,
        name: c"RecordingLeave",
        event: -EVENT_RECORDINGLEAVE,
    },
    EventNameEntry {
        _len: 11,
        name: c"RemoteReply",
        event: EVENT_REMOTEREPLY,
    },
    EventNameEntry {
        _len: 9,
        name: c"SafeState",
        event: EVENT_SAFESTATE,
    },
    EventNameEntry {
        _len: 13,
        name: c"SearchWrapped",
        event: -EVENT_SEARCHWRAPPED,
    },
    EventNameEntry {
        _len: 15,
        name: c"SessionLoadPost",
        event: EVENT_SESSIONLOADPOST,
    },
    EventNameEntry {
        _len: 14,
        name: c"SessionLoadPre",
        event: EVENT_SESSIONLOADPRE,
    },
    EventNameEntry {
        _len: 16,
        name: c"SessionWritePost",
        event: EVENT_SESSIONWRITEPOST,
    },
    EventNameEntry {
        _len: 15,
        name: c"SessionWritePre",
        event: EVENT_SESSIONWRITEPRE,
    },
    EventNameEntry {
        _len: 12,
        name: c"ShellCmdPost",
        event: EVENT_SHELLCMDPOST,
    },
    EventNameEntry {
        _len: 15,
        name: c"ShellFilterPost",
        event: -EVENT_SHELLFILTERPOST,
    },
    EventNameEntry {
        _len: 6,
        name: c"Signal",
        event: EVENT_SIGNAL,
    },
    EventNameEntry {
        _len: 9,
        name: c"SourceCmd",
        event: EVENT_SOURCECMD,
    },
    EventNameEntry {
        _len: 10,
        name: c"SourcePost",
        event: EVENT_SOURCEPOST,
    },
    EventNameEntry {
        _len: 9,
        name: c"SourcePre",
        event: EVENT_SOURCEPRE,
    },
    EventNameEntry {
        _len: 16,
        name: c"SpellFileMissing",
        event: EVENT_SPELLFILEMISSING,
    },
    EventNameEntry {
        _len: 13,
        name: c"StdinReadPost",
        event: EVENT_STDINREADPOST,
    },
    EventNameEntry {
        _len: 12,
        name: c"StdinReadPre",
        event: EVENT_STDINREADPRE,
    },
    EventNameEntry {
        _len: 10,
        name: c"SwapExists",
        event: EVENT_SWAPEXISTS,
    },
    EventNameEntry {
        _len: 6,
        name: c"Syntax",
        event: EVENT_SYNTAX,
    },
    EventNameEntry {
        _len: 9,
        name: c"TabClosed",
        event: EVENT_TABCLOSED,
    },
    EventNameEntry {
        _len: 12,
        name: c"TabClosedPre",
        event: EVENT_TABCLOSEDPRE,
    },
    EventNameEntry {
        _len: 8,
        name: c"TabEnter",
        event: EVENT_TABENTER,
    },
    EventNameEntry {
        _len: 8,
        name: c"TabLeave",
        event: EVENT_TABLEAVE,
    },
    EventNameEntry {
        _len: 8,
        name: c"TabMoved",
        event: EVENT_TABMOVED,
    },
    EventNameEntry {
        _len: 6,
        name: c"TabNew",
        event: EVENT_TABNEW,
    },
    EventNameEntry {
        _len: 13,
        name: c"TabNewEntered",
        event: EVENT_TABNEWENTERED,
    },
    EventNameEntry {
        _len: 11,
        name: c"TermChanged",
        event: EVENT_TERMCHANGED,
    },
    EventNameEntry {
        _len: 9,
        name: c"TermClose",
        event: EVENT_TERMCLOSE,
    },
    EventNameEntry {
        _len: 9,
        name: c"TermEnter",
        event: EVENT_TERMENTER,
    },
    EventNameEntry {
        _len: 9,
        name: c"TermLeave",
        event: EVENT_TERMLEAVE,
    },
    EventNameEntry {
        _len: 8,
        name: c"TermOpen",
        event: EVENT_TERMOPEN,
    },
    EventNameEntry {
        _len: 11,
        name: c"TermRequest",
        event: EVENT_TERMREQUEST,
    },
    EventNameEntry {
        _len: 12,
        name: c"TermResponse",
        event: EVENT_TERMRESPONSE,
    },
    EventNameEntry {
        _len: 11,
        name: c"TextChanged",
        event: -EVENT_TEXTCHANGED,
    },
    EventNameEntry {
        _len: 12,
        name: c"TextChangedI",
        event: -EVENT_TEXTCHANGEDI,
    },
    EventNameEntry {
        _len: 12,
        name: c"TextChangedP",
        event: -EVENT_TEXTCHANGEDP,
    },
    EventNameEntry {
        _len: 12,
        name: c"TextChangedT",
        event: -EVENT_TEXTCHANGEDT,
    },
    EventNameEntry {
        _len: 11,
        name: c"TextPutPost",
        event: -EVENT_TEXTPUTPOST,
    },
    EventNameEntry {
        _len: 10,
        name: c"TextPutPre",
        event: -EVENT_TEXTPUTPRE,
    },
    EventNameEntry {
        _len: 12,
        name: c"TextYankPost",
        event: -EVENT_TEXTYANKPOST,
    },
    EventNameEntry {
        _len: 7,
        name: c"UIEnter",
        event: EVENT_UIENTER,
    },
    EventNameEntry {
        _len: 7,
        name: c"UILeave",
        event: EVENT_UILEAVE,
    },
    EventNameEntry {
        _len: 4,
        name: c"User",
        event: EVENT_USER,
    },
    EventNameEntry {
        _len: 8,
        name: c"VimEnter",
        event: EVENT_VIMENTER,
    },
    EventNameEntry {
        _len: 8,
        name: c"VimLeave",
        event: EVENT_VIMLEAVE,
    },
    EventNameEntry {
        _len: 11,
        name: c"VimLeavePre",
        event: EVENT_VIMLEAVEPRE,
    },
    EventNameEntry {
        _len: 10,
        name: c"VimResized",
        event: EVENT_VIMRESIZED,
    },
    EventNameEntry {
        _len: 9,
        name: c"VimResume",
        event: EVENT_VIMRESUME,
    },
    EventNameEntry {
        _len: 10,
        name: c"VimSuspend",
        event: EVENT_VIMSUSPEND,
    },
    EventNameEntry {
        _len: 9,
        name: c"WinClosed",
        event: -EVENT_WINCLOSED,
    },
    EventNameEntry {
        _len: 8,
        name: c"WinEnter",
        event: -EVENT_WINENTER,
    },
    EventNameEntry {
        _len: 8,
        name: c"WinLeave",
        event: -EVENT_WINLEAVE,
    },
    EventNameEntry {
        _len: 6,
        name: c"WinNew",
        event: EVENT_WINNEW,
    },
    EventNameEntry {
        _len: 9,
        name: c"WinNewPre",
        event: EVENT_WINNEWPRE,
    },
    EventNameEntry {
        _len: 10,
        name: c"WinResized",
        event: -EVENT_WINRESIZED,
    },
    EventNameEntry {
        _len: 11,
        name: c"WinScrolled",
        event: -EVENT_WINSCROLLED,
    },
];

static EVENT_HASH: [event_T; 150] = [
    EVENT_USER,
    EVENT_BUFADD,
    EVENT_BUFNEW,
    EVENT_SIGNAL,
    EVENT_SYNTAX,
    EVENT_TABNEW,
    EVENT_WINNEW,
    EVENT_BUFREAD,
    EVENT_CMDATOM,
    EVENT_EXITPRE,
    EVENT_MARKSET,
    EVENT_QUITPRE,
    EVENT_UIENTER,
    EVENT_UILEAVE,
    EVENT_CHANINFO,
    EVENT_CHANOPEN,
    EVENT_TABENTER,
    EVENT_TABLEAVE,
    EVENT_TABMOVED,
    EVENT_BUFENTER,
    EVENT_BUFLEAVE,
    EVENT_BUFWRITE,
    EVENT_GUIENTER,
    EVENT_FILETYPE,
    EVENT_VIMENTER,
    EVENT_VIMLEAVE,
    EVENT_WINENTER,
    EVENT_WINLEAVE,
    EVENT_PROGRESS,
    EVENT_TERMOPEN,
    EVENT_LSPATTACH,
    EVENT_BUFCREATE,
    EVENT_TABCLOSED,
    EVENT_WINCLOSED,
    EVENT_BUFDELETE,
    EVENT_LSPDETACH,
    EVENT_SAFESTATE,
    EVENT_GUIFAILED,
    EVENT_BUFHIDDEN,
    EVENT_OPTIONSET,
    EVENT_TERMCLOSE,
    EVENT_TERMENTER,
    EVENT_TERMLEAVE,
    EVENT_CHANCLOSE,
    EVENT_LSPNOTIFY,
    EVENT_WINNEWPRE,
    EVENT_SOURCECMD,
    EVENT_SOURCEPRE,
    EVENT_VIMRESUME,
    EVENT_BUFUNLOAD,
    EVENT_FOCUSLOST,
    EVENT_MENUPOPUP,
    EVENT_BUFREADCMD,
    EVENT_BUFREADPRE,
    EVENT_DIRCHANGED,
    EVENT_SOURCEPOST,
    EVENT_BUFFILEPRE,
    EVENT_BUFWIPEOUT,
    EVENT_LSPREQUEST,
    EVENT_CURSORHOLD,
    EVENT_VIMRESIZED,
    EVENT_VIMSUSPEND,
    EVENT_WINRESIZED,
    EVENT_TEXTPUTPRE,
    EVENT_BUFNEWFILE,
    EVENT_SWAPEXISTS,
    EVENT_BUFREADPOST,
    EVENT_VIMLEAVEPRE,
    EVENT_FILEREADCMD,
    EVENT_FILEREADPRE,
    EVENT_REMOTEREPLY,
    EVENT_TERMREQUEST,
    EVENT_FOCUSGAINED,
    EVENT_MODECHANGED,
    EVENT_PACKCHANGED,
    EVENT_TERMCHANGED,
    EVENT_TEXTCHANGED,
    EVENT_BUFWRITECMD,
    EVENT_BUFWRITEPRE,
    EVENT_BUFFILEPOST,
    EVENT_BUFWINENTER,
    EVENT_BUFWINLEAVE,
    EVENT_CMDWINENTER,
    EVENT_CMDWINLEAVE,
    EVENT_LSPPROGRESS,
    EVENT_DIFFUPDATED,
    EVENT_CURSORHOLDI,
    EVENT_CURSORMOVED,
    EVENT_WINSCROLLED,
    EVENT_COLORSCHEME,
    EVENT_INSERTENTER,
    EVENT_INSERTLEAVE,
    EVENT_TEXTPUTPOST,
    EVENT_TABCLOSEDPRE,
    EVENT_CMDLINEENTER,
    EVENT_CMDLINELEAVE,
    EVENT_CMDUNDEFINED,
    EVENT_STDINREADPRE,
    EVENT_SHELLCMDPOST,
    EVENT_BUFWRITEPOST,
    EVENT_FILEENCODING,
    EVENT_FILEREADPOST,
    EVENT_FILEWRITECMD,
    EVENT_FILEWRITEPRE,
    EVENT_COMPLETEDONE,
    EVENT_CURSORMOVEDC,
    EVENT_CURSORMOVEDI,
    EVENT_TERMRESPONSE,
    EVENT_INSERTCHANGE,
    EVENT_TEXTCHANGEDI,
    EVENT_TEXTCHANGEDP,
    EVENT_TEXTCHANGEDT,
    EVENT_TEXTYANKPOST,
    EVENT_FILEAPPENDCMD,
    EVENT_FILEAPPENDPRE,
    EVENT_FILECHANGEDRO,
    EVENT_SEARCHWRAPPED,
    EVENT_FILTERREADPRE,
    EVENT_TABNEWENTERED,
    EVENT_DIRCHANGEDPRE,
    EVENT_STDINREADPOST,
    EVENT_INSERTCHARPRE,
    EVENT_FUNCUNDEFINED,
    EVENT_FILEWRITEPOST,
    EVENT_RECORDINGENTER,
    EVENT_RECORDINGLEAVE,
    EVENT_QUICKFIXCMDPRE,
    EVENT_PACKCHANGEDPRE,
    EVENT_LSPTOKENUPDATE,
    EVENT_CMDLINECHANGED,
    EVENT_SESSIONLOADPRE,
    EVENT_FILEAPPENDPOST,
    EVENT_FILTERREADPOST,
    EVENT_FILTERWRITEPRE,
    EVENT_COLORSCHEMEPRE,
    EVENT_INSERTLEAVEPRE,
    EVENT_SESSIONLOADPOST,
    EVENT_SESSIONWRITEPRE,
    EVENT_SHELLFILTERPOST,
    EVENT_FILTERWRITEPOST,
    EVENT_CMDLINELEAVEPRE,
    EVENT_ENCODINGCHANGED,
    EVENT_COMPLETECHANGED,
    EVENT_COMPLETEDONEPRE,
    EVENT_QUICKFIXCMDPOST,
    EVENT_SESSIONWRITEPOST,
    EVENT_FILECHANGEDSHELL,
    EVENT_SPELLFILEMISSING,
    EVENT_DIAGNOSTICCHANGED,
    EVENT_FILECHANGEDSHELLPOST,
];

static mut autocmds: [AutoCmdVec; NUM_EVENTS] = [EMPTY_AUCMD_VEC; NUM_EVENTS];
static mut active_apc_list: *mut AutoPatCmd = ptr::null_mut();
static mut autocmd_winid: handle_T = 0;
static mut next_augroup_id: c_int = 1;
static mut deleted_augroup: *const c_char = ptr::null();
static mut current_augroup: c_int = AUGROUP_DEFAULT;
static mut au_need_clean: bool = false;
static mut autocmd_blocked: c_int = 0;
static mut autocmd_nested: bool = false;
static mut autocmd_include_groups: bool = false;
static mut termresponse_changed: bool = false;
static mut termresponse_chan_id: u64 = 0;
static mut map_augroup_name_to_id: Map<NvimString, c_int> = EMPTY_NAME_TO_ID_MAP;
static mut map_augroup_id_to_name: Map<c_int, NvimString> = EMPTY_ID_TO_NAME_MAP;
static mut apply_autocmds_nesting: c_int = 0;
static mut filechangeshell_busy: bool = false;
static mut pending_vimresume: TriState = kFalse;
static mut uienter_recursive: bool = false;
static mut focusgained_recursive: bool = false;
static mut focusgained_last_time: Timestamp = 0;
static mut ft_recursive: c_int = 0;

#[inline]
const fn tolower_asc(c: u8) -> u8 {
    if c.is_ascii_uppercase() { c + 0x20 } else { c }
}

#[inline]
unsafe fn vim_stricmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut p1 = s1.cast::<u8>();
    let mut p2 = s2.cast::<u8>();
    unsafe {
        loop {
            let c1 = tolower_asc(*p1);
            let c2 = tolower_asc(*p2);
            if c1 != c2 {
                return c_int::from(c1) - c_int::from(c2);
            }
            if c1 == 0 {
                return 0;
            }
            p1 = p1.add(1);
            p2 = p2.add(1);
        }
    }
}

#[inline]
unsafe fn vim_strnicmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    unsafe { vim_strnicmp_asc(s1, s2, n) }
}

#[inline]
unsafe fn kv_pushp_aucmd(v: &mut AutoCmdVec) -> *mut AutoCmd {
    if v.size == v.capacity {
        v.capacity = if v.capacity != 0 { v.capacity << 1 } else { 8 };
        v.items = unsafe {
            xrealloc(v.items.cast::<c_void>(), size_of::<AutoCmd>() * v.capacity).cast::<AutoCmd>()
        };
    }
    let ptr = unsafe { v.items.add(v.size) };
    v.size += 1;
    ptr
}

#[inline]
unsafe fn kv_destroy_aucmd(v: &mut AutoCmdVec) {
    unsafe {
        xfree(v.items.cast::<c_void>());
    }
    v.size = 0;
    v.capacity = 0;
    v.items = ptr::null_mut();
}

unsafe fn map_get_name_to_id(key: NvimString) -> c_int {
    let ptr = unsafe { map_ref_String_int(&raw mut map_augroup_name_to_id, key, ptr::null_mut()) };
    if ptr.is_null() { 0 } else { unsafe { *ptr } }
}

unsafe fn map_put_name_to_id(key: NvimString, val: c_int) {
    unsafe {
        let ptr = map_put_ref_String_int(
            &raw mut map_augroup_name_to_id,
            key,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        *ptr = val;
    }
}

unsafe fn map_get_id_to_name(key: c_int) -> NvimString {
    let ptr = unsafe { map_ref_int_String(&raw mut map_augroup_id_to_name, key, ptr::null_mut()) };
    if ptr.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        unsafe { *ptr }
    }
}

unsafe fn map_put_id_to_name(key: c_int, val: NvimString) {
    unsafe {
        let ptr = map_put_ref_int_String(
            &raw mut map_augroup_id_to_name,
            key,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        *ptr = val;
    }
}

fn event_name2nr_hash(str_: *const c_char, len: usize) -> isize {
    if str_.is_null() {
        return -1;
    }
    let s = unsafe { std::slice::from_raw_parts(str_.cast::<u8>(), len) };
    let (low, high): (usize, usize) = match len {
        4 => (0, 1),
        6 => match s[0] {
            b'B' | b'b' => (1, 3),
            b'S' | b's' => (3, 5),
            b'T' | b't' => (5, 6),
            b'W' | b'w' => (6, 7),
            _ => (0, 0),
        },
        7 => match s[0] {
            b'B' | b'b' => (7, 8),
            b'C' | b'c' => (8, 9),
            b'E' | b'e' => (9, 10),
            b'M' | b'm' => (10, 11),
            b'Q' | b'q' => (11, 12),
            b'U' | b'u' => (12, 14),
            _ => (0, 0),
        },
        8 => match s[2] {
            b'A' | b'a' => (14, 16),
            b'B' | b'b' => (16, 19),
            b'F' | b'f' => (19, 22),
            b'I' | b'i' => (22, 23),
            b'L' | b'l' => (23, 24),
            b'M' | b'm' => (24, 26),
            b'N' | b'n' => (26, 28),
            b'O' | b'o' => (28, 29),
            b'R' | b'r' => (29, 30),
            _ => (0, 0),
        },
        9 => match s[3] {
            b'A' | b'a' => (30, 31),
            b'C' | b'c' => (31, 34),
            b'D' | b'd' => (34, 36),
            b'E' | b'e' => (36, 37),
            b'F' | b'f' => (37, 38),
            b'H' | b'h' => (38, 39),
            b'I' | b'i' => (39, 40),
            b'M' | b'm' => (40, 43),
            b'N' | b'n' => (43, 46),
            b'R' | b'r' => (46, 49),
            b'U' | b'u' => (49, 52),
            _ => (0, 0),
        },
        10 => match s[5] {
            b'A' | b'a' => (52, 55),
            b'E' | b'e' => (55, 56),
            b'L' | b'l' => (56, 57),
            b'P' | b'p' => (57, 58),
            b'Q' | b'q' => (58, 59),
            b'R' | b'r' => (59, 60),
            b'S' | b's' => (60, 63),
            b'U' | b'u' => (63, 64),
            b'W' | b'w' => (64, 65),
            b'X' | b'x' => (65, 66),
            _ => (0, 0),
        },
        11 => match s[5] {
            b'A' | b'a' => (66, 68),
            b'E' | b'e' => (68, 72),
            b'G' | b'g' => (72, 73),
            b'H' | b'h' => (73, 77),
            b'I' | b'i' => (77, 79),
            b'L' | b'l' => (79, 80),
            b'N' | b'n' => (80, 84),
            b'O' | b'o' => (84, 85),
            b'P' | b'p' => (85, 86),
            b'R' | b'r' => (86, 89),
            b'S' | b's' => (89, 90),
            b'T' | b't' => (90, 92),
            b'U' | b'u' => (92, 93),
            _ => (0, 0),
        },
        12 => match s[2] {
            b'B' | b'b' => (93, 94),
            b'D' | b'd' => (94, 98),
            b'E' | b'e' => (98, 99),
            b'F' | b'f' => (99, 100),
            b'L' | b'l' => (100, 104),
            b'M' | b'm' => (104, 105),
            b'R' | b'r' => (105, 108),
            b'S' | b's' => (108, 109),
            b'X' | b'x' => (109, 113),
            _ => (0, 0),
        },
        13 => match s[4] {
            b'A' | b'a' => (113, 115),
            b'C' | b'c' => (115, 117),
            b'E' | b'e' => (117, 119),
            b'H' | b'h' => (119, 120),
            b'N' | b'n' => (120, 121),
            b'R' | b'r' => (121, 122),
            b'U' | b'u' => (122, 123),
            b'W' | b'w' => (123, 124),
            _ => (0, 0),
        },
        14 => match s[5] {
            b'D' | b'd' => (124, 126),
            b'F' | b'f' => (126, 127),
            b'H' | b'h' => (127, 128),
            b'K' | b'k' => (128, 129),
            b'N' | b'n' => (129, 130),
            b'O' | b'o' => (130, 131),
            b'P' | b'p' => (131, 132),
            b'R' | b'r' => (132, 134),
            b'S' | b's' => (134, 135),
            b'T' | b't' => (135, 136),
            _ => (0, 0),
        },
        15 => match s[1] {
            b'E' | b'e' => (136, 138),
            b'H' | b'h' => (138, 139),
            b'I' | b'i' => (139, 140),
            b'M' | b'm' => (140, 141),
            b'N' | b'n' => (141, 142),
            b'O' | b'o' => (142, 144),
            b'U' | b'u' => (144, 145),
            _ => (0, 0),
        },
        16 => match s[1] {
            b'E' | b'e' => (145, 146),
            b'I' | b'i' => (146, 147),
            b'P' | b'p' => (147, 148),
            _ => (0, 0),
        },
        17 => (148, 149),
        20 => (149, 150),
        _ => (0, 0),
    };

    for (i, &ev_raw) in EVENT_HASH.iter().enumerate().take(high).skip(low) {
        let ev = ev_raw as usize;
        if unsafe { vim_strnicmp_asc(str_, EVENT_NAMES[ev].name.as_ptr(), len) == 0 } {
            return i as isize;
        }
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_init() {
    unsafe {
        let events = (*loop_queues((&raw mut main_loop).cast::<Loop>())).events;
        deferred_events = multiqueue_new_child(events);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_free_all_mem() {
    unsafe {
        multiqueue_free(deferred_events);
    }
}

unsafe fn augroup_map_del(id: c_int, name: *const c_char) {
    unsafe {
        if !name.is_null() {
            let mut key = NvimString {
                data: ptr::null_mut(),
                size: 0,
            };
            map_del_String_int(
                &raw mut map_augroup_name_to_id,
                cstr_as_string(name),
                &raw mut key,
            );
            api_free_string(key);
        }
        if id > 0 {
            let mapped = map_del_int_String(&raw mut map_augroup_id_to_name, id, ptr::null_mut());
            api_free_string(mapped);
        }
    }
}

#[inline]
unsafe fn get_deleted_augroup() -> *const c_char {
    unsafe {
        if deleted_augroup.is_null() {
            deleted_augroup = tr(c"--Deleted--");
        }
        deleted_augroup
    }
}

unsafe fn au_show_for_all_events(group: c_int, pat: *const c_char) {
    unsafe {
        for event in 0..NUM_EVENTS as event_T {
            au_show_for_event(group, event, pat);
        }
    }
}

unsafe fn au_show_for_event(group: c_int, event: event_T, mut pat: *const c_char) {
    unsafe {
        let acs = &raw mut autocmds[event as usize];
        if (*acs).size == 0 {
            return;
        }

        let mut patlen: c_int = 0;
        if *pat != NUL {
            patlen = aucmd_span_pattern(pat, &raw mut pat) as c_int;
            if patlen == 0 {
                return;
            }
        }

        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];
        let mut last_group = AUGROUP_ERROR;
        let mut last_group_name: *const c_char;

        loop {
            let mut last_ap: *mut AutoPat = ptr::null_mut();
            let endpat = pat.add(patlen as usize);

            if aupat_is_buflocal(pat, patlen) {
                let nr = aupat_get_buflocal_nr(pat, patlen);
                aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, nr);
                pat = buflocal_pat.as_ptr();
                patlen = CStr::from_ptr(pat).to_bytes().len() as c_int;
            }

            for i in 0..(*acs).size {
                let ac = (*acs).items.add(i);
                if (*ac).pat.is_null() {
                    continue;
                }

                let ap = (*ac).pat;
                let pat_matches = if patlen != 0 {
                    (*ap).patlen == patlen
                        && std::slice::from_raw_parts(pat.cast::<u8>(), patlen as usize)
                            == std::slice::from_raw_parts((*ap).pat.cast::<u8>(), patlen as usize)
                } else {
                    true
                };
                if (group != AUGROUP_ALL && (*ap).group != group) || !pat_matches {
                    continue;
                }

                if (*ap).group != last_group {
                    last_group = (*ap).group;
                    last_group_name = augroup_name((*ap).group);

                    if got_int {
                        return;
                    }

                    msg_putchar(b'\n' as c_int);
                    if got_int {
                        return;
                    }

                    if (*ap).group != AUGROUP_DEFAULT {
                        if last_group_name.is_null() {
                            msg_puts_hl(get_deleted_augroup(), HLF_E, false);
                        } else {
                            msg_puts_hl(last_group_name, HLF_T, false);
                        }
                        msg_puts(c"  ".as_ptr());
                    }
                    msg_puts_hl(event_nr2name(event), HLF_T, false);
                }

                if last_ap != ap {
                    last_ap = ap;

                    msg_putchar(b'\n' as c_int);
                    if got_int {
                        return;
                    }

                    msg_advance(4);
                    msg_outtrans((*ap).pat, 0, false);
                }

                if got_int {
                    return;
                }

                if msg_col >= 14 {
                    msg_putchar(b'\n' as c_int);
                }
                msg_advance(14);
                if got_int {
                    return;
                }

                let handler_str = aucmd_handler_to_string(ac);
                if !(*ac).desc.is_null() {
                    let msglen = 100usize;
                    let mbuf = xmallocz(msglen).cast::<c_char>();
                    let desc_str = CStr::from_ptr((*ac).desc).to_string_lossy();
                    let formatted = if !(*ac).handler_cmd.is_null() {
                        let h_str = CStr::from_ptr(handler_str).to_string_lossy();
                        format!("{h_str} [{desc_str}]")
                    } else {
                        msg_puts_hl(handler_str, HLF_8, false);
                        format!(" [{desc_str}]")
                    };
                    let copy_len = formatted.len().min(msglen - 1);
                    ptr::copy_nonoverlapping(formatted.as_ptr().cast::<c_char>(), mbuf, copy_len);
                    *mbuf.add(copy_len) = NUL;
                    msg_outtrans(mbuf, 0, false);
                    xfree(mbuf.cast());
                } else if !(*ac).handler_cmd.is_null() {
                    msg_outtrans(handler_str, 0, false);
                } else {
                    msg_puts_hl(handler_str, HLF_8, false);
                }
                xfree(handler_str.cast());
                if p_verbose > 0 {
                    last_set_msg((*ac).script_ctx);
                }

                if got_int {
                    return;
                }
            }

            patlen = aucmd_span_pattern(endpat, &raw mut pat) as c_int;
            if patlen == 0 {
                break;
            }
        }
    }
}

unsafe fn aucmd_del(ac: *mut AutoCmd) {
    unsafe {
        if !(*ac).pat.is_null() {
            (*(*ac).pat).refcount -= 1;
            if (*(*ac).pat).refcount == 0 {
                xfree((*(*ac).pat).pat.cast());
                (*(*ac).pat).pat = ptr::null_mut();
                vim_regfree((*(*ac).pat).reg_prog);
                xfree((*ac).pat.cast());
            }
        }
        (*ac).pat = ptr::null_mut();
        if !(*ac).handler_cmd.is_null() {
            xfree((*ac).handler_cmd.cast());
            (*ac).handler_cmd = ptr::null_mut();
        } else {
            callback_free(&raw mut (*ac).handler_fn);
        }
        xfree((*ac).desc.cast());
        (*ac).desc = ptr::null_mut();

        au_need_clean = true;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_del_for_event_and_group(event: event_T, group: c_int) {
    unsafe {
        let acs = &raw mut autocmds[event as usize];
        for i in 0..(*acs).size {
            let ac = (*acs).items.add(i);
            if !(*ac).pat.is_null() && (*(*ac).pat).group == group {
                aucmd_del(ac);
            }
        }
        au_cleanup();
    }
}

unsafe fn au_cleanup() {
    unsafe {
        if autocmd_busy || !au_need_clean {
            return;
        }

        for event in 0..NUM_EVENTS {
            let acs = (&raw mut autocmds).cast::<AutoCmdVec>().add(event);
            let mut nsize = 0usize;
            for i in 0..(*acs).size {
                let ac = (*acs).items.add(i);
                if nsize != i {
                    ptr::copy_nonoverlapping(ac, (*acs).items.add(nsize), 1);
                }
                if !(*ac).pat.is_null() {
                    nsize += 1;
                }
            }
            if nsize == 0 {
                kv_destroy_aucmd(&mut *acs);
            } else {
                (*acs).size = nsize;
            }
        }

        au_need_clean = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn au_get_autocmds_for_event(event: event_T) -> *mut AutoCmdVec {
    unsafe { &raw mut autocmds[event as usize] }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aubuflocal_remove(buf: *mut buf_T) {
    unsafe {
        let mut apc = active_apc_list;
        while !apc.is_null() {
            if (*buf).b_fnum == (*apc).arg_bufnr {
                (*apc).arg_bufnr = 0;
            }
            apc = (*apc).next;
        }

        for event in 0..NUM_EVENTS as event_T {
            let acs = &raw mut autocmds[event as usize];
            for i in 0..(*acs).size {
                let ac = (*acs).items.add(i);
                if (*ac).pat.is_null() || (*(*ac).pat).buflocal_nr != (*buf).b_fnum {
                    continue;
                }

                aucmd_del(ac);

                if p_verbose >= 6 {
                    verbose_enter();
                    smsg!(
                        0,
                        tr(c"auto-removing autocommand: %s <buffer=%d>"),
                        event_nr2name(event),
                        (*buf).b_fnum,
                    );
                    verbose_leave();
                }
            }
        }
        au_cleanup();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_add(name: *const c_char) -> c_int {
    unsafe {
        assert!(vim_stricmp(name, c"end".as_ptr()) != 0);

        let existing_id = augroup_find(name);
        if existing_id > 0 {
            assert!(existing_id != AUGROUP_DELETED);
            return existing_id;
        }

        if existing_id == AUGROUP_DELETED {
            augroup_map_del(existing_id, name);
        }

        let next_id = next_augroup_id;
        next_augroup_id += 1;
        let name_key = cstr_to_string(name);
        let name_val = cstr_to_string(name);
        map_put_name_to_id(name_key, next_id);
        map_put_id_to_name(next_id, name_val);

        next_id
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_del_by_id(id: c_int) {
    unsafe {
        let name = if id <= 0 {
            ptr::null_mut()
        } else {
            augroup_name(id)
        };
        if name.is_null() {
            semsg!(tr(c"E367: No such group id: %d"), id);
        } else {
            augroup_del(name, id, false);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_del(
    name: *mut c_char,
    id_for_error: c_int,
    stupid_legacy_mode: bool,
) {
    unsafe {
        let group = augroup_find(name);
        if group == AUGROUP_ERROR {
            if id_for_error > 0 {
                semsg!(tr(c"E367: No such group id: %d \"%s\""), id_for_error, name);
            } else {
                semsg!(tr(c"E367: No such group: \"%s\""), name);
            }
            return;
        } else if group == current_augroup {
            emsg(tr(c"E936: Cannot delete the current group"));
            return;
        }

        if stupid_legacy_mode {
            for event in 0..NUM_EVENTS {
                let acs = (&raw mut autocmds).cast::<AutoCmdVec>().add(event);
                for i in 0..(*acs).size {
                    let ap = (*(*acs).items.add(i)).pat;
                    if !ap.is_null() && (*ap).group == group {
                        give_warning(
                            tr(c"W19: Deleting augroup that is still in use"),
                            true,
                            true,
                        );
                        map_put_name_to_id(cstr_as_string(name), AUGROUP_DELETED);
                        augroup_map_del((*ap).group, ptr::null());
                        return;
                    }
                }
            }
        } else {
            for event in 0..NUM_EVENTS {
                let acs = (&raw mut autocmds).cast::<AutoCmdVec>().add(event);
                for i in 0..(*acs).size {
                    let ac = (*acs).items.add(i);
                    if !(*ac).pat.is_null() && (*(*ac).pat).group == group {
                        aucmd_del(ac);
                    }
                }
            }
        }

        augroup_map_del(group, name);
        au_cleanup();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_find(name: *const c_char) -> c_int {
    unsafe {
        let existing_id = map_get_name_to_id(cstr_as_string(name));
        if existing_id == AUGROUP_DELETED || existing_id > 0 {
            existing_id
        } else {
            AUGROUP_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_name(mut group: c_int) -> *mut c_char {
    unsafe {
        assert!(group != 0);

        if group == AUGROUP_DELETED {
            return get_deleted_augroup().cast_mut();
        }

        if group == AUGROUP_ALL {
            group = current_augroup;
        }

        if group == next_augroup_id {
            return c"END".as_ptr().cast_mut();
        }

        if group > next_augroup_id {
            return ptr::null_mut();
        }

        let key = map_get_id_to_name(group);
        if !key.data.is_null() {
            return key.data;
        }

        get_deleted_augroup().cast_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn augroup_exists(name: *const c_char) -> bool {
    unsafe { augroup_find(name) > 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_augroup(arg: *mut c_char, del_group: bool) {
    unsafe {
        if del_group {
            if *arg == NUL {
                emsg(tr(CStr::from_ptr((&raw const e_argreq).cast())));
            } else {
                augroup_del(arg, -1, true);
            }
        } else if vim_stricmp(arg, c"end".as_ptr()) == 0 {
            current_augroup = AUGROUP_DEFAULT;
        } else if *arg != NUL {
            current_augroup = augroup_add(arg);
        } else {
            msg_start();
            msg_ext_set_kind(c"list_cmd".as_ptr());

            for k in 0..map_augroup_name_to_id.set.h.n_keys as usize {
                let name = *map_augroup_name_to_id.set.keys.add(k);
                let value = *map_augroup_name_to_id.values.add(k);
                if value > 0 {
                    msg_puts(name.data);
                } else {
                    msg_puts(augroup_name(value));
                }
                msg_puts(c"  ".as_ptr());
            }

            msg_clr_eos();
            msg_end();
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_all_autocmds() {
    unsafe {
        for event in 0..NUM_EVENTS {
            let acs = (&raw mut autocmds).cast::<AutoCmdVec>().add(event);
            for i in 0..(*acs).size {
                aucmd_del((*acs).items.add(i));
            }
            kv_destroy_aucmd(&mut *acs);
            au_need_clean = false;
        }

        for k in 0..map_augroup_name_to_id.set.h.n_keys as usize {
            let name = *map_augroup_name_to_id.set.keys.add(k);
            api_free_string(name);
        }
        xfree(map_augroup_name_to_id.set.h.hash.cast());
        xfree(map_augroup_name_to_id.set.keys.cast());
        xfree(map_augroup_name_to_id.values.cast());
        map_augroup_name_to_id = EMPTY_NAME_TO_ID_MAP;

        for k in 0..map_augroup_id_to_name.set.h.n_keys as usize {
            let name = *map_augroup_id_to_name.values.add(k);
            api_free_string(name);
        }
        xfree(map_augroup_id_to_name.set.h.hash.cast());
        xfree(map_augroup_id_to_name.set.keys.cast());
        xfree(map_augroup_id_to_name.values.cast());
        map_augroup_id_to_name = EMPTY_ID_TO_NAME_MAP;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn event_name2nr(start: *const c_char, end: *mut *mut c_char) -> event_T {
    unsafe {
        let mut p = start;
        while *p != NUL && !ascii_iswhite(*p) && *p != b',' as c_char && *p != b'|' as c_char {
            p = p.add(1);
        }

        let hash_idx = event_name2nr_hash(start, p.offset_from(start) as usize);
        if *p == b',' as c_char {
            p = p.add(1);
        }
        *end = p.cast_mut();
        if hash_idx < 0 {
            return NUM_EVENTS as event_T;
        }
        let ev = EVENT_HASH[hash_idx as usize] as usize;
        EVENT_NAMES[ev].event.abs() as event_T
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn event_name2nr_str(str_: NvimString) -> event_T {
    let hash_idx = event_name2nr_hash(str_.data, str_.size);
    if hash_idx < 0 {
        return NUM_EVENTS as event_T;
    }
    let ev = EVENT_HASH[hash_idx as usize] as usize;
    EVENT_NAMES[ev].event.abs() as event_T
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn event_nr2name(event: event_T) -> *const c_char {
    if event >= 0 && (event as usize) < NUM_EVENTS {
        EVENT_NAMES[event as usize].name.as_ptr()
    } else {
        c"Unknown".as_ptr()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn event_ignored(event: event_T, mut ei: *mut c_char) -> bool {
    unsafe {
        let mut ignored = false;
        while *ei != NUL {
            let unignore = *ei == b'-' as c_char;
            if unignore {
                ei = ei.add(1);
            }
            if vim_strnicmp(ei, c"all".as_ptr(), 3) == 0
                && (*ei.add(3) == NUL || *ei.add(3) == b',' as c_char)
            {
                ignored = ptr::eq(ei.cast_const(), p_ei.cast_const())
                    || EVENT_NAMES[event as usize].event <= 0;
                ei = ei.add(3 + usize::from(*ei.add(3) == b',' as c_char));
            } else if event_name2nr(ei, &raw mut ei) == event {
                if unignore {
                    return false;
                }
                ignored = true;
            }
        }
        ignored
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn check_ei(mut ei: *mut c_char, win: bool) -> c_int {
    unsafe {
        while *ei != NUL {
            if vim_strnicmp(ei, c"all".as_ptr(), 3) == 0
                && (*ei.add(3) == NUL || *ei.add(3) == b',' as c_char)
            {
                ei = ei.add(3 + usize::from(*ei.add(3) == b',' as c_char));
            } else {
                if *ei == b'-' as c_char {
                    ei = ei.add(1);
                }
                let event = event_name2nr(ei, &raw mut ei);
                if event == NUM_EVENTS as event_T || (win && EVENT_NAMES[event as usize].event > 0)
                {
                    return FAIL;
                }
            }
        }
        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn au_event_disable(what: *mut c_char) -> *mut c_char {
    unsafe {
        let p_ei_len = CStr::from_ptr(p_ei).to_bytes().len();
        let what_len = CStr::from_ptr(what).to_bytes().len();
        let save_ei = xmemdupz(p_ei.cast(), p_ei_len).cast::<c_char>();
        let new_ei = xstrnsave(p_ei, p_ei_len + what_len);
        if *what == b',' as c_char && *p_ei == NUL {
            ptr::copy_nonoverlapping(what.add(1), new_ei, what_len);
        } else {
            ptr::copy_nonoverlapping(what, new_ei.add(p_ei_len), what_len + 1);
        }
        set_option_direct(kOptEventignore, cstr_as_obj(new_ei), 0, SID_NONE);
        xfree(new_ei.cast());
        save_ei
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn au_event_restore(old_ei: *mut c_char) {
    unsafe {
        if !old_ei.is_null() {
            set_option_direct(kOptEventignore, cstr_as_obj(old_ei), 0, SID_NONE);
            xfree(old_ei.cast());
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_autocmd(eap: *mut exarg_T, arg_in: *mut c_char, forceit: c_int) {
    unsafe {
        let mut arg = arg_in;
        let mut envpat: *mut c_char = ptr::null_mut();
        let mut cmd: *mut c_char;
        let mut cmd_need_free = false;
        let mut nested = false;
        let mut once = false;
        let group = if *arg == b'|' as c_char {
            (*eap).nextcmd = arg.add(1);
            arg = c"".as_ptr().cast_mut();
            AUGROUP_ALL
        } else {
            arg_augroup_get(&raw mut arg)
        };

        let mut pat = arg_event_skip(arg, group != AUGROUP_ALL);
        if pat.is_null() {
            return;
        }

        pat = skipwhite(pat);
        'run: {
            if *pat == b'|' as c_char {
                (*eap).nextcmd = pat.add(1);
                pat = c"".as_ptr().cast_mut();
                cmd = c"".as_ptr().cast_mut();
            } else {
                cmd = pat;
                while *cmd != NUL && (!ascii_iswhite(*cmd) || *cmd.sub(1) == b'\\' as c_char) {
                    cmd = cmd.add(1);
                }
                if *cmd != NUL {
                    *cmd = NUL;
                    cmd = cmd.add(1);
                }

                if !vim_strchr(pat, b'$' as c_int).is_null()
                    || !vim_strchr(pat, b'~' as c_int).is_null()
                {
                    envpat = expand_env_save(pat);
                    if !envpat.is_null() {
                        pat = envpat;
                    }
                }

                cmd = skipwhite(cmd);

                let mut invalid_flags = false;
                for _ in 0..2 {
                    if *cmd == NUL {
                        continue;
                    }
                    invalid_flags |=
                        arg_autocmd_flag_get(&raw mut once, &raw mut cmd, c"++once".as_ptr(), 6);
                    invalid_flags |= arg_autocmd_flag_get(
                        &raw mut nested,
                        &raw mut cmd,
                        c"++nested".as_ptr(),
                        8,
                    );
                    invalid_flags |=
                        arg_autocmd_flag_get(&raw mut nested, &raw mut cmd, c"nested".as_ptr(), 6);
                }

                if invalid_flags {
                    break 'run;
                }

                if *cmd != NUL {
                    cmd = expand_sfile(cmd);
                    if cmd.is_null() {
                        break 'run;
                    }
                    cmd_need_free = true;
                }
            }

            let is_showing = forceit == 0 && *cmd == NUL;

            if is_showing {
                msg_ext_set_kind(c"list_cmd".as_ptr());
                msg_puts_title(tr(c"\n--- Autocommands ---"));

                if *arg == b'*' as c_char || *arg == b'|' as c_char || *arg == NUL {
                    au_show_for_all_events(group, pat);
                } else {
                    let event = event_name2nr(arg, &raw mut arg);
                    assert!((event as usize) < NUM_EVENTS);
                    au_show_for_event(group, event, pat);
                }
            } else if *arg == b'*' as c_char || *arg == NUL || *arg == b'|' as c_char {
                if *cmd != NUL {
                    emsg(tr(CStr::from_ptr(
                        (&raw const e_cannot_define_autocommands_for_all_events).cast(),
                    )));
                } else {
                    do_all_autocmd_events(pat, once, c_int::from(nested), cmd, forceit != 0, group);
                }
            } else {
                while *arg != NUL && *arg != b'|' as c_char && !ascii_iswhite(*arg) {
                    let event = event_name2nr(arg, &raw mut arg);
                    assert!((event as usize) < NUM_EVENTS);
                    if do_autocmd_event(
                        event,
                        pat,
                        once,
                        c_int::from(nested),
                        cmd,
                        forceit != 0,
                        group,
                    ) == FAIL
                    {
                        break;
                    }
                }
            }
        }

        if cmd_need_free {
            xfree(cmd.cast());
        }
        xfree(envpat.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_all_autocmd_events(
    pat: *const c_char,
    once: bool,
    nested: c_int,
    cmd: *mut c_char,
    del: bool,
    group: c_int,
) {
    unsafe {
        for event in 0..NUM_EVENTS as event_T {
            if do_autocmd_event(event, pat, once, nested, cmd, del, group) == FAIL {
                return;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_autocmd_event(
    event: event_T,
    mut pat: *const c_char,
    once: bool,
    nested: c_int,
    cmd: *const c_char,
    del: bool,
    group: c_int,
) -> c_int {
    unsafe {
        assert!(*pat != NUL || del);

        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];
        let is_adding_cmd = *cmd != NUL;
        let findgroup = if group == AUGROUP_ALL {
            current_augroup
        } else {
            group
        };

        if *pat == NUL && del {
            aucmd_del_for_event_and_group(event, findgroup);
            return OK;
        }

        let mut patlen = aucmd_span_pattern(pat, &raw mut pat) as c_int;
        while patlen != 0 {
            let endpat = pat.add(patlen as usize);

            let is_buflocal = aupat_is_buflocal(pat, patlen);
            if is_buflocal {
                let buflocal_nr = aupat_get_buflocal_nr(pat, patlen);
                if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_null() {
                    semsg!(
                        tr(CStr::from_ptr(
                            (&raw const e_buffer_nr_invalid_buffer_number).cast(),
                        )),
                        buflocal_nr,
                    );
                    return FAIL;
                }
                aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, buflocal_nr);
                pat = buflocal_pat.as_ptr();
                patlen = CStr::from_ptr(pat).to_bytes().len() as c_int;
            }

            if del {
                assert!(*pat != NUL);

                let acs = &raw mut autocmds[event as usize];
                for i in 0..(*acs).size {
                    let ac = (*acs).items.add(i);
                    let ap = (*ac).pat;
                    if !ap.is_null()
                        && (*ap).group == findgroup
                        && (*ap).patlen == patlen
                        && std::slice::from_raw_parts(pat.cast::<u8>(), patlen as usize)
                            == std::slice::from_raw_parts((*ap).pat.cast::<u8>(), patlen as usize)
                    {
                        aucmd_del(ac);
                    }
                }
            }

            if is_adding_cmd {
                let mut handler_fn = Callback {
                    data: ptr::null_mut(),
                    cb_type: 0,
                };
                autocmd_register(
                    0,
                    event,
                    pat,
                    patlen,
                    group,
                    once,
                    nested != 0,
                    ptr::null_mut(),
                    cmd,
                    &raw mut handler_fn,
                );
            }

            patlen = aucmd_span_pattern(endpat, &raw mut pat) as c_int;
        }

        au_cleanup();
        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_register(
    id: i64,
    event: event_T,
    mut pat: *const c_char,
    mut patlen: c_int,
    group: c_int,
    once: bool,
    nested: bool,
    desc: *mut c_char,
    handler_cmd: *const c_char,
    handler_fn: *mut Callback,
) -> c_int {
    unsafe {
        assert!(group != 0);

        if (patlen as usize) > CStr::from_ptr(pat).to_bytes().len() {
            return FAIL;
        }

        let findgroup = if group == AUGROUP_ALL {
            current_augroup
        } else {
            group
        };

        let is_buflocal = aupat_is_buflocal(pat, patlen);
        let mut buflocal_nr: c_int = 0;
        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];

        if is_buflocal {
            buflocal_nr = aupat_get_buflocal_nr(pat, patlen);
            if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_null() {
                semsg!(
                    tr(CStr::from_ptr(
                        (&raw const e_buffer_nr_invalid_buffer_number).cast(),
                    )),
                    buflocal_nr,
                );
                return FAIL;
            }
            aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, buflocal_nr);
            pat = buflocal_pat.as_ptr();
            patlen = CStr::from_ptr(pat).to_bytes().len() as c_int;
        }

        let mut ap: *mut AutoPat = ptr::null_mut();
        let acs = &raw mut autocmds[event as usize];
        let mut i = (*acs).size as isize - 1;
        while i >= 0 {
            ap = (*(*acs).items.add(i as usize)).pat;
            if ap.is_null() {
                i -= 1;
                continue;
            }
            if (*ap).group != findgroup
                || (*ap).patlen != patlen
                || std::slice::from_raw_parts(pat.cast::<u8>(), patlen as usize)
                    != std::slice::from_raw_parts((*ap).pat.cast::<u8>(), patlen as usize)
            {
                ap = ptr::null_mut();
            }
            break;
        }

        if ap.is_null() {
            ap = xmalloc(size_of::<AutoPat>()).cast::<AutoPat>();
            if is_buflocal {
                (*ap).buflocal_nr = buflocal_nr;
                (*ap).reg_prog = ptr::null_mut();
            } else {
                (*ap).buflocal_nr = 0;
                let reg_pat = file_pat_to_reg_pat(
                    pat,
                    pat.add(patlen as usize),
                    &raw mut (*ap).allow_dirs,
                    1,
                );
                if !reg_pat.is_null() {
                    (*ap).reg_prog = vim_regcomp(reg_pat, RE_MAGIC | RE_NOBREAK);
                } else {
                    (*ap).reg_prog = ptr::null_mut();
                }
                xfree(reg_pat.cast());
                if reg_pat.is_null() || (*ap).reg_prog.is_null() {
                    xfree(ap.cast());
                    return FAIL;
                }
            }

            (*ap).refcount = 0;
            (*ap).pat = xmemdupz(pat.cast(), patlen as usize).cast::<c_char>();
            (*ap).patlen = patlen;

            if event == EVENT_MODECHANGED && !has_event(EVENT_MODECHANGED) {
                get_mode((&raw mut last_mode).cast::<c_char>());
            }

            if (event == EVENT_CURSORMOVED && !has_event(EVENT_CURSORMOVED))
                || (event == EVENT_CURSORMOVEDI && !has_event(EVENT_CURSORMOVEDI))
            {
                last_cursormoved_win = curwin;
                last_cursormoved = (*curwin).w_cursor;
            }

            if (event == EVENT_WINSCROLLED || event == EVENT_WINRESIZED)
                && !(has_event(EVENT_WINSCROLLED) || has_event(EVENT_WINRESIZED))
            {
                let save_curtab = curtab;
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    unuse_tabpage(curtab);
                    use_tabpage(tp);
                    snapshot_windows_scroll_size();
                    tp = (*tp).tp_next;
                }
                unuse_tabpage(curtab);
                use_tabpage(save_curtab);
            }

            (*ap).group = findgroup;
        }

        (*ap).refcount += 1;

        let ac = kv_pushp_aucmd(&mut autocmds[event as usize]);
        (*ac).pat = ap;
        (*ac).id = id;
        if !handler_cmd.is_null() {
            (*ac).handler_cmd = xstrdup(handler_cmd);
        } else {
            (*ac).handler_cmd = ptr::null_mut();
            callback_copy(&raw mut (*ac).handler_fn, handler_fn);
        }
        (*ac).script_ctx = current_sctx;
        let sourcing_lnum = if !exestack.ga_data.is_null() && exestack.ga_len > 0 {
            (*exestack
                .ga_data
                .cast::<estack_T>()
                .add((exestack.ga_len - 1) as usize))
            .es_lnum
        } else {
            0
        };
        (*ac).script_ctx.sc_lnum += sourcing_lnum;
        nlua_set_sctx(&raw mut (*ac).script_ctx);
        (*ac).once = once;
        (*ac).nested = nested;
        (*ac).desc = if desc.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(desc)
        };

        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_span_pattern(
    mut pat: *const c_char,
    start: *mut *const c_char,
) -> usize {
    unsafe {
        while *pat == b',' as c_char {
            pat = pat.add(1);
        }

        let mut p = pat;
        let mut brace_level: c_int = 0;
        while *p != NUL
            && (*p != b',' as c_char
                || brace_level != 0
                || (p > pat && *p.sub(1) == b'\\' as c_char))
        {
            if *p == b'{' as c_char {
                brace_level += 1;
            } else if *p == b'}' as c_char {
                brace_level -= 1;
            }
            p = p.add(1);
        }

        *start = pat;
        p.offset_from(pat) as usize
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_doautocmd(
    arg_start: *mut c_char,
    do_msg: bool,
    did_something: *mut bool,
) -> c_int {
    unsafe {
        let mut arg = arg_start;
        let mut nothing_done = true;

        if !did_something.is_null() {
            *did_something = false;
        }

        let group = arg_augroup_get(&raw mut arg);

        if *arg == b'*' as c_char {
            emsg(tr(c"E217: Can't execute autocommands for ALL events"));
            return FAIL;
        }

        let mut fname = arg_event_skip(arg, group != AUGROUP_ALL);
        if fname.is_null() {
            return FAIL;
        }

        fname = skipwhite(fname);

        while *arg != NUL && ends_excmd(c_int::from(*arg.cast::<u8>())) == 0 && !ascii_iswhite(*arg)
        {
            let ev = event_name2nr(arg, &raw mut arg);
            if apply_autocmds_group(
                ev,
                fname,
                ptr::null_mut(),
                true,
                group,
                curbuf,
                curwin,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
            ) {
                nothing_done = false;
            }
        }

        if nothing_done && do_msg && !aborting() {
            smsg!(0, tr(c"No matching autocommands: %s"), arg_start);
        }
        if !did_something.is_null() {
            *did_something = !nothing_done;
        }

        if aborting() { FAIL } else { OK }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_doautoall(eap: *mut exarg_T) {
    unsafe {
        let mut retval = OK;
        let mut aco: CtxSwitch = std::mem::zeroed();
        let mut arg = (*eap).arg;
        let call_do_modelines = check_nomodeline(&raw mut arg);
        let mut bufref: bufref_T = std::mem::zeroed();
        let mut did_aucmd = false;

        let mut buf = firstbuf;
        while !buf.is_null() {
            if (*buf).b_ml.ml_mfp.is_null() || buf == curbuf {
                buf = (*buf).b_next;
                continue;
            }

            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            set_bufref(&raw mut bufref, buf);

            retval = do_doautocmd(arg, false, &raw mut did_aucmd);

            if call_do_modelines && did_aucmd {
                do_modelines(if is_ctx_win(curwin) { OPT_NOWIN } else { 0 });
            }

            ctx_restore(&raw mut aco);

            if retval == FAIL || !bufref_valid(&raw mut bufref) {
                retval = FAIL;
                break;
            }
            buf = (*buf).b_next;
        }

        if retval == OK {
            do_doautocmd(arg, false, &raw mut did_aucmd);
            if call_do_modelines && did_aucmd {
                do_modelines(0);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn check_nomodeline(argp: *mut *mut c_char) -> bool {
    unsafe {
        let bytes = CStr::from_ptr(*argp).to_bytes();
        if bytes.starts_with(b"<nomodeline>") {
            *argp = skipwhite((*argp).add(12));
            return false;
        }
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_defer(
    event: event_T,
    fname: *mut c_char,
    fname_io: *mut c_char,
    group: c_int,
    buf: *mut buf_T,
    eap: *mut exarg_T,
    data: *mut Object,
) {
    unsafe {
        let evdata = xmalloc(size_of::<AutoCmdEvent>()).cast::<AutoCmdEvent>();
        (*evdata).event = event;
        (*evdata).fname = if !fname.is_null() {
            xstrdup(fname)
        } else {
            ptr::null_mut()
        };
        (*evdata).fname_io = if !fname_io.is_null() {
            xstrdup(fname_io)
        } else {
            ptr::null_mut()
        };
        (*evdata).group = group;
        (*evdata).buf = (*buf).b_fnum;
        (*evdata).eap = eap;
        if !data.is_null() {
            (*evdata).data = xmalloc(size_of::<Object>()).cast::<Object>();
            *(*evdata).data = copy_object(*data, ptr::null_mut());
        } else {
            (*evdata).data = ptr::null_mut();
        }

        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = evdata.cast();
        multiqueue_put_event(
            deferred_events,
            Event {
                handler: Some(deferred_event),
                argv,
            },
        );
    }
}

unsafe extern "C" fn deferred_event(argv: *mut *mut c_void) {
    unsafe {
        let e = (*argv).cast::<AutoCmdEvent>();
        let event = (*e).event;
        let fname = (*e).fname;
        let fname_io = (*e).fname_io;
        let group = (*e).group;
        let eap = (*e).eap;
        let data = (*e).data;

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let buf = find_buffer_by_handle((*e).buf, &raw mut err);
        if !buf.is_null() {
            let mut save_v_event: save_v_event_T = std::mem::zeroed();
            let v_event = get_v_event(&raw mut save_v_event);
            if !data.is_null() && (*data).obj_type == ObjectType::kObjectTypeDict {
                for i in 0..(*data).data.dict.size {
                    let item = *(*data).data.dict.items.add(i);
                    let mut tv: typval_T = std::mem::zeroed();
                    object_to_vim(item.value, &raw mut tv, &raw mut err);
                    if err.err_type != ErrorType::kErrorTypeNone {
                        api_clear_error(&raw mut err);
                        continue;
                    }
                    tv_dict_add_tv(v_event, item.key.data, item.key.size, &raw mut tv);
                    tv_clear(&raw mut tv);
                }
            }
            tv_dict_set_keys_readonly(v_event);

            let win = curwin;
            let mut aco: CtxSwitch = std::mem::zeroed();
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            apply_autocmds_group(
                event, fname, fname_io, false, group, buf, win, eap, data, false,
            );
            ctx_restore(&raw mut aco);

            restore_v_event(v_event, &raw mut save_v_event);
        }

        xfree(fname.cast());
        xfree(fname_io.cast());
        if !data.is_null() {
            api_free_object(*data);
            xfree(data.cast());
        }
        xfree(e.cast());
    }
}

unsafe extern "C" fn deferred_optionset_modified(argv: *mut *mut c_void) {
    unsafe {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let buf = find_buffer_by_handle((*argv as usize) as handle_T, &raw mut err);
        api_clear_error(&raw mut err);
        if !buf.is_null() {
            let new_val = (*argv.add(1) as usize) != 0;
            let old = boolean_obj(!new_val);
            let new = boolean_obj(new_val);
            let mut aco: CtxSwitch = std::mem::zeroed();
            ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            apply_optionset_autocmd_now(kOptModified, OPT_LOCAL, old, old, old, new);
            ctx_restore(&raw mut aco);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_defer_modified(buf: *mut buf_T, new_val: bool) {
    unsafe {
        if !has_event(EVENT_OPTIONSET)
            || (bt_nofile(buf) && (*buf).b_p_bl == 0)
            || *get_vim_var_str(VV_OPTION_TYPE) != NUL
        {
            return;
        }
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = ((*buf).b_fnum as usize) as *mut c_void;
        argv[1] = usize::from(new_val) as *mut c_void;
        multiqueue_put_event(
            deferred_events,
            Event {
                handler: Some(deferred_optionset_modified),
                argv,
            },
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_autocmds(
    event: event_T,
    fname: *mut c_char,
    fname_io: *mut c_char,
    force: bool,
    buf: *mut buf_T,
) -> bool {
    unsafe {
        apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            curwin,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_autocmds_win(
    event: event_T,
    fname: *mut c_char,
    fname_io: *mut c_char,
    force: bool,
    buf: *mut buf_T,
    win: *mut win_T,
) -> bool {
    unsafe {
        apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            win,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_autocmds_exarg(
    event: event_T,
    fname: *mut c_char,
    fname_io: *mut c_char,
    force: bool,
    buf: *mut buf_T,
    eap: *mut exarg_T,
) -> bool {
    unsafe {
        apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            curwin,
            eap,
            ptr::null_mut(),
            false,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_autocmds_retval(
    event: event_T,
    fname: *mut c_char,
    fname_io: *mut c_char,
    force: bool,
    buf: *mut buf_T,
    retval: *mut c_int,
) -> bool {
    unsafe {
        if should_abort(*retval) {
            return false;
        }

        let did_cmd = apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            curwin,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
        );
        if did_cmd && aborting() {
            *retval = FAIL;
        }
        did_cmd
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn has_event(event: event_T) -> bool {
    unsafe { autocmds[event as usize].size != 0 }
}

unsafe fn has_cursorhold() -> bool {
    unsafe {
        has_event(if get_real_state() == MODE_NORMAL_BUSY {
            EVENT_CURSORHOLD
        } else {
            EVENT_CURSORHOLDI
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trigger_cursorhold() -> bool {
    unsafe {
        if !did_cursorhold
            && has_cursorhold()
            && reg_recording == 0
            && typebuf.tb_len == 0
            && !ins_compl_active()
        {
            let state = get_real_state();
            if state == MODE_NORMAL_BUSY || (state & MODE_INSERT) != 0 {
                return true;
            }
        }
        false
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_autocmds_group(
    event: event_T,
    mut fname: *mut c_char,
    fname_io: *mut c_char,
    force: bool,
    group: c_int,
    buf: *mut buf_T,
    win: *mut win_T,
    eap: *mut exarg_T,
    data: *mut Object,
    with_buf: bool,
) -> bool {
    unsafe {
        let mut sfname: *mut c_char = ptr::null_mut();
        let mut retval = false;
        let mut wait_time: proftime_T = 0;
        let mut did_save_redobuff = false;
        let mut save_redo: RedoState = std::mem::zeroed();
        let save_KeyTyped = KeyTyped;
        let mut aco: CtxSwitch = std::mem::zeroed();

        'bypass_au: {
            if event == NUM_EVENTS as event_T
                || autocmds[event as usize].size == 0
                || is_autocmd_blocked()
            {
                break 'bypass_au;
            }

            if autocmd_busy && !(force || autocmd_nested) {
                break 'bypass_au;
            }

            if aborting() {
                break 'bypass_au;
            }

            if filechangeshell_busy
                && (event == EVENT_FILECHANGEDSHELL || event == EVENT_FILECHANGEDSHELLPOST)
            {
                break 'bypass_au;
            }

            if event_ignored(event, p_ei) {
                break 'bypass_au;
            }

            let mut win_ignore = false;
            if buf == curbuf && EVENT_NAMES[event as usize].event <= 0 {
                win_ignore = event_ignored(event, (*curwin).w_onebuf_opt.wo_eiw);
            } else if !buf.is_null()
                && EVENT_NAMES[event as usize].event <= 0
                && (*buf).b_nwindows > 0
            {
                win_ignore = true;
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    let mut wp = if tp == curtab {
                        firstwin
                    } else {
                        (*tp).tp_firstwin
                    };
                    while !wp.is_null() {
                        if (*wp).w_buffer == buf && !event_ignored(event, (*wp).w_onebuf_opt.wo_eiw)
                        {
                            win_ignore = false;
                            break;
                        }
                        wp = (*wp).w_next;
                    }
                    tp = (*tp).tp_next;
                }
            }
            if win_ignore {
                break 'bypass_au;
            }

            if apply_autocmds_nesting == 10 {
                emsg(tr(E_AUTOCOMMAND_NESTING_TOO_DEEP));
                break 'bypass_au;
            }

            if (autocmd_no_enter != 0 && (event == EVENT_WINENTER || event == EVENT_BUFENTER))
                || (autocmd_no_leave != 0 && (event == EVENT_WINLEAVE || event == EVENT_BUFLEAVE))
            {
                break 'bypass_au;
            }

            let save_autocmd_fname = autocmd_fname;
            let save_autocmd_fname_full = autocmd_fname_full;
            let save_autocmd_bufnr = autocmd_bufnr;
            let save_autocmd_winid = autocmd_winid;
            let save_autocmd_match = autocmd_match;
            let save_autocmd_busy = autocmd_busy;
            let save_autocmd_nested = autocmd_nested;

            if fname_io.is_null() {
                if event == EVENT_COLORSCHEME
                    || event == EVENT_COLORSCHEMEPRE
                    || event == EVENT_OPTIONSET
                    || event == EVENT_MODECHANGED
                    || event == EVENT_MARKSET
                {
                    autocmd_fname = ptr::null_mut();
                } else if !fname.is_null() && ends_excmd(c_int::from(*fname.cast::<u8>())) == 0 {
                    autocmd_fname = fname;
                } else if !buf.is_null() {
                    autocmd_fname = (*buf).b_ffname;
                } else {
                    autocmd_fname = ptr::null_mut();
                }
            } else {
                autocmd_fname = fname_io;
            }
            let afile_orig = if !autocmd_fname.is_null() {
                let orig = xstrdup(autocmd_fname);
                autocmd_fname = xstrnsave(autocmd_fname, MAXPATHL);
                orig
            } else {
                ptr::null_mut()
            };
            autocmd_fname_full = false;

            autocmd_bufnr = if buf.is_null() { 0 } else { (*buf).b_fnum };

            if fname.is_null() || *fname == NUL {
                if buf.is_null() {
                    fname = ptr::null_mut();
                } else if event == EVENT_SYNTAX {
                    fname = (*buf).b_p_syn;
                } else if event == EVENT_FILETYPE {
                    fname = (*buf).b_p_ft;
                } else {
                    if !(*buf).b_sfname.is_null() {
                        sfname = xstrdup((*buf).b_sfname);
                    }
                    fname = (*buf).b_ffname;
                }
                if fname.is_null() {
                    fname = c"".as_ptr().cast_mut();
                }
                fname = xstrdup(fname);
            } else {
                sfname = to_slash_save(fname);
                if event == EVENT_CMDATOM
                    || event == EVENT_CMDLINECHANGED
                    || event == EVENT_CMDLINEENTER
                    || event == EVENT_CMDLINELEAVEPRE
                    || event == EVENT_CMDLINELEAVE
                    || event == EVENT_CMDUNDEFINED
                    || event == EVENT_CURSORMOVEDC
                    || event == EVENT_CMDWINENTER
                    || event == EVENT_CMDWINLEAVE
                    || event == EVENT_COLORSCHEME
                    || event == EVENT_COLORSCHEMEPRE
                    || event == EVENT_DIRCHANGED
                    || event == EVENT_DIRCHANGEDPRE
                    || event == EVENT_FILETYPE
                    || event == EVENT_FUNCUNDEFINED
                    || event == EVENT_MARKSET
                    || event == EVENT_MENUPOPUP
                    || event == EVENT_MODECHANGED
                    || event == EVENT_OPTIONSET
                    || event == EVENT_PROGRESS
                    || event == EVENT_QUICKFIXCMDPOST
                    || event == EVENT_QUICKFIXCMDPRE
                    || event == EVENT_REMOTEREPLY
                    || event == EVENT_SIGNAL
                    || event == EVENT_SPELLFILEMISSING
                    || event == EVENT_SYNTAX
                    || event == EVENT_TABCLOSED
                    || event == EVENT_TABMOVED
                    || event == EVENT_USER
                    || event == EVENT_WINCLOSED
                    || event == EVENT_WINRESIZED
                    || event == EVENT_WINSCROLLED
                {
                    fname = to_slash_save(fname);
                    autocmd_fname_full = true;
                } else {
                    fname = FullName_save(fname, false);
                }
            }
            if fname.is_null() {
                xfree(sfname.cast());
                retval = false;
                break 'bypass_au;
            }

            autocmd_winid = if !win.is_null() {
                (*win).handle
            } else if !curwin.is_null() {
                (*curwin).handle
            } else {
                0
            };

            if with_buf && !buf.is_null() && buf != curbuf {
                ctx_switch(&raw mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            }

            let save_changed = (*curbuf).b_changed;
            let old_curbuf = curbuf;

            autocmd_match = fname;

            RedrawingDisabled += 1;

            estack_push(ETYPE_AUCMD, ptr::null_mut(), 0);

            let save_current_sctx = current_sctx;

            if do_profiling == PROF_YES {
                prof_child_enter(&raw mut wait_time);
            }

            let mut funccal_entry: funccal_entry_T = std::mem::zeroed();
            save_funccal(&raw mut funccal_entry);

            if !autocmd_busy {
                save_search_patterns();
                if !ins_compl_active() {
                    save_redobuff(&raw mut save_redo);
                    did_save_redobuff = true;
                }
                (*curbuf).b_did_filetype = (*curbuf).b_keep_filetype;
            }

            autocmd_busy = true;
            filechangeshell_busy = event == EVENT_FILECHANGEDSHELL;
            apply_autocmds_nesting += 1;

            if event == EVENT_FILETYPE && *(*curbuf).b_p_ft != NUL {
                (*curbuf).b_did_filetype = true;
            }

            let tail = path_tail(fname);

            let mut patcmd = AutoPatCmd {
                lastpat: ptr::null_mut(),
                auidx: 0,
                ausize: autocmds[event as usize].size,
                afile_orig,
                fname,
                sfname,
                tail,
                group,
                event,
                script_ctx: sctx_T {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                },
                arg_bufnr: autocmd_bufnr,
                data: ptr::null_mut(),
                next: ptr::null_mut(),
            };
            aucmd_next(&raw mut patcmd);

            if !patcmd.lastpat.is_null() {
                patcmd.next = active_apc_list;
                active_apc_list = &raw mut patcmd;
                patcmd.data = data;

                let save_cmdbang = get_vim_var_nr(VV_CMDBANG);
                let save_cmdarg = if !eap.is_null() {
                    let s = set_cmdarg(eap, ptr::null_mut());
                    set_vim_var_nr(VV_CMDBANG, varnumber_T::from((*eap).forceit));
                    s
                } else {
                    ptr::null_mut()
                };
                retval = true;

                if apply_autocmds_nesting == 1 {
                    check_lnums(true);
                } else {
                    check_lnums_nested(true);
                }

                let save_did_emsg = did_emsg;

                do_cmdline(
                    ptr::null_mut(),
                    Some(getnextac),
                    (&raw mut patcmd).cast(),
                    DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
                );

                did_emsg += save_did_emsg;

                if apply_autocmds_nesting == 1 {
                    reset_lnums();
                }

                if !eap.is_null() {
                    set_cmdarg(ptr::null_mut(), save_cmdarg);
                    set_vim_var_nr(VV_CMDBANG, save_cmdbang);
                }
                if ptr::eq(active_apc_list, &raw const patcmd) {
                    active_apc_list = patcmd.next;
                }
            }

            RedrawingDisabled -= 1;
            autocmd_busy = save_autocmd_busy;
            filechangeshell_busy = false;
            autocmd_nested = save_autocmd_nested;
            if !exestack.ga_data.is_null() && exestack.ga_len > 0 {
                let top = exestack
                    .ga_data
                    .cast::<estack_T>()
                    .add((exestack.ga_len - 1) as usize);
                xfree((*top).es_name.cast());
            }
            estack_pop();
            xfree(afile_orig.cast());
            xfree(autocmd_fname.cast());
            autocmd_fname = save_autocmd_fname;
            autocmd_fname_full = save_autocmd_fname_full;
            autocmd_bufnr = save_autocmd_bufnr;
            autocmd_winid = save_autocmd_winid;
            autocmd_match = save_autocmd_match;
            current_sctx = save_current_sctx;
            restore_funccal();
            if do_profiling == PROF_YES {
                prof_child_exit(&raw mut wait_time);
            }
            KeyTyped = save_KeyTyped;
            xfree(fname.cast());
            xfree(sfname.cast());
            apply_autocmds_nesting -= 1;

            if !autocmd_busy {
                restore_search_patterns();
                if did_save_redobuff {
                    restore_redobuff(&raw mut save_redo);
                }
                (*curbuf).b_did_filetype = false;
                while !au_pending_free_buf.is_null() {
                    let b = (*au_pending_free_buf).b_next;
                    xfree(au_pending_free_buf.cast());
                    au_pending_free_buf = b;
                }
                while !au_pending_free_win.is_null() {
                    let w = (*au_pending_free_win).w_next;
                    xfree(au_pending_free_win.cast());
                    au_pending_free_win = w;
                }
            }

            if curbuf == old_curbuf
                && (event == EVENT_BUFREADPOST
                    || event == EVENT_BUFWRITEPOST
                    || event == EVENT_FILEAPPENDPOST
                    || event == EVENT_VIMLEAVE
                    || event == EVENT_VIMLEAVEPRE)
            {
                if (*curbuf).b_changed != save_changed {
                    need_maketitle = true;
                }
                (*curbuf).b_changed = save_changed;
            }

            au_cleanup();
        }

        if event == EVENT_BUFWIPEOUT && !buf.is_null() {
            aubuflocal_remove(buf);
        }

        if retval && event == EVENT_FILETYPE {
            (*curbuf).b_au_did_filetype = true;
        }

        ctx_restore(&raw mut aco);

        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_termresponse_autocmd(sequence: NvimString, channel_id: u64) {
    unsafe {
        let mut data_items = [
            KeyValuePair {
                key: static_cstr_as_string(c"sequence"),
                value: string_obj(sequence),
            },
            KeyValuePair {
                key: static_cstr_as_string(c"chan"),
                value: integer_obj(channel_id as Integer),
            },
        ];
        let data = Dict {
            items: data_items.as_mut_ptr(),
            size: 2,
            capacity: 2,
        };
        let mut data_obj = dict_obj(data);
        apply_autocmds_group(
            EVENT_TERMRESPONSE,
            ptr::null_mut(),
            ptr::null_mut(),
            true,
            AUGROUP_ALL,
            ptr::null_mut(),
            curwin,
            ptr::null_mut(),
            &raw mut data_obj,
            false,
        );
        termresponse_changed = true;
        termresponse_chan_id = channel_id;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn block_autocmds() {
    unsafe {
        if !is_autocmd_blocked() {
            termresponse_changed = false;
            termresponse_chan_id = 0;
        }
        autocmd_blocked += 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn unblock_autocmds() {
    unsafe {
        autocmd_blocked -= 1;

        if !is_autocmd_blocked() && termresponse_changed && has_event(EVENT_TERMRESPONSE) {
            let sequence = cstr_to_string(get_vim_var_str(VV_TERMRESPONSE));
            do_termresponse_autocmd(sequence, termresponse_chan_id);
            api_free_string(sequence);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn is_autocmd_blocked() -> bool {
    unsafe { autocmd_blocked != 0 }
}

unsafe fn aucmd_next(apc: *mut AutoPatCmd) {
    unsafe {
        let entry = exestack
            .ga_data
            .cast::<estack_T>()
            .add((exestack.ga_len - 1) as usize);

        let acs = &raw mut autocmds[(*apc).event as usize];
        assert!((*apc).ausize <= (*acs).size);
        let mut i = (*apc).auidx;
        while i < (*apc).ausize && !got_int {
            let ac = (*acs).items.add(i);
            let ap = (*ac).pat;
            if ap.is_null() {
                i += 1;
                continue;
            }
            if ap != (*apc).lastpat {
                if (*apc).group != AUGROUP_ALL && (*apc).group != (*ap).group {
                    i += 1;
                    continue;
                }
                let matches = if (*ap).buflocal_nr == 0 {
                    match_file_pat(
                        ptr::null_mut(),
                        &raw mut (*ap).reg_prog,
                        (*apc).fname,
                        (*apc).sfname,
                        (*apc).tail,
                        c_int::from((*ap).allow_dirs),
                    )
                } else {
                    (*ap).buflocal_nr == (*apc).arg_bufnr
                };
                if !matches {
                    i += 1;
                    continue;
                }

                let name = event_nr2name((*apc).event);
                let name_str = CStr::from_ptr(name).to_string_lossy();
                let pat_str = CStr::from_ptr((*ap).pat).to_string_lossy();
                let formatted = format!("{name_str} Autocommands for \"{pat_str}\"");
                let namep = xmemdupz(formatted.as_ptr().cast(), formatted.len()).cast::<c_char>();
                if p_verbose >= 8 {
                    verbose_enter();
                    smsg!(0, tr(c"Executing %s"), namep);
                    verbose_leave();
                }

                xfree((*entry).es_name.cast());
                (*entry).es_name = namep;
                (*entry).es_info = apc.cast();
            }

            (*apc).lastpat = ap;
            (*apc).auidx = i;

            line_breakcheck();
            return;
        }

        xfree((*entry).es_name.cast());
        (*entry).es_name = ptr::null_mut();
        (*entry).es_info = ptr::null_mut();

        (*apc).lastpat = ptr::null_mut();
        (*apc).auidx = usize::MAX;
    }
}

unsafe fn au_callback(ac: *const AutoCmd, apc: *const AutoPatCmd) -> bool {
    unsafe {
        let mut callback = (*ac).handler_fn;
        if callback.cb_type == kCallbackLua {
            let mut data_items = [KeyValuePair {
                key: NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                },
                value: boolean_obj(false),
            }; 8];
            let mut size = 0usize;

            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"id"),
                value: integer_obj((*ac).id),
            };
            size += 1;
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"event"),
                value: cstr_as_obj(event_nr2name((*apc).event)),
            };
            size += 1;
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"file"),
                value: cstr_as_obj((*apc).afile_orig),
            };
            size += 1;
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"match"),
                value: cstr_as_obj(autocmd_match),
            };
            size += 1;
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"buf"),
                value: integer_obj(Integer::from(autocmd_bufnr)),
            };
            size += 1;
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"win"),
                value: integer_obj(Integer::from(autocmd_winid)),
            };
            size += 1;

            if !(*apc).data.is_null() {
                data_items[size] = KeyValuePair {
                    key: static_cstr_as_string(c"data"),
                    value: *(*apc).data,
                };
                size += 1;
            }

            let group = (*(*ac).pat).group;
            match group {
                AUGROUP_ERROR => std::process::abort(),
                AUGROUP_DEFAULT | AUGROUP_ALL | AUGROUP_DELETED => {}
                _ => {
                    data_items[size] = KeyValuePair {
                        key: static_cstr_as_string(c"group"),
                        value: integer_obj(Integer::from(group)),
                    };
                    size += 1;
                }
            }

            let data = Dict {
                items: data_items.as_mut_ptr(),
                size,
                capacity: 8,
            };
            let mut args_items = [dict_obj(data)];
            let args = Array {
                items: args_items.as_mut_ptr(),
                size: 1,
                capacity: 1,
            };

            let luaref = callback.data as isize as LuaRef;
            let result = nlua_call_ref(
                luaref,
                ptr::null(),
                args,
                kRetNilBool,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            result.obj_type == ObjectType::kObjectTypeBoolean && result.data.boolean
        } else {
            let mut argsin: typval_T = std::mem::zeroed();
            let mut rettv: typval_T = std::mem::zeroed();
            callback_call(&raw mut callback, 0, &raw mut argsin, &raw mut rettv);
            false
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getnextac(
    _c: c_int,
    cookie: *mut c_void,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
        let apc = cookie.cast::<AutoPatCmd>();
        let acs = &raw mut autocmds[(*apc).event as usize];

        aucmd_next(apc);
        if (*apc).lastpat.is_null() {
            return ptr::null_mut();
        }

        assert!((*apc).auidx < (*acs).size);
        let ac = (*acs).items.add((*apc).auidx);
        assert!(!(*ac).pat.is_null());
        let mut oneshot = (*ac).once;

        if p_verbose >= 9 {
            verbose_enter_scroll();
            let handler_str = aucmd_handler_to_string(ac);
            smsg!(0, tr(c"autocommand %s"), handler_str);
            msg_puts(c"\n".as_ptr());
            xfree(handler_str.cast());
            verbose_leave_scroll();
        }

        autocmd_nested = (*ac).nested;
        current_sctx = (*ac).script_ctx;
        (*apc).script_ctx = current_sctx;

        let retval = if !(*ac).handler_cmd.is_null() {
            xstrdup((*ac).handler_cmd)
        } else {
            let ac_copy = ptr::read(ac);
            if oneshot {
                (*ac).pat = ptr::null_mut();
            }
            let rv = au_callback(&raw const ac_copy, apc);
            if oneshot {
                (*(*acs).items.add((*apc).auidx)).pat = ac_copy.pat;
            }
            oneshot = oneshot || rv;
            xcalloc(1, 1).cast::<c_char>()
        };

        if oneshot {
            aucmd_del((*acs).items.add((*apc).auidx));
        }

        if (*apc).auidx < (*apc).ausize {
            (*apc).auidx += 1;
        } else {
            (*apc).auidx = usize::MAX;
        }

        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn has_autocmd(event: event_T, sfname: *mut c_char, buf: *mut buf_T) -> bool {
    unsafe {
        if !has_event(event) {
            return false;
        }

        let tail = path_tail(sfname);
        let mut retval = false;

        let fname = FullName_save(sfname, false);
        if fname.is_null() {
            return false;
        }

        let acs = &raw mut autocmds[event as usize];
        for i in 0..(*acs).size {
            let ap = (*(*acs).items.add(i)).pat;
            if !ap.is_null() {
                let matched = if (*ap).buflocal_nr == 0 {
                    match_file_pat(
                        ptr::null_mut(),
                        &raw mut (*ap).reg_prog,
                        fname,
                        sfname,
                        tail,
                        c_int::from((*ap).allow_dirs),
                    )
                } else {
                    !buf.is_null() && (*ap).buflocal_nr == (*buf).b_fnum
                };
                if matched {
                    retval = true;
                    break;
                }
            }
        }

        xfree(fname.cast());
        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_get_augroup_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe { augroup_name(idx + 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_set_expand_context(
    xp: *mut expand_T,
    mut arg: *mut c_char,
    doautocmd: bool,
) -> *mut c_char {
    unsafe {
        let xp_pref = xp.cast::<ExpandPrefix>();
        autocmd_include_groups = false;
        let mut p = arg;
        let mut group = arg_augroup_get(&raw mut arg);

        if *arg == NUL && group != AUGROUP_ALL && !ascii_iswhite(*arg.sub(1)) {
            arg = p;
            group = AUGROUP_ALL;
        }

        p = arg;
        while *p != NUL && !ascii_iswhite(*p) {
            if *p == b',' as c_char {
                arg = p.add(1);
            }
            p = p.add(1);
        }
        if *p == NUL {
            if group == AUGROUP_ALL {
                autocmd_include_groups = true;
            }
            (*xp_pref).xp_context = EXPAND_EVENTS;
            (*xp_pref).xp_pattern = arg;
            return ptr::null_mut();
        }

        arg = skipwhite(p);
        while *arg != NUL && (!ascii_iswhite(*arg) || *arg.sub(1) == b'\\' as c_char) {
            arg = arg.add(1);
        }
        if *arg != NUL {
            arg = skipwhite(arg);
            for _ in 0..2 {
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes.starts_with(b"++once") && ascii_iswhite(*arg.add(6)) {
                    arg = skipwhite(arg.add(6));
                }
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes.starts_with(b"++nested") && ascii_iswhite(*arg.add(8)) {
                    arg = skipwhite(arg.add(8));
                }
                let bytes = CStr::from_ptr(arg).to_bytes();
                if bytes.starts_with(b"nested") && ascii_iswhite(*arg.add(6)) {
                    arg = skipwhite(arg.add(6));
                }
            }
            return arg;
        }

        (*xp_pref).xp_context = if doautocmd {
            EXPAND_FILES
        } else {
            EXPAND_NOTHING
        };
        ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_get_event_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let name = augroup_name(idx + 1);
        if !name.is_null() {
            if !autocmd_include_groups || ptr::eq(name.cast_const(), get_deleted_augroup()) {
                return c"".as_ptr().cast_mut();
            }
            return name;
        }

        let i = idx - next_augroup_id;
        if i < 0 || (i as usize) >= NUM_EVENTS {
            return ptr::null_mut();
        }

        EVENT_NAMES[i as usize].name.as_ptr().cast_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_event_name_no_group(
    _xp: *mut expand_T,
    idx: c_int,
    win: bool,
) -> *mut c_char {
    if idx < 0 || (idx as usize) >= NUM_EVENTS {
        return ptr::null_mut();
    }

    if !win {
        return EVENT_NAMES[idx as usize].name.as_ptr().cast_mut();
    }

    let mut j = 0;
    for item in &EVENT_NAMES {
        if item.event <= 0 {
            j += 1;
        }
        if j == idx + 1 {
            return item.name.as_ptr().cast_mut();
        }
    }
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_supported(event: *const c_char) -> bool {
    unsafe {
        let mut p: *mut c_char = ptr::null_mut();
        event_name2nr(event, &raw mut p) != NUM_EVENTS as event_T
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn au_exists(arg: *const c_char) -> bool {
    unsafe {
        let mut buflocal_buf: *mut buf_T = ptr::null_mut();
        let mut retval = false;

        let arg_save = xstrdup(arg);
        let mut p = vim_strchr(arg_save, b'#' as c_int).cast_mut();
        if !p.is_null() {
            *p = NUL;
            p = p.add(1);
        }

        let mut group = augroup_find(arg_save);
        let event_name = if group == AUGROUP_ERROR {
            group = AUGROUP_ALL;
            arg_save
        } else {
            if p.is_null() {
                xfree(arg_save.cast());
                return true;
            }
            let ev_name = p;
            p = vim_strchr(ev_name, b'#' as c_int).cast_mut();
            if !p.is_null() {
                *p = NUL;
                p = p.add(1);
            }
            ev_name
        };

        let pattern = p;
        let event = event_name2nr(event_name, &raw mut p);
        if event != NUM_EVENTS as event_T {
            let acs = &raw mut autocmds[event as usize];
            if (*acs).size != 0 {
                if !pattern.is_null() && vim_stricmp(pattern, c"<buffer>".as_ptr()) == 0 {
                    buflocal_buf = curbuf;
                }

                for i in 0..(*acs).size {
                    let ap = (*(*acs).items.add(i)).pat;
                    if !ap.is_null() && (group == AUGROUP_ALL || (*ap).group == group) {
                        let pat_match = if pattern.is_null() {
                            true
                        } else if buflocal_buf.is_null() {
                            mb_strcmp_ic(p_fic != 0, (*ap).pat, pattern) == 0
                        } else {
                            (*ap).buflocal_nr == (*buflocal_buf).b_fnum
                        };
                        if pat_match {
                            retval = true;
                            break;
                        }
                    }
                }
            }
        }

        xfree(arg_save.cast());
        retval
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aupat_is_buflocal(pat: *const c_char, patlen: c_int) -> bool {
    if patlen < 8 {
        return false;
    }
    let slice = unsafe { std::slice::from_raw_parts(pat.cast::<u8>(), patlen as usize) };
    slice.starts_with(b"<buffer") && slice[slice.len() - 1] == b'>'
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aupat_get_buflocal_nr(pat: *const c_char, patlen: c_int) -> c_int {
    unsafe {
        assert!(aupat_is_buflocal(pat, patlen));

        if patlen == 8 {
            return (*curbuf).b_fnum;
        }

        if patlen > 9 && *pat.add(7) == b'=' as c_char {
            if patlen == 13 && vim_strnicmp(pat, c"<buffer=abuf>".as_ptr(), 13) == 0 {
                return autocmd_bufnr;
            }

            if skipdigits(pat.add(8)) == pat.add((patlen - 1) as usize).cast_mut() {
                let mut p = pat.add(8).cast_mut();
                return getdigits_int(&raw mut p, false, 0);
            }
        }

        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aupat_normalize_buflocal_pat(
    dest: *mut c_char,
    pat: *const c_char,
    patlen: c_int,
    mut buflocal_nr: c_int,
) {
    unsafe {
        assert!(aupat_is_buflocal(pat, patlen));

        if buflocal_nr == 0 {
            buflocal_nr = (*curbuf).b_fnum;
        }

        let s = format!("<buffer={buflocal_nr}>");
        let copy_len = s.len().min(BUFLOCAL_PAT_LEN - 1);
        ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), dest, copy_len);
        *dest.add(copy_len) = NUL;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_delete_event(
    group: c_int,
    event: event_T,
    pat: *const c_char,
) -> c_int {
    unsafe { do_autocmd_event(event, pat, false, 0, c"".as_ptr(), true, group) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn autocmd_delete_id(id: i64) -> bool {
    unsafe {
        assert!(id > 0);
        let mut success = false;

        for event in 0..NUM_EVENTS {
            let acs = (&raw mut autocmds).cast::<AutoCmdVec>().add(event);
            for i in 0..(*acs).size {
                let ac = (*acs).items.add(i);
                if (*ac).id == id {
                    aucmd_del(ac);
                    success = true;
                }
            }
        }
        success
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn aucmd_handler_to_string(ac: *mut AutoCmd) -> *mut c_char {
    unsafe {
        if !(*ac).handler_cmd.is_null() {
            xstrdup((*ac).handler_cmd)
        } else {
            callback_to_string(&raw mut (*ac).handler_fn, ptr::null_mut())
        }
    }
}

unsafe fn arg_event_skip(arg: *mut c_char, have_group: bool) -> *mut c_char {
    unsafe {
        let mut pat: *mut c_char;
        let mut p: *mut c_char = ptr::null_mut();

        if *arg == b'*' as c_char {
            if *arg.add(1) != NUL && !ascii_iswhite(*arg.add(1)) {
                semsg!(tr(c"E215: Illegal character after *: %s"), arg);
                return ptr::null_mut();
            }
            pat = arg.add(1);
        } else {
            pat = arg;
            while *pat != NUL && *pat != b'|' as c_char && !ascii_iswhite(*pat) {
                if (event_name2nr(pat, &raw mut p) as usize) >= NUM_EVENTS {
                    if have_group {
                        semsg!(tr(c"E216: No such event: %s"), pat);
                    } else {
                        semsg!(tr(c"E216: No such group or event: %s"), pat);
                    }
                    return ptr::null_mut();
                }
                pat = p;
            }
        }
        pat
    }
}

unsafe fn arg_augroup_get(argp: *mut *mut c_char) -> c_int {
    unsafe {
        let arg = *argp;
        let mut p = arg;
        while *p != NUL && !ascii_iswhite(*p) && *p != b'|' as c_char {
            p = p.add(1);
        }
        if p <= arg {
            return AUGROUP_ALL;
        }

        let group_name = xmemdupz(arg.cast(), p.offset_from(arg) as usize).cast::<c_char>();
        let mut group = augroup_find(group_name);
        if group == AUGROUP_ERROR {
            group = AUGROUP_ALL;
        } else {
            *argp = skipwhite(p);
        }
        xfree(group_name.cast());
        group
    }
}

unsafe fn arg_autocmd_flag_get(
    flag: *mut bool,
    cmd_ptr: *mut *mut c_char,
    pattern: *const c_char,
    len: c_int,
) -> bool {
    unsafe {
        let cmd_bytes = CStr::from_ptr(*cmd_ptr).to_bytes();
        let pat_slice = std::slice::from_raw_parts(pattern.cast::<u8>(), len as usize);
        if cmd_bytes.starts_with(pat_slice) && ascii_iswhite(*(*cmd_ptr).add(len as usize)) {
            if *flag {
                semsg!(tr(CStr::from_ptr((&raw const e_duparg2).cast())), pattern);
                return true;
            }
            *flag = true;
            *cmd_ptr = skipwhite((*cmd_ptr).add(len as usize));
        }
        false
    }
}

unsafe extern "C" fn vimresume_event(_argv: *mut *mut c_void) {
    unsafe {
        apply_autocmds(
            EVENT_VIMRESUME,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        pending_vimresume = kFalse;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn may_trigger_vim_suspend_resume(suspend: bool) {
    unsafe {
        if suspend && pending_vimresume == kFalse {
            pending_vimresume = kNone;
            apply_autocmds(
                EVENT_VIMSUSPEND,
                ptr::null_mut(),
                ptr::null_mut(),
                false,
                ptr::null_mut(),
            );
            pending_vimresume = kTrue;
        } else if !suspend && pending_vimresume == kTrue {
            pending_vimresume = kNone;
            let events = (*loop_queues((&raw mut main_loop).cast::<Loop>())).events;
            multiqueue_put_event(
                events,
                Event {
                    handler: Some(vimresume_event),
                    argv: [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC],
                },
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_autocmd_uienter(chanid: u64, attached: bool) {
    unsafe {
        if starting == NO_SCREEN || uienter_recursive {
            return;
        }
        uienter_recursive = true;

        let mut save_v_event: save_v_event_T = std::mem::zeroed();
        let dict = get_v_event(&raw mut save_v_event);
        assert!(chanid < i64::MAX as u64);
        tv_dict_add_nr(dict, c"chan".as_ptr(), 4, chanid as varnumber_T);
        tv_dict_set_keys_readonly(dict);
        apply_autocmds(
            if attached {
                EVENT_UIENTER
            } else {
                EVENT_UILEAVE
            },
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );
        restore_v_event(dict, &raw mut save_v_event);

        uienter_recursive = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_autocmd_focusgained(gained: bool) {
    unsafe {
        if focusgained_recursive {
            return;
        }
        focusgained_recursive = true;
        apply_autocmds(
            if gained {
                EVENT_FOCUSGAINED
            } else {
                EVENT_FOCUSLOST
            },
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            curbuf,
        );

        if gained && focusgained_last_time + 2000 < os_now() {
            check_timestamps(1);
            focusgained_last_time = os_now();
        }

        focusgained_recursive = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_filetype_autocmd(buf: *mut buf_T, force: bool) -> bool {
    unsafe {
        if ft_recursive > 0 && !force {
            return false;
        }

        let secure_save = secure;
        secure = 0;

        ft_recursive += 1;
        (*buf).b_did_filetype = true;
        let ret = apply_autocmds(
            EVENT_FILETYPE,
            (*buf).b_p_ft,
            (*buf).b_fname,
            force || ft_recursive == 1,
            buf,
        );
        ft_recursive -= 1;

        secure = secure_save;
        ret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_name_lookup_and_span_pattern_work() {
        unsafe {
            let mut end: *mut c_char = ptr::null_mut();
            let ev = event_name2nr(c"BufReadPost,BufEnter".as_ptr(), &raw mut end);
            assert_eq!(ev, EVENT_BUFREADPOST);
            assert_eq!(CStr::from_ptr(end).to_bytes(), b"BufEnter");
            assert_eq!(
                CStr::from_ptr(event_nr2name(EVENT_VIMENTER)).to_bytes(),
                b"VimEnter"
            );

            let mut start: *const c_char = ptr::null();
            let len = aucmd_span_pattern(c",,*.{c,h},*.rs".as_ptr(), &raw mut start);
            assert_eq!(len, 7);
            assert_eq!(&CStr::from_ptr(start).to_bytes()[..len], b"*.{c,h}");
        }
    }
}
