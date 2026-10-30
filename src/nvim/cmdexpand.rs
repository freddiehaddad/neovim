//! Functions for command-line completion (`src/nvim/cmdexpand.c`).

use crate::api::command::is_user_cmdidx;
use crate::api::private::helpers::{api_clear_error, api_free_object, cstr_as_string};
use crate::api::private::validate::{Array, Error, NvimString, Object, ObjectData, ObjectType};
use crate::arglist::get_arglist_name;
use crate::autocmd::{
    HLF_T, aucmd_set_expand_context, expand_get_augroup_name, expand_get_event_name,
};
use crate::buffer::{K_UIMESSAGES, expand_bufnames};
use crate::charset::{
    CTRL_V_CONST, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_isspace, ascii_isupper,
    ascii_iswhite, backslash_halve_save, rem_backslash, skipdigits, skiptowhite, skipwhite,
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
use crate::math::FAIL;
use crate::mbyte::{mb_tolower, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memline::{ml_get, ml_get_len};
use crate::memory::{cstr_from_bytes, cstr_to_cstring};
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
    vim_regfree,
};
use crate::runtime::{
    DIP_OPT, DIP_START, ExpandPackAddDir, expand_rtdir, expand_runtime_cmd, script_id_valid,
    script_item, set_context_in_runtime_cmd,
};
use crate::search::{Direction, ignorecase, pat_has_uppercase, searchit};
use crate::sign::{get_sign_name, set_context_in_sign_cmd};
use crate::statusline::fillchar_status;
use crate::strings::{
    gettext, sort_strings, strcase_save, vim_snprintf, vim_strchr, vim_strsave_escaped,
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
use std::ffi::{CStr, CString, c_char};
use std::ptr;

use crate::api::win_config::UIExtension;
use crate::lua::converter::LuaRef;
use crate::runtime::Sctx;

pub(crate) const EXPAND_BUF_LEN: usize = 256;

#[repr(C)]
pub struct ExpandCtx {
    pub xp_pattern: *mut c_char,
    pub xp_context: i32,
    pub xp_pattern_len: usize,
    pub xp_prefix: i32,
    pub xp_arg: *mut c_char,
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
    pub xp_line: *mut c_char,
    pub xp_buf: [u8; EXPAND_BUF_LEN],
    pub xp_search_dir: i32,
    pub xp_pre_incsearch_pos: CursorPos,
}

impl ExpandCtx {
    pub const fn new() -> Self {
        Self {
            xp_pattern: ptr::null_mut(),
            xp_context: 0,
            xp_pattern_len: 0,
            xp_prefix: XP_PREFIX_NONE,
            xp_arg: ptr::null_mut(),
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
            xp_line: ptr::null_mut(),
            xp_buf: [0; EXPAND_BUF_LEN],
            xp_search_dir: 0,
            xp_pre_incsearch_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
        }
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
pub type CompleteListItemGetter = Option<fn(&mut ExpandCtx, i32) -> *mut c_char>;
#[derive(Clone, Copy)]
pub enum LineGetter {
    None,
    ExLine,
    CmdKeyCmd,
    AutoCmd(*mut crate::autocmd::AutoPatCmd),
    Func(*mut crate::profile::Funccall),
    Source(*mut crate::runtime::SourceCookie),
    Loop(*mut crate::ex_docmd::LoopCookie),
    List(*mut crate::eval::funcs::GetListLineCookie),
    LuaEx(*mut crate::lua::executor::ExDocmdCookie),
}

impl LineGetter {
    #[inline]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    #[inline]
    pub fn inner(self) -> Self {
        let mut gp = self;
        while let Self::Loop(cp) = gp {
            gp = unsafe { (*cp).lc_getline };
        }
        gp
    }

    #[inline]
    pub fn is_exline(self) -> bool {
        matches!(self.inner(), Self::ExLine)
    }

    #[inline]
    pub fn is_autocmd(self) -> bool {
        matches!(self.inner(), Self::AutoCmd(_))
    }

    #[inline]
    pub fn is_func(self) -> bool {
        matches!(self.inner(), Self::Func(_))
    }

    #[inline]
    pub fn is_source(self) -> bool {
        matches!(self.inner(), Self::Source(_))
    }

    #[inline]
    pub fn as_func(self) -> Option<*mut crate::profile::Funccall> {
        match self.inner() {
            Self::Func(p) => Some(p),
            _ => None,
        }
    }

    #[inline]
    pub fn as_source(self) -> Option<*mut crate::runtime::SourceCookie> {
        match self.inner() {
            Self::Source(p) => Some(p),
            _ => None,
        }
    }

    pub fn call(self, c: i32, indent: i32, do_concat: bool) -> Option<std::ffi::CString> {
        match self {
            Self::None => None,
            Self::ExLine => crate::ex_getln::getexline(c, indent, do_concat),
            Self::CmdKeyCmd => crate::input::getcmdkeycmd(c, indent, do_concat),
            Self::AutoCmd(cookie) => crate::autocmd::getnextac(c, cookie, indent, do_concat),
            Self::Func(cookie) => {
                crate::eval::userfunc::get_func_line(c, unsafe { &mut *cookie }, indent, do_concat)
            }
            Self::Source(cookie) => {
                crate::runtime::getsourceline(c, unsafe { &mut *cookie }, indent, do_concat)
            }
            Self::Loop(cookie) => {
                crate::ex_docmd::get_loop_line(c, unsafe { &mut *cookie }, indent, do_concat)
            }
            Self::List(cookie) => {
                crate::eval::funcs::get_list_line(c, unsafe { &mut *cookie }, indent, do_concat)
            }
            Self::LuaEx(cookie) => {
                crate::lua::executor::nlua_ex_getline(c, unsafe { &mut *cookie }, indent, do_concat)
            }
        }
    }
}
pub type CmdAddr = i32;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cmdidx {
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
use crate::charset::NUL;
#[cfg(unix)]
use crate::ex_getln::VSE_SHELL;
use crate::option::{OPT_GLOBAL, OPT_LOCAL};
use crate::os::fs::MAXPATHL;
use crate::search::{BACKWARD, FORWARD};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CmdlineColorChunk {
    pub start: i32,
    pub end: i32,
    pub hl_id: i32,
}

pub type CmdlineColors = Vec<CmdlineColorChunk>;

#[repr(C)]
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

#[repr(C)]
#[derive(Clone)]
pub struct CmdlineInfo {
    pub cmdbuff: *mut c_char,
    pub cmdbuff_vec: Vec<c_char>,
    pub cmdbufflen: i32,
    pub cmdlen: i32,
    pub cmdpos: i32,
    pub cmdspos: i32,
    pub cmdfirstc: i32,
    pub cmdindent: i32,
    pub cmdprompt: Option<CString>,
    pub hl_id: i32,
    pub overstrike: i32,
    pub xpc: *mut ExpandCtx,
    pub xp_context: i32,
    pub xp_arg: Option<CString>,
    pub input_fn: i32,
    pub cmdbuff_replaced: bool,
    pub prompt_id: u32,
    pub highlight_callback: Callback,
    pub last_colors: ColoredCmdline,
    pub level: i32,
    pub prev_ccline: Option<Box<CmdlineInfo>>,
    pub special_char: u8,
    pub special_shift: bool,
    pub redraw_state: i32,
    pub one_key: bool,
    pub mouse_used: Option<bool>,
}

impl CmdlineInfo {
    pub const fn new() -> Self {
        Self {
            cmdbuff: ptr::null_mut(),
            cmdbuff_vec: Vec::new(),
            cmdbufflen: 0,
            cmdlen: 0,
            cmdpos: 0,
            cmdspos: 0,
            cmdfirstc: 0,
            cmdindent: 0,
            cmdprompt: None,
            hl_id: 0,
            overstrike: 0,
            xpc: ptr::null_mut(),
            xp_context: 0,
            xp_arg: None,
            input_fn: 0,
            cmdbuff_replaced: false,
            prompt_id: 0,
            highlight_callback: Callback::NONE,
            last_colors: ColoredCmdline::new(),
            level: 0,
            prev_ccline: None,
            special_char: 0,
            special_shift: false,
            redraw_state: 0,
            one_key: false,
            mouse_used: None,
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
    cmd_showtail: bool,
    may_expand_pattern: bool,
    pre_incsearch_pos: CursorPos,
    compl_match_array: Vec<Pumitem>,
    compl_startcol: i32,
    compl_selected: i32,
    cmdline_orig: Option<Vec<u8>>,
    filetype_expand_what: FiletypeExpandWhat,
    breakpt_expand_what: BreakptExpandWhat,
    healthcheck_names: Object,
    healthcheck_last_gen: u32,
    arg1_lua_names: Object,
    arg1_lua_last_xp_line: Option<CString>,
    arg1_lua_last_gen: u32,
    first_match: i32,
}

impl CmdexpandState {
    const fn new() -> Self {
        Self {
            cmd_showtail: false,
            may_expand_pattern: false,
            pre_incsearch_pos: CursorPos {
                lnum: 0,
                col: 0,
                coladd: 0,
            },
            compl_match_array: Vec::new(),
            compl_startcol: 0,
            compl_selected: 0,
            cmdline_orig: None,
            filetype_expand_what: FiletypeExpandWhat::All,
            breakpt_expand_what: BreakptExpandWhat::Add,
            healthcheck_names: Object {
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Integer(0),
            },
            healthcheck_last_gen: 0,
            arg1_lua_names: Object {
                obj_type: ObjectType::ObjectTypeNil,
                data: ObjectData::Integer(0),
            },
            arg1_lua_last_xp_line: None,
            arg1_lua_last_gen: 0,
            first_match: 0,
        }
    }
}

static CMDEXP_STATE: GlobalState<CmdexpandState> = GlobalState::new(CmdexpandState::new());

#[inline]
fn cmdexp() -> &'static mut CmdexpandState {
    unsafe { &mut *CMDEXP_STATE.as_ptr() }
}

#[inline]
fn c_strchr(s: impl crate::strings::IntoCCharPtr, c: c_char) -> *mut c_char {
    let mut s = s.into_c_char_ptr();
    {
        loop {
            if unsafe { *s } == c {
                return s.cast_mut();
            }
            if crate::memory::cstr_is_empty(s) {
                return ptr::null_mut();
            }
            s = s.wrapping_add(1);
        }
    }
}

use crate::mbyte::ptr2cells;
use crate::path::vim_ispathsep;

#[inline]
fn show_match(matches: &[CString], m: usize, showtail: bool) -> *mut c_char {
    let item = matches[m].as_ptr().cast_mut();
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
        _ => (opt_vars().wop_flags & K_OPT_WOP_FLAG_FUZZY) != 0,
    }
}

/// Returns true if fuzzy completion for cmdline completion is enabled and
/// "fuzzystr" is not empty.
pub fn cmdline_fuzzy_complete(fuzzystr: impl crate::strings::IntoCCharPtr) -> bool {
    let fuzzystr = fuzzystr.into_c_char_ptr();
    (opt_vars().wop_flags & K_OPT_WOP_FLAG_FUZZY) != 0 && !crate::memory::cstr_is_empty(fuzzystr)
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
    crate::memory::cstr_as_bytes(p1.as_ptr()).cmp(crate::memory::cstr_as_bytes(p2.as_ptr())) as i32
}

/// Escape special characters in the cmdline completion matches.
fn wildescape(
    xp: &mut ExpandCtx,
    str_arg: impl crate::strings::IntoCCharPtr,
    files: &mut [CString],
) {
    let str_arg = str_arg.into_c_char_ptr();
    {
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
                    let pat = if (xp.xp_backslash & XP_BS_COMMA) != 0 {
                        c" ,".as_ptr()
                    } else {
                        c" ".as_ptr()
                    };
                    *file_slot = vim_strsave_escaped(file_slot.as_ptr(), pat);
                    #[cfg(windows)]
                    {
                        *file_slot = vim_strsave_escaped(file_slot.as_ptr(), c" ");
                    }
                } else if (xp.xp_backslash & XP_BS_COMMA) != 0
                    && file_slot.as_bytes().contains(&b',')
                {
                    *file_slot = vim_strsave_escaped(file_slot.as_ptr(), c",");
                }

                #[cfg(windows)]
                {
                    *file_slot = vim_strsave_fnameescape(file_slot.as_ptr(), vse_what);
                }
                #[cfg(not(windows))]
                {
                    *file_slot = vim_strsave_fnameescape(
                        file_slot.as_ptr(),
                        if xp.xp_shell { VSE_SHELL } else { vse_what },
                    );
                }

                if crate::memory::cstr_first_byte(str_arg) == b'\\'
                    && crate::memory::cstr_byte_at(str_arg, 1) == b'~'
                    && file_slot.as_bytes().first() == Some(&b'~')
                {
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
                *file_slot = vim_strsave_escaped(file_slot.as_ptr(), c"\\|\"");
            }
        }
    }
}

/// Escape special characters in the cmdline completion matches.
fn expand_escape(
    xp: &mut ExpandCtx,
    str_arg: impl crate::strings::IntoCCharPtr,
    files: &mut [CString],
    options: i32,
) {
    let str_arg = str_arg.into_c_char_ptr();
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
    let ccline = get_cmdline_info();
    let mut p: Option<CString>;
    let from_wildtrigger_func = (options & WILD_FUNC_TRIGGER) != 0;
    let wild_navigate = type_arg == WILD_NEXT
        || type_arg == WILD_PREV
        || type_arg == WILD_PAGEUP
        || type_arg == WILD_PAGEDOWN
        || type_arg == WILD_PUM_WANT;

    if xp.xp_numfiles == -1 {
        cmdexp().pre_incsearch_pos = xp.xp_pre_incsearch_pos;
        if ccline.input_fn != 0 && ccline.xp_context == EXPAND_COMMANDS {
            set_cmd_context(
                xp,
                ccline.cmdbuff,
                ccline.cmdlen,
                ccline.cmdpos,
                false as i32,
            );
        } else {
            cmdexp().may_expand_pattern = (options & WILD_MAY_EXPAND_PATTERN) != 0;
            set_expand_context(xp);
            cmdexp().may_expand_pattern = false;
        }
        if xp.xp_context == EXPAND_LUA {
            nlua_expand_pat(xp);
        }
        cmdexp().cmd_showtail = expand_showtail(xp);
    }

    if xp.xp_context == EXPAND_UNSUCCESSFUL {
        beep_flush();
        return true;
    }
    if xp.xp_context == EXPAND_NOTHING {
        return false;
    }

    let i = crate::memory::ptr_offset_from(xp.xp_pattern, ccline.cmdbuff) as i32;
    debug_assert!(ccline.cmdpos >= i);
    xp.xp_pattern_len = (ccline.cmdpos as usize) - (i as usize);

    if from_wildtrigger_func && xp.xp_context == EXPAND_COMMANDS && xp.xp_pattern_len == 0 {
        return false;
    }

    if !g_st().cmd_silent
        && !from_wildtrigger_func
        && !wild_navigate
        && !(ui_has(K_UICMDLINE) || ui_has(K_UIWILDMENU_CONST))
    {
        msg_puts(c"...");
        ui_flush();
    }

    if wild_navigate {
        p = expand_one(xp, ptr::null_mut(), None, 0, type_arg);
    } else {
        let tmp =
            if cmdline_fuzzy_completion_supported(xp) || xp.xp_context == EXPAND_PATTERN_IN_BUF {
                cstr_from_bytes(unsafe {
                    std::slice::from_raw_parts(xp.xp_pattern.cast::<u8>(), xp.xp_pattern_len)
                })
            } else {
                addstar(xp.xp_pattern, xp.xp_pattern_len, xp.xp_context)
            };
        let use_options = options
            | WILD_USE_COMPLETESLASH
            | WILD_HOME_REPLACE
            | WILD_ADD_SLASH
            | WILD_SILENT
            | if escape { WILD_ESCAPE } else { 0 }
            | if opt_vars().p_wic != 0 { WILD_ICASE } else { 0 };
        let orig = cstr_from_bytes(unsafe {
            std::slice::from_raw_parts(
                ccline.cmdbuff.wrapping_add(i as usize).cast::<u8>(),
                xp.xp_pattern_len,
            )
        });
        p = expand_one(
            xp,
            tmp.as_ptr().cast_mut(),
            Some(orig),
            use_options,
            type_arg,
        );
        if let Some(ref p_str) = p
            && type_arg == WILD_LONGEST
        {
            let mut j: usize = 0;
            while j < xp.xp_pattern_len {
                let c = unsafe { *ccline.cmdbuff.wrapping_add((i as usize) + j) };
                if c == b'*' as c_char || c == b'?' as c_char {
                    break;
                }
                j += 1;
            }
            if p_str.to_bytes().len() < j {
                p = None;
            }
        }
    }

    if !wild_navigate && !ccline.cmdbuff.is_null() {
        let slice = crate::memory::cstr_slice(ccline.cmdbuff, ccline.cmdlen as usize);
        let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        cmdexp().cmdline_orig = Some(slice[..len].to_vec());
    }

    if let Some(ref p_str) = p
        && !g_st().got_int
        && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0
    {
        let plen = p_str.to_bytes().len();
        let difflen = (plen as i32) - (xp.xp_pattern_len as i32);
        if ccline.cmdlen + difflen + 4 > ccline.cmdbufflen {
            realloc_cmdbuff(ccline.cmdlen + difflen + 4);
            xp.xp_pattern = ccline.cmdbuff.wrapping_add(i as usize);
        }
        debug_assert!(ccline.cmdpos <= ccline.cmdlen);
        unsafe {
            ptr::copy(
                ccline.cmdbuff.wrapping_add(ccline.cmdpos as usize),
                ccline
                    .cmdbuff
                    .wrapping_add((ccline.cmdpos + difflen) as usize),
                (ccline.cmdlen as usize) - (ccline.cmdpos as usize) + 1,
            )
        };
        unsafe {
            ptr::copy(
                p_str.as_ptr(),
                ccline.cmdbuff.wrapping_add(i as usize),
                plen,
            )
        };
        ccline.cmdlen += difflen;
        ccline.cmdpos += difflen;
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
        expand_one(xp, ptr::null_mut(), None, 0, WILD_FREE);
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
            .or_else(|| {
                let p = show_match(matches, i, showtail);
                if p.is_null() {
                    None
                } else {
                    Some(crate::memory::cstr_to_cstring(p))
                }
            });
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
            pum_cpt_source_idx: 0,
            pum_user_abbr_hlattr: -1,
            pum_user_kind_hlattr: -1,
            pum_comp_idx: None,
        });
    }
    cmdexp().compl_match_array = items;

    let endpos = if showtail {
        showmatches_gettail(xp.xp_pattern, cmdline_unchanged)
    } else {
        xp.xp_pattern
    };
    if ui_has(K_UICMDLINE) && g_st().cmdline_win.is_null() {
        cmdexp().compl_startcol = crate::memory::ptr_offset_from(endpos, ccline.cmdbuff) as i32;
    } else {
        cmdexp().compl_startcol =
            cmd_screencol(crate::memory::ptr_offset_from(endpos, ccline.cmdbuff) as i32);
    }
}

pub fn cmdline_pum_display(changed_array: bool) {
    let arr = &mut cmdexp().compl_match_array;
    pum_display(
        arr.as_mut_ptr(),
        arr.len() as i32,
        cmdexp().compl_selected,
        changed_array,
        cmdexp().compl_startcol,
    );
}

/// Returns true if the cmdline completion popup menu is being displayed.
pub fn cmdline_pum_active() -> bool {
    let arr = &cmdexp().compl_match_array;
    pum_visible() && !arr.is_empty()
}

/// Remove the cmdline completion popup menu (if present), free the list of items.
pub fn cmdline_pum_remove(defer_redraw: bool) {
    pum_undisplay(!defer_redraw);
    cmdexp().compl_match_array = Vec::new();
}

pub fn cmdline_pum_cleanup(cclp: &mut CmdlineInfo) {
    cmdline_pum_remove(false);
    wildmenu_cleanup(cclp);
}

/// Returns the current cmdline completion pattern.
pub fn cmdline_compl_pattern() -> Option<&'static CStr> {
    unsafe {
        let xp = get_cmdline_info().xpc;
        (*xp).xp_orig.as_deref()
    }
}

