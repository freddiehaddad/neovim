//! Autocommand registration, matching, and execution (`src/nvim/autocmd.c`).

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{
    HLF_E, api_clear_error, api_free_object, api_free_string, boolean_obj, copy_object,
    cstr_as_obj, cstr_to_string, dict_obj, find_buffer_by_handle, integer_obj,
    static_cstr_as_string, string_obj,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType,
};
use crate::api::tabpage::Buffer;
use crate::api::ui::NO_SCREEN;
use crate::buffer::{
    Bufref, FileBuf, bt_nofile, buflist_findnr, bufref_valid, do_modelines, set_bufref,
};
use crate::charset::{NUL, ascii_iswhite, getdigits_int, skipdigits, skipwhite};
use crate::cmdexpand::{EXPAND_EVENTS, EXPAND_FILES, EXPAND_NOTHING};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch, is_ctx_win};
use crate::errors::*;
use crate::eval::typval::{
    Callback, K_CALLBACK_LUA, TypVal, Varnumber, callback_copy, callback_free, callback_to_string,
    tv_clear, tv_dict_add_nr, tv_dict_add_tv, tv_dict_set_keys_readonly,
};
use crate::eval::userfunc::{restore_funccal, save_funccal};
use crate::eval::vars::{
    VV_CMDBANG, VV_OPTION_TYPE, VV_TERMRESPONSE, get_vim_var_nr, get_vim_var_str, set_cmdarg,
    set_vim_var_nr,
};
use crate::eval::{callback_call, get_v_event, last_set_msg, restore_v_event};
use crate::event::r#loop::loop_queues;
use crate::event::multiqueue::{
    Event, multiqueue_free, multiqueue_new_child, multiqueue_put_event,
};
use crate::ex_docmd::{Exarg, do_cmdline, ends_excmd, expand_sfile};
use crate::ex_eval::{aborting, should_abort};
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::fileio::{check_timestamps, file_pat_to_reg_pat, match_file_pat};
use crate::globals::*;
use crate::input::{RedoState, restore_redobuff, save_redobuff};
use crate::insexpand::ins_compl_active;
use crate::lua::executor::{K_RET_NIL_BOOL, nlua_call_ref, nlua_set_sctx};
use crate::math::{FAIL, OK};
use crate::mbyte::mb_strcmp_ic;

use crate::message::{
    emsg, give_warning, msg_advance, msg_clr_eos, msg_end, msg_ext_set_kind, msg_outtrans,
    msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, msg_start, semsg, smsg, verbose_enter,
    verbose_enter_scroll, verbose_leave, verbose_leave_scroll,
};
use crate::msgpack_rpc::packer::{Integer, ObjHandle};
use crate::option::{OPT_LOCAL, apply_optionset_autocmd_now, set_option_direct};
use crate::option_vars::*;
use crate::optionstr::OptIndex;
use crate::os::env::{ExpandCtx, expand_env_save};
use crate::os::input::line_breakcheck;
use crate::os::time::{Timestamp, os_now};
use crate::path::{FullName_save, path_tail, to_slash_save};
use crate::profile::{Proftime, prof_child_enter, prof_child_exit};
use crate::regexp::{RE_MAGIC, RE_NOBREAK};
use crate::runtime::{ETYPE_AUCMD, SID_NONE, Sctx, estack_pop, estack_push, rt_st};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::state::{MODE_INSERT, MODE_NORMAL_BUSY, SaveVEvent, get_mode, get_real_state};
use crate::strings::tr;
use crate::window::{
    WinInfoInfo, check_lnums, check_lnums_nested, reset_lnums, snapshot_windows_scroll_size,
    unuse_tabpage, use_tabpage,
};
use std::ffi::{CStr, CString, c_char};
use std::ptr;

pub type AuEvent = i32;
pub(crate) const BUFLOCAL_PAT_LEN: usize = 25;

#[repr(C)]
pub struct AutoPat {
    pub pat: CString,
    pub reg_prog: Option<Box<crate::regexp::Regprog>>,
    pub group: i32,
    pub patlen: i32,
    pub buflocal_nr: i32,
    pub allow_dirs: c_char,
}

#[derive(Clone)]
pub enum AutoCmdHandler {
    None,
    Cmd(CString),
    Fn(Callback),
}

impl AutoCmdHandler {
    #[inline]
    pub const fn is_cmd(&self) -> bool {
        matches!(self, Self::Cmd(_))
    }
}

#[repr(C)]
pub struct AutoCmd {
    pub pat: Option<std::rc::Rc<AutoPat>>,
    pub id: i64,
    pub desc: Option<CString>,
    pub handler: AutoCmdHandler,
    pub script_ctx: crate::runtime::Sctx,
    pub once: bool,
    pub nested: bool,
}

impl AutoCmd {
    pub const fn new() -> Self {
        Self {
            pat: None,
            id: 0,
            desc: None,
            handler: AutoCmdHandler::None,
            script_ctx: crate::runtime::Sctx::new(),
            once: false,
            nested: false,
        }
    }
}

