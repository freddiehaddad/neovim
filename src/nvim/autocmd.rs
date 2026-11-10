#![deny(unsafe_code)]

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
use crate::charset::{ascii_iswhite, getdigits_int};
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
use crate::runtime::{ETYPE_AUCMD, SID_NONE, Sctx, estack_pop, estack_push};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::state::{MODE_INSERT, MODE_NORMAL_BUSY, SaveVEvent, get_mode, get_real_state};
use crate::strings::tr;
use crate::window::{
    WinInfoInfo, check_lnums, check_lnums_nested, reset_lnums, snapshot_windows_scroll_size,
    unuse_tabpage, use_tabpage,
};
use std::ffi::{CStr, CString};
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
    pub allow_dirs: u8,
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
    autocmd_winid: std::cell::Cell<ObjHandle>,
    next_augroup_id: std::cell::Cell<i32>,
    deleted_augroup: std::cell::Cell<Option<&'static CStr>>,
    current_augroup: std::cell::Cell<i32>,
    au_need_clean: std::cell::Cell<bool>,
    autocmd_blocked: std::cell::Cell<i32>,
    autocmd_nested: std::cell::Cell<bool>,
    autocmd_include_groups: std::cell::Cell<bool>,
    termresponse_changed: std::cell::Cell<bool>,
    termresponse_chan_id: std::cell::Cell<u64>,
    apply_autocmds_nesting: std::cell::Cell<i32>,
    filechangeshell_busy: std::cell::Cell<bool>,
    pending_vimresume: std::cell::Cell<TriState>,
    uienter_recursive: std::cell::Cell<bool>,
    focusgained_recursive: std::cell::Cell<bool>,
    focusgained_last_time: std::cell::Cell<Timestamp>,
    ft_recursive: std::cell::Cell<i32>,
}

impl AutocmdState {
    const fn new() -> Self {
        Self {
            autocmd_winid: std::cell::Cell::new(0),
            next_augroup_id: std::cell::Cell::new(1),
            deleted_augroup: std::cell::Cell::new(None),
            current_augroup: std::cell::Cell::new(AUGROUP_DEFAULT),
            au_need_clean: std::cell::Cell::new(false),
            autocmd_blocked: std::cell::Cell::new(0),
            autocmd_nested: std::cell::Cell::new(false),
            autocmd_include_groups: std::cell::Cell::new(false),
            termresponse_changed: std::cell::Cell::new(false),
            termresponse_chan_id: std::cell::Cell::new(0),
            apply_autocmds_nesting: std::cell::Cell::new(0),
            filechangeshell_busy: std::cell::Cell::new(false),
            pending_vimresume: std::cell::Cell::new(K_FALSE),
            uienter_recursive: std::cell::Cell::new(false),
            focusgained_recursive: std::cell::Cell::new(false),
            focusgained_last_time: std::cell::Cell::new(0),
            ft_recursive: std::cell::Cell::new(0),
        }
    }
}

struct AugroupMaps {
    name_to_id: Vec<(std::ffi::CString, i32)>,
    id_to_name: std::collections::BTreeMap<i32, std::ffi::CString>,
}

impl AugroupMaps {
    const fn new() -> Self {
        Self {
            name_to_id: Vec::new(),
            id_to_name: std::collections::BTreeMap::new(),
        }
    }
}

thread_local! {
    static AU_STATE: AutocmdState = const { AutocmdState::new() };
    static AUTOCMDS: [std::cell::RefCell<AutoCmdVec>; NUM_EVENTS] = const {
        [const { std::cell::RefCell::new(Vec::new()) }; NUM_EVENTS]
    };
    static ACTIVE_APC_LIST: std::cell::RefCell<Vec<std::rc::Rc<std::cell::Cell<i32>>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static AUGROUP_MAPS: std::cell::RefCell<AugroupMaps> = const {
        std::cell::RefCell::new(AugroupMaps::new())
    };
}

#[inline]
pub fn with_autocmds_for_event<R>(event: AuEvent, f: impl FnOnce(&mut AutoCmdVec) -> R) -> R {
    AUTOCMDS.with(|a| f(&mut a[event as usize].borrow_mut()))
}

fn map_put_name_to_id(name: &CStr, val: i32) {
    AUGROUP_MAPS.with(|cell| {
        let mut maps = cell.borrow_mut();
        for (existing_name, existing_val) in &mut maps.name_to_id {
            if existing_name.as_c_str() == name {
                *existing_val = val;
                return;
            }
        }
        maps.name_to_id.push((name.to_owned(), val));
    });
}

fn map_get_id_to_name(key: i32) -> Option<CString> {
    AUGROUP_MAPS.with(|cell| cell.borrow().id_to_name.get(&key).cloned())
}

fn map_put_id_to_name(key: i32, name: &CStr) {
    AUGROUP_MAPS.with(|cell| {
        cell.borrow_mut().id_to_name.insert(key, name.to_owned());
    });
}

fn event_name2nr_hash(s: &[u8]) -> isize {
    let len = s.len();
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
        if s.eq_ignore_ascii_case(EVENT_NAMES[ev].name.to_bytes()) {
            return i as isize;
        }
    }
    -1
}

pub fn autocmd_init() {
    let events = loop_queues(&mut g_st().main_loop).events.borrow_mut();
    g_st().deferred_events = crate::globals::GraphPtr::from(multiqueue_new_child(events));
}

pub fn autocmd_free_all_mem() {
    multiqueue_free(g_st().deferred_events.borrow_mut());
}

fn augroup_map_del(id: i32, name: Option<&CStr>) {
    AUGROUP_MAPS.with(|cell| {
        let mut maps = cell.borrow_mut();
        if let Some(name_cstr) = name
            && let Some(pos) = maps
                .name_to_id
                .iter()
                .position(|(existing_name, _)| existing_name.as_c_str() == name_cstr)
        {
            maps.name_to_id.swap_remove(pos);
        }
        if id > 0 {
            maps.id_to_name.remove(&id);
        }
    });
}

#[inline]
fn get_deleted_augroup() -> &'static CStr {
    if let Some(del) = AU_STATE.with(|s| s.deleted_augroup.get()) {
        del
    } else {
        let del = tr(c"--Deleted--");
        AU_STATE.with(|s| s.deleted_augroup.set(Some(del)));
        del
    }
}

fn au_show_for_all_events(group: i32, pat: &CStr) {
    for event in 0..NUM_EVENTS as AuEvent {
        au_show_for_event(group, event, pat);
    }
}

fn au_show_for_event(group: i32, event: AuEvent, pat_in: &CStr) {
    if !has_event(event) {
        return;
    }

    let mut rem_bytes = pat_in.to_bytes();
    let mut pat_opt: Option<&[u8]> = None;
    if !rem_bytes.is_empty() {
        let Some((span, rest)) = aucmd_span_pattern_slice(rem_bytes) else {
            return;
        };
        pat_opt = Some(span);
        rem_bytes = rest;
    }

    let mut buflocal_pat = [0u8; BUFLOCAL_PAT_LEN];
    let mut last_group = AUGROUP_ERROR;
    let mut last_group_name: Option<CString>;

    loop {
        let mut last_ap: Option<std::rc::Rc<AutoPat>> = None;

        if let Some(span) = pat_opt
            && aupat_is_buflocal(span)
        {
            let nr = aupat_get_buflocal_nr(span);
            aupat_normalize_buflocal_pat(&mut buflocal_pat, span, nr);
            let norm = CStr::from_bytes_until_nul(&buflocal_pat)
                .unwrap_or(c"")
                .to_bytes();
            pat_opt = Some(norm);
        }

        let len = AUTOCMDS.with(|a| a[event as usize].borrow().len());
        for idx in 0..len {
            let item = AUTOCMDS.with(|a| {
                let mut acs = a[event as usize].borrow_mut();
                let ac = acs.get_mut(idx)?;
                let ap = ac.pat.as_ref()?.clone();
                let pat_matches = if let Some(span) = pat_opt {
                    ap.patlen == span.len() as i32 && span == ap.pat.as_bytes()
                } else {
                    true
                };
                if (group != AUGROUP_ALL && ap.group != group) || !pat_matches {
                    return None;
                }
                let handler_str = aucmd_handler_to_string(ac);
                let is_cmd = ac.handler.is_cmd();
                let desc = ac.desc.clone();
                let script_ctx = ac.script_ctx;
                Some((ap, handler_str, is_cmd, desc, script_ctx))
            });
            let Some((ap, handler_str, is_cmd, desc, script_ctx)) = item else {
                continue;
            };

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
                    if let Some(ref gn) = last_group_name {
                        msg_puts_hl(gn, HLF_T, false);
                    } else {
                        msg_puts_hl(get_deleted_augroup(), HLF_E, false);
                    }
                    msg_puts(c"  ");
                }
                msg_puts_hl(event_nr2name(event), HLF_T, false);
            }

            if !last_ap
                .as_ref()
                .is_some_and(|l| std::rc::Rc::ptr_eq(l, &ap))
            {
                last_ap = Some(ap.clone());

                msg_putchar(b'\n' as i32);
                if g_st().got_int {
                    return;
                }

                msg_advance(4);
                msg_outtrans(&ap.pat, 0, false);
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

            if let Some(desc) = &desc {
                let msglen = 100usize;
                let mut mbuf = [0u8; 100];
                let desc_str = desc.to_string_lossy();
                let formatted = if is_cmd {
                    let h_str = handler_str.to_string_lossy();
                    format!("{h_str} [{desc_str}]")
                } else {
                    msg_puts_hl(&handler_str, HLF_8, false);
                    format!(" [{desc_str}]")
                };
                let copy_len = formatted.len().min(msglen - 1);
                mbuf[..copy_len].copy_from_slice(&formatted.as_bytes()[..copy_len]);
                mbuf[copy_len] = 0;
                msg_outtrans(crate::memory::bytes_as_cstr(&mbuf), 0, false);
            } else if is_cmd {
                msg_outtrans(&handler_str, 0, false);
            } else {
                msg_puts_hl(&handler_str, HLF_8, false);
            }
            if opt_vars().p_verbose > 0 {
                last_set_msg(script_ctx);
            }

            if g_st().got_int {
                return;
            }
        }

        let Some((span, rest)) = aucmd_span_pattern_slice(rem_bytes) else {
            break;
        };
        pat_opt = Some(span);
        rem_bytes = rest;
    }
}