/// Returns true if fuzzy cmdline completion is active, false otherwise.
pub fn cmdline_compl_is_fuzzy() -> bool {
    unsafe {
        let xp = get_cmdline_info().xpc;
        !xp.is_null() && cmdline_fuzzy_completion_supported(&*xp)
    }
}

/// Checks whether popup menu should be used for cmdline completion wildmenu.
fn cmdline_compl_use_pum(need_wildmenu: bool) -> bool {
    (need_wildmenu
        && (opt_vars().wop_flags & K_OPT_WOP_FLAG_PUM) != 0
        && !(ui_has(K_UICMDLINE) && g_st().cmdline_win.is_null()))
        || ui_has(K_UIWILDMENU_CONST)
        || (ui_has(K_UICMDLINE) && ui_has(K_UIPOPUPMENU_CONST))
}

/// Return the number of characters that should be skipped in the wildmenu.
fn skip_wildmenu_char(xp: &ExpandCtx, s: impl crate::strings::IntoCCharPtr) -> i32 {
    let s = s.into_c_char_ptr();
    {
        if (rem_backslash(s)
            && xp.xp_context != EXPAND_HELP
            && xp.xp_context != EXPAND_PATTERN_IN_BUF)
            || ((xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES)
                && (crate::memory::cstr_first_byte(s) == b'\t'
                    || (crate::memory::cstr_first_byte(s) == b'\\'
                        && crate::memory::cstr_byte_at(s, 1) != 0)))
        {
            #[cfg(not(windows))]
            if xp.xp_shell
                && csh_like_shell() != 0
                && crate::memory::cstr_byte_at(s, 1) == b'\\'
                && crate::memory::cstr_byte_at(s, 2) == b'!'
            {
                return 2;
            }
            return 1;
        }
        0
    }
}

/// Get the length of an item as it will be shown in the status line.
fn wildmenu_match_len(xp: &ExpandCtx, s: impl crate::strings::IntoCCharPtr) -> i32 {
    let mut s = s.into_c_char_ptr();
    {
        let mut len: i32 = 0;
        let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;

        if emenu && menu_is_separator(s) {
            return 1;
        }

        while !crate::memory::cstr_is_empty(s) {
            s = s.wrapping_add(skip_wildmenu_char(xp, s) as usize);
            len += ptr2cells(s);
            s = s.wrapping_add(utfc_ptr2len(s) as usize);
        }

        len
    }
}