impl Default for AutoCmd {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AutoCmd {
    fn drop(&mut self) {
        if let AutoCmdHandler::Fn(cb) = &mut self.handler {
            callback_free(cb);
        }
        self.handler = AutoCmdHandler::None;
    }
}

pub type AutoCmdVec = Vec<AutoCmd>;

pub const AUGROUP_DEFAULT: i32 = -1;
pub const AUGROUP_ERROR: i32 = -2;
pub const AUGROUP_ALL: i32 = -3;
pub const AUGROUP_DELETED: i32 = -4;

pub(crate) const OPT_NOWIN: i32 = 0x10;
pub(crate) const K_OPT_EVENTIGNORE: OptIndex = 88;
pub(crate) const K_OPT_MODIFIED: OptIndex = 196;

pub(crate) const DOCMD_VERBOSE: i32 = 0x01;
pub(crate) const DOCMD_NOWAIT: i32 = 0x02;
pub(crate) const DOCMD_REPEAT: i32 = 0x04;

pub(crate) const PROF_YES: i32 = 1;

pub(crate) const HLF_8: i32 = 1;
pub(crate) const HLF_T: i32 = 23;

pub const EVENT_BUFADD: AuEvent = 0;
pub const EVENT_BUFCREATE: AuEvent = 1;
pub const EVENT_BUFDELETE: AuEvent = 2;
pub const EVENT_BUFENTER: AuEvent = 3;
pub const EVENT_BUFFILEPOST: AuEvent = 4;
pub const EVENT_BUFFILEPRE: AuEvent = 5;
pub const EVENT_BUFHIDDEN: AuEvent = 6;
pub const EVENT_BUFLEAVE: AuEvent = 7;
pub const EVENT_BUFNEW: AuEvent = 8;
pub const EVENT_BUFNEWFILE: AuEvent = 9;
pub const EVENT_BUFREAD: AuEvent = 10;
pub const EVENT_BUFREADCMD: AuEvent = 11;
pub const EVENT_BUFREADPOST: AuEvent = 12;
pub const EVENT_BUFREADPRE: AuEvent = 13;
pub const EVENT_BUFUNLOAD: AuEvent = 14;
pub const EVENT_BUFWINENTER: AuEvent = 15;
pub const EVENT_BUFWINLEAVE: AuEvent = 16;
pub const EVENT_BUFWIPEOUT: AuEvent = 17;
pub const EVENT_BUFWRITE: AuEvent = 18;
pub const EVENT_BUFWRITECMD: AuEvent = 19;
pub const EVENT_BUFWRITEPOST: AuEvent = 20;
pub const EVENT_BUFWRITEPRE: AuEvent = 21;
pub const EVENT_CHANCLOSE: AuEvent = 22;
pub const EVENT_CHANINFO: AuEvent = 23;
pub const EVENT_CHANOPEN: AuEvent = 24;
pub const EVENT_CMDATOM: AuEvent = 25;
pub const EVENT_CMDLINECHANGED: AuEvent = 26;
pub const EVENT_CMDLINEENTER: AuEvent = 27;
pub const EVENT_CMDLINELEAVE: AuEvent = 28;
pub const EVENT_CMDLINELEAVEPRE: AuEvent = 29;
pub const EVENT_CMDUNDEFINED: AuEvent = 30;
pub const EVENT_CMDWINENTER: AuEvent = 31;
pub const EVENT_CMDWINLEAVE: AuEvent = 32;
pub const EVENT_COLORSCHEME: AuEvent = 33;
pub const EVENT_COLORSCHEMEPRE: AuEvent = 34;
pub const EVENT_COMPLETECHANGED: AuEvent = 35;
pub const EVENT_COMPLETEDONE: AuEvent = 36;
pub const EVENT_COMPLETEDONEPRE: AuEvent = 37;
pub const EVENT_CURSORHOLD: AuEvent = 38;
pub const EVENT_CURSORHOLDI: AuEvent = 39;
pub const EVENT_CURSORMOVED: AuEvent = 40;
pub const EVENT_CURSORMOVEDC: AuEvent = 41;
pub const EVENT_CURSORMOVEDI: AuEvent = 42;
pub const EVENT_DIAGNOSTICCHANGED: AuEvent = 43;
pub const EVENT_DIFFUPDATED: AuEvent = 44;
pub const EVENT_DIRCHANGED: AuEvent = 45;
pub const EVENT_DIRCHANGEDPRE: AuEvent = 46;
pub const EVENT_ENCODINGCHANGED: AuEvent = 47;
pub const EVENT_EXITPRE: AuEvent = 48;
pub const EVENT_FILEAPPENDCMD: AuEvent = 49;
pub const EVENT_FILEAPPENDPOST: AuEvent = 50;
pub const EVENT_FILEAPPENDPRE: AuEvent = 51;
pub const EVENT_FILECHANGEDRO: AuEvent = 52;
pub const EVENT_FILECHANGEDSHELL: AuEvent = 53;
pub const EVENT_FILECHANGEDSHELLPOST: AuEvent = 54;
pub const EVENT_FILEENCODING: AuEvent = 55;
pub const EVENT_FILEREADCMD: AuEvent = 56;
pub const EVENT_FILEREADPOST: AuEvent = 57;
pub const EVENT_FILEREADPRE: AuEvent = 58;
pub const EVENT_FILETYPE: AuEvent = 59;
pub const EVENT_FILEWRITECMD: AuEvent = 60;
pub const EVENT_FILEWRITEPOST: AuEvent = 61;
pub const EVENT_FILEWRITEPRE: AuEvent = 62;
pub const EVENT_FILTERREADPOST: AuEvent = 63;
pub const EVENT_FILTERREADPRE: AuEvent = 64;
pub const EVENT_FILTERWRITEPOST: AuEvent = 65;
pub const EVENT_FILTERWRITEPRE: AuEvent = 66;
pub const EVENT_FOCUSGAINED: AuEvent = 67;
pub const EVENT_FOCUSLOST: AuEvent = 68;
pub const EVENT_FUNCUNDEFINED: AuEvent = 69;
pub const EVENT_GUIENTER: AuEvent = 70;
pub const EVENT_GUIFAILED: AuEvent = 71;
pub const EVENT_INSERTCHANGE: AuEvent = 72;
pub const EVENT_INSERTCHARPRE: AuEvent = 73;
pub const EVENT_INSERTENTER: AuEvent = 74;
pub const EVENT_INSERTLEAVE: AuEvent = 75;
pub const EVENT_INSERTLEAVEPRE: AuEvent = 76;
pub const EVENT_LSPATTACH: AuEvent = 77;
pub const EVENT_LSPDETACH: AuEvent = 78;
pub const EVENT_LSPNOTIFY: AuEvent = 79;
pub const EVENT_LSPPROGRESS: AuEvent = 80;
pub const EVENT_LSPREQUEST: AuEvent = 81;
pub const EVENT_LSPTOKENUPDATE: AuEvent = 82;
pub const EVENT_MARKSET: AuEvent = 83;
pub const EVENT_MENUPOPUP: AuEvent = 84;
pub const EVENT_MODECHANGED: AuEvent = 85;
pub const EVENT_OPTIONSET: AuEvent = 86;
pub const EVENT_PACKCHANGED: AuEvent = 87;
pub const EVENT_PACKCHANGEDPRE: AuEvent = 88;
pub const EVENT_PROGRESS: AuEvent = 89;
pub const EVENT_QUICKFIXCMDPOST: AuEvent = 90;
pub const EVENT_QUICKFIXCMDPRE: AuEvent = 91;
pub const EVENT_QUITPRE: AuEvent = 92;
pub const EVENT_RECORDINGENTER: AuEvent = 93;
pub const EVENT_RECORDINGLEAVE: AuEvent = 94;
pub const EVENT_REMOTEREPLY: AuEvent = 95;
pub const EVENT_SAFESTATE: AuEvent = 96;
pub const EVENT_SEARCHWRAPPED: AuEvent = 97;
pub const EVENT_SESSIONLOADPOST: AuEvent = 98;
pub const EVENT_SESSIONLOADPRE: AuEvent = 99;
pub const EVENT_SESSIONWRITEPOST: AuEvent = 100;
pub const EVENT_SESSIONWRITEPRE: AuEvent = 101;
pub const EVENT_SHELLCMDPOST: AuEvent = 102;
pub const EVENT_SHELLFILTERPOST: AuEvent = 103;
pub const EVENT_SIGNAL: AuEvent = 104;
pub const EVENT_SOURCECMD: AuEvent = 105;
pub const EVENT_SOURCEPOST: AuEvent = 106;
pub const EVENT_SOURCEPRE: AuEvent = 107;
pub const EVENT_SPELLFILEMISSING: AuEvent = 108;
pub const EVENT_STDINREADPOST: AuEvent = 109;
pub const EVENT_STDINREADPRE: AuEvent = 110;
pub const EVENT_SWAPEXISTS: AuEvent = 111;
pub const EVENT_SYNTAX: AuEvent = 112;
pub const EVENT_TABCLOSED: AuEvent = 113;
pub const EVENT_TABCLOSEDPRE: AuEvent = 114;
pub const EVENT_TABENTER: AuEvent = 115;
pub const EVENT_TABLEAVE: AuEvent = 116;
pub const EVENT_TABMOVED: AuEvent = 117;
pub const EVENT_TABNEW: AuEvent = 118;
pub const EVENT_TABNEWENTERED: AuEvent = 119;
pub const EVENT_TERMCHANGED: AuEvent = 120;
pub const EVENT_TERMCLOSE: AuEvent = 121;
pub const EVENT_TERMENTER: AuEvent = 122;
pub const EVENT_TERMLEAVE: AuEvent = 123;
pub const EVENT_TERMOPEN: AuEvent = 124;
pub const EVENT_TERMREQUEST: AuEvent = 125;
pub const EVENT_TERMRESPONSE: AuEvent = 126;
pub const EVENT_TEXTCHANGED: AuEvent = 127;
pub const EVENT_TEXTCHANGEDI: AuEvent = 128;
pub const EVENT_TEXTCHANGEDP: AuEvent = 129;
pub const EVENT_TEXTCHANGEDT: AuEvent = 130;
pub const EVENT_TEXTPUTPOST: AuEvent = 131;
pub const EVENT_TEXTPUTPRE: AuEvent = 132;
pub const EVENT_TEXTYANKPOST: AuEvent = 133;
pub const EVENT_UIENTER: AuEvent = 134;
pub const EVENT_UILEAVE: AuEvent = 135;
pub const EVENT_USER: AuEvent = 136;
pub const EVENT_VIMENTER: AuEvent = 137;
pub const EVENT_VIMLEAVE: AuEvent = 138;
pub const EVENT_VIMLEAVEPRE: AuEvent = 139;
pub const EVENT_VIMRESIZED: AuEvent = 140;
pub const EVENT_VIMRESUME: AuEvent = 141;
pub const EVENT_VIMSUSPEND: AuEvent = 142;
pub const EVENT_WINCLOSED: AuEvent = 143;
pub const EVENT_WINENTER: AuEvent = 144;
pub const EVENT_WINLEAVE: AuEvent = 145;
pub const EVENT_WINNEW: AuEvent = 146;
pub const EVENT_WINNEWPRE: AuEvent = 147;
pub const EVENT_WINRESIZED: AuEvent = 148;
pub const EVENT_WINSCROLLED: AuEvent = 149;
pub const NUM_EVENTS: usize = 150;

const E_AUTOCOMMAND_NESTING_TOO_DEEP: &CStr = c"E218: Autocommand nesting too deep";

#[repr(C)]
pub struct AutoPatCmd {
    pub lastpat: Option<std::rc::Rc<AutoPat>>,
    pub auidx: usize,
    pub ausize: usize,
    pub afile_orig: Option<std::ffi::CString>,
    pub fname: Option<std::ffi::CString>,
    pub sfname: Option<std::ffi::CString>,
    pub group: i32,
    pub event: AuEvent,
    pub script_ctx: std::rc::Rc<std::cell::Cell<Sctx>>,
    pub arg_bufnr: std::rc::Rc<std::cell::Cell<i32>>,
    pub data: Option<Object>,
}

#[repr(C)]
pub struct AutoCmdEvent {
    pub event: AuEvent,
    pub fname: Option<std::ffi::CString>,
    pub fname_io: Option<std::ffi::CString>,
    pub buf: Buffer,
    pub group: i32,
    pub data: Option<Object>,
}

impl Drop for AutoCmdEvent {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            api_free_object(data);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ExpandPrefix {
    xp_pattern: *mut c_char,
    xp_context: i32,
}

#[derive(Clone, Copy)]
struct EventNameEntry {
    _len: usize,
    name: &'static CStr,
    event: i32,
}

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

static EVENT_HASH: [AuEvent; 150] = [
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

struct AutocmdState {
    autocmds: [AutoCmdVec; NUM_EVENTS],
    active_apc_list: Vec<std::rc::Rc<std::cell::Cell<i32>>>,
    autocmd_winid: ObjHandle,
    next_augroup_id: i32,
    deleted_augroup: *const c_char,
    current_augroup: i32,
    au_need_clean: bool,
    autocmd_blocked: i32,
    autocmd_nested: bool,
    autocmd_include_groups: bool,
    termresponse_changed: bool,
    termresponse_chan_id: u64,
    map_augroup_name_to_id: Vec<(std::ffi::CString, i32)>,
    map_augroup_id_to_name: std::collections::BTreeMap<i32, std::ffi::CString>,
    apply_autocmds_nesting: i32,
    filechangeshell_busy: bool,
    pending_vimresume: TriState,
    uienter_recursive: bool,
    focusgained_recursive: bool,
    focusgained_last_time: Timestamp,
    ft_recursive: i32,
}

impl AutocmdState {
    const fn new() -> Self {
        Self {
            autocmds: [const { Vec::new() }; NUM_EVENTS],
            active_apc_list: Vec::new(),
            autocmd_winid: 0,
            next_augroup_id: 1,
            deleted_augroup: ptr::null(),
            current_augroup: AUGROUP_DEFAULT,
            au_need_clean: false,
            autocmd_blocked: 0,
            autocmd_nested: false,
            autocmd_include_groups: false,
            termresponse_changed: false,
            termresponse_chan_id: 0,
            map_augroup_name_to_id: Vec::new(),
            map_augroup_id_to_name: std::collections::BTreeMap::new(),
            apply_autocmds_nesting: 0,
            filechangeshell_busy: false,
            pending_vimresume: K_FALSE,
            uienter_recursive: false,
            focusgained_recursive: false,
            focusgained_last_time: 0,
            ft_recursive: 0,
        }
    }
}

static AU_STATE: GlobalState<AutocmdState> = GlobalState::new(AutocmdState::new());

#[inline]
fn au_st() -> &'static mut AutocmdState {
    unsafe { &mut *AU_STATE.as_ptr() }
}

use crate::strings::vim_strchr;

fn map_put_name_to_id(name: impl crate::strings::IntoCCharPtr, val: i32) {
    let name = name.into_c_char_ptr();
    {
        let name_cstr = crate::memory::cstr_from_ptr(name);
        for (existing_name, existing_val) in &mut au_st().map_augroup_name_to_id {
            if existing_name.as_c_str() == name_cstr {
                *existing_val = val;
                return;
            }
        }
        au_st()
            .map_augroup_name_to_id
            .push((name_cstr.to_owned(), val));
    }
}

fn map_get_id_to_name(key: i32) -> *mut c_char {
    au_st()
        .map_augroup_id_to_name
        .get(&key)
        .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
}

fn map_put_id_to_name(key: i32, name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    {
        au_st()
            .map_augroup_id_to_name
            .insert(key, crate::memory::cstr_to_cstring(name));
    }
}

fn event_name2nr_hash(str_: impl crate::strings::IntoCCharPtr, len: usize) -> isize {
    let str_ = str_.into_c_char_ptr();
    if str_.is_null() {
        return -1;
    }
    let s = { crate::memory::cstr_slice(str_, len) };
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
        if crate::memory::cstr_prefix_bytes(str_, len).eq_ignore_ascii_case(
            crate::memory::cstr_prefix_bytes(EVENT_NAMES[ev].name.as_ptr(), len),
        ) {
            return i as isize;
        }
    }
    -1
}

pub fn autocmd_init() {
    let events = loop_queues(&mut g_st().main_loop).events;
    g_st().deferred_events = multiqueue_new_child(events);
}

pub fn autocmd_free_all_mem() {
    multiqueue_free(g_st().deferred_events);
}

fn augroup_map_del(id: i32, name: impl crate::strings::IntoCCharPtr) {
    let name = name.into_c_char_ptr();
    {
        if !name.is_null() {
            let name_cstr = crate::memory::cstr_from_ptr(name);
            if let Some(pos) = au_st()
                .map_augroup_name_to_id
                .iter()
                .position(|(existing_name, _)| existing_name.as_c_str() == name_cstr)
            {
                au_st().map_augroup_name_to_id.swap_remove(pos);
            }
        }
        if id > 0 {
            au_st().map_augroup_id_to_name.remove(&id);
        }
    }
}

#[inline]
fn get_deleted_augroup() -> *const c_char {
    if au_st().deleted_augroup.is_null() {
        au_st().deleted_augroup = tr(c"--Deleted--");
    }
    au_st().deleted_augroup
}

fn au_show_for_all_events(group: i32, pat: impl crate::strings::IntoCCharPtr) {
    let pat = pat.into_c_char_ptr();
    for event in 0..NUM_EVENTS as AuEvent {
        au_show_for_event(group, event, pat);
    }
}

fn au_show_for_event(group: i32, event: AuEvent, pat: impl crate::strings::IntoCCharPtr) {
    let mut pat = pat.into_c_char_ptr();
    unsafe {
        let acs = &raw mut au_st().autocmds[event as usize];
        if (*acs).is_empty() {
            return;
        }

        let mut patlen: i32 = 0;
        if !crate::memory::cstr_is_empty(pat) {
            patlen = aucmd_span_pattern(pat, &raw mut pat) as i32;
            if patlen == 0 {
                return;
            }
        }

        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];
        let mut last_group = AUGROUP_ERROR;
        let mut last_group_name: *const c_char;

        loop {
            let mut last_ap: Option<std::rc::Rc<AutoPat>> = None;
            let endpat = pat.wrapping_add(patlen as usize);

            if aupat_is_buflocal(pat, patlen) {
                let nr = aupat_get_buflocal_nr(pat, patlen);
                aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, nr);
                pat = buflocal_pat.as_ptr();
                patlen = crate::memory::cstr_len(pat) as i32;
            }

            for ac in &mut *acs {
                let Some(ref ap) = ac.pat else {
                    continue;
                };

                let pat_matches = if patlen != 0 {
                    ap.patlen == patlen
                        && crate::memory::cstr_slice(pat, patlen as usize) == ap.pat.as_bytes()
                } else {
                    true
                };
                if (group != AUGROUP_ALL && ap.group != group) || !pat_matches {
                    continue;
                }

                if ap.group != last_group {
                    last_group = ap.group;
                    last_group_name = augroup_name(ap.group);

                    if g_st().got_int {
                        return;
                    }

                    msg_putchar(b'\n' as i32);
                    if g_st().got_int {
                        return;
                    }

                    if ap.group != AUGROUP_DEFAULT {
                        if last_group_name.is_null() {
                            msg_puts_hl(get_deleted_augroup(), HLF_E, false);
                        } else {
                            msg_puts_hl(last_group_name, HLF_T, false);
                        }
                        msg_puts(c"  ");
                    }
                    msg_puts_hl(event_nr2name(event), HLF_T, false);
                }

                if !last_ap.as_ref().is_some_and(|l| std::rc::Rc::ptr_eq(l, ap)) {
                    last_ap = Some(ap.clone());

                    msg_putchar(b'\n' as i32);
                    if g_st().got_int {
                        return;
                    }

                    msg_advance(4);
                    msg_outtrans(ap.pat.as_ptr(), 0, false);
                }

                if g_st().got_int {
                    return;
                }

                if g_st().msg_col >= 14 {
                    msg_putchar(b'\n' as i32);
                }
                msg_advance(14);
                if g_st().got_int {
                    return;
                }

                let handler_str = aucmd_handler_to_string(ac);
                if let Some(desc) = &ac.desc {
                    let msglen = 100usize;
                    let mut mbuf = [0 as c_char; 100];
                    let desc_str = desc.to_string_lossy();
                    let formatted = if ac.handler.is_cmd() {
                        let h_str = handler_str.to_string_lossy();
                        format!("{h_str} [{desc_str}]")
                    } else {
                        msg_puts_hl(handler_str.as_ptr(), HLF_8, false);
                        format!(" [{desc_str}]")
                    };
                    let copy_len = formatted.len().min(msglen - 1);
                    ptr::copy_nonoverlapping(
                        formatted.as_ptr().cast::<c_char>(),
                        mbuf.as_mut_ptr(),
                        copy_len,
                    );
                    mbuf[copy_len] = NUL;
                    msg_outtrans(mbuf.as_ptr(), 0, false);
                } else if ac.handler.is_cmd() {
                    msg_outtrans(handler_str.as_ptr(), 0, false);
                } else {
                    msg_puts_hl(handler_str.as_ptr(), HLF_8, false);
                }
                if opt_vars().p_verbose > 0 {
                    last_set_msg(ac.script_ctx);
                }

                if g_st().got_int {
                    return;
                }
            }

            patlen = aucmd_span_pattern(endpat, &raw mut pat) as i32;
            if patlen == 0 {
                break;
            }
        }
    }
}

