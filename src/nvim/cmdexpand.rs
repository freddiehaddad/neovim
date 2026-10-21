//! Functions for command-line completion (`src/nvim/cmdexpand.c`).

use crate::api::command::is_user_cmdidx;
use crate::api::private::helpers::{
    api_clear_error, api_free_object, cstr_as_string, cstrn_to_string,
};
use crate::api::private::validate::{
    Array, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::arglist::get_arglist_name;
use crate::autocmd::{
    HLF_T, aucmd_set_expand_context, expand_get_augroup_name, expand_get_event_name,
};
use crate::buffer::{ExpandBufnames, kUIMessages};
use crate::charset::{
    Ctrl_V, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_isspace, ascii_isupper,
    ascii_iswhite, backslash_halve_save, rem_backslash, skipdigits, skiptowhite, skipwhite,
    transchar, transchar_byte, vim_isfilec_or_wc, vim_strsize,
};
use crate::cmdhist::get_history_arg;
use crate::cursor::{ltoreq, pos_T};
use crate::drawscreen::{redraw_statuslines, update_screen, win_redraw_last_status};
use crate::errors::*;
use crate::eval::funcs::{get_expr_name, get_function_name};
use crate::eval::typval::{
    Callback, EvalFuncData, VarType, list_T, tv_check_for_string_arg, tv_clear, tv_dict_add_list,
    tv_dict_add_nr, tv_dict_add_str_len, tv_dict_alloc_ret, tv_dict_get_string, tv_get_bool_chk,
    tv_get_string, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_string,
    tv_list_unref, typval_T,
};
use crate::eval::userfunc::get_user_func_name;
use crate::eval::vars::get_user_var_name;
use crate::eval::{call_func_retlist, call_func_retstr, set_context_for_expression};
use crate::ex_cmds::skip_vimgrep_pat;
use crate::ex_docmd::{
    EX_ARGOPT, EX_CMDARG, EX_EXTRA, EX_NOSPC, EX_TRLBAR, EX_XFILE, ends_excmd, exarg_T,
    excmd_get_argt, excmd_get_cmdidx, expand_argopt, expand_findfunc, find_nextcmd,
    get_command_name, set_no_hlsearch, skip_cmd_arg, skip_range,
};
use crate::ex_getln::{
    VSE_BUFFER, VSE_NONE, cmd_screencol, cursorcmd, escape_fname, get_cmdline_info,
    get_cmdline_last_prompt_id, hlf_T, parse_pattern_and_range, put_on_cmdline, realloc_cmdbuff,
    redrawcmd, tilde_replace, vim_strsave_fnameescape,
};
use crate::fuzzy::{FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_to_strmatches};
use crate::garray::{
    ga_append, ga_append_via_ptr, ga_clear_strings, ga_concat_len, ga_grow, ga_init, garray_T,
};
use crate::globals::*;
use crate::grid::{grid_line_fill, grid_line_flush, grid_line_puts, grid_line_start};
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear, hash_hash, hash_init, hash_lookup, hashitem_empty, hashtab_T,
};
use crate::help::{cleanup_help_tags, find_help_tags};
use crate::highlight::{hl_attr, win_hl_attr};
use crate::highlight_group::{get_highlight_name, set_context_in_highlight_cmd};
use crate::input::{beep_flush, char_avail, vpeekc};
use crate::insexpand::find_word_end;
use crate::keycodes::{K_DOWN, K_KENTER, K_LEFT, K_RIGHT, K_UP};
use crate::kvec::KVec;
use crate::lua::executor::{
    kRetObject, nlua_call_user_expand_func, nlua_exec, nlua_expand_get_matches, nlua_expand_pat,
};
use crate::mapping::{ExpandMappings, set_context_in_map_cmd};
use crate::math::{FAIL, OK};
use crate::mbyte::{mb_tolower, utf_head_off, utf_ptr2char, utfc_ptr2len};
use crate::memline::{ml_get, ml_get_len};
use crate::memory::{xfree, xmalloc, xmemcpyz, xmemdupz, xstpcpy, xstrdup};
use crate::menu::{get_menu_name, get_menu_names, menu_is_separator, set_context_in_menu_cmd};
use crate::message::{
    emsg, msg_advance, msg_clr_eos, msg_ext_set_kind, msg_outtrans, msg_outtrans_long, msg_putchar,
    msg_puts, msg_puts_hl, msg_scroll_up, msg_start, semsg,
};
#[cfg(unix)]
use crate::option::csh_like_shell;
use crate::option::{
    ExpandOldSetting, ExpandSettingSubtract, ExpandSettings, ExpandStringSetting, OptInt,
    copy_option_part, get_findfunc, set_context_in_set_cmd,
};
use crate::option_vars::*;
use crate::os::env::{
    env_name_len, expand_T, expand_env_save_opt, get_env_name, home_replace, vim_getenv,
};
#[cfg(not(windows))]
use crate::os::fs::PATHSEP;
#[cfg(windows)]
use crate::os::fs::PATHSEP;
use crate::os::fs::os_isdir;
use crate::os::lang::{get_lang_arg, get_locales};
use crate::os::users::{get_users, match_user};
use crate::path::{
    EW_ADDSLASH, EW_ALLLINKS, EW_CDPATH, EW_DIR, EW_EXEC, EW_FILE, EW_ICASE, EW_KEEPALL,
    EW_NOERROR, EW_NOTFOUND, EW_PATH, EW_SHELLCMD, EW_SILENT, FreeWild, after_pathsep,
    expand_wildcards, expand_wildcards_eval, match_suffix, path_is_absolute, path_tail,
};
use crate::popupmenu::{
    pum_clear, pum_display, pum_get_height, pum_undisplay, pum_visible, pumitem_T,
};
use crate::profile::{get_profile_name, set_context_in_profile_cmd};
use crate::regexp::{
    RE_MAGIC, RE_STRING, regmatch_T, skip_regexp, vim_regcomp, vim_regexec, vim_regexec_nl,
    vim_regfree,
};
use crate::runtime::{
    DIP_OPT, DIP_START, ExpandPackAddDir, ExpandRTDir, expand_runtime_cmd, script_id_valid,
    script_item, set_context_in_runtime_cmd,
};
use crate::search::{Direction, ignorecase, pat_has_uppercase, searchit};
use crate::sign::{get_sign_name, set_context_in_sign_cmd};
use crate::statusline::fillchar_status;
use crate::strings::{
    c_strcpy, gettext, qsort, sort_strings, strcase_save, strcmp, strlen, strncmp, vim_snprintf,
    vim_strchr, vim_strsave_escaped, xstrnsave,
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
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::win_config::UIExtension;
pub type xp_prefix_T = c_int;
pub type CompleteListItemGetter = Option<unsafe fn(*mut expand_T, c_int) -> *mut c_char>;
type user_expand_func_T = unsafe fn(*const c_char, c_int, *mut typval_T) -> *mut c_void;
pub type LineGetter = Option<unsafe fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>;
pub type cmd_addr_T = c_int;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum cmdidx_T {
    CMD_append = 0,
    CMD_abbreviate,
    CMD_abclear,
    CMD_aboveleft,
    CMD_all,
    CMD_amenu,
    CMD_anoremenu,
    CMD_args,
    CMD_argadd,
    CMD_argdelete,
    CMD_argdo,
    CMD_argdedupe,
    CMD_argedit,
    CMD_argglobal,
    CMD_arglocal,
    CMD_argument,
    CMD_ascii,
    CMD_autocmd,
    CMD_augroup,
    CMD_aunmenu,
    CMD_buffer,
    CMD_bNext,
    CMD_ball,
    CMD_badd,
    CMD_balt,
    CMD_bcd,
    CMD_bchdir,
    CMD_bdelete,
    CMD_belowright,
    CMD_bfirst,
    CMD_blast,
    CMD_bmodified,
    CMD_bnext,
    CMD_botright,
    CMD_bprevious,
    CMD_brewind,
    CMD_break,
    CMD_breakadd,
    CMD_breakdel,
    CMD_breaklist,
    CMD_browse,
    CMD_buffers,
    CMD_bufdo,
    CMD_bunload,
    CMD_bwipeout,
    CMD_change,
    CMD_cNext,
    CMD_cNfile,
    CMD_cabbrev,
    CMD_cabclear,
    CMD_cabove,
    CMD_caddbuffer,
    CMD_caddexpr,
    CMD_caddfile,
    CMD_cafter,
    CMD_call,
    CMD_catch,
    CMD_cbuffer,
    CMD_cbefore,
    CMD_cbelow,
    CMD_cbottom,
    CMD_cc,
    CMD_cclose,
    CMD_cd,
    CMD_cdo,
    CMD_center,
    CMD_cexpr,
    CMD_cfile,
    CMD_cfdo,
    CMD_cfirst,
    CMD_cgetfile,
    CMD_cgetbuffer,
    CMD_cgetexpr,
    CMD_chdir,
    CMD_changes,
    CMD_checkhealth,
    CMD_checkpath,
    CMD_checktime,
    CMD_chistory,
    CMD_clist,
    CMD_clast,
    CMD_close,
    CMD_clearjumps,
    CMD_cmap,
    CMD_cmapclear,
    CMD_cmenu,
    CMD_cnext,
    CMD_cnewer,
    CMD_cnfile,
    CMD_cnoremap,
    CMD_cnoreabbrev,
    CMD_cnoremenu,
    CMD_copy,
    CMD_colder,
    CMD_colorscheme,
    CMD_command,
    CMD_comclear,
    CMD_compiler,
    CMD_continue,
    CMD_confirm,
    CMD_connect,
    CMD_const,
    CMD_copen,
    CMD_cprevious,
    CMD_cpfile,
    CMD_cquit,
    CMD_crewind,
    CMD_cunmap,
    CMD_cunabbrev,
    CMD_cunmenu,
    CMD_cwindow,
    CMD_delete,
    CMD_delmarks,
    CMD_debug,
    CMD_debuggreedy,
    CMD_defer,
    CMD_delcommand,
    CMD_delfunction,
    CMD_detach,
    CMD_display,
    CMD_diffupdate,
    CMD_diffget,
    CMD_diffoff,
    CMD_diffpatch,
    CMD_diffput,
    CMD_diffsplit,
    CMD_diffthis,
    CMD_digraphs,
    CMD_djump,
    CMD_dlist,
    CMD_doautocmd,
    CMD_doautoall,
    CMD_drop,
    CMD_dsearch,
    CMD_dsplit,
    CMD_edit,
    CMD_earlier,
    CMD_echo,
    CMD_echoerr,
    CMD_echohl,
    CMD_echomsg,
    CMD_echon,
    CMD_else,
    CMD_elseif,
    CMD_emenu,
    CMD_endif,
    CMD_endfunction,
    CMD_endfor,
    CMD_endtry,
    CMD_endwhile,
    CMD_enew,
    CMD_eval,
    CMD_ex,
    CMD_execute,
    CMD_exit,
    CMD_exmode,
    CMD_exusage,
    CMD_file,
    CMD_files,
    CMD_filetype,
    CMD_filter,
    CMD_find,
    CMD_finally,
    CMD_finish,
    CMD_first,
    CMD_fold,
    CMD_foldclose,
    CMD_folddoopen,
    CMD_folddoclosed,
    CMD_foldopen,
    CMD_for,
    CMD_function,
    CMD_fclose,
    CMD_global,
    CMD_goto,
    CMD_grep,
    CMD_grepadd,
    CMD_gui,
    CMD_gvim,
    CMD_help,
    CMD_helpclose,
    CMD_helpgrep,
    CMD_helptags,
    CMD_highlight,
    CMD_hide,
    CMD_history,
    CMD_horizontal,
    CMD_insert,
    CMD_iabbrev,
    CMD_iabclear,
    CMD_if,
    CMD_ijump,
    CMD_ilist,
    CMD_imap,
    CMD_imapclear,
    CMD_imenu,
    CMD_inoremap,
    CMD_inoreabbrev,
    CMD_inoremenu,
    CMD_intro,
    CMD_iput,
    CMD_isearch,
    CMD_isplit,
    CMD_iunmap,
    CMD_iunabbrev,
    CMD_iunmenu,
    CMD_join,
    CMD_jumps,
    CMD_k,
    CMD_keepmarks,
    CMD_keepjumps,
    CMD_keeppatterns,
    CMD_keepalt,
    CMD_list,
    CMD_lNext,
    CMD_lNfile,
    CMD_last,
    CMD_labove,
    CMD_language,
    CMD_laddexpr,
    CMD_laddbuffer,
    CMD_laddfile,
    CMD_lafter,
    CMD_later,
    CMD_lbuffer,
    CMD_lbefore,
    CMD_lbelow,
    CMD_lbottom,
    CMD_lcd,
    CMD_lchdir,
    CMD_lclose,
    CMD_ldo,
    CMD_left,
    CMD_leftabove,
    CMD_let,
    CMD_lexpr,
    CMD_lfile,
    CMD_lfdo,
    CMD_lfirst,
    CMD_lgetfile,
    CMD_lgetbuffer,
    CMD_lgetexpr,
    CMD_lgrep,
    CMD_lgrepadd,
    CMD_lhelpgrep,
    CMD_lhistory,
    CMD_ll,
    CMD_llast,
    CMD_llist,
    CMD_lmap,
    CMD_lmapclear,
    CMD_lmake,
    CMD_lnoremap,
    CMD_lnext,
    CMD_lnewer,
    CMD_lnfile,
    CMD_loadview,
    CMD_loadkeymap,
    CMD_lockmarks,
    CMD_lockvar,
    CMD_log,
    CMD_lolder,
    CMD_lopen,
    CMD_lprevious,
    CMD_lpfile,
    CMD_lrewind,
    CMD_ltag,
    CMD_lunmap,
    CMD_lua,
    CMD_luado,
    CMD_luafile,
    CMD_lvimgrep,
    CMD_lvimgrepadd,
    CMD_lwindow,
    CMD_ls,
    CMD_lsp,
    CMD_move,
    CMD_mark,
    CMD_make,
    CMD_map,
    CMD_mapclear,
    CMD_marks,
    CMD_match,
    CMD_menu,
    CMD_menutranslate,
    CMD_messages,
    CMD_mkexrc,
    CMD_mksession,
    CMD_mkspell,
    CMD_mkvimrc,
    CMD_mkview,
    CMD_mode,
    CMD_mzscheme,
    CMD_mzfile,
    CMD_next,
    CMD_new,
    CMD_nmap,
    CMD_nmapclear,
    CMD_nmenu,
    CMD_nnoremap,
    CMD_nnoremenu,
    CMD_noremap,
    CMD_noautocmd,
    CMD_nohlsearch,
    CMD_noreabbrev,
    CMD_noremenu,
    CMD_noswapfile,
    CMD_normal,
    CMD_number,
    CMD_nunmap,
    CMD_nunmenu,
    CMD_oldfiles,
    CMD_omap,
    CMD_omapclear,
    CMD_omenu,
    CMD_only,
    CMD_onoremap,
    CMD_onoremenu,
    CMD_options,
    CMD_ounmap,
    CMD_ounmenu,
    CMD_ownsyntax,
    CMD_print,
    CMD_packadd,
    CMD_packdel,
    CMD_packloadall,
    CMD_packupdate,
    CMD_pbuffer,
    CMD_pclose,
    CMD_perl,
    CMD_perldo,
    CMD_perlfile,
    CMD_pedit,
    CMD_pop,
    CMD_popup,
    CMD_ppop,
    CMD_preserve,
    CMD_previous,
    CMD_profile,
    CMD_profdel,
    CMD_psearch,
    CMD_ptag,
    CMD_ptNext,
    CMD_ptfirst,
    CMD_ptjump,
    CMD_ptlast,
    CMD_ptnext,
    CMD_ptprevious,
    CMD_ptrewind,
    CMD_ptselect,
    CMD_put,
    CMD_pwd,
    CMD_python,
    CMD_pydo,
    CMD_pyfile,
    CMD_py3,
    CMD_py3do,
    CMD_python3,
    CMD_py3file,
    CMD_pyx,
    CMD_pyxdo,
    CMD_pythonx,
    CMD_pyxfile,
    CMD_quit,
    CMD_quitall,
    CMD_qall,
    CMD_read,
    CMD_recover,
    CMD_redo,
    CMD_redir,
    CMD_redraw,
    CMD_redrawstatus,
    CMD_redrawtabline,
    CMD_registers,
    CMD_resize,
    CMD_restart,
    CMD_retab,
    CMD_return,
    CMD_rewind,
    CMD_right,
    CMD_rightbelow,
    CMD_rshada,
    CMD_runtime,
    CMD_rundo,
    CMD_ruby,
    CMD_rubydo,
    CMD_rubyfile,
    CMD_rviminfo,
    CMD_substitute,
    CMD_sNext,
    CMD_sargument,
    CMD_sall,
    CMD_sandbox,
    CMD_saveas,
    CMD_sbuffer,
    CMD_sbNext,
    CMD_sball,
    CMD_sbfirst,
    CMD_sblast,
    CMD_sbmodified,
    CMD_sbnext,
    CMD_sbprevious,
    CMD_sbrewind,
    CMD_scriptnames,
    CMD_scriptencoding,
    CMD_set,
    CMD_setfiletype,
    CMD_setglobal,
    CMD_setlocal,
    CMD_sfind,
    CMD_sfirst,
    CMD_simalt,
    CMD_sign,
    CMD_silent,
    CMD_sleep,
    CMD_slast,
    CMD_smagic,
    CMD_smap,
    CMD_smapclear,
    CMD_smenu,
    CMD_snext,
    CMD_snomagic,
    CMD_snoremap,
    CMD_snoremenu,
    CMD_source,
    CMD_sort,
    CMD_split,
    CMD_spellgood,
    CMD_spelldump,
    CMD_spellinfo,
    CMD_spellrepall,
    CMD_spellrare,
    CMD_spellundo,
    CMD_spellwrong,
    CMD_sprevious,
    CMD_srewind,
    CMD_stop,
    CMD_stag,
    CMD_startinsert,
    CMD_startgreplace,
    CMD_startreplace,
    CMD_stopinsert,
    CMD_stjump,
    CMD_stselect,
    CMD_sunhide,
    CMD_sunmap,
    CMD_sunmenu,
    CMD_suspend,
    CMD_sview,
    CMD_swapname,
    CMD_syntax,
    CMD_syntime,
    CMD_syncbind,
    CMD_t,
    CMD_tcd,
    CMD_tchdir,
    CMD_tNext,
    CMD_tag,
    CMD_tags,
    CMD_tab,
    CMD_tabclose,
    CMD_tabdo,
    CMD_tabedit,
    CMD_tabfind,
    CMD_tabfirst,
    CMD_tabmove,
    CMD_tablast,
    CMD_tabnext,
    CMD_tabnew,
    CMD_tabonly,
    CMD_tabprevious,
    CMD_tabNext,
    CMD_tabrewind,
    CMD_tabs,
    CMD_tcl,
    CMD_tcldo,
    CMD_tclfile,
    CMD_terminal,
    CMD_tfirst,
    CMD_throw,
    CMD_tjump,
    CMD_tlast,
    CMD_tlmenu,
    CMD_tlnoremenu,
    CMD_tlunmenu,
    CMD_tmenu,
    CMD_tmap,
    CMD_tmapclear,
    CMD_tnext,
    CMD_tnoremap,
    CMD_topleft,
    CMD_tprevious,
    CMD_trewind,
    CMD_trust,
    CMD_try,
    CMD_tselect,
    CMD_tunmenu,
    CMD_tunmap,
    CMD_undo,
    CMD_undojoin,
    CMD_undolist,
    CMD_unabbreviate,
    CMD_unhide,
    CMD_uniq,
    CMD_unlet,
    CMD_unlockvar,
    CMD_unmap,
    CMD_unmenu,
    CMD_unsilent,
    CMD_update,
    CMD_uptime,
    CMD_vglobal,
    CMD_version,
    CMD_verbose,
    CMD_vertical,
    CMD_visual,
    CMD_view,
    CMD_vimgrep,
    CMD_vimgrepadd,
    CMD_viusage,
    CMD_vmap,
    CMD_vmapclear,
    CMD_vmenu,
    CMD_vnoremap,
    CMD_vnew,
    CMD_vnoremenu,
    CMD_vsplit,
    CMD_vunmap,
    CMD_vunmenu,
    CMD_write,
    CMD_wNext,
    CMD_wall,
    CMD_while,
    CMD_winsize,
    CMD_wincmd,
    CMD_windo,
    CMD_winpos,
    CMD_wnext,
    CMD_wprevious,
    CMD_wq,
    CMD_wqall,
    CMD_wshada,
    CMD_wundo,
    CMD_wviminfo,
    CMD_xit,
    CMD_xall,
    CMD_xmap,
    CMD_xmapclear,
    CMD_xmenu,
    CMD_xnoremap,
    CMD_xnoremenu,
    CMD_xunmap,
    CMD_xunmenu,
    CMD_yank,
    CMD_z,
    CMD_bang,
    CMD_pound,
    CMD_and,
    CMD_lshift,
    CMD_equal,
    CMD_rshift,
    CMD_at,
    CMD_tilde,
    CMD_Next,
    CMD_SIZE,
    CMD_USER = -1,
    CMD_USER_BUF = -2,
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
    pub start: c_int,
    pub end: c_int,
    pub hl_id: c_int,
}

pub type CmdlineColors = KVec<CmdlineColorChunk>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColoredCmdline {
    pub prompt_id: c_uint,
    pub cmdbuff: *mut c_char,
    pub colors: CmdlineColors,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CmdlineInfo {
    pub cmdbuff: *mut c_char,
    pub cmdbufflen: c_int,
    pub cmdlen: c_int,
    pub cmdpos: c_int,
    pub cmdspos: c_int,
    pub cmdfirstc: c_int,
    pub cmdindent: c_int,
    pub cmdprompt: *mut c_char,
    pub hl_id: c_int,
    pub overstrike: c_int,
    pub xpc: *mut expand_T,
    pub xp_context: c_int,
    pub xp_arg: *mut c_char,
    pub input_fn: c_int,
    pub cmdbuff_replaced: bool,
    pub prompt_id: c_uint,
    pub highlight_callback: Callback,
    pub last_colors: ColoredCmdline,
    pub level: c_int,
    pub prev_ccline: *mut CmdlineInfo,
    pub special_char: c_char,
    pub special_shift: bool,
    pub redraw_state: c_int,
    pub one_key: bool,
    pub mouse_used: *mut bool,
}

pub(crate) const Ctrl_N: c_int = 14;
pub(crate) const Ctrl_P: c_int = 16;

pub(crate) const MB_MAXBYTES: usize = 30;

#[cfg(windows)]
const ENV_SEPCHAR: c_int = b';' as c_int;
#[cfg(not(windows))]
const ENV_SEPCHAR: c_int = b':' as c_int;

const PATHSEPSTR: &[u8] = if cfg!(windows) { b"\\" } else { b"/" };

pub const WILD_FREE: c_int = 1;
pub const WILD_EXPAND_FREE: c_int = 2;
pub const WILD_EXPAND_KEEP: c_int = 3;
pub const WILD_NEXT: c_int = 4;
pub const WILD_PREV: c_int = 5;
pub const WILD_ALL: c_int = 6;
pub const WILD_LONGEST: c_int = 7;
pub const WILD_ALL_KEEP: c_int = 8;
pub const WILD_CANCEL: c_int = 9;
pub const WILD_APPLY: c_int = 10;
pub const WILD_PAGEUP: c_int = 11;
pub const WILD_PAGEDOWN: c_int = 12;
pub const WILD_PUM_WANT: c_int = 13;

pub const WILD_LIST_NOTFOUND: c_int = 0x01;
pub const WILD_HOME_REPLACE: c_int = 0x02;
pub const WILD_USE_NL: c_int = 0x04;
pub const WILD_NO_BEEP: c_int = 0x08;
pub const WILD_ADD_SLASH: c_int = 0x10;
pub const WILD_KEEP_ALL: c_int = 0x20;
pub const WILD_SILENT: c_int = 0x40;
pub const WILD_ESCAPE: c_int = 0x80;
pub const WILD_ICASE: c_int = 0x100;
pub const WILD_ALLLINKS: c_int = 0x200;
pub const WILD_USE_COMPLETESLASH: c_int = 0x400;
pub const WILD_NOERROR: c_int = 0x800;
pub const WILD_BUFLASTUSED: c_int = 0x1000;
pub const BUF_DIFF_FILTER: c_int = 0x2000;
pub const WILD_NOSELECT: c_int = 0x4000;
pub const WILD_MAY_EXPAND_PATTERN: c_int = 0x8000;
pub const WILD_FUNC_TRIGGER: c_int = 0x10000;
pub const WILD_NOINSERT: c_int = 0x20000;
pub const WILD_USE_SHELLSLASH: c_int = 0x40000;

pub const XP_PREFIX_NONE: xp_prefix_T = 0;
pub const XP_PREFIX_NO: xp_prefix_T = 1;
pub const XP_PREFIX_INV: xp_prefix_T = 2;

pub const XP_BS_NONE: c_int = 0;
pub const XP_BS_ONE: c_int = 0x1;
pub const XP_BS_THREE: c_int = 0x2;
pub const XP_BS_COMMA: c_int = 0x4;

pub const EXPAND_UNSUCCESSFUL: c_int = -2;
pub const EXPAND_OK: c_int = -1;
pub const EXPAND_NOTHING: c_int = 0;
pub const EXPAND_COMMANDS: c_int = 1;
pub const EXPAND_FILES: c_int = 2;
pub const EXPAND_DIRECTORIES: c_int = 3;
pub const EXPAND_SETTINGS: c_int = 4;
pub const EXPAND_BOOL_SETTINGS: c_int = 5;
pub const EXPAND_TAGS: c_int = 6;
pub const EXPAND_OLD_SETTING: c_int = 7;
pub const EXPAND_HELP: c_int = 8;
pub const EXPAND_BUFFERS: c_int = 9;
pub const EXPAND_EVENTS: c_int = 10;
pub const EXPAND_MENUS: c_int = 11;
pub const EXPAND_SYNTAX: c_int = 12;
pub const EXPAND_HIGHLIGHT: c_int = 13;
pub const EXPAND_AUGROUP: c_int = 14;
pub const EXPAND_USER_VARS: c_int = 15;
pub const EXPAND_MAPPINGS: c_int = 16;
pub const EXPAND_TAGS_LISTFILES: c_int = 17;
pub const EXPAND_FUNCTIONS: c_int = 18;
pub const EXPAND_USER_FUNC: c_int = 19;
pub const EXPAND_EXPRESSION: c_int = 20;
pub const EXPAND_MENUNAMES: c_int = 21;
pub const EXPAND_USER_COMMANDS: c_int = 22;
pub const EXPAND_USER_CMD_FLAGS: c_int = 23;
pub const EXPAND_USER_NARGS: c_int = 24;
pub const EXPAND_USER_COMPLETE: c_int = 25;
pub const EXPAND_ENV_VARS: c_int = 26;
pub const EXPAND_LANGUAGE: c_int = 27;
pub const EXPAND_COLORS: c_int = 28;
pub const EXPAND_COMPILER: c_int = 29;
pub const EXPAND_USER_DEFINED: c_int = 30;
pub const EXPAND_USER_LIST: c_int = 31;
pub const EXPAND_USER_LUA: c_int = 32;
pub const EXPAND_SHELLCMD: c_int = 33;
pub const EXPAND_SIGN: c_int = 34;
pub const EXPAND_PROFILE: c_int = 35;
pub const EXPAND_FILETYPE: c_int = 36;
pub const EXPAND_FILES_IN_PATH: c_int = 37;
pub const EXPAND_OWNSYNTAX: c_int = 38;
pub const EXPAND_LOCALES: c_int = 39;
pub const EXPAND_HISTORY: c_int = 40;
pub const EXPAND_USER: c_int = 41;
pub const EXPAND_SYNTIME: c_int = 42;
pub const EXPAND_USER_ADDR_TYPE: c_int = 43;
pub const EXPAND_PACKADD: c_int = 44;
pub const EXPAND_MESSAGES: c_int = 45;
pub const EXPAND_MAPCLEAR: c_int = 46;
pub const EXPAND_ARGLIST: c_int = 47;
pub const EXPAND_DIFF_BUFFERS: c_int = 48;
pub const EXPAND_BREAKPOINT: c_int = 49;
pub const EXPAND_SCRIPTNAMES: c_int = 50;
pub const EXPAND_RUNTIME: c_int = 51;
pub const EXPAND_STRING_SETTING: c_int = 52;
pub const EXPAND_SETTING_SUBTRACT: c_int = 53;
pub const EXPAND_ARGOPT: c_int = 54;
pub const EXPAND_KEYMAP: c_int = 55;
pub const EXPAND_DIRS_IN_CDPATH: c_int = 56;
pub const EXPAND_SHELLCMDLINE: c_int = 57;
pub const EXPAND_FINDFUNC: c_int = 58;
pub const EXPAND_FILETYPECMD: c_int = 59;
pub const EXPAND_PATTERN_IN_BUF: c_int = 60;
pub const EXPAND_RETAB: c_int = 61;
pub const EXPAND_CHECKHEALTH: c_int = 62;
pub const EXPAND_LUA: c_int = 63;
pub const EXPAND_LSP: c_int = 64;
pub const EXPAND_LOG: c_int = 65;
pub const EXPAND_PACKDEL: c_int = 66;
pub const EXPAND_PACKUPDATE: c_int = 67;
pub const EXPAND_MARKS: c_int = 68;

pub(crate) const EX_NOTRLCOM: u32 = 0x800;

const kOptWopFlagFuzzy: c_uint = 0x01;
const kOptWopFlagTagfile: c_uint = 0x02;
const kOptWopFlagPum: c_uint = 0x04;
const kOptWopFlagExacttext: c_uint = 0x08;

use crate::ex_getln::{kOptWimFlagNoinsert, kOptWimFlagNoselect};

pub(crate) const kOptBoFlagWildmode: c_uint = 0x80000;

pub(crate) const kUICmdline: UIExtension = 0;
pub(crate) const kUIPopupmenu: UIExtension = 1;
pub(crate) const kUIWildmenu: UIExtension = 3;

const WM_SHOWN: c_int = 1;
const WM_SCROLLED: c_int = 2;

pub(crate) const HLF_D: hlf_T = 5;
const HLF_WM: hlf_T = 27;

pub(crate) const RE_LAST: c_int = 2;

pub(crate) const SEARCH_NFMSG: c_int = 0x08;
pub(crate) const SEARCH_OPT: c_int = 0x10;
pub(crate) const SEARCH_NOOF: c_int = 0x80;
pub(crate) const SEARCH_START: c_int = 0x100;
pub(crate) const SEARCH_PEEK: c_int = 0x800;

pub(crate) const TAG_MANY: c_int = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FiletypeExpandWhat {
    All,
    Plugin,
    Indent,
    OnOff,
}

const EXPAND_FILETYPECMD_PLUGIN: c_int = 0x01;
const EXPAND_FILETYPECMD_INDENT: c_int = 0x02;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BreakptExpandWhat {
    Add,
    Del,
    Profdel,
}

static mut cmd_showtail: bool = false;
static mut may_expand_pattern: bool = false;
static mut pre_incsearch_pos: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

static mut compl_match_array: *mut pumitem_T = ptr::null_mut();
static mut compl_match_arraysize: c_int = 0;
static mut compl_startcol: c_int = 0;
static mut compl_selected: c_int = 0;
static mut cmdline_orig: NvimString = NvimString {
    data: ptr::null_mut(),
    size: 0,
};

static mut filetype_expand_what: FiletypeExpandWhat = FiletypeExpandWhat::All;
static mut breakpt_expand_what: BreakptExpandWhat = BreakptExpandWhat::Add;

static mut HEALTHCHECK_NAMES: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};
static mut HEALTHCHECK_LAST_GEN: c_uint = 0;

static mut ARG1_LUA_NAMES: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};
static mut ARG1_LUA_LAST_XP_LINE: *mut c_char = ptr::null_mut();
static mut ARG1_LUA_LAST_GEN: c_uint = 0;

