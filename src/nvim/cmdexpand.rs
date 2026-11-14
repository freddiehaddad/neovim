//! Functions for command-line completion (`src/nvim/cmdexpand.c`).
#![forbid(unsafe_code)]

use crate::api::command::is_user_cmdidx;
use crate::api::private::helpers::{api_clear_error, api_free_object, cstr_as_string};
use crate::api::private::validate::{Array, Error, NvimString, Object, ObjectData, ObjectType};
use crate::arglist::get_arglist_name;
use crate::autocmd::{
    HLF_T, aucmd_set_expand_context, expand_get_augroup_name, expand_get_event_name,
};
use crate::buffer::{K_UIMESSAGES, expand_bufnames};
use crate::charset::{
    CTRL_V_CONST, ascii_isdigit, ascii_isspace, ascii_iswhite, backslash_halve_save, rem_backslash,
    transchar, transchar_byte, vim_isfilec_or_wc, vim_strsize,
};
use crate::cmdhist::get_history_arg;
use crate::cursor::{CursorPos, ltoreq};
use crate::drawscreen::{redraw_statuslines, update_screen, win_redraw_last_status};
use crate::errors::*;
use crate::eval::funcs::{get_expr_name, get_function_name};
use crate::eval::typval::{
    Callback, EvalFuncData, TvList, TypVal, VarType, tv_check_for_string_arg, tv_clear,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str_len, tv_dict_alloc_ret, tv_dict_get_string,
    tv_get_bool_chk, tv_get_string, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_string, tv_list_unref,
};
use crate::eval::userfunc::get_user_func_name;
use crate::eval::vars::get_user_var_name;
use crate::eval::{call_func_retlist, call_func_retstr, set_context_for_expression};
use crate::ex_cmds::skip_vimgrep_pat;
use crate::ex_docmd::{
    EX_ARGOPT, EX_CMDARG, EX_EXTRA, EX_NOSPC, EX_TRLBAR, EX_XFILE, Exarg, ends_excmd,
    excmd_get_argt, excmd_get_cmdidx, expand_argopt, expand_findfunc, find_nextcmd,
    get_command_name, set_no_hlsearch, skip_cmd_arg, skip_range,
};
use crate::ex_getln::{
    Hlf, VSE_BUFFER, VSE_NONE, cmd_screencol, cursorcmd, escape_fname_cstr, get_cmdline_info,
    get_cmdline_last_prompt_id, parse_pattern_and_range, put_on_cmdline, realloc_cmdbuff,
    redrawcmd, tilde_replace, vim_strsave_fnameescape,
};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, FuzmatchStr, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
};
use crate::globals::*;
use crate::grid::{grid_line_fill, grid_line_flush, grid_line_puts, grid_line_start};

use crate::help::{cleanup_help_tags, find_help_tags};
use crate::highlight::{hl_attr, win_hl_attr};
use crate::highlight_group::{get_highlight_name, set_context_in_highlight_cmd};
use crate::input::{beep_flush, char_avail, vpeekc};
use crate::insexpand::find_word_end;
use crate::keycodes::{K_DOWN, K_KENTER, K_LEFT, K_RIGHT, K_UP};
use crate::lua::executor::{
    K_RET_OBJECT, nlua_call_user_expand_func, nlua_exec, nlua_expand_get_matches, nlua_expand_pat,
};
use crate::mapping::{expand_mappings, set_context_in_map_cmd};
use crate::mbyte::{mb_tolower, utf_ptr2char, utfc_ptr2len};
use crate::memline::ml_get_len;
use crate::memory::bytes_to_cstring;
use crate::menu::{get_menu_name, get_menu_names, menu_is_separator, set_context_in_menu_cmd};
use crate::message::{
    emsg, msg_advance, msg_clr_eos, msg_ext_set_kind, msg_outtrans, msg_outtrans_long, msg_putchar,
    msg_puts, msg_puts_hl, msg_scroll_up, msg_start, semsg,
};
#[cfg(unix)]
use crate::option::csh_like_shell;
use crate::option::{
    OptInt, copy_option_part, expand_old_setting, expand_setting_subtract, expand_settings,
    expand_string_setting, get_findfunc, set_context_in_set_cmd,
};
use crate::option_vars::*;
use crate::os::env::{env_name_len, expand_env_save_opt, get_env_name, home_replace, vim_getenv};
#[cfg(not(windows))]
use crate::os::fs::PATHSEP;
#[cfg(windows)]
use crate::os::fs::PATHSEP;
use crate::os::fs::os_isdir;
use crate::os::lang::{get_lang_arg, get_locales};
use crate::os::users::{get_users, match_user};
use crate::path::{
    EW_ADDSLASH, EW_ALLLINKS, EW_CDPATH, EW_DIR, EW_EXEC, EW_FILE, EW_ICASE, EW_KEEPALL,
    EW_NOERROR, EW_NOTFOUND, EW_PATH, EW_SHELLCMD, EW_SILENT, after_pathsep, expand_wildcards_eval,
    expand_wildcards_vec, match_suffix, path_is_absolute, path_tail,
};
use crate::popupmenu::{
    Pumitem, pum_clear, pum_display, pum_get_height, pum_undisplay, pum_visible,
};
use crate::profile::{get_profile_name, set_context_in_profile_cmd};
use crate::regexp::{
    RE_MAGIC, RE_STRING, Regmatch, skip_regexp, vim_regcomp, vim_regexec, vim_regexec_nl,
};
use crate::runtime::{
    DIP_OPT, DIP_START, ExpandPackAddDir, expand_rtdir, expand_runtime_cmd, script_id_valid,
    script_item_name, set_context_in_runtime_cmd,
};
use crate::search::{Direction, ignorecase, pat_has_uppercase, searchit};
use crate::sign::{get_sign_name, set_context_in_sign_cmd};
use crate::statusline::fillchar_status;
use crate::strings::{
    format_to_buf, gettext, sort_strings, strcase_save, vim_strchr, vim_strsave_escaped,
};
use crate::syntax::{
    get_syntax_name, get_syntime_arg, reset_expand_highlight, set_context_in_echohl_cmd,
    set_context_in_syntax_cmd,
};
use crate::tag::expand_tags;
use crate::ui::{ui_flush, ui_has, vim_beep};
use crate::usercmd::{
    cmdcomplete_str_to_type, cmdcomplete_type_to_str, find_ucmd, get_user_cmd_addr_type,
    get_user_cmd_complete, get_user_cmd_flags, get_user_cmd_nargs, get_user_commands,
    set_context_in_user_cmd, set_context_in_user_cmdarg,
};
use crate::window::{global_stl_height, last_status};
use std::ffi::{CStr, CString};

use crate::api::win_config::UIExtension;
use crate::lua::converter::LuaRef;
use crate::runtime::Sctx;

pub(crate) const EXPAND_BUF_LEN: usize = 256;

pub struct ExpandCtx {
    pub xp_pattern: crate::strings::OwnedSlice,
    pub xp_context: i32,
    pub xp_pattern_len: usize,
    pub xp_prefix: i32,
    pub xp_arg: Option<std::ffi::CString>,
    pub xp_luaref: LuaRef,
    pub xp_script_ctx: Sctx,
    pub xp_backslash: i32,
    #[cfg(not(windows))]
    pub xp_shell: bool,
    pub xp_numfiles: i32,
    pub xp_col: i32,
    pub xp_selected: i32,
    pub xp_orig: Option<std::ffi::CString>,
    pub xp_files: Vec<std::ffi::CString>,
    pub xp_files_abbr: Vec<Option<std::ffi::CString>>,
    pub xp_files_kind: Vec<Option<std::ffi::CString>>,
    pub xp_files_menu: Vec<Option<std::ffi::CString>>,
    pub xp_files_info: Vec<Option<std::ffi::CString>>,
    pub xp_line: crate::strings::OwnedSlice,
    pub xp_buf: [u8; EXPAND_BUF_LEN],
    pub xp_item: Option<std::ffi::CString>,
    pub xp_search_dir: i32,
    pub xp_pre_incsearch_pos: CursorPos,
}

impl ExpandCtx {
    pub const fn new() -> Self {
        Self {
            xp_pattern: crate::strings::OwnedSlice::NULL,
            xp_context: 0,
            xp_pattern_len: 0,
            xp_prefix: XP_PREFIX_NONE,
            xp_arg: None,
            xp_luaref: 0,
            xp_script_ctx: Sctx {
                sc_sid: 0,
                sc_seq: 0,
                sc_lnum: 0,
                sc_chan: 0,
            },
            xp_backslash: XP_BS_NONE,
            #[cfg(not(windows))]
            xp_shell: false,
            xp_numfiles: -1,
            xp_col: 0,
            xp_selected: 0,
            xp_orig: None,
            xp_files: Vec::new(),
            xp_files_abbr: Vec::new(),
            xp_files_kind: Vec::new(),
            xp_files_menu: Vec::new(),
            xp_files_info: Vec::new(),
            xp_line: crate::strings::OwnedSlice::NULL,
            xp_buf: [0; EXPAND_BUF_LEN],
            xp_item: None,
            xp_search_dir: 0,
            xp_pre_incsearch_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
        }
    }

    #[inline]
    pub fn set_item(&mut self, item: std::ffi::CString) -> &CStr {
        self.xp_item = Some(item);
        self.xp_item.as_deref().unwrap()
    }
}

impl Default for ExpandCtx {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ExpandCtx {
    fn drop(&mut self) {
        expand_cleanup(self);
    }
}

pub type XpPrefix = i32;
pub type CompleteListItemGetter = Option<fn(&mut ExpandCtx, i32) -> Option<&CStr>>;
#[derive(Clone)]
pub enum LineGetter {
    None,
    ExLine,
    CmdKeyCmd,
    AutoCmd(std::rc::Rc<std::cell::RefCell<crate::autocmd::AutoPatCmd>>),
    Func(crate::eval::userfunc::FunccalId),
    Source(std::rc::Rc<std::cell::RefCell<crate::runtime::SourceCookie>>),
    Loop(std::rc::Rc<std::cell::RefCell<crate::ex_docmd::LoopCookie>>),
    List(std::rc::Rc<crate::eval::funcs::GetListLineCookie>),
    LuaEx(std::rc::Rc<crate::lua::executor::ExDocmdCookie>),
}

impl LineGetter {
    #[inline]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    #[inline]
    pub fn inner(&self) -> Self {
        let mut gp = self.clone();
        while let Self::Loop(cp) = gp {
            gp = cp.borrow().lc_getline.clone();
        }
        gp
    }

    #[inline]
    pub fn is_exline(&self) -> bool {
        matches!(self.inner(), Self::ExLine)
    }

    #[inline]
    pub fn is_autocmd(&self) -> bool {
        matches!(self.inner(), Self::AutoCmd(_))
    }

    #[inline]
    pub fn is_func(&self) -> bool {
        matches!(self.inner(), Self::Func(_))
    }

    #[inline]
    pub fn is_source(&self) -> bool {
        matches!(self.inner(), Self::Source(_))
    }

    #[inline]
    pub fn as_func(&self) -> Option<std::rc::Rc<crate::profile::Funccall>> {
        match self.inner() {
            Self::Func(id) => id.rc(),
            _ => None,
        }
    }