fn aucmd_del(ac: &mut AutoCmd) {
    *ac = AutoCmd::new();
    au_st().au_need_clean = true;
}

pub fn aucmd_del_for_event_and_group(event: AuEvent, group: i32) {
    let acs = &mut au_st().autocmds[event as usize];
    for ac in acs {
        if ac.pat.as_ref().is_some_and(|ap| ap.group == group) {
            aucmd_del(ac);
        }
    }
    au_cleanup();
}

fn au_cleanup() {
    if g_st().autocmd_busy || !au_st().au_need_clean {
        return;
    }

    for acs in &mut au_st().autocmds {
        acs.retain(|ac| ac.pat.is_some());
        if acs.is_empty() {
            *acs = Vec::new();
        }
    }

    au_st().au_need_clean = false;
}

pub fn au_get_autocmds_for_event(event: AuEvent) -> &'static mut AutoCmdVec {
    &mut au_st().autocmds[event as usize]
}

pub fn aubuflocal_remove(buf: &mut FileBuf) {
    for arg_bufnr in au_st().active_apc_list.iter().rev() {
        if buf.b_fnum == arg_bufnr.get() {
            arg_bufnr.set(0);
        }
    }

    for event in 0..NUM_EVENTS as AuEvent {
        let acs = &mut au_st().autocmds[event as usize];
        for ac in acs {
            if !ac
                .pat
                .as_ref()
                .is_some_and(|ap| ap.buflocal_nr == buf.b_fnum)
            {
                continue;
            }

            aucmd_del(ac);

            if opt_vars().p_verbose >= 6 {
                verbose_enter();
                smsg!(
                    0,
                    tr(c"auto-removing autocommand: %s <buffer=%d>"),
                    event_nr2name(event),
                    buf.b_fnum,
                );
                verbose_leave();
            }
        }
    }
    au_cleanup();
}

pub fn augroup_add(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        assert!(!crate::memory::cstr_as_bytes(name).eq_ignore_ascii_case(b"end"));

        let existing_id = augroup_find(name);
        if existing_id > 0 {
            assert!(existing_id != AUGROUP_DELETED);
            return existing_id;
        }

        if existing_id == AUGROUP_DELETED {
            augroup_map_del(existing_id, name);
        }

        let next_id = au_st().next_augroup_id;
        au_st().next_augroup_id += 1;
        map_put_name_to_id(name, next_id);
        map_put_id_to_name(next_id, name);

        next_id
    }
}

pub fn augroup_del_by_id(id: i32) {
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

pub fn augroup_del(
    name: impl crate::strings::IntoCCharPtr,
    id_for_error: i32,
    stupid_legacy_mode: bool,
) {
    let name = name.into_c_char_ptr();
    {
        let group = augroup_find(name);
        if group == AUGROUP_ERROR {
            if id_for_error > 0 {
                semsg!(tr(c"E367: No such group id: %d \"%s\""), id_for_error, name);
            } else {
                semsg!(tr(c"E367: No such group: \"%s\""), name);
            }
            return;
        } else if group == au_st().current_augroup {
            emsg(tr(c"E936: Cannot delete the current group"));
            return;
        }

        if stupid_legacy_mode {
            for acs in &au_st().autocmds {
                for ac in acs {
                    if let Some(ref ap) = ac.pat
                        && ap.group == group
                    {
                        give_warning(
                            tr(c"W19: Deleting augroup that is still in use"),
                            true,
                            true,
                        );
                        map_put_name_to_id(name, AUGROUP_DELETED);
                        augroup_map_del(ap.group, ptr::null());
                        return;
                    }
                }
            }
        } else {
            for acs in &mut au_st().autocmds {
                for ac in acs {
                    if ac.pat.as_ref().is_some_and(|ap| ap.group == group) {
                        aucmd_del(ac);
                    }
                }
            }
        }

        augroup_map_del(group, name);
        au_cleanup();
    }
}

pub fn augroup_find(name: impl crate::strings::IntoCCharPtr) -> i32 {
    let name = name.into_c_char_ptr();
    {
        if name.is_null() {
            return AUGROUP_ERROR;
        }
        let name_cstr = crate::memory::cstr_from_ptr(name);
        let existing_id = au_st()
            .map_augroup_name_to_id
            .iter()
            .find_map(|(k, v)| (k.as_c_str() == name_cstr).then_some(*v))
            .unwrap_or(0);
        if existing_id == AUGROUP_DELETED || existing_id > 0 {
            existing_id
        } else {
            AUGROUP_ERROR
        }
    }
}

pub fn augroup_name(mut group: i32) -> *mut c_char {
    assert!(group != 0);

    if group == AUGROUP_DELETED {
        return get_deleted_augroup().cast_mut();
    }

    if group == AUGROUP_ALL {
        group = au_st().current_augroup;
    }

    if group == au_st().next_augroup_id {
        return c"END".as_ptr().cast_mut();
    }

    if group > au_st().next_augroup_id {
        return ptr::null_mut();
    }

    let key = map_get_id_to_name(group);
    if !key.is_null() {
        return key;
    }

    get_deleted_augroup().cast_mut()
}

pub fn augroup_exists(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    augroup_find(name) > 0
}

pub fn do_augroup(arg: impl crate::strings::IntoCCharPtr, del_group: bool) {
    let arg = arg.into_c_char_ptr();
    {
        if del_group {
            if crate::memory::cstr_is_empty(arg) {
                emsg(tr(E_ARGREQ));
            } else {
                augroup_del(arg, -1, true);
            }
        } else if crate::memory::cstr_as_bytes(arg).eq_ignore_ascii_case(b"end") {
            au_st().current_augroup = AUGROUP_DEFAULT;
        } else if !crate::memory::cstr_is_empty(arg) {
            au_st().current_augroup = augroup_add(arg);
        } else {
            msg_start();
            msg_ext_set_kind(c"list_cmd");

            for (name, value) in &au_st().map_augroup_name_to_id {
                if *value > 0 {
                    msg_puts(name.as_ptr());
                } else {
                    msg_puts(augroup_name(*value));
                }
                msg_puts(c"  ");
            }

            msg_clr_eos();
            msg_end();
        }
    }
}

pub fn free_all_autocmds() {
    for acs in &mut au_st().autocmds {
        for ac in acs.iter_mut() {
            aucmd_del(ac);
        }
        *acs = Vec::new();
        au_st().au_need_clean = false;
    }

    au_st().map_augroup_name_to_id = Vec::new();

    au_st().map_augroup_id_to_name = std::collections::BTreeMap::new();
}

pub fn event_name2nr(start: impl crate::strings::IntoCCharPtr, end: *mut *mut c_char) -> AuEvent {
    let start = start.into_c_char_ptr();
    {
        let mut p = start;
        while !crate::memory::cstr_is_empty(p)
            && !ascii_iswhite(crate::memory::cstr_first_char(p))
            && crate::memory::cstr_first_byte(p) != b','
            && crate::memory::cstr_first_byte(p) != b'|'
        {
            p = p.wrapping_add(1);
        }

        let hash_idx = event_name2nr_hash(start, crate::memory::ptr_offset_from(p, start) as usize);
        if crate::memory::cstr_first_byte(p) == b',' {
            p = p.wrapping_add(1);
        }
        unsafe {
            *end = p.cast_mut();
        }
        if hash_idx < 0 {
            return NUM_EVENTS as AuEvent;
        }
        let ev = EVENT_HASH[hash_idx as usize] as usize;
        EVENT_NAMES[ev].event.abs() as AuEvent
    }
}

pub fn event_name2nr_str(str_: NvimString) -> AuEvent {
    let hash_idx = event_name2nr_hash(str_.data, str_.size);
    if hash_idx < 0 {
        return NUM_EVENTS as AuEvent;
    }
    let ev = EVENT_HASH[hash_idx as usize] as usize;
    EVENT_NAMES[ev].event.abs() as AuEvent
}

pub fn event_nr2name(event: AuEvent) -> &'static std::ffi::CStr {
    if event >= 0 && (event as usize) < NUM_EVENTS {
        EVENT_NAMES[event as usize].name
    } else {
        c"Unknown"
    }
}