fn aucmd_del(ac: &mut AutoCmd) {
    *ac = AutoCmd::new();
    AU_STATE.with(|s| s.au_need_clean.set(true));
}

pub fn aucmd_del_for_event_and_group(event: AuEvent, group: i32) {
    with_autocmds_for_event(event, |acs| {
        for ac in acs {
            if ac.pat.as_ref().is_some_and(|ap| ap.group == group) {
                aucmd_del(ac);
            }
        }
    });
    au_cleanup();
}

fn au_cleanup() {
    if g_st().autocmd_busy || !AU_STATE.with(|s| s.au_need_clean.get()) {
        return;
    }

    AUTOCMDS.with(|all| {
        for cell in all {
            let mut acs = cell.borrow_mut();
            acs.retain(|ac| ac.pat.is_some());
            if acs.is_empty() {
                *acs = Vec::new();
            }
        }
    });

    AU_STATE.with(|s| s.au_need_clean.set(false));
}

pub fn aubuflocal_remove(buf: &mut FileBuf) {
    ACTIVE_APC_LIST.with(|cell| {
        for arg_bufnr in cell.borrow().iter().rev() {
            if buf.b_fnum == arg_bufnr.get() {
                arg_bufnr.set(0);
            }
        }
    });

    for event in 0..NUM_EVENTS as AuEvent {
        let mut removed_count = 0usize;
        with_autocmds_for_event(event, |acs| {
            for ac in acs {
                if ac
                    .pat
                    .as_ref()
                    .is_some_and(|ap| ap.buflocal_nr == buf.b_fnum)
                {
                    aucmd_del(ac);
                    removed_count += 1;
                }
            }
        });
        if opt_vars().p_verbose >= 6 {
            for _ in 0..removed_count {
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

pub fn augroup_add(name: &CStr) -> i32 {
    assert!(!name.to_bytes().eq_ignore_ascii_case(b"end"));

    let existing_id = augroup_find(name);
    if existing_id > 0 {
        assert!(existing_id != AUGROUP_DELETED);
        return existing_id;
    }

    if existing_id == AUGROUP_DELETED {
        augroup_map_del(existing_id, Some(name));
    }

    let next_id = AU_STATE.with(|s| s.next_augroup_id.get());
    AU_STATE.with(|s| s.next_augroup_id.set(s.next_augroup_id.get() + (1)));
    map_put_name_to_id(name, next_id);
    map_put_id_to_name(next_id, name);

    next_id
}

pub fn augroup_del_by_id(id: i32) {
    let name = if id <= 0 { None } else { augroup_name(id) };
    let Some(name) = name else {
        semsg!(tr(c"E367: No such group id: %d"), id);
        return;
    };
    augroup_del(&name, id, false);
}

pub fn augroup_del(name: &CStr, id_for_error: i32, stupid_legacy_mode: bool) {
    let group = augroup_find(name);
    if group == AUGROUP_ERROR {
        if id_for_error > 0 {
            semsg!(tr(c"E367: No such group id: %d \"%s\""), id_for_error, name,);
        } else {
            semsg!(tr(c"E367: No such group: \"%s\""), name);
        }
        return;
    } else if group == AU_STATE.with(|s| s.current_augroup.get()) {
        emsg(tr(c"E936: Cannot delete the current group"));
        return;
    }

    if stupid_legacy_mode {
        let in_use = AUTOCMDS.with(|all| {
            all.iter().any(|cell| {
                cell.borrow()
                    .iter()
                    .any(|ac| ac.pat.as_ref().is_some_and(|ap| ap.group == group))
            })
        });
        if in_use {
            give_warning(
                tr(c"W19: Deleting augroup that is still in use"),
                true,
                true,
            );
            map_put_name_to_id(name, AUGROUP_DELETED);
            augroup_map_del(group, None);
            return;
        }
    } else {
        AUTOCMDS.with(|all| {
            for cell in all {
                for ac in cell.borrow_mut().iter_mut() {
                    if ac.pat.as_ref().is_some_and(|ap| ap.group == group) {
                        aucmd_del(ac);
                    }
                }
            }
        });
    }

    augroup_map_del(group, Some(name));
    au_cleanup();
}

pub fn augroup_find(name: &CStr) -> i32 {
    if name.to_bytes().is_empty() {
        return AUGROUP_ERROR;
    }
    let existing_id = AUGROUP_MAPS.with(|cell| {
        cell.borrow()
            .name_to_id
            .iter()
            .find_map(|(k, v)| (k.as_c_str() == name).then_some(*v))
            .unwrap_or(0)
    });
    if existing_id == AUGROUP_DELETED || existing_id > 0 {
        existing_id
    } else {
        AUGROUP_ERROR
    }
}

pub fn augroup_name(mut group: i32) -> Option<CString> {
    assert!(group != 0);

    if group == AUGROUP_DELETED {
        return Some(get_deleted_augroup().to_owned());
    }

    if group == AUGROUP_ALL {
        group = AU_STATE.with(|s| s.current_augroup.get());
    }

    if group == AU_STATE.with(|s| s.next_augroup_id.get()) {
        return Some(c"END".to_owned());
    }

    if group > AU_STATE.with(|s| s.next_augroup_id.get()) {
        return None;
    }

    if let Some(key) = map_get_id_to_name(group) {
        return Some(key);
    }

    Some(get_deleted_augroup().to_owned())
}

pub fn augroup_exists(name: Option<&CStr>) -> bool {
    name.is_some_and(|n| augroup_find(n) > 0)
}

pub fn do_augroup(arg: &CStr, del_group: bool) {
    let arg_bytes = arg.to_bytes();
    if del_group {
        if arg_bytes.is_empty() {
            emsg(tr(E_ARGREQ));
        } else {
            augroup_del(arg, -1, true);
        }
    } else if arg_bytes.eq_ignore_ascii_case(b"end") {
        AU_STATE.with(|s| s.current_augroup.set(AUGROUP_DEFAULT));
    } else if !arg_bytes.is_empty() {
        AU_STATE.with(|s| s.current_augroup.set(augroup_add(arg)));
    } else {
        msg_start();
        msg_ext_set_kind(c"list_cmd");

        let entries = AUGROUP_MAPS.with(|cell| cell.borrow().name_to_id.clone());
        for (name, value) in &entries {
            if *value > 0 {
                msg_puts(name);
            } else {
                let del_name = augroup_name(*value);
                msg_puts(del_name.as_deref().unwrap_or(c""));
            }
            msg_puts(c"  ");
        }

        msg_clr_eos();
        msg_end();
    }
}

pub fn free_all_autocmds() {
    AUTOCMDS.with(|all| {
        for cell in all {
            let mut acs = cell.borrow_mut();
            for ac in acs.iter_mut() {
                aucmd_del(ac);
            }
            *acs = Vec::new();
        }
    });
    AU_STATE.with(|s| s.au_need_clean.set(false));

    AUGROUP_MAPS.with(|cell| {
        let mut maps = cell.borrow_mut();
        maps.name_to_id.clear();
        maps.id_to_name.clear();
    });
}

pub fn event_name2nr(s: &mut &[u8]) -> AuEvent {
    let bytes = *s;
    let len = bytes
        .iter()
        .position(|&b| ascii_iswhite(i32::from(b)) || b == b',' || b == b'|')
        .unwrap_or(bytes.len());
    let hash_idx = event_name2nr_hash(&bytes[..len]);
    let next = if bytes.get(len) == Some(&b',') {
        len + 1
    } else {
        len
    };
    *s = &bytes[next..];
    if hash_idx < 0 {
        return NUM_EVENTS as AuEvent;
    }
    let ev = EVENT_HASH[hash_idx as usize] as usize;
    EVENT_NAMES[ev].event.abs() as AuEvent
}

pub fn event_name2nr_str(str_: NvimString) -> AuEvent {
    let hash_idx = event_name2nr_hash(str_.as_bytes());
    if hash_idx < 0 {
        return NUM_EVENTS as AuEvent;
    }
    let ev = EVENT_HASH[hash_idx as usize] as usize;
    EVENT_NAMES[ev].event.abs() as AuEvent
}

pub fn event_nr2name(event: AuEvent) -> &'static std::ffi::CStr {
    EVENT_NAMES
        .get(event as usize)
        .map_or(c"Unknown", |e| e.name)
}

pub fn event_ignored(event: AuEvent, ei: &CStr) -> bool {
    let is_p_ei = ptr::eq(ei.as_ptr(), opt_vars().p_ei.as_ptr());
    let mut rem = ei.to_bytes();
    let full_len = rem.len();
    let mut ignored = false;
    while !rem.is_empty() {
        let unignore = rem[0] == b'-';
        if unignore {
            rem = &rem[1..];
        }
        if rem.len() >= 3
            && rem[..3].eq_ignore_ascii_case(b"all")
            && (rem.len() == 3 || rem[3] == b',')
        {
            ignored = (is_p_ei && rem.len() == full_len) || EVENT_NAMES[event as usize].event <= 0;
            rem = &rem[3 + usize::from(rem.get(3) == Some(&b','))..];
        } else if event_name2nr(&mut rem) == event {
            if unignore {
                return false;
            }
            ignored = true;
        }
    }
    ignored
}

pub fn check_ei(ei: &CStr, win: bool) -> bool {
    let mut rem = ei.to_bytes();
    while !rem.is_empty() {
        if rem.len() >= 3
            && rem[..3].eq_ignore_ascii_case(b"all")
            && (rem.len() == 3 || rem[3] == b',')
        {
            rem = &rem[3 + usize::from(rem.get(3) == Some(&b','))..];
        } else {
            if rem[0] == b'-' {
                rem = &rem[1..];
            }
            let event = event_name2nr(&mut rem);
            if event == NUM_EVENTS as AuEvent || (win && EVENT_NAMES[event as usize].event > 0) {
                return false;
            }
        }
    }
    true
}

pub fn au_event_disable(what: &std::ffi::CStr) -> std::ffi::CString {
    let save_ei = opt_vars().p_ei.to_owned();
    let p_ei_bytes = save_ei.as_bytes();
    let what_bytes = what.to_bytes();
    let mut new_ei_vec: Vec<u8> = Vec::with_capacity(p_ei_bytes.len() + what_bytes.len());
    if what_bytes.first() == Some(&b',') && p_ei_bytes.is_empty() {
        new_ei_vec.extend_from_slice(&what_bytes[1..]);
    } else {
        new_ei_vec.extend_from_slice(p_ei_bytes);
        new_ei_vec.extend_from_slice(what_bytes);
    }
    let new_ei = CString::new(new_ei_vec).unwrap_or_default();
    set_option_direct(
        K_OPT_EVENTIGNORE,
        cstr_as_obj(Some(new_ei.as_c_str())),
        0,
        SID_NONE,
    );
    save_ei
}

pub fn au_event_restore(old_ei: Option<std::ffi::CString>) {
    if let Some(old_ei) = old_ei {
        set_option_direct(
            K_OPT_EVENTIGNORE,
            cstr_as_obj(Some(old_ei.as_c_str())),
            0,
            SID_NONE,
        );
    }
}

pub fn do_autocmd(eap: &mut Exarg, arg_in: &CStr, forceit: bool) {
    let mut arg = arg_in;
    let mut envpat: Option<CString> = None;
    let mut pat_owned: Option<CString> = None;
    let mut cmd: &CStr;
    let mut cmd_owned: Option<CString> = None;
    let mut nested = false;
    let mut once = false;
    let group = if arg.to_bytes().first() == Some(&b'|') {
        eap.nextcmd = crate::strings::OwnedSlice::from_cstr(arg).wrapping_add(1);
        arg = c"";
        AUGROUP_ALL
    } else {
        arg_augroup_get(&mut arg)
    };

    let Some(pat_after_ev) = arg_event_skip(arg, group != AUGROUP_ALL) else {
        return;
    };

    let mut pat = crate::charset::skipwhite(pat_after_ev);
    'run: {
        if pat.to_bytes().first() == Some(&b'|') {
            eap.nextcmd = crate::strings::OwnedSlice::from_cstr(pat).wrapping_add(1);
            pat = c"";
            cmd = c"";
        } else {
            let pat_bytes = pat.to_bytes();
            let mut pat_len = 0usize;
            while pat_len < pat_bytes.len()
                && (!ascii_iswhite(i32::from(pat_bytes[pat_len]))
                    || (pat_len > 0 && pat_bytes[pat_len - 1] == b'\\'))
            {
                pat_len += 1;
            }
            if pat_len < pat_bytes.len() {
                cmd = crate::memory::CStrSubslice::advance(pat, pat_len + 1);
                pat_owned = Some(CString::new(&pat_bytes[..pat_len]).unwrap_or_default());
                pat = pat_owned.as_deref().unwrap();
            } else {
                cmd = c"";
            }

            if crate::strings::vim_haschr(pat, b'$' as i32)
                || crate::strings::vim_haschr(pat, b'~' as i32)
            {
                envpat = Some(expand_env_save(pat));
                pat = envpat.as_deref().unwrap();
            }

            cmd = crate::charset::skipwhite(cmd);

            let mut invalid_flags = false;
            for _ in 0..2 {
                if cmd.to_bytes().is_empty() {
                    continue;
                }
                invalid_flags |= arg_autocmd_flag_get(&mut once, &mut cmd, b"++once");
                invalid_flags |= arg_autocmd_flag_get(&mut nested, &mut cmd, b"++nested");
                invalid_flags |= arg_autocmd_flag_get(&mut nested, &mut cmd, b"nested");
            }

            if invalid_flags {
                break 'run;
            }

            if !cmd.to_bytes().is_empty() {
                cmd_owned = expand_sfile(cmd);
                match cmd_owned {
                    Some(ref co) => cmd = co.as_c_str(),
                    None => break 'run,
                }
            }
        }

        let is_showing = !forceit && cmd.to_bytes().is_empty();

        let mut arg_rem = arg.to_bytes();
        if is_showing {
            msg_ext_set_kind(c"list_cmd");
            msg_puts_title(tr(c"\n--- Autocommands ---"));

            if matches!(arg_rem.first(), None | Some(&b'*') | Some(&b'|')) {
                au_show_for_all_events(group, pat);
            } else {
                let event = event_name2nr(&mut arg_rem);
                assert!((event as usize) < NUM_EVENTS);
                au_show_for_event(group, event, pat);
            }
        } else if matches!(arg_rem.first(), None | Some(&b'*') | Some(&b'|')) {
            if !cmd.to_bytes().is_empty() {
                emsg(tr(E_CANNOT_DEFINE_AUTOCOMMANDS_FOR_ALL_EVENTS));
            } else {
                do_all_autocmd_events(pat, once, i32::from(nested), cmd, forceit, group);
            }
        } else {
            while !arg_rem.is_empty() && arg_rem[0] != b'|' && !ascii_iswhite(i32::from(arg_rem[0]))
            {
                let event = event_name2nr(&mut arg_rem);
                assert!((event as usize) < NUM_EVENTS);
                if !do_autocmd_event(event, pat, once, i32::from(nested), cmd, forceit, group) {
                    break;
                }
            }
        }
    }
    drop(cmd_owned);
    drop(envpat);
    drop(pat_owned);
}

pub fn do_all_autocmd_events(
    pat: &CStr,
    once: bool,
    nested: i32,
    cmd: &CStr,
    del: bool,
    group: i32,
) {
    for event in 0..NUM_EVENTS as AuEvent {
        if !do_autocmd_event(event, pat, once, nested, cmd, del, group) {
            return;
        }
    }
}

pub fn do_autocmd_event(
    event: AuEvent,
    pat_in: &CStr,
    once: bool,
    nested: i32,
    cmd: &CStr,
    del: bool,
    group: i32,
) -> bool {
    assert!(!pat_in.to_bytes().is_empty() || del);

    let mut buflocal_pat = [0u8; BUFLOCAL_PAT_LEN];
    let is_adding_cmd = !cmd.to_bytes().is_empty();
    let findgroup = if group == AUGROUP_ALL {
        AU_STATE.with(|s| s.current_augroup.get())
    } else {
        group
    };

    if pat_in.to_bytes().is_empty() && del {
        aucmd_del_for_event_and_group(event, findgroup);
        return true;
    }

    let mut rem_bytes = pat_in.to_bytes();
    while let Some((span, rest)) = aucmd_span_pattern_slice(rem_bytes) {
        rem_bytes = rest;
        let mut pat_slice = span;

        let is_buflocal = aupat_is_buflocal(pat_slice);
        if is_buflocal {
            let buflocal_nr = aupat_get_buflocal_nr(pat_slice);
            if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_none() {
                semsg!(tr(E_BUFFER_NR_INVALID_BUFFER_NUMBER), buflocal_nr,);
                return false;
            }
            aupat_normalize_buflocal_pat(&mut buflocal_pat, pat_slice, buflocal_nr);
            pat_slice = CStr::from_bytes_until_nul(&buflocal_pat)
                .unwrap_or(c"")
                .to_bytes();
        }

        if del {
            assert!(!pat_slice.is_empty());

            with_autocmds_for_event(event, |acs| {
                for ac in acs {
                    if let Some(ref ap) = ac.pat
                        && ap.group == findgroup
                        && ap.patlen == pat_slice.len() as i32
                        && pat_slice == ap.pat.as_bytes()
                    {
                        aucmd_del(ac);
                    }
                }
            });
        }

        if is_adding_cmd {
            let mut handler_fn = Callback::NONE;
            autocmd_register(
                0,
                event,
                pat_slice,
                group,
                once,
                nested != 0,
                None,
                Some(cmd),
                &mut handler_fn,
            );
        }
    }

    au_cleanup();
    true
}

pub fn autocmd_register(
    id: i64,
    event: AuEvent,
    pat: &[u8],
    group: i32,
    once: bool,
    nested: bool,
    desc: Option<&CStr>,
    handler_cmd: Option<&CStr>,
    handler_fn: &mut Callback,
) -> bool {
    assert!(group != 0);

    if pat.contains(&0) {
        return false;
    }

    let findgroup = if group == AUGROUP_ALL {
        AU_STATE.with(|s| s.current_augroup.get())
    } else {
        group
    };

    let mut pat_slice = pat;
    let is_buflocal = aupat_is_buflocal(pat_slice);
    let mut buflocal_nr: i32 = 0;
    let mut buflocal_pat = [0u8; BUFLOCAL_PAT_LEN];

    if is_buflocal {
        buflocal_nr = aupat_get_buflocal_nr(pat_slice);
        if buflocal_nr == 0 || buflist_findnr(buflocal_nr).is_none() {
            semsg!(tr(E_BUFFER_NR_INVALID_BUFFER_NUMBER), buflocal_nr,);
            return false;
        }
        aupat_normalize_buflocal_pat(&mut buflocal_pat, pat_slice, buflocal_nr);
        pat_slice = CStr::from_bytes_until_nul(&buflocal_pat)
            .unwrap_or(c"")
            .to_bytes();
    }
    let patlen = pat_slice.len() as i32;

    let mut ap: Option<std::rc::Rc<AutoPat>> = None;
    AUTOCMDS.with(|a| {
        let acs = a[event as usize].borrow();
        for ac in acs.iter().rev() {
            let Some(ref ac_ap) = ac.pat else {
                continue;
            };
            if ac_ap.group == findgroup
                && ac_ap.patlen == patlen
                && pat_slice == ac_ap.pat.as_bytes()
            {
                ap = Some(ac_ap.clone());
            }
            break;
        }
    });

    let ap = if let Some(ap) = ap {
        ap
    } else {
        let mut allow_dirs: u8 = 0;
        let reg_prog = if is_buflocal {
            None
        } else {
            let mut allow_dirs_b = false;
            let reg_pat = file_pat_to_reg_pat(pat_slice, Some(&mut allow_dirs_b), 1);
            allow_dirs = u8::from(allow_dirs_b);
            let prog = match reg_pat {
                Some(ref rp) => crate::regexp::vim_regcomp_box(rp, RE_MAGIC | RE_NOBREAK),
                None => None,
            };
            if reg_pat.is_none() || prog.is_none() {
                return false;
            }
            prog
        };

        if event == EVENT_MODECHANGED && !has_event(EVENT_MODECHANGED) {
            get_mode(&mut g_st().last_mode);
        }

        if (event == EVENT_CURSORMOVED && !has_event(EVENT_CURSORMOVED))
            || (event == EVENT_CURSORMOVEDI && !has_event(EVENT_CURSORMOVEDI))
        {
            g_st().last_cursormoved_win = crate::globals::curwin_ptr();
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
            use_tabpage(save_curtab.borrow_mut());
        }

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
    if let Some(cmd_cstr) = handler_cmd {
        ac.handler = AutoCmdHandler::Cmd(cmd_cstr.to_owned());
    } else {
        let mut cb = Callback::new();
        callback_copy(&mut cb, handler_fn);
        ac.handler = AutoCmdHandler::Fn(cb);
    }
    ac.script_ctx = g_st().current_sctx;
    let sourcing_lnum = if crate::runtime::estack_is_empty() {
        0
    } else {
        crate::runtime::sourcing_lnum()
    };
    ac.script_ctx.sc_lnum += sourcing_lnum;
    nlua_set_sctx(&mut ac.script_ctx);
    ac.once = once;
    ac.nested = nested;
    ac.desc = desc.map(CStr::to_owned);
    with_autocmds_for_event(event, |acs| acs.push(ac));

    true
}

pub fn aucmd_span_pattern_slice(mut pat: &[u8]) -> Option<(&[u8], &[u8])> {
    while pat.first() == Some(&b',') {
        pat = &pat[1..];
    }
    if pat.is_empty() || pat[0] == 0 {
        return None;
    }
    let mut i = 0usize;
    let mut brace_level: i32 = 0;
    while i < pat.len()
        && pat[i] != 0
        && (pat[i] != b',' || brace_level != 0 || (i > 0 && pat[i - 1] == b'\\'))
    {
        if pat[i] == b'{' {
            brace_level += 1;
        } else if pat[i] == b'}' {
            brace_level -= 1;
        }
        i += 1;
    }
    Some((&pat[..i], &pat[i..]))
}

pub fn do_doautocmd(arg_start: &CStr, do_msg: bool, mut did_something: Option<&mut bool>) -> bool {
    let mut arg = arg_start;
    let mut nothing_done = true;

    if let Some(ref mut p) = did_something {
        **p = false;
    }

    let group = arg_augroup_get(&mut arg);

    if arg.to_bytes().first() == Some(&b'*') {
        emsg(tr(c"E217: Can't execute autocommands for ALL events"));
        return false;
    }

    let Some(fname_after_ev) = arg_event_skip(arg, group != AUGROUP_ALL) else {
        return false;
    };

    let fname = crate::charset::skipwhite(fname_after_ev);

    while let Some(&b0) = arg.to_bytes().first()
        && ends_excmd(b0) == 0
        && !ascii_iswhite(i32::from(b0))
    {
        let arg_bytes = arg.to_bytes();
        let mut rem = arg_bytes;
        let ev = event_name2nr(&mut rem);
        arg = crate::memory::CStrSubslice::advance(arg, arg_bytes.len() - rem.len());
        if apply_autocmds_group(
            ev,
            Some(fname),
            None,
            true,
            group,
            curbuf_opt_mut(),
            Some(crate::globals::curwin_mut()),
            None,
            None,
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

pub fn ex_doautoall(eap: &mut Exarg) {
    let mut retval = true;
    let mut aco: CtxSwitch = CtxSwitch::new();
    let mut arg = eap.arg.as_cstr();
    let call_do_modelines = check_nomodeline(&mut arg);
    let mut bufref: Bufref = Bufref::new();
    let mut did_aucmd = false;

    for buf in for_all_buffers_ptr() {
        if buf.b_ml.ml_mfp.is_none() || g_st().curbuf == buf {
            continue;
        }

        ctx_switch(&mut aco, None, None, Some(&mut *buf), 0);
        set_bufref(&mut bufref, Some(&mut *buf));

        let ok = do_doautocmd(arg, false, Some(&mut did_aucmd));
        retval = ok;

        if call_do_modelines && did_aucmd {
            do_modelines(if is_ctx_win(curwin_mut()) {
                OPT_NOWIN
            } else {
                0
            });
        }

        ctx_restore(&mut aco);

        if !ok || !bufref_valid(Some(&bufref)) {
            retval = false;
            break;
        }
    }

    if retval {
        do_doautocmd(arg, false, Some(&mut did_aucmd));
        if call_do_modelines && did_aucmd {
            do_modelines(0);
        }
    }
}

pub fn check_nomodeline(argp: &mut &CStr) -> bool {
    let bytes = argp.to_bytes();
    if let Some(rest) = bytes.strip_prefix(b"<nomodeline>") {
        let skipped = crate::charset::skipwhite_slice(rest);
        *argp = crate::memory::CStrSubslice::advance(*argp, bytes.len() - skipped.len());
        return false;
    }
    true
}

pub fn aucmd_defer(
    event: AuEvent,
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    group: i32,
    buf: &mut FileBuf,
    data: Option<&mut Object>,
) {
    let evdata = Box::new(AutoCmdEvent {
        event,
        fname: fname.map(CStr::to_owned),
        fname_io: fname_io.map(CStr::to_owned),
        group,
        buf: buf.b_fnum,
        data: data.map(|d| copy_object(*d, None)),
    });

    multiqueue_put_event(
        g_st().deferred_events.borrow_mut(),
        Event::AutoCmdDeferred(evdata),
    );
}

pub fn deferred_event(mut e: AutoCmdEvent) {
    let event = e.event;
    let group = e.group;
    let data = e.data.take();

    let mut err = Error::new();
    if let Some(buf) = find_buffer_by_handle(e.buf, &mut err) {
        let buf_ptr = crate::globals::GraphPtr::<FileBuf>::from_ref(buf);
        let mut save_v_event: SaveVEvent = SaveVEvent::new();
        let v_event = get_v_event(&mut save_v_event);
        if let Some(ref data_obj) = data
            && data_obj.obj_type == ObjectType::ObjectTypeDict
        {
            for i in 0..data_obj.data.dict().size {
                let item = data_obj.data.dict()[i];
                let mut tv: TypVal = TypVal::new();
                object_to_vim(item.value, &mut tv, &mut err);
                if err.err_type != ErrorType::ErrorTypeNone {
                    api_clear_error(&mut err);
                    continue;
                }
                tv_dict_add_tv(&mut *v_event, item.key.as_bytes(), &mut tv);
                tv_clear(&mut tv);
            }
        }
        tv_dict_set_keys_readonly(&mut *v_event);

        let win = curwin_ptr();
        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, None, None, Some(buf), 0);

        apply_autocmds_group(
            event,
            e.fname.as_deref(),
            e.fname_io.as_deref(),
            false,
            group,
            buf_ptr.opt_mut(),
            win.opt(),
            None,
            data.as_ref(),
            false,
        );

        ctx_restore(&mut aco);

        restore_v_event(&mut *v_event, &mut save_v_event);
    }
    if let Some(data_obj) = data {
        api_free_object(data_obj);
    }
}

pub fn deferred_optionset_modified(bufnr: i32, new_val: bool) {
    let mut err = Error::new();
    let buf = find_buffer_by_handle(bufnr, &mut err);
    api_clear_error(&mut err);
    if let Some(buf) = buf {
        let old = boolean_obj(!new_val);
        let new = boolean_obj(new_val);
        let mut aco: CtxSwitch = CtxSwitch::new();
        ctx_switch(&mut aco, None, None, Some(buf), 0);
        apply_optionset_autocmd_now(K_OPT_MODIFIED, OPT_LOCAL, old, old, old, new);
        ctx_restore(&mut aco);
    }
}

pub fn aucmd_defer_modified(buf: &mut FileBuf, new_val: bool) {
    if !has_event(EVENT_OPTIONSET)
        || (bt_nofile(Some(buf)) && !buf.b_p_bl)
        || !get_vim_var_str(VV_OPTION_TYPE).is_empty()
    {
        return;
    }
    multiqueue_put_event(
        g_st().deferred_events.borrow_mut(),
        Event::AutoCmdOptionSetModified {
            bufnr: buf.b_fnum,
            new_val,
        },
    );
}

pub fn apply_autocmds(
    event: AuEvent,
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    force: bool,
    buf: Option<&mut FileBuf>,
) -> bool {
    apply_autocmds_group(
        event,
        fname,
        fname_io,
        force,
        AUGROUP_ALL,
        buf,
        Some(crate::globals::curwin_mut()),
        None,
        None,
        false,
    )
}
pub fn apply_autocmds_win(
    event: AuEvent,
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    force: bool,
    buf: Option<&mut FileBuf>,
    win: Option<&WinInfoInfo>,
) -> bool {
    apply_autocmds_group(
        event,
        fname,
        fname_io,
        force,
        AUGROUP_ALL,
        buf,
        win,
        None,
        None,
        false,
    )
}

pub fn apply_autocmds_exarg(
    event: AuEvent,
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    force: bool,
    buf: Option<&mut FileBuf>,
    eap: Option<&Exarg>,
) -> bool {
    apply_autocmds_group(
        event,
        fname,
        fname_io,
        force,
        AUGROUP_ALL,
        buf,
        Some(crate::globals::curwin_mut()),
        eap,
        None,
        false,
    )
}

pub fn apply_autocmds_retval(
    event: AuEvent,
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    force: bool,
    buf: Option<&mut FileBuf>,
    retval: &mut bool,
) -> bool {
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
        None,
        false,
    );
    if did_cmd && aborting() {
        *retval = false;
    }
    did_cmd
}

pub fn has_event(event: AuEvent) -> bool {
    AUTOCMDS.with(|a| !a[event as usize].borrow().is_empty())
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
    fname: Option<&CStr>,
    fname_io: Option<&CStr>,
    force: bool,
    group: i32,
    buf: Option<&mut FileBuf>,
    win: Option<&WinInfoInfo>,
    eap: Option<&Exarg>,
    data: Option<&Object>,
    with_buf: bool,
) -> bool {
    let buf = crate::globals::GraphPtr::from(buf);
    let mut fname = fname;
    {
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

            if g_st().autocmd_busy && !(force || AU_STATE.with(|s| s.autocmd_nested.get())) {
                break 'bypass_au;
            }

            if aborting() {
                break 'bypass_au;
            }

            if AU_STATE.with(|s| s.filechangeshell_busy.get())
                && (event == EVENT_FILECHANGEDSHELL || event == EVENT_FILECHANGEDSHELLPOST)
            {
                break 'bypass_au;
            }

            if event_ignored(event, opt_vars().p_ei.as_cstr()) {
                break 'bypass_au;
            }

            let mut win_ignore = false;
            if buf == g_st().curbuf.as_ptr() && EVENT_NAMES[event as usize].event <= 0 {
                win_ignore = event_ignored(event, curwin_mut().w_onebuf_opt.wo_eiw.as_cstr());
            } else if !buf.is_null() && EVENT_NAMES[event as usize].event <= 0 && buf.b_nwindows > 0
            {
                win_ignore = true;
                for tp in for_all_tabpages_ptr() {
                    for wp in for_all_windows_in_tab_mut(&*tp) {
                        if wp.w_buffer == buf
                            && !event_ignored(event, wp.w_onebuf_opt.wo_eiw.as_cstr())
                        {
                            win_ignore = false;
                            break;
                        }
                    }
                }
            }
            if win_ignore {
                break 'bypass_au;
            }

            if AU_STATE.with(|s| s.apply_autocmds_nesting.get()) == 10 {
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
            let save_autocmd_winid = AU_STATE.with(|s| s.autocmd_winid.get());
            let save_autocmd_match = g_st().autocmd_match.take();
            let save_autocmd_busy = g_st().autocmd_busy;
            let save_autocmd_nested = AU_STATE.with(|s| s.autocmd_nested.get());

            let raw_autocmd_fname = if fname_io.is_none() {
                if event == EVENT_COLORSCHEME
                    || event == EVENT_COLORSCHEMEPRE
                    || event == EVENT_OPTIONSET
                    || event == EVENT_MODECHANGED
                    || event == EVENT_MARKSET
                {
                    None
                } else if let Some(f) = fname
                    && ends_excmd(i32::from(f.to_bytes().first().copied().unwrap_or(0))) == 0
                {
                    fname
                } else if !buf.is_null() {
                    buf.ffname_opt()
                } else {
                    None
                }
            } else {
                fname_io
            };
            let afile_orig_owned = if let Some(orig_cstr) = raw_autocmd_fname {
                let orig = orig_cstr.to_owned();
                g_st().autocmd_fname = Some(orig.clone());
                Some(orig)
            } else {
                g_st().autocmd_fname = None;
                None
            };
            g_st().autocmd_fname_full = false;

            g_st().autocmd_bufnr = if buf.is_null() { 0 } else { buf.b_fnum };

            let mut sfname_owned: Option<CString> = None;
            let fname_owned: Option<CString> = if fname.is_none_or(|s| s.to_bytes().is_empty()) {
                if buf.is_null() {
                    fname = None;
                } else if event == EVENT_SYNTAX {
                    fname = Some(buf.b_p_syn.as_cstr());
                } else if event == EVENT_FILETYPE {
                    fname = Some(buf.b_p_ft.as_cstr());
                } else {
                    sfname_owned = buf.b_sfname.clone();
                    fname = buf.ffname_opt();
                }
                Some(fname.unwrap_or(c"").to_owned())
            } else {
                let fname_opt = fname;
                sfname_owned = to_slash_save(fname_opt);
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
                    to_slash_save(fname_opt)
                } else {
                    FullName_save(fname_opt, false)
                }
            };
            fname = fname_owned.as_deref();
            if fname.is_none() {
                retval = false;
                break 'bypass_au;
            }

            AU_STATE.with(|s| {
                s.autocmd_winid.set(if let Some(win) = win {
                    win.handle
                } else if !(curwin_ptr()).is_null() {
                    curwin_mut().handle
                } else {
                    0
                })
            });

            if with_buf && !buf.is_null() && buf != g_st().curbuf.as_ptr() {
                ctx_switch(&mut aco, None, None, buf.opt_mut(), 0);
            }

            let save_changed = curbuf_mut().b_changed;
            let old_curbuf = g_st().curbuf;

            g_st().autocmd_match = fname.map(CStr::to_owned);

            g_st().redrawing_disabled += 1;

            estack_push(ETYPE_AUCMD, None, 0);

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
            AU_STATE.with(|s| s.filechangeshell_busy.set(event == EVENT_FILECHANGEDSHELL));
            AU_STATE.with(|s| {
                s.apply_autocmds_nesting
                    .set(s.apply_autocmds_nesting.get() + (1))
            });

            if event == EVENT_FILETYPE && !curbuf_mut().b_p_ft.is_empty() {
                curbuf_mut().b_did_filetype = true;
            }

            let ausize = AUTOCMDS.with(|a| a[event as usize].borrow().len());
            let mut patcmd = AutoPatCmd {
                lastpat: None,
                auidx: 0,
                ausize,
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
                ACTIVE_APC_LIST.with(|cell| cell.borrow_mut().push(patcmd.arg_bufnr.clone()));
                patcmd.data = data.copied();

                let save_cmdbang = get_vim_var_nr(VV_CMDBANG);
                let save_cmdarg = if let Some(eap) = eap {
                    let s = set_cmdarg(Some(eap), None);
                    set_vim_var_nr(VV_CMDBANG, Varnumber::from(eap.forceit));
                    s
                } else {
                    None
                };
                retval = true;

                if AU_STATE.with(|s| s.apply_autocmds_nesting.get()) == 1 {
                    check_lnums(true);
                } else {
                    check_lnums_nested(true);
                }

                let save_did_emsg = g_st().did_emsg;

                do_cmdline(
                    None,
                    crate::cmdexpand::LineGetter::AutoCmd((&mut patcmd).into()),
                    DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT,
                );

                g_st().did_emsg += save_did_emsg;

                if AU_STATE.with(|s| s.apply_autocmds_nesting.get()) == 1 {
                    reset_lnums();
                }

                if eap.is_some() {
                    set_cmdarg(None, save_cmdarg);
                    set_vim_var_nr(VV_CMDBANG, save_cmdbang);
                }
                ACTIVE_APC_LIST.with(|cell| {
                    let mut list = cell.borrow_mut();
                    if list
                        .last()
                        .is_some_and(|last| std::rc::Rc::ptr_eq(last, &patcmd.arg_bufnr))
                    {
                        list.pop();
                    }
                });
            }

            g_st().redrawing_disabled -= 1;
            g_st().autocmd_busy = save_autocmd_busy;
            AU_STATE.with(|s| s.filechangeshell_busy.set(false));
            AU_STATE.with(|s| s.autocmd_nested.set(save_autocmd_nested));
            estack_pop();
            g_st().autocmd_fname = save_autocmd_fname;
            g_st().autocmd_fname_full = save_autocmd_fname_full;
            g_st().autocmd_bufnr = save_autocmd_bufnr;
            AU_STATE.with(|s| s.autocmd_winid.set(save_autocmd_winid));
            g_st().autocmd_match = save_autocmd_match;
            g_st().current_sctx = save_current_sctx;
            restore_funccal();
            if g_st().do_profiling == PROF_YES {
                prof_child_exit(&mut wait_time);
            }
            g_st().key_typed = save_key_typed;
            drop(patcmd);
            AU_STATE.with(|s| {
                s.apply_autocmds_nesting
                    .set(s.apply_autocmds_nesting.get() - (1))
            });

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
            aubuflocal_remove(buf.borrow_mut());
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
        items: crate::globals::GraphPtr::new(data_items.as_mut_ptr()),
        size: 2,
        capacity: 2,
    };
    let data_obj = dict_obj(data);
    apply_autocmds_group(
        EVENT_TERMRESPONSE,
        None,
        None,
        true,
        AUGROUP_ALL,
        None,
        Some(crate::globals::curwin_mut()),
        None,
        Some(&data_obj),
        false,
    );
    AU_STATE.with(|s| s.termresponse_changed.set(true));
    AU_STATE.with(|s| s.termresponse_chan_id.set(channel_id));
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn block_autocmds() {
    if !is_autocmd_blocked() {
        AU_STATE.with(|s| s.termresponse_changed.set(false));
        AU_STATE.with(|s| s.termresponse_chan_id.set(0));
    }
    AU_STATE.with(|s| s.autocmd_blocked.set(s.autocmd_blocked.get() + (1)));
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn unblock_autocmds() {
    AU_STATE.with(|s| s.autocmd_blocked.set(s.autocmd_blocked.get() - (1)));

    if !is_autocmd_blocked()
        && AU_STATE.with(|s| s.termresponse_changed.get())
        && has_event(EVENT_TERMRESPONSE)
    {
        let sequence = cstr_to_string(Some(get_vim_var_str(VV_TERMRESPONSE)));
        do_termresponse_autocmd(sequence, AU_STATE.with(|s| s.termresponse_chan_id.get()));
        api_free_string(sequence);
    }
}

pub fn is_autocmd_blocked() -> bool {
    AU_STATE.with(|s| s.autocmd_blocked.get()) != 0
}

fn aucmd_next(apc: &mut AutoPatCmd) {
    let acs_len = AUTOCMDS.with(|a| a[apc.event as usize].borrow().len());
    assert!(apc.ausize <= acs_len);
    let mut i = apc.auidx;
    while i < apc.ausize && !g_st().got_int {
        let ap_opt = AUTOCMDS.with(|a| a[apc.event as usize].borrow()[i].pat.clone());
        let Some(ap) = ap_opt else {
            i += 1;
            continue;
        };
        if !apc
            .lastpat
            .as_ref()
            .is_some_and(|l| std::rc::Rc::ptr_eq(l, &ap))
        {
            if apc.group != AUGROUP_ALL && apc.group != ap.group {
                i += 1;
                continue;
            }
            let matches = if ap.buflocal_nr == 0 {
                let tail_cstr = apc.fname.as_deref().map_or(c"", |f| path_tail(f));
                match_file_pat(
                    None,
                    ap.reg_prog.as_deref(),
                    apc.fname.as_deref(),
                    apc.sfname.as_deref(),
                    tail_cstr,
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
            let name_str = name.to_string_lossy();
            let pat_str = ap.pat.to_string_lossy();
            let formatted = format!("{name_str} Autocommands for \"{pat_str}\"");
            let name_cstr = CString::new(formatted).unwrap_or_default();
            if opt_vars().p_verbose >= 8 {
                verbose_enter();
                smsg!(0, tr(c"Executing %s"), &name_cstr);
                verbose_leave();
            }

            crate::runtime::estack_set_top_info(
                Some(name_cstr),
                crate::profile::EStackInfo::AuCmd(apc.script_ctx.clone()),
            );
        }

        apc.lastpat = Some(ap);
        apc.auidx = i;

        line_breakcheck();
        return;
    }

    crate::runtime::estack_set_top_info(None, crate::profile::EStackInfo::None);

    apc.lastpat = None;
    apc.auidx = usize::MAX;
}

fn au_callback(mut callback: Callback, id: i64, group: i32, apc: &AutoPatCmd) -> bool {
    if callback.cb_type() == K_CALLBACK_LUA {
        let mut data_items = [KeyValuePair {
            key: NvimString::new(),
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
            value: cstr_as_obj(Some(event_nr2name(apc.event))),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"file"),
            value: cstr_as_obj(apc.afile_orig.as_deref()),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"match"),
            value: cstr_as_obj(g_st().autocmd_match.as_deref()),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"buf"),
            value: integer_obj(Integer::from(g_st().autocmd_bufnr)),
        };
        size += 1;
        data_items[size] = KeyValuePair {
            key: static_cstr_as_string(c"win"),
            value: integer_obj(Integer::from(AU_STATE.with(|s| s.autocmd_winid.get()))),
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
            items: crate::globals::GraphPtr::new(data_items.as_mut_ptr()),
            size,
            capacity: 8,
        };
        let mut args_items = [dict_obj(data)];
        let args = Array {
            items: crate::globals::GraphPtr::new(args_items.as_mut_ptr()),
            size: 1,
            capacity: 1,
        };

        let luaref = callback.data.as_luaref();
        let result = nlua_call_ref(luaref, None, args, K_RET_NIL_BOOL, None, None);
        result.obj_type == ObjectType::ObjectTypeBoolean && result.data.boolean()
    } else {
        let mut argsin: [TypVal; 1] = [TypVal::new()];
        let mut rettv: TypVal = TypVal::new();
        callback_call(&mut callback, 0, &mut argsin, &mut rettv);
        false
    }
}

pub fn getnextac(_c: i32, apc: &mut AutoPatCmd, _indent: i32, _do_concat: bool) -> Option<CString> {
    aucmd_next(apc);
    apc.lastpat.as_ref()?;

    let auidx = apc.auidx;
    let (mut oneshot, handler_str_opt, nested, script_ctx, cmd_or_cb) =
        with_autocmds_for_event(apc.event, |acs| {
            assert!(auidx < acs.len());
            let ac = &mut acs[auidx];
            assert!(ac.pat.is_some());
            let oneshot = ac.once;
            let handler_str = (opt_vars().p_verbose >= 9).then(|| aucmd_handler_to_string(ac));
            let nested = ac.nested;
            let script_ctx = ac.script_ctx;
            let group = ac.pat.as_ref().unwrap().group;
            let id = ac.id;
            let cmd_or_cb = match &ac.handler {
                AutoCmdHandler::Cmd(cmd) => Ok(cmd.clone()),
                AutoCmdHandler::Fn(cb) => {
                    let cb = cb.clone();
                    let saved_pat = if oneshot { ac.pat.take() } else { None };
                    Err((cb, id, group, saved_pat))
                }
                AutoCmdHandler::None => {
                    let saved_pat = if oneshot { ac.pat.take() } else { None };
                    Err((Callback::NONE, id, group, saved_pat))
                }
            };
            (oneshot, handler_str, nested, script_ctx, cmd_or_cb)
        });

    if let Some(handler_str) = handler_str_opt {
        verbose_enter_scroll();
        smsg!(0, tr(c"autocommand %s"), &handler_str);
        msg_puts(c"\n");
        verbose_leave_scroll();
    }

    AU_STATE.with(|s| s.autocmd_nested.set(nested));
    g_st().current_sctx = script_ctx;
    apc.script_ctx.set(g_st().current_sctx);

    let retval = match cmd_or_cb {
        Ok(cmd) => cmd,
        Err((cb, id, group, saved_pat)) => {
            let rv = au_callback(cb, id, group, apc);
            if oneshot {
                with_autocmds_for_event(apc.event, |acs| {
                    acs[auidx].pat = saved_pat;
                });
            }
            oneshot = oneshot || rv;
            CString::default()
        }
    };

    if oneshot {
        with_autocmds_for_event(apc.event, |acs| {
            aucmd_del(&mut acs[auidx]);
        });
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
    sfname: Option<&CStr>,
    mut buf: Option<&mut FileBuf>,
    is_full: bool,
) -> bool {
    if !has_event(event) {
        return false;
    }

    let tail = path_tail(sfname.unwrap_or(c""));
    let mut retval = false;

    let fname = if is_full {
        sfname.map(CStr::to_owned)
    } else {
        FullName_save(sfname, false)
    };
    let Some(fname) = fname else {
        return false;
    };

    AUTOCMDS.with(|a| {
        for ac in a[event as usize].borrow().iter() {
            if let Some(ref ap) = ac.pat {
                let matched = if ap.buflocal_nr == 0 {
                    match_file_pat(
                        None,
                        ap.reg_prog.as_deref(),
                        Some(&fname),
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
    });

    retval
}

pub fn has_autocmd(event: AuEvent, sfname: Option<&CStr>, buf: Option<&mut FileBuf>) -> bool {
    has_autocmd_ext(event, sfname, buf, false)
}

pub fn has_autocmd_fullname(
    event: AuEvent,
    fname: Option<&CStr>,
    buf: Option<&mut FileBuf>,
) -> bool {
    has_autocmd_ext(event, fname, buf, true)
}

pub fn expand_get_augroup_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    let name = augroup_name(idx + 1)?;
    if name.as_c_str() == get_deleted_augroup() {
        return Some(get_deleted_augroup());
    }
    Some(xp.set_item(name))
}

pub fn aucmd_set_expand_context<'a>(
    xp: &mut ExpandCtx,
    mut arg: &'a CStr,
    doautocmd: bool,
) -> Option<&'a CStr> {
    AU_STATE.with(|s| s.autocmd_include_groups.set(false));
    let p = arg;
    let mut group = arg_augroup_get(&mut arg);

    if arg.to_bytes().is_empty()
        && group != AUGROUP_ALL
        && !ascii_iswhite(
            *crate::globals::GraphPtr::of(arg.as_ptr().cast::<u8>().wrapping_sub(1)).borrow()
                as i32,
        )
    {
        arg = p;
        group = AUGROUP_ALL;
    }

    let ev_base = arg;
    let arg_bytes = arg.to_bytes();
    let mut p_idx = 0usize;
    while p_idx < arg_bytes.len() && !ascii_iswhite(arg_bytes[p_idx] as i32) {
        if arg_bytes[p_idx] == b',' {
            arg = crate::memory::CStrSubslice::advance(ev_base, p_idx + 1);
        }
        p_idx += 1;
    }
    if p_idx == arg_bytes.len() {
        if group == AUGROUP_ALL {
            AU_STATE.with(|s| s.autocmd_include_groups.set(true));
        }
        xp.xp_context = EXPAND_EVENTS;
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
        return None;
    }

    let p_cstr = crate::memory::CStrSubslice::advance(ev_base, p_idx);
    arg = crate::charset::skipwhite(p_cstr);
    let pat_bytes = arg.to_bytes();
    let mut pat_idx = 0usize;
    while pat_idx < pat_bytes.len()
        && (!ascii_iswhite(pat_bytes[pat_idx] as i32)
            || (pat_idx > 0 && pat_bytes[pat_idx - 1] == b'\\'))
    {
        pat_idx += 1;
    }
    arg = crate::memory::CStrSubslice::advance(arg, pat_idx);
    if pat_idx < pat_bytes.len() {
        arg = crate::charset::skipwhite(arg);
        for _ in 0..2 {
            let bytes = arg.to_bytes();
            if bytes.starts_with(b"++once")
                && bytes.get(6).is_some_and(|&b| ascii_iswhite(b as i32))
            {
                arg = crate::charset::skipwhite_at(arg, 6);
            }
            let bytes = arg.to_bytes();
            if bytes.starts_with(b"++nested")
                && bytes.get(8).is_some_and(|&b| ascii_iswhite(b as i32))
            {
                arg = crate::charset::skipwhite_at(arg, 8);
            }
            let bytes = arg.to_bytes();
            if bytes.starts_with(b"nested")
                && bytes.get(6).is_some_and(|&b| ascii_iswhite(b as i32))
            {
                arg = crate::charset::skipwhite_at(arg, 6);
            }
        }
        return Some(arg);
    }

    xp.xp_context = if doautocmd {
        EXPAND_FILES
    } else {
        EXPAND_NOTHING
    };
    None
}

pub fn expand_get_event_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if let Some(name) = augroup_name(idx + 1) {
        if !AU_STATE.with(|s| s.autocmd_include_groups.get())
            || name.as_c_str() == get_deleted_augroup()
        {
            return Some(c"");
        }
        return Some(xp.set_item(name));
    }

    let i = usize::try_from(idx - AU_STATE.with(|s| s.next_augroup_id.get())).ok()?;
    EVENT_NAMES.get(i).map(|e| e.name)
}

pub fn get_event_name_no_group(_xp: &mut ExpandCtx, idx: i32, win: bool) -> Option<&'static CStr> {
    let i = usize::try_from(idx).ok()?;
    if i >= NUM_EVENTS {
        return None;
    }

    if !win {
        return Some(EVENT_NAMES[i].name);
    }

    let mut j = 0;
    for item in &EVENT_NAMES {
        if item.event <= 0 {
            j += 1;
        }
        if j == idx + 1 {
            return Some(item.name);
        }
    }
    None
}

pub fn autocmd_supported(event: &CStr) -> bool {
    let mut rem = event.to_bytes();
    event_name2nr(&mut rem) != NUM_EVENTS as AuEvent
}

pub fn au_exists(arg: &CStr) -> bool {
    let mut buflocal_fnum: Option<i32> = None;
    let mut retval = false;

    let mut arg_save_vec: Vec<u8> = arg.to_bytes_with_nul().to_vec();
    let first_hash = arg_save_vec.iter().position(|&b| b == b'#');
    if let Some(idx) = first_hash {
        arg_save_vec[idx] = 0;
    }

    let arg_save = crate::memory::bytes_as_cstr(&arg_save_vec);
    let mut group = augroup_find(arg_save);
    let mut p_idx = first_hash.map(|idx| idx + 1);
    let ev_start = if group == AUGROUP_ERROR {
        group = AUGROUP_ALL;
        0
    } else {
        let Some(start) = p_idx else {
            return true;
        };
        let second_hash = arg_save_vec[start..].iter().position(|&b| b == b'#');
        if let Some(rel) = second_hash {
            arg_save_vec[start + rel] = 0;
            p_idx = Some(start + rel + 1);
        } else {
            p_idx = None;
        }
        start
    };

    let event_name = crate::memory::bytes_as_cstr(&arg_save_vec[ev_start..]);
    let pattern_opt = p_idx.map(|idx| crate::memory::bytes_as_cstr(&arg_save_vec[idx..]));
    let mut ev_rem = event_name.to_bytes();
    let event = event_name2nr(&mut ev_rem);
    if event != NUM_EVENTS as AuEvent {
        AUTOCMDS.with(|a| {
            let acs = a[event as usize].borrow();
            if !acs.is_empty() {
                if let Some(pattern) = pattern_opt
                    && pattern.to_bytes().eq_ignore_ascii_case(b"<buffer>")
                {
                    buflocal_fnum = Some(curbuf_mut().b_fnum);
                }

                for ac in acs.iter() {
                    if let Some(ref ap) = ac.pat
                        && (group == AUGROUP_ALL || ap.group == group)
                    {
                        let pat_match = if let Some(pattern) = pattern_opt {
                            if let Some(fnum) = buflocal_fnum {
                                ap.buflocal_nr == fnum
                            } else {
                                mb_strcmp_ic(opt_vars().p_fic, ap.pat.as_c_str(), pattern) == 0
                            }
                        } else {
                            true
                        };
                        if pat_match {
                            retval = true;
                            break;
                        }
                    }
                }
            }
        });
    }

    retval
}

pub fn aupat_is_buflocal(slice: &[u8]) -> bool {
    slice.len() >= 8 && slice.starts_with(b"<buffer") && slice[slice.len() - 1] == b'>'
}

pub fn aupat_get_buflocal_nr(slice: &[u8]) -> i32 {
    assert!(aupat_is_buflocal(slice));
    let patlen = slice.len();

    if patlen == 8 {
        return curbuf_mut().b_fnum;
    }

    if patlen > 9 && slice.get(7) == Some(&b'=') {
        if patlen == 13 && slice.eq_ignore_ascii_case(b"<buffer=abuf>") {
            return g_st().autocmd_bufnr;
        }

        let digits = &slice[8..(patlen - 1)];
        if !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) {
            let mut rem = digits;
            return getdigits_int(&mut rem, false, 0);
        }
    }

    0
}

pub fn aupat_normalize_buflocal_pat(dest: &mut [u8], pat: &[u8], mut buflocal_nr: i32) {
    assert!(aupat_is_buflocal(pat));

    if buflocal_nr == 0 {
        buflocal_nr = curbuf_mut().b_fnum;
    }

    let s = format!("<buffer={buflocal_nr}>");
    let copy_len = s.len().min(dest.len().saturating_sub(1));
    dest[..copy_len].copy_from_slice(&s.as_bytes()[..copy_len]);
    if !dest.is_empty() {
        dest[copy_len] = 0;
    }
}

pub fn autocmd_delete_id(id: i64) -> bool {
    assert!(id > 0);
    let mut success = false;

    AUTOCMDS.with(|all| {
        for cell in all {
            for ac in cell.borrow_mut().iter_mut() {
                if ac.id == id {
                    aucmd_del(ac);
                    success = true;
                }
            }
        }
    });
    success
}

pub fn aucmd_handler_to_string(ac: &mut AutoCmd) -> CString {
    match &mut ac.handler {
        AutoCmdHandler::Cmd(cmd) => cmd.clone(),
        AutoCmdHandler::Fn(cb) => callback_to_string(cb),
        AutoCmdHandler::None => CString::default(),
    }
}

fn arg_event_skip(arg: &CStr, have_group: bool) -> Option<&CStr> {
    let arg_bytes = arg.to_bytes();
    if arg_bytes.first() == Some(&b'*') {
        if arg_bytes
            .get(1)
            .is_some_and(|&b| !ascii_iswhite(i32::from(b)))
        {
            semsg!(tr(c"E215: Illegal character after *: %s"), arg);
            return None;
        }
        Some(crate::memory::CStrSubslice::advance(arg, 1))
    } else {
        let mut rem = arg_bytes;
        while !rem.is_empty() && rem[0] != b'|' && !ascii_iswhite(i32::from(rem[0])) {
            let pat_offset = arg_bytes.len() - rem.len();
            if (event_name2nr(&mut rem) as usize) >= NUM_EVENTS {
                let pat = crate::memory::CStrSubslice::advance(arg, pat_offset);
                if have_group {
                    semsg!(tr(c"E216: No such event: %s"), pat);
                } else {
                    semsg!(tr(c"E216: No such group or event: %s"), pat);
                }
                return None;
            }
        }
        Some(crate::memory::CStrSubslice::advance(
            arg,
            arg_bytes.len() - rem.len(),
        ))
    }
}

fn arg_augroup_get(argp_ref: &mut &CStr) -> i32 {
    let arg_bytes = argp_ref.to_bytes();
    let len = arg_bytes
        .iter()
        .take_while(|&&b| !ascii_iswhite(b as i32) && b != b'|')
        .count();
    if len == 0 {
        return AUGROUP_ALL;
    }

    let group_name = CString::new(&arg_bytes[..len]).unwrap_or_default();
    let mut group = augroup_find(&group_name);
    if group == AUGROUP_ERROR {
        group = AUGROUP_ALL;
    } else {
        *argp_ref = crate::charset::skipwhite_at(argp_ref, len);
    }
    group
}

fn arg_autocmd_flag_get(flag: &mut bool, cmd_ptr: &mut &CStr, pat_slice: &[u8]) -> bool {
    let cmd_bytes = cmd_ptr.to_bytes();
    let len = pat_slice.len();
    if cmd_bytes.starts_with(pat_slice)
        && cmd_bytes.get(len).is_some_and(|&b| ascii_iswhite(b as i32))
    {
        if *flag {
            let pat_cstr = CString::new(pat_slice).unwrap_or_default();
            semsg!(tr(E_DUPARG2), &pat_cstr);
            return true;
        }
        *flag = true;
        *cmd_ptr = crate::charset::skipwhite_at(cmd_ptr, len);
    }
    false
}

pub fn vimresume_event() {
    apply_autocmds(EVENT_VIMRESUME, None, None, false, None);
    AU_STATE.with(|s| s.pending_vimresume.set(K_FALSE));
}

pub fn may_trigger_vim_suspend_resume(suspend: bool) {
    if suspend && AU_STATE.with(|s| s.pending_vimresume.get()) == K_FALSE {
        AU_STATE.with(|s| s.pending_vimresume.set(K_NONE));
        apply_autocmds(EVENT_VIMSUSPEND, None, None, false, None);
        AU_STATE.with(|s| s.pending_vimresume.set(K_TRUE));
    } else if !suspend && AU_STATE.with(|s| s.pending_vimresume.get()) == K_TRUE {
        AU_STATE.with(|s| s.pending_vimresume.set(K_NONE));
        let events = loop_queues(&mut g_st().main_loop).events.borrow_mut();
        multiqueue_put_event(events, Event::AutoCmdVimResume);
    }
}

pub fn do_autocmd_uienter(chanid: u64, attached: bool) {
    if starting_val() == NO_SCREEN || AU_STATE.with(|s| s.uienter_recursive.get()) {
        return;
    }
    AU_STATE.with(|s| s.uienter_recursive.set(true));

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
        None,
        None,
        false,
        curbuf_opt_mut(),
    );
    restore_v_event(&mut *dict, &mut save_v_event);

    AU_STATE.with(|s| s.uienter_recursive.set(false));
}

pub fn do_autocmd_focusgained(gained: bool) {
    if AU_STATE.with(|s| s.focusgained_recursive.get()) {
        return;
    }
    AU_STATE.with(|s| s.focusgained_recursive.set(true));
    apply_autocmds(
        if gained {
            EVENT_FOCUSGAINED
        } else {
            EVENT_FOCUSLOST
        },
        None,
        None,
        false,
        curbuf_opt_mut(),
    );

    if gained && AU_STATE.with(|s| s.focusgained_last_time.get()) + 2000 < os_now() {
        check_timestamps(1);
        AU_STATE.with(|s| s.focusgained_last_time.set(os_now()));
    }

    AU_STATE.with(|s| s.focusgained_recursive.set(false));
}

pub fn do_filetype_autocmd(buf: &mut FileBuf, force: bool) -> bool {
    let mut buf_ptr = crate::globals::GraphPtr::from(&mut *buf);
    if AU_STATE.with(|s| s.ft_recursive.get()) > 0 && !force {
        return false;
    }

    let secure_save = g_st().secure;
    g_st().secure = 0;

    AU_STATE.with(|s| s.ft_recursive.set(s.ft_recursive.get() + (1)));
    buf_ptr.b_did_filetype = true;
    let ret = apply_autocmds(
        EVENT_FILETYPE,
        Some(buf_ptr.b_p_ft.as_cstr()),
        buf_ptr.fname_opt(),
        force || AU_STATE.with(|s| s.ft_recursive.get()) == 1,
        buf_ptr.opt_mut(),
    );
    AU_STATE.with(|s| s.ft_recursive.set(s.ft_recursive.get() - (1)));

    g_st().secure = secure_save;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_name_lookup_and_span_pattern_work() {
        let mut end: &[u8] = b"BufReadPost,BufEnter";
        let ev = event_name2nr(&mut end);
        assert_eq!(ev, EVENT_BUFREADPOST);
        assert_eq!(end, b"BufEnter");
        assert_eq!(event_nr2name(EVENT_VIMENTER).to_bytes(), b"VimEnter");

        let (span, rest) = aucmd_span_pattern_slice(b",,*.{c,h},*.rs").unwrap();
        assert_eq!(span, b"*.{c,h}");
        assert_eq!(rest, b",*.rs");
    }
}