#[inline]
unsafe fn c_strchr(mut s: *const c_char, c: c_char) -> *mut c_char {
    unsafe {
        loop {
            if *s == c {
                return s.cast_mut();
            }
            if *s == NUL {
                return ptr::null_mut();
            }
            s = s.add(1);
        }
    }
}

use crate::mbyte::ptr2cells;
use crate::path::vim_ispathsep;

#[inline]
unsafe fn ga_push<T>(gap: *mut garray_T, item: T) {
    unsafe {
        let slot = ga_append_via_ptr(gap, std::mem::size_of::<T>()).cast::<T>();
        ptr::write(slot, item);
    }
}

#[inline]
unsafe fn show_match(matches: *mut *mut c_char, m: usize, showtail: bool) -> *mut c_char {
    unsafe {
        let item = *matches.add(m);
        if showtail {
            showmatches_gettail(item, false)
        } else {
            item
        }
    }
}

/// Returns true if fuzzy completion is supported for a given cmdline completion context.
unsafe fn cmdline_fuzzy_completion_supported(xp: *const expand_T) -> bool {
    unsafe {
        match (*xp).xp_context {
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
            _ => (wop_flags & kOptWopFlagFuzzy) != 0,
        }
    }
}

/// Returns true if fuzzy completion for cmdline completion is enabled and
/// "fuzzystr" is not empty.
pub unsafe fn cmdline_fuzzy_complete(fuzzystr: *const c_char) -> bool {
    unsafe { (wop_flags & kOptWopFlagFuzzy) != 0 && *fuzzystr != NUL }
}

/// Sort function for the completion matches.
/// <SNR> functions should be sorted to the end.
unsafe fn sort_func_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let p1 = *s1.cast::<*const c_char>();
        let p2 = *s2.cast::<*const c_char>();

        if *p1 != b'<' as c_char && *p2 == b'<' as c_char {
            return -1;
        }
        if *p1 == b'<' as c_char && *p2 != b'<' as c_char {
            return 1;
        }
        strcmp(p1, p2)
    }
}

/// Escape special characters in the cmdline completion matches.
unsafe fn wildescape(
    xp: *mut expand_T,
    str_arg: *const c_char,
    numfiles: c_int,
    files: *mut *mut c_char,
) {
    unsafe {
        let vse_what = if (*xp).xp_context == EXPAND_BUFFERS {
            VSE_BUFFER
        } else {
            VSE_NONE
        };

        if (*xp).xp_context == EXPAND_FILES
            || (*xp).xp_context == EXPAND_FILES_IN_PATH
            || (*xp).xp_context == EXPAND_SHELLCMD
            || (*xp).xp_context == EXPAND_BUFFERS
            || (*xp).xp_context == EXPAND_DIRECTORIES
            || (*xp).xp_context == EXPAND_DIRS_IN_CDPATH
        {
            for i in 0..(numfiles as usize) {
                let file_slot = files.add(i);
                if ((*xp).xp_backslash & XP_BS_THREE) != 0 {
                    let pat = if ((*xp).xp_backslash & XP_BS_COMMA) != 0 {
                        c" ,".as_ptr()
                    } else {
                        c" ".as_ptr()
                    };
                    let p = vim_strsave_escaped(*file_slot, pat);
                    xfree((*file_slot).cast());
                    *file_slot = p;
                    #[cfg(windows)]
                    {
                        let p2 = vim_strsave_escaped(*file_slot, c" ".as_ptr());
                        xfree((*file_slot).cast());
                        *file_slot = p2;
                    }
                } else if ((*xp).xp_backslash & XP_BS_COMMA) != 0
                    && !vim_strchr(*file_slot, b',' as c_int).is_null()
                {
                    let p = vim_strsave_escaped(*file_slot, c",".as_ptr());
                    xfree((*file_slot).cast());
                    *file_slot = p;
                }

                #[cfg(windows)]
                let p = vim_strsave_fnameescape(*file_slot, vse_what);
                #[cfg(not(windows))]
                let p = vim_strsave_fnameescape(
                    *file_slot,
                    if (*xp).xp_shell { VSE_SHELL } else { vse_what },
                );
                xfree((*file_slot).cast());
                *file_slot = p;

                if *str_arg == b'\\' as c_char
                    && *str_arg.add(1) == b'~' as c_char
                    && *(*file_slot) == b'~' as c_char
                {
                    escape_fname(file_slot);
                }
            }
            (*xp).xp_backslash = XP_BS_NONE;

            if *(*files) == b'+' as c_char {
                escape_fname(files);
            }
        } else if (*xp).xp_context == EXPAND_TAGS {
            for i in 0..(numfiles as usize) {
                let file_slot = files.add(i);
                let p = vim_strsave_escaped(*file_slot, c"\\|\"".as_ptr());
                xfree((*file_slot).cast());
                *file_slot = p;
            }
        }
    }
}

/// Escape special characters in the cmdline completion matches.
unsafe fn ExpandEscape(
    xp: *mut expand_T,
    str_arg: *mut c_char,
    numfiles: c_int,
    files: *mut *mut c_char,
    options: c_int,
) {
    unsafe {
        if (options & WILD_HOME_REPLACE) != 0 {
            tilde_replace(str_arg, numfiles, files);
        }

        if (options & WILD_ESCAPE) != 0 {
            wildescape(xp, str_arg, numfiles, files);
        }
    }
}

/// Return FAIL if this is not an appropriate context in which to do
/// completion of anything, return OK if it is (even if there are no matches).
pub unsafe fn nextwild(xp: *mut expand_T, type_arg: c_int, options: c_int, escape: bool) -> c_int {
    unsafe {
        let ccline = get_cmdline_info();
        let mut p: *mut c_char;
        let from_wildtrigger_func = (options & WILD_FUNC_TRIGGER) != 0;
        let wild_navigate = type_arg == WILD_NEXT
            || type_arg == WILD_PREV
            || type_arg == WILD_PAGEUP
            || type_arg == WILD_PAGEDOWN
            || type_arg == WILD_PUM_WANT;

        if (*xp).xp_numfiles == -1 {
            pre_incsearch_pos = (*xp).xp_pre_incsearch_pos;
            if (*ccline).input_fn != 0 && (*ccline).xp_context == EXPAND_COMMANDS {
                set_cmd_context(
                    xp,
                    (*ccline).cmdbuff,
                    (*ccline).cmdlen,
                    (*ccline).cmdpos,
                    false as c_int,
                );
            } else {
                may_expand_pattern = (options & WILD_MAY_EXPAND_PATTERN) != 0;
                set_expand_context(xp);
                may_expand_pattern = false;
            }
            if (*xp).xp_context == EXPAND_LUA {
                nlua_expand_pat(xp);
            }
            cmd_showtail = expand_showtail(xp);
        }

        if (*xp).xp_context == EXPAND_UNSUCCESSFUL {
            beep_flush();
            return OK;
        }
        if (*xp).xp_context == EXPAND_NOTHING {
            return FAIL;
        }

        let i = (*xp).xp_pattern.offset_from((*ccline).cmdbuff) as c_int;
        debug_assert!((*ccline).cmdpos >= i);
        (*xp).xp_pattern_len = ((*ccline).cmdpos as usize) - (i as usize);

        if from_wildtrigger_func && (*xp).xp_context == EXPAND_COMMANDS && (*xp).xp_pattern_len == 0
        {
            return FAIL;
        }

        if !cmd_silent
            && !from_wildtrigger_func
            && !wild_navigate
            && !(ui_has(kUICmdline) || ui_has(kUIWildmenu))
        {
            msg_puts(c"...".as_ptr());
            ui_flush();
        }

        if wild_navigate {
            p = ExpandOne(xp, ptr::null_mut(), ptr::null_mut(), 0, type_arg);
        } else {
            let tmp = if cmdline_fuzzy_completion_supported(xp)
                || (*xp).xp_context == EXPAND_PATTERN_IN_BUF
            {
                xstrnsave((*xp).xp_pattern, (*xp).xp_pattern_len)
            } else {
                addstar((*xp).xp_pattern, (*xp).xp_pattern_len, (*xp).xp_context)
            };
            let use_options = options
                | WILD_USE_COMPLETESLASH
                | WILD_HOME_REPLACE
                | WILD_ADD_SLASH
                | WILD_SILENT
                | if escape { WILD_ESCAPE } else { 0 }
                | if p_wic != 0 { WILD_ICASE } else { 0 };
            p = ExpandOne(
                xp,
                tmp,
                xstrnsave((*ccline).cmdbuff.add(i as usize), (*xp).xp_pattern_len),
                use_options,
                type_arg,
            );
            xfree(tmp.cast());
            if !p.is_null() && type_arg == WILD_LONGEST {
                let mut j: usize = 0;
                while j < (*xp).xp_pattern_len {
                    let c = *(*ccline).cmdbuff.add((i as usize) + j);
                    if c == b'*' as c_char || c == b'?' as c_char {
                        break;
                    }
                    j += 1;
                }
                if strlen(p) < j {
                    xfree(p.cast());
                    p = ptr::null_mut();
                }
            }
        }

        if !wild_navigate && !(*ccline).cmdbuff.is_null() {
            xfree(cmdline_orig.data.cast());
            cmdline_orig = cstrn_to_string((*ccline).cmdbuff, (*ccline).cmdlen as usize);
        }

        if !p.is_null() && !got_int && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0 {
            let plen = strlen(p);
            let difflen = (plen as c_int) - ((*xp).xp_pattern_len as c_int);
            if (*ccline).cmdlen + difflen + 4 > (*ccline).cmdbufflen {
                realloc_cmdbuff((*ccline).cmdlen + difflen + 4);
                (*xp).xp_pattern = (*ccline).cmdbuff.add(i as usize);
            }
            debug_assert!((*ccline).cmdpos <= (*ccline).cmdlen);
            ptr::copy(
                (*ccline).cmdbuff.add((*ccline).cmdpos as usize),
                (*ccline).cmdbuff.add(((*ccline).cmdpos + difflen) as usize),
                ((*ccline).cmdlen as usize) - ((*ccline).cmdpos as usize) + 1,
            );
            ptr::copy(p, (*ccline).cmdbuff.add(i as usize), plen);
            (*ccline).cmdlen += difflen;
            (*ccline).cmdpos += difflen;
        }

        redrawcmd();
        cursorcmd();

        if (*xp).xp_context == EXPAND_MAPPINGS && p.is_null() {
            return FAIL;
        }

        if (*xp).xp_numfiles <= 0 && p.is_null() {
            beep_flush();
        } else if (*xp).xp_numfiles == 1
            && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0
            && !wild_navigate
        {
            ExpandOne(xp, ptr::null_mut(), ptr::null_mut(), 0, WILD_FREE);
        }

        xfree(p.cast());

        OK
    }
}