pub fn event_ignored(event: AuEvent, mut ei: *mut c_char) -> bool {
    let mut ignored = false;
    while !crate::memory::cstr_is_empty(ei) {
        let unignore = crate::memory::cstr_first_byte(ei) == b'-';
        if unignore {
            ei = ei.wrapping_add(1);
        }
        if crate::memory::cstr_prefix_bytes(ei, 3).eq_ignore_ascii_case(b"all")
            && (crate::memory::cstr_byte_at(ei, 3) == 0
                || crate::memory::cstr_byte_at(ei, 3) == b',')
        {
            ignored = ptr::eq(ei.cast_const(), opt_vars().p_ei.cast_const())
                || EVENT_NAMES[event as usize].event <= 0;
            ei = ei.wrapping_add(3 + usize::from(crate::memory::cstr_byte_at(ei, 3) == b','));
        } else if event_name2nr(ei, &raw mut ei) == event {
            if unignore {
                return false;
            }
            ignored = true;
        }
    }
    ignored
}

pub fn check_ei(mut ei: *mut c_char, win: bool) -> bool {
    while !crate::memory::cstr_is_empty(ei) {
        if crate::memory::cstr_prefix_bytes(ei, 3).eq_ignore_ascii_case(b"all")
            && (crate::memory::cstr_byte_at(ei, 3) == 0
                || crate::memory::cstr_byte_at(ei, 3) == b',')
        {
            ei = ei.wrapping_add(3 + usize::from(crate::memory::cstr_byte_at(ei, 3) == b','));
        } else {
            if crate::memory::cstr_first_byte(ei) == b'-' {
                ei = ei.wrapping_add(1);
            }
            let event = event_name2nr(ei, &raw mut ei);
            if event == NUM_EVENTS as AuEvent || (win && EVENT_NAMES[event as usize].event > 0) {
                return false;
            }
        }
    }
    true
}

pub fn au_event_disable(what: &std::ffi::CStr) -> std::ffi::CString {
    let p_ei_bytes = crate::memory::cstr_as_bytes(opt_vars().p_ei);
    let what_bytes = what.to_bytes();
    let save_ei = crate::memory::cstr_to_cstring(opt_vars().p_ei);
    let mut new_ei_vec: Vec<u8> = Vec::with_capacity(p_ei_bytes.len() + what_bytes.len());
    if what_bytes.first() == Some(&b',') && p_ei_bytes.is_empty() {
        new_ei_vec.extend_from_slice(&what_bytes[1..]);
    } else {
        new_ei_vec.extend_from_slice(p_ei_bytes);
        new_ei_vec.extend_from_slice(what_bytes);
    }
    let new_ei = CString::new(new_ei_vec).unwrap_or_default();
    set_option_direct(K_OPT_EVENTIGNORE, cstr_as_obj(new_ei.as_ptr()), 0, SID_NONE);
    save_ei
}

pub fn au_event_restore(old_ei: Option<std::ffi::CString>) {
    if let Some(old_ei) = old_ei {
        set_option_direct(K_OPT_EVENTIGNORE, cstr_as_obj(old_ei.as_ptr()), 0, SID_NONE);
    }
}

pub fn do_autocmd(eap: &mut Exarg, arg_in: *mut c_char, forceit: i32) {
    let mut arg = arg_in;
    let mut envpat: Option<CString> = None;
    let mut cmd: *mut c_char;
    let mut cmd_owned: Option<CString> = None;
    let mut nested = false;
    let mut once = false;
    let group = if crate::memory::cstr_first_byte(arg) == b'|' {
        eap.nextcmd = arg.wrapping_add(1);
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
        if crate::memory::cstr_first_byte(pat) == b'|' {
            eap.nextcmd = pat.wrapping_add(1);
            pat = c"".as_ptr().cast_mut();
            cmd = c"".as_ptr().cast_mut();
        } else {
            cmd = pat;
            while !crate::memory::cstr_is_empty(cmd)
                && (!ascii_iswhite(crate::memory::cstr_first_char(cmd))
                    || crate::memory::cstr_first_byte(cmd.wrapping_sub(1)) == b'\\')
            {
                cmd = cmd.wrapping_add(1);
            }
            if !crate::memory::cstr_is_empty(cmd) {
                unsafe {
                    *cmd = NUL;
                }
                cmd = cmd.wrapping_add(1);
            }

            if crate::strings::vim_haschr(pat, b'$' as i32)
                || crate::strings::vim_haschr(pat, b'~' as i32)
            {
                let ep = expand_env_save(pat);
                pat = ep.as_ptr().cast_mut();
                envpat = Some(ep);
            }

            cmd = skipwhite(cmd);

            let mut invalid_flags = false;
            for _ in 0..2 {
                if crate::memory::cstr_is_empty(cmd) {
                    continue;
                }
                invalid_flags |= arg_autocmd_flag_get(&mut once, &raw mut cmd, c"++once", 6);
                invalid_flags |= arg_autocmd_flag_get(&mut nested, &raw mut cmd, c"++nested", 8);
                invalid_flags |= arg_autocmd_flag_get(&mut nested, &raw mut cmd, c"nested", 6);
            }

            if invalid_flags {
                break 'run;
            }

            if !crate::memory::cstr_is_empty(cmd) {
                cmd_owned = expand_sfile(cmd);
                match cmd_owned {
                    Some(ref co) => cmd = co.as_ptr().cast_mut(),
                    None => break 'run,
                }
            }
        }

        let is_showing = forceit == 0 && crate::memory::cstr_is_empty(cmd);

        if is_showing {
            msg_ext_set_kind(c"list_cmd");
            msg_puts_title(tr(c"\n--- Autocommands ---"));

            if crate::memory::cstr_first_byte(arg) == b'*'
                || crate::memory::cstr_first_byte(arg) == b'|'
                || crate::memory::cstr_is_empty(arg)
            {
                au_show_for_all_events(group, pat);
            } else {
                let event = event_name2nr(arg, &raw mut arg);
                assert!((event as usize) < NUM_EVENTS);
                au_show_for_event(group, event, pat);
            }
        } else if crate::memory::cstr_first_byte(arg) == b'*'
            || crate::memory::cstr_is_empty(arg)
            || crate::memory::cstr_first_byte(arg) == b'|'
        {
            if !crate::memory::cstr_is_empty(cmd) {
                emsg(tr(E_CANNOT_DEFINE_AUTOCOMMANDS_FOR_ALL_EVENTS));
            } else {
                do_all_autocmd_events(pat, once, i32::from(nested), cmd, forceit != 0, group);
            }
        } else {
            while !crate::memory::cstr_is_empty(arg)
                && crate::memory::cstr_first_byte(arg) != b'|'
                && !ascii_iswhite(crate::memory::cstr_first_char(arg))
            {
                let event = event_name2nr(arg, &raw mut arg);
                assert!((event as usize) < NUM_EVENTS);
                if !do_autocmd_event(
                    event,
                    pat,
                    once,
                    i32::from(nested),
                    cmd,
                    forceit != 0,
                    group,
                ) {
                    break;
                }
            }
        }
    }
    drop(cmd_owned);
    drop(envpat);
}

pub fn do_all_autocmd_events(
    pat: impl crate::strings::IntoCCharPtr,
    once: bool,
    nested: i32,
    cmd: impl crate::strings::IntoCCharPtr,
    del: bool,
    group: i32,
) {
    let cmd = cmd.into_c_char_ptr();
    let pat = pat.into_c_char_ptr();
    for event in 0..NUM_EVENTS as AuEvent {
        if !do_autocmd_event(event, pat, once, nested, cmd, del, group) {
            return;
        }
    }
}

pub fn do_autocmd_event(
    event: AuEvent,
    pat: impl crate::strings::IntoCCharPtr,
    once: bool,
    nested: i32,
    cmd: impl crate::strings::IntoCCharPtr,
    del: bool,
    group: i32,
) -> bool {
    let mut pat = pat.into_c_char_ptr();
    let cmd = cmd.into_c_char_ptr();
    {
        assert!(!crate::memory::cstr_is_empty(pat) || del);

        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];
        let is_adding_cmd = !crate::memory::cstr_is_empty(cmd);
        let findgroup = if group == AUGROUP_ALL {
            au_st().current_augroup
        } else {
            group
        };

        if crate::memory::cstr_is_empty(pat) && del {
            aucmd_del_for_event_and_group(event, findgroup);
            return true;
        }

        let mut patlen = aucmd_span_pattern(pat, &raw mut pat) as i32;
        while patlen != 0 {
            let endpat = pat.wrapping_add(patlen as usize);

            let is_buflocal = aupat_is_buflocal(pat, patlen);
            if is_buflocal {
                let buflocal_nr = aupat_get_buflocal_nr(pat, patlen);
                if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_null() {
                    semsg!(tr(E_BUFFER_NR_INVALID_BUFFER_NUMBER), buflocal_nr,);
                    return false;
                }
                aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, buflocal_nr);
                pat = buflocal_pat.as_ptr();
                patlen = crate::memory::cstr_len(pat) as i32;
            }

            if del {
                assert!(!crate::memory::cstr_is_empty(pat));

                let acs = &mut au_st().autocmds[event as usize];
                for ac in acs {
                    if let Some(ref ap) = ac.pat
                        && ap.group == findgroup
                        && ap.patlen == patlen
                        && crate::memory::cstr_slice(pat, patlen as usize) == ap.pat.as_bytes()
                    {
                        aucmd_del(ac);
                    }
                }
            }

            if is_adding_cmd {
                let mut handler_fn = Callback::NONE;
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
                    &mut handler_fn,
                );
            }

            patlen = aucmd_span_pattern(endpat, &raw mut pat) as i32;
        }

        au_cleanup();
        true
    }
}