/// Show wildchar matches in the status line.
fn redraw_wildmenu(xp: &ExpandCtx, matches: &[CString], mut match_idx: i32, showtail: bool) {
    let mut highlight = true;
    let mut selstart: *mut c_char = ptr::null_mut();
    let mut selstart_col: i32 = 0;
    let mut selend: *mut c_char = ptr::null_mut();
    let mut add_left = false;

    if matches.is_empty() {
        return;
    }
    let num_matches = matches.len() as i32;

    let mut buf_vec = vec![0 as c_char; (g_st().columns_global as usize) * MB_MAXBYTES + 1];
    let buf = buf_vec.as_mut_ptr();

    if match_idx == -1 {
        match_idx = 0;
        highlight = false;
    }
    let mut clen = wildmenu_match_len(xp, show_match(matches, match_idx as usize, showtail)) + 3;
    if match_idx == 0 {
        cmdexp().first_match = 0;
    } else if match_idx < cmdexp().first_match {
        cmdexp().first_match = match_idx;
        add_left = true;
    } else {
        for i in cmdexp().first_match..match_idx {
            clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
        }
        if cmdexp().first_match > 0 {
            clen += 2;
        }
        if clen > g_st().columns_global {
            cmdexp().first_match = match_idx;
            clen = 2;
            let mut i = match_idx;
            while i < num_matches {
                clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
                if clen >= g_st().columns_global {
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
        while cmdexp().first_match > 0 {
            clen += wildmenu_match_len(
                xp,
                show_match(matches, (cmdexp().first_match - 1) as usize, showtail),
            ) + 2;
            if clen >= g_st().columns_global {
                break;
            }
            cmdexp().first_match -= 1;
        }
    }

    let mut group: Hlf = 0;
    let fillchar = fillchar_status(&mut group, curwin_mut());
    let attr = win_hl_attr(curwin_mut(), group);

    let mut len: usize;
    if cmdexp().first_match == 0 {
        unsafe {
            *buf = NUL;
        }
        len = 0;
    } else {
        crate::memory::cstr_cpy(buf, c"< ");
        len = 2;
    }
    clen = len as i32;

    let mut i = cmdexp().first_match;
    while clen + wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2
        < g_st().columns_global
    {
        if i == match_idx {
            selstart = buf.wrapping_add(len);
            selstart_col = clen;
        }

        let mut s = show_match(matches, i as usize, showtail);
        let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;
        if emenu && menu_is_separator(s) {
            crate::memory::cstr_cpy(buf.wrapping_add(len), transchar(b'|' as i32));
            let l = crate::memory::cstr_len(buf.wrapping_add(len));
            len += l;
            clen += l as i32;
        } else {
            while !crate::memory::cstr_is_empty(s) {
                s = s.wrapping_add(skip_wildmenu_char(xp, s) as usize);
                clen += ptr2cells(s);
                let l = utfc_ptr2len(s);
                if l > 1 {
                    unsafe { ptr::copy_nonoverlapping(s, buf.wrapping_add(len), l as usize) };
                    s = s.wrapping_add((l - 1) as usize);
                    len += l as usize;
                } else {
                    crate::memory::cstr_cpy(
                        buf.wrapping_add(len),
                        transchar_byte(i32::from(crate::memory::cstr_first_byte(s))),
                    );
                    len += crate::memory::cstr_len(buf.wrapping_add(len));
                }
                s = s.wrapping_add(1);
            }
        }
        if i == match_idx {
            selend = buf.wrapping_add(len);
        }

        unsafe {
            *buf.wrapping_add(len) = b' ' as c_char;
        }
        len += 1;
        unsafe {
            *buf.wrapping_add(len) = b' ' as c_char;
        }
        len += 1;
        clen += 2;
        i += 1;
        if i == num_matches {
            break;
        }
    }

    if i != num_matches {
        unsafe {
            *buf.wrapping_add(len) = b'>' as c_char;
        }
        len += 1;
        clen += 1;
    }

    unsafe {
        *buf.wrapping_add(len) = NUL;
    }

    let mut row = g_st().cmdline_row - 1;
    if row >= 0 {
        if g_st().wild_menu_showing == 0 {
            if g_st().msg_scrolled > 0 {
                if g_st().cmdline_row == g_st().rows_global - 1 {
                    msg_scroll_up(false, false);
                    g_st().msg_scrolled += 1;
                } else {
                    g_st().cmdline_row += 1;
                    row += 1;
                }
                g_st().wild_menu_showing = WM_SCROLLED;
            } else {
                if lastwin_mut().w_status_height == 0 && global_stl_height() == 0 {
                    g_st().save_p_ls = opt_vars().p_ls as i32;
                    g_st().save_p_wmh = opt_vars().p_wmh as i32;
                    opt_vars().p_ls = 2;
                    opt_vars().p_wmh = 0;
                    last_status(false);
                }
                g_st().wild_menu_showing = WM_SHOWN;
            }
        }

        grid_line_start(
            if g_st().wild_menu_showing == WM_SCROLLED {
                &raw mut g_st().msg_grid_adj
            } else {
                &raw mut g_st().default_gridview
            },
            row,
        );

        grid_line_puts(0, buf, -1, attr);
        if !selstart.is_null() && highlight {
            unsafe {
                *selend = NUL;
            }
            grid_line_puts(selstart_col, selstart, -1, hl_attr(HLF_WM));
        }

        grid_line_fill(clen, g_st().columns_global, fillchar, attr);

        grid_line_flush();
    }

    win_redraw_last_status(g_st().topframe);
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
        debug_assert!(g_st().pum_want.active);
        findex = g_st().pum_want.item;
    }

    if findex < 0 || findex >= xp.xp_numfiles {
        if xp.xp_orig.is_some() {
            findex = -1;
        } else {
            findex = if findex < 0 { xp.xp_numfiles - 1 } else { 0 };
        }
    }

    if opt_vars().p_wmnu != 0 {
        let arr = &cmdexp().compl_match_array;
        if !arr.is_empty() {
            cmdexp().compl_selected = findex;
            cmdline_pum_display(false);
        } else if cmdline_compl_use_pum(true) {
            cmdline_pum_create(
                &*get_cmdline_info(),
                xp,
                &xp.xp_files.as_slice()[..xp.xp_numfiles as usize],
                cmdexp().cmd_showtail,
                false,
            );
            cmdexp().compl_selected = findex;
            pum_clear();
            cmdline_pum_display(true);
        } else {
            redraw_wildmenu(
                xp,
                &xp.xp_files.as_slice()[..xp.xp_numfiles as usize],
                findex,
                cmdexp().cmd_showtail,
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
    str_arg: impl crate::strings::IntoCCharPtr,
    options: i32,
) -> Option<CString> {
    let str_arg = str_arg.into_c_char_ptr().cast_mut();
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
            expand_escape(xp, str_arg, &mut xp_files.as_mut_slice()[..nfiles], options);
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
                        if match_suffix(xp.xp_files.as_slice()[i].as_ptr().cast_mut()) {
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
    let first_file = files[0].as_ptr();

    while crate::memory::cstr_byte_at(first_file, len) != 0 {
        let mb_len = utfc_ptr2len(first_file.wrapping_add(len)) as usize;
        let c0 = utf_ptr2char(first_file.wrapping_add(len));
        let mut i: usize = 1;
        while (i as i32) < xp.xp_numfiles {
            let ci = utf_ptr2char(files[i].as_ptr().wrapping_add(len));

            if opt_vars().p_fic != 0
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

    cstr_from_bytes(crate::memory::cstr_slice(first_file, len))
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
    str_arg: *mut c_char,
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

        let arr = &cmdexp().compl_match_array;
        if !arr.is_empty() {
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

        ss = expand_one_start(mode, xp, str_arg, options);
    }

    if mode == WILD_LONGEST && xp.xp_numfiles > 0 {
        ss = Some(find_longest_match(xp, options));
        xp.xp_selected = -1;
    }

    if mode == WILD_ALL && xp.xp_numfiles > 0 && !g_st().got_int {
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
        ss = Some(cstr_from_bytes(&out));
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
    cmdexp().cmdline_orig = None;
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
            let m = matches[j as usize].as_ptr();
            msg_outtrans(m, HLF_D, false);
            if let Some((kind_str, fname_str)) = crate::tag::tag_listfiles_extra(j as usize) {
                msg_advance(maxlen + 1);
                msg_puts(kind_str);
                msg_advance(maxlen + 3);
                msg_outtrans_long(fname_str, HLF_D);
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
        let p: *mut c_char;
        if xp.xp_context == EXPAND_FILES
            || xp.xp_context == EXPAND_SHELLCMD
            || xp.xp_context == EXPAND_BUFFERS
        {
            let match_j = matches[j as usize].as_ptr();
            if xp.xp_numfiles != -1 {
                let exp_path = expand_env_save_opt(match_j.cast_mut(), true, ptr::null_mut());
                let halved_slash = backslash_halve_save(exp_path.as_ptr());
                isdir = os_isdir(halved_slash.as_ptr());
            } else {
                isdir = os_isdir(match_j);
            }
            if showtail {
                p = show_match(matches, j as usize, showtail);
            } else {
                let name_buff = g_st().name_buff.as_mut_ptr();
                home_replace(None, match_j, name_buff, MAXPATHL, true);
                p = name_buff;
            }
        } else {
            isdir = false;
            p = show_match(matches, j as usize, showtail);
        }
        lastlen = msg_outtrans(p, if isdir { HLF_D } else { 0 }, false);
        j += lines;
    }
    if g_st().msg_col > 0 {
        msg_clr_eos();
        msg_putchar(b'\n' as i32);
    }
}

#[inline]
fn show_pum_matches(
    ccline: &mut CmdlineInfo,
    xp: &ExpandCtx,
    matches: &[CString],
    showtail: bool,
    noselect: bool,
    cmdline_unchanged: bool,
) {
    cmdline_pum_create(ccline, xp, matches, showtail, cmdline_unchanged);
    cmdexp().compl_selected = if noselect { -1 } else { 0 };
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
    let ccline = get_cmdline_info();
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
        let retval = expand_cmdline(xp, ccline.cmdbuff, ccline.cmdpos, &mut temp_matches);
        if retval != EXPAND_OK {
            return retval;
        }
        showtail = expand_showtail(xp);
        &temp_matches
    } else {
        showtail = cmdexp().cmd_showtail;
        &xp.xp_files.as_slice()[..xp.xp_numfiles as usize]
    };

    if cmdline_compl_use_pum(display_wildmenu && !display_list) {
        show_pum_matches(
            &mut *ccline,
            xp,
            matches,
            showtail,
            noselect,
            cmdline_unchanged,
        );
        if xp.xp_numfiles == -1 {
            free_xp_files_extra(xp);
        }
        return EXPAND_OK;
    }

    if display_list {
        g_st().msg_didany = false;
        msg_start();
        if !ui_has(K_UIMESSAGES) {
            msg_putchar(b'\n' as i32);
        }
        ui_flush();
        g_st().cmdline_row = g_st().msg_row;
        g_st().msg_didany = false;
        msg_ext_set_kind(c"wildlist");
        msg_start();
    }

    if g_st().got_int {
        g_st().got_int = false;
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
                let name_buff = g_st().name_buff.as_mut_ptr();
                home_replace(None, match_i.as_ptr(), name_buff, MAXPATHL, true);
                vim_strsize(name_buff)
            } else {
                vim_strsize(show_match(matches, i, showtail))
            };
            maxlen = maxlen.max(len);
        }

        let lines = if xp.xp_context == EXPAND_TAGS_LISTFILES {
            num_matches
        } else {
            maxlen += 2;
            let mut columns = (g_st().columns_global + 2) / maxlen;
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
            if g_st().got_int {
                g_st().got_int = false;
                break;
            }
        }

        g_st().cmdline_row = g_st().msg_row;
    }

    if display_wildmenu && display_list {
        if cmdline_compl_use_pum(true) {
            show_pum_matches(
                &mut *ccline,
                xp,
                matches,
                showtail,
                noselect,
                cmdline_unchanged,
            );
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
fn showmatches_gettail(s: impl crate::strings::IntoCCharPtr, eager: bool) -> *mut c_char {
    let s = s.into_c_char_ptr().cast_mut();
    {
        let mut t = s;
        let mut had_sep = false;

        let mut p = s;
        while !crate::memory::cstr_is_empty(p) {
            #[cfg(windows)]
            let is_sep = vim_ispathsep(crate::memory::cstr_first_char(p)) && !rem_backslash(p);
            #[cfg(not(windows))]
            let is_sep = vim_ispathsep(crate::memory::cstr_first_char(p));

            if is_sep {
                if eager {
                    t = p.wrapping_add(1);
                } else {
                    had_sep = true;
                }
            } else if had_sep {
                t = p;
                had_sep = false;
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
        }
        t
    }
}

/// Return true if we only need to show the tail of completion matches.
fn expand_showtail(xp: &mut ExpandCtx) -> bool {
    if xp.xp_context != EXPAND_FILES
        && xp.xp_context != EXPAND_SHELLCMD
        && xp.xp_context != EXPAND_DIRECTORIES
    {
        return false;
    }

    let end = path_tail(xp.xp_pattern);
    if end == xp.xp_pattern {
        return false;
    }

    let mut s = xp.xp_pattern;
    while (s as usize) < (end as usize) {
        if rem_backslash(s) {
            s = s.wrapping_add(1);
        } else if crate::strings::vim_haschr(c"*?[", crate::memory::cstr_first_byte(s) as i32) {
            return false;
        }
        s = s.wrapping_add(1);
    }
    true
}

/// Prepare a string for expansion.
pub fn addstar(fname: impl crate::strings::IntoCCharPtr, mut len: usize, context: i32) -> CString {
    let fname = fname.into_c_char_ptr();
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
                    && crate::memory::cstr_first_byte(fname) == b'/')
                || context == EXPAND_CHECKHEALTH
                || context == EXPAND_LSP
                || context == EXPAND_LOG
                || context == EXPAND_PACKDEL
                || context == EXPAND_PACKUPDATE
                || context == EXPAND_MARKS
                || context == EXPAND_LUA
            {
                cstr_from_bytes(crate::memory::cstr_slice(fname, len))
            } else {
                let mut new_len = len + 2;
                for i in 0..len {
                    let ch = unsafe { *fname.wrapping_add(i) };
                    if ch == b'*' as c_char || ch == b'~' as c_char {
                        new_len += 1;
                    }
                    if context == EXPAND_BUFFERS && ch == b'.' as c_char {
                        new_len += 1;
                    }
                    if (context == EXPAND_USER_DEFINED || context == EXPAND_USER_LIST)
                        && ch == b'\\' as c_char
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
                        && crate::memory::cstr_byte_at(fname, i) == b'\\'
                    {
                        i += 1;
                        if i == len {
                            break;
                        }
                    }

                    match crate::memory::cstr_byte_at(fname, i) {
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
                    retval[j] = crate::memory::cstr_byte_at(fname, i);
                    i += 1;
                    j += 1;
                }
                cstr_from_bytes(&retval[..j])
            }
        } else {
            let mut retval_vec = vec![0 as c_char; len + 4];
            let retval = retval_vec.as_mut_ptr();
            crate::memory::cstr_ncpy(retval, fname, len);

            let tail = path_tail(retval);
            #[cfg(windows)]
            let ends_in_star = len > 0 && crate::memory::cstr_byte_at(retval, len - 1) == b'*';
            #[cfg(not(windows))]
            let mut ends_in_star = len > 0 && crate::memory::cstr_byte_at(retval, len - 1) == b'*';
            #[cfg(not(windows))]
            {
                let mut k = (len as isize) - 2;
                while k >= 0 {
                    if crate::memory::cstr_byte_at(retval, k as usize) != b'\\' {
                        break;
                    }
                    ends_in_star = !ends_in_star;
                    k -= 1;
                }
            }
            if (crate::memory::cstr_first_byte(retval) != b'~' || tail != retval)
                && !ends_in_star
                && !crate::strings::vim_haschr(tail, b'$' as i32)
                && !crate::strings::vim_haschr(retval, b'`' as i32)
            {
                unsafe {
                    *retval.wrapping_add(len) = b'*' as c_char;
                }
                len += 1;
            } else if len > 0 && crate::memory::cstr_byte_at(retval, len - 1) == b'$' {
                len -= 1;
            }
            unsafe {
                *retval.wrapping_add(len) = NUL;
            }
            cstr_from_bytes(crate::memory::cstr_slice(retval, len))
        }
    }
}

/// Must parse the command line so far to work out what context we are in.
pub fn set_expand_context(xp: &mut ExpandCtx) {
    let ccline = get_cmdline_info();

    if (ccline.cmdfirstc == b'/' as i32 || ccline.cmdfirstc == b'?' as i32)
        && cmdexp().may_expand_pattern
    {
        xp.xp_context = EXPAND_PATTERN_IN_BUF;
        xp.xp_search_dir = if ccline.cmdfirstc == b'/' as i32 {
            FORWARD
        } else {
            BACKWARD
        };
        xp.xp_pattern = ccline.cmdbuff;
        xp.xp_pattern_len = ccline.cmdpos as usize;
        g_st().search_global.first_line = 0;
        return;
    }

    if ccline.cmdfirstc != b':' as i32
        && ccline.cmdfirstc != b'>' as i32
        && ccline.cmdfirstc != b'=' as i32
        && ccline.input_fn == 0
    {
        xp.xp_context = EXPAND_NOTHING;
        return;
    }

    set_cmd_context(
        xp,
        ccline.cmdbuff,
        ccline.cmdlen,
        ccline.cmdpos,
        true as i32,
    );
}

/// Sets the index of a built-in or user defined command "cmd" in eap->cmdidx.
fn set_cmd_index(
    cmd: impl crate::strings::IntoCCharPtr,
    eap: &mut Exarg,
    xp: &mut ExpandCtx,
    complp: &mut i32,
) -> *const c_char {
    let cmd = cmd.into_c_char_ptr();
    {
        let mut p: *const c_char;
        let fuzzy = cmdline_fuzzy_complete(cmd);

        if !fuzzy
            && opt_vars().p_ic == 0
            && crate::memory::cstr_first_byte(cmd) == b'k'
            && crate::memory::cstr_byte_at(cmd, 1) != b'e'
        {
            eap.cmdidx = Cmdidx::CmdK as i32;
            p = cmd.wrapping_add(1);
        } else {
            p = cmd;
            while ascii_isalpha(crate::memory::cstr_first_char(p))
                || crate::memory::cstr_first_byte(p) == b'*'
            {
                p = p.wrapping_add(1);
            }
            if ascii_isupper(crate::memory::cstr_first_char(cmd)) {
                while ascii_isalnum(crate::memory::cstr_first_char(p))
                    || crate::memory::cstr_first_byte(p) == b'*'
                {
                    p = p.wrapping_add(1);
                }
            }
            if crate::memory::cstr_first_byte(cmd) == b'p'
                && crate::memory::cstr_byte_at(cmd, 1) == b'y'
                && p == cmd.wrapping_add(2)
                && crate::memory::cstr_first_byte(p) == b'3'
            {
                p = p.wrapping_add(1);
                while ascii_isalpha(crate::memory::cstr_first_char(p))
                    || crate::memory::cstr_first_byte(p) == b'*'
                {
                    p = p.wrapping_add(1);
                }
            }
            if p == cmd
                && crate::strings::vim_haschr(
                    c"@*!=><&~#",
                    crate::memory::cstr_first_byte(p) as i32,
                )
            {
                p = p.wrapping_add(1);
            }
            let len = crate::memory::ptr_offset_from(p, cmd) as usize;

            if len == 0 {
                xp.xp_context = EXPAND_UNSUCCESSFUL;
                return ptr::null();
            }

            eap.cmdidx = excmd_get_cmdidx(cmd, len) as i32;

            if (unsafe { *cmd } >= b'A' as c_char && unsafe { *cmd } <= b'Z' as c_char)
                || (fuzzy
                    && eap.cmdidx != Cmdidx::CmdBang as i32
                    && !crate::memory::cstr_is_empty(p))
            {
                while ascii_isalnum(crate::memory::cstr_first_char(p))
                    || crate::memory::cstr_first_byte(p) == b'*'
                {
                    p = p.wrapping_add(1);
                }
            }
        }

        if crate::memory::cstr_is_empty(p) && ascii_isalnum(unsafe { *p.wrapping_sub(1) }) {
            return ptr::null();
        }

        if eap.cmdidx == Cmdidx::CmdSize as i32 {
            if crate::memory::cstr_first_byte(cmd) == b's'
                && crate::strings::vim_haschr(c"cgriI", crate::memory::cstr_byte_at(cmd, 1) as i32)
            {
                eap.cmdidx = Cmdidx::CmdSubstitute as i32;
                p = cmd.wrapping_add(1);
            } else if unsafe { *cmd } >= b'A' as c_char && unsafe { *cmd } <= b'Z' as c_char {
                eap.cmd = cmd.cast_mut();
                p = find_ucmd(eap, p.cast_mut(), None, Some(&mut *xp), Some(&mut *complp));
                if p.is_null() {
                    eap.cmdidx = Cmdidx::CmdSize as i32;
                }
            }
        }
        if eap.cmdidx == Cmdidx::CmdSize as i32 {
            xp.xp_context = EXPAND_UNSUCCESSFUL;
            return ptr::null();
        }

        p
    }
}

/// Set the completion context for a command argument with wild card characters.
fn set_context_for_wildcard_arg(
    eap: Option<&Exarg>,
    arg: impl crate::strings::IntoCCharPtr,
    usefilter: bool,
    xp: &mut ExpandCtx,
    complp: &mut i32,
) {
    let arg = arg.into_c_char_ptr();
    {
        let mut in_quote = false;
        let mut bow: *const c_char = ptr::null();
        let mut len: usize;

        xp.xp_pattern = skipwhite(arg);
        let mut p: *const c_char = xp.xp_pattern;
        while !crate::memory::cstr_is_empty(p) {
            let mut c = utf_ptr2char(p);
            if c == b'\\' as i32 && crate::memory::cstr_byte_at(p, 1) != 0 {
                p = p.wrapping_add(1);
            } else if c == b'`' as i32 {
                if !in_quote {
                    xp.xp_pattern = p.cast_mut();
                    bow = p.wrapping_add(1);
                }
                in_quote = !in_quote;
            } else if c == b'|' as i32
                || c == b'\n' as i32
                || c == b'"' as i32
                || (ascii_iswhite(c)
                    && (!eap.is_some_and(|eap| (eap.argt & EX_NOSPC) != 0) || usefilter))
            {
                len = 0;
                while !crate::memory::cstr_is_empty(p) {
                    c = utf_ptr2char(p);
                    if c == b'`' as i32 || vim_isfilec_or_wc(c) {
                        break;
                    }
                    len = utfc_ptr2len(p) as usize;
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }
                if in_quote {
                    bow = p;
                } else {
                    xp.xp_pattern = p.cast_mut();
                }
                p = p.wrapping_sub(len);
            }
            p = p.wrapping_add(utfc_ptr2len(p) as usize);
        }

        if !bow.is_null() && in_quote {
            xp.xp_pattern = bow.cast_mut();
        }
        xp.xp_context = EXPAND_FILES;

        if usefilter
            || eap.is_some_and(|eap| {
                eap.cmdidx == Cmdidx::CmdBang as i32 || eap.cmdidx == Cmdidx::CmdTerminal as i32
            })
            || *complp == EXPAND_SHELLCMDLINE
        {
            #[cfg(not(windows))]
            {
                xp.xp_shell = true;
            }
            if xp.xp_pattern == skipwhite(arg) {
                xp.xp_context = EXPAND_SHELLCMD;
            }
        }

        if crate::memory::cstr_first_byte(xp.xp_pattern) == b'$' {
            p = xp.xp_pattern.wrapping_add(1);
            p = p.wrapping_add(env_name_len(p));
            if crate::memory::cstr_is_empty(p) {
                xp.xp_context = EXPAND_ENV_VARS;
                xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
                if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST {
                    *complp = EXPAND_ENV_VARS;
                }
            }
        }

        if crate::memory::cstr_first_byte(xp.xp_pattern) == b'~' {
            p = xp.xp_pattern.wrapping_add(1);
            while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'/' {
                p = p.wrapping_add(1);
            }
            if crate::memory::cstr_is_empty(p)
                && (p as usize) > (xp.xp_pattern.wrapping_add(1) as usize)
                && match_user(crate::memory::cstr_as_bytes(xp.xp_pattern.wrapping_add(1))) >= 1
            {
                xp.xp_context = EXPAND_USER;
                xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
            }
        }
    }
}

/// Set the completion context for the "++opt=arg" argument. Always returns NULL.
fn set_context_in_argopt(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        let p = vim_strchr(arg, b'=' as i32);
        if p.is_null() {
            xp.xp_pattern = arg.cast_mut();
        } else {
            xp.xp_pattern = p.wrapping_add(1);
        }

        xp.xp_context = EXPAND_ARGOPT;
        ptr::null()
    }
}

/// Set the completion context for the :filter command.
fn set_context_in_filter_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        if !crate::memory::cstr_is_empty(arg) {
            arg = skip_vimgrep_pat(arg.cast_mut(), None, None);
        }
        if crate::memory::cstr_is_empty(arg) {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }
        skipwhite(arg)
    }
}

/// Set the completion context for the :match command.
fn set_context_in_match_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(arg) || ends_excmd(crate::memory::cstr_first_char(arg)) == 0
        {
            set_context_in_echohl_cmd(xp, arg);
            arg = skipwhite(skiptowhite(arg));
            if !crate::memory::cstr_is_empty(arg) {
                xp.xp_context = EXPAND_NOTHING;
                arg = skip_regexp(
                    arg.wrapping_add(1).cast_mut(),
                    crate::memory::cstr_first_byte(arg) as i32,
                    opt_vars().p_magic,
                );
            }
        }
        find_nextcmd(arg)
    }
}

/// Returns a pointer to the next command after a :global or a :v command.
fn find_cmd_after_global_cmd(arg: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        let delim = crate::memory::cstr_first_byte(arg) as i32;
        if delim != 0 {
            arg = arg.wrapping_add(1);
        }

        while !crate::memory::cstr_is_empty(arg)
            && (crate::memory::cstr_first_byte(arg) as i32) != delim
        {
            if crate::memory::cstr_first_byte(arg) == b'\\'
                && crate::memory::cstr_byte_at(arg, 1) != 0
            {
                arg = arg.wrapping_add(1);
            }
            arg = arg.wrapping_add(1);
        }
        if !crate::memory::cstr_is_empty(arg) {
            return arg.wrapping_add(1);
        }

        ptr::null()
    }
}

/// Returns a pointer to the next command after a :substitute or a :& command.
fn find_cmd_after_substitute_cmd(arg: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        let delim = crate::memory::cstr_first_byte(arg) as i32;
        if delim != 0 {
            arg = arg.wrapping_add(1);
            arg = skip_regexp(arg.cast_mut(), delim, opt_vars().p_magic);

            if !crate::memory::cstr_is_empty(arg)
                && (crate::memory::cstr_first_byte(arg) as i32) == delim
            {
                arg = arg.wrapping_add(1);
                while !crate::memory::cstr_is_empty(arg)
                    && (crate::memory::cstr_first_byte(arg) as i32) != delim
                {
                    if crate::memory::cstr_first_byte(arg) == b'\\'
                        && crate::memory::cstr_byte_at(arg, 1) != 0
                    {
                        arg = arg.wrapping_add(1);
                    }
                    arg = arg.wrapping_add(1);
                }
                if !crate::memory::cstr_is_empty(arg) {
                    arg = arg.wrapping_add(1);
                }
            }
        }
        while !crate::memory::cstr_is_empty(arg) && c_strchr(c"|\"#", unsafe { *arg }).is_null() {
            arg = arg.wrapping_add(1);
        }
        if !crate::memory::cstr_is_empty(arg) {
            return arg;
        }

        ptr::null()
    }
}

/// Returns a pointer to the next command after an :isearch/:dsearch/etc. command.
fn find_cmd_after_isearch_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        arg = skipwhite(skipdigits(arg));
        if crate::memory::cstr_first_byte(arg) != b'/' {
            return ptr::null();
        }

        arg = arg.wrapping_add(1);
        while !crate::memory::cstr_is_empty(arg) && crate::memory::cstr_first_byte(arg) != b'/' {
            if crate::memory::cstr_first_byte(arg) == b'\\'
                && crate::memory::cstr_byte_at(arg, 1) != 0
            {
                arg = arg.wrapping_add(1);
            }
            arg = arg.wrapping_add(1);
        }
        if !crate::memory::cstr_is_empty(arg) {
            arg = skipwhite(arg.wrapping_add(1));

            if crate::memory::cstr_is_empty(arg) || c_strchr(c"|\"\n", unsafe { *arg }).is_null() {
                xp.xp_context = EXPAND_NOTHING;
            } else {
                return arg;
            }
        }

        ptr::null()
    }
}