    #[inline]
    pub fn as_source(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<crate::runtime::SourceCookie>>> {
        match self.inner() {
            Self::Source(p) => Some(p),
            _ => None,
        }
    }

    #[inline]
    pub fn breakpoint_get(&self) -> Option<crate::cursor::Linenr> {
        match self.inner() {
            Self::Func(id) => id.rc().map(|fc| fc.fc_breakpoint.get()),
            Self::Source(p) => Some(p.borrow().breakpoint),
            _ => None,
        }
    }

    #[inline]
    pub fn breakpoint_set(&self, val: crate::cursor::Linenr) {
        match self.inner() {
            Self::Func(id) => {
                if let Some(fc) = id.rc() {
                    fc.fc_breakpoint.set(val);
                }
            }
            Self::Source(p) => {
                p.borrow_mut().breakpoint = val;
            }
            _ => {}
        }
    }

    #[inline]
    pub fn dbg_tick_get(&self) -> Option<i32> {
        match self.inner() {
            Self::Func(id) => id.rc().map(|fc| fc.fc_dbg_tick.get()),
            Self::Source(p) => Some(p.borrow().dbg_tick),
            _ => None,
        }
    }

    #[inline]
    pub fn dbg_tick_set(&self, val: i32) {
        match self.inner() {
            Self::Func(id) => {
                if let Some(fc) = id.rc() {
                    fc.fc_dbg_tick.set(val);
                }
            }
            Self::Source(p) => {
                p.borrow_mut().dbg_tick = val;
            }
            _ => {}
        }
    }

    pub fn call(&self, c: i32, indent: i32, do_concat: bool) -> Option<std::ffi::CString> {
        match self {
            Self::None => None,
            Self::ExLine => crate::ex_getln::getexline(c, indent, do_concat),
            Self::CmdKeyCmd => crate::input::getcmdkeycmd(c, indent, do_concat),
            Self::AutoCmd(cookie) => crate::autocmd::getnextac(c, cookie, indent, do_concat),
            Self::Func(cookie) => {
                crate::eval::userfunc::get_func_line(c, &cookie.unwrap_rc(), indent, do_concat)
            }
            Self::Source(cookie) => {
                crate::runtime::getsourceline(c, &mut cookie.borrow_mut(), indent, do_concat)
            }
            Self::Loop(cookie) => crate::ex_docmd::get_loop_line(c, cookie, indent, do_concat),
            Self::List(cookie) => crate::eval::funcs::get_list_line(c, cookie, indent, do_concat),
            Self::LuaEx(cookie) => {
                crate::lua::executor::nlua_ex_getline(c, cookie, indent, do_concat)
            }
        }
    }
}
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CmdAddr {
    #[default]
    Lines = 0,
    Windows = 1,
    Arguments = 2,
    LoadedBuffers = 3,
    Buffers = 4,
    Tabs = 5,
    TabsRelative = 6,
    QuickfixValid = 7,
    Quickfix = 8,
    Unsigned = 9,
    Other = 10,
    None = 11,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Cmdidx {
    #[default]
    CmdAppend = 0,
    CmdAbbreviate,
    CmdAbclear,
    CmdAboveleft,
    CmdAll,
    CmdAmenu,
    CmdAnoremenu,
    CmdArgs,
    CmdArgadd,
    CmdArgdelete,
    CmdArgdo,
    CmdArgdedupe,
    CmdArgedit,
    CmdArgglobal,
    CmdArglocal,
    CmdArgument,
    CmdAscii,
    CmdAutocmd,
    CmdAugroup,
    CmdAunmenu,
    CmdBuffer,
    CmdBnext,
    CmdBall,
    CmdBadd,
    CmdBalt,
    CmdBcd,
    CmdBchdir,
    CmdBdelete,
    CmdBelowright,
    CmdBfirst,
    CmdBlast,
    CmdBmodified,
    CmdBnextVariant,
    CmdBotright,
    CmdBprevious,
    CmdBrewind,
    CmdBreak,
    CmdBreakadd,
    CmdBreakdel,
    CmdBreaklist,
    CmdBrowse,
    CmdBuffers,
    CmdBufdo,
    CmdBunload,
    CmdBwipeout,
    CmdChange,
    CmdCnext,
    CmdCnfile,
    CmdCabbrev,
    CmdCabclear,
    CmdCabove,
    CmdCaddbuffer,
    CmdCaddexpr,
    CmdCaddfile,
    CmdCafter,
    CmdCall,
    CmdCatch,
    CmdCbuffer,
    CmdCbefore,
    CmdCbelow,
    CmdCbottom,
    CmdCc,
    CmdCclose,
    CmdCd,
    CmdCdo,
    CmdCenter,
    CmdCexpr,
    CmdCfile,
    CmdCfdo,
    CmdCfirst,
    CmdCgetfile,
    CmdCgetbuffer,
    CmdCgetexpr,
    CmdChdir,
    CmdChanges,
    CmdCheckhealth,
    CmdCheckpath,
    CmdChecktime,
    CmdChistory,
    CmdClist,
    CmdClast,
    CmdClose,
    CmdClearjumps,
    CmdCmap,
    CmdCmapclear,
    CmdCmenu,
    CmdCnextVariant,
    CmdCnewer,
    CmdCnfileVariant,
    CmdCnoremap,
    CmdCnoreabbrev,
    CmdCnoremenu,
    CmdCopy,
    CmdColder,
    CmdColorscheme,
    CmdCommand,
    CmdComclear,
    CmdCompiler,
    CmdContinue,
    CmdConfirm,
    CmdConnect,
    CmdConst,
    CmdCopen,
    CmdCprevious,
    CmdCpfile,
    CmdCquit,
    CmdCrewind,
    CmdCunmap,
    CmdCunabbrev,
    CmdCunmenu,
    CmdCwindow,
    CmdDelete,
    CmdDelmarks,
    CmdDebug,
    CmdDebuggreedy,
    CmdDefer,
    CmdDelcommand,
    CmdDelfunction,
    CmdDetach,
    CmdDisplay,
    CmdDiffupdate,
    CmdDiffget,
    CmdDiffoff,
    CmdDiffpatch,
    CmdDiffput,
    CmdDiffsplit,
    CmdDiffthis,
    CmdDigraphs,
    CmdDjump,
    CmdDlist,
    CmdDoautocmd,
    CmdDoautoall,
    CmdDrop,
    CmdDsearch,
    CmdDsplit,
    CmdEdit,
    CmdEarlier,
    CmdEcho,
    CmdEchoerr,
    CmdEchohl,
    CmdEchomsg,
    CmdEchon,
    CmdElse,
    CmdElseif,
    CmdEmenu,
    CmdEndif,
    CmdEndfunction,
    CmdEndfor,
    CmdEndtry,
    CmdEndwhile,
    CmdEnew,
    CmdEval,
    CmdEx,
    CmdExecute,
    CmdExit,
    CmdExmode,
    CmdExusage,
    CmdFile,
    CmdFiles,
    CmdFiletype,
    CmdFilter,
    CmdFind,
    CmdFinally,
    CmdFinish,
    CmdFirst,
    CmdFold,
    CmdFoldclose,
    CmdFolddoopen,
    CmdFolddoclosed,
    CmdFoldopen,
    CmdFor,
    CmdFunction,
    CmdFclose,
    CmdGlobal,
    CmdGoto,
    CmdGrep,
    CmdGrepadd,
    CmdGui,
    CmdGvim,
    CmdHelp,
    CmdHelpclose,
    CmdHelpgrep,
    CmdHelptags,
    CmdHighlight,
    CmdHide,
    CmdHistory,
    CmdHorizontal,
    CmdInsert,
    CmdIabbrev,
    CmdIabclear,
    CmdIf,
    CmdIjump,
    CmdIlist,
    CmdImap,
    CmdImapclear,
    CmdImenu,
    CmdInoremap,
    CmdInoreabbrev,
    CmdInoremenu,
    CmdIntro,
    CmdIput,
    CmdIsearch,
    CmdIsplit,
    CmdIunmap,
    CmdIunabbrev,
    CmdIunmenu,
    CmdJoin,
    CmdJumps,
    CmdK,
    CmdKeepmarks,
    CmdKeepjumps,
    CmdKeeppatterns,
    CmdKeepalt,
    CmdList,
    CmdLnext,
    CmdLnfile,
    CmdLast,
    CmdLabove,
    CmdLanguage,
    CmdLaddexpr,
    CmdLaddbuffer,
    CmdLaddfile,
    CmdLafter,
    CmdLater,
    CmdLbuffer,
    CmdLbefore,
    CmdLbelow,
    CmdLbottom,
    CmdLcd,
    CmdLchdir,
    CmdLclose,
    CmdLdo,
    CmdLeft,
    CmdLeftabove,
    CmdLet,
    CmdLexpr,
    CmdLfile,
    CmdLfdo,
    CmdLfirst,
    CmdLgetfile,
    CmdLgetbuffer,
    CmdLgetexpr,
    CmdLgrep,
    CmdLgrepadd,
    CmdLhelpgrep,
    CmdLhistory,
    CmdLl,
    CmdLlast,
    CmdLlist,
    CmdLmap,
    CmdLmapclear,
    CmdLmake,
    CmdLnoremap,
    CmdLnextVariant,
    CmdLnewer,
    CmdLnfileVariant,
    CmdLoadview,
    CmdLoadkeymap,
    CmdLockmarks,
    CmdLockvar,
    CmdLog,
    CmdLolder,
    CmdLopen,
    CmdLprevious,
    CmdLpfile,
    CmdLrewind,
    CmdLtag,
    CmdLunmap,
    CmdLua,
    CmdLuado,
    CmdLuafile,
    CmdLvimgrep,
    CmdLvimgrepadd,
    CmdLwindow,
    CmdLs,
    CmdLsp,
    CmdMove,
    CmdMark,
    CmdMake,
    CmdMap,
    CmdMapclear,
    CmdMarks,
    CmdMatch,
    CmdMenu,
    CmdMenutranslate,
    CmdMessages,
    CmdMkexrc,
    CmdMksession,
    CmdMkspell,
    CmdMkvimrc,
    CmdMkview,
    CmdMode,
    CmdMzscheme,
    CmdMzfile,
    CmdNext,
    CmdNew,
    CmdNmap,
    CmdNmapclear,
    CmdNmenu,
    CmdNnoremap,
    CmdNnoremenu,
    CmdNoremap,
    CmdNoautocmd,
    CmdNohlsearch,
    CmdNoreabbrev,
    CmdNoremenu,
    CmdNoswapfile,
    CmdNormal,
    CmdNumber,
    CmdNunmap,
    CmdNunmenu,
    CmdOldfiles,
    CmdOmap,
    CmdOmapclear,
    CmdOmenu,
    CmdOnly,
    CmdOnoremap,
    CmdOnoremenu,
    CmdOptions,
    CmdOunmap,
    CmdOunmenu,
    CmdOwnsyntax,
    CmdPrint,
    CmdPackadd,
    CmdPackdel,
    CmdPackloadall,
    CmdPackupdate,
    CmdPbuffer,
    CmdPclose,
    CmdPerl,
    CmdPerldo,
    CmdPerlfile,
    CmdPedit,
    CmdPop,
    CmdPopup,
    CmdPpop,
    CmdPreserve,
    CmdPrevious,
    CmdProfile,
    CmdProfdel,
    CmdPsearch,
    CmdPtag,
    CmdPtnext,
    CmdPtfirst,
    CmdPtjump,
    CmdPtlast,
    CmdPtnextVariant,
    CmdPtprevious,
    CmdPtrewind,
    CmdPtselect,
    CmdPut,
    CmdPwd,
    CmdPython,
    CmdPydo,
    CmdPyfile,
    CmdPy3,
    CmdPy3do,
    CmdPython3,
    CmdPy3file,
    CmdPyx,
    CmdPyxdo,
    CmdPythonx,
    CmdPyxfile,
    CmdQuit,
    CmdQuitall,
    CmdQall,
    CmdRead,
    CmdRecover,
    CmdRedo,
    CmdRedir,
    CmdRedraw,
    CmdRedrawstatus,
    CmdRedrawtabline,
    CmdRegisters,
    CmdResize,
    CmdRestart,
    CmdRetab,
    CmdReturn,
    CmdRewind,
    CmdRight,
    CmdRightbelow,
    CmdRshada,
    CmdRuntime,
    CmdRundo,
    CmdRuby,
    CmdRubydo,
    CmdRubyfile,
    CmdRviminfo,
    CmdSubstitute,
    CmdSnext,
    CmdSargument,
    CmdSall,
    CmdSandbox,
    CmdSaveas,
    CmdSbuffer,
    CmdSbnext,
    CmdSball,
    CmdSbfirst,
    CmdSblast,
    CmdSbmodified,
    CmdSbnextVariant,
    CmdSbprevious,
    CmdSbrewind,
    CmdScriptnames,
    CmdScriptencoding,
    CmdSet,
    CmdSetfiletype,
    CmdSetglobal,
    CmdSetlocal,
    CmdSfind,
    CmdSfirst,
    CmdSimalt,
    CmdSign,
    CmdSilent,
    CmdSleep,
    CmdSlast,
    CmdSmagic,
    CmdSmap,
    CmdSmapclear,
    CmdSmenu,
    CmdSnextVariant,
    CmdSnomagic,
    CmdSnoremap,
    CmdSnoremenu,
    CmdSource,
    CmdSort,
    CmdSplit,
    CmdSpellgood,
    CmdSpelldump,
    CmdSpellinfo,
    CmdSpellrepall,
    CmdSpellrare,
    CmdSpellundo,
    CmdSpellwrong,
    CmdSprevious,
    CmdSrewind,
    CmdStop,
    CmdStag,
    CmdStartinsert,
    CmdStartgreplace,
    CmdStartreplace,
    CmdStopinsert,
    CmdStjump,
    CmdStselect,
    CmdSunhide,
    CmdSunmap,
    CmdSunmenu,
    CmdSuspend,
    CmdSview,
    CmdSwapname,
    CmdSyntax,
    CmdSyntime,
    CmdSyncbind,
    CmdT,
    CmdTcd,
    CmdTchdir,
    CmdTnext,
    CmdTag,
    CmdTags,
    CmdTab,
    CmdTabclose,
    CmdTabdo,
    CmdTabedit,
    CmdTabfind,
    CmdTabfirst,
    CmdTabmove,
    CmdTablast,
    CmdTabnextVariant,
    CmdTabnew,
    CmdTabonly,
    CmdTabprevious,
    CmdTabnext,
    CmdTabrewind,
    CmdTabs,
    CmdTcl,
    CmdTcldo,
    CmdTclfile,
    CmdTerminal,
    CmdTfirst,
    CmdThrow,
    CmdTjump,
    CmdTlast,
    CmdTlmenu,
    CmdTlnoremenu,
    CmdTlunmenu,
    CmdTmenu,
    CmdTmap,
    CmdTmapclear,
    CmdTnextVariant,
    CmdTnoremap,
    CmdTopleft,
    CmdTprevious,
    CmdTrewind,
    CmdTrust,
    CmdTry,
    CmdTselect,
    CmdTunmenu,
    CmdTunmap,
    CmdUndo,
    CmdUndojoin,
    CmdUndolist,
    CmdUnabbreviate,
    CmdUnhide,
    CmdUniq,
    CmdUnlet,
    CmdUnlockvar,
    CmdUnmap,
    CmdUnmenu,
    CmdUnsilent,
    CmdUpdate,
    CmdUptime,
    CmdVglobal,
    CmdVersion,
    CmdVerbose,
    CmdVertical,
    CmdVisual,
    CmdView,
    CmdVimgrep,
    CmdVimgrepadd,
    CmdViusage,
    CmdVmap,
    CmdVmapclear,
    CmdVmenu,
    CmdVnoremap,
    CmdVnew,
    CmdVnoremenu,
    CmdVsplit,
    CmdVunmap,
    CmdVunmenu,
    CmdWrite,
    CmdWnext,
    CmdWall,
    CmdWhile,
    CmdWinsize,
    CmdWincmd,
    CmdWindo,
    CmdWinpos,
    CmdWnextVariant,
    CmdWprevious,
    CmdWq,
    CmdWqall,
    CmdWshada,
    CmdWundo,
    CmdWviminfo,
    CmdXit,
    CmdXall,
    CmdXmap,
    CmdXmapclear,
    CmdXmenu,
    CmdXnoremap,
    CmdXnoremenu,
    CmdXunmap,
    CmdXunmenu,
    CmdYank,
    CmdZ,
    CmdBang,
    CmdPound,
    CmdAnd,
    CmdLshift,
    CmdEqual,
    CmdRshift,
    CmdAt,
    CmdTilde,
    CmdCapNext,
    CmdSize,
    CmdUser = -1,
    CmdUserBuf = -2,
}
impl Cmdidx {
    pub const ALL: [Self; 564] = [
        Self::CmdAppend,
        Self::CmdAbbreviate,
        Self::CmdAbclear,
        Self::CmdAboveleft,
        Self::CmdAll,
        Self::CmdAmenu,
        Self::CmdAnoremenu,
        Self::CmdArgs,
        Self::CmdArgadd,
        Self::CmdArgdelete,
        Self::CmdArgdo,
        Self::CmdArgdedupe,
        Self::CmdArgedit,
        Self::CmdArgglobal,
        Self::CmdArglocal,
        Self::CmdArgument,
        Self::CmdAscii,
        Self::CmdAutocmd,
        Self::CmdAugroup,
        Self::CmdAunmenu,
        Self::CmdBuffer,
        Self::CmdBnext,
        Self::CmdBall,
        Self::CmdBadd,
        Self::CmdBalt,
        Self::CmdBcd,
        Self::CmdBchdir,
        Self::CmdBdelete,
        Self::CmdBelowright,
        Self::CmdBfirst,
        Self::CmdBlast,
        Self::CmdBmodified,
        Self::CmdBnextVariant,
        Self::CmdBotright,
        Self::CmdBprevious,
        Self::CmdBrewind,
        Self::CmdBreak,
        Self::CmdBreakadd,
        Self::CmdBreakdel,
        Self::CmdBreaklist,
        Self::CmdBrowse,
        Self::CmdBuffers,
        Self::CmdBufdo,
        Self::CmdBunload,
        Self::CmdBwipeout,
        Self::CmdChange,
        Self::CmdCnext,
        Self::CmdCnfile,
        Self::CmdCabbrev,
        Self::CmdCabclear,
        Self::CmdCabove,
        Self::CmdCaddbuffer,
        Self::CmdCaddexpr,
        Self::CmdCaddfile,
        Self::CmdCafter,
        Self::CmdCall,
        Self::CmdCatch,
        Self::CmdCbuffer,
        Self::CmdCbefore,
        Self::CmdCbelow,
        Self::CmdCbottom,
        Self::CmdCc,
        Self::CmdCclose,
        Self::CmdCd,
        Self::CmdCdo,
        Self::CmdCenter,
        Self::CmdCexpr,
        Self::CmdCfile,
        Self::CmdCfdo,
        Self::CmdCfirst,
        Self::CmdCgetfile,
        Self::CmdCgetbuffer,
        Self::CmdCgetexpr,
        Self::CmdChdir,
        Self::CmdChanges,
        Self::CmdCheckhealth,
        Self::CmdCheckpath,
        Self::CmdChecktime,
        Self::CmdChistory,
        Self::CmdClist,
        Self::CmdClast,
        Self::CmdClose,
        Self::CmdClearjumps,
        Self::CmdCmap,
        Self::CmdCmapclear,
        Self::CmdCmenu,
        Self::CmdCnextVariant,
        Self::CmdCnewer,
        Self::CmdCnfileVariant,
        Self::CmdCnoremap,
        Self::CmdCnoreabbrev,
        Self::CmdCnoremenu,
        Self::CmdCopy,
        Self::CmdColder,
        Self::CmdColorscheme,
        Self::CmdCommand,
        Self::CmdComclear,
        Self::CmdCompiler,
        Self::CmdContinue,
        Self::CmdConfirm,
        Self::CmdConnect,
        Self::CmdConst,
        Self::CmdCopen,
        Self::CmdCprevious,
        Self::CmdCpfile,
        Self::CmdCquit,
        Self::CmdCrewind,
        Self::CmdCunmap,
        Self::CmdCunabbrev,
        Self::CmdCunmenu,
        Self::CmdCwindow,
        Self::CmdDelete,
        Self::CmdDelmarks,
        Self::CmdDebug,
        Self::CmdDebuggreedy,
        Self::CmdDefer,
        Self::CmdDelcommand,
        Self::CmdDelfunction,
        Self::CmdDetach,
        Self::CmdDisplay,
        Self::CmdDiffupdate,
        Self::CmdDiffget,
        Self::CmdDiffoff,
        Self::CmdDiffpatch,
        Self::CmdDiffput,
        Self::CmdDiffsplit,
        Self::CmdDiffthis,
        Self::CmdDigraphs,
        Self::CmdDjump,
        Self::CmdDlist,
        Self::CmdDoautocmd,
        Self::CmdDoautoall,
        Self::CmdDrop,
        Self::CmdDsearch,
        Self::CmdDsplit,
        Self::CmdEdit,
        Self::CmdEarlier,
        Self::CmdEcho,
        Self::CmdEchoerr,
        Self::CmdEchohl,
        Self::CmdEchomsg,
        Self::CmdEchon,
        Self::CmdElse,
        Self::CmdElseif,
        Self::CmdEmenu,
        Self::CmdEndif,
        Self::CmdEndfunction,
        Self::CmdEndfor,
        Self::CmdEndtry,
        Self::CmdEndwhile,
        Self::CmdEnew,
        Self::CmdEval,
        Self::CmdEx,
        Self::CmdExecute,
        Self::CmdExit,
        Self::CmdExmode,
        Self::CmdExusage,
        Self::CmdFile,
        Self::CmdFiles,
        Self::CmdFiletype,
        Self::CmdFilter,
        Self::CmdFind,
        Self::CmdFinally,
        Self::CmdFinish,
        Self::CmdFirst,
        Self::CmdFold,
        Self::CmdFoldclose,
        Self::CmdFolddoopen,
        Self::CmdFolddoclosed,
        Self::CmdFoldopen,
        Self::CmdFor,
        Self::CmdFunction,
        Self::CmdFclose,
        Self::CmdGlobal,
        Self::CmdGoto,
        Self::CmdGrep,
        Self::CmdGrepadd,
        Self::CmdGui,
        Self::CmdGvim,
        Self::CmdHelp,
        Self::CmdHelpclose,
        Self::CmdHelpgrep,
        Self::CmdHelptags,
        Self::CmdHighlight,
        Self::CmdHide,
        Self::CmdHistory,
        Self::CmdHorizontal,
        Self::CmdInsert,
        Self::CmdIabbrev,
        Self::CmdIabclear,
        Self::CmdIf,
        Self::CmdIjump,
        Self::CmdIlist,
        Self::CmdImap,
        Self::CmdImapclear,
        Self::CmdImenu,
        Self::CmdInoremap,
        Self::CmdInoreabbrev,
        Self::CmdInoremenu,
        Self::CmdIntro,
        Self::CmdIput,
        Self::CmdIsearch,
        Self::CmdIsplit,
        Self::CmdIunmap,
        Self::CmdIunabbrev,
        Self::CmdIunmenu,
        Self::CmdJoin,
        Self::CmdJumps,
        Self::CmdK,
        Self::CmdKeepmarks,
        Self::CmdKeepjumps,
        Self::CmdKeeppatterns,
        Self::CmdKeepalt,
        Self::CmdList,
        Self::CmdLnext,
        Self::CmdLnfile,
        Self::CmdLast,
        Self::CmdLabove,
        Self::CmdLanguage,
        Self::CmdLaddexpr,
        Self::CmdLaddbuffer,
        Self::CmdLaddfile,
        Self::CmdLafter,
        Self::CmdLater,
        Self::CmdLbuffer,
        Self::CmdLbefore,
        Self::CmdLbelow,
        Self::CmdLbottom,
        Self::CmdLcd,
        Self::CmdLchdir,
        Self::CmdLclose,
        Self::CmdLdo,
        Self::CmdLeft,
        Self::CmdLeftabove,
        Self::CmdLet,
        Self::CmdLexpr,
        Self::CmdLfile,
        Self::CmdLfdo,
        Self::CmdLfirst,
        Self::CmdLgetfile,
        Self::CmdLgetbuffer,
        Self::CmdLgetexpr,
        Self::CmdLgrep,
        Self::CmdLgrepadd,
        Self::CmdLhelpgrep,
        Self::CmdLhistory,
        Self::CmdLl,
        Self::CmdLlast,
        Self::CmdLlist,
        Self::CmdLmap,
        Self::CmdLmapclear,
        Self::CmdLmake,
        Self::CmdLnoremap,
        Self::CmdLnextVariant,
        Self::CmdLnewer,
        Self::CmdLnfileVariant,
        Self::CmdLoadview,
        Self::CmdLoadkeymap,
        Self::CmdLockmarks,
        Self::CmdLockvar,
        Self::CmdLog,
        Self::CmdLolder,
        Self::CmdLopen,
        Self::CmdLprevious,
        Self::CmdLpfile,
        Self::CmdLrewind,
        Self::CmdLtag,
        Self::CmdLunmap,
        Self::CmdLua,
        Self::CmdLuado,
        Self::CmdLuafile,
        Self::CmdLvimgrep,
        Self::CmdLvimgrepadd,
        Self::CmdLwindow,
        Self::CmdLs,
        Self::CmdLsp,
        Self::CmdMove,
        Self::CmdMark,
        Self::CmdMake,
        Self::CmdMap,
        Self::CmdMapclear,
        Self::CmdMarks,
        Self::CmdMatch,
        Self::CmdMenu,
        Self::CmdMenutranslate,
        Self::CmdMessages,
        Self::CmdMkexrc,
        Self::CmdMksession,
        Self::CmdMkspell,
        Self::CmdMkvimrc,
        Self::CmdMkview,
        Self::CmdMode,
        Self::CmdMzscheme,
        Self::CmdMzfile,
        Self::CmdNext,
        Self::CmdNew,
        Self::CmdNmap,
        Self::CmdNmapclear,
        Self::CmdNmenu,
        Self::CmdNnoremap,
        Self::CmdNnoremenu,
        Self::CmdNoremap,
        Self::CmdNoautocmd,
        Self::CmdNohlsearch,
        Self::CmdNoreabbrev,
        Self::CmdNoremenu,
        Self::CmdNoswapfile,
        Self::CmdNormal,
        Self::CmdNumber,
        Self::CmdNunmap,
        Self::CmdNunmenu,
        Self::CmdOldfiles,
        Self::CmdOmap,
        Self::CmdOmapclear,
        Self::CmdOmenu,
        Self::CmdOnly,
        Self::CmdOnoremap,
        Self::CmdOnoremenu,
        Self::CmdOptions,
        Self::CmdOunmap,
        Self::CmdOunmenu,
        Self::CmdOwnsyntax,
        Self::CmdPrint,
        Self::CmdPackadd,
        Self::CmdPackdel,
        Self::CmdPackloadall,
        Self::CmdPackupdate,
        Self::CmdPbuffer,
        Self::CmdPclose,
        Self::CmdPerl,
        Self::CmdPerldo,
        Self::CmdPerlfile,
        Self::CmdPedit,
        Self::CmdPop,
        Self::CmdPopup,
        Self::CmdPpop,
        Self::CmdPreserve,
        Self::CmdPrevious,
        Self::CmdProfile,
        Self::CmdProfdel,
        Self::CmdPsearch,
        Self::CmdPtag,
        Self::CmdPtnext,
        Self::CmdPtfirst,
        Self::CmdPtjump,
        Self::CmdPtlast,
        Self::CmdPtnextVariant,
        Self::CmdPtprevious,
        Self::CmdPtrewind,
        Self::CmdPtselect,
        Self::CmdPut,
        Self::CmdPwd,
        Self::CmdPython,
        Self::CmdPydo,
        Self::CmdPyfile,
        Self::CmdPy3,
        Self::CmdPy3do,
        Self::CmdPython3,
        Self::CmdPy3file,
        Self::CmdPyx,
        Self::CmdPyxdo,
        Self::CmdPythonx,
        Self::CmdPyxfile,
        Self::CmdQuit,
        Self::CmdQuitall,
        Self::CmdQall,
        Self::CmdRead,
        Self::CmdRecover,
        Self::CmdRedo,
        Self::CmdRedir,
        Self::CmdRedraw,
        Self::CmdRedrawstatus,
        Self::CmdRedrawtabline,
        Self::CmdRegisters,
        Self::CmdResize,
        Self::CmdRestart,
        Self::CmdRetab,
        Self::CmdReturn,
        Self::CmdRewind,
        Self::CmdRight,
        Self::CmdRightbelow,
        Self::CmdRshada,
        Self::CmdRuntime,
        Self::CmdRundo,
        Self::CmdRuby,
        Self::CmdRubydo,
        Self::CmdRubyfile,
        Self::CmdRviminfo,
        Self::CmdSubstitute,
        Self::CmdSnext,
        Self::CmdSargument,
        Self::CmdSall,
        Self::CmdSandbox,
        Self::CmdSaveas,
        Self::CmdSbuffer,
        Self::CmdSbnext,
        Self::CmdSball,
        Self::CmdSbfirst,
        Self::CmdSblast,
        Self::CmdSbmodified,
        Self::CmdSbnextVariant,
        Self::CmdSbprevious,
        Self::CmdSbrewind,
        Self::CmdScriptnames,
        Self::CmdScriptencoding,
        Self::CmdSet,
        Self::CmdSetfiletype,
        Self::CmdSetglobal,
        Self::CmdSetlocal,
        Self::CmdSfind,
        Self::CmdSfirst,
        Self::CmdSimalt,
        Self::CmdSign,
        Self::CmdSilent,
        Self::CmdSleep,
        Self::CmdSlast,
        Self::CmdSmagic,
        Self::CmdSmap,
        Self::CmdSmapclear,
        Self::CmdSmenu,
        Self::CmdSnextVariant,
        Self::CmdSnomagic,
        Self::CmdSnoremap,
        Self::CmdSnoremenu,
        Self::CmdSource,
        Self::CmdSort,
        Self::CmdSplit,
        Self::CmdSpellgood,
        Self::CmdSpelldump,
        Self::CmdSpellinfo,
        Self::CmdSpellrepall,
        Self::CmdSpellrare,
        Self::CmdSpellundo,
        Self::CmdSpellwrong,
        Self::CmdSprevious,
        Self::CmdSrewind,
        Self::CmdStop,
        Self::CmdStag,
        Self::CmdStartinsert,
        Self::CmdStartgreplace,
        Self::CmdStartreplace,
        Self::CmdStopinsert,
        Self::CmdStjump,
        Self::CmdStselect,
        Self::CmdSunhide,
        Self::CmdSunmap,
        Self::CmdSunmenu,
        Self::CmdSuspend,
        Self::CmdSview,
        Self::CmdSwapname,
        Self::CmdSyntax,
        Self::CmdSyntime,
        Self::CmdSyncbind,
        Self::CmdT,
        Self::CmdTcd,
        Self::CmdTchdir,
        Self::CmdTnext,
        Self::CmdTag,
        Self::CmdTags,
        Self::CmdTab,
        Self::CmdTabclose,
        Self::CmdTabdo,
        Self::CmdTabedit,
        Self::CmdTabfind,
        Self::CmdTabfirst,
        Self::CmdTabmove,
        Self::CmdTablast,
        Self::CmdTabnextVariant,
        Self::CmdTabnew,
        Self::CmdTabonly,
        Self::CmdTabprevious,
        Self::CmdTabnext,
        Self::CmdTabrewind,
        Self::CmdTabs,
        Self::CmdTcl,
        Self::CmdTcldo,
        Self::CmdTclfile,
        Self::CmdTerminal,
        Self::CmdTfirst,
        Self::CmdThrow,
        Self::CmdTjump,
        Self::CmdTlast,
        Self::CmdTlmenu,
        Self::CmdTlnoremenu,
        Self::CmdTlunmenu,
        Self::CmdTmenu,
        Self::CmdTmap,
        Self::CmdTmapclear,
        Self::CmdTnextVariant,
        Self::CmdTnoremap,
        Self::CmdTopleft,
        Self::CmdTprevious,
        Self::CmdTrewind,
        Self::CmdTrust,
        Self::CmdTry,
        Self::CmdTselect,
        Self::CmdTunmenu,
        Self::CmdTunmap,
        Self::CmdUndo,
        Self::CmdUndojoin,
        Self::CmdUndolist,
        Self::CmdUnabbreviate,
        Self::CmdUnhide,
        Self::CmdUniq,
        Self::CmdUnlet,
        Self::CmdUnlockvar,
        Self::CmdUnmap,
        Self::CmdUnmenu,
        Self::CmdUnsilent,
        Self::CmdUpdate,
        Self::CmdUptime,
        Self::CmdVglobal,
        Self::CmdVersion,
        Self::CmdVerbose,
        Self::CmdVertical,
        Self::CmdVisual,
        Self::CmdView,
        Self::CmdVimgrep,
        Self::CmdVimgrepadd,
        Self::CmdViusage,
        Self::CmdVmap,
        Self::CmdVmapclear,
        Self::CmdVmenu,
        Self::CmdVnoremap,
        Self::CmdVnew,
        Self::CmdVnoremenu,
        Self::CmdVsplit,
        Self::CmdVunmap,
        Self::CmdVunmenu,
        Self::CmdWrite,
        Self::CmdWnext,
        Self::CmdWall,
        Self::CmdWhile,
        Self::CmdWinsize,
        Self::CmdWincmd,
        Self::CmdWindo,
        Self::CmdWinpos,
        Self::CmdWnextVariant,
        Self::CmdWprevious,
        Self::CmdWq,
        Self::CmdWqall,
        Self::CmdWshada,
        Self::CmdWundo,
        Self::CmdWviminfo,
        Self::CmdXit,
        Self::CmdXall,
        Self::CmdXmap,
        Self::CmdXmapclear,
        Self::CmdXmenu,
        Self::CmdXnoremap,
        Self::CmdXnoremenu,
        Self::CmdXunmap,
        Self::CmdXunmenu,
        Self::CmdYank,
        Self::CmdZ,
        Self::CmdBang,
        Self::CmdPound,
        Self::CmdAnd,
        Self::CmdLshift,
        Self::CmdEqual,
        Self::CmdRshift,
        Self::CmdAt,
        Self::CmdTilde,
        Self::CmdCapNext,
    ];

    #[inline]
    pub const fn is_user(self) -> bool {
        (self as i32) < 0
    }

    #[inline]
    pub const fn is_builtin(self) -> bool {
        (self as i32) >= 0 && (self as i32) < (Self::CmdSize as i32)
    }

    #[inline]
    pub const fn from_raw(idx: i32) -> Self {
        match idx {
            -1 => Self::CmdUser,
            -2 => Self::CmdUserBuf,
            i if i >= 0 && (i as usize) < Self::ALL.len() => Self::ALL[i as usize],
            _ => Self::CmdSize,
        }
    }
}

use crate::change::TAB;
#[cfg(unix)]
use crate::ex_getln::VSE_SHELL;
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::os::fs::MAXPATHL;
use crate::search::{BACKWARD, FORWARD};

#[derive(Clone, Copy)]
pub struct CmdlineColorChunk {
    pub start: i32,
    pub end: i32,
    pub hl_id: i32,
}

pub type CmdlineColors = Vec<CmdlineColorChunk>;

#[derive(Clone)]
pub struct ColoredCmdline {
    pub prompt_id: u32,
    pub cmdbuff: Option<std::ffi::CString>,
    pub colors: CmdlineColors,
}

impl ColoredCmdline {
    pub const fn new() -> Self {
        Self {
            prompt_id: 0,
            cmdbuff: None,
            colors: Vec::new(),
        }
    }
}

pub struct CmdlineInfo {
    pub cmdbuff_vec: std::cell::RefCell<Vec<u8>>,
    pub cmdbufflen: std::cell::Cell<i32>,
    pub cmdlen: std::cell::Cell<i32>,
    pub cmdpos: std::cell::Cell<i32>,
    pub cmdspos: std::cell::Cell<i32>,
    pub cmdfirstc: std::cell::Cell<i32>,
    pub cmdindent: std::cell::Cell<i32>,
    pub cmdprompt: std::cell::RefCell<Option<CString>>,
    pub hl_id: std::cell::Cell<i32>,
    pub overstrike: std::cell::Cell<i32>,
    pub xpc: std::cell::RefCell<Option<ExpandCtx>>,
    pub compl_orig: std::cell::RefCell<Option<CString>>,
    pub compl_is_fuzzy: std::cell::Cell<bool>,
    pub xp_context: std::cell::Cell<i32>,
    pub xp_arg: std::cell::RefCell<Option<CString>>,
    pub input_fn: std::cell::Cell<i32>,
    pub cmdbuff_replaced: std::cell::Cell<bool>,
    pub prompt_id: std::cell::Cell<u32>,
    pub highlight_callback: std::cell::RefCell<Callback>,
    pub last_colors: std::cell::RefCell<ColoredCmdline>,
    pub level: std::cell::Cell<i32>,
    pub prev_ccline: std::cell::RefCell<Option<std::rc::Rc<CmdlineInfo>>>,
    pub special_char: std::cell::Cell<u8>,
    pub special_shift: std::cell::Cell<bool>,
    pub redraw_state: std::cell::Cell<i32>,
    pub one_key: std::cell::Cell<bool>,
    pub mouse_used: std::cell::Cell<Option<bool>>,
}

impl CmdlineInfo {
    pub const fn new() -> Self {
        Self {
            cmdbuff_vec: std::cell::RefCell::new(Vec::new()),
            cmdbufflen: std::cell::Cell::new(0),
            cmdlen: std::cell::Cell::new(0),
            cmdpos: std::cell::Cell::new(0),
            cmdspos: std::cell::Cell::new(0),
            cmdfirstc: std::cell::Cell::new(0),
            cmdindent: std::cell::Cell::new(0),
            cmdprompt: std::cell::RefCell::new(None),
            hl_id: std::cell::Cell::new(0),
            overstrike: std::cell::Cell::new(0),
            xpc: std::cell::RefCell::new(None),
            compl_orig: std::cell::RefCell::new(None),
            compl_is_fuzzy: std::cell::Cell::new(false),
            xp_context: std::cell::Cell::new(0),
            xp_arg: std::cell::RefCell::new(None),
            input_fn: std::cell::Cell::new(0),
            cmdbuff_replaced: std::cell::Cell::new(false),
            prompt_id: std::cell::Cell::new(0),
            highlight_callback: std::cell::RefCell::new(Callback::NONE),
            last_colors: std::cell::RefCell::new(ColoredCmdline::new()),
            level: std::cell::Cell::new(0),
            prev_ccline: std::cell::RefCell::new(None),
            special_char: std::cell::Cell::new(0),
            special_shift: std::cell::Cell::new(false),
            redraw_state: std::cell::Cell::new(0),
            one_key: std::cell::Cell::new(false),
            mouse_used: std::cell::Cell::new(None),
        }
    }

    #[inline]
    pub fn cmdbuff(&self) -> crate::strings::OwnedSlice {
        let vec = self.cmdbuff_vec.borrow();
        if vec.is_empty() {
            crate::strings::OwnedSlice::NULL
        } else {
            crate::strings::OwnedSlice::from_bytes(&vec)
        }
    }
}

pub(crate) const CTRL_N: i32 = 14;
pub(crate) const CTRL_P: i32 = 16;

pub(crate) const MB_MAXBYTES: usize = 30;

#[cfg(windows)]
const ENV_SEPCHAR: i32 = b';' as i32;
#[cfg(not(windows))]
const ENV_SEPCHAR: i32 = b':' as i32;

const PATHSEPSTR: &[u8] = b"/";

pub const WILD_FREE: i32 = 1;
pub const WILD_EXPAND_FREE: i32 = 2;
pub const WILD_EXPAND_KEEP: i32 = 3;
pub const WILD_NEXT: i32 = 4;
pub const WILD_PREV: i32 = 5;
pub const WILD_ALL: i32 = 6;
pub const WILD_LONGEST: i32 = 7;
pub const WILD_ALL_KEEP: i32 = 8;
pub const WILD_CANCEL: i32 = 9;
pub const WILD_APPLY: i32 = 10;
pub const WILD_PAGEUP: i32 = 11;
pub const WILD_PAGEDOWN: i32 = 12;
pub const WILD_PUM_WANT: i32 = 13;

pub const WILD_LIST_NOTFOUND: i32 = 0x01;
pub const WILD_HOME_REPLACE: i32 = 0x02;
pub const WILD_USE_NL: i32 = 0x04;
pub const WILD_NO_BEEP: i32 = 0x08;
pub const WILD_ADD_SLASH: i32 = 0x10;
pub const WILD_KEEP_ALL: i32 = 0x20;
pub const WILD_SILENT: i32 = 0x40;
pub const WILD_ESCAPE: i32 = 0x80;
pub const WILD_ICASE: i32 = 0x100;
pub const WILD_ALLLINKS: i32 = 0x200;
pub const WILD_USE_COMPLETESLASH: i32 = 0x400;
pub const WILD_NOERROR: i32 = 0x800;
pub const WILD_BUFLASTUSED: i32 = 0x1000;
pub const BUF_DIFF_FILTER: i32 = 0x2000;
pub const WILD_NOSELECT: i32 = 0x4000;
pub const WILD_MAY_EXPAND_PATTERN: i32 = 0x8000;
pub const WILD_FUNC_TRIGGER: i32 = 0x10000;
pub const WILD_NOINSERT: i32 = 0x20000;
pub const WILD_USE_SHELLSLASH: i32 = 0x40000;

pub const XP_PREFIX_NONE: XpPrefix = 0;
pub const XP_PREFIX_NO: XpPrefix = 1;
pub const XP_PREFIX_INV: XpPrefix = 2;

pub const XP_BS_NONE: i32 = 0;
pub const XP_BS_ONE: i32 = 0x1;
pub const XP_BS_THREE: i32 = 0x2;
pub const XP_BS_COMMA: i32 = 0x4;

pub const EXPAND_UNSUCCESSFUL: i32 = -2;
pub const EXPAND_OK: i32 = -1;
pub const EXPAND_NOTHING: i32 = 0;
pub const EXPAND_COMMANDS: i32 = 1;
pub const EXPAND_FILES: i32 = 2;
pub const EXPAND_DIRECTORIES: i32 = 3;
pub const EXPAND_SETTINGS: i32 = 4;
pub const EXPAND_BOOL_SETTINGS: i32 = 5;
pub const EXPAND_TAGS: i32 = 6;
pub const EXPAND_OLD_SETTING: i32 = 7;
pub const EXPAND_HELP: i32 = 8;
pub const EXPAND_BUFFERS: i32 = 9;
pub const EXPAND_EVENTS: i32 = 10;
pub const EXPAND_MENUS: i32 = 11;
pub const EXPAND_SYNTAX: i32 = 12;
pub const EXPAND_HIGHLIGHT: i32 = 13;
pub const EXPAND_AUGROUP: i32 = 14;
pub const EXPAND_USER_VARS: i32 = 15;
pub const EXPAND_MAPPINGS: i32 = 16;
pub const EXPAND_TAGS_LISTFILES: i32 = 17;
pub const EXPAND_FUNCTIONS: i32 = 18;
pub const EXPAND_USER_FUNC: i32 = 19;
pub const EXPAND_EXPRESSION: i32 = 20;
pub const EXPAND_MENUNAMES: i32 = 21;
pub const EXPAND_USER_COMMANDS: i32 = 22;
pub const EXPAND_USER_CMD_FLAGS: i32 = 23;
pub const EXPAND_USER_NARGS: i32 = 24;
pub const EXPAND_USER_COMPLETE: i32 = 25;
pub const EXPAND_ENV_VARS: i32 = 26;
pub const EXPAND_LANGUAGE: i32 = 27;
pub const EXPAND_COLORS: i32 = 28;
pub const EXPAND_COMPILER: i32 = 29;
pub const EXPAND_USER_DEFINED: i32 = 30;
pub const EXPAND_USER_LIST: i32 = 31;
pub const EXPAND_USER_LUA: i32 = 32;
pub const EXPAND_SHELLCMD: i32 = 33;
pub const EXPAND_SIGN: i32 = 34;
pub const EXPAND_PROFILE: i32 = 35;
pub const EXPAND_FILETYPE: i32 = 36;
pub const EXPAND_FILES_IN_PATH: i32 = 37;
pub const EXPAND_OWNSYNTAX: i32 = 38;
pub const EXPAND_LOCALES: i32 = 39;
pub const EXPAND_HISTORY: i32 = 40;
pub const EXPAND_USER: i32 = 41;
pub const EXPAND_SYNTIME: i32 = 42;
pub const EXPAND_USER_ADDR_TYPE: i32 = 43;
pub const EXPAND_PACKADD: i32 = 44;
pub const EXPAND_MESSAGES: i32 = 45;
pub const EXPAND_MAPCLEAR: i32 = 46;
pub const EXPAND_ARGLIST: i32 = 47;
pub const EXPAND_DIFF_BUFFERS: i32 = 48;
pub const EXPAND_BREAKPOINT: i32 = 49;
pub const EXPAND_SCRIPTNAMES: i32 = 50;
pub const EXPAND_RUNTIME: i32 = 51;
pub const EXPAND_STRING_SETTING: i32 = 52;
pub const EXPAND_SETTING_SUBTRACT: i32 = 53;
pub const EXPAND_ARGOPT: i32 = 54;
pub const EXPAND_KEYMAP: i32 = 55;
pub const EXPAND_DIRS_IN_CDPATH: i32 = 56;
pub const EXPAND_SHELLCMDLINE: i32 = 57;
pub const EXPAND_FINDFUNC: i32 = 58;
pub const EXPAND_FILETYPECMD: i32 = 59;
pub const EXPAND_PATTERN_IN_BUF: i32 = 60;
pub const EXPAND_RETAB: i32 = 61;
pub const EXPAND_CHECKHEALTH: i32 = 62;
pub const EXPAND_LUA: i32 = 63;
pub const EXPAND_LSP: i32 = 64;
pub const EXPAND_LOG: i32 = 65;
pub const EXPAND_PACKDEL: i32 = 66;
pub const EXPAND_PACKUPDATE: i32 = 67;
pub const EXPAND_MARKS: i32 = 68;

pub(crate) const EX_NOTRLCOM: u32 = 0x800;

const K_OPT_WOP_FLAG_FUZZY: u32 = 0x01;
const K_OPT_WOP_FLAG_TAGFILE: u32 = 0x02;
const K_OPT_WOP_FLAG_PUM: u32 = 0x04;
const K_OPT_WOP_FLAG_EXACTTEXT: u32 = 0x08;

use crate::ex_getln::{K_OPT_WIM_FLAG_NOINSERT, K_OPT_WIM_FLAG_NOSELECT};

pub(crate) const K_OPT_BO_FLAG_WILDMODE: u32 = 0x80000;

pub(crate) const K_UICMDLINE: UIExtension = 0;
pub(crate) const K_UIPOPUPMENU_CONST: UIExtension = 1;
pub(crate) const K_UIWILDMENU_CONST: UIExtension = 3;

const WM_SHOWN: i32 = 1;
const WM_SCROLLED: i32 = 2;

pub(crate) const HLF_D: Hlf = 5;
const HLF_WM: Hlf = 27;

pub(crate) const RE_LAST: i32 = 2;

pub(crate) const SEARCH_NFMSG: i32 = 0x08;
pub(crate) const SEARCH_OPT: i32 = 0x10;
pub(crate) const SEARCH_NOOF: i32 = 0x80;
pub(crate) const SEARCH_START: i32 = 0x100;
pub(crate) const SEARCH_PEEK: i32 = 0x800;

pub(crate) const TAG_MANY: i32 = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FiletypeExpandWhat {
    All,
    Plugin,
    Indent,
    OnOff,
}

const EXPAND_FILETYPECMD_PLUGIN: i32 = 0x01;
const EXPAND_FILETYPECMD_INDENT: i32 = 0x02;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BreakptExpandWhat {
    Add,
    Del,
    Profdel,
}

struct CmdexpandState {
    cmd_showtail: std::cell::Cell<bool>,
    may_expand_pattern: std::cell::Cell<bool>,
    pre_incsearch_pos: std::cell::Cell<CursorPos>,
    compl_startcol: std::cell::Cell<i32>,
    compl_selected: std::cell::Cell<i32>,
    filetype_expand_what: std::cell::Cell<FiletypeExpandWhat>,
    breakpt_expand_what: std::cell::Cell<BreakptExpandWhat>,
    healthcheck_names: crate::api::private::validate::CloneCell<Object>,
    healthcheck_last_gen: std::cell::Cell<u32>,
    arg1_lua_names: crate::api::private::validate::CloneCell<Object>,
    arg1_lua_last_gen: std::cell::Cell<u32>,
    first_match: std::cell::Cell<i32>,
}

impl CmdexpandState {
    const fn new() -> Self {
        Self {
            cmd_showtail: std::cell::Cell::new(false),
            may_expand_pattern: std::cell::Cell::new(false),
            pre_incsearch_pos: std::cell::Cell::new(CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            }),
            compl_startcol: std::cell::Cell::new(0),
            compl_selected: std::cell::Cell::new(0),
            filetype_expand_what: std::cell::Cell::new(FiletypeExpandWhat::All),
            breakpt_expand_what: std::cell::Cell::new(BreakptExpandWhat::Add),
            healthcheck_names: crate::api::private::validate::CloneCell::new(Object {
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Integer(0),
            }),
            healthcheck_last_gen: std::cell::Cell::new(0),
            arg1_lua_names: crate::api::private::validate::CloneCell::new(Object {
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Integer(0),
            }),
            arg1_lua_last_gen: std::cell::Cell::new(0),
            first_match: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static CMDEXP_STATE: CmdexpandState = const { CmdexpandState::new() };
    static COMPL_MATCH_ARRAY: std::cell::RefCell<std::rc::Rc<[Pumitem]>> = {
        std::cell::RefCell::new(std::rc::Rc::from([]))
    };
    static CMDLINE_ORIG: std::cell::RefCell<Option<Vec<u8>>> = const {
        std::cell::RefCell::new(None)
    };
    static ARG1_LUA_LAST_XP_LINE: std::cell::RefCell<Option<CString>> = const {
        std::cell::RefCell::new(None)
    };
}

use crate::mbyte::utfc_slice2len;
use crate::path::vim_ispathsep;

#[inline]
fn show_match(matches: &[CString], m: usize, showtail: bool) -> &CStr {
    let item = matches[m].as_c_str();
    if showtail {
        showmatches_gettail(item, false)
    } else {
        item
    }
}

/// Returns true if fuzzy completion is supported for a given cmdline completion context.
fn cmdline_fuzzy_completion_supported(xp: &ExpandCtx) -> bool {
    match xp.xp_context {
        EXPAND_BOOL_SETTINGS
        | EXPAND_COLORS
        | EXPAND_COMPILER
        | EXPAND_DIRECTORIES
        | EXPAND_DIRS_IN_CDPATH
        | EXPAND_FILES
        | EXPAND_FILES_IN_PATH
        | EXPAND_FILETYPE
        | EXPAND_FILETYPECMD
        | EXPAND_FINDFUNC
        | EXPAND_HELP
        | EXPAND_KEYMAP
        | EXPAND_LUA
        | EXPAND_OLD_SETTING
        | EXPAND_STRING_SETTING
        | EXPAND_SETTING_SUBTRACT
        | EXPAND_OWNSYNTAX
        | EXPAND_PACKADD
        | EXPAND_RUNTIME
        | EXPAND_SHELLCMD
        | EXPAND_SHELLCMDLINE
        | EXPAND_TAGS
        | EXPAND_TAGS_LISTFILES
        | EXPAND_USER_LIST
        | EXPAND_USER_LUA => false,
        _ => (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_FUZZY) != 0,
    }
}

/// Returns true if fuzzy completion for cmdline completion is enabled and
/// "fuzzystr" is not empty.
pub fn cmdline_fuzzy_complete(fuzzystr: &CStr) -> bool {
    (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_FUZZY) != 0 && !fuzzystr.to_bytes().is_empty()
}

/// Sort function for the completion matches.
/// <SNR> functions should be sorted to the end.
fn sort_func_compare(p1: &CString, p2: &CString) -> i32 {
    let b1 = p1.as_bytes().first().copied().unwrap_or(0);
    let b2 = p2.as_bytes().first().copied().unwrap_or(0);
    if b1 != b'<' && b2 == b'<' {
        return -1;
    }
    if b1 == b'<' && b2 != b'<' {
        return 1;
    }
    p1.to_bytes().cmp(p2.to_bytes()) as i32
}

/// Escape special characters in the cmdline completion matches.
fn wildescape(xp: &mut ExpandCtx, str_arg: &[u8], files: &mut [CString]) {
    let str_arg = crate::memory::bytes_until_nul(str_arg);
    let vse_what = if xp.xp_context == EXPAND_BUFFERS {
        VSE_BUFFER
    } else {
        VSE_NONE
    };

    if xp.xp_context == EXPAND_FILES
        || xp.xp_context == EXPAND_FILES_IN_PATH
        || xp.xp_context == EXPAND_SHELLCMD
        || xp.xp_context == EXPAND_BUFFERS
        || xp.xp_context == EXPAND_DIRECTORIES
        || xp.xp_context == EXPAND_DIRS_IN_CDPATH
    {
        for file_slot in files.iter_mut() {
            if (xp.xp_backslash & XP_BS_THREE) != 0 {
                let pat: &[u8] = if (xp.xp_backslash & XP_BS_COMMA) != 0 {
                    b" ,"
                } else {
                    b" "
                };
                *file_slot = vim_strsave_escaped(file_slot.as_bytes(), pat);
                #[cfg(windows)]
                {
                    *file_slot = vim_strsave_escaped(file_slot.as_bytes(), b" ");
                }
            } else if (xp.xp_backslash & XP_BS_COMMA) != 0 && file_slot.as_bytes().contains(&b',') {
                *file_slot = vim_strsave_escaped(file_slot.as_bytes(), b",");
            }

            #[cfg(windows)]
            {
                *file_slot = vim_strsave_fnameescape(file_slot.to_bytes(), vse_what);
            }
            #[cfg(not(windows))]
            {
                *file_slot = vim_strsave_fnameescape(
                    file_slot.to_bytes(),
                    if xp.xp_shell { VSE_SHELL } else { vse_what },
                );
            }

            if str_arg.starts_with(b"\\~") && file_slot.as_bytes().first() == Some(&b'~') {
                escape_fname_cstr(file_slot);
            }
        }
        xp.xp_backslash = XP_BS_NONE;

        if let Some(first) = files.first_mut()
            && first.as_bytes().first() == Some(&b'+')
        {
            escape_fname_cstr(first);
        }
    } else if xp.xp_context == EXPAND_TAGS {
        for file_slot in files.iter_mut() {
            *file_slot = vim_strsave_escaped(file_slot.as_bytes(), b"\\|\"");
        }
    }
}

/// Escape special characters in the cmdline completion matches.
fn expand_escape(xp: &mut ExpandCtx, str_arg: &[u8], files: &mut [CString], options: i32) {
    let str_arg = crate::memory::bytes_until_nul(str_arg);
    if (options & WILD_HOME_REPLACE) != 0 && !files.is_empty() {
        tilde_replace(str_arg, files);
    }

    if (options & WILD_ESCAPE) != 0 {
        wildescape(xp, str_arg, files);
    }
}

/// Return FAIL if this is not an appropriate context in which to do
/// completion of anything, return OK if it is (even if there are no matches).
pub fn nextwild(xp: &mut ExpandCtx, type_arg: i32, options: i32, escape: bool) -> bool {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;
    let mut p: Option<CString>;
    let from_wildtrigger_func = (options & WILD_FUNC_TRIGGER) != 0;
    let wild_navigate = type_arg == WILD_NEXT
        || type_arg == WILD_PREV
        || type_arg == WILD_PAGEUP
        || type_arg == WILD_PAGEDOWN
        || type_arg == WILD_PUM_WANT;

    if xp.xp_numfiles == -1 {
        CMDEXP_STATE.with(|s| s.pre_incsearch_pos.set(xp.xp_pre_incsearch_pos));
        if ccline.input_fn.get() != 0 && ccline.xp_context.get() == EXPAND_COMMANDS {
            set_cmd_context(
                xp,
                &ccline.cmdbuff_vec.borrow(),
                ccline.cmdlen.get(),
                ccline.cmdpos.get(),
                false,
            );
        } else {
            CMDEXP_STATE.with(|s| {
                s.may_expand_pattern
                    .set((options & WILD_MAY_EXPAND_PATTERN) != 0)
            });
            set_expand_context(xp);
            CMDEXP_STATE.with(|s| s.may_expand_pattern.set(false));
        }
        if xp.xp_context == EXPAND_LUA {
            nlua_expand_pat(xp);
        }
        CMDEXP_STATE.with(|s| s.cmd_showtail.set(expand_showtail(xp)));
    }

    if xp.xp_context == EXPAND_UNSUCCESSFUL {
        beep_flush();
        return true;
    }
    if xp.xp_context == EXPAND_NOTHING {
        return false;
    }

    let i = ((xp.xp_pattern).offset_from(ccline.cmdbuff()) as isize) as i32;
    debug_assert!(ccline.cmdpos.get() >= i);
    xp.xp_pattern_len = (ccline.cmdpos.get() as usize) - (i as usize);

    if from_wildtrigger_func && xp.xp_context == EXPAND_COMMANDS && xp.xp_pattern_len == 0 {
        return false;
    }

    if !g_st().cmd_silent.get()
        && !from_wildtrigger_func
        && !wild_navigate
        && !(ui_has(K_UICMDLINE) || ui_has(K_UIWILDMENU_CONST))
    {
        msg_puts(c"...");
        ui_flush();
    }

    if wild_navigate {
        p = expand_one(xp, None, None, 0, type_arg);
    } else {
        let tmp =
            if cmdline_fuzzy_completion_supported(xp) || xp.xp_context == EXPAND_PATTERN_IN_BUF {
                bytes_to_cstring(xp.xp_pattern.slice(xp.xp_pattern_len))
            } else {
                addstar(xp.xp_pattern.slice(xp.xp_pattern_len), xp.xp_context)
            };
        let use_options = options
            | WILD_USE_COMPLETESLASH
            | WILD_HOME_REPLACE
            | WILD_ADD_SLASH
            | WILD_SILENT
            | if escape { WILD_ESCAPE } else { 0 }
            | if opt_vars().p_wic.get() {
                WILD_ICASE
            } else {
                0
            };
        let orig =
            bytes_to_cstring((ccline.cmdbuff().wrapping_add(i as usize)).slice(xp.xp_pattern_len));
        p = expand_one(xp, Some(tmp.as_c_str()), Some(orig), use_options, type_arg);
        if let Some(ref p_str) = p
            && type_arg == WILD_LONGEST
        {
            let mut j: usize = 0;
            while j < xp.xp_pattern_len {
                let c = ccline.cmdbuff_vec.borrow()[(i as usize) + j];
                if c == b'*' || c == b'?' {
                    break;
                }
                j += 1;
            }
            if p_str.to_bytes().len() < j {
                p = None;
            }
        }
    }

    if !wild_navigate && !ccline.cmdbuff().is_null() {
        let cmdbuff = ccline.cmdbuff();
        let slice = cmdbuff.slice(ccline.cmdlen.get() as usize);
        let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        CMDLINE_ORIG.with(|cell| *cell.borrow_mut() = Some(slice[..len].to_vec()));
    }

    if let Some(ref p_str) = p
        && !g_st().got_int.get()
        && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0
    {
        let plen = p_str.to_bytes().len();
        let difflen = (plen as i32) - (xp.xp_pattern_len as i32);
        if ccline.cmdlen.get() + difflen + 4 > ccline.cmdbufflen.get() {
            realloc_cmdbuff(ccline.cmdlen.get() + difflen + 4);
        }
        debug_assert!(ccline.cmdpos.get() <= ccline.cmdlen.get());

        ccline.cmdbuff_vec.borrow_mut().copy_within(
            ccline.cmdpos.get() as usize..=ccline.cmdlen.get() as usize,
            (ccline.cmdpos.get() + difflen) as usize,
        );
        ccline.cmdbuff_vec.borrow_mut()[i as usize..i as usize + plen]
            .copy_from_slice(&p_str.to_bytes_with_nul()[..plen]);
        ccline.cmdlen.set(ccline.cmdlen.get() + (difflen));
        ccline.cmdpos.set(ccline.cmdpos.get() + (difflen));
        xp.xp_pattern = ccline.cmdbuff().wrapping_add(i as usize);
    }

    redrawcmd();
    cursorcmd();

    if xp.xp_context == EXPAND_MAPPINGS && p.is_none() {
        return false;
    }

    if xp.xp_numfiles <= 0 && p.is_none() {
        beep_flush();
    } else if xp.xp_numfiles == 1
        && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0
        && !wild_navigate
    {
        expand_one(xp, None, None, 0, WILD_FREE);
    }

    true
}

/// Create completion popup menu with items from "matches".
fn cmdline_pum_create(
    ccline: &CmdlineInfo,
    xp: &ExpandCtx,
    matches: &[CString],
    showtail: bool,
    cmdline_unchanged: bool,
) {
    let mut items = Vec::with_capacity(matches.len());
    for i in 0..matches.len() {
        let pum_text = xp
            .xp_files_abbr
            .as_slice()
            .get(i)
            .and_then(|opt| opt.clone())
            .or_else(|| Some(show_match(matches, i, showtail).to_owned()));
        let pum_info = xp
            .xp_files_info
            .as_slice()
            .get(i)
            .and_then(|opt| opt.clone());
        let pum_extra = xp
            .xp_files_menu
            .as_slice()
            .get(i)
            .and_then(|opt| opt.clone());
        let pum_kind = xp
            .xp_files_kind
            .as_slice()
            .get(i)
            .and_then(|opt| opt.clone());
        items.push(Pumitem {
            pum_text,
            pum_kind,
            pum_extra,
            pum_info,
            pum_user_abbr_hlattr: -1,
            pum_user_kind_hlattr: -1,
            pum_comp_idx: None,
        });
    }
    COMPL_MATCH_ARRAY.with(|cell| *cell.borrow_mut() = items.into());
    *ccline.compl_orig.borrow_mut() = xp.xp_orig.clone();
    ccline
        .compl_is_fuzzy
        .set(cmdline_fuzzy_completion_supported(xp));

    let pat_cstr = xp.xp_pattern.as_cstr();
    let tail_off = if showtail {
        (showmatches_gettail(pat_cstr, cmdline_unchanged).as_ptr() as usize)
            .saturating_sub(pat_cstr.as_ptr() as usize)
    } else {
        0
    };
    let endpos_addr = xp.xp_pattern.addr().wrapping_add(tail_off);
    let cmdbuff_addr = ccline.cmdbuff().addr();
    if ui_has(K_UICMDLINE) && g_st().cmdline_win.get().is_null() {
        CMDEXP_STATE.with(|s| {
            s.compl_startcol
                .set(((endpos_addr as isize) - (cmdbuff_addr as isize)) as i32)
        });
    } else {
        CMDEXP_STATE.with(|s| {
            s.compl_startcol.set(cmd_screencol(
                ((endpos_addr as isize) - (cmdbuff_addr as isize)) as i32,
            ))
        });
    }
}

pub fn cmdline_pum_display(changed_array: bool) {
    let arr = COMPL_MATCH_ARRAY.with(|cell| cell.borrow().clone());
    pum_display(
        arr,
        CMDEXP_STATE.with(|s| s.compl_selected.get()),
        changed_array,
        CMDEXP_STATE.with(|s| s.compl_startcol.get()),
    );
}

/// Returns true if the cmdline completion popup menu is being displayed.
pub fn cmdline_pum_active() -> bool {
    pum_visible() && COMPL_MATCH_ARRAY.with(|cell| !cell.borrow().is_empty())
}

/// Remove the cmdline completion popup menu (if present), free the list of items.
pub fn cmdline_pum_remove(defer_redraw: bool) {
    pum_undisplay(!defer_redraw);
    COMPL_MATCH_ARRAY.with(|cell| *cell.borrow_mut() = std::rc::Rc::from([]));
}

pub fn cmdline_pum_cleanup(cclp: &CmdlineInfo) {
    cmdline_pum_remove(false);
    wildmenu_cleanup(cclp);
}

/// Returns the current cmdline completion pattern.
pub fn cmdline_compl_pattern() -> Option<CString> {
    let ccline = get_cmdline_info();
    if let Ok(xpc) = ccline.xpc.try_borrow() {
        return xpc.as_ref()?.xp_orig.clone();
    }
    ccline.compl_orig.borrow().clone()
}

/// Returns true if fuzzy cmdline completion is active, false otherwise.
pub fn cmdline_compl_is_fuzzy() -> bool {
    let ccline = get_cmdline_info();
    if let Ok(xpc) = ccline.xpc.try_borrow() {
        return xpc.as_ref().is_some_and(cmdline_fuzzy_completion_supported);
    }
    ccline.compl_is_fuzzy.get()
}

/// Checks whether popup menu should be used for cmdline completion wildmenu.
fn cmdline_compl_use_pum(need_wildmenu: bool) -> bool {
    (need_wildmenu
        && (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_PUM) != 0
        && !(ui_has(K_UICMDLINE) && g_st().cmdline_win.get().is_null()))
        || ui_has(K_UIWILDMENU_CONST)
        || (ui_has(K_UICMDLINE) && ui_has(K_UIPOPUPMENU_CONST))
}

/// Return the number of characters that should be skipped in the wildmenu.
fn skip_wildmenu_char(xp: &ExpandCtx, s_bytes: &[u8]) -> usize {
    let s_bytes = crate::memory::bytes_until_nul(s_bytes);
    if (rem_backslash(s_bytes)
        && xp.xp_context != EXPAND_HELP
        && xp.xp_context != EXPAND_PATTERN_IN_BUF)
        || ((xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES)
            && (s_bytes.first() == Some(&b'\t')
                || (s_bytes.first() == Some(&b'\\') && s_bytes.len() > 1)))
    {
        #[cfg(not(windows))]
        if xp.xp_shell
            && csh_like_shell()
            && s_bytes.get(1) == Some(&b'\\')
            && s_bytes.get(2) == Some(&b'!')
        {
            return 2;
        }
        return 1;
    }
    0
}

/// Get the length of an item as it will be shown in the status line.
fn wildmenu_match_len(xp: &ExpandCtx, s: &CStr) -> i32 {
    let mut len: i32 = 0;
    let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;

    if emenu && menu_is_separator(s.to_bytes()) {
        return 1;
    }

    let mut rem = s.to_bytes();
    while !rem.is_empty() {
        let skip = skip_wildmenu_char(xp, rem).min(rem.len());
        rem = &rem[skip..];
        if rem.is_empty() {
            break;
        }
        len += crate::mbyte::slice2cells(rem);
        let step = utfc_slice2len(rem).max(1).min(rem.len());
        rem = &rem[step..];
    }

    len
}

/// Show wildchar matches in the status line.
fn redraw_wildmenu(xp: &ExpandCtx, matches: &[CString], mut match_idx: i32, showtail: bool) {
    let mut highlight = true;
    let mut selstart_col: i32 = 0;
    let mut add_left = false;

    if matches.is_empty() {
        return;
    }
    let num_matches = matches.len() as i32;

    let mut buf_vec = vec![0u8; (g_st().columns_global.get() as usize) * MB_MAXBYTES + 1];

    if match_idx == -1 {
        match_idx = 0;
        highlight = false;
    }
    let mut clen = wildmenu_match_len(xp, show_match(matches, match_idx as usize, showtail)) + 3;
    if match_idx == 0 {
        CMDEXP_STATE.with(|s| s.first_match.set(0));
    } else if match_idx < CMDEXP_STATE.with(|s| s.first_match.get()) {
        CMDEXP_STATE.with(|s| s.first_match.set(match_idx));
        add_left = true;
    } else {
        for i in CMDEXP_STATE.with(|s| s.first_match.get())..match_idx {
            clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
        }
        if CMDEXP_STATE.with(|s| s.first_match.get()) > 0 {
            clen += 2;
        }
        if clen > g_st().columns_global.get() {
            CMDEXP_STATE.with(|s| s.first_match.set(match_idx));
            clen = 2;
            let mut i = match_idx;
            while i < num_matches {
                clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
                if clen >= g_st().columns_global.get() {
                    break;
                }
                i += 1;
            }
            if i == num_matches {
                add_left = true;
            }
        }
    }
    if add_left {
        while CMDEXP_STATE.with(|s| s.first_match.get()) > 0 {
            clen += wildmenu_match_len(
                xp,
                show_match(
                    matches,
                    (CMDEXP_STATE.with(|s| s.first_match.get()) - 1) as usize,
                    showtail,
                ),
            ) + 2;
            if clen >= g_st().columns_global.get() {
                break;
            }
            CMDEXP_STATE.with(|s| s.first_match.set(s.first_match.get() - (1)));
        }
    }

    let mut group: Hlf = 0;
    let fillchar = fillchar_status(&mut group, &curwin());
    let attr = win_hl_attr(&curwin(), group);

    let mut len: usize;
    if CMDEXP_STATE.with(|s| s.first_match.get()) == 0 {
        buf_vec[0] = 0;
        len = 0;
    } else {
        buf_vec[..3].copy_from_slice(b"< \0");
        len = 2;
    }
    clen = len as i32;

    let mut selstart_idx: Option<usize> = None;
    let mut selend_idx: usize = 0;

    let mut i = CMDEXP_STATE.with(|s| s.first_match.get());
    while clen + wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2
        < g_st().columns_global.get()
    {
        if i == match_idx {
            selstart_idx = Some(len);
            selstart_col = clen;
        }

        let s = show_match(matches, i as usize, showtail);
        let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;
        if emenu && menu_is_separator(s.to_bytes()) {
            let tc_buf = transchar(b'|' as i32);
            let tc = tc_buf.to_bytes();
            buf_vec[len..len + tc.len()].copy_from_slice(tc);
            len += tc.len();
            clen += tc.len() as i32;
        } else {
            let mut rem = s.to_bytes();
            while !rem.is_empty() {
                let skip = skip_wildmenu_char(xp, rem).min(rem.len());
                rem = &rem[skip..];
                if rem.is_empty() {
                    break;
                }
                clen += crate::mbyte::slice2cells(rem);
                let l = utfc_slice2len(rem).max(1).min(rem.len());
                if l > 1 {
                    buf_vec[len..len + l].copy_from_slice(&rem[..l]);
                    len += l;
                } else {
                    let tc_buf = transchar_byte(rem[0]);
                    let tc = tc_buf.to_bytes();
                    buf_vec[len..len + tc.len()].copy_from_slice(tc);
                    len += tc.len();
                }
                rem = &rem[l..];
            }
        }
        if i == match_idx {
            selend_idx = len;
        }

        buf_vec[len] = b' ';
        len += 1;
        buf_vec[len] = b' ';
        len += 1;
        clen += 2;
        i += 1;
        if i == num_matches {
            break;
        }
    }

    if i != num_matches {
        buf_vec[len] = b'>';
        len += 1;
        clen += 1;
    }

    buf_vec[len] = 0;

    let mut row = g_st().cmdline_row.get() - 1;
    if row >= 0 {
        if g_st().wild_menu_showing.get() == 0 {
            if g_st().msg_scrolled.get() > 0 {
                if g_st().cmdline_row.get() == g_st().rows_global.get() - 1 {
                    msg_scroll_up(false, false);
                    g_st().msg_scrolled.set(g_st().msg_scrolled.get() + (1));
                } else {
                    g_st().cmdline_row.set(g_st().cmdline_row.get() + (1));
                    row += 1;
                }
                g_st().wild_menu_showing.set(WM_SCROLLED);
            } else {
                if lastwin().w_status_height.get() == 0 && global_stl_height() == 0 {
                    g_st().save_p_ls.set(opt_vars().p_ls.get() as i32);
                    g_st().save_p_wmh.set(opt_vars().p_wmh.get() as i32);
                    opt_vars().p_ls.set(2);
                    opt_vars().p_wmh.set(0);
                    last_status(false);
                }
                g_st().wild_menu_showing.set(WM_SHOWN);
            }
        }

        grid_line_start(
            if g_st().wild_menu_showing.get() == WM_SCROLLED {
                &g_st().msg_grid_adj
            } else {
                &g_st().default_gridview
            },
            row,
        );

        grid_line_puts(0, crate::memory::bytes_as_cstr(&buf_vec), -1, attr);
        if let Some(start_idx) = selstart_idx
            && highlight
        {
            buf_vec[selend_idx] = 0;
            grid_line_puts(
                selstart_col,
                crate::memory::bytes_as_cstr(&buf_vec[start_idx..]),
                -1,
                hl_attr(HLF_WM),
            );
        }

        grid_line_fill(clen, g_st().columns_global.get(), fillchar, attr);

        grid_line_flush();
    }

    win_redraw_last_status(&topframe());
}

/// Get the next or prev cmdline completion match.
fn get_next_or_prev_match(mode: i32, xp: &mut ExpandCtx) -> Option<CString> {
    if xp.xp_numfiles <= 0 {
        return None;
    }

    let mut findex = xp.xp_selected;

    if mode == WILD_PREV {
        if findex == -1 {
            findex = xp.xp_numfiles;
        }
        findex -= 1;
    } else if mode == WILD_NEXT {
        findex += 1;
    } else if mode == WILD_PAGEUP || mode == WILD_PAGEDOWN {
        let mut ht = pum_get_height();
        if ht > 3 {
            ht -= 2;
        }

        if mode == WILD_PAGEUP {
            if findex == 0 {
                findex = -1;
            } else if findex < 0 {
                findex = xp.xp_numfiles - 1;
            } else {
                findex = (findex - ht).max(0);
            }
        } else if findex == xp.xp_numfiles - 1 {
            findex = -1;
        } else if findex < 0 {
            findex = 0;
        } else {
            findex = (findex + ht).min(xp.xp_numfiles - 1);
        }
    } else {
        debug_assert!(g_st().pum_want.active.get());
        findex = g_st().pum_want.item.get();
    }

    if findex < 0 || findex >= xp.xp_numfiles {
        if xp.xp_orig.is_some() {
            findex = -1;
        } else {
            findex = if findex < 0 { xp.xp_numfiles - 1 } else { 0 };
        }
    }

    if opt_vars().p_wmnu.get() {
        let ccline = get_cmdline_info();
        *ccline.compl_orig.borrow_mut() = xp.xp_orig.clone();
        ccline
            .compl_is_fuzzy
            .set(cmdline_fuzzy_completion_supported(xp));
        let has_arr = COMPL_MATCH_ARRAY.with(|cell| !cell.borrow().is_empty());
        if has_arr {
            CMDEXP_STATE.with(|s| s.compl_selected.set(findex));
            cmdline_pum_display(false);
        } else if cmdline_compl_use_pum(true) {
            cmdline_pum_create(
                &ccline,
                xp,
                &xp.xp_files.as_slice()[..xp.xp_numfiles as usize],
                CMDEXP_STATE.with(|s| s.cmd_showtail.get()),
                false,
            );
            CMDEXP_STATE.with(|s| s.compl_selected.set(findex));
            pum_clear();
            cmdline_pum_display(true);
        } else {
            redraw_wildmenu(
                xp,
                &xp.xp_files.as_slice()[..xp.xp_numfiles as usize],
                findex,
                CMDEXP_STATE.with(|s| s.cmd_showtail.get()),
            );
        }
    }

    xp.xp_selected = findex;
    if findex == -1 {
        xp.xp_orig.clone()
    } else {
        Some(xp.xp_files.as_slice()[findex as usize].clone())
    }
}

/// Start the command-line expansion and get the matches.
fn expand_one_start(
    mode: i32,
    xp: &mut ExpandCtx,
    str_arg: &CStr,
    options: i32,
) -> Option<CString> {
    let mut ss: Option<CString> = None;

    let mut xp_files = std::mem::take(&mut xp.xp_files);
    let expand_res = expand_from_context(xp, str_arg, &mut xp_files, options);
    xp.xp_files = xp_files;
    if !expand_res {
        xp.xp_numfiles = 0;
        #[cfg(windows)]
        if (options & WILD_SILENT) == 0 && (options & WILD_LIST_NOTFOUND) != 0 {
            semsg!(gettext(E_NOMATCH2), str_arg);
        }
    } else {
        xp.xp_numfiles = xp.xp_files.len() as i32;
        if xp.xp_numfiles == 0 {
            if (options & WILD_SILENT) == 0 {
                semsg!(gettext(E_NOMATCH2), str_arg);
            }
        } else {
            let nfiles = xp.xp_numfiles as usize;
            let mut xp_files = std::mem::take(&mut xp.xp_files);
            expand_escape(
                xp,
                str_arg.to_bytes(),
                &mut xp_files.as_mut_slice()[..nfiles],
                options,
            );
            xp.xp_files = xp_files;

            if mode != WILD_ALL && mode != WILD_ALL_KEEP && mode != WILD_LONGEST {
                let mut non_suf_match = if xp.xp_numfiles != 0 {
                    xp.xp_numfiles
                } else {
                    1
                };
                if (xp.xp_context == EXPAND_FILES || xp.xp_context == EXPAND_DIRECTORIES)
                    && xp.xp_numfiles > 1
                {
                    non_suf_match = 0;
                    for i in 0..2 {
                        if match_suffix(xp.xp_files.as_slice()[i].as_c_str()) {
                            non_suf_match += 1;
                        }
                    }
                }
                if non_suf_match != 1 {
                    if (options & WILD_SILENT) == 0 {
                        emsg(gettext(E_TOOMANY));
                    } else if (options & WILD_NO_BEEP) == 0 {
                        beep_flush();
                    }
                }
                if !(non_suf_match != 1 && mode == WILD_EXPAND_FREE) {
                    ss = Some(xp.xp_files.as_slice()[0].clone());
                }
            }
        }
    }

    ss
}

/// Return the longest common part in the list of cmdline completion matches.
fn find_longest_match(xp: &mut ExpandCtx, options: i32) -> CString {
    let mut len: usize = 0;
    let files = xp.xp_files.as_slice();
    let first_bytes = files[0].as_bytes();

    while len < first_bytes.len() {
        let mb_len = utfc_ptr2len(&first_bytes[len..]) as usize;
        let c0 = utf_ptr2char(&first_bytes[len..]);
        let mut i: usize = 1;
        while (i as i32) < xp.xp_numfiles {
            let ci_bytes = files[i].as_bytes();
            let ci = utf_ptr2char(ci_bytes.get(len..).unwrap_or(&[]));

            if opt_vars().p_fic.get()
                && (xp.xp_context == EXPAND_DIRECTORIES
                    || xp.xp_context == EXPAND_FILES
                    || xp.xp_context == EXPAND_SHELLCMD
                    || xp.xp_context == EXPAND_BUFFERS)
            {
                if mb_tolower(c0) != mb_tolower(ci) {
                    break;
                }
            } else if c0 != ci {
                break;
            }
            i += 1;
        }
        if (i as i32) < xp.xp_numfiles {
            if (options & WILD_NO_BEEP) == 0 {
                vim_beep(K_OPT_BO_FLAG_WILDMODE);
            }
            break;
        }
        len += mb_len;
    }

    bytes_to_cstring(&first_bytes[..len.min(first_bytes.len())])
}

pub fn free_xp_files_extra(xp: &mut ExpandCtx) {
    xp.xp_files_abbr.clear();
    xp.xp_files_kind.clear();
    xp.xp_files_menu.clear();
    xp.xp_files_info.clear();
}

/// Do wildcard expansion on the string "str".
pub fn expand_one(
    xp: &mut ExpandCtx,
    str_arg: Option<&CStr>,
    orig: Option<CString>,
    options: i32,
    mode: i32,
) -> Option<CString> {
    let mut ss: Option<CString> = None;

    if mode == WILD_NEXT
        || mode == WILD_PREV
        || mode == WILD_PAGEUP
        || mode == WILD_PAGEDOWN
        || mode == WILD_PUM_WANT
    {
        return get_next_or_prev_match(mode, xp);
    }

    if mode == WILD_CANCEL {
        ss = Some(xp.xp_orig.clone().unwrap_or_default());
    } else if mode == WILD_APPLY {
        ss = Some(if xp.xp_selected == -1 {
            xp.xp_orig.clone().unwrap_or_default()
        } else {
            xp.xp_files.as_slice()[xp.xp_selected as usize].clone()
        });
    }

    if xp.xp_numfiles != -1 && mode != WILD_ALL && mode != WILD_LONGEST {
        xp.xp_files.clear();
        if mode != WILD_FREE && mode != WILD_CANCEL && mode != WILD_APPLY {
            xp.xp_files = Vec::new();
        }
        free_xp_files_extra(xp);
        xp.xp_numfiles = -1;
        xp.xp_orig = None;

        if COMPL_MATCH_ARRAY.with(|cell| !cell.borrow().is_empty()) {
            cmdline_pum_remove(false);
        }
    }
    xp.xp_selected = if (options & WILD_NOSELECT) != 0 {
        -1
    } else {
        0
    };

    if mode == WILD_FREE {
        return None;
    }

    if xp.xp_numfiles == -1 && mode != WILD_APPLY && mode != WILD_CANCEL {
        xp.xp_orig = orig;

        ss = expand_one_start(mode, xp, str_arg.unwrap_or(c""), options);
    }

    if mode == WILD_LONGEST && xp.xp_numfiles > 0 {
        ss = Some(find_longest_match(xp, options));
        xp.xp_selected = -1;
    }

    if mode == WILD_ALL && xp.xp_numfiles > 0 && !g_st().got_int.get() {
        let prefix: &[u8] = if xp.xp_prefix == XP_PREFIX_NO {
            b"no"
        } else if xp.xp_prefix == XP_PREFIX_INV {
            b"inv"
        } else {
            b""
        };
        let suffix: &[u8] = if (options & WILD_USE_NL) != 0 {
            b"\n"
        } else {
            b" "
        };
        let n = xp.xp_numfiles - 1;

        let files = xp.xp_files.as_slice();
        let mut out = Vec::new();
        for (idx, f) in files.iter().take(xp.xp_numfiles as usize).enumerate() {
            let i = idx as i32;
            if i > 0 {
                out.extend_from_slice(prefix);
            }
            out.extend_from_slice(f.as_bytes());
            if i < n {
                out.extend_from_slice(suffix);
            }
        }
        ss = Some(bytes_to_cstring(&out));
    }

    if mode == WILD_EXPAND_FREE || mode == WILD_ALL {
        expand_cleanup(xp);
    }

    ss
}

/// Prepare an expand structure for use.
pub fn expand_init(xp: &mut ExpandCtx) {
    *xp = ExpandCtx::new();
}

/// Cleanup an expand structure after use.
pub fn expand_cleanup(xp: &mut ExpandCtx) {
    if xp.xp_numfiles >= 0 {
        free_xp_files_extra(xp);
        xp.xp_numfiles = -1;
    }
    xp.xp_files = Vec::new();
    xp.xp_orig = None;
}

pub fn clear_cmdline_orig() {
    CMDLINE_ORIG.with(|cell| *cell.borrow_mut() = None);
}

/// Display one line of completion matches.
fn showmatches_oneline(
    xp: &ExpandCtx,
    matches: &[CString],
    lines: i32,
    linenr: i32,
    maxlen: i32,
    showtail: bool,
) {
    let num_matches = matches.len() as i32;
    let mut lastlen: i32 = 999;
    let mut j = linenr;
    while j < num_matches {
        if xp.xp_context == EXPAND_TAGS_LISTFILES {
            msg_outtrans(&matches[j as usize], HLF_D, false);
            if let Some((kind_str, fname_str)) = crate::tag::tag_listfiles_extra(j as usize) {
                msg_advance(maxlen + 1);
                msg_puts(&kind_str);
                msg_advance(maxlen + 3);
                msg_outtrans_long(&fname_str, HLF_D);
            }
            break;
        }
        let mut i = maxlen - lastlen;
        while {
            i -= 1;
            i >= 0
        } {
            msg_putchar(b' ' as i32);
        }
        let isdir: bool;
        let nb_owned: std::ffi::CString;
        let p_cstr: &CStr;
        if xp.xp_context == EXPAND_FILES
            || xp.xp_context == EXPAND_SHELLCMD
            || xp.xp_context == EXPAND_BUFFERS
        {
            let match_j = matches[j as usize].as_c_str();
            if xp.xp_numfiles != -1 {
                let exp_path = expand_env_save_opt(&matches[j as usize], true, None);
                let halved_slash = backslash_halve_save(exp_path.as_bytes());
                isdir = os_isdir(&halved_slash);
            } else {
                isdir = os_isdir(match_j);
            }
            if showtail {
                p_cstr = show_match(matches, j as usize, showtail);
            } else {
                let mut name_buff = [0u8; MAXPATHL];
                home_replace(None, Some(match_j), &mut name_buff, true);
                nb_owned = crate::memory::cstring_until_nul(&name_buff);
                p_cstr = &nb_owned;
            }
        } else {
            isdir = false;
            p_cstr = show_match(matches, j as usize, showtail);
        }
        lastlen = msg_outtrans(p_cstr, if isdir { HLF_D } else { 0 }, false);
        j += lines;
    }
    if g_st().msg_col.get() > 0 {
        msg_clr_eos();
        msg_putchar(b'\n' as i32);
    }
}

#[inline]
fn show_pum_matches(
    ccline: &CmdlineInfo,
    xp: &ExpandCtx,
    matches: &[CString],
    showtail: bool,
    noselect: bool,
    cmdline_unchanged: bool,
) {
    cmdline_pum_create(ccline, xp, matches, showtail, cmdline_unchanged);
    CMDEXP_STATE.with(|s| s.compl_selected.set(if noselect { -1 } else { 0 }));
    pum_clear();
    cmdline_pum_display(true);
}

/// Display completion matches.
pub fn showmatches(
    xp: &mut ExpandCtx,
    display_wildmenu: bool,
    display_list: bool,
    wim_flags_arg: i32,
) -> i32 {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;
    let mut temp_matches: Vec<CString> = Vec::new();
    let showtail: bool;
    let noselect = (wim_flags_arg & i32::from(K_OPT_WIM_FLAG_NOSELECT)) != 0;
    let noinsert = (wim_flags_arg & i32::from(K_OPT_WIM_FLAG_NOINSERT)) != 0;
    let cmdline_unchanged = noselect || noinsert;

    let matches: &[CString] = if xp.xp_numfiles == -1 {
        set_expand_context(xp);
        if xp.xp_context == EXPAND_LUA {
            nlua_expand_pat(xp);
        }
        let retval = expand_cmdline(
            xp,
            ccline.cmdbuff().addr(),
            ccline.cmdpos.get(),
            &mut temp_matches,
        );
        if retval != EXPAND_OK {
            return retval;
        }
        showtail = expand_showtail(xp);
        &temp_matches
    } else {
        showtail = CMDEXP_STATE.with(|s| s.cmd_showtail.get());
        &xp.xp_files.as_slice()[..xp.xp_numfiles as usize]
    };

    if cmdline_compl_use_pum(display_wildmenu && !display_list) {
        show_pum_matches(ccline, xp, matches, showtail, noselect, cmdline_unchanged);
        if xp.xp_numfiles == -1 {
            free_xp_files_extra(xp);
        }
        return EXPAND_OK;
    }

    if display_list {
        g_st().msg_didany.set(false);
        msg_start();
        if !ui_has(K_UIMESSAGES) {
            msg_putchar(b'\n' as i32);
        }
        ui_flush();
        g_st().cmdline_row.set(g_st().msg_row.get());
        g_st().msg_didany.set(false);
        msg_ext_set_kind(c"wildlist");
        msg_start();
    }

    if g_st().got_int.get() {
        g_st().got_int.set(false);
    } else if display_wildmenu && !display_list {
        redraw_wildmenu(xp, matches, if noselect { -1 } else { 0 }, showtail);
    } else if display_list {
        let num_matches = matches.len() as i32;
        let mut maxlen: i32 = 0;
        for (i, match_i) in matches.iter().enumerate() {
            let len = if !showtail
                && (xp.xp_context == EXPAND_FILES
                    || xp.xp_context == EXPAND_SHELLCMD
                    || xp.xp_context == EXPAND_BUFFERS)
            {
                let mut name_buff = [0u8; MAXPATHL];
                home_replace(None, Some(match_i.as_c_str()), &mut name_buff, true);
                vim_strsize(crate::memory::bytes_as_cstr(&name_buff))
            } else {
                vim_strsize(show_match(matches, i, showtail))
            };
            maxlen = maxlen.max(len);
        }

        let lines = if xp.xp_context == EXPAND_TAGS_LISTFILES {
            num_matches
        } else {
            maxlen += 2;
            let mut columns = (g_st().columns_global.get() + 2) / maxlen;
            if columns < 1 {
                columns = 1;
            }
            (num_matches + columns - 1) / columns
        };

        if xp.xp_context == EXPAND_TAGS_LISTFILES {
            msg_puts_hl(gettext(c"tagname"), HLF_T, false);
            msg_clr_eos();
            msg_advance(maxlen - 3);
            msg_puts_hl(gettext(c" kind file\n"), HLF_T, false);
        }

        for i in 0..lines {
            showmatches_oneline(xp, matches, lines, i, maxlen, showtail);
            if g_st().got_int.get() {
                g_st().got_int.set(false);
                break;
            }
        }

        g_st().cmdline_row.set(g_st().msg_row.get());
    }

    if display_wildmenu && display_list {
        if cmdline_compl_use_pum(true) {
            show_pum_matches(ccline, xp, matches, showtail, noselect, cmdline_unchanged);
        } else {
            redraw_wildmenu(xp, matches, if noselect { -1 } else { 0 }, showtail);
        }
    }

    if xp.xp_numfiles == -1 {
        free_xp_files_extra(xp);
    }

    EXPAND_OK
}

/// path_tail() version for showmatches() and redraw_wildmenu().
fn showmatches_gettail(s: &CStr, eager: bool) -> &CStr {
    let bytes = s.to_bytes();
    let mut t = 0usize;
    let mut had_sep = false;
    let mut p = 0usize;
    while p < bytes.len() {
        #[cfg(windows)]
        let is_sep = vim_ispathsep(i32::from(bytes[p])) && !rem_backslash(&bytes[p..]);
        #[cfg(not(windows))]
        let is_sep = vim_ispathsep(i32::from(bytes[p]));

        if is_sep {
            if eager {
                t = p + 1;
            } else {
                had_sep = true;
            }
        } else if had_sep {
            t = p;
            had_sep = false;
        }
        p += utfc_slice2len(&bytes[p..]).max(1);
    }
    CStr::from_bytes_until_nul(&s.to_bytes_with_nul()[t..]).unwrap_or(c"")
}

/// Return true if we only need to show the tail of completion matches.
fn expand_showtail(xp: &mut ExpandCtx) -> bool {
    if xp.xp_context != EXPAND_FILES
        && xp.xp_context != EXPAND_SHELLCMD
        && xp.xp_context != EXPAND_DIRECTORIES
    {
        return false;
    }

    let pat_cstr = xp.xp_pattern.as_cstr();
    let end = path_tail(pat_cstr).as_ptr();
    if end == pat_cstr.as_ptr() {
        return false;
    }

    let pat_bytes = pat_cstr.to_bytes();
    let end_off = (end as usize).saturating_sub(pat_cstr.as_ptr() as usize);
    let mut idx = 0usize;
    while idx < end_off && idx < pat_bytes.len() {
        if rem_backslash(&pat_bytes[idx..]) {
            idx += 1;
        } else if crate::strings::vim_haschr(c"*?[", i32::from(pat_bytes[idx])) {
            return false;
        }
        idx += 1;
    }
    true
}

/// Prepare a string for expansion.
pub fn addstar(fname_slice: &[u8], context: i32) -> CString {
    let fname_slice = crate::memory::bytes_until_nul(fname_slice);
    let len = fname_slice.len();
    {
        if context != EXPAND_FILES
            && context != EXPAND_FILES_IN_PATH
            && context != EXPAND_SHELLCMD
            && context != EXPAND_DIRECTORIES
            && context != EXPAND_DIRS_IN_CDPATH
        {
            if context == EXPAND_FINDFUNC
                || context == EXPAND_HELP
                || context == EXPAND_COLORS
                || context == EXPAND_COMPILER
                || context == EXPAND_OWNSYNTAX
                || context == EXPAND_FILETYPE
                || context == EXPAND_KEYMAP
                || context == EXPAND_PACKADD
                || context == EXPAND_RUNTIME
                || ((context == EXPAND_TAGS_LISTFILES || context == EXPAND_TAGS)
                    && fname_slice.first() == Some(&b'/'))
                || context == EXPAND_CHECKHEALTH
                || context == EXPAND_LSP
                || context == EXPAND_LOG
                || context == EXPAND_PACKDEL
                || context == EXPAND_PACKUPDATE
                || context == EXPAND_MARKS
                || context == EXPAND_LUA
            {
                bytes_to_cstring(fname_slice)
            } else {
                let mut new_len = len + 2;
                for &ch in fname_slice {
                    if ch == b'*' || ch == b'~' {
                        new_len += 1;
                    }
                    if context == EXPAND_BUFFERS && ch == b'.' {
                        new_len += 1;
                    }
                    if (context == EXPAND_USER_DEFINED || context == EXPAND_USER_LIST)
                        && ch == b'\\'
                    {
                        new_len += 1;
                    }
                }
                let mut retval = vec![0u8; new_len];
                retval[0] = b'^';
                let mut j: usize = 1;
                let mut i: usize = 0;
                while i < len {
                    if context != EXPAND_USER_DEFINED
                        && context != EXPAND_USER_LIST
                        && fname_slice[i] == b'\\'
                    {
                        i += 1;
                        if i == len {
                            break;
                        }
                    }

                    match fname_slice[i] {
                        b'*' => {
                            retval[j] = b'.';
                            j += 1;
                        }
                        b'~' => {
                            retval[j] = b'\\';
                            j += 1;
                        }
                        b'?' => {
                            retval[j] = b'.';
                            i += 1;
                            j += 1;
                            continue;
                        }
                        b'.' if context == EXPAND_BUFFERS => {
                            retval[j] = b'\\';
                            j += 1;
                        }
                        b'\\' if context == EXPAND_USER_DEFINED || context == EXPAND_USER_LIST => {
                            retval[j] = b'\\';
                            j += 1;
                        }
                        _ => {}
                    }
                    retval[j] = fname_slice[i];
                    i += 1;
                    j += 1;
                }
                bytes_to_cstring(&retval[..j])
            }
        } else {
            let mut len = len;
            let mut retval_vec = vec![0u8; len + 4];
            retval_vec[..len].copy_from_slice(fname_slice);
            let retval_cstr = crate::memory::bytes_as_cstr(&retval_vec);
            let tail_cstr = path_tail(retval_cstr);
            let tail_is_start = tail_cstr.as_ptr() == retval_cstr.as_ptr();
            #[cfg(windows)]
            let ends_in_star = len > 0 && retval_vec[len - 1] == b'*';
            #[cfg(not(windows))]
            let mut ends_in_star = len > 0 && retval_vec[len - 1] == b'*';
            #[cfg(not(windows))]
            {
                let mut k = (len as isize) - 2;
                while k >= 0 {
                    if retval_vec[k as usize] != b'\\' {
                        break;
                    }
                    ends_in_star = !ends_in_star;
                    k -= 1;
                }
            }
            if (retval_vec[0] != b'~' || !tail_is_start)
                && !ends_in_star
                && !crate::strings::vim_haschr(tail_cstr, b'$' as i32)
                && !crate::strings::vim_haschr(retval_cstr, b'`' as i32)
            {
                retval_vec[len] = b'*';
                len += 1;
            } else if len > 0 && retval_vec[len - 1] == b'$' {
                len -= 1;
            }
            retval_vec[len] = 0;
            bytes_to_cstring(&retval_vec[..len])
        }
    }
}

/// Must parse the command line so far to work out what context we are in.
pub fn set_expand_context(xp: &mut ExpandCtx) {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;

    if (ccline.cmdfirstc.get() == b'/' as i32 || ccline.cmdfirstc.get() == b'?' as i32)
        && CMDEXP_STATE.with(|s| s.may_expand_pattern.get())
    {
        xp.xp_context = EXPAND_PATTERN_IN_BUF;
        xp.xp_search_dir = if ccline.cmdfirstc.get() == b'/' as i32 {
            FORWARD
        } else {
            BACKWARD
        };
        xp.xp_pattern = ccline.cmdbuff();
        xp.xp_pattern_len = ccline.cmdpos.get() as usize;
        g_st().search_global.first_line.set(0);
        return;
    }

    if ccline.cmdfirstc.get() != b':' as i32
        && ccline.cmdfirstc.get() != b'>' as i32
        && ccline.cmdfirstc.get() != b'=' as i32
        && ccline.input_fn.get() == 0
    {
        xp.xp_context = EXPAND_NOTHING;
        return;
    }

    set_cmd_context(
        xp,
        &ccline.cmdbuff_vec.borrow(),
        ccline.cmdlen.get(),
        ccline.cmdpos.get(),
        true,
    );
}

/// Sets the index of a built-in or user defined command "cmd" in eap->cmdidx.
fn set_cmd_index<'a>(
    cmd: &'a CStr,
    eap: &mut Exarg,
    xp: &mut ExpandCtx,
    complp: &mut i32,
) -> Option<&'a CStr> {
    let cmd_with_nul = cmd.to_bytes_with_nul();
    let cmd_bytes = cmd.to_bytes();
    let fuzzy = cmdline_fuzzy_complete(cmd);
    let mut p_idx: usize;

    if !fuzzy
        && !opt_vars().p_ic.get()
        && cmd_bytes.first() == Some(&b'k')
        && cmd_bytes.get(1) != Some(&b'e')
    {
        eap.cmdidx = Cmdidx::CmdK;
        p_idx = 1;
    } else {
        p_idx = cmd_bytes
            .iter()
            .take_while(|&&b| b.is_ascii_alphabetic() || b == b'*')
            .count();
        if cmd_bytes.first().is_some_and(u8::is_ascii_uppercase) {
            p_idx += cmd_bytes[p_idx..]
                .iter()
                .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'*')
                .count();
        }
        if cmd_bytes.starts_with(b"py3") && p_idx == 2 {
            p_idx += 1 + cmd_bytes[3..]
                .iter()
                .take_while(|&&b| b.is_ascii_alphabetic() || b == b'*')
                .count();
        }
        if p_idx == 0 && cmd_bytes.first().is_some_and(|b| b"@*!=><&~#".contains(b)) {
            p_idx = 1;
        }
        if p_idx == 0 {
            xp.xp_context = EXPAND_UNSUCCESSFUL;
            return None;
        }

        eap.cmdidx = excmd_get_cmdidx(cmd_bytes, p_idx);

        if cmd_bytes[0].is_ascii_uppercase()
            || (fuzzy && eap.cmdidx != Cmdidx::CmdBang && p_idx < cmd_bytes.len())
        {
            p_idx += cmd_bytes[p_idx..]
                .iter()
                .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'*')
                .count();
        }
    }