pub fn autocmd_register(
    id: i64,
    event: AuEvent,
    pat: impl crate::strings::IntoCCharPtr,
    mut patlen: i32,
    group: i32,
    once: bool,
    nested: bool,
    desc: impl crate::strings::IntoCCharPtr,
    handler_cmd: impl crate::strings::IntoCCharPtr,
    handler_fn: &mut Callback,
) -> bool {
    let desc = desc.into_c_char_ptr();
    let mut pat = pat.into_c_char_ptr();
    let handler_cmd = handler_cmd.into_c_char_ptr();
    {
        assert!(group != 0);

        if (patlen as usize) > crate::memory::cstr_len(pat) {
            return false;
        }

        let findgroup = if group == AUGROUP_ALL {
            au_st().current_augroup
        } else {
            group
        };

        let is_buflocal = aupat_is_buflocal(pat, patlen);
        let mut buflocal_nr: i32 = 0;
        let mut buflocal_pat = [0 as c_char; BUFLOCAL_PAT_LEN];

        if is_buflocal {
            buflocal_nr = aupat_get_buflocal_nr(pat, patlen);
            if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_null() {
                semsg!(tr(E_BUFFER_NR_INVALID_BUFFER_NUMBER), buflocal_nr,);
                return false;
            }
            aupat_normalize_buflocal_pat(buflocal_pat.as_mut_ptr(), pat, patlen, buflocal_nr);
            pat = buflocal_pat.as_ptr();
            patlen = crate::memory::cstr_len(pat) as i32;
        }

        let mut ap: Option<std::rc::Rc<AutoPat>> = None;
        let acs = &mut au_st().autocmds[event as usize];
        for ac in acs.iter().rev() {
            let Some(ref ac_ap) = ac.pat else {
                continue;
            };
            if ac_ap.group == findgroup
                && ac_ap.patlen == patlen
                && crate::memory::cstr_slice(pat, patlen as usize) == ac_ap.pat.as_bytes()
            {
                ap = Some(ac_ap.clone());
            }
            break;
        }

        let ap = if let Some(ap) = ap {
            ap
        } else {
            let mut allow_dirs: c_char = 0;
            let reg_prog = if is_buflocal {
                None
            } else {
                let reg_pat = file_pat_to_reg_pat(
                    pat,
                    pat.wrapping_add(patlen as usize),
                    &raw mut allow_dirs,
                    1,
                );
                let prog = match reg_pat {
                    Some(ref rp) => crate::regexp::vim_regcomp_box(
                        rp.as_ptr().cast_mut(),
                        RE_MAGIC | RE_NOBREAK,
                    ),
                    None => None,
                };
                if reg_pat.is_none() || prog.is_none() {
                    return false;
                }
                prog
            };

            if event == EVENT_MODECHANGED && !has_event(EVENT_MODECHANGED) {
                get_mode((&raw mut g_st().last_mode).cast::<c_char>());
            }

            if (event == EVENT_CURSORMOVED && !has_event(EVENT_CURSORMOVED))
                || (event == EVENT_CURSORMOVEDI && !has_event(EVENT_CURSORMOVEDI))
            {
                g_st().last_cursormoved_win = curwin_ptr();
                g_st().last_cursormoved = curwin_mut().w_cursor;
            }

            if (event == EVENT_WINSCROLLED || event == EVENT_WINRESIZED)
                && !(has_event(EVENT_WINSCROLLED) || has_event(EVENT_WINRESIZED))
            {
                let save_curtab = g_st().curtab;
                for tp in for_all_tabpages_ptr() {
                    unuse_tabpage(curtab_mut());
                    use_tabpage(tp);
                    snapshot_windows_scroll_size();
                }
                unuse_tabpage(curtab_mut());
                use_tabpage(save_curtab);
            }

            let pat_slice = crate::memory::cstr_slice(pat, patlen as usize);
            std::rc::Rc::new(AutoPat {
                pat: CString::new(pat_slice).unwrap(),
                reg_prog,
                group: findgroup,
                patlen,
                buflocal_nr: if is_buflocal { buflocal_nr } else { 0 },
                allow_dirs,
            })
        };

        let mut ac = AutoCmd::new();
        ac.pat = Some(ap);
        ac.id = id;
        if !handler_cmd.is_null() {
            ac.handler = AutoCmdHandler::Cmd(crate::memory::cstr_to_cstring(handler_cmd));
        } else {
            let mut cb = Callback::new();
            callback_copy(&mut cb, handler_fn);
            ac.handler = AutoCmdHandler::Fn(cb);
        }
        ac.script_ctx = g_st().current_sctx;
        let sourcing_lnum = rt_st().exestack.last().map_or(0, |top| top.es_lnum);
        ac.script_ctx.sc_lnum += sourcing_lnum;
        nlua_set_sctx(&mut ac.script_ctx);
        ac.once = once;
        ac.nested = nested;
        ac.desc = if desc.is_null() {
            None
        } else {
            Some(crate::memory::cstr_to_cstring(desc))
        };
        acs.push(ac);

        true
    }
}

pub fn aucmd_span_pattern(
    pat: impl crate::strings::IntoCCharPtr,
    start: *mut *const c_char,
) -> usize {
    let mut pat = pat.into_c_char_ptr();
    {
        while crate::memory::cstr_first_byte(pat) == b',' {
            pat = pat.wrapping_add(1);
        }

        let mut p = pat;
        let mut brace_level: i32 = 0;
        while !crate::memory::cstr_is_empty(p)
            && (crate::memory::cstr_first_byte(p) != b','
                || brace_level != 0
                || (p > pat && unsafe { *p.wrapping_sub(1) } == b'\\' as c_char))
        {
            if crate::memory::cstr_first_byte(p) == b'{' {
                brace_level += 1;
            } else if crate::memory::cstr_first_byte(p) == b'}' {
                brace_level -= 1;
            }
            p = p.wrapping_add(1);
        }

        unsafe {
            *start = pat;
        }
        crate::memory::ptr_offset_from(p, pat) as usize
    }
}

pub fn do_doautocmd(
    arg_start: impl crate::strings::IntoCCharPtr,
    do_msg: bool,
    mut did_something: Option<&mut bool>,
) -> bool {
    let arg_start = arg_start.into_c_char_ptr().cast_mut();
    {
        let mut arg = arg_start;
        let mut nothing_done = true;

        if let Some(ref mut p) = did_something {
            **p = false;
        }

        let group = arg_augroup_get(&raw mut arg);

        if crate::memory::cstr_first_byte(arg) == b'*' {
            emsg(tr(c"E217: Can't execute autocommands for ALL events"));
            return false;
        }

        let mut fname = arg_event_skip(arg, group != AUGROUP_ALL);
        if fname.is_null() {
            return false;
        }

        fname = skipwhite(fname);

        while !crate::memory::cstr_is_empty(arg)
            && ends_excmd(i32::from(unsafe { *arg.cast::<u8>() })) == 0
            && !ascii_iswhite(crate::memory::cstr_first_char(arg))
        {
            let ev = event_name2nr(arg, &raw mut arg);
            if apply_autocmds_group(
                ev,
                fname,
                ptr::null_mut(),
                true,
                group,
                g_st().curbuf,
                Some(crate::globals::curwin_mut()),
                None,
                ptr::null_mut(),
                false,
            ) {
                nothing_done = false;
            }
        }

        if nothing_done && do_msg && !aborting() {
            smsg!(0, tr(c"No matching autocommands: %s"), arg_start);
        }
        if let Some(ref mut p) = did_something {
            **p = !nothing_done;
        }

        !aborting()
    }
}

pub fn ex_doautoall(eap: &mut Exarg) {
    let mut retval = OK;
    let mut aco: CtxSwitch = CtxSwitch::new();
    let mut arg = eap.arg;
    let call_do_modelines = check_nomodeline(&raw mut arg);
    let mut bufref: Bufref = Bufref::new();
    let mut did_aucmd = false;

    for buf in for_all_buffers_ptr() {
        if buf.b_ml.ml_mfp.is_none() || std::ptr::eq(buf, g_st().curbuf) {
            continue;
        }

        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        set_bufref(&mut bufref, buf);

        let ok = do_doautocmd(arg, false, Some(&mut did_aucmd));
        retval = if ok { OK } else { FAIL };

        if call_do_modelines && did_aucmd {
            do_modelines(if is_ctx_win(curwin_ptr()) {
                OPT_NOWIN
            } else {
                0
            });
        }

        ctx_restore(&mut aco);

        if !ok || !bufref_valid(Some(&bufref)) {
            retval = FAIL;
            break;
        }
    }

    if retval == OK {
        do_doautocmd(arg, false, Some(&mut did_aucmd));
        if call_do_modelines && did_aucmd {
            do_modelines(0);
        }
    }
}

pub fn check_nomodeline(argp: *mut *mut c_char) -> bool {
    unsafe {
        let bytes = crate::memory::cstr_as_bytes(*argp);
        if bytes.starts_with(b"<nomodeline>") {
            *argp = skipwhite((*argp).wrapping_add(12));
            return false;
        }
        true
    }
}

pub fn aucmd_defer(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    group: i32,
    buf: &mut FileBuf,
    data: Option<&mut Object>,
) {
    let fname = fname.into_c_char_ptr();
    let fname_io = fname_io.into_c_char_ptr();
    {
        let evdata = Box::new(AutoCmdEvent {
            event,
            fname: (!fname.is_null()).then(|| crate::memory::cstr_to_cstring(fname)),
            fname_io: (!fname_io.is_null()).then(|| crate::memory::cstr_to_cstring(fname_io)),
            group,
            buf: buf.b_fnum,
            data: (data.is_some()).then(|| copy_object(*data.unwrap(), None)),
        });

        multiqueue_put_event(
            unsafe { &mut *g_st().deferred_events },
            Event::AutoCmdDeferred(evdata),
        );
    }
}

pub fn deferred_event(mut e: AutoCmdEvent) {
    unsafe {
        let event = e.event;
        let fname = e
            .fname
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let fname_io = e
            .fname_io
            .as_ref()
            .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
        let group = e.group;
        let data = e.data.as_mut().map_or(ptr::null_mut(), |d| &raw mut *d);

        let mut err = Error::new();
        let buf = find_buffer_by_handle(e.buf, &mut err);
        if !buf.is_null() {
            let mut save_v_event: SaveVEvent = SaveVEvent::new();
            let v_event = get_v_event(&mut save_v_event);
            if !data.is_null() && (*data).obj_type == ObjectType::ObjectTypeDict {
                for i in 0..(*data).data.dict().size {
                    let item = (*data).data.dict()[i];
                    let mut tv: TypVal = TypVal::new();
                    object_to_vim(item.value, &mut tv, &mut err);
                    if err.err_type != ErrorType::ErrorTypeNone {
                        api_clear_error(&mut err);
                        continue;
                    }
                    tv_dict_add_tv(&mut *v_event, item.key.data, item.key.size, &mut tv);
                    tv_clear(&mut tv);
                }
            }
            tv_dict_set_keys_readonly(&mut *v_event);

            let win = curwin_ptr();
            let mut aco: CtxSwitch = CtxSwitch::new();
            ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            apply_autocmds_group(
                event,
                fname,
                fname_io,
                false,
                group,
                buf,
                win.as_ref(),
                None,
                data,
                false,
            );
            ctx_restore(&mut aco);

            restore_v_event(&mut *v_event, &mut save_v_event);
        }
    }
}

pub fn deferred_optionset_modified(bufnr: i32, new_val: bool) {
    let mut err = Error::new();
    let buf = find_buffer_by_handle(bufnr, &mut err);
    api_clear_error(&mut err);
    if !buf.is_null() {
        let old = boolean_obj(!new_val);
        let new = boolean_obj(new_val);
        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
        apply_optionset_autocmd_now(K_OPT_MODIFIED, OPT_LOCAL, old, old, old, new);
        ctx_restore(&mut aco);
    }
}

pub fn aucmd_defer_modified(buf: &mut FileBuf, new_val: bool) {
    if !has_event(EVENT_OPTIONSET)
        || (bt_nofile(Some(buf)) && buf.b_p_bl == 0)
        || !crate::memory::cstr_is_empty(get_vim_var_str(VV_OPTION_TYPE))
    {
        return;
    }
    multiqueue_put_event(
        unsafe { &mut *g_st().deferred_events },
        Event::AutoCmdOptionSetModified {
            bufnr: buf.b_fnum,
            new_val,
        },
    );
}

pub fn apply_autocmds(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    force: bool,
    buf: *mut FileBuf,
) -> bool {
    let fname_io = fname_io.into_c_char_ptr();
    let fname = fname.into_c_char_ptr().cast_mut();
    {
        apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            Some(crate::globals::curwin_mut()),
            None,
            ptr::null_mut(),
            false,
        )
    }
}

pub fn apply_autocmds_win(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    force: bool,
    buf: *mut FileBuf,
    win: Option<&WinInfoInfo>,
) -> bool {
    let fname = fname.into_c_char_ptr();
    let fname_io = fname_io.into_c_char_ptr();
    apply_autocmds_group(
        event,
        fname,
        fname_io,
        force,
        AUGROUP_ALL,
        buf,
        win,
        None,
        ptr::null_mut(),
        false,
    )
}

pub fn apply_autocmds_exarg(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    force: bool,
    buf: *mut FileBuf,
    eap: Option<&Exarg>,
) -> bool {
    let fname = fname.into_c_char_ptr();
    let fname_io = fname_io.into_c_char_ptr();
    {
        apply_autocmds_group(
            event,
            fname,
            fname_io,
            force,
            AUGROUP_ALL,
            buf,
            Some(crate::globals::curwin_mut()),
            (eap.is_some()).then(|| eap.unwrap()),
            ptr::null_mut(),
            false,
        )
    }
}