/// Create completion popup menu with items from "matches".
unsafe fn cmdline_pum_create(
    ccline: *const CmdlineInfo,
    xp: *mut expand_T,
    matches: *mut *mut c_char,
    numMatches: c_int,
    showtail: bool,
    cmdline_unchanged: bool,
) {
    unsafe {
        debug_assert!(numMatches >= 0);
        compl_match_array =
            xmalloc(std::mem::size_of::<pumitem_T>() * (numMatches as usize)).cast::<pumitem_T>();
        compl_match_arraysize = numMatches;
        for i in 0..(numMatches as usize) {
            let pum_text =
                if !(*xp).xp_files_abbr.is_null() && !(*(*xp).xp_files_abbr.add(i)).is_null() {
                    *(*xp).xp_files_abbr.add(i)
                } else {
                    show_match(matches, i, showtail)
                };
            let pum_info = if !(*xp).xp_files_info.is_null() {
                *(*xp).xp_files_info.add(i)
            } else {
                ptr::null_mut()
            };
            let pum_extra = if !(*xp).xp_files_menu.is_null() {
                *(*xp).xp_files_menu.add(i)
            } else {
                ptr::null_mut()
            };
            let pum_kind = if !(*xp).xp_files_kind.is_null() {
                *(*xp).xp_files_kind.add(i)
            } else {
                ptr::null_mut()
            };
            *compl_match_array.add(i) = pumitem_T {
                pum_text,
                pum_kind,
                pum_extra,
                pum_info,
                pum_cpt_source_idx: 0,
                pum_user_abbr_hlattr: -1,
                pum_user_kind_hlattr: -1,
            };
        }

        let endpos = if showtail {
            showmatches_gettail((*xp).xp_pattern, cmdline_unchanged)
        } else {
            (*xp).xp_pattern
        };
        if ui_has(kUICmdline) && cmdline_win.is_null() {
            compl_startcol = endpos.offset_from((*ccline).cmdbuff) as c_int;
        } else {
            compl_startcol = cmd_screencol(endpos.offset_from((*ccline).cmdbuff) as c_int);
        }
    }
}

pub unsafe fn cmdline_pum_display(changed_array: bool) {
    unsafe {
        pum_display(
            compl_match_array,
            compl_match_arraysize,
            compl_selected,
            changed_array,
            compl_startcol,
        );
    }
}

/// Returns true if the cmdline completion popup menu is being displayed.
pub unsafe fn cmdline_pum_active() -> bool {
    unsafe { pum_visible() && !compl_match_array.is_null() }
}

/// Remove the cmdline completion popup menu (if present), free the list of items.
pub unsafe fn cmdline_pum_remove(defer_redraw: bool) {
    unsafe {
        pum_undisplay(!defer_redraw);
        xfree(compl_match_array.cast());
        compl_match_array = ptr::null_mut();
        compl_match_arraysize = 0;
    }
}

pub unsafe fn cmdline_pum_cleanup(cclp: *mut CmdlineInfo) {
    unsafe {
        cmdline_pum_remove(false);
        wildmenu_cleanup(cclp);
    }
}

/// Returns the current cmdline completion pattern.
pub unsafe fn cmdline_compl_pattern() -> *mut c_char {
    unsafe {
        let xp = (*get_cmdline_info()).xpc;
        if xp.is_null() {
            ptr::null_mut()
        } else {
            (*xp).xp_orig
        }
    }
}

/// Returns true if fuzzy cmdline completion is active, false otherwise.
pub unsafe fn cmdline_compl_is_fuzzy() -> bool {
    unsafe {
        let xp = (*get_cmdline_info()).xpc;
        !xp.is_null() && cmdline_fuzzy_completion_supported(xp)
    }
}

/// Checks whether popup menu should be used for cmdline completion wildmenu.
unsafe fn cmdline_compl_use_pum(need_wildmenu: bool) -> bool {
    unsafe {
        (need_wildmenu
            && (wop_flags & kOptWopFlagPum) != 0
            && !(ui_has(kUICmdline) && cmdline_win.is_null()))
            || ui_has(kUIWildmenu)
            || (ui_has(kUICmdline) && ui_has(kUIPopupmenu))
    }
}

/// Return the number of characters that should be skipped in the wildmenu.
unsafe fn skip_wildmenu_char(xp: *mut expand_T, s: *mut c_char) -> c_int {
    unsafe {
        if (rem_backslash(s)
            && (*xp).xp_context != EXPAND_HELP
            && (*xp).xp_context != EXPAND_PATTERN_IN_BUF)
            || (((*xp).xp_context == EXPAND_MENUS || (*xp).xp_context == EXPAND_MENUNAMES)
                && (*s == b'\t' as c_char || (*s == b'\\' as c_char && *s.add(1) != NUL)))
        {
            #[cfg(not(windows))]
            if (*xp).xp_shell
                && csh_like_shell() != 0
                && *s.add(1) == b'\\' as c_char
                && *s.add(2) == b'!' as c_char
            {
                return 2;
            }
            return 1;
        }
        0
    }
}

/// Get the length of an item as it will be shown in the status line.
unsafe fn wildmenu_match_len(xp: *mut expand_T, mut s: *mut c_char) -> c_int {
    unsafe {
        let mut len: c_int = 0;
        let emenu = (*xp).xp_context == EXPAND_MENUS || (*xp).xp_context == EXPAND_MENUNAMES;

        if emenu && menu_is_separator(s) {
            return 1;
        }

        while *s != NUL {
            s = s.add(skip_wildmenu_char(xp, s) as usize);
            len += ptr2cells(s);
            s = s.add(utfc_ptr2len(s) as usize);
        }

        len
    }
}