    if p_idx >= cmd_bytes.len() && cmd_bytes[p_idx - 1].is_ascii_alphanumeric() {
        return None;
    }

    if eap.cmdidx == Cmdidx::CmdSize {
        if cmd_bytes.first() == Some(&b's')
            && cmd_bytes.get(1).is_some_and(|b| b"cgriI".contains(b))
        {
            eap.cmdidx = Cmdidx::CmdSubstitute;
            p_idx = 1;
        } else if cmd_bytes.first().is_some_and(u8::is_ascii_uppercase) {
            eap.cmd = crate::strings::OwnedSlice::from_cstr(cmd);
            if let Some(off) = find_ucmd(eap, p_idx, None, Some(&mut *xp), Some(&mut *complp)) {
                p_idx = off;
            } else {
                eap.cmdidx = Cmdidx::CmdSize;
                xp.xp_context = EXPAND_UNSUCCESSFUL;
                return None;
            }
        }
    }
    if eap.cmdidx == Cmdidx::CmdSize {
        xp.xp_context = EXPAND_UNSUCCESSFUL;
        return None;
    }

    Some(CStr::from_bytes_until_nul(&cmd_with_nul[p_idx..]).unwrap_or(c""))
}

/// Set the completion context for a command argument with wild card characters.
fn set_context_for_wildcard_arg(
    eap: Option<&Exarg>,
    arg: &CStr,
    usefilter: bool,
    xp: &mut ExpandCtx,
    complp: &mut i32,
) {
    let mut in_quote = false;
    let mut bow: Option<&CStr> = None;

    let arg_skipped = crate::charset::skipwhite(arg);
    let mut pat = arg_skipped;
    let mut p = pat;
    while !p.to_bytes().is_empty() {
        let mut c = utf_ptr2char(p.to_bytes());
        if c == b'\\' as i32 && p.to_bytes().get(1).copied().unwrap_or(0) != 0 {
            p = crate::memory::CStrSubslice::advance(p, 2);
            continue;
        } else if c == b'`' as i32 {
            if !in_quote {
                pat = p;
                bow = Some(crate::memory::CStrSubslice::advance(p, 1));
            }
            in_quote = !in_quote;
        } else if c == b'|' as i32
            || c == b'\n' as i32
            || c == b'"' as i32
            || (ascii_iswhite(c)
                && (!eap.is_some_and(|eap| (eap.argt & EX_NOSPC) != 0) || usefilter))
        {
            while !p.to_bytes().is_empty() {
                c = utf_ptr2char(p.to_bytes());
                if c == b'`' as i32 || vim_isfilec_or_wc(c) {
                    break;
                }
                p = crate::memory::CStrSubslice::advance(p, utfc_ptr2len(p.to_bytes()) as usize);
            }
            if in_quote {
                bow = Some(p);
            } else {
                pat = p;
            }
            continue;
        }
        p = crate::memory::CStrSubslice::advance(p, utfc_ptr2len(p.to_bytes()) as usize);
    }

    if let Some(bow_cstr) = bow
        && in_quote
    {
        pat = bow_cstr;
    }
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(pat);
    xp.xp_context = EXPAND_FILES;

    if usefilter
        || eap.is_some_and(|eap| eap.cmdidx == Cmdidx::CmdBang || eap.cmdidx == Cmdidx::CmdTerminal)
        || *complp == EXPAND_SHELLCMDLINE
    {
        #[cfg(not(windows))]
        {
            xp.xp_shell = true;
        }
        if xp.xp_pattern.addr() == arg_skipped.as_ptr() as usize {
            xp.xp_context = EXPAND_SHELLCMD;
        }
    }

    if xp.xp_pattern.first_byte() == b'$' {
        let p_check = xp.xp_pattern.wrapping_add(1);
        let p_check = p_check.wrapping_add(env_name_len(p_check.as_bytes()));
        if p_check.is_empty() {
            xp.xp_context = EXPAND_ENV_VARS;
            xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
            if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST {
                *complp = EXPAND_ENV_VARS;
            }
        }
    }

    if xp.xp_pattern.first_byte() == b'~' {
        let mut p_check = xp.xp_pattern.wrapping_add(1);
        while !p_check.is_empty() && p_check.first_byte() != b'/' {
            p_check = p_check.wrapping_add(1);
        }
        if p_check.is_empty()
            && p_check.addr() > xp.xp_pattern.wrapping_add(1).addr()
            && match_user(xp.xp_pattern.wrapping_add(1).as_bytes()) >= 1
        {
            xp.xp_context = EXPAND_USER;
            xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
        }
    }
}