pub fn apply_autocmds_retval(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    force: bool,
    buf: *mut FileBuf,
    retval: &mut i32,
) -> bool {
    let fname = fname.into_c_char_ptr();
    let fname_io = fname_io.into_c_char_ptr();
    {
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
            Some(crate::globals::curwin_mut()),
            None,
            ptr::null_mut(),
            false,
        );
        if did_cmd && aborting() {
            *retval = FAIL;
        }
        did_cmd
    }
}

pub fn has_event(event: AuEvent) -> bool {
    let acs = &au_st().autocmds[event as usize];
    !acs.is_empty()
}

fn has_cursorhold() -> bool {
    has_event(if get_real_state() == MODE_NORMAL_BUSY {
        EVENT_CURSORHOLD
    } else {
        EVENT_CURSORHOLDI
    })
}

pub fn trigger_cursorhold() -> bool {
    if !g_st().did_cursorhold
        && has_cursorhold()
        && g_st().reg_recording == 0
        && g_st().typebuf.tb_len == 0
        && !ins_compl_active()
    {
        let state = get_real_state();
        if state == MODE_NORMAL_BUSY || (state & MODE_INSERT) != 0 {
            return true;
        }
    }
    false
}

pub fn apply_autocmds_group(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    fname_io: impl crate::strings::IntoCCharPtr,
    force: bool,
    group: i32,
    buf: *mut FileBuf,
    win: Option<&WinInfoInfo>,
    eap: Option<&Exarg>,
    data: *mut Object,
    with_buf: bool,
) -> bool {
    let mut fname = fname.into_c_char_ptr();
    let fname_io = fname_io.into_c_char_ptr();
    unsafe {
        let mut retval = false;
        let mut wait_time: Proftime = 0;
        let mut did_save_redobuff = false;
        let mut save_redo = RedoState::new();
        let save_key_typed = g_st().key_typed;
        let mut aco: CtxSwitch = CtxSwitch::new();

        'bypass_au: {
            if event == NUM_EVENTS as AuEvent || !has_event(event) || is_autocmd_blocked() {
                break 'bypass_au;
            }

            if g_st().autocmd_busy && !(force || au_st().autocmd_nested) {
                break 'bypass_au;
            }

            if aborting() {
                break 'bypass_au;
            }

            if au_st().filechangeshell_busy
                && (event == EVENT_FILECHANGEDSHELL || event == EVENT_FILECHANGEDSHELLPOST)
            {
                break 'bypass_au;
            }

            if event_ignored(event, opt_vars().p_ei) {
                break 'bypass_au;
            }

            let mut win_ignore = false;
            if buf == g_st().curbuf && EVENT_NAMES[event as usize].event <= 0 {
                win_ignore = event_ignored(event, curwin_mut().w_onebuf_opt.wo_eiw);
            } else if !buf.is_null()
                && EVENT_NAMES[event as usize].event <= 0
                && (*buf).b_nwindows > 0
            {
                win_ignore = true;
                for tp in for_all_tabpages_ptr() {
                    for wp in for_all_windows_in_tab_mut(&*tp) {
                        if wp.w_buffer == buf && !event_ignored(event, wp.w_onebuf_opt.wo_eiw) {
                            win_ignore = false;
                            break;
                        }
                    }
                }
            }
            if win_ignore {
                break 'bypass_au;
            }

            if au_st().apply_autocmds_nesting == 10 {
                emsg(tr(E_AUTOCOMMAND_NESTING_TOO_DEEP));
                break 'bypass_au;
            }

            if (g_st().autocmd_no_enter != 0
                && (event == EVENT_WINENTER || event == EVENT_BUFENTER))
                || (g_st().autocmd_no_leave != 0
                    && (event == EVENT_WINLEAVE || event == EVENT_BUFLEAVE))
            {
                break 'bypass_au;
            }

            let save_autocmd_fname = g_st().autocmd_fname.take();
            let save_autocmd_fname_full = g_st().autocmd_fname_full;
            let save_autocmd_bufnr = g_st().autocmd_bufnr;
            let save_autocmd_winid = au_st().autocmd_winid;
            let save_autocmd_match = g_st().autocmd_match.take();
            let save_autocmd_busy = g_st().autocmd_busy;
            let save_autocmd_nested = au_st().autocmd_nested;

            let raw_autocmd_fname = if fname_io.is_null() {
                if event == EVENT_COLORSCHEME
                    || event == EVENT_COLORSCHEMEPRE
                    || event == EVENT_OPTIONSET
                    || event == EVENT_MODECHANGED
                    || event == EVENT_MARKSET
                {
                    ptr::null_mut()
                } else if !fname.is_null() && ends_excmd(i32::from(*fname.cast::<u8>())) == 0 {
                    fname
                } else if !buf.is_null() {
                    (*buf).b_ffname
                } else {
                    ptr::null_mut()
                }
            } else {
                fname_io
            };
            let afile_orig_owned = if !raw_autocmd_fname.is_null() {
                let orig = crate::memory::cstr_to_cstring(raw_autocmd_fname);
                g_st().autocmd_fname = Some(orig.clone());
                Some(orig)
            } else {
                g_st().autocmd_fname = None;
                None
            };
            g_st().autocmd_fname_full = false;

            g_st().autocmd_bufnr = if buf.is_null() { 0 } else { (*buf).b_fnum };

            let mut sfname_owned: Option<CString> = None;
            let fname_owned: Option<CString> = if crate::memory::cstr_is_empty(fname) {
                if buf.is_null() {
                    fname = ptr::null_mut();
                } else if event == EVENT_SYNTAX {
                    fname = (*buf).b_p_syn;
                } else if event == EVENT_FILETYPE {
                    fname = (*buf).b_p_ft;
                } else {
                    if !(*buf).b_sfname.is_null() {
                        sfname_owned = Some(crate::memory::cstr_to_cstring((*buf).b_sfname));
                    }
                    fname = (*buf).b_ffname;
                }
                if fname.is_null() {
                    fname = c"".as_ptr().cast_mut();
                }
                Some(crate::memory::cstr_to_cstring(fname))
            } else {
                sfname_owned = to_slash_save(fname);
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
                    g_st().autocmd_fname_full = true;
                    to_slash_save(fname)
                } else {
                    FullName_save(fname, false)
                }
            };
            fname = fname_owned
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
            if fname.is_null() {
                retval = false;
                break 'bypass_au;
            }

            au_st().autocmd_winid = if let Some(win) = win {
                win.handle
            } else if !(curwin_ptr()).is_null() {
                curwin_mut().handle
            } else {
                0
            };

            if with_buf && !buf.is_null() && buf != g_st().curbuf {
                ctx_switch(&mut aco, ptr::null_mut(), ptr::null_mut(), buf, 0);
            }

            let save_changed = curbuf_mut().b_changed;
            let old_curbuf = g_st().curbuf;

            g_st().autocmd_match =
                (!fname.is_null()).then(|| crate::memory::cstr_to_cstring(fname));

            g_st().redrawing_disabled += 1;

            estack_push(ETYPE_AUCMD, ptr::null(), 0);

            let save_current_sctx = g_st().current_sctx;

            if g_st().do_profiling == PROF_YES {
                prof_child_enter(&mut wait_time);
            }

            save_funccal();

            if !g_st().autocmd_busy {
                save_search_patterns();
                if !ins_compl_active() {
                    save_redobuff(&mut save_redo);
                    did_save_redobuff = true;
                }
                curbuf_mut().b_did_filetype = curbuf_mut().b_keep_filetype;
            }

            g_st().autocmd_busy = true;
            au_st().filechangeshell_busy = event == EVENT_FILECHANGEDSHELL;
            au_st().apply_autocmds_nesting += 1;

            if event == EVENT_FILETYPE && !crate::memory::cstr_is_empty(curbuf_mut().b_p_ft) {
                curbuf_mut().b_did_filetype = true;
            }

            let acs = &au_st().autocmds[event as usize];
            let mut patcmd = AutoPatCmd {
                lastpat: None,
                auidx: 0,
                ausize: acs.len(),
                afile_orig: afile_orig_owned,
                fname: fname_owned,
                sfname: sfname_owned,
                group,
                event,
                script_ctx: std::rc::Rc::new(std::cell::Cell::new(Sctx {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                })),
                arg_bufnr: std::rc::Rc::new(std::cell::Cell::new(g_st().autocmd_bufnr)),
                data: None,
            };
            aucmd_next(&mut patcmd);

            if patcmd.lastpat.is_some() {
                au_st().active_apc_list.push(patcmd.arg_bufnr.clone());
                patcmd.data = (!data.is_null()).then(|| *data);

                let save_cmdbang = get_vim_var_nr(VV_CMDBANG);
                let save_cmdarg = if let Some(eap) = eap {
                    let s = set_cmdarg(Some(eap), None);
                    set_vim_var_nr(VV_CMDBANG, Varnumber::from(eap.forceit));
                    s
                } else {
                    None
                };
                retval = true;

                if au_st().apply_autocmds_nesting == 1 {
                    check_lnums(true);
                } else {
                    check_lnums_nested(true);
                }

                let save_did_emsg = g_st().did_emsg;

                do_cmdline(
                    ptr::null_mut(),
                    crate::cmdexpand::LineGetter::AutoCmd(&raw mut patcmd),
                    DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
                );

                g_st().did_emsg += save_did_emsg;

                if au_st().apply_autocmds_nesting == 1 {
                    reset_lnums();
                }

                if eap.is_some() {
                    set_cmdarg(None, save_cmdarg);
                    set_vim_var_nr(VV_CMDBANG, save_cmdbang);
                }
                if au_st()
                    .active_apc_list
                    .last()
                    .is_some_and(|last| std::rc::Rc::ptr_eq(last, &patcmd.arg_bufnr))
                {
                    au_st().active_apc_list.pop();
                }
            }

            g_st().redrawing_disabled -= 1;
            g_st().autocmd_busy = save_autocmd_busy;
            au_st().filechangeshell_busy = false;
            au_st().autocmd_nested = save_autocmd_nested;
            estack_pop();
            g_st().autocmd_fname = save_autocmd_fname;
            g_st().autocmd_fname_full = save_autocmd_fname_full;
            g_st().autocmd_bufnr = save_autocmd_bufnr;
            au_st().autocmd_winid = save_autocmd_winid;
            g_st().autocmd_match = save_autocmd_match;
            g_st().current_sctx = save_current_sctx;
            restore_funccal();
            if g_st().do_profiling == PROF_YES {
                prof_child_exit(&mut wait_time);
            }
            g_st().key_typed = save_key_typed;
            drop(patcmd);
            au_st().apply_autocmds_nesting -= 1;

            if !g_st().autocmd_busy {
                restore_search_patterns();
                if did_save_redobuff {
                    restore_redobuff(&mut save_redo);
                }
                curbuf_mut().b_did_filetype = false;
                g_st().au_pending_free_buf.clear();
                g_st().au_pending_free_win.clear();
            }

            if g_st().curbuf == old_curbuf
                && (event == EVENT_BUFREADPOST
                    || event == EVENT_BUFWRITEPOST
                    || event == EVENT_FILEAPPENDPOST
                    || event == EVENT_VIMLEAVE
                    || event == EVENT_VIMLEAVEPRE)
            {
                if curbuf_mut().b_changed != save_changed {
                    g_st().need_maketitle = true;
                }
                curbuf_mut().b_changed = save_changed;
            }

            au_cleanup();
        }

        if event == EVENT_BUFWIPEOUT && !buf.is_null() {
            aubuflocal_remove(&mut *buf);
        }

        if retval && event == EVENT_FILETYPE {
            curbuf_mut().b_au_did_filetype = true;
        }

        ctx_restore(&mut aco);

        retval
    }
}