/// Show wildchar matches in the status line.
unsafe fn redraw_wildmenu(
    xp: *mut expand_T,
    num_matches: c_int,
    matches: *mut *mut c_char,
    mut match_idx: c_int,
    showtail: bool,
) {
    static mut first_match: c_int = 0;

    unsafe {
        let mut highlight = true;
        let mut selstart: *mut c_char = ptr::null_mut();
        let mut selstart_col: c_int = 0;
        let mut selend: *mut c_char = ptr::null_mut();
        let mut add_left = false;

        if matches.is_null() {
            return;
        }

        let buf = xmalloc((Columns as usize) * MB_MAXBYTES + 1).cast::<c_char>();

        if match_idx == -1 {
            match_idx = 0;
            highlight = false;
        }
        let mut clen =
            wildmenu_match_len(xp, show_match(matches, match_idx as usize, showtail)) + 3;
        if match_idx == 0 {
            first_match = 0;
        } else if match_idx < first_match {
            first_match = match_idx;
            add_left = true;
        } else {
            for i in first_match..match_idx {
                clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
            }
            if first_match > 0 {
                clen += 2;
            }
            if clen > Columns {
                first_match = match_idx;
                clen = 2;
                let mut i = match_idx;
                while i < num_matches {
                    clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
                    if clen >= Columns {
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
            while first_match > 0 {
                clen += wildmenu_match_len(
                    xp,
                    show_match(matches, (first_match - 1) as usize, showtail),
                ) + 2;
                if clen >= Columns {
                    break;
                }
                first_match -= 1;
            }
        }

        let mut group: hlf_T = 0;
        let fillchar = fillchar_status(&raw mut group, curwin);
        let attr = win_hl_attr(curwin, group);

        let mut len: usize;
        if first_match == 0 {
            *buf = NUL;
            len = 0;
        } else {
            c_strcpy(buf, c"< ".as_ptr());
            len = 2;
        }
        clen = len as c_int;

        let mut i = first_match;
        while clen + wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2 < Columns
        {
            if i == match_idx {
                selstart = buf.add(len);
                selstart_col = clen;
            }

            let mut s = show_match(matches, i as usize, showtail);
            let emenu = (*xp).xp_context == EXPAND_MENUS || (*xp).xp_context == EXPAND_MENUNAMES;
            if emenu && menu_is_separator(s) {
                c_strcpy(buf.add(len), transchar(b'|' as c_int));
                let l = strlen(buf.add(len));
                len += l;
                clen += l as c_int;
            } else {
                while *s != NUL {
                    s = s.add(skip_wildmenu_char(xp, s) as usize);
                    clen += ptr2cells(s);
                    let l = utfc_ptr2len(s);
                    if l > 1 {
                        ptr::copy_nonoverlapping(s, buf.add(len), l as usize);
                        s = s.add((l - 1) as usize);
                        len += l as usize;
                    } else {
                        c_strcpy(buf.add(len), transchar_byte(c_int::from(*s as u8)));
                        len += strlen(buf.add(len));
                    }
                    s = s.add(1);
                }
            }
            if i == match_idx {
                selend = buf.add(len);
            }

            *buf.add(len) = b' ' as c_char;
            len += 1;
            *buf.add(len) = b' ' as c_char;
            len += 1;
            clen += 2;
            i += 1;
            if i == num_matches {
                break;
            }
        }

        if i != num_matches {
            *buf.add(len) = b'>' as c_char;
            len += 1;
            clen += 1;
        }

        *buf.add(len) = NUL;

        let mut row = cmdline_row - 1;
        if row >= 0 {
            if wild_menu_showing == 0 {
                if msg_scrolled > 0 {
                    if cmdline_row == Rows - 1 {
                        msg_scroll_up(false, false);
                        msg_scrolled += 1;
                    } else {
                        cmdline_row += 1;
                        row += 1;
                    }
                    wild_menu_showing = WM_SCROLLED;
                } else {
                    if (*lastwin).w_status_height == 0 && global_stl_height() == 0 {
                        save_p_ls = p_ls as c_int;
                        save_p_wmh = p_wmh as c_int;
                        p_ls = 2;
                        p_wmh = 0;
                        last_status(false);
                    }
                    wild_menu_showing = WM_SHOWN;
                }
            }

            grid_line_start(
                if wild_menu_showing == WM_SCROLLED {
                    &raw mut msg_grid_adj
                } else {
                    &raw mut default_gridview
                },
                row,
            );

            grid_line_puts(0, buf, -1, attr);
            if !selstart.is_null() && highlight {
                *selend = NUL;
                grid_line_puts(selstart_col, selstart, -1, hl_attr(HLF_WM));
            }

            grid_line_fill(clen, Columns, fillchar, attr);

            grid_line_flush();
        }

        win_redraw_last_status(topframe);
        xfree(buf.cast());
    }
}

/// Get the next or prev cmdline completion match.
unsafe fn get_next_or_prev_match(mode: c_int, xp: *mut expand_T) -> *mut c_char {
    unsafe {
        if (*xp).xp_numfiles <= 0 {
            return ptr::null_mut();
        }

        let mut findex = (*xp).xp_selected;

        if mode == WILD_PREV {
            if findex == -1 {
                findex = (*xp).xp_numfiles;
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
                    findex = (*xp).xp_numfiles - 1;
                } else {
                    findex = (findex - ht).max(0);
                }
            } else if findex == (*xp).xp_numfiles - 1 {
                findex = -1;
            } else if findex < 0 {
                findex = 0;
            } else {
                findex = (findex + ht).min((*xp).xp_numfiles - 1);
            }
        } else {
            debug_assert!(pum_want.active);
            findex = pum_want.item;
        }

        if findex < 0 || findex >= (*xp).xp_numfiles {
            if !(*xp).xp_orig.is_null() {
                findex = -1;
            } else {
                findex = if findex < 0 { (*xp).xp_numfiles - 1 } else { 0 };
            }
        }

        if p_wmnu != 0 {
            if !compl_match_array.is_null() {
                compl_selected = findex;
                cmdline_pum_display(false);
            } else if cmdline_compl_use_pum(true) {
                cmdline_pum_create(
                    get_cmdline_info(),
                    xp,
                    (*xp).xp_files,
                    (*xp).xp_numfiles,
                    cmd_showtail,
                    false,
                );
                compl_selected = findex;
                pum_clear();
                cmdline_pum_display(true);
            } else {
                redraw_wildmenu(xp, (*xp).xp_numfiles, (*xp).xp_files, findex, cmd_showtail);
            }
        }

        (*xp).xp_selected = findex;
        xstrdup(if findex == -1 {
            (*xp).xp_orig
        } else {
            *(*xp).xp_files.add(findex as usize)
        })
    }
}

/// Start the command-line expansion and get the matches.
unsafe fn ExpandOne_start(
    mode: c_int,
    xp: *mut expand_T,
    str_arg: *mut c_char,
    options: c_int,
) -> *mut c_char {
    unsafe {
        let mut ss: *mut c_char = ptr::null_mut();

        if ExpandFromContext(
            xp,
            str_arg,
            &raw mut (*xp).xp_files,
            &raw mut (*xp).xp_numfiles,
            options,
        ) == FAIL
        {
            #[cfg(windows)]
            if (options & WILD_SILENT) == 0 && (options & WILD_LIST_NOTFOUND) != 0 {
                semsg!(gettext(e_nomatch2.as_ptr()), str_arg);
            }
        } else if (*xp).xp_numfiles == 0 {
            if (options & WILD_SILENT) == 0 {
                semsg!(gettext(e_nomatch2.as_ptr()), str_arg);
            }
        } else {
            ExpandEscape(xp, str_arg, (*xp).xp_numfiles, (*xp).xp_files, options);

            if mode != WILD_ALL && mode != WILD_ALL_KEEP && mode != WILD_LONGEST {
                let mut non_suf_match = if (*xp).xp_numfiles != 0 {
                    (*xp).xp_numfiles
                } else {
                    1
                };
                if ((*xp).xp_context == EXPAND_FILES || (*xp).xp_context == EXPAND_DIRECTORIES)
                    && (*xp).xp_numfiles > 1
                {
                    non_suf_match = 0;
                    for i in 0..2 {
                        if match_suffix(*(*xp).xp_files.add(i)) {
                            non_suf_match += 1;
                        }
                    }
                }
                if non_suf_match != 1 {
                    if (options & WILD_SILENT) == 0 {
                        emsg(gettext(e_toomany.as_ptr()));
                    } else if (options & WILD_NO_BEEP) == 0 {
                        beep_flush();
                    }
                }
                if !(non_suf_match != 1 && mode == WILD_EXPAND_FREE) {
                    ss = xstrdup(*(*xp).xp_files);
                }
            }
        }

        ss
    }
}

/// Return the longest common part in the list of cmdline completion matches.
unsafe fn find_longest_match(xp: *mut expand_T, options: c_int) -> *mut c_char {
    unsafe {
        let mut len: usize = 0;
        let first_file = *(*xp).xp_files;

        while *first_file.add(len) != NUL {
            let mb_len = utfc_ptr2len(first_file.add(len)) as usize;
            let c0 = utf_ptr2char(first_file.add(len));
            let mut i: c_int = 1;
            while i < (*xp).xp_numfiles {
                let ci = utf_ptr2char((*(*xp).xp_files.add(i as usize)).add(len));

                if p_fic != 0
                    && ((*xp).xp_context == EXPAND_DIRECTORIES
                        || (*xp).xp_context == EXPAND_FILES
                        || (*xp).xp_context == EXPAND_SHELLCMD
                        || (*xp).xp_context == EXPAND_BUFFERS)
                {
                    if mb_tolower(c0) != mb_tolower(ci) {
                        break;
                    }
                } else if c0 != ci {
                    break;
                }
                i += 1;
            }
            if i < (*xp).xp_numfiles {
                if (options & WILD_NO_BEEP) == 0 {
                    vim_beep(kOptBoFlagWildmode);
                }
                break;
            }
            len += mb_len;
        }

        xmemdupz(first_file.cast(), len).cast()
    }
}

pub unsafe fn free_xp_files_extra(xp: *mut expand_T, numfiles: c_int) {
    unsafe {
        if !(*xp).xp_files_abbr.is_null() {
            FreeWild(numfiles, (*xp).xp_files_abbr);
            (*xp).xp_files_abbr = ptr::null_mut();
        }
        if !(*xp).xp_files_kind.is_null() {
            FreeWild(numfiles, (*xp).xp_files_kind);
            (*xp).xp_files_kind = ptr::null_mut();
        }
        if !(*xp).xp_files_menu.is_null() {
            FreeWild(numfiles, (*xp).xp_files_menu);
            (*xp).xp_files_menu = ptr::null_mut();
        }
        if !(*xp).xp_files_info.is_null() {
            FreeWild(numfiles, (*xp).xp_files_info);
            (*xp).xp_files_info = ptr::null_mut();
        }
    }
}

/// Do wildcard expansion on the string "str".
pub unsafe fn ExpandOne(
    xp: *mut expand_T,
    str_arg: *mut c_char,
    orig: *mut c_char,
    options: c_int,
    mode: c_int,
) -> *mut c_char {
    unsafe {
        let mut ss: *mut c_char = ptr::null_mut();
        let mut orig_saved = false;

        if mode == WILD_NEXT
            || mode == WILD_PREV
            || mode == WILD_PAGEUP
            || mode == WILD_PAGEDOWN
            || mode == WILD_PUM_WANT
        {
            return get_next_or_prev_match(mode, xp);
        }

        if mode == WILD_CANCEL {
            ss = xstrdup(if !(*xp).xp_orig.is_null() {
                (*xp).xp_orig
            } else {
                c"".as_ptr()
            });
        } else if mode == WILD_APPLY {
            ss = xstrdup(if (*xp).xp_selected == -1 {
                if !(*xp).xp_orig.is_null() {
                    (*xp).xp_orig
                } else {
                    c"".as_ptr()
                }
            } else {
                *(*xp).xp_files.add((*xp).xp_selected as usize)
            });
        }

        if (*xp).xp_numfiles != -1 && mode != WILD_ALL && mode != WILD_LONGEST {
            FreeWild((*xp).xp_numfiles, (*xp).xp_files);
            free_xp_files_extra(xp, (*xp).xp_numfiles);
            (*xp).xp_numfiles = -1;
            xfree((*xp).xp_orig.cast());
            (*xp).xp_orig = ptr::null_mut();

            if !compl_match_array.is_null() {
                cmdline_pum_remove(false);
            }
        }
        (*xp).xp_selected = if (options & WILD_NOSELECT) != 0 {
            -1
        } else {
            0
        };

        if mode == WILD_FREE {
            return ptr::null_mut();
        }

        if (*xp).xp_numfiles == -1 && mode != WILD_APPLY && mode != WILD_CANCEL {
            xfree((*xp).xp_orig.cast());
            (*xp).xp_orig = orig;
            orig_saved = true;

            ss = ExpandOne_start(mode, xp, str_arg, options);
        }

        if mode == WILD_LONGEST && (*xp).xp_numfiles > 0 {
            ss = find_longest_match(xp, options);
            (*xp).xp_selected = -1;
        }

        if mode == WILD_ALL && (*xp).xp_numfiles > 0 && !got_int {
            let mut ss_size: usize = 0;
            let mut prefix: *const c_char = c"".as_ptr();
            let suffix: *const c_char = if (options & WILD_USE_NL) != 0 {
                c"\n".as_ptr()
            } else {
                c" ".as_ptr()
            };
            let n = (*xp).xp_numfiles - 1;

            if (*xp).xp_prefix == XP_PREFIX_NO {
                prefix = c"no".as_ptr();
                ss_size = 2 * (n as usize);
            } else if (*xp).xp_prefix == XP_PREFIX_INV {
                prefix = c"inv".as_ptr();
                ss_size = 3 * (n as usize);
            }

            for i in 0..((*xp).xp_numfiles as usize) {
                ss_size += strlen(*(*xp).xp_files.add(i)) + 1;
            }
            ss_size += 1;

            ss = xmalloc(ss_size).cast::<c_char>();
            *ss = NUL;
            let mut ssp = ss;
            for i in 0..(*xp).xp_numfiles {
                if i > 0 {
                    ssp = xstpcpy(ssp, prefix);
                }
                ssp = xstpcpy(ssp, *(*xp).xp_files.add(i as usize));
                if i < n {
                    ssp = xstpcpy(ssp, suffix);
                }
                debug_assert!((ssp as usize) < (ss.add(ss_size) as usize));
            }
        }

        if mode == WILD_EXPAND_FREE || mode == WILD_ALL {
            ExpandCleanup(xp);
        }

        if !orig_saved {
            xfree(orig.cast());
        }

        ss
    }
}

/// Prepare an expand structure for use.
pub unsafe fn ExpandInit(xp: *mut expand_T) {
    unsafe {
        ptr::write_bytes(xp, 0, 1);
        (*xp).xp_backslash = XP_BS_NONE;
        (*xp).xp_prefix = XP_PREFIX_NONE;
        (*xp).xp_numfiles = -1;
    }
}

/// Cleanup an expand structure after use.
pub unsafe fn ExpandCleanup(xp: *mut expand_T) {
    unsafe {
        if (*xp).xp_numfiles >= 0 {
            free_xp_files_extra(xp, (*xp).xp_numfiles);
            FreeWild((*xp).xp_numfiles, (*xp).xp_files);
            (*xp).xp_numfiles = -1;
        }
        xfree((*xp).xp_orig.cast());
        (*xp).xp_orig = ptr::null_mut();
    }
}

pub unsafe fn clear_cmdline_orig() {
    unsafe {
        xfree(cmdline_orig.data.cast());
        cmdline_orig.data = ptr::null_mut();
        cmdline_orig.size = 0;
    }
}

/// Display one line of completion matches.
unsafe fn showmatches_oneline(
    xp: *mut expand_T,
    matches: *mut *mut c_char,
    numMatches: c_int,
    lines: c_int,
    linenr: c_int,
    maxlen: c_int,
    showtail: bool,
) {
    unsafe {
        let mut lastlen: c_int = 999;
        let mut j = linenr;
        while j < numMatches {
            if (*xp).xp_context == EXPAND_TAGS_LISTFILES {
                let m = *matches.add(j as usize);
                msg_outtrans(m, HLF_D, false);
                let p = m.add(strlen(m) + 1);
                msg_advance(maxlen + 1);
                msg_puts(p);
                msg_advance(maxlen + 3);
                msg_outtrans_long(p.add(2), HLF_D);
                break;
            }
            let mut i = maxlen - lastlen;
            while {
                i -= 1;
                i >= 0
            } {
                msg_putchar(b' ' as c_int);
            }
            let isdir: bool;
            let p: *mut c_char;
            if (*xp).xp_context == EXPAND_FILES
                || (*xp).xp_context == EXPAND_SHELLCMD
                || (*xp).xp_context == EXPAND_BUFFERS
            {
                let match_j = *matches.add(j as usize);
                if (*xp).xp_numfiles != -1 {
                    let exp_path = expand_env_save_opt(match_j, true, ptr::null_mut());
                    let path = if !exp_path.is_null() {
                        exp_path
                    } else {
                        match_j
                    };
                    let halved_slash = backslash_halve_save(path);
                    isdir = os_isdir(halved_slash);
                    xfree(exp_path.cast());
                    if halved_slash != path {
                        xfree(halved_slash.cast());
                    }
                } else {
                    isdir = os_isdir(match_j);
                }
                if showtail {
                    p = show_match(matches, j as usize, showtail);
                } else {
                    let name_buff = (&raw mut NameBuff).cast::<c_char>();
                    home_replace(ptr::null(), match_j, name_buff, MAXPATHL, true);
                    p = name_buff;
                }
            } else {
                isdir = false;
                p = show_match(matches, j as usize, showtail);
            }
            lastlen = msg_outtrans(p, if isdir { HLF_D } else { 0 }, false);
            j += lines;
        }
        if msg_col > 0 {
            msg_clr_eos();
            msg_putchar(b'\n' as c_int);
        }
    }
}

#[inline]
unsafe fn show_pum_matches(
    ccline: *mut CmdlineInfo,
    xp: *mut expand_T,
    matches: *mut *mut c_char,
    numMatches: c_int,
    showtail: bool,
    noselect: bool,
    cmdline_unchanged: bool,
) {
    unsafe {
        cmdline_pum_create(ccline, xp, matches, numMatches, showtail, cmdline_unchanged);
        compl_selected = if noselect { -1 } else { 0 };
        pum_clear();
        cmdline_pum_display(true);
    }
}

/// Display completion matches.
pub unsafe fn showmatches(
    xp: *mut expand_T,
    display_wildmenu: bool,
    display_list: bool,
    wim_flags_arg: c_int,
) -> c_int {
    unsafe {
        let ccline = get_cmdline_info();
        let mut numMatches: c_int = 0;
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let showtail: bool;
        let noselect = (wim_flags_arg & c_int::from(kOptWimFlagNoselect)) != 0;
        let noinsert = (wim_flags_arg & c_int::from(kOptWimFlagNoinsert)) != 0;
        let cmdline_unchanged = noselect || noinsert;

        if (*xp).xp_numfiles == -1 {
            set_expand_context(xp);
            if (*xp).xp_context == EXPAND_LUA {
                nlua_expand_pat(xp);
            }
            let retval = expand_cmdline(
                xp,
                (*ccline).cmdbuff,
                (*ccline).cmdpos,
                &raw mut numMatches,
                &raw mut matches,
            );
            if retval != EXPAND_OK {
                return retval;
            }
            showtail = expand_showtail(xp);
        } else {
            numMatches = (*xp).xp_numfiles;
            matches = (*xp).xp_files;
            showtail = cmd_showtail;
        }

        if cmdline_compl_use_pum(display_wildmenu && !display_list) {
            show_pum_matches(
                ccline,
                xp,
                matches,
                numMatches,
                showtail,
                noselect,
                cmdline_unchanged,
            );
            return EXPAND_OK;
        }

        if display_list {
            msg_didany = false;
            msg_start();
            if !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }
            ui_flush();
            cmdline_row = msg_row;
            msg_didany = false;
            msg_ext_set_kind(c"wildlist".as_ptr());
            msg_start();
        }

        if got_int {
            got_int = false;
        } else if display_wildmenu && !display_list {
            redraw_wildmenu(
                xp,
                numMatches,
                matches,
                if noselect { -1 } else { 0 },
                showtail,
            );
        } else if display_list {
            let mut maxlen: c_int = 0;
            for i in 0..(numMatches as usize) {
                let len = if !showtail
                    && ((*xp).xp_context == EXPAND_FILES
                        || (*xp).xp_context == EXPAND_SHELLCMD
                        || (*xp).xp_context == EXPAND_BUFFERS)
                {
                    let name_buff = (&raw mut NameBuff).cast::<c_char>();
                    home_replace(ptr::null(), *matches.add(i), name_buff, MAXPATHL, true);
                    vim_strsize(name_buff)
                } else {
                    vim_strsize(show_match(matches, i, showtail))
                };
                maxlen = maxlen.max(len);
            }

            let lines = if (*xp).xp_context == EXPAND_TAGS_LISTFILES {
                numMatches
            } else {
                maxlen += 2;
                let mut columns = (Columns + 2) / maxlen;
                if columns < 1 {
                    columns = 1;
                }
                (numMatches + columns - 1) / columns
            };

            if (*xp).xp_context == EXPAND_TAGS_LISTFILES {
                msg_puts_hl(gettext(c"tagname".as_ptr()), HLF_T, false);
                msg_clr_eos();
                msg_advance(maxlen - 3);
                msg_puts_hl(gettext(c" kind file\n".as_ptr()), HLF_T, false);
            }

            for i in 0..lines {
                showmatches_oneline(xp, matches, numMatches, lines, i, maxlen, showtail);
                if got_int {
                    got_int = false;
                    break;
                }
            }

            cmdline_row = msg_row;
        }

        if display_wildmenu && display_list {
            if cmdline_compl_use_pum(true) {
                show_pum_matches(
                    ccline,
                    xp,
                    matches,
                    numMatches,
                    showtail,
                    noselect,
                    cmdline_unchanged,
                );
            } else {
                redraw_wildmenu(
                    xp,
                    numMatches,
                    matches,
                    if noselect { -1 } else { 0 },
                    showtail,
                );
            }
        }

        if (*xp).xp_numfiles == -1 {
            FreeWild(numMatches, matches);
            free_xp_files_extra(xp, numMatches);
        }

        EXPAND_OK
    }
}

/// path_tail() version for showmatches() and redraw_wildmenu().
unsafe fn showmatches_gettail(s: *mut c_char, eager: bool) -> *mut c_char {
    unsafe {
        let mut t = s;
        let mut had_sep = false;

        let mut p = s;
        while *p != NUL {
            #[cfg(windows)]
            let is_sep = vim_ispathsep(*p as c_int) && !rem_backslash(p);
            #[cfg(not(windows))]
            let is_sep = vim_ispathsep(*p as c_int);

            if is_sep {
                if eager {
                    t = p.add(1);
                } else {
                    had_sep = true;
                }
            } else if had_sep {
                t = p;
                had_sep = false;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        t
    }
}

/// Return true if we only need to show the tail of completion matches.
unsafe fn expand_showtail(xp: *mut expand_T) -> bool {
    unsafe {
        if (*xp).xp_context != EXPAND_FILES
            && (*xp).xp_context != EXPAND_SHELLCMD
            && (*xp).xp_context != EXPAND_DIRECTORIES
        {
            return false;
        }

        let end = path_tail((*xp).xp_pattern);
        if end == (*xp).xp_pattern {
            return false;
        }

        let mut s = (*xp).xp_pattern;
        while (s as usize) < (end as usize) {
            if rem_backslash(s) {
                s = s.add(1);
            } else if !vim_strchr(c"*?[".as_ptr(), *s as u8 as c_int).is_null() {
                return false;
            }
            s = s.add(1);
        }
        true
    }
}

/// Prepare a string for expansion.
pub unsafe fn addstar(fname: *mut c_char, mut len: usize, context: c_int) -> *mut c_char {
    unsafe {
        let retval: *mut c_char;

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
                    && *fname == b'/' as c_char)
                || context == EXPAND_CHECKHEALTH
                || context == EXPAND_LSP
                || context == EXPAND_LOG
                || context == EXPAND_PACKDEL
                || context == EXPAND_PACKUPDATE
                || context == EXPAND_MARKS
                || context == EXPAND_LUA
            {
                retval = xstrnsave(fname, len);
            } else {
                let mut new_len = len + 2;
                for i in 0..len {
                    let ch = *fname.add(i);
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
                retval = xmalloc(new_len).cast::<c_char>();
                *retval = b'^' as c_char;
                let mut j: usize = 1;
                let mut i: usize = 0;
                while i < len {
                    if context != EXPAND_USER_DEFINED
                        && context != EXPAND_USER_LIST
                        && *fname.add(i) == b'\\' as c_char
                    {
                        i += 1;
                        if i == len {
                            break;
                        }
                    }

                    match *fname.add(i) as u8 {
                        b'*' => {
                            *retval.add(j) = b'.' as c_char;
                            j += 1;
                        }
                        b'~' => {
                            *retval.add(j) = b'\\' as c_char;
                            j += 1;
                        }
                        b'?' => {
                            *retval.add(j) = b'.' as c_char;
                            i += 1;
                            j += 1;
                            continue;
                        }
                        b'.' if context == EXPAND_BUFFERS => {
                            *retval.add(j) = b'\\' as c_char;
                            j += 1;
                        }
                        b'\\' if context == EXPAND_USER_DEFINED || context == EXPAND_USER_LIST => {
                            *retval.add(j) = b'\\' as c_char;
                            j += 1;
                        }
                        _ => {}
                    }
                    *retval.add(j) = *fname.add(i);
                    i += 1;
                    j += 1;
                }
                *retval.add(j) = NUL;
            }
        } else {
            retval = xmalloc(len + 4).cast::<c_char>();
            xmemcpyz(retval.cast(), fname.cast(), len);

            let tail = path_tail(retval);
            #[cfg(windows)]
            let ends_in_star = len > 0 && *retval.add(len - 1) == b'*' as c_char;
            #[cfg(not(windows))]
            let mut ends_in_star = len > 0 && *retval.add(len - 1) == b'*' as c_char;
            #[cfg(not(windows))]
            {
                let mut k = (len as isize) - 2;
                while k >= 0 {
                    if *retval.add(k as usize) != b'\\' as c_char {
                        break;
                    }
                    ends_in_star = !ends_in_star;
                    k -= 1;
                }
            }
            if (*retval != b'~' as c_char || tail != retval)
                && !ends_in_star
                && vim_strchr(tail, b'$' as c_int).is_null()
                && vim_strchr(retval, b'`' as c_int).is_null()
            {
                *retval.add(len) = b'*' as c_char;
                len += 1;
            } else if len > 0 && *retval.add(len - 1) == b'$' as c_char {
                len -= 1;
            }
            *retval.add(len) = NUL;
        }
        retval
    }
}

/// Must parse the command line so far to work out what context we are in.
pub unsafe fn set_expand_context(xp: *mut expand_T) {
    unsafe {
        let ccline = get_cmdline_info();

        if ((*ccline).cmdfirstc == b'/' as c_int || (*ccline).cmdfirstc == b'?' as c_int)
            && may_expand_pattern
        {
            (*xp).xp_context = EXPAND_PATTERN_IN_BUF;
            (*xp).xp_search_dir = if (*ccline).cmdfirstc == b'/' as c_int {
                FORWARD
            } else {
                BACKWARD
            };
            (*xp).xp_pattern = (*ccline).cmdbuff;
            (*xp).xp_pattern_len = (*ccline).cmdpos as usize;
            Search.first_line = 0;
            return;
        }

        if (*ccline).cmdfirstc != b':' as c_int
            && (*ccline).cmdfirstc != b'>' as c_int
            && (*ccline).cmdfirstc != b'=' as c_int
            && (*ccline).input_fn == 0
        {
            (*xp).xp_context = EXPAND_NOTHING;
            return;
        }

        set_cmd_context(
            xp,
            (*ccline).cmdbuff,
            (*ccline).cmdlen,
            (*ccline).cmdpos,
            true as c_int,
        );
    }
}

/// Sets the index of a built-in or user defined command "cmd" in eap->cmdidx.
unsafe fn set_cmd_index(
    cmd: *const c_char,
    eap: *mut exarg_T,
    xp: *mut expand_T,
    complp: *mut c_int,
) -> *const c_char {
    unsafe {
        let mut p: *const c_char;
        let fuzzy = cmdline_fuzzy_complete(cmd);

        if !fuzzy && p_ic == 0 && *cmd == b'k' as c_char && *cmd.add(1) != b'e' as c_char {
            (*eap).cmdidx = cmdidx_T::CMD_k as c_int;
            p = cmd.add(1);
        } else {
            p = cmd;
            while ascii_isalpha(*p) || *p == b'*' as c_char {
                p = p.add(1);
            }
            if ascii_isupper(*cmd) {
                while ascii_isalnum(*p) || *p == b'*' as c_char {
                    p = p.add(1);
                }
            }
            if *cmd == b'p' as c_char
                && *cmd.add(1) == b'y' as c_char
                && p == cmd.add(2)
                && *p == b'3' as c_char
            {
                p = p.add(1);
                while ascii_isalpha(*p) || *p == b'*' as c_char {
                    p = p.add(1);
                }
            }
            if p == cmd && !vim_strchr(c"@*!=><&~#".as_ptr(), *p as u8 as c_int).is_null() {
                p = p.add(1);
            }
            let len = p.offset_from(cmd) as usize;

            if len == 0 {
                (*xp).xp_context = EXPAND_UNSUCCESSFUL;
                return ptr::null();
            }

            (*eap).cmdidx = excmd_get_cmdidx(cmd, len) as c_int;

            if (*cmd >= b'A' as c_char && *cmd <= b'Z' as c_char)
                || (fuzzy && (*eap).cmdidx != cmdidx_T::CMD_bang as c_int as c_int && *p != NUL)
            {
                while ascii_isalnum(*p) || *p == b'*' as c_char {
                    p = p.add(1);
                }
            }
        }

        if *p == NUL && ascii_isalnum(*p.sub(1)) {
            return ptr::null();
        }

        if (*eap).cmdidx == cmdidx_T::CMD_SIZE as c_int as c_int {
            if *cmd == b's' as c_char
                && !vim_strchr(c"cgriI".as_ptr(), *cmd.add(1) as u8 as c_int).is_null()
            {
                (*eap).cmdidx = cmdidx_T::CMD_substitute as c_int;
                p = cmd.add(1);
            } else if *cmd >= b'A' as c_char && *cmd <= b'Z' as c_char {
                (*eap).cmd = cmd.cast_mut();
                p = find_ucmd(eap.cast(), p.cast_mut(), ptr::null_mut(), xp, complp);
                if p.is_null() {
                    (*eap).cmdidx = cmdidx_T::CMD_SIZE as c_int;
                }
            }
        }
        if (*eap).cmdidx == cmdidx_T::CMD_SIZE as c_int as c_int {
            (*xp).xp_context = EXPAND_UNSUCCESSFUL;
            return ptr::null();
        }

        p
    }
}

/// Set the completion context for a command argument with wild card characters.
unsafe fn set_context_for_wildcard_arg(
    eap: *mut exarg_T,
    arg: *const c_char,
    usefilter: bool,
    xp: *mut expand_T,
    complp: *mut c_int,
) {
    unsafe {
        let mut in_quote = false;
        let mut bow: *const c_char = ptr::null();
        let mut len: usize;

        (*xp).xp_pattern = skipwhite(arg);
        let mut p: *const c_char = (*xp).xp_pattern;
        while *p != NUL {
            let mut c = utf_ptr2char(p);
            if c == b'\\' as c_int && *p.add(1) != NUL {
                p = p.add(1);
            } else if c == b'`' as c_int {
                if !in_quote {
                    (*xp).xp_pattern = p.cast_mut();
                    bow = p.add(1);
                }
                in_quote = !in_quote;
            } else if c == b'|' as c_int
                || c == b'\n' as c_int
                || c == b'"' as c_int
                || (ascii_iswhite(c)
                    && (!(!eap.is_null() && ((*eap).argt & EX_NOSPC) != 0) || usefilter))
            {
                len = 0;
                while *p != NUL {
                    c = utf_ptr2char(p);
                    if c == b'`' as c_int || vim_isfilec_or_wc(c) {
                        break;
                    }
                    len = utfc_ptr2len(p) as usize;
                    p = p.add(utfc_ptr2len(p) as usize);
                }
                if in_quote {
                    bow = p;
                } else {
                    (*xp).xp_pattern = p.cast_mut();
                }
                p = p.sub(len);
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        if !bow.is_null() && in_quote {
            (*xp).xp_pattern = bow.cast_mut();
        }
        (*xp).xp_context = EXPAND_FILES;

        if usefilter
            || (!eap.is_null()
                && ((*eap).cmdidx == cmdidx_T::CMD_bang as c_int
                    || (*eap).cmdidx == cmdidx_T::CMD_terminal as c_int))
            || *complp == EXPAND_SHELLCMDLINE
        {
            #[cfg(not(windows))]
            {
                (*xp).xp_shell = true;
            }
            if (*xp).xp_pattern == skipwhite(arg) {
                (*xp).xp_context = EXPAND_SHELLCMD;
            }
        }

        if *(*xp).xp_pattern == b'$' as c_char {
            p = (*xp).xp_pattern.add(1);
            p = p.add(env_name_len(p));
            if *p == NUL {
                (*xp).xp_context = EXPAND_ENV_VARS;
                (*xp).xp_pattern = (*xp).xp_pattern.add(1);
                if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST {
                    *complp = EXPAND_ENV_VARS;
                }
            }
        }

        if *(*xp).xp_pattern == b'~' as c_char {
            p = (*xp).xp_pattern.add(1);
            while *p != NUL && *p != b'/' as c_char {
                p = p.add(1);
            }
            if *p == NUL
                && (p as usize) > ((*xp).xp_pattern.add(1) as usize)
                && match_user((*xp).xp_pattern.add(1)) >= 1
            {
                (*xp).xp_context = EXPAND_USER;
                (*xp).xp_pattern = (*xp).xp_pattern.add(1);
            }
        }
    }
}

/// Set the completion context for the "++opt=arg" argument. Always returns NULL.
unsafe fn set_context_in_argopt(xp: *mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        let p = vim_strchr(arg, b'=' as c_int);
        if p.is_null() {
            (*xp).xp_pattern = arg.cast_mut();
        } else {
            (*xp).xp_pattern = p.add(1);
        }

        (*xp).xp_context = EXPAND_ARGOPT;
        ptr::null()
    }
}

/// Set the completion context for the :filter command.
unsafe fn set_context_in_filter_cmd(xp: *mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        if *arg != NUL {
            arg = skip_vimgrep_pat(arg.cast_mut(), ptr::null_mut(), ptr::null_mut());
        }
        if arg.is_null() || *arg == NUL {
            (*xp).xp_context = EXPAND_NOTHING;
            return ptr::null();
        }
        skipwhite(arg)
    }
}

/// Set the completion context for the :match command.
unsafe fn set_context_in_match_cmd(xp: *mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        if *arg == NUL || ends_excmd(*arg as c_int) == 0 {
            set_context_in_echohl_cmd(xp, arg);
            arg = skipwhite(skiptowhite(arg));
            if *arg != NUL {
                (*xp).xp_context = EXPAND_NOTHING;
                arg = skip_regexp(arg.add(1).cast_mut(), *arg as u8 as c_int, p_magic);
            }
        }
        find_nextcmd(arg)
    }
}

/// Returns a pointer to the next command after a :global or a :v command.
unsafe fn find_cmd_after_global_cmd(mut arg: *const c_char) -> *const c_char {
    unsafe {
        let delim = *arg as u8 as c_int;
        if delim != 0 {
            arg = arg.add(1);
        }

        while *arg != NUL && (*arg as u8 as c_int) != delim {
            if *arg == b'\\' as c_char && *arg.add(1) != NUL {
                arg = arg.add(1);
            }
            arg = arg.add(1);
        }
        if *arg != NUL {
            return arg.add(1);
        }

        ptr::null()
    }
}

/// Returns a pointer to the next command after a :substitute or a :& command.
unsafe fn find_cmd_after_substitute_cmd(mut arg: *const c_char) -> *const c_char {
    unsafe {
        let delim = *arg as u8 as c_int;
        if delim != 0 {
            arg = arg.add(1);
            arg = skip_regexp(arg.cast_mut(), delim, p_magic);

            if *arg != NUL && (*arg as u8 as c_int) == delim {
                arg = arg.add(1);
                while *arg != NUL && (*arg as u8 as c_int) != delim {
                    if *arg == b'\\' as c_char && *arg.add(1) != NUL {
                        arg = arg.add(1);
                    }
                    arg = arg.add(1);
                }
                if *arg != NUL {
                    arg = arg.add(1);
                }
            }
        }
        while *arg != NUL && c_strchr(c"|\"#".as_ptr(), *arg).is_null() {
            arg = arg.add(1);
        }
        if *arg != NUL {
            return arg;
        }

        ptr::null()
    }
}

/// Returns a pointer to the next command after an :isearch/:dsearch/etc. command.
unsafe fn find_cmd_after_isearch_cmd(xp: *mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        arg = skipwhite(skipdigits(arg));
        if *arg != b'/' as c_char {
            return ptr::null();
        }

        arg = arg.add(1);
        while *arg != NUL && *arg != b'/' as c_char {
            if *arg == b'\\' as c_char && *arg.add(1) != NUL {
                arg = arg.add(1);
            }
            arg = arg.add(1);
        }
        if *arg != NUL {
            arg = skipwhite(arg.add(1));

            if *arg == NUL || c_strchr(c"|\"\n".as_ptr(), *arg).is_null() {
                (*xp).xp_context = EXPAND_NOTHING;
            } else {
                return arg;
            }
        }

        ptr::null()
    }
}