/// Set the completion context for the "++opt=arg" argument.
fn set_context_in_argopt(xp: &mut ExpandCtx, arg: &CStr) {
    if let Some(p) = vim_strchr(arg, b'=' as i32) {
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(p).wrapping_add(1);
    } else {
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
    }

    xp.xp_context = EXPAND_ARGOPT;
}

/// Set the completion context for the :filter command.
fn set_context_in_filter_cmd<'a>(xp: &mut ExpandCtx, arg: &'a CStr) -> Option<&'a CStr> {
    let mut p = Some(arg);
    if !arg.to_bytes().is_empty() {
        p = skip_vimgrep_pat(arg, None, None);
    }
    let Some(p_cstr) = p else {
        xp.xp_context = EXPAND_NOTHING;
        return None;
    };
    if p_cstr.to_bytes().is_empty() {
        xp.xp_context = EXPAND_NOTHING;
        return None;
    }
    Some(crate::charset::skipwhite(p_cstr))
}

/// Set the completion context for the :match command.
fn set_context_in_match_cmd<'a>(xp: &mut ExpandCtx, arg: &'a CStr) -> Option<&'a CStr> {
    let mut p = arg;
    if arg.to_bytes().is_empty() || ends_excmd(arg.to_bytes()[0]) == 0 {
        set_context_in_echohl_cmd(xp, arg);
        let ws_off = crate::charset::skiptowhite_len(p.to_bytes());
        p = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(p, ws_off));
        if !p.to_bytes().is_empty() {
            xp.xp_context = EXPAND_NOTHING;
            p = skip_regexp(
                crate::memory::CStrSubslice::advance(p, 1),
                i32::from(p.to_bytes()[0]),
                i32::from(opt_vars().p_magic.get()),
            );
        }
    }
    find_nextcmd(p)
}