/// Set the completion context for the :unlet command. Always returns NULL.
fn set_context_in_unlet_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let mut arg = arg.into_c_char_ptr();
    {
        while {
            xp.xp_pattern = c_strchr(arg, b' ' as c_char);
            !xp.xp_pattern.is_null()
        } {
            arg = xp.xp_pattern.wrapping_add(1);
        }

        xp.xp_context = EXPAND_USER_VARS;
        xp.xp_pattern = arg.cast_mut();

        if crate::memory::cstr_first_byte(xp.xp_pattern) == b'$' {
            xp.xp_context = EXPAND_ENV_VARS;
            xp.xp_pattern = xp.xp_pattern.wrapping_add(1);
        }

        ptr::null()
    }
}

/// Set the completion context for the :language command. Always returns NULL.
fn set_context_in_lang_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        let p = skiptowhite(arg);
        if crate::memory::cstr_is_empty(p) {
            xp.xp_context = EXPAND_LANGUAGE;
            xp.xp_pattern = arg.cast_mut();
        } else {
            let n = crate::memory::ptr_offset_from(p, arg) as usize;
            if (crate::memory::cstr_prefix_bytes(arg, n)
                == crate::memory::cstr_prefix_bytes(c"messages", n))
                || (crate::memory::cstr_prefix_bytes(arg, n)
                    == crate::memory::cstr_prefix_bytes(c"ctype", n))
                || (crate::memory::cstr_prefix_bytes(arg, n)
                    == crate::memory::cstr_prefix_bytes(c"time", n))
                || (crate::memory::cstr_prefix_bytes(arg, n)
                    == crate::memory::cstr_prefix_bytes(c"collate", n))
            {
                xp.xp_context = EXPAND_LOCALES;
                xp.xp_pattern = skipwhite(p);
            } else {
                xp.xp_context = EXPAND_NOTHING;
            }
        }

        ptr::null()
    }
}

/// Set the completion context for the :breakadd command. Always returns NULL.
fn set_context_in_breakadd_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
    cmdidx: Cmdidx,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        xp.xp_context = EXPAND_BREAKPOINT;
        xp.xp_pattern = arg.cast_mut();

        if cmdidx == Cmdidx::CmdBreakadd {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Add;
        } else if cmdidx == Cmdidx::CmdBreakdel {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Del;
        } else {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Profdel;
        }

        let mut p: *const c_char = skipwhite(arg);
        if crate::memory::cstr_is_empty(p) {
            return ptr::null();
        }
        let subcmd_start = p;

        if (b"file " == crate::memory::cstr_prefix_bytes(p, 5))
            || (b"func " == crate::memory::cstr_prefix_bytes(p, 5))
        {
            p = p.wrapping_add(4);
            p = skipwhite(p);

            if ascii_isdigit(crate::memory::cstr_first_char(p)) {
                p = skipdigits(p);
                if crate::memory::cstr_first_byte(p) != b' ' {
                    xp.xp_context = EXPAND_NOTHING;
                    return ptr::null();
                }
                p = skipwhite(p);
            }
            if b"file" == crate::memory::cstr_prefix_bytes(subcmd_start, 4) {
                xp.xp_context = EXPAND_FILES;
            } else {
                xp.xp_context = EXPAND_USER_FUNC;
            }
            xp.xp_pattern = p.cast_mut();
        } else if b"expr " == crate::memory::cstr_prefix_bytes(p, 5) {
            xp.xp_context = EXPAND_EXPRESSION;
            xp.xp_pattern = skipwhite(p.wrapping_add(5));
        }

        ptr::null()
    }
}

fn set_context_in_scriptnames_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        xp.xp_context = EXPAND_NOTHING;
        xp.xp_pattern = ptr::null_mut();

        let p = skipwhite(arg);
        if ascii_isdigit(crate::memory::cstr_first_char(p)) {
            return ptr::null();
        }

        xp.xp_context = EXPAND_SCRIPTNAMES;
        xp.xp_pattern = p;

        ptr::null()
    }
}

/// Set the completion context for the :filetype command. Always returns NULL.
fn set_context_in_filetype_cmd(
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        xp.xp_context = EXPAND_FILETYPECMD;
        xp.xp_pattern = arg.cast_mut();
        cmdexp().filetype_expand_what = FiletypeExpandWhat::All;

        let mut p = skipwhite(arg);
        if crate::memory::cstr_is_empty(p) {
            return ptr::null();
        }

        let mut val: i32 = 0;

        loop {
            if (crate::memory::cstr_prefix_bytes(p, 6) == &b"plugin"[..6])
                && ascii_iswhite(crate::memory::cstr_char_at(p, 6))
            {
                val |= EXPAND_FILETYPECMD_PLUGIN;
                p = skipwhite(p.wrapping_add(6));
                continue;
            }
            if (crate::memory::cstr_prefix_bytes(p, 6) == &b"indent"[..6])
                && ascii_iswhite(crate::memory::cstr_char_at(p, 6))
            {
                val |= EXPAND_FILETYPECMD_INDENT;
                p = skipwhite(p.wrapping_add(6));
                continue;
            }
            break;
        }

        if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 && (val & EXPAND_FILETYPECMD_INDENT) != 0 {
            cmdexp().filetype_expand_what = FiletypeExpandWhat::OnOff;
        } else if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 {
            cmdexp().filetype_expand_what = FiletypeExpandWhat::Indent;
        } else if (val & EXPAND_FILETYPECMD_INDENT) != 0 {
            cmdexp().filetype_expand_what = FiletypeExpandWhat::Plugin;
        }

        xp.xp_pattern = p;

        ptr::null()
    }
}

/// Sets the completion context for commands that involve a search pattern and a line range.
fn set_context_with_pattern(xp: &mut ExpandCtx) {
    let ccline = get_cmdline_info();

    g_st().emsg_off += 1;
    let mut skiplen: i32 = 0;
    let mut dummy: i32 = 0;
    let mut patlen: i32 = 0;
    let mut magic = opt_vars().p_magic != 0;
    let retval = parse_pattern_and_range(
        &mut cmdexp().pre_incsearch_pos,
        &mut dummy,
        &mut skiplen,
        &mut patlen,
        &mut magic,
    );
    g_st().emsg_off -= 1;

    if !retval || ccline.cmdpos <= skiplen || ccline.cmdpos > skiplen + patlen {
        return;
    }

    xp.xp_pattern = ccline.cmdbuff.wrapping_add(skiplen as usize);
    xp.xp_pattern_len = (ccline.cmdpos - skiplen) as usize;
    xp.xp_context = EXPAND_PATTERN_IN_BUF;
    xp.xp_search_dir = FORWARD;
}