/// Set the completion context for the :unlet command. Always returns NULL.
unsafe fn set_context_in_unlet_cmd(xp: *mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        while {
            (*xp).xp_pattern = c_strchr(arg, b' ' as c_char);
            !(*xp).xp_pattern.is_null()
        } {
            arg = (*xp).xp_pattern.add(1);
        }

        (*xp).xp_context = EXPAND_USER_VARS;
        (*xp).xp_pattern = arg.cast_mut();

        if *(*xp).xp_pattern == b'$' as c_char {
            (*xp).xp_context = EXPAND_ENV_VARS;
            (*xp).xp_pattern = (*xp).xp_pattern.add(1);
        }

        ptr::null()
    }
}

/// Set the completion context for the :language command. Always returns NULL.
unsafe fn set_context_in_lang_cmd(xp: *mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        let p = skiptowhite(arg);
        if *p == NUL {
            (*xp).xp_context = EXPAND_LANGUAGE;
            (*xp).xp_pattern = arg.cast_mut();
        } else {
            let n = p.offset_from(arg) as usize;
            if strncmp(arg, c"messages".as_ptr(), n) == 0
                || strncmp(arg, c"ctype".as_ptr(), n) == 0
                || strncmp(arg, c"time".as_ptr(), n) == 0
                || strncmp(arg, c"collate".as_ptr(), n) == 0
            {
                (*xp).xp_context = EXPAND_LOCALES;
                (*xp).xp_pattern = skipwhite(p);
            } else {
                (*xp).xp_context = EXPAND_NOTHING;
            }
        }

        ptr::null()
    }
}

/// Set the completion context for the :breakadd command. Always returns NULL.
unsafe fn set_context_in_breakadd_cmd(
    xp: *mut expand_T,
    arg: *const c_char,
    cmdidx: cmdidx_T,
) -> *const c_char {
    unsafe {
        (*xp).xp_context = EXPAND_BREAKPOINT;
        (*xp).xp_pattern = arg.cast_mut();

        if cmdidx == cmdidx_T::CMD_breakadd {
            breakpt_expand_what = BreakptExpandWhat::Add;
        } else if cmdidx == cmdidx_T::CMD_breakdel {
            breakpt_expand_what = BreakptExpandWhat::Del;
        } else {
            breakpt_expand_what = BreakptExpandWhat::Profdel;
        }

        let mut p: *const c_char = skipwhite(arg);
        if *p == NUL {
            return ptr::null();
        }
        let subcmd_start = p;

        if strncmp(c"file ".as_ptr(), p, 5) == 0 || strncmp(c"func ".as_ptr(), p, 5) == 0 {
            p = p.add(4);
            p = skipwhite(p);

            if ascii_isdigit(*p as c_int) {
                p = skipdigits(p);
                if *p != b' ' as c_char {
                    (*xp).xp_context = EXPAND_NOTHING;
                    return ptr::null();
                }
                p = skipwhite(p);
            }
            if strncmp(c"file".as_ptr(), subcmd_start, 4) == 0 {
                (*xp).xp_context = EXPAND_FILES;
            } else {
                (*xp).xp_context = EXPAND_USER_FUNC;
            }
            (*xp).xp_pattern = p.cast_mut();
        } else if strncmp(c"expr ".as_ptr(), p, 5) == 0 {
            (*xp).xp_context = EXPAND_EXPRESSION;
            (*xp).xp_pattern = skipwhite(p.add(5));
        }

        ptr::null()
    }
}

unsafe fn set_context_in_scriptnames_cmd(xp: *mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        (*xp).xp_context = EXPAND_NOTHING;
        (*xp).xp_pattern = ptr::null_mut();

        let p = skipwhite(arg);
        if ascii_isdigit(*p as c_int) {
            return ptr::null();
        }

        (*xp).xp_context = EXPAND_SCRIPTNAMES;
        (*xp).xp_pattern = p;

        ptr::null()
    }
}

/// Set the completion context for the :filetype command. Always returns NULL.
unsafe fn set_context_in_filetype_cmd(xp: *mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        (*xp).xp_context = EXPAND_FILETYPECMD;
        (*xp).xp_pattern = arg.cast_mut();
        filetype_expand_what = FiletypeExpandWhat::All;

        let mut p = skipwhite(arg);
        if *p == NUL {
            return ptr::null();
        }

        let mut val: c_int = 0;

        loop {
            if strncmp(p, c"plugin".as_ptr(), 6) == 0 && ascii_iswhite(*p.add(6) as c_int) {
                val |= EXPAND_FILETYPECMD_PLUGIN;
                p = skipwhite(p.add(6));
                continue;
            }
            if strncmp(p, c"indent".as_ptr(), 6) == 0 && ascii_iswhite(*p.add(6) as c_int) {
                val |= EXPAND_FILETYPECMD_INDENT;
                p = skipwhite(p.add(6));
                continue;
            }
            break;
        }

        if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 && (val & EXPAND_FILETYPECMD_INDENT) != 0 {
            filetype_expand_what = FiletypeExpandWhat::OnOff;
        } else if (val & EXPAND_FILETYPECMD_PLUGIN) != 0 {
            filetype_expand_what = FiletypeExpandWhat::Indent;
        } else if (val & EXPAND_FILETYPECMD_INDENT) != 0 {
            filetype_expand_what = FiletypeExpandWhat::Plugin;
        }

        (*xp).xp_pattern = p;

        ptr::null()
    }
}

/// Sets the completion context for commands that involve a search pattern and a line range.
unsafe fn set_context_with_pattern(xp: *mut expand_T) {
    unsafe {
        let ccline = get_cmdline_info();

        emsg_off += 1;
        let mut skiplen: c_int = 0;
        let mut dummy: c_int = 0;
        let mut patlen: c_int = 0;
        let mut magic = p_magic != 0;
        let retval = parse_pattern_and_range(
            &raw mut pre_incsearch_pos,
            &raw mut dummy,
            &raw mut skiplen,
            &raw mut patlen,
            &raw mut magic,
        );
        emsg_off -= 1;

        if !retval || (*ccline).cmdpos <= skiplen || (*ccline).cmdpos > skiplen + patlen {
            return;
        }

        (*xp).xp_pattern = (*ccline).cmdbuff.add(skiplen as usize);
        (*xp).xp_pattern_len = ((*ccline).cmdpos - skiplen) as usize;
        (*xp).xp_context = EXPAND_PATTERN_IN_BUF;
        (*xp).xp_search_dir = FORWARD;
    }
}