/// Returns a slice to the next command after a :global or a :v command.
fn find_cmd_after_global_cmd(arg: &CStr) -> Option<&CStr> {
    let with_nul = arg.to_bytes_with_nul();
    let bytes = arg.to_bytes();
    let (&delim, mut rem) = bytes.split_first()?;

    while !rem.is_empty() && rem[0] != delim {
        if rem[0] == b'\\' && rem.len() >= 2 {
            rem = &rem[2..];
        } else {
            rem = &rem[1..];
        }
    }
    if !rem.is_empty() {
        let off = bytes.len() - rem.len() + 1;
        Some(CStr::from_bytes_until_nul(&with_nul[off..]).unwrap_or(c""))
    } else {
        None
    }
}

/// Returns a slice to the next command after a :substitute or a :& command.
fn find_cmd_after_substitute_cmd(arg: &CStr) -> Option<&CStr> {
    let mut p = arg;
    let delim = p.to_bytes().first().copied().unwrap_or(0);
    if delim != 0 {
        p = skip_regexp(
            crate::memory::CStrSubslice::advance(p, 1),
            i32::from(delim),
            i32::from(opt_vars().p_magic.get()),
        );
        let bytes = p.to_bytes();
        if let Some(mut rem) = bytes.strip_prefix(&[delim]) {
            while !rem.is_empty() && rem[0] != delim {
                if rem[0] == b'\\' && rem.len() >= 2 {
                    rem = &rem[2..];
                } else {
                    rem = &rem[1..];
                }
            }
            if !rem.is_empty() {
                rem = &rem[1..];
            }
            p = crate::memory::CStrSubslice::advance(p, bytes.len() - rem.len());
        }
    }
    let bytes = p.to_bytes();
    bytes
        .iter()
        .position(|&b| matches!(b, b'|' | b'"' | b'#'))
        .map(|pos| crate::memory::CStrSubslice::advance(p, pos))
}

/// Returns a slice to the next command after an :isearch/:dsearch/etc. command.
fn find_cmd_after_isearch_cmd<'a>(xp: &mut ExpandCtx, arg: &'a CStr) -> Option<&'a CStr> {
    let with_nul = arg.to_bytes_with_nul();
    let bytes = arg.to_bytes();
    let rem = crate::charset::skipwhite_slice(crate::charset::skipdigits_slice(bytes));
    let mut rem = rem.strip_prefix(b"/")?;

    while !rem.is_empty() && rem[0] != b'/' {
        if rem[0] == b'\\' && rem.len() >= 2 {
            rem = &rem[2..];
        } else {
            rem = &rem[1..];
        }
    }
    if let Some(after_slash) = rem.strip_prefix(b"/") {
        let rem = crate::charset::skipwhite_slice(after_slash);
        if rem
            .first()
            .is_some_and(|&b| matches!(b, b'|' | b'"' | b'\n'))
        {
            let off = bytes.len() - rem.len();
            return Some(CStr::from_bytes_until_nul(&with_nul[off..]).unwrap_or(c""));
        }
        xp.xp_context = EXPAND_NOTHING;
    }

    None
}

/// Set the completion context for the :unlet command.
fn set_context_in_unlet_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    let mut p = crate::strings::OwnedSlice::from_cstr(arg);
    while let Some(pos) = p.as_bytes().iter().position(|&b| b == b' ') {
        p = p.wrapping_add(pos + 1);
    }

    xp.xp_context = EXPAND_USER_VARS;
    xp.xp_pattern = p;

    if xp.xp_pattern.first_byte() == b'$' {
        xp.xp_context = EXPAND_ENV_VARS;
        xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
    }
}

/// Set the completion context for the :language command.
fn set_context_in_lang_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    let arg_slice = crate::strings::OwnedSlice::from_cstr(arg);
    let bytes = arg_slice.as_bytes();
    let p_rem = crate::charset::skiptowhite_slice(bytes);
    if p_rem.is_empty() {
        xp.xp_context = EXPAND_LANGUAGE;
        xp.xp_pattern = arg_slice;
    } else {
        let n = bytes.len() - p_rem.len();
        let word = &bytes[..n];
        if b"messages".starts_with(word)
            || b"ctype".starts_with(word)
            || b"time".starts_with(word)
            || b"collate".starts_with(word)
        {
            xp.xp_context = EXPAND_LOCALES;
            let rest = crate::charset::skipwhite_slice(p_rem);
            xp.xp_pattern = arg_slice.wrapping_add(bytes.len() - rest.len());
        } else {
            xp.xp_context = EXPAND_NOTHING;
        }
    }
}

/// Set the completion context for the :breakadd command.
fn set_context_in_breakadd_cmd(xp: &mut ExpandCtx, arg: &CStr, cmdidx: Cmdidx) {
    let arg_slice = crate::strings::OwnedSlice::from_cstr(arg);
    xp.xp_context = EXPAND_BREAKPOINT;
    xp.xp_pattern = arg_slice.clone();

    if cmdidx == Cmdidx::CmdBreakadd {
        CMDEXP_STATE.with(|s| s.breakpt_expand_what.set(BreakptExpandWhat::Add));
    } else if cmdidx == Cmdidx::CmdBreakdel {
        CMDEXP_STATE.with(|s| s.breakpt_expand_what.set(BreakptExpandWhat::Del));
    } else {
        CMDEXP_STATE.with(|s| s.breakpt_expand_what.set(BreakptExpandWhat::Profdel));
    }

    let bytes = arg_slice.as_bytes();
    let p_rem = crate::charset::skipwhite_slice(bytes);
    if p_rem.is_empty() {
        return;
    }

    if p_rem.starts_with(b"file ") || p_rem.starts_with(b"func ") {
        let is_file = p_rem.starts_with(b"file");
        let mut rem = crate::charset::skipwhite_slice(&p_rem[4..]);

        if rem.first().is_some_and(u8::is_ascii_digit) {
            rem = crate::charset::skipdigits_slice(rem);
            if rem.first() != Some(&b' ') {
                xp.xp_context = EXPAND_NOTHING;
                return;
            }
            rem = crate::charset::skipwhite_slice(rem);
        }
        xp.xp_context = if is_file {
            EXPAND_FILES
        } else {
            EXPAND_USER_FUNC
        };
        xp.xp_pattern = arg_slice.wrapping_add(bytes.len() - rem.len());
    } else if let Some(after_expr) = p_rem.strip_prefix(b"expr ") {
        let rem = crate::charset::skipwhite_slice(after_expr);
        xp.xp_context = EXPAND_EXPRESSION;
        xp.xp_pattern = arg_slice.wrapping_add(bytes.len() - rem.len());
    }
}

fn set_context_in_scriptnames_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    xp.xp_context = EXPAND_NOTHING;
    xp.xp_pattern = crate::strings::OwnedSlice::NULL;

    let p = crate::strings::OwnedSlice::from_cstr(arg).skipwhite();
    if ascii_isdigit(p.first_byte()) {
        return;
    }

    xp.xp_context = EXPAND_SCRIPTNAMES;
    xp.xp_pattern = p;
}

/// Set the completion context for the :filetype command.
fn set_context_in_filetype_cmd(xp: &mut ExpandCtx, arg: &CStr) {
    let arg_slice = crate::strings::OwnedSlice::from_cstr(arg);
    xp.xp_context = EXPAND_FILETYPECMD;
    xp.xp_pattern = arg_slice.clone();
    CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::All));

    let mut p = arg_slice.skipwhite();
    if p.is_empty() {
        return;
    }

    let mut val: i32 = 0;

    loop {
        let p_bytes = p.as_bytes();
        if p_bytes.starts_with(b"plugin")
            && p_bytes.get(6).is_some_and(|&b| ascii_iswhite(i32::from(b)))
        {
            val |= EXPAND_FILETYPECMD_PLUGIN;
            p = p.wrapping_add(6).skipwhite();
            continue;
        }
        if p_bytes.starts_with(b"indent")
            && p_bytes.get(6).is_some_and(|&b| ascii_iswhite(i32::from(b)))
        {
            val |= EXPAND_FILETYPECMD_INDENT;
            p = p.wrapping_add(6).skipwhite();
            continue;
        }
        break;
    }

    if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 && (val & EXPAND_FILETYPECMD_INDENT) != 0 {
        CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::OnOff));
    } else if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 {
        CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::Indent));
    } else if (val & EXPAND_FILETYPECMD_INDENT) != 0 {
        CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::Plugin));
    }

    xp.xp_pattern = p;
}

/// Sets the completion context for commands that involve a search pattern and a line range.
fn set_context_with_pattern(xp: &mut ExpandCtx) {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    let mut skiplen: i32 = 0;
    let mut dummy: i32 = 0;
    let mut patlen: i32 = 0;
    let mut magic = opt_vars().p_magic.get();
    let mut pos = CMDEXP_STATE.with(|s| s.pre_incsearch_pos.get());
    let retval =
        parse_pattern_and_range(&mut pos, &mut dummy, &mut skiplen, &mut patlen, &mut magic);
    CMDEXP_STATE.with(|s| s.pre_incsearch_pos.set(pos));
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));

    if !retval || ccline.cmdpos.get() <= skiplen || ccline.cmdpos.get() > skiplen + patlen {
        return;
    }

    xp.xp_pattern = ccline.cmdbuff().wrapping_add(skiplen as usize);
    xp.xp_pattern_len = (ccline.cmdpos.get() - skiplen) as usize;
    xp.xp_context = EXPAND_PATTERN_IN_BUF;
    xp.xp_search_dir = FORWARD;
}