pub fn do_termresponse_autocmd(sequence: NvimString, channel_id: u64) {
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
        Some(crate::globals::curwin_mut()),
        None,
        &raw mut data_obj,
        false,
    );
    au_st().termresponse_changed = true;
    au_st().termresponse_chan_id = channel_id;
}

#[unsafe(no_mangle)]
pub extern "C" fn block_autocmds() {
    if !is_autocmd_blocked() {
        au_st().termresponse_changed = false;
        au_st().termresponse_chan_id = 0;
    }
    au_st().autocmd_blocked += 1;
}

#[unsafe(no_mangle)]
pub extern "C" fn unblock_autocmds() {
    au_st().autocmd_blocked -= 1;

    if !is_autocmd_blocked() && au_st().termresponse_changed && has_event(EVENT_TERMRESPONSE) {
        let sequence = cstr_to_string(get_vim_var_str(VV_TERMRESPONSE));
        do_termresponse_autocmd(sequence, au_st().termresponse_chan_id);
        api_free_string(sequence);
    }
}

pub fn is_autocmd_blocked() -> bool {
    au_st().autocmd_blocked != 0
}

fn aucmd_next(apc: &mut AutoPatCmd) {
    let entry = rt_st().exestack.last_mut().unwrap();

    let acs = &mut au_st().autocmds[apc.event as usize];
    assert!(apc.ausize <= acs.len());
    let mut i = apc.auidx;
    while i < apc.ausize && !g_st().got_int {
        let ac = &(acs)[i];
        let Some(ref ap) = ac.pat else {
            i += 1;
            continue;
        };
        if !apc
            .lastpat
            .as_ref()
            .is_some_and(|l| std::rc::Rc::ptr_eq(l, ap))
        {
            if apc.group != AUGROUP_ALL && apc.group != ap.group {
                i += 1;
                continue;
            }
            let matches = if ap.buflocal_nr == 0 {
                let fname_ptr = apc
                    .fname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                let sfname_ptr = apc
                    .sfname
                    .as_ref()
                    .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                let tail_ptr = if fname_ptr.is_null() {
                    ptr::null_mut()
                } else {
                    path_tail(fname_ptr)
                };
                match_file_pat(
                    ptr::null_mut(),
                    ap.reg_prog.as_deref(),
                    fname_ptr,
                    sfname_ptr,
                    tail_ptr,
                    i32::from(ap.allow_dirs),
                )
            } else {
                ap.buflocal_nr == apc.arg_bufnr.get()
            };
            if !matches {
                i += 1;
                continue;
            }

            let name = event_nr2name(apc.event);
            let name_str = crate::memory::cstr_lossy(name);
            let pat_str = ap.pat.to_string_lossy();
            let formatted = format!("{name_str} Autocommands for \"{pat_str}\"");
            let name_cstr = CString::new(formatted).unwrap_or_default();
            if opt_vars().p_verbose >= 8 {
                verbose_enter();
                smsg!(0, tr(c"Executing %s"), name_cstr.as_ptr());
                verbose_leave();
            }

            entry.es_name = Some(name_cstr);
            entry.es_info = crate::profile::EStackInfo::AuCmd(apc.script_ctx.clone());
        }

        apc.lastpat = Some(ap.clone());
        apc.auidx = i;

        line_breakcheck();
        return;
    }

    entry.es_name = None;
    entry.es_info = crate::profile::EStackInfo::None;

    apc.lastpat = None;
    apc.auidx = usize::MAX;
}

fn au_callback(mut callback: Callback, id: i64, group: i32, apc: &AutoPatCmd) -> bool {
    if callback.cb_type() == K_CALLBACK_LUA {
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
            value: integer_obj(id),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"event"),
            value: cstr_as_obj(event_nr2name(apc.event)),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"file"),
            value: cstr_as_obj(apc.afile_orig.as_ref().map_or(ptr::null(), |s| s.as_ptr())),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"match"),
            value: cstr_as_obj(
                unsafe { (&raw const g_st().autocmd_match).as_ref() }
                    .unwrap()
                    .as_ref()
                    .map_or(ptr::null(), |s| s.as_ptr()),
            ),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"buf"),
            value: integer_obj(Integer::from(g_st().autocmd_bufnr)),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"win"),
            value: integer_obj(Integer::from(au_st().autocmd_winid)),
        };
        size += 1;

        if let Some(data_val) = apc.data {
            data_items[size] = KeyValuePair {
                key: static_cstr_as_string(c"data"),
                value: data_val,
            };
            size += 1;
        }

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

        let luaref = callback.data.as_luaref();
        let result = nlua_call_ref(luaref, ptr::null(), args, K_RET_NIL_BOOL, None, None);
        result.obj_type == ObjectType::ObjectTypeBoolean && result.data.boolean()
    } else {
        let mut argsin: TypVal = TypVal::new();
        let mut rettv: TypVal = TypVal::new();
        callback_call(&mut callback, 0, &raw mut argsin, &mut rettv);
        false
    }
}

pub fn getnextac(_c: i32, apc: *mut AutoPatCmd, _indent: i32, _do_concat: bool) -> Option<CString> {
    let apc = unsafe { &mut *apc };
    let acs = &mut au_st().autocmds[apc.event as usize];
    aucmd_next(apc);
    apc.lastpat.as_ref()?;

    assert!(apc.auidx < acs.len());
    let auidx = apc.auidx;
    let ac = &mut (acs)[auidx];
    assert!(ac.pat.is_some());
    let mut oneshot = ac.once;

    if opt_vars().p_verbose >= 9 {
        verbose_enter_scroll();
        let handler_str = aucmd_handler_to_string(ac);
        smsg!(0, tr(c"autocommand %s"), handler_str.as_ptr());
        msg_puts(c"\n");
        verbose_leave_scroll();
    }

    au_st().autocmd_nested = ac.nested;
    g_st().current_sctx = ac.script_ctx;
    apc.script_ctx.set(g_st().current_sctx);

    let retval = match &ac.handler {
        AutoCmdHandler::Cmd(cmd) => cmd.clone(),
        AutoCmdHandler::Fn(cb) => {
            let cb = cb.clone();
            let id = ac.id;
            let group = ac.pat.as_ref().unwrap().group;
            let saved_pat = if oneshot { ac.pat.take() } else { None };
            let rv = au_callback(cb, id, group, apc);
            if oneshot {
                (acs)[auidx].pat = saved_pat;
            }
            oneshot = oneshot || rv;
            CString::default()
        }
        AutoCmdHandler::None => {
            let id = ac.id;
            let group = ac.pat.as_ref().unwrap().group;
            let saved_pat = if oneshot { ac.pat.take() } else { None };
            let rv = au_callback(Callback::NONE, id, group, apc);
            if oneshot {
                (acs)[auidx].pat = saved_pat;
            }
            oneshot = oneshot || rv;
            CString::default()
        }
    };

    if oneshot {
        aucmd_del(&mut (acs)[auidx]);
    }

    if apc.auidx < apc.ausize {
        apc.auidx += 1;
    } else {
        apc.auidx = usize::MAX;
    }

    Some(retval)
}

fn has_autocmd_ext(
    event: AuEvent,
    sfname: impl crate::strings::IntoCCharPtr,
    mut buf: Option<&mut FileBuf>,
    is_full: bool,
) -> bool {
    let sfname = sfname.into_c_char_ptr();
    {
        if !has_event(event) {
            return false;
        }

        let tail = path_tail(sfname);
        let mut retval = false;

        let fname = if is_full {
            (!sfname.is_null()).then(|| crate::memory::cstr_to_cstring(sfname))
        } else {
            FullName_save(sfname, false)
        };
        let Some(fname) = fname else {
            return false;
        };

        let acs = &mut au_st().autocmds[event as usize];
        for ac in acs {
            if let Some(ref ap) = ac.pat {
                let matched = if ap.buflocal_nr == 0 {
                    match_file_pat(
                        ptr::null_mut(),
                        ap.reg_prog.as_deref(),
                        fname.as_ptr().cast_mut(),
                        sfname,
                        tail,
                        i32::from(ap.allow_dirs),
                    )
                } else {
                    buf.is_some() && ap.buflocal_nr == buf.as_deref_mut().unwrap().b_fnum
                };
                if matched {
                    retval = true;
                    break;
                }
            }
        }

        retval
    }
}

pub fn has_autocmd(
    event: AuEvent,
    sfname: impl crate::strings::IntoCCharPtr,
    buf: Option<&mut FileBuf>,
) -> bool {
    let sfname = sfname.into_c_char_ptr();
    has_autocmd_ext(event, sfname, buf, false)
}

pub fn has_autocmd_fullname(
    event: AuEvent,
    fname: impl crate::strings::IntoCCharPtr,
    buf: Option<&mut FileBuf>,
) -> bool {
    let fname = fname.into_c_char_ptr();
    has_autocmd_ext(event, fname, buf, true)
}

pub fn expand_get_augroup_name(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    augroup_name(idx + 1)
}

pub fn aucmd_set_expand_context(
    xp: &mut ExpandCtx,
    mut arg: *mut c_char,
    doautocmd: bool,
) -> *mut c_char {
    let xp_pref = (&raw mut *xp).cast::<ExpandPrefix>();
    let xp_pref = { unsafe { &mut *xp_pref } };
    au_st().autocmd_include_groups = false;
    let mut p = arg;
    let mut group = arg_augroup_get(&raw mut arg);

    if crate::memory::cstr_is_empty(arg)
        && group != AUGROUP_ALL
        && !ascii_iswhite(unsafe { *arg.wrapping_sub(1) })
    {
        arg = p;
        group = AUGROUP_ALL;
    }

    p = arg;
    while !crate::memory::cstr_is_empty(p) && !ascii_iswhite(crate::memory::cstr_first_char(p)) {
        if crate::memory::cstr_first_byte(p) == b',' {
            arg = p.wrapping_add(1);
        }
        p = p.wrapping_add(1);
    }
    if crate::memory::cstr_is_empty(p) {
        if group == AUGROUP_ALL {
            au_st().autocmd_include_groups = true;
        }
        xp_pref.xp_context = EXPAND_EVENTS;
        xp_pref.xp_pattern = arg;
        return ptr::null_mut();
    }

    arg = skipwhite(p);
    while !crate::memory::cstr_is_empty(arg)
        && (!ascii_iswhite(crate::memory::cstr_first_char(arg))
            || crate::memory::cstr_first_byte(arg.wrapping_sub(1)) == b'\\')
    {
        arg = arg.wrapping_add(1);
    }
    if !crate::memory::cstr_is_empty(arg) {
        arg = skipwhite(arg);
        for _ in 0..2 {
            let bytes = crate::memory::cstr_as_bytes(arg);
            if bytes.starts_with(b"++once") && ascii_iswhite(crate::memory::cstr_char_at(arg, 6)) {
                arg = skipwhite(arg.wrapping_add(6));
            }
            let bytes = crate::memory::cstr_as_bytes(arg);
            if bytes.starts_with(b"++nested") && ascii_iswhite(crate::memory::cstr_char_at(arg, 8))
            {
                arg = skipwhite(arg.wrapping_add(8));
            }
            let bytes = crate::memory::cstr_as_bytes(arg);
            if bytes.starts_with(b"nested") && ascii_iswhite(crate::memory::cstr_char_at(arg, 6)) {
                arg = skipwhite(arg.wrapping_add(6));
            }
        }
        return arg;
    }

    xp_pref.xp_context = if doautocmd {
        EXPAND_FILES
    } else {
        EXPAND_NOTHING
    };
    ptr::null_mut()
}