/// Set the completion context in "xp" for command "cmd" with index "cmdidx".
unsafe fn set_context_by_cmdname(
    cmd: *const c_char,
    cmdidx: cmdidx_T,
    xp: *mut expand_T,
    mut arg: *const c_char,
    argt: u32,
    context: c_int,
    forceit: bool,
) -> *const c_char {
    unsafe {
        match cmdidx {
            cmdidx_T::CMD_find | cmdidx_T::CMD_sfind | cmdidx_T::CMD_tabfind => {
                if (*xp).xp_context == EXPAND_FILES {
                    (*xp).xp_context = if (*get_findfunc()).cb_type != 0 {
                        EXPAND_FINDFUNC
                    } else {
                        EXPAND_FILES_IN_PATH
                    };
                }
            }
            cmdidx_T::CMD_bcd
            | cmdidx_T::CMD_bchdir
            | cmdidx_T::CMD_cd
            | cmdidx_T::CMD_chdir
            | cmdidx_T::CMD_lcd
            | cmdidx_T::CMD_lchdir
            | cmdidx_T::CMD_tcd
            | cmdidx_T::CMD_tchdir => {
                if (*xp).xp_context == EXPAND_FILES {
                    (*xp).xp_context = EXPAND_DIRS_IN_CDPATH;
                }
            }
            cmdidx_T::CMD_help => {
                (*xp).xp_context = EXPAND_HELP;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_aboveleft
            | cmdidx_T::CMD_argdo
            | cmdidx_T::CMD_belowright
            | cmdidx_T::CMD_botright
            | cmdidx_T::CMD_browse
            | cmdidx_T::CMD_bufdo
            | cmdidx_T::CMD_cdo
            | cmdidx_T::CMD_cfdo
            | cmdidx_T::CMD_confirm
            | cmdidx_T::CMD_debug
            | cmdidx_T::CMD_folddoclosed
            | cmdidx_T::CMD_folddoopen
            | cmdidx_T::CMD_hide
            | cmdidx_T::CMD_horizontal
            | cmdidx_T::CMD_keepalt
            | cmdidx_T::CMD_keepjumps
            | cmdidx_T::CMD_keepmarks
            | cmdidx_T::CMD_keeppatterns
            | cmdidx_T::CMD_ldo
            | cmdidx_T::CMD_leftabove
            | cmdidx_T::CMD_lfdo
            | cmdidx_T::CMD_lockmarks
            | cmdidx_T::CMD_noautocmd
            | cmdidx_T::CMD_noswapfile
            | cmdidx_T::CMD_restart
            | cmdidx_T::CMD_rightbelow
            | cmdidx_T::CMD_sandbox
            | cmdidx_T::CMD_silent
            | cmdidx_T::CMD_tab
            | cmdidx_T::CMD_tabdo
            | cmdidx_T::CMD_topleft
            | cmdidx_T::CMD_unsilent
            | cmdidx_T::CMD_verbose
            | cmdidx_T::CMD_vertical
            | cmdidx_T::CMD_windo => {
                return arg;
            }
            cmdidx_T::CMD_filter => {
                return set_context_in_filter_cmd(xp, arg);
            }
            cmdidx_T::CMD_match => {
                return set_context_in_match_cmd(xp, arg);
            }
            cmdidx_T::CMD_command => {
                return set_context_in_user_cmd(xp, arg);
            }
            cmdidx_T::CMD_delcommand => {
                (*xp).xp_context = EXPAND_USER_COMMANDS;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_global | cmdidx_T::CMD_vglobal => {
                let nextcmd = find_cmd_after_global_cmd(arg);
                if nextcmd.is_null() && may_expand_pattern {
                    set_context_with_pattern(xp);
                }
                return nextcmd;
            }
            cmdidx_T::CMD_and | cmdidx_T::CMD_substitute => {
                let nextcmd = find_cmd_after_substitute_cmd(arg);
                if nextcmd.is_null() && may_expand_pattern {
                    set_context_with_pattern(xp);
                }
                return nextcmd;
            }
            cmdidx_T::CMD_isearch
            | cmdidx_T::CMD_dsearch
            | cmdidx_T::CMD_ilist
            | cmdidx_T::CMD_dlist
            | cmdidx_T::CMD_ijump
            | cmdidx_T::CMD_psearch
            | cmdidx_T::CMD_djump
            | cmdidx_T::CMD_isplit
            | cmdidx_T::CMD_dsplit => {
                return find_cmd_after_isearch_cmd(xp, arg);
            }
            cmdidx_T::CMD_autocmd => {
                return aucmd_set_expand_context(xp, arg.cast_mut(), false);
            }
            cmdidx_T::CMD_doautocmd | cmdidx_T::CMD_doautoall => {
                return aucmd_set_expand_context(xp, arg.cast_mut(), true);
            }
            cmdidx_T::CMD_set => {
                set_context_in_set_cmd(xp, arg.cast_mut(), 0);
            }
            cmdidx_T::CMD_setglobal => {
                set_context_in_set_cmd(xp, arg.cast_mut(), OPT_GLOBAL);
            }
            cmdidx_T::CMD_setlocal => {
                set_context_in_set_cmd(xp, arg.cast_mut(), OPT_LOCAL);
            }
            cmdidx_T::CMD_tag
            | cmdidx_T::CMD_stag
            | cmdidx_T::CMD_ptag
            | cmdidx_T::CMD_ltag
            | cmdidx_T::CMD_tselect
            | cmdidx_T::CMD_stselect
            | cmdidx_T::CMD_ptselect
            | cmdidx_T::CMD_tjump
            | cmdidx_T::CMD_stjump
            | cmdidx_T::CMD_ptjump => {
                if (wop_flags & kOptWopFlagTagfile) != 0 {
                    (*xp).xp_context = EXPAND_TAGS_LISTFILES;
                } else {
                    (*xp).xp_context = EXPAND_TAGS;
                }
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_augroup => {
                (*xp).xp_context = EXPAND_AUGROUP;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_syntax => {
                set_context_in_syntax_cmd(xp, arg);
            }
            cmdidx_T::CMD_const
            | cmdidx_T::CMD_let
            | cmdidx_T::CMD_if
            | cmdidx_T::CMD_elseif
            | cmdidx_T::CMD_while
            | cmdidx_T::CMD_for
            | cmdidx_T::CMD_echo
            | cmdidx_T::CMD_echon
            | cmdidx_T::CMD_execute
            | cmdidx_T::CMD_echomsg
            | cmdidx_T::CMD_echoerr
            | cmdidx_T::CMD_call
            | cmdidx_T::CMD_return
            | cmdidx_T::CMD_cexpr
            | cmdidx_T::CMD_caddexpr
            | cmdidx_T::CMD_cgetexpr
            | cmdidx_T::CMD_lexpr
            | cmdidx_T::CMD_laddexpr
            | cmdidx_T::CMD_lgetexpr => {
                set_context_for_expression(xp, arg.cast_mut(), cmdidx);
            }
            cmdidx_T::CMD_unlet => {
                return set_context_in_unlet_cmd(xp, arg);
            }
            cmdidx_T::CMD_function | cmdidx_T::CMD_delfunction => {
                (*xp).xp_context = EXPAND_USER_FUNC;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_echohl => {
                set_context_in_echohl_cmd(xp, arg);
            }
            cmdidx_T::CMD_highlight => {
                set_context_in_highlight_cmd(xp, arg);
            }
            cmdidx_T::CMD_sign => {
                set_context_in_sign_cmd(xp, arg.cast_mut());
            }
            cmdidx_T::CMD_bdelete | cmdidx_T::CMD_bwipeout | cmdidx_T::CMD_bunload => {
                while {
                    (*xp).xp_pattern = c_strchr(arg, b' ' as c_char);
                    !(*xp).xp_pattern.is_null()
                } {
                    arg = (*xp).xp_pattern.add(1);
                }
                (*xp).xp_context = EXPAND_BUFFERS;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_buffer
            | cmdidx_T::CMD_sbuffer
            | cmdidx_T::CMD_pbuffer
            | cmdidx_T::CMD_checktime => {
                (*xp).xp_context = EXPAND_BUFFERS;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_diffget | cmdidx_T::CMD_diffput => {
                (*xp).xp_context = EXPAND_DIFF_BUFFERS;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_USER | cmdidx_T::CMD_USER_BUF => {
                return set_context_in_user_cmdarg(cmd, arg, argt, context, xp, forceit);
            }
            cmdidx_T::CMD_map
            | cmdidx_T::CMD_noremap
            | cmdidx_T::CMD_nmap
            | cmdidx_T::CMD_nnoremap
            | cmdidx_T::CMD_vmap
            | cmdidx_T::CMD_vnoremap
            | cmdidx_T::CMD_omap
            | cmdidx_T::CMD_onoremap
            | cmdidx_T::CMD_imap
            | cmdidx_T::CMD_inoremap
            | cmdidx_T::CMD_cmap
            | cmdidx_T::CMD_cnoremap
            | cmdidx_T::CMD_lmap
            | cmdidx_T::CMD_lnoremap
            | cmdidx_T::CMD_smap
            | cmdidx_T::CMD_snoremap
            | cmdidx_T::CMD_xmap
            | cmdidx_T::CMD_xnoremap => {
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
            cmdidx_T::CMD_unmap
            | cmdidx_T::CMD_nunmap
            | cmdidx_T::CMD_vunmap
            | cmdidx_T::CMD_ounmap
            | cmdidx_T::CMD_iunmap
            | cmdidx_T::CMD_cunmap
            | cmdidx_T::CMD_lunmap
            | cmdidx_T::CMD_sunmap
            | cmdidx_T::CMD_xunmap => {
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
            cmdidx_T::CMD_mapclear
            | cmdidx_T::CMD_nmapclear
            | cmdidx_T::CMD_vmapclear
            | cmdidx_T::CMD_omapclear
            | cmdidx_T::CMD_imapclear
            | cmdidx_T::CMD_cmapclear
            | cmdidx_T::CMD_lmapclear
            | cmdidx_T::CMD_smapclear
            | cmdidx_T::CMD_xmapclear => {
                (*xp).xp_context = EXPAND_MAPCLEAR;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_abbreviate
            | cmdidx_T::CMD_noreabbrev
            | cmdidx_T::CMD_cabbrev
            | cmdidx_T::CMD_cnoreabbrev
            | cmdidx_T::CMD_iabbrev
            | cmdidx_T::CMD_inoreabbrev => {
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
            cmdidx_T::CMD_unabbreviate | cmdidx_T::CMD_cunabbrev | cmdidx_T::CMD_iunabbrev => {
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
            cmdidx_T::CMD_menu
            | cmdidx_T::CMD_noremenu
            | cmdidx_T::CMD_unmenu
            | cmdidx_T::CMD_amenu
            | cmdidx_T::CMD_anoremenu
            | cmdidx_T::CMD_aunmenu
            | cmdidx_T::CMD_nmenu
            | cmdidx_T::CMD_nnoremenu
            | cmdidx_T::CMD_nunmenu
            | cmdidx_T::CMD_vmenu
            | cmdidx_T::CMD_vnoremenu
            | cmdidx_T::CMD_vunmenu
            | cmdidx_T::CMD_omenu
            | cmdidx_T::CMD_onoremenu
            | cmdidx_T::CMD_ounmenu
            | cmdidx_T::CMD_imenu
            | cmdidx_T::CMD_inoremenu
            | cmdidx_T::CMD_iunmenu
            | cmdidx_T::CMD_cmenu
            | cmdidx_T::CMD_cnoremenu
            | cmdidx_T::CMD_cunmenu
            | cmdidx_T::CMD_tlmenu
            | cmdidx_T::CMD_tlnoremenu
            | cmdidx_T::CMD_tlunmenu
            | cmdidx_T::CMD_tmenu
            | cmdidx_T::CMD_tunmenu
            | cmdidx_T::CMD_popup
            | cmdidx_T::CMD_emenu => {
                return set_context_in_menu_cmd(xp, cmd, arg.cast_mut(), forceit);
            }
            cmdidx_T::CMD_colorscheme => {
                (*xp).xp_context = EXPAND_COLORS;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_compiler => {
                (*xp).xp_context = EXPAND_COMPILER;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_ownsyntax => {
                (*xp).xp_context = EXPAND_OWNSYNTAX;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_setfiletype => {
                (*xp).xp_context = EXPAND_FILETYPE;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_packadd => {
                (*xp).xp_context = EXPAND_PACKADD;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_runtime => {
                set_context_in_runtime_cmd(xp, arg);
            }
            cmdidx_T::CMD_language => {
                return set_context_in_lang_cmd(xp, arg);
            }
            cmdidx_T::CMD_profile => {
                set_context_in_profile_cmd(xp, arg);
            }
            cmdidx_T::CMD_checkhealth => {
                (*xp).xp_context = EXPAND_CHECKHEALTH;
            }
            cmdidx_T::CMD_marks => {
                (*xp).xp_context = EXPAND_MARKS;
            }
            cmdidx_T::CMD_log => {
                (*xp).xp_context = EXPAND_LOG;
            }
            cmdidx_T::CMD_lsp => {
                (*xp).xp_context = EXPAND_LSP;
            }
            cmdidx_T::CMD_packdel => {
                (*xp).xp_context = EXPAND_PACKDEL;
            }
            cmdidx_T::CMD_packupdate => {
                (*xp).xp_context = EXPAND_PACKUPDATE;
            }
            cmdidx_T::CMD_retab => {
                (*xp).xp_context = EXPAND_RETAB;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_messages => {
                (*xp).xp_context = EXPAND_MESSAGES;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_history => {
                (*xp).xp_context = EXPAND_HISTORY;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_syntime => {
                (*xp).xp_context = EXPAND_SYNTIME;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_argdelete => {
                while {
                    (*xp).xp_pattern = vim_strchr(arg, b' ' as c_int);
                    !(*xp).xp_pattern.is_null()
                } {
                    arg = (*xp).xp_pattern.add(1);
                }
                (*xp).xp_context = EXPAND_ARGLIST;
                (*xp).xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_breakadd | cmdidx_T::CMD_profdel | cmdidx_T::CMD_breakdel => {
                return set_context_in_breakadd_cmd(xp, arg, cmdidx);
            }
            cmdidx_T::CMD_scriptnames => {
                return set_context_in_scriptnames_cmd(xp, arg);
            }
            cmdidx_T::CMD_filetype => {
                return set_context_in_filetype_cmd(xp, arg);
            }
            cmdidx_T::CMD_lua | cmdidx_T::CMD_equal => {
                (*xp).xp_context = EXPAND_LUA;
            }
            _ => {}
        }
        ptr::null()
    }
}

/// Parse one command from `buff` and set completion context in `xp`.
unsafe fn set_one_cmd_context(xp: *mut expand_T, buff: *const c_char) -> *const c_char {
    unsafe {
        let mut ea: exarg_T = std::mem::zeroed();
        let mut context: c_int = EXPAND_NOTHING;
        let mut forceit = false;
        let mut usefilter = false;

        ExpandInit(xp);
        (*xp).xp_pattern = buff.cast_mut();
        (*xp).xp_line = buff.cast_mut();
        (*xp).xp_context = EXPAND_COMMANDS;
        ea.argt = 0;

        let mut cmd = buff;
        while !vim_strchr(c" \t:|".as_ptr(), *cmd as u8 as c_int).is_null() {
            cmd = cmd.add(1);
        }
        (*xp).xp_pattern = cmd.cast_mut();

        if *cmd == NUL {
            return ptr::null();
        }
        if *cmd == b'"' as c_char {
            (*xp).xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        cmd = skip_range(cmd, &raw mut (*xp).xp_context);
        (*xp).xp_pattern = cmd.cast_mut();
        if *cmd == NUL {
            return ptr::null();
        }
        if *cmd == b'"' as c_char {
            (*xp).xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        if *cmd == b'|' as c_char || *cmd == b'\n' as c_char {
            return cmd.add(1);
        }

        let mut p = set_cmd_index(cmd, &raw mut ea, xp, &raw mut context);
        if p.is_null() {
            return ptr::null();
        }

        (*xp).xp_context = EXPAND_NOTHING;

        if *p == b'!' as c_char {
            forceit = true;
            p = p.add(1);
        }

        if !is_user_cmdidx(std::mem::transmute::<c_int, cmdidx_T>(ea.cmdidx)) {
            ea.argt = excmd_get_argt(std::mem::transmute::<c_int, cmdidx_T>(ea.cmdidx));
        }

        let mut arg: *const c_char = skipwhite(p);

        if (ea.argt & EX_ARGOPT) != 0 {
            while *arg != NUL && strncmp(arg, c"++".as_ptr(), 2) == 0 {
                p = arg.add(2);
                while *p != NUL && !ascii_isspace(*p as c_int) {
                    p = p.add(utfc_ptr2len(p) as usize);
                }

                if *p == NUL && (ea.argt & EX_ARGOPT) != 0 {
                    return set_context_in_argopt(xp, arg.add(2));
                }

                arg = skipwhite(p);
            }
        }

        if ea.cmdidx == cmdidx_T::CMD_write as c_int || ea.cmdidx == cmdidx_T::CMD_update as c_int {
            if *arg == b'>' as c_char {
                arg = arg.add(1);
                if *arg == b'>' as c_char {
                    arg = arg.add(1);
                }
                arg = skipwhite(arg);
            } else if *arg == b'!' as c_char && ea.cmdidx == cmdidx_T::CMD_write as c_int {
                arg = arg.add(1);
                usefilter = true;
            }
        }

        if ea.cmdidx == cmdidx_T::CMD_read as c_int {
            usefilter = forceit;
            if *arg == b'!' as c_char {
                arg = arg.add(1);
                usefilter = true;
            }
        }

        if ea.cmdidx == cmdidx_T::CMD_lshift as c_int || ea.cmdidx == cmdidx_T::CMD_rshift as c_int
        {
            while *arg == *cmd {
                arg = arg.add(1);
            }
            arg = skipwhite(arg);
        }

        if (ea.argt & EX_CMDARG) != 0 && !usefilter && *arg == b'+' as c_char {
            p = arg.add(1);
            arg = skip_cmd_arg(arg.cast_mut(), false);

            if *arg == NUL {
                return p;
            }

            arg = skipwhite(arg);
        }

        if (ea.argt & EX_TRLBAR) != 0 && !usefilter {
            p = arg;
            if ea.cmdidx == cmdidx_T::CMD_redir as c_int as c_int
                && *p == b'@' as c_char
                && *p.add(1) == b'"' as c_char
            {
                p = p.add(2);
            }
            while *p != NUL {
                if *p == Ctrl_V {
                    if *p.add(1) != NUL {
                        p = p.add(1);
                    }
                } else if ((*p == b'"' as c_char && (ea.argt & EX_NOTRLCOM) == 0)
                    || *p == b'|' as c_char
                    || *p == b'\n' as c_char)
                    && *p.sub(1) != b'\\' as c_char
                {
                    if *p == b'|' as c_char || *p == b'\n' as c_char {
                        return p.add(1);
                    }
                    return ptr::null();
                }
                p = p.add(utfc_ptr2len(p) as usize);
            }
        }

        if (ea.argt & EX_EXTRA) == 0 && *arg != NUL && c_strchr(c"|\"".as_ptr(), *arg).is_null() {
            return ptr::null();
        }

        p = buff;
        (*xp).xp_pattern = p.cast_mut();
        let len = strlen(buff);
        while *p != NUL && (p as usize) < (buff.add(len) as usize) {
            if *p == b' ' as c_char || *p == TAB {
                p = p.add(1);
                (*xp).xp_pattern = p.cast_mut();
            } else {
                if *p == b'\\' as c_char && *p.add(1) != NUL {
                    p = p.add(1);
                }
                p = p.add(utfc_ptr2len(p) as usize);
            }
        }

        if (ea.argt & EX_XFILE) != 0 {
            set_context_for_wildcard_arg(&raw mut ea, arg, usefilter, xp, &raw mut context);
        }

        set_context_by_cmdname(
            cmd,
            std::mem::transmute::<c_int, cmdidx_T>(ea.cmdidx),
            xp,
            arg,
            ea.argt,
            context,
            forceit,
        )
    }
}

/// Set the completion context in "xp" for command "str".
pub unsafe fn set_cmd_context(
    xp: *mut expand_T,
    str_arg: *mut c_char,
    len: c_int,
    col: c_int,
    use_ccline: c_int,
) {
    unsafe {
        let ccline = get_cmdline_info();
        let mut old_char: c_char = NUL;

        if col < len {
            old_char = *str_arg.add(col as usize);
            *str_arg.add(col as usize) = NUL;
        }
        let mut nextcomm: *const c_char = str_arg;

        filetype_expand_what = FiletypeExpandWhat::All;
        breakpt_expand_what = BreakptExpandWhat::Add;

        if use_ccline != 0 && (*ccline).cmdfirstc == b'=' as c_int {
            set_context_for_expression(xp, str_arg, cmdidx_T::CMD_SIZE);
        } else if use_ccline != 0 && (*ccline).input_fn != 0 {
            (*xp).xp_context = (*ccline).xp_context;
            (*xp).xp_pattern = (*ccline).cmdbuff;
            (*xp).xp_arg = (*ccline).xp_arg;
            if (*xp).xp_context == EXPAND_SHELLCMDLINE {
                let mut context = (*xp).xp_context;
                set_context_for_wildcard_arg(
                    ptr::null_mut(),
                    (*xp).xp_pattern,
                    false,
                    xp,
                    &raw mut context,
                );
            }
        } else {
            while !nextcomm.is_null() {
                nextcomm = set_one_cmd_context(xp, nextcomm);
            }
        }

        (*xp).xp_line = str_arg;
        (*xp).xp_col = col;

        if col < len {
            *str_arg.add(col as usize) = old_char;
        }
    }
}

/// Expand the command line "str" from context "xp".
pub unsafe fn expand_cmdline(
    xp: *mut expand_T,
    str_arg: *const c_char,
    col: c_int,
    matchcount: *mut c_int,
    matches: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut options = WILD_ADD_SLASH | WILD_SILENT;

        if (*xp).xp_context == EXPAND_UNSUCCESSFUL {
            beep_flush();
            return EXPAND_UNSUCCESSFUL;
        }
        if (*xp).xp_context == EXPAND_NOTHING {
            return EXPAND_NOTHING;
        }

        debug_assert!(str_arg.add(col as usize).offset_from((*xp).xp_pattern) >= 0);
        (*xp).xp_pattern_len = str_arg.add(col as usize).offset_from((*xp).xp_pattern) as usize;
        let file_str = if cmdline_fuzzy_completion_supported(xp) {
            xstrdup((*xp).xp_pattern)
        } else {
            addstar((*xp).xp_pattern, (*xp).xp_pattern_len, (*xp).xp_context)
        };

        if p_wic != 0 {
            options += WILD_ICASE;
        }

        if ExpandFromContext(xp, file_str, matches, matchcount, options) == FAIL {
            *matchcount = 0;
            *matches = ptr::null_mut();
        }
        xfree(file_str.cast());

        EXPAND_OK
    }
}

/// Expand file or directory names.
unsafe fn expand_files_and_dirs(
    xp: *mut expand_T,
    mut pat: *mut c_char,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    mut flags: c_int,
    options: c_int,
) -> c_int {
    unsafe {
        let mut free_pat = false;

        if (*xp).xp_backslash != XP_BS_NONE {
            free_pat = true;
            let pat_len = strlen(pat);
            pat = xstrnsave(pat, pat_len);

            let mut pat_end = pat.add(pat_len);
            let mut p = pat;
            while *p != NUL {
                if *p == b'\\' as c_char {
                    if ((*xp).xp_backslash & XP_BS_THREE) != 0
                        && *p.add(1) == b'\\' as c_char
                        && *p.add(2) == b'\\' as c_char
                        && *p.add(3) == b' ' as c_char
                    {
                        let from = p.add(3);
                        ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                        pat_end = pat_end.sub(3);
                    } else if ((*xp).xp_backslash & XP_BS_ONE) != 0 && *p.add(1) == b' ' as c_char {
                        let from = p.add(1);
                        ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                        pat_end = pat_end.sub(1);
                    } else if ((*xp).xp_backslash & XP_BS_COMMA) != 0 {
                        if *p.add(1) == b'\\' as c_char && *p.add(2) == b',' as c_char {
                            let from = p.add(2);
                            ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                            pat_end = pat_end.sub(2);
                        } else {
                            #[cfg(windows)]
                            if *p.add(1) == b',' as c_char {
                                let from = p.add(1);
                                ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                                pat_end = pat_end.sub(1);
                            }
                        }
                    }
                }
                p = p.add(1);
            }
        }

        let ret = if (*xp).xp_context == EXPAND_FINDFUNC {
            expand_findfunc(xp, pat, matches, numMatches)
        } else {
            if (*xp).xp_context == EXPAND_FILES {
                flags |= EW_FILE;
            } else if (*xp).xp_context == EXPAND_FILES_IN_PATH {
                flags |= EW_FILE | EW_PATH;
            } else if (*xp).xp_context == EXPAND_DIRS_IN_CDPATH {
                flags = (flags | EW_DIR | EW_CDPATH) & !EW_FILE;
            } else {
                flags = (flags | EW_DIR) & !EW_FILE;
            }
            if (options & WILD_ICASE) != 0 {
                flags |= EW_ICASE;
            }
            expand_wildcards_eval(&raw mut pat, numMatches, matches, flags)
        };
        if free_pat {
            xfree(pat.cast());
        }
        #[cfg(windows)]
        if ((options & WILD_USE_SHELLSLASH) != 0 && p_ssl == 0)
            || ((options & WILD_USE_COMPLETESLASH) != 0
                && ((*p_csl == NUL && p_ssl == 0) || *p_csl == b'b' as c_char))
        {
            for j in 0..(*numMatches as usize) {
                let mut ptr_iter = *(*matches).add(j);
                while *ptr_iter != NUL {
                    if *ptr_iter == PATHSEP as c_char {
                        *ptr_iter = b'\\' as c_char;
                    }
                    ptr_iter = ptr_iter.add(1);
                }
            }
        }
        ret
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":filetype".
unsafe fn get_filetypecmd_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 {
        return ptr::null_mut();
    }

    let i = idx as usize;
    unsafe {
        if filetype_expand_what == FiletypeExpandWhat::All && i < 5 {
            const OPTS_ALL: [&CStr; 5] = [c"detect", c"indent", c"plugin", c"on", c"off"];
            return OPTS_ALL[i].as_ptr().cast_mut();
        }
        if filetype_expand_what == FiletypeExpandWhat::Plugin && i < 3 {
            const OPTS_PLUGIN: [&CStr; 3] = [c"plugin", c"on", c"off"];
            return OPTS_PLUGIN[i].as_ptr().cast_mut();
        }
        if filetype_expand_what == FiletypeExpandWhat::Indent && i < 3 {
            const OPTS_INDENT: [&CStr; 3] = [c"indent", c"on", c"off"];
            return OPTS_INDENT[i].as_ptr().cast_mut();
        }
        if filetype_expand_what == FiletypeExpandWhat::OnOff && i < 2 {
            const OPTS_ONOFF: [&CStr; 2] = [c"on", c"off"];
            return OPTS_ONOFF[i].as_ptr().cast_mut();
        }
    }
    ptr::null_mut()
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":breakadd" / ":breakdel" / ":profdel".
unsafe fn get_breakadd_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if (0..=3).contains(&idx) {
        const OPTS: [&CStr; 4] = [c"expr", c"file", c"func", c"here"];
        let i = idx as usize;

        unsafe {
            if breakpt_expand_what == BreakptExpandWhat::Add {
                return OPTS[i].as_ptr().cast_mut();
            } else if breakpt_expand_what == BreakptExpandWhat::Del {
                if idx <= 2 {
                    return OPTS[i + 1].as_ptr().cast_mut();
                }
            } else if idx <= 1 {
                return OPTS[i + 1].as_ptr().cast_mut();
            }
        }
    }
    ptr::null_mut()
}

/// Function given to ExpandGeneric() to obtain the possible arguments for ":scriptnames".
unsafe fn get_scriptnames_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if !script_id_valid(idx + 1) {
            return ptr::null_mut();
        }

        let si = script_item(idx + 1);
        let name_buff = (&raw mut NameBuff).cast::<c_char>();
        home_replace(ptr::null(), (*si).sn_name, name_buff, MAXPATHL, true);
        name_buff
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":retab".
unsafe fn get_retab_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"-indentonly".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":messages".
unsafe fn get_messages_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"clear".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

unsafe fn get_mapclear_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"<buffer>".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Completion for |:checkhealth| command.
unsafe fn get_healthcheck_names(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let prompt_id = get_cmdline_last_prompt_id();
        if HEALTHCHECK_LAST_GEN != prompt_id || HEALTHCHECK_LAST_GEN == 0 {
            let a = Array::new();
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let code = c"return vim.health._complete()";
            let res = nlua_exec(
                NvimString {
                    data: code.as_ptr().cast_mut(),
                    size: code.to_bytes().len(),
                },
                ptr::null(),
                a,
                kRetObject,
                ptr::null_mut(),
                &raw mut err,
            );
            api_clear_error(&raw mut err);
            api_free_object(HEALTHCHECK_NAMES);
            HEALTHCHECK_NAMES = res;
            HEALTHCHECK_LAST_GEN = prompt_id;
        }

        if HEALTHCHECK_NAMES.obj_type == ObjectType::kObjectTypeArray
            && idx >= 0
            && (idx as usize) < HEALTHCHECK_NAMES.data.array.size
        {
            let item = *HEALTHCHECK_NAMES.data.array.items.add(idx as usize);
            if item.obj_type == ObjectType::kObjectTypeString {
                return item.data.string.data;
            }
        }
        ptr::null_mut()
    }
}

/// Completes arg1 via Lua.
unsafe fn get_arg1_from_lua(lua: *const c_char, xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let prompt_id = get_cmdline_last_prompt_id();
        if ARG1_LUA_LAST_XP_LINE.is_null()
            || strcmp(ARG1_LUA_LAST_XP_LINE, (*xp).xp_line) != 0
            || ARG1_LUA_LAST_GEN != prompt_id
        {
            xfree(ARG1_LUA_LAST_XP_LINE.cast());
            ARG1_LUA_LAST_XP_LINE = xstrdup((*xp).xp_line);
            let mut args_items = [
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string((*xp).xp_pattern),
                    },
                },
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData {
                        string: cstr_as_string((*xp).xp_line),
                    },
                },
            ];
            let args = Array {
                size: 2,
                capacity: 2,
                items: args_items.as_mut_ptr(),
            };
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let res = nlua_exec(
                cstr_as_string(lua),
                ptr::null(),
                args,
                kRetObject,
                ptr::null_mut(),
                &raw mut err,
            );
            api_clear_error(&raw mut err);
            api_free_object(ARG1_LUA_NAMES);
            ARG1_LUA_NAMES = res;
            ARG1_LUA_LAST_GEN = prompt_id;
        }

        if ARG1_LUA_NAMES.obj_type == ObjectType::kObjectTypeArray
            && idx >= 0
            && (idx as usize) < ARG1_LUA_NAMES.data.array.size
        {
            let item = *ARG1_LUA_NAMES.data.array.items.add(idx as usize);
            if item.obj_type == ObjectType::kObjectTypeString {
                return item.data.string.data;
            }
        }
        ptr::null_mut()
    }
}

unsafe fn get_marks_arg(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        get_arg1_from_lua(
            c"return require'vim._core.marks'.complete(...)".as_ptr(),
            xp,
            idx,
        )
    }
}

unsafe fn get_log_arg(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.log_complete(...)".as_ptr(),
            xp,
            idx,
        )
    }
}

unsafe fn get_lsp_arg(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.lsp_complete(...)".as_ptr(),
            xp,
            idx,
        )
    }
}

unsafe fn get_packdel_arg(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.packdel_complete(...)".as_ptr(),
            xp,
            idx,
        )
    }
}

unsafe fn get_packupdate_arg(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.packupdate_complete(...)".as_ptr(),
            xp,
            idx,
        )
    }
}

/// Do the expansion based on xp->xp_context and "rmp".
unsafe fn ExpandOther(
    pat: *mut c_char,
    xp: *mut expand_T,
    rmp: *mut regmatch_T,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
) -> c_int {
    struct ExpGen {
        context: c_int,
        func: unsafe fn(*mut expand_T, c_int) -> *mut c_char,
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

    unsafe {
        for entry in &tab {
            if (*xp).xp_context == entry.context {
                if entry.ic {
                    (*rmp).rm_ic = true;
                }
                ExpandGeneric(
                    pat,
                    xp,
                    rmp,
                    matches,
                    numMatches,
                    Some(entry.func),
                    entry.escaped,
                );
                return OK;
            }
        }
        FAIL
    }
}

/// Map wild expand options to flags for expand_wildcards().
fn map_wildopts_to_ewflags(options: c_int) -> c_int {
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
unsafe fn ExpandFromContext(
    xp: *mut expand_T,
    mut pat: *mut c_char,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    options: c_int,
) -> c_int {
    unsafe {
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let flags = map_wildopts_to_ewflags(options);
        let fuzzy = cmdline_fuzzy_complete(pat) && cmdline_fuzzy_completion_supported(xp);

        if (*xp).xp_context == EXPAND_FILES
            || (*xp).xp_context == EXPAND_DIRECTORIES
            || (*xp).xp_context == EXPAND_FILES_IN_PATH
            || (*xp).xp_context == EXPAND_FINDFUNC
            || (*xp).xp_context == EXPAND_DIRS_IN_CDPATH
        {
            return expand_files_and_dirs(xp, pat, matches, numMatches, flags, options);
        }

        *matches = ptr::null_mut();
        *numMatches = 0;
        if (*xp).xp_context == EXPAND_HELP {
            if find_help_tags(
                if *pat == NUL { c"help".as_ptr() } else { pat },
                numMatches,
                matches,
                false,
            ) == OK
            {
                cleanup_help_tags(*numMatches, *matches);
                return OK;
            }
            return FAIL;
        }

        if (*xp).xp_context == EXPAND_SHELLCMD {
            expand_shellcmd(pat, matches, numMatches, flags);
            return OK;
        }
        if (*xp).xp_context == EXPAND_OLD_SETTING {
            return ExpandOldSetting(numMatches, matches);
        }
        if (*xp).xp_context == EXPAND_BUFFERS {
            return ExpandBufnames(pat, numMatches, matches, options);
        }
        if (*xp).xp_context == EXPAND_DIFF_BUFFERS {
            return ExpandBufnames(pat, numMatches, matches, options | BUF_DIFF_FILTER);
        }
        if (*xp).xp_context == EXPAND_TAGS || (*xp).xp_context == EXPAND_TAGS_LISTFILES {
            return expand_tags((*xp).xp_context == EXPAND_TAGS, pat, numMatches, matches);
        }
        if (*xp).xp_context == EXPAND_COLORS {
            let mut directories: [*mut c_char; 2] =
                [c"colors".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(
                pat,
                DIP_START + DIP_OPT,
                numMatches,
                matches,
                directories.as_mut_ptr(),
            );
        }
        if (*xp).xp_context == EXPAND_COMPILER {
            let mut directories: [*mut c_char; 2] =
                [c"compiler".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, numMatches, matches, directories.as_mut_ptr());
        }
        if (*xp).xp_context == EXPAND_OWNSYNTAX {
            let mut directories: [*mut c_char; 2] =
                [c"syntax".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, numMatches, matches, directories.as_mut_ptr());
        }
        if (*xp).xp_context == EXPAND_FILETYPE {
            let mut directories: [*mut c_char; 4] = [
                c"syntax".as_ptr().cast_mut(),
                c"indent".as_ptr().cast_mut(),
                c"ftplugin".as_ptr().cast_mut(),
                ptr::null_mut(),
            ];
            return ExpandRTDir(pat, 0, numMatches, matches, directories.as_mut_ptr());
        }
        if (*xp).xp_context == EXPAND_KEYMAP {
            let mut directories: [*mut c_char; 2] =
                [c"keymap".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, numMatches, matches, directories.as_mut_ptr());
        }
        if (*xp).xp_context == EXPAND_USER_LIST {
            return ExpandUserList(xp, matches, numMatches);
        }
        if (*xp).xp_context == EXPAND_USER_LUA {
            return ExpandUserLua(xp, numMatches, matches);
        }
        if (*xp).xp_context == EXPAND_PACKADD {
            return ExpandPackAddDir(pat, numMatches, matches);
        }
        if (*xp).xp_context == EXPAND_RUNTIME {
            return expand_runtime_cmd(pat, numMatches, matches);
        }
        if (*xp).xp_context == EXPAND_PATTERN_IN_BUF {
            return expand_pattern_in_buf(pat, (*xp).xp_search_dir, matches, numMatches);
        }

        let mut tofree: *mut c_char = ptr::null_mut();
        if (*xp).xp_context == EXPAND_USER_FUNC && strncmp(pat, c"^s:".as_ptr(), 3) == 0 {
            let len = strlen(pat) + 20;
            tofree = xmalloc(len).cast::<c_char>();
            vim_snprintf!(tofree, len, c"^<SNR>\\d\\+_%s".as_ptr(), pat.add(3));
            pat = tofree;
        }

        if (*xp).xp_context == EXPAND_LUA {
            xfree(tofree.cast());
            return nlua_expand_get_matches(numMatches, matches);
        }

        if !fuzzy {
            regmatch.regprog = vim_regcomp(pat, if p_magic != 0 { RE_MAGIC } else { 0 });
            if regmatch.regprog.is_null() {
                xfree(tofree.cast());
                return FAIL;
            }
            regmatch.rm_ic = ignorecase(pat) != 0;
        }

        let ret = if (*xp).xp_context == EXPAND_SETTINGS || (*xp).xp_context == EXPAND_BOOL_SETTINGS
        {
            ExpandSettings(xp, &raw mut regmatch, pat, numMatches, matches, fuzzy)
        } else if (*xp).xp_context == EXPAND_STRING_SETTING {
            ExpandStringSetting(xp, &raw mut regmatch, numMatches, matches)
        } else if (*xp).xp_context == EXPAND_SETTING_SUBTRACT {
            ExpandSettingSubtract(xp, &raw mut regmatch, numMatches, matches)
        } else if (*xp).xp_context == EXPAND_MAPPINGS {
            ExpandMappings(pat, &raw mut regmatch, numMatches, matches)
        } else if (*xp).xp_context == EXPAND_ARGOPT {
            expand_argopt(pat, xp, &raw mut regmatch, matches, numMatches)
        } else if (*xp).xp_context == EXPAND_USER_DEFINED {
            ExpandUserDefined(pat, xp, &raw mut regmatch, matches, numMatches)
        } else {
            ExpandOther(pat, xp, &raw mut regmatch, matches, numMatches)
        };

        if !fuzzy {
            vim_regfree(regmatch.regprog);
        }
        xfree(tofree.cast());

        ret
    }
}

/// Expand a list of names.
pub unsafe fn ExpandGeneric(
    pat: *const c_char,
    xp: *mut expand_T,
    regmatch: *mut regmatch_T,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    func: CompleteListItemGetter,
    escaped: bool,
) {
    unsafe {
        let fuzzy = cmdline_fuzzy_complete(pat);
        *matches = ptr::null_mut();
        *numMatches = 0;

        let Some(func_fn) = func else {
            return;
        };

        let mut ga: garray_T = std::mem::zeroed();
        if !fuzzy {
            ga_init(&raw mut ga, std::mem::size_of::<*mut c_char>() as c_int, 30);
        } else {
            ga_init(
                &raw mut ga,
                std::mem::size_of::<fuzmatch_str_T>() as c_int,
                30,
            );
        }

        let mut i: c_int = 0;
        loop {
            let mut str_item = func_fn(xp, i);
            i += 1;
            if str_item.is_null() {
                break;
            }
            if *str_item == NUL {
                continue;
            }

            let match_ok: bool;
            let mut score: c_int = 0;
            if *(*xp).xp_pattern != NUL {
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

            str_item = if escaped {
                vim_strsave_escaped(str_item, c" \t\\.".as_ptr())
            } else {
                xstrdup(str_item)
            };

            if fuzzy {
                ga_push(
                    &raw mut ga,
                    fuzmatch_str_T {
                        idx: ga.ga_len,
                        str_ptr: str_item,
                        score,
                    },
                );
            } else {
                ga_push(&raw mut ga, str_item);
            }

            if ptr::fn_addr_eq(
                func_fn,
                get_menu_names as unsafe fn(*mut expand_T, c_int) -> *mut c_char,
            ) {
                let end = str_item.add(strlen(str_item) - 1);
                if *end == 1 {
                    *end = b'.' as c_char;
                }
            }
        }

        if ga.ga_len == 0 {
            return;
        }

        let sort_matches = !fuzzy
            && (*xp).xp_context != EXPAND_MENUNAMES
            && (*xp).xp_context != EXPAND_STRING_SETTING
            && (*xp).xp_context != EXPAND_MENUS
            && (*xp).xp_context != EXPAND_SCRIPTNAMES
            && (*xp).xp_context != EXPAND_ARGOPT;

        let funcsort = (*xp).xp_context == EXPAND_EXPRESSION
            || (*xp).xp_context == EXPAND_FUNCTIONS
            || (*xp).xp_context == EXPAND_USER_FUNC;

        if sort_matches {
            if funcsort {
                qsort(
                    ga.ga_data,
                    ga.ga_len as usize,
                    std::mem::size_of::<*mut c_char>(),
                    sort_func_compare,
                );
            } else {
                sort_strings(ga.ga_data.cast::<*mut c_char>(), ga.ga_len);
            }
        }

        if !fuzzy {
            *matches = ga.ga_data.cast::<*mut c_char>();
            *numMatches = ga.ga_len;
        } else {
            fuzzymatches_to_strmatches(
                ga.ga_data.cast::<fuzmatch_str_T>(),
                matches,
                ga.ga_len,
                funcsort,
            );
            *numMatches = ga.ga_len;
        }

        reset_expand_highlight();
    }
}

/// Expand shell command matches in one directory of $PATH.
unsafe fn expand_shellcmd_onedir(
    mut pathed_pattern: *mut c_char,
    pathlen: usize,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    flags: c_int,
    ht: *mut hashtab_T,
    gap: *mut garray_T,
) {
    unsafe {
        if expand_wildcards(1, &raw mut pathed_pattern, numMatches, matches, flags) != OK {
            return;
        }

        ga_grow(gap, *numMatches);

        for i in 0..(*numMatches as usize) {
            let mut name = *(*matches).add(i);
            let namelen = strlen(name);

            if namelen > pathlen {
                let hash: hash_T = hash_hash(name.add(pathlen));
                let hi = hash_lookup(ht, name.add(pathlen), namelen - pathlen, hash);
                if hashitem_empty(&*hi) {
                    ptr::copy(name.add(pathlen), name, namelen - pathlen + 1);
                    *((*gap).ga_data as *mut *mut c_char).add((*gap).ga_len as usize) = name;
                    (*gap).ga_len += 1;
                    hash_add_item(ht, hi, name, hash);
                    name = ptr::null_mut();
                }
            }
            xfree(name.cast());
        }
        xfree((*matches).cast());
    }
}

/// Complete a shell command.
unsafe fn expand_shellcmd(
    filepat: *mut c_char,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    flagsarg: c_int,
) {
    unsafe {
        let mut path: *mut c_char = ptr::null_mut();
        let mut ga: garray_T = std::mem::zeroed();
        let buf = xmalloc(MAXPATHL).cast::<c_char>();
        let mut flags = flagsarg;
        let mut did_curdir = false;

        let mut patlen = strlen(filepat);
        let pat = xmemdupz(filepat.cast(), patlen).cast::<c_char>();
        let mut e = pat.add(patlen);
        let mut s = pat;
        while *s != NUL {
            if *s == b'\\' as c_char {
                let p = s.add(1);
                if *p == b' ' as c_char {
                    ptr::copy(p, s, (e.offset_from(p) as usize) + 1);
                    e = e.sub(1);
                }
            }
            s = s.add(1);
        }
        patlen = e.offset_from(pat) as usize;

        flags |= EW_FILE | EW_EXEC | EW_SHELLCMD;

        let mut mustfree = false;
        if *pat == b'.' as c_char
            && (vim_ispathsep(*pat.add(1) as c_int)
                || (*pat.add(1) == b'.' as c_char && vim_ispathsep(*pat.add(2) as c_int)))
        {
            path = c".".as_ptr().cast_mut();
        } else {
            if !path_is_absolute(pat) {
                path = vim_getenv(c"PATH".as_ptr());
            }
            if path.is_null() {
                path = c"".as_ptr().cast_mut();
            } else {
                mustfree = true;
            }
        }

        ga_init(&raw mut ga, std::mem::size_of::<*mut c_char>() as c_int, 10);
        let mut found_ht: hashtab_T = std::mem::zeroed();
        hash_init(&raw mut found_ht);
        s = path;
        loop {
            let mut pathlen: usize;
            let seplen: usize;

            if *s == NUL {
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
                    e = s.add(strlen(s));
                }

                pathlen = e.offset_from(s) as usize;
                if strncmp(s, c".".as_ptr(), pathlen) == 0 {
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
                    xmemcpyz(buf.cast(), s.cast(), pathlen);
                    if seplen > 0 {
                        xmemcpyz(buf.add(pathlen).cast(), PATHSEPSTR.as_ptr().cast(), seplen);
                        pathlen += seplen;
                    }
                }
                xmemcpyz(buf.add(pathlen).cast(), pat.cast(), patlen);

                expand_shellcmd_onedir(
                    buf,
                    pathlen,
                    matches,
                    numMatches,
                    flags,
                    &raw mut found_ht,
                    &raw mut ga,
                );
            }

            if *e != NUL {
                e = e.add(1);
            }
            s = e;
        }
        *matches = ga.ga_data.cast::<*mut c_char>();
        *numMatches = ga.ga_len;

        xfree(buf.cast());
        xfree(pat.cast());
        if mustfree {
            xfree(path.cast());
        }
        hash_clear(&raw mut found_ht);
    }
}

/// Call "user_expand_func()" to invoke a user defined Vim script function.
unsafe fn call_user_expand_func(
    user_expand_func: user_expand_func_T,
    xp: *mut expand_T,
) -> *mut c_void {
    unsafe {
        let ccline = get_cmdline_info();
        let mut keep: c_char = 0;
        let mut args: [typval_T; 4] = std::mem::zeroed();
        let save_current_sctx = current_sctx;

        if (*xp).xp_arg.is_null() || *(*xp).xp_arg == NUL || (*xp).xp_line.is_null() {
            return ptr::null_mut();
        }

        if !(*ccline).cmdbuff.is_null() {
            keep = *(*ccline).cmdbuff.add((*ccline).cmdlen as usize);
            *(*ccline).cmdbuff.add((*ccline).cmdlen as usize) = 0;
        }

        let pat = xstrnsave((*xp).xp_pattern, (*xp).xp_pattern_len);
        args[0].v_type = VarType::VAR_STRING;
        args[1].v_type = VarType::VAR_STRING;
        args[2].v_type = VarType::VAR_NUMBER;
        args[3].v_type = VarType::VAR_UNKNOWN;
        args[0].vval.v_string = pat;
        args[1].vval.v_string = (*xp).xp_line;
        args[2].vval.v_number = i64::from((*xp).xp_col);

        current_sctx = (*xp).xp_script_ctx;

        let ret = user_expand_func((*xp).xp_arg, 3, args.as_mut_ptr());

        current_sctx = save_current_sctx;
        if !(*ccline).cmdbuff.is_null() {
            *(*ccline).cmdbuff.add((*ccline).cmdlen as usize) = keep;
        }

        xfree(pat.cast());
        ret
    }
}

/// Expand names with a function defined by the user (EXPAND_USER_DEFINED).
unsafe fn ExpandUserDefined(
    pat: *const c_char,
    xp: *mut expand_T,
    regmatch: *mut regmatch_T,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
) -> c_int {
    unsafe {
        let fuzzy = cmdline_fuzzy_complete(pat);
        *matches = ptr::null_mut();
        *numMatches = 0;

        let retstr = call_user_expand_func(call_func_retstr, xp).cast::<c_char>();
        if retstr.is_null() {
            return FAIL;
        }

        let mut ga: garray_T = std::mem::zeroed();
        if !fuzzy {
            ga_init(&raw mut ga, std::mem::size_of::<*mut c_char>() as c_int, 3);
        } else {
            ga_init(
                &raw mut ga,
                std::mem::size_of::<fuzmatch_str_T>() as c_int,
                3,
            );
        }

        let mut s = retstr;
        while *s != NUL {
            let mut e = vim_strchr(s, b'\n' as c_int);
            if e.is_null() {
                e = s.add(strlen(s));
            }
            let keep = *e;
            *e = NUL;

            let match_ok: bool;
            let mut score: c_int = 0;
            if *(*xp).xp_pattern != NUL {
                if !fuzzy {
                    match_ok = vim_regexec(regmatch, s, 0);
                } else {
                    score = fuzzy_match_str(s, pat);
                    match_ok = score != FUZZY_SCORE_NONE;
                }
            } else {
                match_ok = true;
            }

            *e = keep;

            if match_ok {
                let p = xmemdupz(s.cast(), e.offset_from(s) as usize).cast::<c_char>();

                if !fuzzy {
                    ga_push(&raw mut ga, p);
                } else {
                    ga_push(
                        &raw mut ga,
                        fuzmatch_str_T {
                            idx: ga.ga_len,
                            str_ptr: p,
                            score,
                        },
                    );
                }
            }

            if *e != NUL {
                e = e.add(1);
            }
            s = e;
        }
        xfree(retstr.cast());

        if ga.ga_len == 0 {
            return OK;
        }

        if !fuzzy {
            *matches = ga.ga_data.cast::<*mut c_char>();
            *numMatches = ga.ga_len;
        } else {
            fuzzymatches_to_strmatches(
                ga.ga_data.cast::<fuzmatch_str_T>(),
                matches,
                ga.ga_len,
                false,
            );
            *numMatches = ga.ga_len;
        }
        OK
    }
}

pub unsafe fn expand_process_user_list(
    retlist: *mut list_T,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
    xp: *mut expand_T,
) {
    unsafe {
        let mut ga: garray_T = std::mem::zeroed();
        let mut ga_abbr: garray_T = std::mem::zeroed();
        let mut ga_kind: garray_T = std::mem::zeroed();
        let mut ga_menu: garray_T = std::mem::zeroed();
        let mut ga_info: garray_T = std::mem::zeroed();
        let mut have_extra = false;
        let ptr_size = std::mem::size_of::<*mut c_char>() as c_int;
        ga_init(&raw mut ga, ptr_size, 3);
        ga_init(&raw mut ga_abbr, ptr_size, 3);
        ga_init(&raw mut ga_kind, ptr_size, 3);
        ga_init(&raw mut ga_menu, ptr_size, 3);
        ga_init(&raw mut ga_info, ptr_size, 3);

        if !retlist.is_null() {
            let mut li = (*retlist).lv_first;
            while !li.is_null() {
                let tv = &raw const (*li).li_tv;
                let p: *mut c_char;
                let mut abbr: *mut c_char = ptr::null_mut();
                let mut kind: *mut c_char = ptr::null_mut();
                let mut menu: *mut c_char = ptr::null_mut();
                let mut info: *mut c_char = ptr::null_mut();

                if (*tv).v_type == VarType::VAR_STRING {
                    if (*tv).vval.v_string.is_null() {
                        li = (*li).li_next;
                        continue;
                    }
                    p = xstrdup((*tv).vval.v_string);
                } else if (*tv).v_type == VarType::VAR_DICT && !(*tv).vval.v_dict.is_null() {
                    let d = (*tv).vval.v_dict;
                    let word = tv_dict_get_string(d, c"word".as_ptr(), false);
                    if word.is_null() {
                        li = (*li).li_next;
                        continue;
                    }
                    p = xstrdup(word);
                    abbr = tv_dict_get_string(d, c"abbr".as_ptr(), true);
                    kind = tv_dict_get_string(d, c"kind".as_ptr(), true);
                    menu = tv_dict_get_string(d, c"menu".as_ptr(), true);
                    info = tv_dict_get_string(d, c"info".as_ptr(), true);
                    if !abbr.is_null() || !kind.is_null() || !menu.is_null() || !info.is_null() {
                        have_extra = true;
                    }
                } else {
                    li = (*li).li_next;
                    continue;
                }

                ga_push(&raw mut ga, p);
                ga_push(&raw mut ga_abbr, abbr);
                ga_push(&raw mut ga_kind, kind);
                ga_push(&raw mut ga_menu, menu);
                ga_push(&raw mut ga_info, info);

                li = (*li).li_next;
            }
        }

        *matches = ga.ga_data.cast::<*mut c_char>();
        *numMatches = ga.ga_len;
        if have_extra && ga.ga_len > 0 {
            (*xp).xp_files_abbr = ga_abbr.ga_data.cast::<*mut c_char>();
            (*xp).xp_files_kind = ga_kind.ga_data.cast::<*mut c_char>();
            (*xp).xp_files_menu = ga_menu.ga_data.cast::<*mut c_char>();
            (*xp).xp_files_info = ga_info.ga_data.cast::<*mut c_char>();
        } else {
            for i in 0..(ga_abbr.ga_len as usize) {
                xfree((*(ga_abbr.ga_data as *mut *mut c_char).add(i)).cast());
            }
            xfree(ga_abbr.ga_data);
            for i in 0..(ga_kind.ga_len as usize) {
                xfree((*(ga_kind.ga_data as *mut *mut c_char).add(i)).cast());
            }
            xfree(ga_kind.ga_data);
            for i in 0..(ga_menu.ga_len as usize) {
                xfree((*(ga_menu.ga_data as *mut *mut c_char).add(i)).cast());
            }
            xfree(ga_menu.ga_data);
            for i in 0..(ga_info.ga_len as usize) {
                xfree((*(ga_info.ga_data as *mut *mut c_char).add(i)).cast());
            }
            xfree(ga_info.ga_data);
        }
    }
}

/// Expand names with a list returned by a function defined by the user.
unsafe fn ExpandUserList(
    xp: *mut expand_T,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
) -> c_int {
    unsafe {
        *matches = ptr::null_mut();
        *numMatches = 0;
        let retlist = call_user_expand_func(call_func_retlist, xp).cast::<list_T>();
        if retlist.is_null() {
            return FAIL;
        }

        expand_process_user_list(retlist, matches, numMatches, xp);
        tv_list_unref(retlist);
        OK
    }
}

unsafe fn ExpandUserLua(
    xp: *mut expand_T,
    numMatches: *mut c_int,
    matches: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut rettv: typval_T = std::mem::zeroed();
        nlua_call_user_expand_func(xp, &raw mut rettv);
        if rettv.v_type != VarType::VAR_LIST {
            tv_clear(&raw mut rettv);
            return FAIL;
        }

        let retlist = rettv.vval.v_list;
        expand_process_user_list(retlist, matches, numMatches, xp);
        tv_list_unref(retlist);
        OK
    }
}

/// Expand `file` for all comma-separated directories in `path`.
pub unsafe fn globpath(
    mut path: *mut c_char,
    file: *mut c_char,
    ga: *mut garray_T,
    expand_options: c_int,
    dirs: bool,
) {
    unsafe {
        let buf = xmalloc(MAXPATHL).cast::<c_char>();

        let mut xpc: expand_T = std::mem::zeroed();
        ExpandInit(&raw mut xpc);
        xpc.xp_context = if dirs {
            EXPAND_DIRECTORIES
        } else {
            EXPAND_FILES
        };

        let filelen = strlen(file);

        while *path != NUL {
            let mut pathlen = copy_option_part(&raw mut path, buf, MAXPATHL, c",".as_ptr());
            let seplen = if *buf != NUL && after_pathsep(buf, buf.add(pathlen)) == 0 {
                PATHSEPSTR.len()
            } else {
                0
            };

            if pathlen + seplen + filelen < MAXPATHL {
                if seplen > 0 {
                    xmemcpyz(buf.add(pathlen).cast(), PATHSEPSTR.as_ptr().cast(), seplen);
                    pathlen += seplen;
                }
                xmemcpyz(buf.add(pathlen).cast(), file.cast(), filelen);

                let mut p: *mut *mut c_char = ptr::null_mut();
                let mut num_p: c_int = 0;
                ExpandFromContext(
                    &raw mut xpc,
                    buf,
                    &raw mut p,
                    &raw mut num_p,
                    WILD_SILENT | expand_options,
                );
                if num_p > 0 {
                    ExpandEscape(&raw mut xpc, buf, num_p, p, WILD_SILENT | expand_options);

                    ga_grow(ga, num_p);
                    for i in 0..(num_p as usize) {
                        *((*ga).ga_data as *mut *mut c_char).add((*ga).ga_len as usize) = *p.add(i);
                        (*ga).ga_len += 1;
                    }
                    xfree(p.cast());
                }
            }
        }

        xfree(buf.cast());
    }
}

/// Translate some keys pressed when 'wildmenu' is used.
pub unsafe fn wildmenu_translate_key(
    cclp: *mut CmdlineInfo,
    key: c_int,
    xp: *mut expand_T,
    did_wild_list: bool,
) -> c_int {
    unsafe {
        let mut c = key;

        if cmdline_pum_active() || did_wild_list || wild_menu_showing != 0 {
            if c == K_LEFT {
                c = Ctrl_P;
            } else if c == K_RIGHT {
                c = Ctrl_N;
            }
        }

        if (*xp).xp_context == EXPAND_MENUNAMES
            && (*cclp).cmdpos > 1
            && *(*cclp).cmdbuff.add(((*cclp).cmdpos - 1) as usize) == b'.' as c_char
            && *(*cclp).cmdbuff.add(((*cclp).cmdpos - 2) as usize) != b'\\' as c_char
            && (c == b'\n' as c_int || c == b'\r' as c_int || c == K_KENTER)
        {
            c = K_DOWN;
        }

        c
    }
}

/// Delete characters on the command line, from "from" to the current position.
unsafe fn cmdline_del(cclp: *mut CmdlineInfo, from: c_int) {
    unsafe {
        debug_assert!((*cclp).cmdpos <= (*cclp).cmdlen);
        ptr::copy(
            (*cclp).cmdbuff.add((*cclp).cmdpos as usize),
            (*cclp).cmdbuff.add(from as usize),
            ((*cclp).cmdlen as usize) - ((*cclp).cmdpos as usize) + 1,
        );
        (*cclp).cmdlen -= (*cclp).cmdpos - from;
        (*cclp).cmdpos = from;
    }
}

/// Handle a key pressed when the wild menu for the menu names (EXPAND_MENUNAMES) is displayed.
unsafe fn wildmenu_process_key_menunames(
    cclp: *mut CmdlineInfo,
    mut key: c_int,
    xp: *mut expand_T,
) -> c_int {
    unsafe {
        if key == K_DOWN
            && (*cclp).cmdpos > 0
            && *(*cclp).cmdbuff.add(((*cclp).cmdpos - 1) as usize) == b'.' as c_char
        {
            key = p_wc as c_int;
            KeyTyped = true;
        } else if key == K_UP {
            let mut found = false;

            let mut j = (*xp).xp_pattern.offset_from((*cclp).cmdbuff) as c_int;
            let mut i: c_int = 0;
            while {
                j -= 1;
                j > 0
            } {
                if *(*cclp).cmdbuff.add(j as usize) == b' ' as c_char
                    && *(*cclp).cmdbuff.add((j - 1) as usize) != b'\\' as c_char
                {
                    i = j + 1;
                    break;
                }
                if *(*cclp).cmdbuff.add(j as usize) == b'.' as c_char
                    && *(*cclp).cmdbuff.add((j - 1) as usize) != b'\\' as c_char
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
            key = p_wc as c_int;
            KeyTyped = true;
            (*xp).xp_context = EXPAND_NOTHING;
        }

        key
    }
}

/// Handle a key pressed when the wild menu for file/dir/shellcmd names is displayed.
unsafe fn wildmenu_process_key_filenames(
    cclp: *mut CmdlineInfo,
    mut key: c_int,
    xp: *mut expand_T,
) -> c_int {
    unsafe {
        let upseg: [c_char; 5] = [
            PATHSEP as c_char,
            b'.' as c_char,
            b'.' as c_char,
            PATHSEP as c_char,
            NUL,
        ];

        if key == K_DOWN
            && (*cclp).cmdpos > 0
            && *(*cclp).cmdbuff.add(((*cclp).cmdpos - 1) as usize) == PATHSEP as c_char
            && ((*cclp).cmdpos < 3
                || *(*cclp).cmdbuff.add(((*cclp).cmdpos - 2) as usize) != b'.' as c_char
                || *(*cclp).cmdbuff.add(((*cclp).cmdpos - 3) as usize) != b'.' as c_char)
        {
            key = p_wc as c_int;
            KeyTyped = true;
        } else if strncmp((*xp).xp_pattern, upseg.as_ptr().add(1), 3) == 0 && key == K_DOWN {
            let mut found = false;

            let mut j = (*cclp).cmdpos;
            let i = (*xp).xp_pattern.offset_from((*cclp).cmdbuff) as c_int;
            while {
                j -= 1;
                j > i
            } {
                j -= utf_head_off((*cclp).cmdbuff, (*cclp).cmdbuff.add(j as usize));
                if vim_ispathsep(*(*cclp).cmdbuff.add(j as usize) as c_int) {
                    found = true;
                    break;
                }
            }
            if found
                && *(*cclp).cmdbuff.add((j - 1) as usize) == b'.' as c_char
                && *(*cclp).cmdbuff.add((j - 2) as usize) == b'.' as c_char
                && (vim_ispathsep(*(*cclp).cmdbuff.add((j - 3) as usize) as c_int) || j == i + 2)
            {
                cmdline_del(cclp, j - 2);
                key = p_wc as c_int;
                KeyTyped = true;
            }
        } else if key == K_UP {
            let mut found = false;

            let mut j = (*cclp).cmdpos - 1;
            let mut i = (*xp).xp_pattern.offset_from((*cclp).cmdbuff) as c_int;
            while {
                j -= 1;
                j > i
            } {
                j -= utf_head_off((*cclp).cmdbuff, (*cclp).cmdbuff.add(j as usize));
                #[cfg(windows)]
                let is_sep = vim_ispathsep(*(*cclp).cmdbuff.add(j as usize) as c_int)
                    && vim_strchr(
                        c" *?[{`$%#".as_ptr(),
                        *(*cclp).cmdbuff.add((j + 1) as usize) as u8 as c_int,
                    )
                    .is_null();
                #[cfg(not(windows))]
                let is_sep = vim_ispathsep(*(*cclp).cmdbuff.add(j as usize) as c_int);

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
            } else if strncmp((*cclp).cmdbuff.add(j as usize), upseg.as_ptr(), 4) == 0 {
                j += 4;
            } else if strncmp((*cclp).cmdbuff.add(j as usize), upseg.as_ptr().add(1), 3) == 0
                && j == i
            {
                j += 3;
            } else {
                j = 0;
            }

            if j > 0 {
                cmdline_del(cclp, j);
                put_on_cmdline(upseg.as_ptr().add(1), 3, false);
            } else if (*cclp).cmdpos > i {
                cmdline_del(cclp, i);
            }

            key = p_wc as c_int;
            KeyTyped = true;
        }

        key
    }
}

/// Handle a key pressed when wild menu is displayed.
pub unsafe fn wildmenu_process_key(cclp: *mut CmdlineInfo, key: c_int, xp: *mut expand_T) -> c_int {
    unsafe {
        if (*xp).xp_context == EXPAND_MENUNAMES {
            return wildmenu_process_key_menunames(cclp, key, xp);
        }
        if (*xp).xp_context == EXPAND_FILES
            || (*xp).xp_context == EXPAND_DIRECTORIES
            || (*xp).xp_context == EXPAND_SHELLCMD
        {
            return wildmenu_process_key_filenames(cclp, key, xp);
        }

        key
    }
}

/// Free expanded names when finished walking through the matches.
pub unsafe fn wildmenu_cleanup(cclp: *mut CmdlineInfo) {
    unsafe {
        if p_wmnu == 0 || wild_menu_showing == 0 {
            return;
        }

        let skt = KeyTyped;
        let old_RedrawingDisabled = RedrawingDisabled;

        if (*cclp).input_fn != 0 {
            RedrawingDisabled = 0;
        }

        set_no_hlsearch(true);

        if wild_menu_showing == WM_SCROLLED {
            cmdline_row -= 1;
            redrawcmd();
            wild_menu_showing = 0;
        } else if save_p_ls != -1 {
            p_ls = OptInt::from(save_p_ls);
            p_wmh = OptInt::from(save_p_wmh);
            last_status(false);
            update_screen();
            redrawcmd();
            save_p_ls = -1;
            wild_menu_showing = 0;
        } else {
            win_redraw_last_status(topframe);
            wild_menu_showing = 0;
            redraw_statuslines();
        }
        KeyTyped = skt;
        if (*cclp).input_fn != 0 {
            RedrawingDisabled = old_RedrawingDisabled;
        }
    }
}

/// "getcompletion()" function.
pub unsafe fn f_getcompletion(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut xpc: expand_T = std::mem::zeroed();
        let mut filtered = false;
        let mut options =
            WILD_SILENT | WILD_USE_NL | WILD_ADD_SLASH | WILD_NO_BEEP | WILD_HOME_REPLACE;

        if tv_check_for_string_arg(argvars, 1) == FAIL {
            return;
        }
        let type_str = tv_get_string(argvars.add(1));

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            filtered = tv_get_bool_chk(argvars.add(2), ptr::null_mut()) != 0;
        }

        if p_wic != 0 {
            options |= WILD_ICASE;
        }

        if !filtered {
            options |= WILD_KEEP_ALL;
        }

        if (*argvars).v_type != VarType::VAR_STRING {
            if !tv_get_string_chk(argvars).is_null() {
                emsg(gettext(e_invarg.as_ptr()));
            }
            return;
        }
        let pattern = tv_get_string(argvars);
        let mut pattern_start = pattern;

        if strcmp(type_str, c"cmdline".as_ptr()) == 0 {
            let cmdline_len = strlen(pattern) as c_int;
            set_cmd_context(
                &raw mut xpc,
                pattern.cast_mut(),
                cmdline_len,
                cmdline_len,
                false as c_int,
            );
            pattern_start = xpc.xp_pattern;
            xpc.xp_pattern_len = strlen(xpc.xp_pattern);
            xpc.xp_col = cmdline_len;
        } else {
            ExpandInit(&raw mut xpc);
            xpc.xp_pattern = pattern.cast_mut();
            xpc.xp_pattern_len = strlen(xpc.xp_pattern);
            xpc.xp_line = pattern.cast_mut();

            xpc.xp_context = cmdcomplete_str_to_type(type_str);
            match xpc.xp_context {
                EXPAND_NOTHING => {
                    semsg!(gettext(e_invarg2.as_ptr()), type_str);
                    return;
                }
                EXPAND_USER_DEFINED => {
                    if strncmp(type_str, c"custom,".as_ptr(), 7) != 0 {
                        semsg!(gettext(e_invarg2.as_ptr()), type_str);
                        return;
                    }
                    xpc.xp_arg = type_str.add(7).cast_mut();
                }
                EXPAND_USER_LIST => {
                    if strncmp(type_str, c"customlist,".as_ptr(), 11) != 0 {
                        semsg!(gettext(e_invarg2.as_ptr()), type_str);
                        return;
                    }
                    xpc.xp_arg = type_str.add(11).cast_mut();
                }
                EXPAND_MENUS => {
                    set_context_in_menu_cmd(&raw mut xpc, c"menu".as_ptr(), xpc.xp_pattern, false);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_SIGN => {
                    set_context_in_sign_cmd(&raw mut xpc, xpc.xp_pattern);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_RUNTIME => {
                    set_context_in_runtime_cmd(&raw mut xpc, xpc.xp_pattern);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_SHELLCMDLINE => {
                    let mut context = EXPAND_SHELLCMDLINE;
                    set_context_for_wildcard_arg(
                        ptr::null_mut(),
                        xpc.xp_pattern,
                        false,
                        &raw mut xpc,
                        &raw mut context,
                    );
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_FILETYPECMD => {
                    filetype_expand_what = FiletypeExpandWhat::All;
                }
                EXPAND_BREAKPOINT => {
                    breakpt_expand_what = BreakptExpandWhat::Add;
                }
                _ => {}
            }
        }

        if xpc.xp_context == EXPAND_LUA {
            xpc.xp_col = strlen(xpc.xp_line) as c_int;
            nlua_expand_pat(&raw mut xpc);
            xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
        }
        let pat = if cmdline_fuzzy_completion_supported(&raw const xpc) {
            xmemdupz(xpc.xp_pattern.cast(), xpc.xp_pattern_len).cast::<c_char>()
        } else {
            addstar(xpc.xp_pattern, xpc.xp_pattern_len, xpc.xp_context)
        };

        ExpandOne(&raw mut xpc, pat, ptr::null_mut(), options, WILD_ALL_KEEP);
        tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);

        for i in 0..(xpc.xp_numfiles as usize) {
            tv_list_append_string((*rettv).vval.v_list, *xpc.xp_files.add(i), -1);
        }
        xfree(pat.cast());
        ExpandCleanup(&raw mut xpc);
    }
}

/// "getcompletiontype()" function.
pub unsafe fn f_getcompletiontype(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if tv_check_for_string_arg(argvars, 0) == FAIL {
            return;
        }

        let pat = tv_get_string(argvars);
        let mut xpc: expand_T = std::mem::zeroed();
        ExpandInit(&raw mut xpc);

        let cmdline_len = strlen(pat) as c_int;
        set_cmd_context(
            &raw mut xpc,
            pat.cast_mut(),
            cmdline_len,
            cmdline_len,
            false as c_int,
        );
        (*rettv).vval.v_string = cmdcomplete_type_to_str(xpc.xp_context, xpc.xp_arg);

        ExpandCleanup(&raw mut xpc);
    }
}

/// "cmdcomplete_info()" function.
pub unsafe fn f_cmdcomplete_info(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let ccline = get_cmdline_info();

        tv_dict_alloc_ret(rettv);
        if ccline.is_null() || (*ccline).xpc.is_null() || (*(*ccline).xpc).xp_files.is_null() {
            return;
        }

        let retdict = (*rettv).vval.v_dict;
        let mut ret = tv_dict_add_str_len(
            retdict,
            c"cmdline_orig".as_ptr(),
            12,
            cmdline_orig.data,
            cmdline_orig.size as c_int,
        );
        if ret == OK {
            ret = tv_dict_add_nr(
                retdict,
                c"pum_visible".as_ptr(),
                11,
                i64::from(pum_visible()),
            );
        }
        if ret == OK {
            ret = tv_dict_add_nr(
                retdict,
                c"selected".as_ptr(),
                8,
                i64::from((*(*ccline).xpc).xp_selected),
            );
        }
        if ret == OK {
            let li = tv_list_alloc((*(*ccline).xpc).xp_numfiles as isize);
            ret = tv_dict_add_list(retdict, c"matches".as_ptr(), 7, li);
            let mut idx: c_int = 0;
            while ret == OK && idx < (*(*ccline).xpc).xp_numfiles {
                tv_list_append_string(li, *(*(*ccline).xpc).xp_files.add(idx as usize), -1);
                idx += 1;
            }
        }
    }
}

/// Copy a substring from the current buffer (curbuf).
unsafe fn copy_substring_from_pos(
    start: *mut pos_T,
    end: *mut pos_T,
    match_out: *mut *mut c_char,
    match_end: *mut pos_T,
) -> c_int {
    unsafe {
        let exacttext = (wop_flags & kOptWopFlagExacttext) != 0;

        if (*start).lnum > (*end).lnum
            || ((*start).lnum == (*end).lnum && (*start).col >= (*end).col)
        {
            return FAIL;
        }

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, 1, 128);

        let start_line = ml_get((*start).lnum);
        let start_ptr = start_line.add((*start).col as usize);
        let is_single_line = (*start).lnum == (*end).lnum;

        let mut segment_len = if is_single_line {
            (*end).col - (*start).col
        } else {
            ml_get_len((*start).lnum) - (*start).col
        };
        ga_grow(&raw mut ga, segment_len + 2);
        ga_concat_len(&raw mut ga, start_ptr, segment_len as usize);
        if !is_single_line {
            if exacttext {
                ga_concat_len(&raw mut ga, c"\\n".as_ptr(), 2);
            } else {
                ga_append(&raw mut ga, b'\n');
            }
        }

        if !is_single_line {
            let mut lnum = (*start).lnum + 1;
            while lnum < (*end).lnum {
                let line = ml_get(lnum);
                let linelen = ml_get_len(lnum);
                ga_grow(&raw mut ga, linelen + 2);
                ga_concat_len(&raw mut ga, line, linelen as usize);
                if exacttext {
                    ga_concat_len(&raw mut ga, c"\\n".as_ptr(), 2);
                } else {
                    ga_append(&raw mut ga, b'\n');
                }
                lnum += 1;
            }
        }

        let end_line = ml_get((*end).lnum);
        let word_end = find_word_end(end_line.add((*end).col as usize));
        segment_len = word_end.offset_from(end_line) as c_int;
        ga_grow(&raw mut ga, segment_len);
        let start_off = if is_single_line { (*end).col } else { 0 };
        ga_concat_len(
            &raw mut ga,
            end_line.add(start_off as usize),
            (segment_len - start_off) as usize,
        );

        ga_grow(&raw mut ga, 1);
        ga_append(&raw mut ga, 0);

        *match_out = ga.ga_data.cast::<c_char>();
        (*match_end).lnum = (*end).lnum;
        (*match_end).col = segment_len;

        OK
    }
}

/// Returns true if the given string `str` matches the regex pattern `pat`.
unsafe fn is_regex_match(pat: *mut c_char, str_arg: *mut c_char) -> bool {
    unsafe {
        if strcmp(pat, str_arg) == 0 {
            return true;
        }

        let mut regmatch: regmatch_T = std::mem::zeroed();

        emsg_off += 1;
        msg_silent += 1;
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING);
        emsg_off -= 1;
        msg_silent -= 1;

        if regmatch.regprog.is_null() {
            return false;
        }
        regmatch.rm_ic = p_ic != 0;
        if p_ic != 0 && p_scs != 0 {
            regmatch.rm_ic = !pat_has_uppercase(pat);
        }

        emsg_off += 1;
        msg_silent += 1;
        let result = vim_regexec_nl(&raw mut regmatch, str_arg, 0);
        emsg_off -= 1;
        msg_silent -= 1;

        vim_regfree(regmatch.regprog);
        result
    }
}

/// Constructs a new match string by appending text from the buffer to `pat`.
unsafe fn concat_pattern_with_buffer_match(
    pat: *mut c_char,
    pat_len: c_int,
    end_match_pos: *mut pos_T,
    lowercase: bool,
) -> *mut c_char {
    unsafe {
        let line = ml_get((*end_match_pos).lnum);
        let word_end = find_word_end(line.add((*end_match_pos).col as usize));
        let match_len = word_end.offset_from(line.add((*end_match_pos).col as usize)) as c_int;
        let match_str = xmalloc((match_len as usize) + (pat_len as usize) + 1).cast::<c_char>();

        ptr::copy(pat, match_str, pat_len as usize);
        if match_len > 0 {
            if lowercase {
                let mword = xstrnsave(line.add((*end_match_pos).col as usize), match_len as usize);
                let lower = strcase_save(mword, false);
                xfree(mword.cast());
                ptr::copy(lower, match_str.add(pat_len as usize), match_len as usize);
                xfree(lower.cast());
            } else {
                ptr::copy(
                    line.add((*end_match_pos).col as usize),
                    match_str.add(pat_len as usize),
                    match_len as usize,
                );
            }
        }
        *match_str.add((pat_len + match_len) as usize) = NUL;
        match_str
    }
}

/// Search for strings matching "pat" in the specified range and return them.
unsafe fn expand_pattern_in_buf(
    pat: *mut c_char,
    dir: Direction,
    matches: *mut *mut *mut c_char,
    numMatches: *mut c_int,
) -> c_int {
    unsafe {
        let exacttext = (wop_flags & kOptWopFlagExacttext) != 0;
        let has_range = Search.first_line != 0;

        *matches = ptr::null_mut();
        *numMatches = 0;

        if pat.is_null() || *pat == NUL {
            return FAIL;
        }

        let pat_len = strlen(pat) as c_int;
        let mut cur_match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut prev_match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        if has_range {
            cur_match_pos.lnum = Search.first_line;
        } else {
            cur_match_pos = pre_incsearch_pos;
        }

        let search_flags = SEARCH_OPT
            | SEARCH_NOOF
            | SEARCH_PEEK
            | SEARCH_NFMSG
            | if has_range { SEARCH_START } else { 0 };

        let mut ga: garray_T = std::mem::zeroed();
        ga_init(&raw mut ga, std::mem::size_of::<*mut c_char>() as c_int, 10);

        let mut end_match_pos: pos_T = std::mem::zeroed();
        let mut word_end_pos: pos_T = std::mem::zeroed();
        let mut looped_around = false;
        let mut compl_started = false;
        let mut full_match: *mut c_char = ptr::null_mut();

        loop {
            emsg_off += 1;
            msg_silent += 1;
            let found_new_match = searchit(
                ptr::null_mut(),
                curbuf,
                &raw mut cur_match_pos,
                &raw mut end_match_pos,
                dir,
                pat,
                pat_len as usize,
                1,
                search_flags,
                RE_LAST,
                p_magic != 0,
                ptr::null_mut(),
            );
            msg_silent -= 1;
            emsg_off -= 1;

            if found_new_match == FAIL {
                break;
            }

            if has_range
                && (cur_match_pos.lnum < Search.first_line || cur_match_pos.lnum > Search.last_line)
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

            if char_avail() || got_int {
                if got_int {
                    let _ = vpeekc();
                    got_int = false;
                }
                ga_clear_strings(&raw mut ga);
                return FAIL;
            }

            if end_match_pos.lnum > (*curbuf).b_ml.ml_line_count {
                cur_match_pos.lnum = 1;
                cur_match_pos.col = 0;
                cur_match_pos.coladd = 0;
                continue;
            }

            if copy_substring_from_pos(
                &raw mut cur_match_pos,
                &raw mut end_match_pos,
                &raw mut full_match,
                &raw mut word_end_pos,
            ) == FAIL
            {
                break;
            }

            let mut match_str = if exacttext {
                full_match
            } else {
                let mut m =
                    concat_pattern_with_buffer_match(pat, pat_len, &raw mut end_match_pos, false);

                if !is_regex_match(m, full_match) {
                    xfree(m.cast());
                    m = concat_pattern_with_buffer_match(
                        pat,
                        pat_len,
                        &raw mut end_match_pos,
                        true,
                    );
                    if !is_regex_match(m, full_match) {
                        xfree(m.cast());
                        xfree(full_match.cast());
                        continue;
                    }
                }
                xfree(full_match.cast());
                m
            };

            for i in 0..(ga.ga_len as usize) {
                if strcmp(match_str, *(ga.ga_data as *mut *mut c_char).add(i)) == 0 {
                    xfree(match_str.cast());
                    match_str = ptr::null_mut();
                    break;
                }
            }
            if !match_str.is_null() {
                ga_grow(&raw mut ga, 1);
                *(ga.ga_data as *mut *mut c_char).add(ga.ga_len as usize) = match_str;
                ga.ga_len += 1;
                if ga.ga_len > TAG_MANY {
                    break;
                }
            }
            if has_range {
                cur_match_pos = word_end_pos;
            }
        }

        *matches = ga.ga_data.cast::<*mut c_char>();
        *numMatches = ga.ga_len;
        OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_init_and_cleanup_work() {
        unsafe {
            let mut xp: expand_T = std::mem::zeroed();
            ExpandInit(&raw mut xp);
            assert_eq!(xp.xp_context, EXPAND_NOTHING);
            assert_eq!(xp.xp_backslash, XP_BS_NONE);
            ExpandCleanup(&raw mut xp);
            assert!(!cmdline_pum_active());
        }
    }
}