/// Set the completion context in "xp" for command "cmd" with index "cmdidx".
fn set_context_by_cmdname<'a>(
    cmd: &'a CStr,
    cmdidx: Cmdidx,
    xp: &mut ExpandCtx,
    arg_cstr: &'a CStr,
    argt: u32,
    context: i32,
    forceit: bool,
) -> Option<&'a CStr> {
    let mut arg = crate::strings::OwnedSlice::from_cstr(arg_cstr);
    match cmdidx {
        Cmdidx::CmdFind | Cmdidx::CmdSfind | Cmdidx::CmdTabfind => {
            if xp.xp_context == EXPAND_FILES {
                xp.xp_context = if get_findfunc().cb_type() != 0 {
                    EXPAND_FINDFUNC
                } else {
                    EXPAND_FILES_IN_PATH
                };
            }
        }
        Cmdidx::CmdBcd
        | Cmdidx::CmdBchdir
        | Cmdidx::CmdCd
        | Cmdidx::CmdChdir
        | Cmdidx::CmdLcd
        | Cmdidx::CmdLchdir
        | Cmdidx::CmdTcd
        | Cmdidx::CmdTchdir => {
            if xp.xp_context == EXPAND_FILES {
                xp.xp_context = EXPAND_DIRS_IN_CDPATH;
            }
        }
        Cmdidx::CmdHelp => {
            xp.xp_context = EXPAND_HELP;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdAboveleft
        | Cmdidx::CmdArgdo
        | Cmdidx::CmdBelowright
        | Cmdidx::CmdBotright
        | Cmdidx::CmdBrowse
        | Cmdidx::CmdBufdo
        | Cmdidx::CmdCdo
        | Cmdidx::CmdCfdo
        | Cmdidx::CmdConfirm
        | Cmdidx::CmdDebug
        | Cmdidx::CmdFolddoclosed
        | Cmdidx::CmdFolddoopen
        | Cmdidx::CmdHide
        | Cmdidx::CmdHorizontal
        | Cmdidx::CmdKeepalt
        | Cmdidx::CmdKeepjumps
        | Cmdidx::CmdKeepmarks
        | Cmdidx::CmdKeeppatterns
        | Cmdidx::CmdLdo
        | Cmdidx::CmdLeftabove
        | Cmdidx::CmdLfdo
        | Cmdidx::CmdLockmarks
        | Cmdidx::CmdNoautocmd
        | Cmdidx::CmdNoswapfile
        | Cmdidx::CmdRestart
        | Cmdidx::CmdRightbelow
        | Cmdidx::CmdSandbox
        | Cmdidx::CmdSilent
        | Cmdidx::CmdTab
        | Cmdidx::CmdTabdo
        | Cmdidx::CmdTopleft
        | Cmdidx::CmdUnsilent
        | Cmdidx::CmdVerbose
        | Cmdidx::CmdVertical
        | Cmdidx::CmdWindo => {
            return Some(arg_cstr);
        }
        Cmdidx::CmdFilter => {
            return set_context_in_filter_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdMatch => {
            return set_context_in_match_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdCommand => {
            return set_context_in_user_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdDelcommand => {
            xp.xp_context = EXPAND_USER_COMMANDS;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdGlobal | Cmdidx::CmdVglobal => {
            let nextcmd = find_cmd_after_global_cmd(arg_cstr);
            if nextcmd.is_none() && CMDEXP_STATE.with(|s| s.may_expand_pattern.get()) {
                set_context_with_pattern(xp);
            }
            return nextcmd;
        }
        Cmdidx::CmdAnd | Cmdidx::CmdSubstitute => {
            let nextcmd = find_cmd_after_substitute_cmd(arg_cstr);
            if nextcmd.is_none() && CMDEXP_STATE.with(|s| s.may_expand_pattern.get()) {
                set_context_with_pattern(xp);
            }
            return nextcmd;
        }
        Cmdidx::CmdIsearch
        | Cmdidx::CmdDsearch
        | Cmdidx::CmdIlist
        | Cmdidx::CmdDlist
        | Cmdidx::CmdIjump
        | Cmdidx::CmdPsearch
        | Cmdidx::CmdDjump
        | Cmdidx::CmdIsplit
        | Cmdidx::CmdDsplit => {
            return find_cmd_after_isearch_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdAutocmd => {
            return aucmd_set_expand_context(xp, arg_cstr, false);
        }
        Cmdidx::CmdDoautocmd | Cmdidx::CmdDoautoall => {
            return aucmd_set_expand_context(xp, arg_cstr, true);
        }
        Cmdidx::CmdSet => {
            set_context_in_set_cmd(xp, arg_cstr, 0);
        }
        Cmdidx::CmdSetglobal => {
            set_context_in_set_cmd(xp, arg_cstr, OPT_GLOBAL);
        }
        Cmdidx::CmdSetlocal => {
            set_context_in_set_cmd(xp, arg_cstr, OPT_LOCAL);
        }
        Cmdidx::CmdTag
        | Cmdidx::CmdStag
        | Cmdidx::CmdPtag
        | Cmdidx::CmdLtag
        | Cmdidx::CmdTselect
        | Cmdidx::CmdStselect
        | Cmdidx::CmdPtselect
        | Cmdidx::CmdTjump
        | Cmdidx::CmdStjump
        | Cmdidx::CmdPtjump => {
            if (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_TAGFILE) != 0 {
                xp.xp_context = EXPAND_TAGS_LISTFILES;
            } else {
                xp.xp_context = EXPAND_TAGS;
            }
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdAugroup => {
            xp.xp_context = EXPAND_AUGROUP;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdSyntax => {
            set_context_in_syntax_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdConst
        | Cmdidx::CmdLet
        | Cmdidx::CmdIf
        | Cmdidx::CmdElseif
        | Cmdidx::CmdWhile
        | Cmdidx::CmdFor
        | Cmdidx::CmdEcho
        | Cmdidx::CmdEchon
        | Cmdidx::CmdExecute
        | Cmdidx::CmdEchomsg
        | Cmdidx::CmdEchoerr
        | Cmdidx::CmdCall
        | Cmdidx::CmdReturn
        | Cmdidx::CmdCexpr
        | Cmdidx::CmdCaddexpr
        | Cmdidx::CmdCgetexpr
        | Cmdidx::CmdLexpr
        | Cmdidx::CmdLaddexpr
        | Cmdidx::CmdLgetexpr => {
            set_context_for_expression(xp, arg_cstr, cmdidx);
        }
        Cmdidx::CmdUnlet => {
            set_context_in_unlet_cmd(xp, arg_cstr);
            return None;
        }
        Cmdidx::CmdFunction | Cmdidx::CmdDelfunction => {
            xp.xp_context = EXPAND_USER_FUNC;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdEchohl => {
            set_context_in_echohl_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdHighlight => {
            set_context_in_highlight_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdSign => {
            set_context_in_sign_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdBdelete | Cmdidx::CmdBwipeout | Cmdidx::CmdBunload => {
            while let Some(pos) = arg.as_bytes().iter().position(|&b| b == b' ') {
                arg = arg.wrapping_add(pos + 1);
            }
            xp.xp_context = EXPAND_BUFFERS;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdBuffer | Cmdidx::CmdSbuffer | Cmdidx::CmdPbuffer | Cmdidx::CmdChecktime => {
            xp.xp_context = EXPAND_BUFFERS;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdDiffget | Cmdidx::CmdDiffput => {
            xp.xp_context = EXPAND_DIFF_BUFFERS;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdUser | Cmdidx::CmdUserBuf => {
            return set_context_in_user_cmdarg(cmd, arg_cstr, argt, context, xp, forceit);
        }
        Cmdidx::CmdMap
        | Cmdidx::CmdNoremap
        | Cmdidx::CmdNmap
        | Cmdidx::CmdNnoremap
        | Cmdidx::CmdVmap
        | Cmdidx::CmdVnoremap
        | Cmdidx::CmdOmap
        | Cmdidx::CmdOnoremap
        | Cmdidx::CmdImap
        | Cmdidx::CmdInoremap
        | Cmdidx::CmdCmap
        | Cmdidx::CmdCnoremap
        | Cmdidx::CmdLmap
        | Cmdidx::CmdLnoremap
        | Cmdidx::CmdSmap
        | Cmdidx::CmdSnoremap
        | Cmdidx::CmdXmap
        | Cmdidx::CmdXnoremap => {
            return set_context_in_map_cmd(xp, cmd, arg_cstr, forceit, false, false, cmdidx);
        }
        Cmdidx::CmdUnmap
        | Cmdidx::CmdNunmap
        | Cmdidx::CmdVunmap
        | Cmdidx::CmdOunmap
        | Cmdidx::CmdIunmap
        | Cmdidx::CmdCunmap
        | Cmdidx::CmdLunmap
        | Cmdidx::CmdSunmap
        | Cmdidx::CmdXunmap => {
            return set_context_in_map_cmd(xp, cmd, arg_cstr, forceit, false, true, cmdidx);
        }
        Cmdidx::CmdMapclear
        | Cmdidx::CmdNmapclear
        | Cmdidx::CmdVmapclear
        | Cmdidx::CmdOmapclear
        | Cmdidx::CmdImapclear
        | Cmdidx::CmdCmapclear
        | Cmdidx::CmdLmapclear
        | Cmdidx::CmdSmapclear
        | Cmdidx::CmdXmapclear => {
            xp.xp_context = EXPAND_MAPCLEAR;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdAbbreviate
        | Cmdidx::CmdNoreabbrev
        | Cmdidx::CmdCabbrev
        | Cmdidx::CmdCnoreabbrev
        | Cmdidx::CmdIabbrev
        | Cmdidx::CmdInoreabbrev => {
            return set_context_in_map_cmd(xp, cmd, arg_cstr, forceit, true, false, cmdidx);
        }
        Cmdidx::CmdUnabbreviate | Cmdidx::CmdCunabbrev | Cmdidx::CmdIunabbrev => {
            return set_context_in_map_cmd(xp, cmd, arg_cstr, forceit, true, true, cmdidx);
        }
        Cmdidx::CmdMenu
        | Cmdidx::CmdNoremenu
        | Cmdidx::CmdUnmenu
        | Cmdidx::CmdAmenu
        | Cmdidx::CmdAnoremenu
        | Cmdidx::CmdAunmenu
        | Cmdidx::CmdNmenu
        | Cmdidx::CmdNnoremenu
        | Cmdidx::CmdNunmenu
        | Cmdidx::CmdVmenu
        | Cmdidx::CmdVnoremenu
        | Cmdidx::CmdVunmenu
        | Cmdidx::CmdOmenu
        | Cmdidx::CmdOnoremenu
        | Cmdidx::CmdOunmenu
        | Cmdidx::CmdImenu
        | Cmdidx::CmdInoremenu
        | Cmdidx::CmdIunmenu
        | Cmdidx::CmdCmenu
        | Cmdidx::CmdCnoremenu
        | Cmdidx::CmdCunmenu
        | Cmdidx::CmdTlmenu
        | Cmdidx::CmdTlnoremenu
        | Cmdidx::CmdTlunmenu
        | Cmdidx::CmdTmenu
        | Cmdidx::CmdTunmenu
        | Cmdidx::CmdPopup
        | Cmdidx::CmdEmenu => {
            set_context_in_menu_cmd(xp, cmd, arg_cstr, forceit);
            return None;
        }
        Cmdidx::CmdColorscheme => {
            xp.xp_context = EXPAND_COLORS;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdCompiler => {
            xp.xp_context = EXPAND_COMPILER;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdOwnsyntax => {
            xp.xp_context = EXPAND_OWNSYNTAX;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdSetfiletype => {
            xp.xp_context = EXPAND_FILETYPE;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdPackadd => {
            xp.xp_context = EXPAND_PACKADD;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdRuntime => {
            set_context_in_runtime_cmd(xp, arg_cstr);
        }
        Cmdidx::CmdLanguage => {
            set_context_in_lang_cmd(xp, arg_cstr);
            return None;
        }
        Cmdidx::CmdProfile => {
            set_context_in_profile_cmd(xp, arg_cstr.to_bytes());
        }
        Cmdidx::CmdCheckhealth => {
            xp.xp_context = EXPAND_CHECKHEALTH;
        }
        Cmdidx::CmdMarks => {
            xp.xp_context = EXPAND_MARKS;
        }
        Cmdidx::CmdLog => {
            xp.xp_context = EXPAND_LOG;
        }
        Cmdidx::CmdLsp => {
            xp.xp_context = EXPAND_LSP;
        }
        Cmdidx::CmdPackdel => {
            xp.xp_context = EXPAND_PACKDEL;
        }
        Cmdidx::CmdPackupdate => {
            xp.xp_context = EXPAND_PACKUPDATE;
        }
        Cmdidx::CmdRetab => {
            xp.xp_context = EXPAND_RETAB;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdMessages => {
            xp.xp_context = EXPAND_MESSAGES;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdHistory => {
            xp.xp_context = EXPAND_HISTORY;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdSyntime => {
            xp.xp_context = EXPAND_SYNTIME;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdArgdelete => {
            while let Some(pos) = arg.as_bytes().iter().position(|&b| b == b' ') {
                arg = arg.wrapping_add(pos + 1);
            }
            xp.xp_context = EXPAND_ARGLIST;
            xp.xp_pattern = arg;
        }
        Cmdidx::CmdBreakadd | Cmdidx::CmdProfdel | Cmdidx::CmdBreakdel => {
            set_context_in_breakadd_cmd(xp, arg_cstr, cmdidx);
            return None;
        }
        Cmdidx::CmdScriptnames => {
            set_context_in_scriptnames_cmd(xp, arg_cstr);
            return None;
        }
        Cmdidx::CmdFiletype => {
            set_context_in_filetype_cmd(xp, arg_cstr);
            return None;
        }
        Cmdidx::CmdLua | Cmdidx::CmdEqual => {
            xp.xp_context = EXPAND_LUA;
        }
        _ => {}
    }
    None
}

/// Parse one command from `buff` and set completion context in `xp`.
fn set_one_cmd_context<'a>(xp: &mut ExpandCtx, buff_cstr: &'a CStr) -> Option<&'a CStr> {
    let mut ea: Exarg = Exarg::new();
    let mut context: i32 = EXPAND_NOTHING;
    let mut forceit = false;
    let mut usefilter = false;

    expand_init(xp);
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(buff_cstr);
    xp.xp_line = xp.xp_pattern.clone();
    xp.xp_context = EXPAND_COMMANDS;
    ea.argt = 0;

    let mut cmd = buff_cstr;
    while let Some(&b) = cmd.to_bytes().first()
        && crate::strings::vim_haschr(c" \t:|", i32::from(b))
    {
        cmd = crate::memory::CStrSubslice::advance(cmd, 1);
    }
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(cmd);

    if cmd.to_bytes().is_empty() {
        return None;
    }
    if cmd.to_bytes().first() == Some(&b'"') {
        xp.xp_context = EXPAND_NOTHING;
        return None;
    }

    cmd = skip_range(cmd, Some(&mut xp.xp_context));
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(cmd);
    if cmd.to_bytes().is_empty() {
        return None;
    }
    if cmd.to_bytes().first() == Some(&b'"') {
        xp.xp_context = EXPAND_NOTHING;
        return None;
    }

    if matches!(cmd.to_bytes().first(), Some(&b'|' | &b'\n')) {
        return Some(crate::memory::CStrSubslice::advance(cmd, 1));
    }

    let cmd_cstr = cmd;
    let mut p = set_cmd_index(cmd_cstr, &mut ea, xp, &mut context)?;

    xp.xp_context = EXPAND_NOTHING;

    if p.to_bytes().first() == Some(&b'!') {
        forceit = true;
        p = crate::memory::CStrSubslice::advance(p, 1);
    }

    if !is_user_cmdidx(ea.cmdidx) {
        ea.argt = excmd_get_argt(ea.cmdidx);
    }

    let mut arg = crate::charset::skipwhite(p);

    if (ea.argt & EX_ARGOPT) != 0 {
        while arg.to_bytes().starts_with(b"++") {
            p = crate::memory::CStrSubslice::advance(arg, 2);
            while let Some(&b) = p.to_bytes().first()
                && !ascii_isspace(i32::from(b))
            {
                p = crate::memory::CStrSubslice::advance(p, utfc_ptr2len(p.to_bytes()) as usize);
            }

            if p.to_bytes().is_empty() && (ea.argt & EX_ARGOPT) != 0 {
                set_context_in_argopt(xp, crate::memory::CStrSubslice::advance(arg, 2));
                return None;
            }

            arg = crate::charset::skipwhite(p);
        }
    }

    if ea.cmdidx == Cmdidx::CmdWrite || ea.cmdidx == Cmdidx::CmdUpdate {
        if arg.to_bytes().first() == Some(&b'>') {
            arg = crate::memory::CStrSubslice::advance(arg, 1);
            if arg.to_bytes().first() == Some(&b'>') {
                arg = crate::memory::CStrSubslice::advance(arg, 1);
            }
            arg = crate::charset::skipwhite(arg);
        } else if arg.to_bytes().first() == Some(&b'!') && ea.cmdidx == Cmdidx::CmdWrite {
            arg = crate::memory::CStrSubslice::advance(arg, 1);
            usefilter = true;
        }
    }

    if ea.cmdidx == Cmdidx::CmdRead {
        usefilter = forceit;
        if arg.to_bytes().first() == Some(&b'!') {
            arg = crate::memory::CStrSubslice::advance(arg, 1);
            usefilter = true;
        }
    }

    if ea.cmdidx == Cmdidx::CmdLshift || ea.cmdidx == Cmdidx::CmdRshift {
        let cmd_first = cmd.to_bytes().first().copied().unwrap_or(0);
        while arg.to_bytes().first().copied().unwrap_or(0) == cmd_first && cmd_first != 0 {
            arg = crate::memory::CStrSubslice::advance(arg, 1);
        }
        arg = crate::charset::skipwhite(arg);
    }

    if (ea.argt & EX_CMDARG) != 0 && !usefilter && arg.to_bytes().first() == Some(&b'+') {
        p = crate::memory::CStrSubslice::advance(arg, 1);
        arg = skip_cmd_arg(arg, false);

        if arg.to_bytes().is_empty() {
            return Some(p);
        }

        arg = crate::charset::skipwhite(arg);
    }

    if (ea.argt & EX_TRLBAR) != 0 && !usefilter {
        p = arg;
        let p_base = p;
        let p_bytes = p_base.to_bytes();
        let mut idx = if ea.cmdidx == Cmdidx::CmdRedir && p_bytes.starts_with(b"@\"") {
            2
        } else {
            0
        };
        while idx < p_bytes.len() {
            let b = p_bytes[idx];
            if b == CTRL_V_CONST as u8 {
                if idx + 1 < p_bytes.len() {
                    idx += 1;
                }
            } else if ((b == b'"' && (ea.argt & EX_NOTRLCOM) == 0) || b == b'|' || b == b'\n')
                && (idx == 0 || p_bytes[idx - 1] != b'\\')
            {
                if b == b'|' || b == b'\n' {
                    return Some(crate::memory::CStrSubslice::advance(p_base, idx + 1));
                }
                return None;
            }
            idx += crate::mbyte::utfc_slice2len(&p_bytes[idx..]).max(1);
        }
    }

    if (ea.argt & EX_EXTRA) == 0
        && !arg.to_bytes().is_empty()
        && !matches!(arg.to_bytes().first(), Some(&b'|' | &b'"'))
    {
        return None;
    }

    let buff_bytes = buff_cstr.to_bytes();
    let mut idx = 0usize;
    let buff_slice = crate::strings::OwnedSlice::from_cstr(buff_cstr);
    xp.xp_pattern = buff_slice.clone();
    while idx < buff_bytes.len() {
        if matches!(buff_bytes[idx], b' ' | TAB) {
            idx += 1;
            xp.xp_pattern = buff_slice.wrapping_add(idx);
        } else {
            if buff_bytes[idx] == b'\\' && idx + 1 < buff_bytes.len() {
                idx += 1;
            }
            idx += crate::mbyte::utfc_slice2len(&buff_bytes[idx..]).max(1);
        }
    }

    let arg_cstr = arg;
    if (ea.argt & EX_XFILE) != 0 {
        set_context_for_wildcard_arg(Some(&ea), arg_cstr, usefilter, xp, &mut context);
    }

    set_context_by_cmdname(cmd_cstr, ea.cmdidx, xp, arg_cstr, ea.argt, context, forceit)
}

/// Set the completion context in "xp" for command "str".
pub fn set_cmd_context(xp: &mut ExpandCtx, str_buf: &[u8], len: i32, col: i32, use_ccline: bool) {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;
    let truncated_buf: Option<Vec<u8>> = if col < len && (col as usize) < str_buf.len() {
        let mut v = str_buf[..col as usize].to_vec();
        v.push(0);
        Some(v)
    } else {
        None
    };
    let work_slice: &[u8] = truncated_buf.as_deref().unwrap_or(str_buf);

    CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::All));
    CMDEXP_STATE.with(|s| s.breakpt_expand_what.set(BreakptExpandWhat::Add));

    if use_ccline && ccline.cmdfirstc.get() == b'=' as i32 {
        set_context_for_expression(
            xp,
            crate::memory::bytes_as_cstr(work_slice),
            Cmdidx::CmdSize,
        );
    } else if use_ccline && ccline.input_fn.get() != 0 {
        xp.xp_context = ccline.xp_context.get();
        xp.xp_pattern = ccline.cmdbuff();
        xp.xp_arg = ccline.xp_arg.borrow().clone();
        if xp.xp_context == EXPAND_SHELLCMDLINE {
            let mut context = xp.xp_context;
            set_context_for_wildcard_arg(
                None,
                crate::memory::bytes_as_cstr(work_slice),
                false,
                xp,
                &mut context,
            );
        }
    } else {
        let mut nextcomm = Some(crate::memory::bytes_as_cstr(work_slice));
        while let Some(comm) = nextcomm {
            nextcomm = set_one_cmd_context(xp, comm);
        }
    }

    xp.xp_col = col;
    xp.xp_line = crate::strings::OwnedSlice::from_bytes(str_buf);
    if !xp.xp_pattern.is_null() {
        let work_base = work_slice.as_ptr() as usize;
        let addr = xp.xp_pattern.addr();
        if addr >= work_base && addr < work_base + work_slice.len() {
            let offset = (addr - work_base).min(str_buf.len().saturating_sub(1));
            xp.xp_pattern = xp.xp_line.wrapping_add(offset);
        }
    }
}

/// Expand the command line "str" from context "xp".
pub fn expand_cmdline(
    xp: &mut ExpandCtx,
    str_addr: usize,
    col: i32,
    matches: &mut Vec<CString>,
) -> i32 {
    let mut options = WILD_ADD_SLASH | WILD_SILENT;

    if xp.xp_context == EXPAND_UNSUCCESSFUL {
        beep_flush();
        return EXPAND_UNSUCCESSFUL;
    }
    if xp.xp_context == EXPAND_NOTHING {
        return EXPAND_NOTHING;
    }

    xp.xp_pattern_len = str_addr
        .wrapping_add(col as usize)
        .saturating_sub(xp.xp_pattern.addr());
    let file_str = if cmdline_fuzzy_completion_supported(xp) {
        xp.xp_pattern.to_cstring()
    } else {
        addstar(xp.xp_pattern.slice(xp.xp_pattern_len), xp.xp_context)
    };

    if opt_vars().p_wic.get() {
        options += WILD_ICASE;
    }

    if !expand_from_context(xp, file_str.as_c_str(), matches, options) {
        matches.clear();
    }

    EXPAND_OK
}

/// Expand file or directory names.
fn expand_files_and_dirs(
    xp: &mut ExpandCtx,
    pat_arg: &CStr,
    matches: &mut Vec<CString>,
    mut flags: i32,
    options: i32,
) -> bool {
    {
        let mut pat_owned: Option<Vec<u8>> = None;

        if xp.xp_backslash != XP_BS_NONE {
            let pat_bytes = pat_arg.to_bytes();
            let pat_len = pat_bytes.len();
            let mut pat_vec = pat_arg.to_bytes_with_nul().to_vec();
            let mut cur_len = pat_len;
            let mut idx = 0usize;
            while idx < cur_len {
                if pat_vec[idx] == b'\\' {
                    if (xp.xp_backslash & XP_BS_THREE) != 0
                        && pat_vec.get(idx + 1..idx + 4) == Some(b"\\\\ ")
                    {
                        pat_vec.copy_within(idx + 3..=cur_len, idx);
                        cur_len -= 3;
                    } else if (xp.xp_backslash & XP_BS_ONE) != 0
                        && pat_vec.get(idx + 1) == Some(&b' ')
                    {
                        pat_vec.copy_within(idx + 1..=cur_len, idx);
                        cur_len -= 1;
                    } else if (xp.xp_backslash & XP_BS_COMMA) != 0 {
                        if pat_vec.get(idx + 1..idx + 3) == Some(b"\\,") {
                            pat_vec.copy_within(idx + 2..=cur_len, idx);
                            cur_len -= 2;
                        } else {
                            #[cfg(windows)]
                            if pat_vec.get(idx + 1) == Some(&b',') {
                                pat_vec.copy_within(idx + 1..=cur_len, idx);
                                cur_len -= 1;
                            }
                        }
                    }
                }
                idx += 1;
            }
            pat_owned = Some(pat_vec);
        }

        let pat_cstr = pat_owned
            .as_deref()
            .map_or(pat_arg, crate::memory::bytes_as_cstr);
        let ret = if xp.xp_context == EXPAND_FINDFUNC {
            expand_findfunc(xp, pat_cstr, matches)
        } else {
            if xp.xp_context == EXPAND_FILES {
                flags |= EW_FILE;
            } else if xp.xp_context == EXPAND_FILES_IN_PATH {
                flags |= EW_FILE | EW_PATH;
            } else if xp.xp_context == EXPAND_DIRS_IN_CDPATH {
                flags = (flags | EW_DIR | EW_CDPATH) & !EW_FILE;
            } else {
                flags = (flags | EW_DIR) & !EW_FILE;
            }
            if (options & WILD_ICASE) != 0 {
                flags |= EW_ICASE;
            }
            expand_wildcards_eval(pat_cstr, matches, flags)
        };
        drop(pat_owned);
        #[cfg(windows)]
        if ((options & WILD_USE_SHELLSLASH) != 0 && !opt_vars().p_ssl.get())
            || ((options & WILD_USE_COMPLETESLASH) != 0
                && ((opt_vars().p_csl.is_empty() && !opt_vars().p_ssl.get())
                    || opt_vars().p_csl.first_byte() == b'b'))
        {
            for slot in matches.iter_mut() {
                if slot.as_bytes().contains(&PATHSEP) {
                    let bytes: Vec<u8> = slot
                        .as_bytes()
                        .iter()
                        .map(|&b| if b == PATHSEP { b'\\' } else { b })
                        .collect();
                    *slot = bytes_to_cstring(&bytes);
                }
            }
        }
        ret
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":filetype".
fn get_filetypecmd_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx < 0 {
        return None;
    }

    let i = idx as usize;
    if CMDEXP_STATE.with(|s| s.filetype_expand_what.get()) == FiletypeExpandWhat::All && i < 5 {
        const OPTS_ALL: [&CStr; 5] = [c"detect", c"indent", c"plugin", c"on", c"off"];
        return Some(OPTS_ALL[i]);
    }
    if CMDEXP_STATE.with(|s| s.filetype_expand_what.get()) == FiletypeExpandWhat::Plugin && i < 3 {
        const OPTS_PLUGIN: [&CStr; 3] = [c"plugin", c"on", c"off"];
        return Some(OPTS_PLUGIN[i]);
    }
    if CMDEXP_STATE.with(|s| s.filetype_expand_what.get()) == FiletypeExpandWhat::Indent && i < 3 {
        const OPTS_INDENT: [&CStr; 3] = [c"indent", c"on", c"off"];
        return Some(OPTS_INDENT[i]);
    }
    if CMDEXP_STATE.with(|s| s.filetype_expand_what.get()) == FiletypeExpandWhat::OnOff && i < 2 {
        const OPTS_ONOFF: [&CStr; 2] = [c"on", c"off"];
        return Some(OPTS_ONOFF[i]);
    }
    None
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":breakadd" / ":breakdel" / ":profdel".
fn get_breakadd_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if (0..=3).contains(&idx) {
        const OPTS: [&CStr; 4] = [c"expr", c"file", c"func", c"here"];
        let i = idx as usize;

        if CMDEXP_STATE.with(|s| s.breakpt_expand_what.get()) == BreakptExpandWhat::Add {
            return Some(OPTS[i]);
        } else if CMDEXP_STATE.with(|s| s.breakpt_expand_what.get()) == BreakptExpandWhat::Del {
            if idx <= 2 {
                return Some(OPTS[i + 1]);
            }
        } else if idx <= 1 {
            return Some(OPTS[i + 1]);
        }
    }
    None
}

/// Function given to ExpandGeneric() to obtain the possible arguments for ":scriptnames".
fn get_scriptnames_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    if !script_id_valid(idx + 1) {
        return None;
    }

    let sname = script_item_name(idx + 1);
    let mut name_buff = [0u8; MAXPATHL];
    home_replace(None, sname.as_deref(), &mut name_buff, true);
    let item = crate::memory::cstring_until_nul(&name_buff);
    Some(xp.set_item(item))
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":retab".
fn get_retab_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx == 0).then_some(c"-indentonly")
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":messages".
fn get_messages_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx == 0).then_some(c"clear")
}

fn get_mapclear_arg(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    (idx == 0).then_some(c"<buffer>")
}

/// Completion for |:checkhealth| command.
fn get_healthcheck_names(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    let prompt_id = get_cmdline_last_prompt_id();
    if CMDEXP_STATE.with(|s| s.healthcheck_last_gen.get()) != prompt_id
        || CMDEXP_STATE.with(|s| s.healthcheck_last_gen.get()) == 0
    {
        let a = Array::new();
        let mut err = Error::new();
        let code = c"return vim.health._complete()";
        let res = nlua_exec(NvimString::from_cstr(code), None, a, K_RET_OBJECT, &mut err);
        api_clear_error(&mut err);
        api_free_object(CMDEXP_STATE.with(|s| s.healthcheck_names.get()));
        CMDEXP_STATE.with(|s| s.healthcheck_names.set(res));
        CMDEXP_STATE.with(|s| s.healthcheck_last_gen.set(prompt_id));
    }

    if CMDEXP_STATE.with(|s| s.healthcheck_names.get()).obj_type == ObjectType::ObjectTypeArray
        && idx >= 0
        && (idx as usize)
            < CMDEXP_STATE
                .with(|s| s.healthcheck_names.get())
                .data
                .array()
                .size
    {
        let names = CMDEXP_STATE
            .with(|s| s.healthcheck_names.get())
            .data
            .array();
        let item = &names[idx as usize];
        if item.obj_type == ObjectType::ObjectTypeString && !item.data.string().data.is_null() {
            return Some(xp.set_item(crate::memory::bytes_to_cstring(item.data.as_bytes())));
        }
    }
    None
}

/// Completes arg1 via Lua.
fn get_arg1_from_lua<'a>(
    lua: &std::ffi::CStr,
    xp: &'a mut ExpandCtx,
    idx: i32,
) -> Option<&'a CStr> {
    let prompt_id = get_cmdline_last_prompt_id();
    let xp_line_bytes = xp.xp_line.as_bytes();

    let line_changed = ARG1_LUA_LAST_XP_LINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_none_or(|s| s.as_bytes() != xp_line_bytes)
    });
    if line_changed || CMDEXP_STATE.with(|s| s.arg1_lua_last_gen.get()) != prompt_id {
        ARG1_LUA_LAST_XP_LINE.with(|cell| {
            *cell.borrow_mut() = Some(xp.xp_line.to_cstring());
        });
        let mut args_items = [
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(xp.xp_pattern.as_opt_cstr())),
            },
            Object {
                obj_type: ObjectType::ObjectTypeString,
                data: ObjectData::String(cstr_as_string(xp.xp_line.as_opt_cstr())),
            },
        ];
        let args = Array::from_slice(&mut args_items[..2]);
        let mut err = Error::new();
        let res = nlua_exec(
            cstr_as_string(Some(lua)),
            None,
            args,
            K_RET_OBJECT,
            &mut err,
        );
        api_clear_error(&mut err);
        api_free_object(CMDEXP_STATE.with(|s| s.arg1_lua_names.get()));
        CMDEXP_STATE.with(|s| s.arg1_lua_names.set(res));
        CMDEXP_STATE.with(|s| s.arg1_lua_last_gen.set(prompt_id));
    }

    if CMDEXP_STATE.with(|s| s.arg1_lua_names.get()).obj_type == ObjectType::ObjectTypeArray
        && idx >= 0
        && (idx as usize)
            < CMDEXP_STATE
                .with(|s| s.arg1_lua_names.get())
                .data
                .array()
                .size
    {
        let names = CMDEXP_STATE.with(|s| s.arg1_lua_names.get()).data.array();
        let item = &names[idx as usize];
        if item.obj_type == ObjectType::ObjectTypeString && !item.data.string().data.is_null() {
            return Some(xp.set_item(crate::memory::bytes_to_cstring(item.data.as_bytes())));
        }
    }
    None
}

fn get_marks_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_arg1_from_lua(c"return require'vim._core.marks'.complete(...)", xp, idx)
}

fn get_log_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.log_complete(...)",
        xp,
        idx,
    )
}

fn get_lsp_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.lsp_complete(...)",
        xp,
        idx,
    )
}

fn get_packdel_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.packdel_complete(...)",
        xp,
        idx,
    )
}

fn get_packupdate_arg(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.packupdate_complete(...)",
        xp,
        idx,
    )
}

/// Do the expansion based on xp->xp_context and "rmp".
fn expand_other(
    pat: &CStr,
    xp: &mut ExpandCtx,
    rmp: &mut Regmatch,
    matches: &mut Vec<CString>,
) -> bool {
    struct ExpGen {
        context: i32,
        func: fn(&mut ExpandCtx, i32) -> Option<&CStr>,
        ic: bool,
        escaped: bool,
    }

    let tab: [ExpGen; 37] = [
        ExpGen {
            context: EXPAND_COMMANDS,
            func: get_command_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_FILETYPECMD,
            func: get_filetypecmd_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MAPCLEAR,
            func: get_mapclear_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MESSAGES,
            func: get_messages_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_HISTORY,
            func: get_history_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_COMMANDS,
            func: get_user_commands,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_ADDR_TYPE,
            func: get_user_cmd_addr_type,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_CMD_FLAGS,
            func: get_user_cmd_flags,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_NARGS,
            func: get_user_cmd_nargs,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_COMPLETE,
            func: get_user_cmd_complete,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_VARS,
            func: get_user_var_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_FUNCTIONS,
            func: get_function_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_FUNC,
            func: get_user_func_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_EXPRESSION,
            func: get_expr_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MENUS,
            func: get_menu_name,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MENUNAMES,
            func: get_menu_names,
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_SYNTAX,
            func: get_syntax_name,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_SYNTIME,
            func: get_syntime_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_HIGHLIGHT,
            func: get_highlight_name,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_EVENTS,
            func: expand_get_event_name,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_AUGROUP,
            func: expand_get_augroup_name,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_SIGN,
            func: get_sign_name,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_PROFILE,
            func: get_profile_name,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_LANGUAGE,
            func: get_lang_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_LOCALES,
            func: get_locales,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_ENV_VARS,
            func: get_env_name,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER,
            func: get_users,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_ARGLIST,
            func: get_arglist_name,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_BREAKPOINT,
            func: get_breakadd_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_SCRIPTNAMES,
            func: get_scriptnames_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_RETAB,
            func: get_retab_arg,
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_CHECKHEALTH,
            func: get_healthcheck_names,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_MARKS,
            func: get_marks_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_LOG,
            func: get_log_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_LSP,
            func: get_lsp_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_PACKDEL,
            func: get_packdel_arg,
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_PACKUPDATE,
            func: get_packupdate_arg,
            ic: true,
            escaped: false,
        },
    ];

    for entry in &tab {
        if xp.xp_context == entry.context {
            if entry.ic {
                rmp.rm_ic = true;
            }
            expand_generic(pat, xp, rmp, matches, Some(entry.func), entry.escaped);
            return true;
        }
    }
    false
}

/// Map wild expand options to flags for expand_wildcards().
fn map_wildopts_to_ewflags(options: i32) -> i32 {
    let mut flags = EW_DIR;
    if (options & WILD_LIST_NOTFOUND) != 0 {
        flags |= EW_NOTFOUND;
    }
    if (options & WILD_ADD_SLASH) != 0 {
        flags |= EW_ADDSLASH;
    }
    if (options & WILD_KEEP_ALL) != 0 {
        flags |= EW_KEEPALL;
    }
    if (options & WILD_SILENT) != 0 {
        flags |= EW_SILENT;
    }
    if (options & WILD_NOERROR) != 0 {
        flags |= EW_NOERROR;
    }
    if (options & WILD_ALLLINKS) != 0 {
        flags |= EW_ALLLINKS;
    }
    flags
}

/// Do the expansion based on xp->xp_context and "pat".
fn expand_from_context(
    xp: &mut ExpandCtx,
    pat_arg: &CStr,
    matches: &mut Vec<CString>,
    options: i32,
) -> bool {
    let mut pat_cstr = pat_arg;
    let mut regmatch: Regmatch = Regmatch::new();
    let flags = map_wildopts_to_ewflags(options);
    let fuzzy = cmdline_fuzzy_complete(pat_cstr) && cmdline_fuzzy_completion_supported(xp);

    if xp.xp_context == EXPAND_FILES
        || xp.xp_context == EXPAND_DIRECTORIES
        || xp.xp_context == EXPAND_FILES_IN_PATH
        || xp.xp_context == EXPAND_FINDFUNC
        || xp.xp_context == EXPAND_DIRS_IN_CDPATH
    {
        return expand_files_and_dirs(xp, pat_cstr, matches, flags, options);
    }

    matches.clear();
    if xp.xp_context == EXPAND_HELP {
        if find_help_tags(
            if pat_cstr.to_bytes().is_empty() {
                c"help"
            } else {
                pat_cstr
            },
            matches,
            false,
        ) {
            cleanup_help_tags(matches);
            return true;
        }
        return false;
    }

    if xp.xp_context == EXPAND_SHELLCMD {
        expand_shellcmd(pat_cstr, matches, flags);
        return true;
    }
    if xp.xp_context == EXPAND_OLD_SETTING {
        return expand_old_setting(matches);
    }
    if xp.xp_context == EXPAND_BUFFERS {
        return expand_bufnames(pat_cstr.to_bytes(), matches, options);
    }
    if xp.xp_context == EXPAND_DIFF_BUFFERS {
        return expand_bufnames(pat_cstr.to_bytes(), matches, options | BUF_DIFF_FILTER);
    }
    if xp.xp_context == EXPAND_TAGS || xp.xp_context == EXPAND_TAGS_LISTFILES {
        return expand_tags(xp.xp_context == EXPAND_TAGS, pat_cstr, matches);
    }
    if xp.xp_context == EXPAND_COLORS {
        return expand_rtdir(pat_cstr, DIP_START + DIP_OPT, matches, &[c"colors"]);
    }
    if xp.xp_context == EXPAND_COMPILER {
        return expand_rtdir(pat_cstr, 0, matches, &[c"compiler"]);
    }
    if xp.xp_context == EXPAND_OWNSYNTAX {
        return expand_rtdir(pat_cstr, 0, matches, &[c"syntax"]);
    }
    if xp.xp_context == EXPAND_FILETYPE {
        return expand_rtdir(pat_cstr, 0, matches, &[c"syntax", c"indent", c"ftplugin"]);
    }
    if xp.xp_context == EXPAND_KEYMAP {
        return expand_rtdir(pat_cstr, 0, matches, &[c"keymap"]);
    }
    if xp.xp_context == EXPAND_USER_LIST {
        return expand_user_list(xp, matches);
    }
    if xp.xp_context == EXPAND_USER_LUA {
        return expand_user_lua(xp, matches);
    }
    if xp.xp_context == EXPAND_PACKADD {
        return ExpandPackAddDir(pat_cstr, matches);
    }
    if xp.xp_context == EXPAND_RUNTIME {
        return expand_runtime_cmd(pat_cstr, matches);
    }
    if xp.xp_context == EXPAND_PATTERN_IN_BUF {
        return expand_pattern_in_buf(pat_cstr, xp.xp_search_dir, matches);
    }

    let mut tofree: Option<Vec<u8>> = None;
    if xp.xp_context == EXPAND_USER_FUNC && pat_cstr.to_bytes().starts_with(b"^s:") {
        let len = pat_cstr.to_bytes().len() + 20;
        let mut buf = vec![0u8; len];
        format_to_buf!(
            &mut buf,
            c"^<SNR>\\d\\+_%s",
            crate::memory::CStrSubslice::advance(pat_cstr, 3)
        );
        tofree = Some(buf);
        pat_cstr = crate::memory::bytes_as_cstr(tofree.as_deref().unwrap());
    }

    if xp.xp_context == EXPAND_LUA {
        return nlua_expand_get_matches(matches);
    }

    if !fuzzy {
        regmatch.set_prog(vim_regcomp(
            pat_cstr,
            if opt_vars().p_magic.get() {
                RE_MAGIC
            } else {
                0
            },
        ));
        if regmatch.regprog.is_null() {
            return false;
        }
        regmatch.rm_ic = ignorecase(pat_cstr);
    }

    let ret = if xp.xp_context == EXPAND_SETTINGS || xp.xp_context == EXPAND_BOOL_SETTINGS {
        expand_settings(xp, &mut regmatch, Some(pat_cstr), matches, fuzzy)
    } else if xp.xp_context == EXPAND_STRING_SETTING {
        expand_string_setting(xp, &mut regmatch, matches)
    } else if xp.xp_context == EXPAND_SETTING_SUBTRACT {
        expand_setting_subtract(xp, &mut regmatch, matches)
    } else if xp.xp_context == EXPAND_MAPPINGS {
        expand_mappings(pat_cstr, &mut regmatch, matches)
    } else if xp.xp_context == EXPAND_ARGOPT {
        expand_argopt(pat_cstr, xp, &mut regmatch, matches)
    } else if xp.xp_context == EXPAND_USER_DEFINED {
        expand_user_defined(pat_cstr, xp, &mut regmatch, matches)
    } else {
        expand_other(pat_cstr, xp, &mut regmatch, matches)
    };

    if !fuzzy {
        regmatch.free_prog();
    }
    drop(tofree);

    ret
}

/// Expand a list of names.
pub fn expand_generic(
    pat: &CStr,
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<CString>,
    func: CompleteListItemGetter,
    escaped: bool,
) {
    let fuzzy = cmdline_fuzzy_complete(pat);
    matches.clear();

    let Some(func_fn) = func else {
        return;
    };

    let mut str_ga: Vec<CString> = Vec::new();
    let mut fuz_ga: Vec<FuzmatchStr> = Vec::new();
    let has_pat = !xp.xp_pattern.is_empty();
    let xp_context = xp.xp_context;

    let mut i: i32 = 0;
    while let Some(str_item) = func_fn(xp, i) {
        i += 1;
        if str_item.to_bytes().is_empty() {
            continue;
        }

        let match_ok: bool;
        let mut score: i32 = 0;
        if has_pat {
            if !fuzzy {
                match_ok = vim_regexec(regmatch, str_item, 0);
            } else {
                score = fuzzy_match_str(Some(str_item.to_bytes()), Some(pat.to_bytes()));
                match_ok = score != FUZZY_SCORE_NONE;
            }
        } else {
            match_ok = true;
        }

        if !match_ok {
            continue;
        }

        let mut item_cstr = if escaped {
            vim_strsave_escaped(str_item.to_bytes(), b" \t\\.")
        } else {
            str_item.to_owned()
        };

        if xp_context == EXPAND_MENUNAMES && item_cstr.as_bytes().last() == Some(&1) {
            let mut bytes = item_cstr.into_bytes();
            if let Some(last) = bytes.last_mut() {
                *last = b'.';
            }
            item_cstr = CString::new(bytes).unwrap_or_default();
        }

        if fuzzy {
            fuz_ga.push(FuzmatchStr {
                idx: fuz_ga.len() as i32,
                str_cstr: item_cstr,
                score,
            });
        } else {
            str_ga.push(item_cstr);
        }
    }

    if (!fuzzy && str_ga.is_empty()) || (fuzzy && fuz_ga.is_empty()) {
        return;
    }

    let sort_matches = !fuzzy
        && xp.xp_context != EXPAND_MENUNAMES
        && xp.xp_context != EXPAND_STRING_SETTING
        && xp.xp_context != EXPAND_MENUS
        && xp.xp_context != EXPAND_SCRIPTNAMES
        && xp.xp_context != EXPAND_ARGOPT;

    let funcsort = xp.xp_context == EXPAND_EXPRESSION
        || xp.xp_context == EXPAND_FUNCTIONS
        || xp.xp_context == EXPAND_USER_FUNC;

    if sort_matches {
        if funcsort {
            str_ga.sort_by(|a, b| sort_func_compare(a, b).cmp(&0));
        } else {
            sort_strings(&mut str_ga);
        }
    }

    if !fuzzy {
        *matches = str_ga;
    } else {
        fuzzymatches_slice_to_strmatches(&mut fuz_ga, matches, funcsort);
    }

    reset_expand_highlight();
}

/// Expand shell command matches in one directory of $PATH.
fn expand_shellcmd_onedir(
    pathed_pattern: &CStr,
    pathlen: usize,
    flags: i32,
    found_ht: &mut std::collections::HashSet<Vec<u8>>,
    gap: &mut Vec<CString>,
) {
    let mut dir_matches: Vec<std::ffi::CString> = Vec::new();
    if !expand_wildcards_vec(&[pathed_pattern], &mut dir_matches, flags) {
        return;
    }

    gap.reserve(dir_matches.len());

    for name in dir_matches {
        let name_bytes = name.as_bytes();
        let namelen = name_bytes.len();

        if namelen > pathlen {
            let tail_bytes = &name_bytes[pathlen..];
            if found_ht.insert(tail_bytes.to_vec()) {
                gap.push(bytes_to_cstring(tail_bytes));
            }
        }
    }
}

/// Complete a shell command.
fn expand_shellcmd(filepat: &CStr, matches: &mut Vec<CString>, flagsarg: i32) {
    matches.clear();
    let mut buf_vec = vec![0u8; MAXPATHL];
    let mut flags = flagsarg;
    let mut did_curdir = false;

    let mut patlen = filepat.to_bytes().len();
    let mut pat_vec = filepat.to_bytes_with_nul().to_vec();
    let mut idx = 0usize;
    while idx < patlen {
        if pat_vec[idx] == b'\\' && pat_vec.get(idx + 1) == Some(&b' ') {
            pat_vec.copy_within(idx + 1..=patlen, idx);
            patlen -= 1;
        }
        idx += 1;
    }
    let pat_bytes = &pat_vec[..patlen];

    flags |= EW_FILE | EW_EXEC | EW_SHELLCMD;

    let mut path_owned: Option<std::ffi::CString> = None;
    let path_cstr: &CStr = if pat_vec.first() == Some(&b'.')
        && (pat_vec.get(1).is_some_and(|&b| vim_ispathsep(i32::from(b)))
            || (pat_vec.get(1) == Some(&b'.')
                && pat_vec.get(2).is_some_and(|&b| vim_ispathsep(i32::from(b)))))
    {
        c"."
    } else {
        if !path_is_absolute(pat_bytes) {
            path_owned = vim_getenv(c"PATH");
        }
        path_owned.as_deref().unwrap_or(c"")
    };

    let mut found_ht: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let mut s = path_cstr;
    loop {
        let mut pathlen: usize;
        let seplen: usize;
        let mut e: &CStr;

        if s.to_bytes().is_empty() {
            if did_curdir {
                break;
            }

            did_curdir = true;
            flags |= EW_DIR;

            e = s;
            pathlen = 0;
            seplen = 0;
        } else {
            e = vim_strchr(s, ENV_SEPCHAR)
                .unwrap_or_else(|| crate::memory::CStrSubslice::advance(s, s.to_bytes().len()));

            pathlen = (e.as_ptr() as usize).saturating_sub(s.as_ptr() as usize);
            let s_seg = &s.to_bytes()[..pathlen];
            if s_seg == &b"."[..pathlen.min(1)] {
                did_curdir = true;
                flags |= EW_DIR;
            } else {
                flags &= !EW_DIR;
            }

            seplen = if !after_pathsep(s_seg) {
                PATHSEPSTR.len()
            } else {
                0
            };
        }

        if pathlen + seplen + patlen < MAXPATHL {
            if pathlen > 0 {
                buf_vec[..pathlen].copy_from_slice(&s.to_bytes()[..pathlen]);
                if seplen > 0 {
                    buf_vec[pathlen..pathlen + seplen].copy_from_slice(&PATHSEPSTR[..seplen]);
                    pathlen += seplen;
                }
            }
            buf_vec[pathlen..pathlen + patlen].copy_from_slice(&pat_vec[..patlen]);
            buf_vec[pathlen + patlen] = 0;

            expand_shellcmd_onedir(
                crate::memory::bytes_as_cstr(&buf_vec),
                pathlen,
                flags,
                &mut found_ht,
                matches,
            );
        }

        if !e.to_bytes().is_empty() {
            e = crate::memory::CStrSubslice::advance(e, 1);
        }
        s = e;
    }
}

/// Call "user_expand_func()" to invoke a user defined Vim script function.
fn call_user_expand_func<F, R>(user_expand_func: F, xp: &mut ExpandCtx, default: R) -> R
where
    F: FnOnce(&CStr, i32, &mut [TypVal; 4]) -> R,
{
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;
    let mut keep: u8 = 0;
    let mut args: [TypVal; 4] = [const { TypVal::new() }; 4];
    let save_current_sctx = g_st().current_sctx.get();

    let Some(arg_cstr) = xp.xp_arg.as_deref().filter(|s| !s.to_bytes().is_empty()) else {
        return default;
    };
    if xp.xp_line.is_null() {
        return default;
    }

    if !ccline.cmdbuff_vec.borrow().is_empty() {
        keep = ccline.cmdbuff_vec.borrow()[ccline.cmdlen.get() as usize];
        ccline.cmdbuff_vec.borrow_mut()[ccline.cmdlen.get() as usize] = 0;
    }

    let pat = bytes_to_cstring(xp.xp_pattern.slice(xp.xp_pattern_len));
    args[0].v_type = VarType::VarString;
    args[1].v_type = VarType::VarString;
    args[2].v_type = VarType::VarNumber;
    args[3].v_type = VarType::VarUnknown;
    args[0].vval = crate::eval::typval::TypvalVval::from_cstr(&pat);
    args[1].vval = crate::eval::typval::TypvalVval::from_opt_cstr(xp.xp_line.as_opt_cstr());
    args[2].vval = crate::eval::typval::TypvalVval::Number(i64::from(xp.xp_col));

    g_st().current_sctx.set(xp.xp_script_ctx);

    let ret = user_expand_func(arg_cstr, 3, &mut args);

    g_st().current_sctx.set(save_current_sctx);
    if !ccline.cmdbuff_vec.borrow().is_empty() {
        ccline.cmdbuff_vec.borrow_mut()[ccline.cmdlen.get() as usize] = keep;
    }

    ret
}

/// Expand names with a function defined by the user (EXPAND_USER_DEFINED).
fn expand_user_defined(
    pat: &CStr,
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<CString>,
) -> bool {
    let fuzzy = cmdline_fuzzy_complete(pat);
    matches.clear();

    let Some(retstr_cstr) = call_user_expand_func(
        |name, argc, argv| call_func_retstr(name, argc, argv),
        xp,
        None,
    ) else {
        return false;
    };
    let mut str_ga: Vec<CString> = Vec::new();
    let mut fuz_ga: Vec<FuzmatchStr> = Vec::new();

    let mut rem = retstr_cstr.to_bytes();
    while !rem.is_empty() {
        let line_len = rem.iter().position(|&b| b == b'\n').unwrap_or(rem.len());
        let line_cstr = bytes_to_cstring(&rem[..line_len]);

        let match_ok: bool;
        let mut score: i32 = 0;
        if !xp.xp_pattern.is_empty() {
            if !fuzzy {
                match_ok = vim_regexec(regmatch, &line_cstr, 0);
            } else {
                score = fuzzy_match_str(Some(line_cstr.to_bytes()), Some(pat.to_bytes()));
                match_ok = score != FUZZY_SCORE_NONE;
            }
        } else {
            match_ok = true;
        }

        if match_ok {
            if !fuzzy {
                str_ga.push(line_cstr);
            } else {
                fuz_ga.push(FuzmatchStr {
                    idx: fuz_ga.len() as i32,
                    str_cstr: line_cstr,
                    score,
                });
            }
        }

        rem = &rem[(line_len + 1).min(rem.len())..];
    }

    if (!fuzzy && str_ga.is_empty()) || (fuzzy && fuz_ga.is_empty()) {
        return true;
    }

    if !fuzzy {
        *matches = str_ga;
    } else {
        fuzzymatches_slice_to_strmatches(&mut fuz_ga, matches, false);
    }
    true
}

pub fn expand_process_user_list(
    retlist: Option<&TvList>,
    matches: &mut Vec<CString>,
    xp: &mut ExpandCtx,
) {
    matches.clear();
    let mut ga_abbr: Vec<Option<std::ffi::CString>> = Vec::new();
    let mut ga_kind: Vec<Option<std::ffi::CString>> = Vec::new();
    let mut ga_menu: Vec<Option<std::ffi::CString>> = Vec::new();
    let mut ga_info: Vec<Option<std::ffi::CString>> = Vec::new();
    let mut have_extra = false;

    if let Some(retlist) = retlist {
        for li in crate::eval::typval::list_iter_ptr(retlist.first_opt()) {
            let tv = li.tv();
            let p: CString;
            let mut abbr: Option<std::ffi::CString> = None;
            let mut kind: Option<std::ffi::CString> = None;
            let mut menu: Option<std::ffi::CString> = None;
            let mut info: Option<std::ffi::CString> = None;

            if tv.v_type == VarType::VarString {
                if tv.vval.v_string_is_null() {
                    continue;
                }
                p = tv.vval.v_string_owned();
            } else if tv.v_type == VarType::VarDict && !tv.vval.v_dict().is_null() {
                let d = tv.vval.v_dict_ref();
                let Some(word) = tv_dict_get_string(Some(d), c"word") else {
                    continue;
                };
                p = word.into_owned();
                abbr = crate::eval::typval::tv_dict_get_cstring(Some(d), c"abbr");
                kind = crate::eval::typval::tv_dict_get_cstring(Some(d), c"kind");
                menu = crate::eval::typval::tv_dict_get_cstring(Some(d), c"menu");
                info = crate::eval::typval::tv_dict_get_cstring(Some(d), c"info");
                if abbr.is_some() || kind.is_some() || menu.is_some() || info.is_some() {
                    have_extra = true;
                }
            } else {
                continue;
            }

            matches.push(p);
            ga_abbr.push(abbr);
            ga_kind.push(kind);
            ga_menu.push(menu);
            ga_info.push(info);
        }
    }

    if have_extra && !matches.is_empty() {
        xp.xp_files_abbr = ga_abbr;
        xp.xp_files_kind = ga_kind;
        xp.xp_files_menu = ga_menu;
        xp.xp_files_info = ga_info;
    }
}

/// Expand names with a list returned by a function defined by the user.
fn expand_user_list(xp: &mut ExpandCtx, matches: &mut Vec<CString>) -> bool {
    matches.clear();
    let Some(retlist) = call_user_expand_func(
        |name, argc, argv| call_func_retlist(name, argc, argv),
        xp,
        None,
    ) else {
        return false;
    };

    expand_process_user_list(Some(&retlist), matches, xp);
    tv_list_unref(Some(&retlist));
    true
}

fn expand_user_lua(xp: &mut ExpandCtx, matches: &mut Vec<CString>) -> bool {
    let mut rettv: TypVal = TypVal::new();
    nlua_call_user_expand_func(xp, &mut rettv);
    if rettv.v_type != VarType::VarList {
        tv_clear(&mut rettv);
        return false;
    }

    let retlist = rettv.vval.v_list();
    expand_process_user_list(retlist.opt(), matches, xp);
    tv_list_unref(retlist.opt());
    true
}

/// Expand `file` for all comma-separated directories in `path`.
pub fn globpath(
    path_arg: &CStr,
    file_arg: &CStr,
    ga: &mut Vec<CString>,
    expand_options: i32,
    dirs: bool,
) {
    let file_bytes = file_arg.to_bytes();
    let filelen = file_bytes.len();
    let mut buf_vec = vec![0u8; MAXPATHL];

    let mut xpc: ExpandCtx = ExpandCtx::new();
    xpc.xp_context = if dirs {
        EXPAND_DIRECTORIES
    } else {
        EXPAND_FILES
    };

    let mut path_bytes = path_arg.to_bytes();
    while !path_bytes.is_empty() {
        let mut pathlen = copy_option_part(&mut path_bytes, &mut buf_vec[..MAXPATHL], c",");
        let seplen = if buf_vec[0] != 0 && !after_pathsep(&buf_vec[..pathlen]) {
            PATHSEPSTR.len()
        } else {
            0
        };

        if pathlen + seplen + filelen < MAXPATHL {
            if seplen > 0 {
                buf_vec[pathlen..pathlen + seplen].copy_from_slice(&PATHSEPSTR[..seplen]);
                pathlen += seplen;
            }
            buf_vec[pathlen..pathlen + filelen].copy_from_slice(file_bytes);
            buf_vec[pathlen + filelen] = 0;

            let mut p: Vec<CString> = Vec::new();
            let pat_cstr = crate::memory::bytes_as_cstr(&buf_vec);
            expand_from_context(&mut xpc, pat_cstr, &mut p, WILD_SILENT | expand_options);
            if !p.is_empty() {
                expand_escape(
                    &mut xpc,
                    pat_cstr.to_bytes(),
                    &mut p,
                    WILD_SILENT | expand_options,
                );
                ga.extend(p);
            }
        }
    }
}

/// Translate some keys pressed when 'wildmenu' is used.
pub fn wildmenu_translate_key(
    cclp: &CmdlineInfo,
    key: i32,
    xp: &mut ExpandCtx,
    did_wild_list: bool,
) -> i32 {
    let mut c = key;

    if cmdline_pum_active() || did_wild_list || g_st().wild_menu_showing.get() != 0 {
        if c == K_LEFT {
            c = CTRL_P;
        } else if c == K_RIGHT {
            c = CTRL_N;
        }
    }

    if xp.xp_context == EXPAND_MENUNAMES
        && cclp.cmdpos.get() > 1
        && cclp.cmdbuff_vec.borrow()[(cclp.cmdpos.get() - 1) as usize] == b'.'
        && cclp.cmdbuff_vec.borrow()[(cclp.cmdpos.get() - 2) as usize] != b'\\'
        && (c == b'\n' as i32 || c == b'\r' as i32 || c == K_KENTER)
    {
        c = K_DOWN;
    }

    c
}

/// Delete characters on the command line, from "from" to the current position.
fn cmdline_del(cclp: &CmdlineInfo, from: i32) {
    debug_assert!(cclp.cmdpos.get() <= cclp.cmdlen.get());

    cclp.cmdbuff_vec.borrow_mut().copy_within(
        cclp.cmdpos.get() as usize..=cclp.cmdlen.get() as usize,
        from as usize,
    );
    cclp.cmdlen
        .set(cclp.cmdlen.get() - (cclp.cmdpos.get() - from));
    cclp.cmdpos.set(from);
}

/// Handle a key pressed when the wild menu for the menu names (EXPAND_MENUNAMES) is displayed.
fn wildmenu_process_key_menunames(cclp: &CmdlineInfo, mut key: i32, xp: &mut ExpandCtx) -> i32 {
    let cmd_vec = cclp.cmdbuff_vec.borrow();
    let cmd_slice = &cmd_vec[..cclp.cmdlen.get() as usize];
    if key == K_DOWN
        && cclp.cmdpos.get() > 0
        && cmd_slice.get((cclp.cmdpos.get() - 1) as usize) == Some(&b'.')
    {
        key = opt_vars().p_wc.get() as i32;
        g_st().key_typed.set(true);
    } else if key == K_UP {
        let mut found = false;

        let mut j = ((xp.xp_pattern).offset_from(cclp.cmdbuff()) as isize) as i32;
        let mut i: i32 = 0;
        while {
            j -= 1;
            j > 0
        } {
            if cmd_slice[j as usize] == b' ' && cmd_slice[(j - 1) as usize] != b'\\' {
                i = j + 1;
                break;
            }
            if cmd_slice[j as usize] == b'.' && cmd_slice[(j - 1) as usize] != b'\\' {
                if found {
                    i = j + 1;
                    break;
                } else {
                    found = true;
                }
            }
        }
        drop(cmd_vec);
        if i > 0 {
            cmdline_del(cclp, i);
        }
        key = opt_vars().p_wc.get() as i32;
        g_st().key_typed.set(true);
        xp.xp_context = EXPAND_NOTHING;
    }

    key
}

/// Handle a key pressed when the wild menu for file/dir/shellcmd names is displayed.
fn wildmenu_process_key_filenames(cclp: &CmdlineInfo, mut key: i32, xp: &mut ExpandCtx) -> i32 {
    let upseg: [u8; 5] = [PATHSEP, b'.', b'.', PATHSEP, 0];
    let cmd_vec = cclp.cmdbuff_vec.borrow();
    let cmd_slice = &cmd_vec[..=cclp.cmdlen.get() as usize];

    if key == K_DOWN
        && cclp.cmdpos.get() > 0
        && cmd_slice[(cclp.cmdpos.get() - 1) as usize] == PATHSEP
        && (cclp.cmdpos.get() < 3
            || cmd_slice[(cclp.cmdpos.get() - 2) as usize] != b'.'
            || cmd_slice[(cclp.cmdpos.get() - 3) as usize] != b'.')
    {
        key = opt_vars().p_wc.get() as i32;
        g_st().key_typed.set(true);
    } else if xp.xp_pattern.as_bytes().starts_with(&upseg[1..4]) && key == K_DOWN {
        let mut found = false;

        let mut j = cclp.cmdpos.get();
        let i = ((xp.xp_pattern).offset_from(cclp.cmdbuff()) as isize) as i32;
        while {
            j -= 1;
            j > i
        } {
            j -= crate::mbyte::utf_head_off(&cmd_vec, j as usize);
            if vim_ispathsep(cmd_slice[j as usize]) {
                found = true;
                break;
            }
        }
        if found
            && cmd_slice[(j - 1) as usize] == b'.'
            && cmd_slice[(j - 2) as usize] == b'.'
            && (vim_ispathsep(cmd_slice[(j - 3) as usize]) || j == i + 2)
        {
            drop(cmd_vec);
            cmdline_del(cclp, j - 2);
            key = opt_vars().p_wc.get() as i32;
            g_st().key_typed.set(true);
        }
    } else if key == K_UP {
        let mut found = false;

        let mut j = cclp.cmdpos.get() - 1;
        let mut i = ((xp.xp_pattern).offset_from(cclp.cmdbuff()) as isize) as i32;
        while {
            j -= 1;
            j > i
        } {
            j -= crate::mbyte::utf_head_off(&cmd_vec, j as usize);
            #[cfg(windows)]
            let is_sep = vim_ispathsep(cmd_slice[j as usize])
                && !b" *?[{`$%#".contains(&cmd_slice[(j + 1) as usize]);
            #[cfg(not(windows))]
            let is_sep = vim_ispathsep(cmd_slice[j as usize]);

            if is_sep {
                if found {
                    i = j + 1;
                    break;
                } else {
                    found = true;
                }
            }
        }

        if !found {
            j = i;
        } else if cmd_slice
            .get(j as usize..)
            .is_some_and(|s| s.starts_with(&upseg[..4]))
        {
            j += 4;
        } else if cmd_slice
            .get(j as usize..)
            .is_some_and(|s| s.starts_with(&upseg[1..4]))
            && j == i
        {
            j += 3;
        } else {
            j = 0;
        }

        drop(cmd_vec);
        if j > 0 {
            cmdline_del(cclp, j);
            put_on_cmdline(&upseg[1..], 3, false);
        } else if cclp.cmdpos.get() > i {
            cmdline_del(cclp, i);
        }

        key = opt_vars().p_wc.get() as i32;
        g_st().key_typed.set(true);
    }

    key
}

/// Handle a key pressed when wild menu is displayed.
pub fn wildmenu_process_key(cclp: &CmdlineInfo, key: i32, xp: &mut ExpandCtx) -> i32 {
    if xp.xp_context == EXPAND_MENUNAMES {
        return wildmenu_process_key_menunames(cclp, key, xp);
    }
    if xp.xp_context == EXPAND_FILES
        || xp.xp_context == EXPAND_DIRECTORIES
        || xp.xp_context == EXPAND_SHELLCMD
    {
        return wildmenu_process_key_filenames(cclp, key, xp);
    }

    key
}

/// Free expanded names when finished walking through the matches.
pub fn wildmenu_cleanup(cclp: &CmdlineInfo) {
    if !opt_vars().p_wmnu.get() || g_st().wild_menu_showing.get() == 0 {
        return;
    }

    let skt = g_st().key_typed.get();
    let old_redrawing_disabled = g_st().redrawing_disabled.get();

    if cclp.input_fn.get() != 0 {
        g_st().redrawing_disabled.set(0);
    }

    set_no_hlsearch(true);

    if g_st().wild_menu_showing.get() == WM_SCROLLED {
        g_st().cmdline_row.set(g_st().cmdline_row.get() - (1));
        redrawcmd();
        g_st().wild_menu_showing.set(0);
    } else if g_st().save_p_ls.get() != -1 {
        opt_vars().p_ls.set(OptInt::from(g_st().save_p_ls.get()));
        opt_vars().p_wmh.set(OptInt::from(g_st().save_p_wmh.get()));
        last_status(false);
        update_screen();
        redrawcmd();
        g_st().save_p_ls.set(-1);
        g_st().wild_menu_showing.set(0);
    } else {
        win_redraw_last_status(&topframe());
        g_st().wild_menu_showing.set(0);
        redraw_statuslines();
    }
    g_st().key_typed.set(skt);
    if cclp.input_fn.get() != 0 {
        g_st().redrawing_disabled.set(old_redrawing_disabled);
    }
}

/// "getcompletion()" function.
pub fn f_getcompletion(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut xpc: ExpandCtx = ExpandCtx::new();
    let mut filtered = false;
    let mut options = WILD_SILENT | WILD_USE_NL | WILD_ADD_SLASH | WILD_NO_BEEP | WILD_HOME_REPLACE;

    if !tv_check_for_string_arg(argvars, 1) {
        return;
    }
    let type_str = tv_get_string(&argvars[1]);

    if argvars[2].v_type != VarType::VarUnknown {
        filtered = tv_get_bool_chk(&argvars[2], None) != 0;
    }

    if opt_vars().p_wic.get() {
        options |= WILD_ICASE;
    }

    if !filtered {
        options |= WILD_KEEP_ALL;
    }

    if argvars[0].v_type != VarType::VarString {
        if tv_get_string_chk(&argvars[0]).is_some() {
            emsg(gettext(E_INVARG));
        }
        return;
    }
    let pattern_cstr = tv_get_string(&argvars[0]);
    let pattern = pattern_cstr.as_ptr();
    let mut pattern_start = pattern as usize;

    if type_str.to_bytes() == b"cmdline" {
        let cmdline_len = pattern_cstr.to_bytes().len() as i32;
        set_cmd_context(
            &mut xpc,
            pattern_cstr.to_bytes_with_nul(),
            cmdline_len,
            cmdline_len,
            false,
        );
        pattern_start = xpc.xp_pattern.addr();
        xpc.xp_pattern_len = (xpc.xp_pattern).len();
        xpc.xp_col = cmdline_len;
    } else {
        let pat_slice = crate::strings::OwnedSlice::from_cstr(&pattern_cstr);
        xpc.xp_pattern_len = pat_slice.len();
        xpc.xp_pattern = pat_slice.clone();
        xpc.xp_line = pat_slice;

        xpc.xp_context = cmdcomplete_str_to_type(type_str.to_bytes());
        match xpc.xp_context {
            EXPAND_NOTHING => {
                semsg!(gettext(E_INVARG2), type_str);
                return;
            }
            EXPAND_USER_DEFINED => {
                if !type_str.to_bytes().starts_with(b"custom,") {
                    semsg!(gettext(E_INVARG2), type_str);
                    return;
                }
                xpc.xp_arg = Some(type_str[7..].to_owned());
            }
            EXPAND_USER_LIST => {
                if !type_str.to_bytes().starts_with(b"customlist,") {
                    semsg!(gettext(E_INVARG2), type_str);
                    return;
                }
                xpc.xp_arg = Some(type_str[11..].to_owned());
            }
            EXPAND_MENUS => {
                set_context_in_menu_cmd(&mut xpc, c"menu", &pattern_cstr, false);
                xpc.xp_pattern_len -=
                    ((xpc.xp_pattern).offset_from(pattern_start) as isize) as usize;
            }
            EXPAND_SIGN => {
                set_context_in_sign_cmd(&mut xpc, &pattern_cstr);
                xpc.xp_pattern_len -=
                    ((xpc.xp_pattern).offset_from(pattern_start) as isize) as usize;
            }
            EXPAND_RUNTIME => {
                set_context_in_runtime_cmd(&mut xpc, &pattern_cstr);
                xpc.xp_pattern_len -=
                    ((xpc.xp_pattern).offset_from(pattern_start) as isize) as usize;
            }
            EXPAND_SHELLCMDLINE => {
                let mut context = EXPAND_SHELLCMDLINE;
                set_context_for_wildcard_arg(None, &pattern_cstr, false, &mut xpc, &mut context);
                xpc.xp_pattern_len -=
                    ((xpc.xp_pattern).offset_from(pattern_start) as isize) as usize;
            }
            EXPAND_FILETYPECMD => {
                CMDEXP_STATE.with(|s| s.filetype_expand_what.set(FiletypeExpandWhat::All));
            }
            EXPAND_BREAKPOINT => {
                CMDEXP_STATE.with(|s| s.breakpt_expand_what.set(BreakptExpandWhat::Add));
            }
            _ => {}
        }
    }

    if xpc.xp_context == EXPAND_LUA {
        xpc.xp_col = (xpc.xp_line).len() as i32;
        nlua_expand_pat(&mut xpc);
        xpc.xp_pattern_len -= ((xpc.xp_pattern).offset_from(pattern_start) as isize) as usize;
    }
    let pat = if cmdline_fuzzy_completion_supported(&xpc) {
        bytes_to_cstring(xpc.xp_pattern.slice(xpc.xp_pattern_len))
    } else {
        addstar((xpc.xp_pattern).slice(xpc.xp_pattern_len), xpc.xp_context)
    };

    expand_one(&mut xpc, Some(pat.as_c_str()), None, options, WILD_ALL_KEEP);
    tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);

    for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
        tv_list_append_string(
            rettv.vval.v_list_ref(),
            Some(f.as_c_str()),
            f.as_bytes().len() as isize,
        );
    }
}

/// "getcompletiontype()" function.
pub fn f_getcompletiontype(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::NULL_STRING;

    if !tv_check_for_string_arg(argvars, 0) {
        return;
    }

    let pat = tv_get_string(&argvars[0]);
    let mut xpc: ExpandCtx = ExpandCtx::new();

    let pat_bytes = pat.to_bytes_with_nul();
    let cmdline_len = (pat_bytes.len() - 1) as i32;
    set_cmd_context(&mut xpc, pat_bytes, cmdline_len, cmdline_len, false);
    crate::eval::typval::tv_set_string(
        rettv,
        cmdcomplete_type_to_str(xpc.xp_context, xpc.xp_arg.as_deref().map(CStr::to_bytes))
            .as_deref(),
    );
}

/// "cmdcomplete_info()" function.
pub fn f_cmdcomplete_info(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let ccline_rc = get_cmdline_info();
    let ccline = &*ccline_rc;

    tv_dict_alloc_ret(rettv);
    let xpc_borrow = ccline.xpc.borrow();
    let Some(xpc) = xpc_borrow.as_ref() else {
        return;
    };
    if xpc.xp_files.capacity() == 0 {
        return;
    }
    let retdict = rettv.vval.v_dict();
    let mut ret = CMDLINE_ORIG.with(|cell| {
        let orig = cell.borrow();
        tv_dict_add_str_len(&retdict, c"cmdline_orig", orig.as_deref())
    });
    if ret {
        ret = tv_dict_add_nr(&retdict, c"pum_visible", i64::from(pum_visible()));
    }
    if ret {
        ret = tv_dict_add_nr(&retdict, c"selected", i64::from(xpc.xp_selected));
    }
    if ret {
        let li = tv_list_alloc(xpc.xp_numfiles as isize);

        ret = tv_dict_add_list(&retdict, c"matches", Some(&li));

        let files = xpc.xp_files.as_slice();
        let mut idx: usize = 0;
        while ret && (idx as i32) < xpc.xp_numfiles {
            tv_list_append_string(
                &li,
                Some(files[idx].as_c_str()),
                files[idx].as_bytes().len() as isize,
            );
            idx += 1;
        }
    }
}

/// Copy a substring from the current buffer (curbuf).
fn copy_substring_from_pos(
    start: &mut CursorPos,
    end: &mut CursorPos,
    match_end: &mut CursorPos,
) -> Option<CString> {
    let exacttext = (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_EXACTTEXT) != 0;

    if start.lnum > end.lnum || (start.lnum == end.lnum && start.col >= end.col) {
        return None;
    }

    let mut ga: Vec<u8> = Vec::with_capacity(128);

    let start_line = crate::memline::ml_get_slice(start.lnum);
    let start_col = (start.col as usize).min(start_line.len());
    let is_single_line = start.lnum == end.lnum;

    let mut segment_len = if is_single_line {
        end.col - start.col
    } else {
        ml_get_len(start.lnum) - start.col
    };
    let seg_end = (start_col + segment_len.max(0) as usize).min(start_line.len());
    ga.extend_from_slice(&start_line[start_col..seg_end]);
    if !is_single_line {
        if exacttext {
            ga.extend_from_slice(b"\\n");
        } else {
            ga.push(b'\n');
        }
    }

    if !is_single_line {
        let mut lnum = start.lnum + 1;
        while lnum < end.lnum {
            let line = crate::memline::ml_get_slice(lnum);
            ga.extend_from_slice(&line);
            if exacttext {
                ga.extend_from_slice(b"\\n");
            } else {
                ga.push(b'\n');
            }
            lnum += 1;
        }
    }

    let end_line_slice = crate::memline::ml_get_slice(end.lnum);
    let end_col_u = (end.col.max(0) as usize).min(end_line_slice.len());
    let word_end_tail = find_word_end(&end_line_slice[end_col_u..]);
    segment_len = (end_line_slice.len() - word_end_tail.len()) as i32;
    let start_off = if is_single_line { end.col } else { 0 };
    let start_u = (start_off.max(0) as usize).min(end_line_slice.len());
    ga.extend_from_slice(
        &end_line_slice[start_u..(segment_len as usize).min(end_line_slice.len())],
    );

    match_end.lnum = end.lnum;
    match_end.col = segment_len;

    Some(bytes_to_cstring(&ga))
}

/// Returns true if the given string `str` matches the regex pattern `pat`.
fn is_regex_match(pat: &CStr, str_arg: &CStr) -> bool {
    if pat.to_bytes() == str_arg.to_bytes() {
        return true;
    }

    let mut regmatch: Regmatch = Regmatch::new();

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    g_st().msg_silent.set(g_st().msg_silent.get() + (1));
    regmatch.set_prog(vim_regcomp(pat, RE_MAGIC + RE_STRING));
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    g_st().msg_silent.set(g_st().msg_silent.get() - (1));

    if regmatch.regprog.is_null() {
        return false;
    }
    regmatch.rm_ic = opt_vars().p_ic.get();
    if opt_vars().p_ic.get() && opt_vars().p_scs.get() {
        regmatch.rm_ic = !pat_has_uppercase(pat);
    }

    g_st().emsg_off.set(g_st().emsg_off.get() + (1));
    g_st().msg_silent.set(g_st().msg_silent.get() + (1));
    let result = vim_regexec_nl(&mut regmatch, Some(str_arg), 0);
    g_st().emsg_off.set(g_st().emsg_off.get() - (1));
    g_st().msg_silent.set(g_st().msg_silent.get() - (1));

    regmatch.free_prog();
    result
}

/// Constructs a new match string by appending text from the buffer to `pat`.
fn concat_pattern_with_buffer_match(
    pat: &[u8],
    end_match_pos: &mut CursorPos,
    lowercase: bool,
) -> CString {
    let line = crate::memline::ml_get_slice(end_match_pos.lnum);
    let col_u = (end_match_pos.col as usize).min(line.len());
    let from_col = &line[col_u..];
    let word_end_tail = find_word_end(from_col);
    let match_len = from_col.len() - word_end_tail.len();
    let mut out = Vec::with_capacity(match_len + pat.len());
    out.extend_from_slice(pat);
    if match_len > 0 {
        let match_slice = &line[col_u..(col_u + match_len).min(line.len())];
        if lowercase {
            let lower = strcase_save(match_slice, false);
            let lower_bytes = lower.to_bytes();
            out.extend_from_slice(&lower_bytes[..lower_bytes.len().min(match_len)]);
        } else {
            out.extend_from_slice(match_slice);
        }
    }
    bytes_to_cstring(&out)
}

/// Search for strings matching "pat" in the specified range and return them.
fn expand_pattern_in_buf(pat: &CStr, dir: Direction, matches: &mut Vec<CString>) -> bool {
    let exacttext = (opt_vars().wop_flags.get() & K_OPT_WOP_FLAG_EXACTTEXT) != 0;
    let has_range = g_st().search_global.first_line.get() != 0;

    matches.clear();

    if pat.to_bytes().is_empty() {
        return false;
    }

    let pat_len = pat.to_bytes().len() as i32;
    let mut cur_match_pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };
    let mut prev_match_pos = CursorPos {
        lnum: 0,
        col: 0,
        coladd: 0,
    };
    if has_range {
        cur_match_pos.lnum = g_st().search_global.first_line.get();
    } else {
        cur_match_pos = CMDEXP_STATE.with(|s| s.pre_incsearch_pos.get());
    }

    let search_flags = SEARCH_OPT
        | SEARCH_NOOF
        | SEARCH_PEEK
        | SEARCH_NFMSG
        | if has_range { SEARCH_START } else { 0 };

    let mut end_match_pos: CursorPos = CursorPos::new();
    let mut word_end_pos: CursorPos = CursorPos::new();
    let mut looped_around = false;
    let mut compl_started = false;

    loop {
        g_st().emsg_off.set(g_st().emsg_off.get() + (1));
        g_st().msg_silent.set(g_st().msg_silent.get() + (1));
        let found_new_match = searchit(
            None,
            &curbuf(),
            &mut cur_match_pos,
            Some(&mut end_match_pos),
            dir,
            pat,
            pat_len as usize,
            1,
            search_flags,
            RE_LAST,
            opt_vars().p_magic.get(),
            None,
        );
        g_st().msg_silent.set(g_st().msg_silent.get() - (1));
        g_st().emsg_off.set(g_st().emsg_off.get() - (1));

        if found_new_match == 0 {
            break;
        }

        if has_range
            && (cur_match_pos.lnum < g_st().search_global.first_line.get()
                || cur_match_pos.lnum > g_st().search_global.last_line.get())
        {
            break;
        }

        if compl_started
            && ((dir == FORWARD && ltoreq(cur_match_pos, prev_match_pos))
                || (dir == BACKWARD && ltoreq(prev_match_pos, cur_match_pos)))
        {
            if looped_around {
                break;
            } else {
                looped_around = true;
            }
        }

        compl_started = true;
        prev_match_pos = cur_match_pos;

        if char_avail() || g_st().got_int.get() {
            if g_st().got_int.get() {
                let _ = vpeekc();
                g_st().got_int.set(false);
            }
            matches.clear();
            return false;
        }

        if end_match_pos.lnum > curbuf().b_ml.ml_line_count.get() {
            cur_match_pos.lnum = 1;
            cur_match_pos.col = 0;
            cur_match_pos.coladd = 0;
            continue;
        }

        let Some(full_match) =
            copy_substring_from_pos(&mut cur_match_pos, &mut end_match_pos, &mut word_end_pos)
        else {
            break;
        };

        let match_cstr = if exacttext {
            full_match
        } else {
            let pat_slice = pat.to_bytes();
            let mut m = concat_pattern_with_buffer_match(pat_slice, &mut end_match_pos, false);

            if !is_regex_match(m.as_c_str(), full_match.as_c_str()) {
                m = concat_pattern_with_buffer_match(pat_slice, &mut end_match_pos, true);
                if !is_regex_match(m.as_c_str(), full_match.as_c_str()) {
                    continue;
                }
            }
            m
        };

        if !matches.contains(&match_cstr) {
            matches.push(match_cstr);
            if matches.len() as i32 > TAG_MANY {
                break;
            }
        }
        if has_range {
            cur_match_pos = word_end_pos;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_init_and_cleanup_work() {
        let mut xp: ExpandCtx = ExpandCtx::new();
        expand_init(&mut xp);
        assert_eq!(xp.xp_context, EXPAND_NOTHING);
        assert_eq!(xp.xp_backslash, XP_BS_NONE);
        expand_cleanup(&mut xp);
        assert!(!cmdline_pum_active());
    }

    #[test]
    fn cmdidx_from_raw_roundtrips_all_variants() {
        for (i, &c) in Cmdidx::ALL.iter().enumerate() {
            assert_eq!(c as i32, i as i32);
            assert!(Cmdidx::from_raw(i as i32) == c);
        }
        assert!(Cmdidx::from_raw(-1) == Cmdidx::CmdUser);
        assert!(Cmdidx::from_raw(-2) == Cmdidx::CmdUserBuf);
        assert!(Cmdidx::from_raw(Cmdidx::CmdSize as i32) == Cmdidx::CmdSize);
    }
}