/// Set the completion context in "xp" for command "cmd" with index "cmdidx".
fn set_context_by_cmdname(
    cmd: impl crate::strings::IntoCCharPtr,
    cmdidx: Cmdidx,
    xp: &mut ExpandCtx,
    arg: impl crate::strings::IntoCCharPtr,
    argt: u32,
    context: i32,
    forceit: bool,
) -> *const c_char {
    let cmd = cmd.into_c_char_ptr();
    let mut arg = arg.into_c_char_ptr();
    {
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
                xp.xp_pattern = arg.cast_mut();
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
                return arg;
            }
            Cmdidx::CmdFilter => {
                return set_context_in_filter_cmd(xp, arg);
            }
            Cmdidx::CmdMatch => {
                return set_context_in_match_cmd(xp, arg);
            }
            Cmdidx::CmdCommand => {
                return set_context_in_user_cmd(xp, arg);
            }
            Cmdidx::CmdDelcommand => {
                xp.xp_context = EXPAND_USER_COMMANDS;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdGlobal | Cmdidx::CmdVglobal => {
                let nextcmd = find_cmd_after_global_cmd(arg);
                if nextcmd.is_null() && cmdexp().may_expand_pattern {
                    set_context_with_pattern(xp);
                }
                return nextcmd;
            }
            Cmdidx::CmdAnd | Cmdidx::CmdSubstitute => {
                let nextcmd = find_cmd_after_substitute_cmd(arg);
                if nextcmd.is_null() && cmdexp().may_expand_pattern {
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
                return find_cmd_after_isearch_cmd(xp, arg);
            }
            Cmdidx::CmdAutocmd => {
                return aucmd_set_expand_context(xp, arg.cast_mut(), false);
            }
            Cmdidx::CmdDoautocmd | Cmdidx::CmdDoautoall => {
                return aucmd_set_expand_context(xp, arg.cast_mut(), true);
            }
            Cmdidx::CmdSet => {
                set_context_in_set_cmd(xp, arg.cast_mut(), 0);
            }
            Cmdidx::CmdSetglobal => {
                set_context_in_set_cmd(xp, arg.cast_mut(), OPT_GLOBAL);
            }
            Cmdidx::CmdSetlocal => {
                set_context_in_set_cmd(xp, arg.cast_mut(), OPT_LOCAL);
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
                if (opt_vars().wop_flags & K_OPT_WOP_FLAG_TAGFILE) != 0 {
                    xp.xp_context = EXPAND_TAGS_LISTFILES;
                } else {
                    xp.xp_context = EXPAND_TAGS;
                }
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdAugroup => {
                xp.xp_context = EXPAND_AUGROUP;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdSyntax => {
                set_context_in_syntax_cmd(xp, arg);
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
                set_context_for_expression(xp, arg.cast_mut(), cmdidx);
            }
            Cmdidx::CmdUnlet => {
                return set_context_in_unlet_cmd(xp, arg);
            }
            Cmdidx::CmdFunction | Cmdidx::CmdDelfunction => {
                xp.xp_context = EXPAND_USER_FUNC;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdEchohl => {
                set_context_in_echohl_cmd(xp, arg);
            }
            Cmdidx::CmdHighlight => {
                set_context_in_highlight_cmd(xp, arg);
            }
            Cmdidx::CmdSign => {
                set_context_in_sign_cmd(xp, arg.cast_mut());
            }
            Cmdidx::CmdBdelete | Cmdidx::CmdBwipeout | Cmdidx::CmdBunload => {
                while {
                    xp.xp_pattern = c_strchr(arg, b' ' as c_char);
                    !xp.xp_pattern.is_null()
                } {
                    arg = xp.xp_pattern.wrapping_add(1);
                }
                xp.xp_context = EXPAND_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdBuffer | Cmdidx::CmdSbuffer | Cmdidx::CmdPbuffer | Cmdidx::CmdChecktime => {
                xp.xp_context = EXPAND_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdDiffget | Cmdidx::CmdDiffput => {
                xp.xp_context = EXPAND_DIFF_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdUser | Cmdidx::CmdUserBuf => {
                return set_context_in_user_cmdarg(cmd, arg, argt, context, xp, forceit);
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
                return set_context_in_map_cmd(
                    xp,
                    cmd.cast_mut(),
                    arg.cast_mut(),
                    forceit,
                    false,
                    false,
                    cmdidx,
                );
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
                return set_context_in_map_cmd(
                    xp,
                    cmd.cast_mut(),
                    arg.cast_mut(),
                    forceit,
                    false,
                    true,
                    cmdidx,
                );
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
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdAbbreviate
            | Cmdidx::CmdNoreabbrev
            | Cmdidx::CmdCabbrev
            | Cmdidx::CmdCnoreabbrev
            | Cmdidx::CmdIabbrev
            | Cmdidx::CmdInoreabbrev => {
                return set_context_in_map_cmd(
                    xp,
                    cmd.cast_mut(),
                    arg.cast_mut(),
                    forceit,
                    true,
                    false,
                    cmdidx,
                );
            }
            Cmdidx::CmdUnabbreviate | Cmdidx::CmdCunabbrev | Cmdidx::CmdIunabbrev => {
                return set_context_in_map_cmd(
                    xp,
                    cmd.cast_mut(),
                    arg.cast_mut(),
                    forceit,
                    true,
                    true,
                    cmdidx,
                );
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
                set_context_in_menu_cmd(xp, cmd, arg, forceit);
                return ptr::null_mut();
            }
            Cmdidx::CmdColorscheme => {
                xp.xp_context = EXPAND_COLORS;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdCompiler => {
                xp.xp_context = EXPAND_COMPILER;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdOwnsyntax => {
                xp.xp_context = EXPAND_OWNSYNTAX;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdSetfiletype => {
                xp.xp_context = EXPAND_FILETYPE;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdPackadd => {
                xp.xp_context = EXPAND_PACKADD;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdRuntime => {
                set_context_in_runtime_cmd(xp, arg);
            }
            Cmdidx::CmdLanguage => {
                return set_context_in_lang_cmd(xp, arg);
            }
            Cmdidx::CmdProfile => {
                set_context_in_profile_cmd(xp, arg);
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
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdMessages => {
                xp.xp_context = EXPAND_MESSAGES;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdHistory => {
                xp.xp_context = EXPAND_HISTORY;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdSyntime => {
                xp.xp_context = EXPAND_SYNTIME;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdArgdelete => {
                while {
                    xp.xp_pattern = vim_strchr(arg, b' ' as i32);
                    !xp.xp_pattern.is_null()
                } {
                    arg = xp.xp_pattern.wrapping_add(1);
                }
                xp.xp_context = EXPAND_ARGLIST;
                xp.xp_pattern = arg.cast_mut();
            }
            Cmdidx::CmdBreakadd | Cmdidx::CmdProfdel | Cmdidx::CmdBreakdel => {
                return set_context_in_breakadd_cmd(xp, arg, cmdidx);
            }
            Cmdidx::CmdScriptnames => {
                return set_context_in_scriptnames_cmd(xp, arg);
            }
            Cmdidx::CmdFiletype => {
                return set_context_in_filetype_cmd(xp, arg);
            }
            Cmdidx::CmdLua | Cmdidx::CmdEqual => {
                xp.xp_context = EXPAND_LUA;
            }
            _ => {}
        }
        ptr::null()
    }
}

/// Parse one command from `buff` and set completion context in `xp`.
fn set_one_cmd_context(
    xp: &mut ExpandCtx,
    buff: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let buff = buff.into_c_char_ptr();
    {
        let mut ea: Exarg = Exarg::new();
        let mut context: i32 = EXPAND_NOTHING;
        let mut forceit = false;
        let mut usefilter = false;

        expand_init(xp);
        xp.xp_pattern = buff.cast_mut();
        xp.xp_line = buff.cast_mut();
        xp.xp_context = EXPAND_COMMANDS;
        ea.argt = 0;

        let mut cmd = buff;
        while crate::strings::vim_haschr(c" \t:|", crate::memory::cstr_first_byte(cmd) as i32) {
            cmd = cmd.wrapping_add(1);
        }
        xp.xp_pattern = cmd.cast_mut();

        if crate::memory::cstr_is_empty(cmd) {
            return ptr::null();
        }
        if crate::memory::cstr_first_byte(cmd) == b'"' {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        cmd = skip_range(cmd, Some(&mut xp.xp_context));
        xp.xp_pattern = cmd.cast_mut();
        if crate::memory::cstr_is_empty(cmd) {
            return ptr::null();
        }
        if crate::memory::cstr_first_byte(cmd) == b'"' {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        if crate::memory::cstr_first_byte(cmd) == b'|'
            || crate::memory::cstr_first_byte(cmd) == b'\n'
        {
            return cmd.wrapping_add(1);
        }

        let mut p = set_cmd_index(cmd, &mut ea, xp, &mut context);
        if p.is_null() {
            return ptr::null();
        }

        xp.xp_context = EXPAND_NOTHING;

        if crate::memory::cstr_first_byte(p) == b'!' {
            forceit = true;
            p = p.wrapping_add(1);
        }

        if !is_user_cmdidx(Cmdidx::from_raw(ea.cmdidx)) {
            ea.argt = excmd_get_argt(Cmdidx::from_raw(ea.cmdidx));
        }

        let mut arg: *const c_char = skipwhite(p);

        if (ea.argt & EX_ARGOPT) != 0 {
            while !crate::memory::cstr_is_empty(arg)
                && (crate::memory::cstr_prefix_bytes(arg, 2) == &b"++"[..2])
            {
                p = arg.wrapping_add(2);
                while !crate::memory::cstr_is_empty(p)
                    && !ascii_isspace(crate::memory::cstr_first_char(p))
                {
                    p = p.wrapping_add(utfc_ptr2len(p) as usize);
                }

                if crate::memory::cstr_is_empty(p) && (ea.argt & EX_ARGOPT) != 0 {
                    return set_context_in_argopt(xp, arg.wrapping_add(2));
                }

                arg = skipwhite(p);
            }
        }

        if ea.cmdidx == Cmdidx::CmdWrite as i32 || ea.cmdidx == Cmdidx::CmdUpdate as i32 {
            if crate::memory::cstr_first_byte(arg) == b'>' {
                arg = arg.wrapping_add(1);
                if crate::memory::cstr_first_byte(arg) == b'>' {
                    arg = arg.wrapping_add(1);
                }
                arg = skipwhite(arg);
            } else if crate::memory::cstr_first_byte(arg) == b'!'
                && ea.cmdidx == Cmdidx::CmdWrite as i32
            {
                arg = arg.wrapping_add(1);
                usefilter = true;
            }
        }

        if ea.cmdidx == Cmdidx::CmdRead as i32 {
            usefilter = forceit;
            if crate::memory::cstr_first_byte(arg) == b'!' {
                arg = arg.wrapping_add(1);
                usefilter = true;
            }
        }

        if ea.cmdidx == Cmdidx::CmdLshift as i32 || ea.cmdidx == Cmdidx::CmdRshift as i32 {
            while unsafe { *arg } == unsafe { *cmd } {
                arg = arg.wrapping_add(1);
            }
            arg = skipwhite(arg);
        }

        if (ea.argt & EX_CMDARG) != 0 && !usefilter && crate::memory::cstr_first_byte(arg) == b'+' {
            p = arg.wrapping_add(1);
            arg = skip_cmd_arg(arg.cast_mut(), false);

            if crate::memory::cstr_is_empty(arg) {
                return p;
            }

            arg = skipwhite(arg);
        }

        if (ea.argt & EX_TRLBAR) != 0 && !usefilter {
            p = arg;
            if ea.cmdidx == Cmdidx::CmdRedir as i32
                && crate::memory::cstr_first_byte(p) == b'@'
                && crate::memory::cstr_byte_at(p, 1) == b'"'
            {
                p = p.wrapping_add(2);
            }
            while !crate::memory::cstr_is_empty(p) {
                if unsafe { *p } == CTRL_V_CONST {
                    if crate::memory::cstr_byte_at(p, 1) != 0 {
                        p = p.wrapping_add(1);
                    }
                } else if ((crate::memory::cstr_first_byte(p) == b'"'
                    && (ea.argt & EX_NOTRLCOM) == 0)
                    || crate::memory::cstr_first_byte(p) == b'|'
                    || crate::memory::cstr_first_byte(p) == b'\n')
                    && unsafe { *p.wrapping_sub(1) } != b'\\' as c_char
                {
                    if crate::memory::cstr_first_byte(p) == b'|'
                        || crate::memory::cstr_first_byte(p) == b'\n'
                    {
                        return p.wrapping_add(1);
                    }
                    return ptr::null();
                }
                p = p.wrapping_add(utfc_ptr2len(p) as usize);
            }
        }

        if (ea.argt & EX_EXTRA) == 0
            && !crate::memory::cstr_is_empty(arg)
            && c_strchr(c"|\"", unsafe { *arg }).is_null()
        {
            return ptr::null();
        }

        p = buff;
        xp.xp_pattern = p.cast_mut();
        let len = crate::memory::cstr_len(buff);
        while !crate::memory::cstr_is_empty(p) && (p as usize) < (buff.wrapping_add(len) as usize) {
            if crate::memory::cstr_first_byte(p) == b' '
                || crate::memory::cstr_first_byte(p) == TAB
            {
                p = p.wrapping_add(1);
                xp.xp_pattern = p.cast_mut();
            } else {
                if crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) != 0
                {
                    p = p.wrapping_add(1);
                }
                p = p.wrapping_add(utfc_ptr2len(p) as usize);
            }
        }

        if (ea.argt & EX_XFILE) != 0 {
            set_context_for_wildcard_arg(Some(&ea), arg, usefilter, xp, &mut context);
        }

        set_context_by_cmdname(
            cmd,
            Cmdidx::from_raw(ea.cmdidx),
            xp,
            arg,
            ea.argt,
            context,
            forceit,
        )
    }
}

/// Set the completion context in "xp" for command "str".
pub fn set_cmd_context(
    xp: &mut ExpandCtx,
    str_arg: *mut c_char,
    len: i32,
    col: i32,
    use_ccline: i32,
) {
    let ccline = get_cmdline_info();
    let mut old_char: c_char = NUL;

    if col < len {
        old_char = unsafe { *str_arg.wrapping_add(col as usize) };
        unsafe {
            *str_arg.wrapping_add(col as usize) = NUL;
        }
    }
    let mut nextcomm: *const c_char = str_arg;

    cmdexp().filetype_expand_what = FiletypeExpandWhat::All;
    cmdexp().breakpt_expand_what = BreakptExpandWhat::Add;

    if use_ccline != 0 && ccline.cmdfirstc == b'=' as i32 {
        set_context_for_expression(xp, str_arg, Cmdidx::CmdSize);
    } else if use_ccline != 0 && ccline.input_fn != 0 {
        xp.xp_context = ccline.xp_context;
        xp.xp_pattern = ccline.cmdbuff;
        xp.xp_arg = ccline
            .xp_arg
            .as_ref()
            .map_or(ptr::null_mut(), |a| a.as_ptr().cast_mut());
        if xp.xp_context == EXPAND_SHELLCMDLINE {
            let mut context = xp.xp_context;
            set_context_for_wildcard_arg(None, xp.xp_pattern, false, xp, &mut context);
        }
    } else {
        while !nextcomm.is_null() {
            nextcomm = set_one_cmd_context(xp, nextcomm);
        }
    }

    xp.xp_line = str_arg;
    xp.xp_col = col;

    if col < len {
        unsafe {
            *str_arg.wrapping_add(col as usize) = old_char;
        }
    }
}

/// Expand the command line "str" from context "xp".
pub fn expand_cmdline(
    xp: &mut ExpandCtx,
    str_arg: impl crate::strings::IntoCCharPtr,
    col: i32,
    matches: &mut Vec<CString>,
) -> i32 {
    let str_arg = str_arg.into_c_char_ptr();
    {
        let mut options = WILD_ADD_SLASH | WILD_SILENT;

        if xp.xp_context == EXPAND_UNSUCCESSFUL {
            beep_flush();
            return EXPAND_UNSUCCESSFUL;
        }
        if xp.xp_context == EXPAND_NOTHING {
            return EXPAND_NOTHING;
        }

        debug_assert!(
            crate::memory::ptr_offset_from(str_arg.wrapping_add(col as usize), xp.xp_pattern) >= 0
        );
        xp.xp_pattern_len =
            crate::memory::ptr_offset_from(str_arg.wrapping_add(col as usize), xp.xp_pattern)
                as usize;
        let file_str = if cmdline_fuzzy_completion_supported(xp) {
            cstr_to_cstring(xp.xp_pattern)
        } else {
            addstar(xp.xp_pattern, xp.xp_pattern_len, xp.xp_context)
        };

        if opt_vars().p_wic != 0 {
            options += WILD_ICASE;
        }

        if !expand_from_context(xp, file_str.as_ptr().cast_mut(), matches, options) {
            matches.clear();
        }

        EXPAND_OK
    }
}

/// Expand file or directory names.
fn expand_files_and_dirs(
    xp: &mut ExpandCtx,
    pat: impl crate::strings::IntoCCharPtr,
    matches: &mut Vec<CString>,
    mut flags: i32,
    options: i32,
) -> bool {
    let mut pat = pat.into_c_char_ptr().cast_mut();
    {
        let mut pat_owned: Option<Vec<c_char>> = None;

        if xp.xp_backslash != XP_BS_NONE {
            let pat_len = crate::memory::cstr_len(pat);
            let mut pat_vec = vec![0 as c_char; pat_len + 1];
            crate::memory::cstr_ncpy(pat_vec.as_mut_ptr(), pat, pat_len);
            pat = pat_vec.as_mut_ptr();
            pat_owned = Some(pat_vec);

            let mut pat_end = pat.wrapping_add(pat_len);
            let mut p = pat;
            while !crate::memory::cstr_is_empty(p) {
                if crate::memory::cstr_first_byte(p) == b'\\' {
                    if (xp.xp_backslash & XP_BS_THREE) != 0
                        && crate::memory::cstr_byte_at(p, 1) == b'\\'
                        && crate::memory::cstr_byte_at(p, 2) == b'\\'
                        && crate::memory::cstr_byte_at(p, 3) == b' '
                    {
                        let from = p.wrapping_add(3);
                        unsafe {
                            ptr::copy(
                                from,
                                p,
                                (crate::memory::ptr_offset_from(pat_end, from) as usize) + 1,
                            )
                        };
                        pat_end = pat_end.wrapping_sub(3);
                    } else if (xp.xp_backslash & XP_BS_ONE) != 0
                        && crate::memory::cstr_byte_at(p, 1) == b' '
                    {
                        let from = p.wrapping_add(1);
                        unsafe {
                            ptr::copy(
                                from,
                                p,
                                (crate::memory::ptr_offset_from(pat_end, from) as usize) + 1,
                            )
                        };
                        pat_end = pat_end.wrapping_sub(1);
                    } else if (xp.xp_backslash & XP_BS_COMMA) != 0 {
                        if crate::memory::cstr_byte_at(p, 1) == b'\\'
                            && crate::memory::cstr_byte_at(p, 2) == b','
                        {
                            let from = p.wrapping_add(2);
                            unsafe {
                                ptr::copy(
                                    from,
                                    p,
                                    (crate::memory::ptr_offset_from(pat_end, from) as usize) + 1,
                                )
                            };
                            pat_end = pat_end.wrapping_sub(2);
                        } else {
                            #[cfg(windows)]
                            if crate::memory::cstr_byte_at(p, 1) == b',' {
                                let from = p.wrapping_add(1);
                                unsafe {
                                    ptr::copy(
                                        from,
                                        p,
                                        (crate::memory::ptr_offset_from(pat_end, from) as usize)
                                            + 1,
                                    )
                                };
                                pat_end = pat_end.wrapping_sub(1);
                            }
                        }
                    }
                }
                p = p.wrapping_add(1);
            }
        }

        let ret = if xp.xp_context == EXPAND_FINDFUNC {
            expand_findfunc(xp, pat, matches)
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
            expand_wildcards_eval(&raw mut pat, matches, flags)
        };
        drop(pat_owned);
        #[cfg(windows)]
        if ((options & WILD_USE_SHELLSLASH) != 0 && opt_vars().p_ssl == 0)
            || ((options & WILD_USE_COMPLETESLASH) != 0
                && ((crate::memory::cstr_is_empty(opt_vars().p_csl) && opt_vars().p_ssl == 0)
                    || crate::memory::cstr_first_byte(opt_vars().p_csl) == b'b'))
        {
            for slot in matches.iter_mut() {
                if slot.as_bytes().contains(&PATHSEP) {
                    let bytes: Vec<u8> = slot
                        .as_bytes()
                        .iter()
                        .map(|&b| if b == PATHSEP { b'\\' } else { b })
                        .collect();
                    *slot = cstr_from_bytes(&bytes);
                }
            }
        }
        ret
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":filetype".
fn get_filetypecmd_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx < 0 {
        return ptr::null_mut();
    }

    let i = idx as usize;
    if cmdexp().filetype_expand_what == FiletypeExpandWhat::All && i < 5 {
        const OPTS_ALL: [&CStr; 5] = [c"detect", c"indent", c"plugin", c"on", c"off"];
        return OPTS_ALL[i].as_ptr().cast_mut();
    }
    if cmdexp().filetype_expand_what == FiletypeExpandWhat::Plugin && i < 3 {
        const OPTS_PLUGIN: [&CStr; 3] = [c"plugin", c"on", c"off"];
        return OPTS_PLUGIN[i].as_ptr().cast_mut();
    }
    if cmdexp().filetype_expand_what == FiletypeExpandWhat::Indent && i < 3 {
        const OPTS_INDENT: [&CStr; 3] = [c"indent", c"on", c"off"];
        return OPTS_INDENT[i].as_ptr().cast_mut();
    }
    if cmdexp().filetype_expand_what == FiletypeExpandWhat::OnOff && i < 2 {
        const OPTS_ONOFF: [&CStr; 2] = [c"on", c"off"];
        return OPTS_ONOFF[i].as_ptr().cast_mut();
    }
    ptr::null_mut()
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":breakadd" / ":breakdel" / ":profdel".
fn get_breakadd_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if (0..=3).contains(&idx) {
        const OPTS: [&CStr; 4] = [c"expr", c"file", c"func", c"here"];
        let i = idx as usize;

        if cmdexp().breakpt_expand_what == BreakptExpandWhat::Add {
            return OPTS[i].as_ptr().cast_mut();
        } else if cmdexp().breakpt_expand_what == BreakptExpandWhat::Del {
            if idx <= 2 {
                return OPTS[i + 1].as_ptr().cast_mut();
            }
        } else if idx <= 1 {
            return OPTS[i + 1].as_ptr().cast_mut();
        }
    }
    ptr::null_mut()
}

/// Function given to ExpandGeneric() to obtain the possible arguments for ":scriptnames".
fn get_scriptnames_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if !script_id_valid(idx + 1) {
        return ptr::null_mut();
    }

    let si = script_item(idx + 1);
    let name_buff = g_st().name_buff.as_mut_ptr();
    home_replace(
        None,
        (*si).name_cstr().map_or(ptr::null(), CStr::as_ptr),
        name_buff,
        MAXPATHL,
        true,
    );
    name_buff
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":retab".
fn get_retab_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx == 0 {
        c"-indentonly".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":messages".
fn get_messages_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx == 0 {
        c"clear".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

fn get_mapclear_arg(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    if idx == 0 {
        c"<buffer>".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Completion for |:checkhealth| command.
fn get_healthcheck_names(_xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    let prompt_id = get_cmdline_last_prompt_id();
    if cmdexp().healthcheck_last_gen != prompt_id || cmdexp().healthcheck_last_gen == 0 {
        let a = Array::new();
        let mut err = Error::new();
        let code = c"return vim.health._complete()";
        let res = nlua_exec(
            NvimString {
                data: code.as_ptr().cast_mut(),
                size: code.to_bytes().len(),
            },
            ptr::null(),
            a,
            K_RET_OBJECT,
            None,
            &mut err,
        );
        api_clear_error(&mut err);
        api_free_object(cmdexp().healthcheck_names);
        cmdexp().healthcheck_names = res;
        cmdexp().healthcheck_last_gen = prompt_id;
    }

    if cmdexp().healthcheck_names.obj_type == ObjectType::ObjectTypeArray
        && idx >= 0
        && (idx as usize) < cmdexp().healthcheck_names.data.array().size
    {
        let item = unsafe {
            *cmdexp()
                .healthcheck_names
                .data
                .array()
                .items
                .wrapping_add(idx as usize)
        };
        if item.obj_type == ObjectType::ObjectTypeString {
            return item.data.string().data;
        }
    }
    ptr::null_mut()
}

/// Completes arg1 via Lua.
fn get_arg1_from_lua(lua: &std::ffi::CStr, xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    unsafe {
        let prompt_id = get_cmdline_last_prompt_id();
        let last_line_ptr = &raw const cmdexp().arg1_lua_last_xp_line;
        if (*last_line_ptr).as_ref().is_none_or(|s| {
            crate::memory::cstr_as_bytes(s.as_ptr()) != crate::memory::cstr_as_bytes(xp.xp_line)
        }) || cmdexp().arg1_lua_last_gen != prompt_id
        {
            cmdexp().arg1_lua_last_xp_line = Some(cstr_to_cstring(xp.xp_line));
            let mut args_items = [
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(xp.xp_pattern)),
                },
                Object {
                    obj_type: ObjectType::ObjectTypeString,
                    data: ObjectData::String(cstr_as_string(xp.xp_line)),
                },
            ];
            let args = Array {
                size: 2,
                capacity: 2,
                items: args_items.as_mut_ptr(),
            };
            let mut err = Error::new();
            let res = nlua_exec(
                cstr_as_string(lua.as_ptr()),
                ptr::null(),
                args,
                K_RET_OBJECT,
                None,
                &mut err,
            );
            api_clear_error(&mut err);
            api_free_object(cmdexp().arg1_lua_names);
            cmdexp().arg1_lua_names = res;
            cmdexp().arg1_lua_last_gen = prompt_id;
        }

        if cmdexp().arg1_lua_names.obj_type == ObjectType::ObjectTypeArray
            && idx >= 0
            && (idx as usize) < cmdexp().arg1_lua_names.data.array().size
        {
            let item = cmdexp().arg1_lua_names.data.array()[idx as usize];
            if item.obj_type == ObjectType::ObjectTypeString {
                return item.data.string().data;
            }
        }
        ptr::null_mut()
    }
}

fn get_marks_arg(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    get_arg1_from_lua(c"return require'vim._core.marks'.complete(...)", xp, idx)
}

fn get_log_arg(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.log_complete(...)",
        xp,
        idx,
    )
}

fn get_lsp_arg(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.lsp_complete(...)",
        xp,
        idx,
    )
}

fn get_packdel_arg(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.packdel_complete(...)",
        xp,
        idx,
    )
}

fn get_packupdate_arg(xp: &mut ExpandCtx, idx: i32) -> *mut c_char {
    get_arg1_from_lua(
        c"return require'vim._core.ex_cmd'.packupdate_complete(...)",
        xp,
        idx,
    )
}

/// Do the expansion based on xp->xp_context and "rmp".
fn expand_other(
    pat: impl crate::strings::IntoCCharPtr,
    xp: &mut ExpandCtx,
    rmp: &mut Regmatch,
    matches: &mut Vec<CString>,
) -> bool {
    let pat = pat.into_c_char_ptr();
    struct ExpGen {
        context: i32,
        func: fn(&mut ExpandCtx, i32) -> *mut c_char,
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
            func: |xp, idx| {
                get_history_arg(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_COMMANDS,
            func: |xp, idx| {
                get_user_commands(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_ADDR_TYPE,
            func: |xp, idx| {
                get_user_cmd_addr_type(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_CMD_FLAGS,
            func: |xp, idx| {
                get_user_cmd_flags(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_NARGS,
            func: |xp, idx| {
                get_user_cmd_nargs(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_COMPLETE,
            func: |xp, idx| {
                get_user_cmd_complete(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_VARS,
            func: |xp, idx| {
                get_user_var_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_FUNCTIONS,
            func: |xp, idx| {
                get_function_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER_FUNC,
            func: |xp, idx| {
                get_user_func_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_EXPRESSION,
            func: |xp, idx| {
                get_expr_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MENUS,
            func: |xp, idx| {
                get_menu_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_MENUNAMES,
            func: |xp, idx| {
                get_menu_names(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: false,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_SYNTAX,
            func: |xp, idx| {
                get_syntax_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_SYNTIME,
            func: |xp, idx| {
                get_syntime_arg(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_HIGHLIGHT,
            func: |xp, idx| {
                get_highlight_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_EVENTS,
            func: |xp, idx| {
                expand_get_event_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_AUGROUP,
            func: |xp, idx| {
                expand_get_augroup_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_SIGN,
            func: |xp, idx| {
                get_sign_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_PROFILE,
            func: |xp, idx| {
                get_profile_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_LANGUAGE,
            func: |xp, idx| {
                get_lang_arg(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_LOCALES,
            func: |xp, idx| get_locales(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_ENV_VARS,
            func: |xp, idx| get_env_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            ic: true,
            escaped: true,
        },
        ExpGen {
            context: EXPAND_USER,
            func: |xp, idx| get_users(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            ic: true,
            escaped: false,
        },
        ExpGen {
            context: EXPAND_ARGLIST,
            func: |xp, idx| {
                get_arglist_name(xp, idx).map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
            },
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
    pat: impl crate::strings::IntoCCharPtr,
    matches: &mut Vec<CString>,
    options: i32,
) -> bool {
    let mut pat = pat.into_c_char_ptr().cast_mut();
    {
        let mut regmatch: Regmatch = Regmatch::new();
        let flags = map_wildopts_to_ewflags(options);
        let fuzzy = cmdline_fuzzy_complete(pat) && cmdline_fuzzy_completion_supported(xp);

        if xp.xp_context == EXPAND_FILES
            || xp.xp_context == EXPAND_DIRECTORIES
            || xp.xp_context == EXPAND_FILES_IN_PATH
            || xp.xp_context == EXPAND_FINDFUNC
            || xp.xp_context == EXPAND_DIRS_IN_CDPATH
        {
            return expand_files_and_dirs(xp, pat, matches, flags, options);
        }

        matches.clear();
        if xp.xp_context == EXPAND_HELP {
            if find_help_tags(
                if crate::memory::cstr_is_empty(pat) {
                    c"help"
                } else {
                    crate::memory::cstr_from_ptr(pat)
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
            expand_shellcmd(pat, matches, flags);
            return true;
        }
        if xp.xp_context == EXPAND_OLD_SETTING {
            return expand_old_setting(matches);
        }
        if xp.xp_context == EXPAND_BUFFERS {
            return expand_bufnames(pat, matches, options);
        }
        if xp.xp_context == EXPAND_DIFF_BUFFERS {
            return expand_bufnames(pat, matches, options | BUF_DIFF_FILTER);
        }
        if xp.xp_context == EXPAND_TAGS || xp.xp_context == EXPAND_TAGS_LISTFILES {
            return expand_tags(xp.xp_context == EXPAND_TAGS, pat, matches);
        }
        if xp.xp_context == EXPAND_COLORS {
            return expand_rtdir(pat, DIP_START + DIP_OPT, matches, &[c"colors"]);
        }
        if xp.xp_context == EXPAND_COMPILER {
            return expand_rtdir(pat, 0, matches, &[c"compiler"]);
        }
        if xp.xp_context == EXPAND_OWNSYNTAX {
            return expand_rtdir(pat, 0, matches, &[c"syntax"]);
        }
        if xp.xp_context == EXPAND_FILETYPE {
            return expand_rtdir(pat, 0, matches, &[c"syntax", c"indent", c"ftplugin"]);
        }
        if xp.xp_context == EXPAND_KEYMAP {
            return expand_rtdir(pat, 0, matches, &[c"keymap"]);
        }
        if xp.xp_context == EXPAND_USER_LIST {
            return expand_user_list(xp, matches);
        }
        if xp.xp_context == EXPAND_USER_LUA {
            return expand_user_lua(xp, matches);
        }
        if xp.xp_context == EXPAND_PACKADD {
            return ExpandPackAddDir(pat, matches);
        }
        if xp.xp_context == EXPAND_RUNTIME {
            return expand_runtime_cmd(pat, matches);
        }
        if xp.xp_context == EXPAND_PATTERN_IN_BUF {
            return expand_pattern_in_buf(pat, xp.xp_search_dir, matches);
        }

        let mut tofree: Option<Vec<c_char>> = None;
        if xp.xp_context == EXPAND_USER_FUNC
            && (crate::memory::cstr_prefix_bytes(pat, 3) == &b"^s:"[..3])
        {
            let len = crate::memory::cstr_len(pat) + 20;
            let mut buf = vec![0 as c_char; len];
            vim_snprintf!(
                buf.as_mut_ptr(),
                len,
                c"^<SNR>\\d\\+_%s",
                pat.wrapping_add(3)
            );
            pat = buf.as_mut_ptr();
            tofree = Some(buf);
        }

        if xp.xp_context == EXPAND_LUA {
            return nlua_expand_get_matches(matches);
        }

        if !fuzzy {
            regmatch.regprog = vim_regcomp(pat, if opt_vars().p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                return false;
            }
            regmatch.rm_ic = ignorecase(pat) != 0;
        }

        let ret = if xp.xp_context == EXPAND_SETTINGS || xp.xp_context == EXPAND_BOOL_SETTINGS {
            expand_settings(xp, &mut regmatch, pat, matches, fuzzy)
        } else if xp.xp_context == EXPAND_STRING_SETTING {
            expand_string_setting(xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_SETTING_SUBTRACT {
            expand_setting_subtract(xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_MAPPINGS {
            expand_mappings(pat, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_ARGOPT {
            expand_argopt(pat, xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_USER_DEFINED {
            expand_user_defined(pat, xp, &mut regmatch, matches)
        } else {
            expand_other(pat, xp, &mut regmatch, matches)
        };

        if !fuzzy {
            vim_regfree(regmatch.regprog);
        }
        drop(tofree);

        ret
    }
}

/// Expand a list of names.
pub fn expand_generic(
    pat: impl crate::strings::IntoCCharPtr,
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<CString>,
    func: CompleteListItemGetter,
    escaped: bool,
) {
    let pat = pat.into_c_char_ptr();
    {
        let fuzzy = cmdline_fuzzy_complete(pat);
        matches.clear();

        let Some(func_fn) = func else {
            return;
        };

        let mut str_ga: Vec<CString> = Vec::new();
        let mut fuz_ga: Vec<FuzmatchStr> = Vec::new();

        let mut i: i32 = 0;
        loop {
            let str_item = func_fn(xp, i);
            i += 1;
            if str_item.is_null() {
                break;
            }
            if crate::memory::cstr_is_empty(str_item) {
                continue;
            }

            let match_ok: bool;
            let mut score: i32 = 0;
            if !crate::memory::cstr_is_empty(xp.xp_pattern) {
                if !fuzzy {
                    match_ok = vim_regexec(regmatch, str_item, 0);
                } else {
                    score = fuzzy_match_str(str_item, pat);
                    match_ok = score != FUZZY_SCORE_NONE;
                }
            } else {
                match_ok = true;
            }

            if !match_ok {
                continue;
            }

            let mut item_cstr = if escaped {
                vim_strsave_escaped(str_item, c" \t\\.")
            } else {
                cstr_to_cstring(str_item)
            };

            if xp.xp_context == EXPAND_MENUNAMES && item_cstr.as_bytes().last() == Some(&1) {
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
}

/// Expand shell command matches in one directory of $PATH.
fn expand_shellcmd_onedir(
    pathed_pattern: impl crate::strings::IntoCCharPtr,
    pathlen: usize,
    flags: i32,
    found_ht: &mut std::collections::HashSet<Vec<u8>>,
    gap: &mut Vec<CString>,
) {
    let mut pathed_pattern = pathed_pattern.into_c_char_ptr().cast_mut();
    let mut dir_matches: Vec<std::ffi::CString> = Vec::new();
    if !expand_wildcards_vec(1, &raw mut pathed_pattern, &mut dir_matches, flags) {
        return;
    }

    gap.reserve(dir_matches.len());

    for name in dir_matches {
        let name_bytes = name.as_bytes();
        let namelen = name_bytes.len();

        if namelen > pathlen {
            let tail_bytes = &name_bytes[pathlen..];
            if found_ht.insert(tail_bytes.to_vec()) {
                gap.push(cstr_from_bytes(tail_bytes));
            }
        }
    }
}

/// Complete a shell command.
fn expand_shellcmd(
    filepat: impl crate::strings::IntoCCharPtr,
    matches: &mut Vec<CString>,
    flagsarg: i32,
) {
    let filepat = filepat.into_c_char_ptr();
    {
        matches.clear();
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();
        let mut flags = flagsarg;
        let mut did_curdir = false;

        let mut patlen = crate::memory::cstr_len(filepat);
        let mut pat_vec = vec![0 as c_char; patlen + 1];
        crate::memory::cstr_ncpy(pat_vec.as_mut_ptr(), filepat, patlen);
        let pat = pat_vec.as_mut_ptr();
        let mut e = pat.wrapping_add(patlen);
        let mut s = pat;
        while !crate::memory::cstr_is_empty(s) {
            if crate::memory::cstr_first_byte(s) == b'\\' {
                let p = s.wrapping_add(1);
                if crate::memory::cstr_first_byte(p) == b' ' {
                    unsafe { ptr::copy(p, s, (crate::memory::ptr_offset_from(e, p) as usize) + 1) };
                    e = e.wrapping_sub(1);
                }
            }
            s = s.wrapping_add(1);
        }
        patlen = crate::memory::ptr_offset_from(e, pat) as usize;

        flags |= EW_FILE | EW_EXEC | EW_SHELLCMD;

        let mut path_owned: Option<std::ffi::CString> = None;
        let path: *mut c_char = if crate::memory::cstr_first_byte(pat) == b'.'
            && (vim_ispathsep(crate::memory::cstr_char_at(pat, 1))
                || (crate::memory::cstr_byte_at(pat, 1) == b'.'
                    && vim_ispathsep(crate::memory::cstr_char_at(pat, 2))))
        {
            c".".as_ptr().cast_mut()
        } else {
            if !path_is_absolute(pat) {
                path_owned = vim_getenv(c"PATH");
            }
            path_owned
                .as_ref()
                .map_or(c"".as_ptr().cast_mut(), |c| c.as_ptr().cast_mut())
        };

        let mut found_ht: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        s = path;
        loop {
            let mut pathlen: usize;
            let seplen: usize;

            if crate::memory::cstr_is_empty(s) {
                if did_curdir {
                    break;
                }

                did_curdir = true;
                flags |= EW_DIR;

                e = s;
                pathlen = 0;
                seplen = 0;
            } else {
                e = vim_strchr(s, ENV_SEPCHAR);
                if e.is_null() {
                    e = s.wrapping_add(crate::memory::cstr_len(s));
                }

                pathlen = crate::memory::ptr_offset_from(e, s) as usize;
                if crate::memory::cstr_prefix_bytes(s, pathlen)
                    == crate::memory::cstr_prefix_bytes(c".", pathlen)
                {
                    did_curdir = true;
                    flags |= EW_DIR;
                } else {
                    flags &= !EW_DIR;
                }

                seplen = if after_pathsep(s, e) == 0 {
                    PATHSEPSTR.len()
                } else {
                    0
                };
            }

            if pathlen + seplen + patlen < MAXPATHL {
                if pathlen > 0 {
                    crate::memory::cstr_ncpy(buf, s, pathlen);
                    if seplen > 0 {
                        crate::memory::cstr_ncpy(
                            buf.wrapping_add(pathlen),
                            PATHSEPSTR.as_ptr().cast(),
                            seplen,
                        );
                        pathlen += seplen;
                    }
                }
                crate::memory::cstr_ncpy(buf.wrapping_add(pathlen), pat, patlen);

                expand_shellcmd_onedir(buf, pathlen, flags, &mut found_ht, matches);
            }

            if !crate::memory::cstr_is_empty(e) {
                e = e.wrapping_add(1);
            }
            s = e;
        }
    }
}

/// Call "user_expand_func()" to invoke a user defined Vim script function.
fn call_user_expand_func<R>(
    user_expand_func: fn(*const c_char, i32, *mut TypVal) -> R,
    xp: *mut ExpandCtx,
    default: R,
) -> R {
    unsafe {
        let ccline = get_cmdline_info();
        let mut keep: c_char = 0;
        let mut args: [TypVal; 4] = [TypVal::new(); 4];
        let save_current_sctx = g_st().current_sctx;

        if crate::memory::cstr_is_empty((*xp).xp_arg) || (*xp).xp_line.is_null() {
            return default;
        }

        if !ccline.cmdbuff.is_null() {
            keep = *ccline.cmdbuff.wrapping_add(ccline.cmdlen as usize);
            *ccline.cmdbuff.wrapping_add(ccline.cmdlen as usize) = 0;
        }

        let pat = cstr_from_bytes(crate::memory::cstr_slice(
            (*xp).xp_pattern,
            (*xp).xp_pattern_len,
        ));
        args[0].v_type = VarType::VarString;
        args[1].v_type = VarType::VarString;
        args[2].v_type = VarType::VarNumber;
        args[3].v_type = VarType::VarUnknown;
        args[0].vval = crate::eval::typval::TypvalVval::String(pat.as_ptr().cast_mut());
        args[1].vval = crate::eval::typval::TypvalVval::String((*xp).xp_line);
        args[2].vval = crate::eval::typval::TypvalVval::Number(i64::from((*xp).xp_col));

        g_st().current_sctx = (*xp).xp_script_ctx;

        let ret = user_expand_func((*xp).xp_arg, 3, args.as_mut_ptr());

        g_st().current_sctx = save_current_sctx;
        if !ccline.cmdbuff.is_null() {
            *ccline.cmdbuff.wrapping_add(ccline.cmdlen as usize) = keep;
        }

        ret
    }
}

/// Expand names with a function defined by the user (EXPAND_USER_DEFINED).
fn expand_user_defined(
    pat: impl crate::strings::IntoCCharPtr,
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<CString>,
) -> bool {
    let pat = pat.into_c_char_ptr();
    {
        let fuzzy = cmdline_fuzzy_complete(pat);
        matches.clear();

        let Some(retstr_cstr) = call_user_expand_func(call_func_retstr, xp, None) else {
            return false;
        };
        let mut retstr_bytes = retstr_cstr.into_bytes_with_nul();
        let retstr = retstr_bytes.as_mut_ptr().cast::<c_char>();

        let mut str_ga: Vec<CString> = Vec::new();
        let mut fuz_ga: Vec<FuzmatchStr> = Vec::new();

        let mut s = retstr;
        while !crate::memory::cstr_is_empty(s) {
            let mut e = vim_strchr(s, b'\n' as i32);
            if e.is_null() {
                e = s.wrapping_add(crate::memory::cstr_len(s));
            }
            let keep = unsafe { *e };
            unsafe {
                *e = NUL;
            }

            let match_ok: bool;
            let mut score: i32 = 0;
            if !crate::memory::cstr_is_empty(xp.xp_pattern) {
                if !fuzzy {
                    match_ok = vim_regexec(regmatch, s, 0);
                } else {
                    score = fuzzy_match_str(s, pat);
                    match_ok = score != FUZZY_SCORE_NONE;
                }
            } else {
                match_ok = true;
            }

            unsafe {
                *e = keep;
            }

            if match_ok {
                let p = cstr_from_bytes(crate::memory::cstr_slice(
                    s,
                    crate::memory::ptr_offset_from(e, s) as usize,
                ));

                if !fuzzy {
                    str_ga.push(p);
                } else {
                    fuz_ga.push(FuzmatchStr {
                        idx: fuz_ga.len() as i32,
                        str_cstr: p,
                        score,
                    });
                }
            }

            if !crate::memory::cstr_is_empty(e) {
                e = e.wrapping_add(1);
            }
            s = e;
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
        for li in crate::eval::typval::list_iter_ptr(retlist.lv_first) {
            let tv = &li.li_tv;
            let p: CString;
            let mut abbr: Option<std::ffi::CString> = None;
            let mut kind: Option<std::ffi::CString> = None;
            let mut menu: Option<std::ffi::CString> = None;
            let mut info: Option<std::ffi::CString> = None;

            if tv.v_type == VarType::VarString {
                if tv.vval.v_string().is_null() {
                    continue;
                }
                p = cstr_to_cstring(tv.vval.v_string());
            } else if tv.v_type == VarType::VarDict && !tv.vval.v_dict().is_null() {
                let d = tv.vval.v_dict();
                let word = tv_dict_get_string(unsafe { d.as_ref() }, c"word");
                if word.is_null() {
                    continue;
                }
                p = cstr_to_cstring(word);
                abbr = crate::eval::typval::tv_dict_get_cstring(unsafe { d.as_ref() }, c"abbr");
                kind = crate::eval::typval::tv_dict_get_cstring(unsafe { d.as_ref() }, c"kind");
                menu = crate::eval::typval::tv_dict_get_cstring(unsafe { d.as_ref() }, c"menu");
                info = crate::eval::typval::tv_dict_get_cstring(unsafe { d.as_ref() }, c"info");
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
    let retlist = call_user_expand_func(call_func_retlist, xp, ptr::null_mut());
    if retlist.is_null() {
        return false;
    }

    expand_process_user_list(unsafe { retlist.as_ref() }, matches, xp);
    tv_list_unref(retlist);
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
    expand_process_user_list(unsafe { retlist.as_ref() }, matches, xp);
    tv_list_unref(retlist);
    true
}

/// Expand `file` for all comma-separated directories in `path`.
pub fn globpath(
    mut path: *mut c_char,
    file: impl crate::strings::IntoCCharPtr,
    ga: &mut Vec<CString>,
    expand_options: i32,
    dirs: bool,
) {
    let file = file.into_c_char_ptr();
    {
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();

        let mut xpc: ExpandCtx = ExpandCtx::new();
        xpc.xp_context = if dirs {
            EXPAND_DIRECTORIES
        } else {
            EXPAND_FILES
        };

        let filelen = crate::memory::cstr_len(file);

        while !crate::memory::cstr_is_empty(path) {
            let mut pathlen = copy_option_part(&raw mut path, buf, MAXPATHL, c",");
            let seplen = if !crate::memory::cstr_is_empty(buf)
                && after_pathsep(buf, buf.wrapping_add(pathlen)) == 0
            {
                PATHSEPSTR.len()
            } else {
                0
            };

            if pathlen + seplen + filelen < MAXPATHL {
                if seplen > 0 {
                    crate::memory::cstr_ncpy(
                        buf.wrapping_add(pathlen),
                        PATHSEPSTR.as_ptr().cast(),
                        seplen,
                    );
                    pathlen += seplen;
                }
                crate::memory::cstr_ncpy(buf.wrapping_add(pathlen), file, filelen);

                let mut p: Vec<CString> = Vec::new();
                expand_from_context(&mut xpc, buf, &mut p, WILD_SILENT | expand_options);
                if !p.is_empty() {
                    expand_escape(&mut xpc, buf, &mut p, WILD_SILENT | expand_options);
                    ga.extend(p);
                }
            }
        }
    }
}

/// Translate some keys pressed when 'wildmenu' is used.
pub fn wildmenu_translate_key(
    cclp: &mut CmdlineInfo,
    key: i32,
    xp: &mut ExpandCtx,
    did_wild_list: bool,
) -> i32 {
    let mut c = key;

    if cmdline_pum_active() || did_wild_list || g_st().wild_menu_showing != 0 {
        if c == K_LEFT {
            c = CTRL_P;
        } else if c == K_RIGHT {
            c = CTRL_N;
        }
    }

    if xp.xp_context == EXPAND_MENUNAMES
        && cclp.cmdpos > 1
        && unsafe { *cclp.cmdbuff.wrapping_add((cclp.cmdpos - 1) as usize) } == b'.' as c_char
        && unsafe { *cclp.cmdbuff.wrapping_add((cclp.cmdpos - 2) as usize) } != b'\\' as c_char
        && (c == b'\n' as i32 || c == b'\r' as i32 || c == K_KENTER)
    {
        c = K_DOWN;
    }

    c
}

/// Delete characters on the command line, from "from" to the current position.
fn cmdline_del(cclp: &mut CmdlineInfo, from: i32) {
    debug_assert!(cclp.cmdpos <= cclp.cmdlen);
    unsafe {
        ptr::copy(
            cclp.cmdbuff.wrapping_add(cclp.cmdpos as usize),
            cclp.cmdbuff.wrapping_add(from as usize),
            (cclp.cmdlen as usize) - (cclp.cmdpos as usize) + 1,
        )
    };
    cclp.cmdlen -= cclp.cmdpos - from;
    cclp.cmdpos = from;
}

/// Handle a key pressed when the wild menu for the menu names (EXPAND_MENUNAMES) is displayed.
fn wildmenu_process_key_menunames(cclp: &mut CmdlineInfo, mut key: i32, xp: &mut ExpandCtx) -> i32 {
    if key == K_DOWN
        && cclp.cmdpos > 0
        && unsafe { *cclp.cmdbuff.wrapping_add((cclp.cmdpos - 1) as usize) } == b'.' as c_char
    {
        key = opt_vars().p_wc as i32;
        g_st().key_typed = true;
    } else if key == K_UP {
        let mut found = false;

        let mut j = crate::memory::ptr_offset_from(xp.xp_pattern, cclp.cmdbuff) as i32;
        let mut i: i32 = 0;
        while {
            j -= 1;
            j > 0
        } {
            if crate::memory::cstr_byte_at(cclp.cmdbuff, j as usize) == b' '
                && unsafe { *cclp.cmdbuff.wrapping_add((j - 1) as usize) } != b'\\' as c_char
            {
                i = j + 1;
                break;
            }
            if crate::memory::cstr_byte_at(cclp.cmdbuff, j as usize) == b'.'
                && unsafe { *cclp.cmdbuff.wrapping_add((j - 1) as usize) } != b'\\' as c_char
            {
                if found {
                    i = j + 1;
                    break;
                } else {
                    found = true;
                }
            }
        }
        if i > 0 {
            cmdline_del(cclp, i);
        }
        key = opt_vars().p_wc as i32;
        g_st().key_typed = true;
        xp.xp_context = EXPAND_NOTHING;
    }

    key
}

/// Handle a key pressed when the wild menu for file/dir/shellcmd names is displayed.
fn wildmenu_process_key_filenames(cclp: &mut CmdlineInfo, mut key: i32, xp: &mut ExpandCtx) -> i32 {
    let upseg: [c_char; 5] = [
        PATHSEP as c_char,
        b'.' as c_char,
        b'.' as c_char,
        PATHSEP as c_char,
        NUL,
    ];

    if key == K_DOWN
        && cclp.cmdpos > 0
        && unsafe { *cclp.cmdbuff.wrapping_add((cclp.cmdpos - 1) as usize) } == PATHSEP as c_char
        && (cclp.cmdpos < 3
            || crate::memory::cstr_first_byte(
                cclp.cmdbuff.wrapping_add((cclp.cmdpos - 2) as usize),
            ) != b'.'
            || crate::memory::cstr_first_byte(
                cclp.cmdbuff.wrapping_add((cclp.cmdpos - 3) as usize),
            ) != b'.')
    {
        key = opt_vars().p_wc as i32;
        g_st().key_typed = true;
    } else if (crate::memory::cstr_prefix_bytes(xp.xp_pattern, 3)
        == crate::memory::cstr_prefix_bytes(upseg.as_ptr().wrapping_add(1), 3))
        && key == K_DOWN
    {
        let mut found = false;

        let mut j = cclp.cmdpos;
        let i = crate::memory::ptr_offset_from(xp.xp_pattern, cclp.cmdbuff) as i32;
        while {
            j -= 1;
            j > i
        } {
            j -= utf_head_off(cclp.cmdbuff, cclp.cmdbuff.wrapping_add(j as usize));
            if vim_ispathsep(crate::memory::cstr_char_at(cclp.cmdbuff, j as usize)) {
                found = true;
                break;
            }
        }
        if found
            && unsafe { *cclp.cmdbuff.wrapping_add((j - 1) as usize) } == b'.' as c_char
            && unsafe { *cclp.cmdbuff.wrapping_add((j - 2) as usize) } == b'.' as c_char
            && (vim_ispathsep(unsafe { *cclp.cmdbuff.wrapping_add((j - 3) as usize) } as i32)
                || j == i + 2)
        {
            cmdline_del(cclp, j - 2);
            key = opt_vars().p_wc as i32;
            g_st().key_typed = true;
        }
    } else if key == K_UP {
        let mut found = false;

        let mut j = cclp.cmdpos - 1;
        let mut i = crate::memory::ptr_offset_from(xp.xp_pattern, cclp.cmdbuff) as i32;
        while {
            j -= 1;
            j > i
        } {
            j -= utf_head_off(cclp.cmdbuff, cclp.cmdbuff.wrapping_add(j as usize));
            #[cfg(windows)]
            let is_sep = vim_ispathsep(crate::memory::cstr_char_at(cclp.cmdbuff, j as usize))
                && !crate::strings::vim_haschr(c" *?[{`$%#", unsafe {
                    *cclp.cmdbuff.wrapping_add((j + 1) as usize)
                } as u8 as i32);
            #[cfg(not(windows))]
            let is_sep = vim_ispathsep(crate::memory::cstr_char_at(cclp.cmdbuff, j as usize));

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
        } else if crate::memory::cstr_prefix_bytes(cclp.cmdbuff.wrapping_add(j as usize), 4)
            == crate::memory::cstr_prefix_bytes(upseg.as_ptr(), 4)
        {
            j += 4;
        } else if crate::memory::cstr_prefix_bytes(cclp.cmdbuff.wrapping_add(j as usize), 3)
            == crate::memory::cstr_prefix_bytes(upseg.as_ptr().wrapping_add(1), 3)
            && j == i
        {
            j += 3;
        } else {
            j = 0;
        }

        if j > 0 {
            cmdline_del(cclp, j);
            put_on_cmdline(upseg.as_ptr().wrapping_add(1), 3, false);
        } else if cclp.cmdpos > i {
            cmdline_del(cclp, i);
        }

        key = opt_vars().p_wc as i32;
        g_st().key_typed = true;
    }

    key
}

/// Handle a key pressed when wild menu is displayed.
pub fn wildmenu_process_key(cclp: &mut CmdlineInfo, key: i32, xp: &mut ExpandCtx) -> i32 {
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
pub fn wildmenu_cleanup(cclp: &mut CmdlineInfo) {
    if opt_vars().p_wmnu == 0 || g_st().wild_menu_showing == 0 {
        return;
    }

    let skt = g_st().key_typed;
    let old_redrawing_disabled = g_st().redrawing_disabled;

    if cclp.input_fn != 0 {
        g_st().redrawing_disabled = 0;
    }

    set_no_hlsearch(true);

    if g_st().wild_menu_showing == WM_SCROLLED {
        g_st().cmdline_row -= 1;
        redrawcmd();
        g_st().wild_menu_showing = 0;
    } else if g_st().save_p_ls != -1 {
        opt_vars().p_ls = OptInt::from(g_st().save_p_ls);
        opt_vars().p_wmh = OptInt::from(g_st().save_p_wmh);
        last_status(false);
        update_screen();
        redrawcmd();
        g_st().save_p_ls = -1;
        g_st().wild_menu_showing = 0;
    } else {
        win_redraw_last_status(g_st().topframe);
        g_st().wild_menu_showing = 0;
        redraw_statuslines();
    }
    g_st().key_typed = skt;
    if cclp.input_fn != 0 {
        g_st().redrawing_disabled = old_redrawing_disabled;
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

    if opt_vars().p_wic != 0 {
        options |= WILD_ICASE;
    }

    if !filtered {
        options |= WILD_KEEP_ALL;
    }

    if argvars[0].v_type != VarType::VarString {
        if !tv_get_string_chk(&argvars[0]).is_null() {
            emsg(gettext(E_INVARG));
        }
        return;
    }
    let pattern = tv_get_string(&argvars[0]);
    let mut pattern_start = pattern;

    if crate::memory::cstr_as_bytes(type_str) == b"cmdline" {
        let cmdline_len = crate::memory::cstr_len(pattern) as i32;
        set_cmd_context(
            &mut xpc,
            pattern.cast_mut(),
            cmdline_len,
            cmdline_len,
            false as i32,
        );
        pattern_start = xpc.xp_pattern;
        xpc.xp_pattern_len = crate::memory::cstr_len(xpc.xp_pattern);
        xpc.xp_col = cmdline_len;
    } else {
        xpc.xp_pattern = pattern.cast_mut();
        xpc.xp_pattern_len = crate::memory::cstr_len(xpc.xp_pattern);
        xpc.xp_line = pattern.cast_mut();

        xpc.xp_context = cmdcomplete_str_to_type(type_str);
        match xpc.xp_context {
            EXPAND_NOTHING => {
                semsg!(gettext(E_INVARG2), type_str);
                return;
            }
            EXPAND_USER_DEFINED => {
                if !(crate::memory::cstr_prefix_bytes(type_str, 7) == &b"custom,"[..7]) {
                    semsg!(gettext(E_INVARG2), type_str);
                    return;
                }
                xpc.xp_arg = type_str.wrapping_add(7).cast_mut();
            }
            EXPAND_USER_LIST => {
                if !(crate::memory::cstr_prefix_bytes(type_str, 11) == &b"customlist,"[..11]) {
                    semsg!(gettext(E_INVARG2), type_str);
                    return;
                }
                xpc.xp_arg = type_str.wrapping_add(11).cast_mut();
            }
            EXPAND_MENUS => {
                let pat_p = xpc.xp_pattern;
                set_context_in_menu_cmd(&mut xpc, c"menu", pat_p, false);
                xpc.xp_pattern_len -=
                    crate::memory::ptr_offset_from(xpc.xp_pattern, pattern_start) as usize;
            }
            EXPAND_SIGN => {
                let pat_p = xpc.xp_pattern;
                set_context_in_sign_cmd(&mut xpc, pat_p);
                xpc.xp_pattern_len -=
                    crate::memory::ptr_offset_from(xpc.xp_pattern, pattern_start) as usize;
            }
            EXPAND_RUNTIME => {
                let pat_p = xpc.xp_pattern;
                set_context_in_runtime_cmd(&mut xpc, pat_p);
                xpc.xp_pattern_len -=
                    crate::memory::ptr_offset_from(xpc.xp_pattern, pattern_start) as usize;
            }
            EXPAND_SHELLCMDLINE => {
                let mut context = EXPAND_SHELLCMDLINE;
                let pat_p = xpc.xp_pattern;
                set_context_for_wildcard_arg(None, pat_p, false, &mut xpc, &mut context);
                xpc.xp_pattern_len -=
                    crate::memory::ptr_offset_from(xpc.xp_pattern, pattern_start) as usize;
            }
            EXPAND_FILETYPECMD => {
                cmdexp().filetype_expand_what = FiletypeExpandWhat::All;
            }
            EXPAND_BREAKPOINT => {
                cmdexp().breakpt_expand_what = BreakptExpandWhat::Add;
            }
            _ => {}
        }
    }

    if xpc.xp_context == EXPAND_LUA {
        xpc.xp_col = crate::memory::cstr_len(xpc.xp_line) as i32;
        nlua_expand_pat(&mut xpc);
        xpc.xp_pattern_len -=
            crate::memory::ptr_offset_from(xpc.xp_pattern, pattern_start) as usize;
    }
    let pat = if cmdline_fuzzy_completion_supported(&xpc) {
        cstr_from_bytes(unsafe {
            std::slice::from_raw_parts(xpc.xp_pattern.cast::<u8>(), xpc.xp_pattern_len)
        })
    } else {
        addstar(xpc.xp_pattern, xpc.xp_pattern_len, xpc.xp_context)
    };

    expand_one(
        &mut xpc,
        pat.as_ptr().cast_mut(),
        None,
        options,
        WILD_ALL_KEEP,
    );
    tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);

    for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
        tv_list_append_string(
            rettv.vval.v_list_ref(),
            f.as_ptr(),
            f.as_bytes().len() as isize,
        );
    }
}

/// "getcompletiontype()" function.
pub fn f_getcompletiontype(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarString;
    rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

    if !tv_check_for_string_arg(argvars, 0) {
        return;
    }

    let pat = tv_get_string(&argvars[0]);
    let mut xpc: ExpandCtx = ExpandCtx::new();

    let cmdline_len = crate::memory::cstr_len(pat) as i32;
    set_cmd_context(
        &mut xpc,
        pat.cast_mut(),
        cmdline_len,
        cmdline_len,
        false as i32,
    );
    crate::eval::typval::tv_set_string(
        rettv,
        cmdcomplete_type_to_str(xpc.xp_context, xpc.xp_arg)
            .as_ref()
            .map_or(ptr::null(), |s| s.as_ptr()),
    );
}

/// "cmdcomplete_info()" function.
pub fn f_cmdcomplete_info(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    unsafe {
        let ccline = get_cmdline_info();

        tv_dict_alloc_ret(rettv);
        if ccline.xpc.is_null() || (*ccline.xpc).xp_files.capacity() == 0 {
            return;
        }
        let orig_ptr = &cmdexp().cmdline_orig;
        let orig_ref = orig_ptr.as_ref();

        let retdict = rettv.vval.v_dict();
        let mut ret = tv_dict_add_str_len(
            &mut *retdict,
            c"cmdline_orig",
            orig_ref.map_or(ptr::null(), |v| v.as_ptr().cast::<c_char>()),
            orig_ref.map_or(0, |v| v.len() as i32),
        );
        if ret {
            ret = tv_dict_add_nr(&mut *retdict, c"pum_visible", i64::from(pum_visible()));
        }
        if ret {
            ret = tv_dict_add_nr(
                &mut *retdict,
                c"selected",
                i64::from((*ccline.xpc).xp_selected),
            );
        }
        if ret {
            let li = tv_list_alloc((*ccline.xpc).xp_numfiles as isize);
            ret = tv_dict_add_list(&mut *retdict, c"matches", li);
            let files = (*ccline.xpc).xp_files.as_slice();
            let mut idx: usize = 0;
            while ret && (idx as i32) < (*ccline.xpc).xp_numfiles {
                tv_list_append_string(
                    &mut *li,
                    files[idx].as_ptr(),
                    files[idx].as_bytes().len() as isize,
                );
                idx += 1;
            }
        }
    }
}

/// Copy a substring from the current buffer (curbuf).
fn copy_substring_from_pos(
    start: &mut CursorPos,
    end: &mut CursorPos,
    match_end: &mut CursorPos,
) -> Option<CString> {
    let exacttext = (opt_vars().wop_flags & K_OPT_WOP_FLAG_EXACTTEXT) != 0;

    if start.lnum > end.lnum || (start.lnum == end.lnum && start.col >= end.col) {
        return None;
    }

    let mut ga: Vec<u8> = Vec::with_capacity(128);

    let start_line = ml_get(start.lnum);
    let start_ptr = start_line.wrapping_add(start.col as usize);
    let is_single_line = start.lnum == end.lnum;

    let mut segment_len = if is_single_line {
        end.col - start.col
    } else {
        ml_get_len(start.lnum) - start.col
    };
    ga.extend_from_slice(unsafe {
        std::slice::from_raw_parts(start_ptr.cast::<u8>(), segment_len as usize)
    });
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
            let line = ml_get(lnum);
            let linelen = ml_get_len(lnum);
            ga.extend_from_slice(unsafe {
                std::slice::from_raw_parts(line.cast::<u8>(), linelen as usize)
            });
            if exacttext {
                ga.extend_from_slice(b"\\n");
            } else {
                ga.push(b'\n');
            }
            lnum += 1;
        }
    }

    let end_line = ml_get(end.lnum);
    let word_end = find_word_end(end_line.wrapping_add(end.col as usize));
    segment_len = crate::memory::ptr_offset_from(word_end, end_line) as i32;
    let start_off = if is_single_line { end.col } else { 0 };
    ga.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            end_line.wrapping_add(start_off as usize).cast::<u8>(),
            (segment_len - start_off) as usize,
        )
    });

    match_end.lnum = end.lnum;
    match_end.col = segment_len;

    Some(cstr_from_bytes(&ga))
}

/// Returns true if the given string `str` matches the regex pattern `pat`.
fn is_regex_match(
    pat: impl crate::strings::IntoCCharPtr,
    str_arg: impl crate::strings::IntoCCharPtr,
) -> bool {
    let pat = pat.into_c_char_ptr();
    let str_arg = str_arg.into_c_char_ptr();
    {
        if crate::memory::cstr_as_bytes(pat) == crate::memory::cstr_as_bytes(str_arg) {
            return true;
        }

        let mut regmatch: Regmatch = Regmatch::new();

        g_st().emsg_off += 1;
        g_st().msg_silent += 1;
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        g_st().emsg_off -= 1;
        g_st().msg_silent -= 1;

        if regmatch.regprog.is_null() {
            return false;
        }
        regmatch.rm_ic = opt_vars().p_ic != 0;
        if opt_vars().p_ic != 0 && opt_vars().p_scs != 0 {
            regmatch.rm_ic = !pat_has_uppercase(pat.cast_mut());
        }

        g_st().emsg_off += 1;
        g_st().msg_silent += 1;
        let result = vim_regexec_nl(&mut regmatch, str_arg, 0);
        g_st().emsg_off -= 1;
        g_st().msg_silent -= 1;

        vim_regfree(regmatch.regprog);
        result
    }
}

/// Constructs a new match string by appending text from the buffer to `pat`.
fn concat_pattern_with_buffer_match(
    pat: impl crate::strings::IntoCCharPtr,
    pat_len: i32,
    end_match_pos: &mut CursorPos,
    lowercase: bool,
) -> CString {
    let pat = pat.into_c_char_ptr();
    {
        let line = ml_get(end_match_pos.lnum);
        let word_end = find_word_end(line.wrapping_add(end_match_pos.col as usize));
        let match_len =
            crate::memory::ptr_offset_from(word_end, line.wrapping_add(end_match_pos.col as usize))
                as i32;
        let mut out = Vec::with_capacity((match_len as usize) + (pat_len as usize));
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(pat.cast::<u8>(), pat_len as usize)
        });
        if match_len > 0 {
            let match_slice = unsafe {
                std::slice::from_raw_parts(
                    line.wrapping_add(end_match_pos.col as usize).cast::<u8>(),
                    match_len as usize,
                )
            };
            if lowercase {
                let mword = cstr_from_bytes(match_slice);
                let lower = strcase_save(&mword, false);
                let lower_bytes = lower.to_bytes();
                out.extend_from_slice(&lower_bytes[..lower_bytes.len().min(match_len as usize)]);
            } else {
                out.extend_from_slice(match_slice);
            }
        }
        cstr_from_bytes(&out)
    }
}

/// Search for strings matching "pat" in the specified range and return them.
fn expand_pattern_in_buf(
    pat: impl crate::strings::IntoCCharPtr,
    dir: Direction,
    matches: &mut Vec<CString>,
) -> bool {
    let pat = pat.into_c_char_ptr();
    {
        let exacttext = (opt_vars().wop_flags & K_OPT_WOP_FLAG_EXACTTEXT) != 0;
        let has_range = g_st().search_global.first_line != 0;

        matches.clear();

        if crate::memory::cstr_is_empty(pat) {
            return false;
        }

        let pat_len = crate::memory::cstr_len(pat) as i32;
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
            cur_match_pos.lnum = g_st().search_global.first_line;
        } else {
            cur_match_pos = cmdexp().pre_incsearch_pos;
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
            g_st().emsg_off += 1;
            g_st().msg_silent += 1;
            let found_new_match = searchit(
                ptr::null_mut(),
                curbuf_mut(),
                &mut cur_match_pos,
                Some(&mut end_match_pos),
                dir,
                pat,
                pat_len as usize,
                1,
                search_flags,
                RE_LAST,
                opt_vars().p_magic != 0,
                None,
            );
            g_st().msg_silent -= 1;
            g_st().emsg_off -= 1;

            if found_new_match == FAIL {
                break;
            }

            if has_range
                && (cur_match_pos.lnum < g_st().search_global.first_line
                    || cur_match_pos.lnum > g_st().search_global.last_line)
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

            if char_avail() || g_st().got_int {
                if g_st().got_int {
                    let _ = vpeekc();
                    g_st().got_int = false;
                }
                matches.clear();
                return false;
            }

            if end_match_pos.lnum > curbuf_mut().b_ml.ml_line_count {
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
                let mut m =
                    concat_pattern_with_buffer_match(pat, pat_len, &mut end_match_pos, false);

                if !is_regex_match(m.as_ptr(), full_match.as_ptr()) {
                    m = concat_pattern_with_buffer_match(pat, pat_len, &mut end_match_pos, true);
                    if !is_regex_match(m.as_ptr(), full_match.as_ptr()) {
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