pub fn expand_get_event_name(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    let name = augroup_name(idx + 1);
    if !name.is_null() {
        if !au_st().autocmd_include_groups || ptr::eq(name.cast_const(), get_deleted_augroup()) {
            return c"".as_ptr().cast_mut();
        }
        return name;
    }

    let i = idx - au_st().next_augroup_id;
    if i < 0 || (i as usize) >= NUM_EVENTS {
        return ptr::null_mut();
    }

    EVENT_NAMES[i as usize].name.as_ptr().cast_mut()
}

pub fn get_event_name_no_group(_xp: &mut ExpandCtx, idx: i32, win: bool) -> *mut c_char {
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

pub fn autocmd_supported(event: impl crate::strings::IntoCCharPtr) -> bool {
    let event = event.into_c_char_ptr();
    let mut p: *mut c_char = ptr::null_mut();
    event_name2nr(event, &raw mut p) != NUM_EVENTS as AuEvent
}

pub fn au_exists(arg: impl crate::strings::IntoCCharPtr) -> bool {
    let arg = arg.into_c_char_ptr();
    {
        let mut buflocal_buf: *mut FileBuf = ptr::null_mut();
        let mut retval = false;

        let mut arg_save_vec: Vec<c_char> = crate::memory::cstr_to_cchar_vec(arg);
        let arg_save = arg_save_vec.as_mut_ptr();
        let mut p = vim_strchr(arg_save, b'#' as i32);
        if !p.is_null() {
            unsafe {
                *p = NUL;
            }
            p = p.wrapping_add(1);
        }

        let mut group = augroup_find(arg_save);
        let event_name = if group == AUGROUP_ERROR {
            group = AUGROUP_ALL;
            arg_save
        } else {
            if p.is_null() {
                return true;
            }
            let ev_name = p;
            p = vim_strchr(ev_name, b'#' as i32);
            if !p.is_null() {
                unsafe {
                    *p = NUL;
                }
                p = p.wrapping_add(1);
            }
            ev_name
        };

        let pattern = p;
        let event = event_name2nr(event_name, &raw mut p);
        if event != NUM_EVENTS as AuEvent {
            let acs = &mut au_st().autocmds[event as usize];
            if !acs.is_empty() {
                if !pattern.is_null()
                    && crate::memory::cstr_as_bytes(pattern).eq_ignore_ascii_case(b"<buffer>")
                {
                    buflocal_buf = g_st().curbuf;
                }

                for ac in acs {
                    if let Some(ref ap) = ac.pat
                        && (group == AUGROUP_ALL || ap.group == group)
                    {
                        let pat_match = if pattern.is_null() {
                            true
                        } else if buflocal_buf.is_null() {
                            mb_strcmp_ic(opt_vars().p_fic != 0, ap.pat.as_ptr(), pattern) == 0
                        } else {
                            ap.buflocal_nr == unsafe { (*buflocal_buf).b_fnum }
                        };
                        if pat_match {
                            retval = true;
                            break;
                        }
                    }
                }
            }
        }

        retval
    }
}

pub fn aupat_is_buflocal(pat: impl crate::strings::IntoCCharPtr, patlen: i32) -> bool {
    let pat = pat.into_c_char_ptr();
    if patlen < 8 {
        return false;
    }
    let slice = { crate::memory::cstr_slice(pat, patlen as usize) };
    slice.starts_with(b"<buffer") && slice[slice.len() - 1] == b'>'
}

pub fn aupat_get_buflocal_nr(pat: impl crate::strings::IntoCCharPtr, patlen: i32) -> i32 {
    let pat = pat.into_c_char_ptr();
    {
        assert!(aupat_is_buflocal(pat, patlen));

        if patlen == 8 {
            return curbuf_mut().b_fnum;
        }

        if patlen > 9 && crate::memory::cstr_byte_at(pat, 7) == b'=' {
            if patlen == 13
                && crate::memory::cstr_prefix_bytes(pat, 13).eq_ignore_ascii_case(b"<buffer=abuf>")
            {
                return g_st().autocmd_bufnr;
            }

            if skipdigits(pat.wrapping_add(8)) == pat.wrapping_add((patlen - 1) as usize).cast_mut()
            {
                let mut p = pat.wrapping_add(8).cast_mut();
                return getdigits_int(&raw mut p, false, 0);
            }
        }

        0
    }
}

pub fn aupat_normalize_buflocal_pat(
    dest: *mut c_char,
    pat: impl crate::strings::IntoCCharPtr,
    patlen: i32,
    mut buflocal_nr: i32,
) {
    let pat = pat.into_c_char_ptr();
    {
        assert!(aupat_is_buflocal(pat, patlen));

        if buflocal_nr == 0 {
            buflocal_nr = curbuf_mut().b_fnum;
        }

        let s = format!("<buffer={buflocal_nr}>");
        let copy_len = s.len().min(BUFLOCAL_PAT_LEN - 1);
        unsafe { ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), dest, copy_len) };
        unsafe {
            *dest.wrapping_add(copy_len) = NUL;
        }
    }
}

pub fn autocmd_delete_id(id: i64) -> bool {
    assert!(id > 0);
    let mut success = false;

    for acs in &mut au_st().autocmds {
        for ac in acs {
            if ac.id == id {
                aucmd_del(ac);
                success = true;
            }
        }
    }
    success
}

pub fn aucmd_handler_to_string(ac: &mut AutoCmd) -> CString {
    match &mut ac.handler {
        AutoCmdHandler::Cmd(cmd) => cmd.clone(),
        AutoCmdHandler::Fn(cb) => callback_to_string(cb),
        AutoCmdHandler::None => CString::default(),
    }
}

fn arg_event_skip(arg: impl crate::strings::IntoCCharPtr, have_group: bool) -> *mut c_char {
    let arg = arg.into_c_char_ptr().cast_mut();
    {
        let mut pat: *mut c_char;
        let mut p: *mut c_char = ptr::null_mut();

        if crate::memory::cstr_first_byte(arg) == b'*' {
            if crate::memory::cstr_byte_at(arg, 1) != 0
                && !ascii_iswhite(crate::memory::cstr_char_at(arg, 1))
            {
                semsg!(tr(c"E215: Illegal character after *: %s"), arg);
                return ptr::null_mut();
            }
            pat = arg.wrapping_add(1);
        } else {
            pat = arg;
            while !crate::memory::cstr_is_empty(pat)
                && crate::memory::cstr_first_byte(pat) != b'|'
                && !ascii_iswhite(crate::memory::cstr_first_char(pat))
            {
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

fn arg_augroup_get(argp: *mut *mut c_char) -> i32 {
    let arg = unsafe { *argp };
    let mut p = arg;
    while !crate::memory::cstr_is_empty(p)
        && !ascii_iswhite(crate::memory::cstr_first_char(p))
        && crate::memory::cstr_first_byte(p) != b'|'
    {
        p = p.wrapping_add(1);
    }
    if p <= arg {
        return AUGROUP_ALL;
    }

    let len = crate::memory::ptr_offset_from(p, arg) as usize;
    let group_name = CString::new(crate::memory::cstr_slice(arg, len)).unwrap_or_default();
    let mut group = augroup_find(group_name.as_ptr());
    if group == AUGROUP_ERROR {
        group = AUGROUP_ALL;
    } else {
        unsafe {
            *argp = skipwhite(p);
        }
    }
    group
}

fn arg_autocmd_flag_get(
    flag: &mut bool,
    cmd_ptr: *mut *mut c_char,
    pattern: &std::ffi::CStr,
    len: i32,
) -> bool {
    unsafe {
        let cmd_bytes = crate::memory::cstr_as_bytes(*cmd_ptr);
        let pat_slice = crate::memory::cstr_slice(pattern.as_ptr(), len as usize);
        if cmd_bytes.starts_with(pat_slice)
            && ascii_iswhite(crate::memory::cstr_char_at(*cmd_ptr, len as usize))
        {
            if *flag {
                semsg!(tr(E_DUPARG2), pattern.as_ptr());
                return true;
            }
            *flag = true;
            *cmd_ptr = skipwhite((*cmd_ptr).wrapping_add(len as usize));
        }
        false
    }
}

pub fn vimresume_event() {
    apply_autocmds(
        EVENT_VIMRESUME,
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        ptr::null_mut(),
    );
    au_st().pending_vimresume = K_FALSE;
}

pub fn may_trigger_vim_suspend_resume(suspend: bool) {
    if suspend && au_st().pending_vimresume == K_FALSE {
        au_st().pending_vimresume = K_NONE;
        apply_autocmds(
            EVENT_VIMSUSPEND,
            ptr::null_mut(),
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        au_st().pending_vimresume = K_TRUE;
    } else if !suspend && au_st().pending_vimresume == K_TRUE {
        au_st().pending_vimresume = K_NONE;
        let events = loop_queues(&mut g_st().main_loop).events;
        multiqueue_put_event(unsafe { &mut *events }, Event::AutoCmdVimResume);
    }
}

pub fn do_autocmd_uienter(chanid: u64, attached: bool) {
    if starting_val() == NO_SCREEN || au_st().uienter_recursive {
        return;
    }
    au_st().uienter_recursive = true;

    let mut save_v_event: SaveVEvent = SaveVEvent::new();
    let dict = get_v_event(&mut save_v_event);
    assert!(chanid < i64::MAX as u64);
    tv_dict_add_nr(&mut *dict, c"chan", chanid as Varnumber);
    tv_dict_set_keys_readonly(&mut *dict);
    apply_autocmds(
        if attached {
            EVENT_UIENTER
        } else {
            EVENT_UILEAVE
        },
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        g_st().curbuf,
    );
    restore_v_event(&mut *dict, &mut save_v_event);

    au_st().uienter_recursive = false;
}

pub fn do_autocmd_focusgained(gained: bool) {
    if au_st().focusgained_recursive {
        return;
    }
    au_st().focusgained_recursive = true;
    apply_autocmds(
        if gained {
            EVENT_FOCUSGAINED
        } else {
            EVENT_FOCUSLOST
        },
        ptr::null_mut(),
        ptr::null_mut(),
        false,
        g_st().curbuf,
    );

    if gained && au_st().focusgained_last_time + 2000 < os_now() {
        check_timestamps(1);
        au_st().focusgained_last_time = os_now();
    }

    au_st().focusgained_recursive = false;
}

pub fn do_filetype_autocmd(buf: &mut FileBuf, force: bool) -> bool {
    if au_st().ft_recursive > 0 && !force {
        return false;
    }

    let secure_save = g_st().secure;
    g_st().secure = 0;

    au_st().ft_recursive += 1;
    buf.b_did_filetype = true;
    let ret = apply_autocmds(
        EVENT_FILETYPE,
        buf.b_p_ft,
        buf.b_fname,
        force || au_st().ft_recursive == 1,
        &raw mut *buf,
    );
    au_st().ft_recursive -= 1;

    g_st().secure = secure_save;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_name_lookup_and_span_pattern_work() {
        let mut end: *mut c_char = ptr::null_mut();
        let ev = event_name2nr(c"BufReadPost,BufEnter", &raw mut end);
        assert_eq!(ev, EVENT_BUFREADPOST);
        assert_eq!(crate::memory::cstr_as_bytes(end), b"BufEnter");
        assert_eq!(
            crate::memory::cstr_as_bytes(event_nr2name(EVENT_VIMENTER)),
            b"VimEnter"
        );

        let mut start: *const c_char = ptr::null();
        let len = aucmd_span_pattern(c",,*.{c,h},*.rs", &raw mut start);
        assert_eq!(len, 7);
        assert_eq!(&crate::memory::cstr_as_bytes(start)[..len], b"*.{c,h}");
    }
}
