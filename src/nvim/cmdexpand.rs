//! Functions for command-line completion (`src/nvim/cmdexpand.c`).

use crate::api::command::is_user_cmdidx;
use crate::api::private::helpers::{api_clear_error, api_free_object, cstr_as_string};
use crate::api::private::validate::{
    Array, Error, NvimString, Object, ObjectData, ObjectType,
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
    VSE_BUFFER, VSE_NONE, cmd_screencol, cursorcmd, escape_fname_cstr, get_cmdline_info,
    get_cmdline_last_prompt_id, hlf_T, parse_pattern_and_range, put_on_cmdline, realloc_cmdbuff,
    redrawcmd, tilde_replace, vim_strsave_fnameescape,
};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
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
    kRetObject, nlua_call_user_expand_func, nlua_exec, nlua_expand_get_matches, nlua_expand_pat,
};
use crate::mapping::{ExpandMappings, set_context_in_map_cmd};
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
    EW_NOERROR, EW_NOTFOUND, EW_PATH, EW_SHELLCMD, EW_SILENT, after_pathsep, expand_wildcards_eval,
    expand_wildcards_vec, match_suffix, path_is_absolute, path_tail,
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
use std::ffi::{CStr, CString, c_char, c_int, c_uint};
use std::ptr;

use crate::api::win_config::UIExtension;
pub type xp_prefix_T = c_int;
pub type CompleteListItemGetter = Option<fn(&mut expand_T, c_int) -> *mut c_char>;
#[derive(Clone, Copy)]
pub enum LineGetter {
    None,
    ExLine,
    CmdKeyCmd,
    AutoCmd(*mut crate::autocmd::AutoPatCmd),
    Func(*mut crate::profile::funccall_T),
    Source(*mut crate::runtime::source_cookie_T),
    Loop(*mut crate::ex_docmd::loop_cookie),
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
    pub fn as_func(self) -> Option<*mut crate::profile::funccall_T> {
        match self.inner() {
            Self::Func(p) => Some(p),
            _ => None,
        }
    }

    #[inline]
    pub fn as_source(self) -> Option<*mut crate::runtime::source_cookie_T> {
        match self.inner() {
            Self::Source(p) => Some(p),
            _ => None,
        }
    }

    pub fn call(
        self,
        c: c_int,
        indent: c_int,
        do_concat: bool,
    ) -> Option<std::ffi::CString> {
        unsafe {
            match self {
                Self::None => None,
                Self::ExLine => crate::ex_getln::getexline(c, indent, do_concat),
                Self::CmdKeyCmd => crate::input::getcmdkeycmd(c, indent, do_concat),
                Self::AutoCmd(cookie) => crate::autocmd::getnextac(c, cookie, indent, do_concat),
                Self::Func(cookie) => {
                    crate::eval::userfunc::get_func_line(c, &mut *cookie, indent, do_concat)
                }
                Self::Source(cookie) => crate::runtime::getsourceline(c, &mut *cookie, indent, do_concat),
                Self::Loop(cookie) => crate::ex_docmd::get_loop_line(c, &mut *cookie, indent, do_concat),
                Self::List(cookie) => {
                    crate::eval::funcs::get_list_line(c, &mut *cookie, indent, do_concat)
                }
                Self::LuaEx(cookie) => {
                    crate::lua::executor::nlua_ex_getline(c, &mut *cookie, indent, do_concat)
                }
            }
        }
    }
}
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
impl cmdidx_T {
    pub const ALL: [Self; 564] = [
        Self::CMD_append,
        Self::CMD_abbreviate,
        Self::CMD_abclear,
        Self::CMD_aboveleft,
        Self::CMD_all,
        Self::CMD_amenu,
        Self::CMD_anoremenu,
        Self::CMD_args,
        Self::CMD_argadd,
        Self::CMD_argdelete,
        Self::CMD_argdo,
        Self::CMD_argdedupe,
        Self::CMD_argedit,
        Self::CMD_argglobal,
        Self::CMD_arglocal,
        Self::CMD_argument,
        Self::CMD_ascii,
        Self::CMD_autocmd,
        Self::CMD_augroup,
        Self::CMD_aunmenu,
        Self::CMD_buffer,
        Self::CMD_bNext,
        Self::CMD_ball,
        Self::CMD_badd,
        Self::CMD_balt,
        Self::CMD_bcd,
        Self::CMD_bchdir,
        Self::CMD_bdelete,
        Self::CMD_belowright,
        Self::CMD_bfirst,
        Self::CMD_blast,
        Self::CMD_bmodified,
        Self::CMD_bnext,
        Self::CMD_botright,
        Self::CMD_bprevious,
        Self::CMD_brewind,
        Self::CMD_break,
        Self::CMD_breakadd,
        Self::CMD_breakdel,
        Self::CMD_breaklist,
        Self::CMD_browse,
        Self::CMD_buffers,
        Self::CMD_bufdo,
        Self::CMD_bunload,
        Self::CMD_bwipeout,
        Self::CMD_change,
        Self::CMD_cNext,
        Self::CMD_cNfile,
        Self::CMD_cabbrev,
        Self::CMD_cabclear,
        Self::CMD_cabove,
        Self::CMD_caddbuffer,
        Self::CMD_caddexpr,
        Self::CMD_caddfile,
        Self::CMD_cafter,
        Self::CMD_call,
        Self::CMD_catch,
        Self::CMD_cbuffer,
        Self::CMD_cbefore,
        Self::CMD_cbelow,
        Self::CMD_cbottom,
        Self::CMD_cc,
        Self::CMD_cclose,
        Self::CMD_cd,
        Self::CMD_cdo,
        Self::CMD_center,
        Self::CMD_cexpr,
        Self::CMD_cfile,
        Self::CMD_cfdo,
        Self::CMD_cfirst,
        Self::CMD_cgetfile,
        Self::CMD_cgetbuffer,
        Self::CMD_cgetexpr,
        Self::CMD_chdir,
        Self::CMD_changes,
        Self::CMD_checkhealth,
        Self::CMD_checkpath,
        Self::CMD_checktime,
        Self::CMD_chistory,
        Self::CMD_clist,
        Self::CMD_clast,
        Self::CMD_close,
        Self::CMD_clearjumps,
        Self::CMD_cmap,
        Self::CMD_cmapclear,
        Self::CMD_cmenu,
        Self::CMD_cnext,
        Self::CMD_cnewer,
        Self::CMD_cnfile,
        Self::CMD_cnoremap,
        Self::CMD_cnoreabbrev,
        Self::CMD_cnoremenu,
        Self::CMD_copy,
        Self::CMD_colder,
        Self::CMD_colorscheme,
        Self::CMD_command,
        Self::CMD_comclear,
        Self::CMD_compiler,
        Self::CMD_continue,
        Self::CMD_confirm,
        Self::CMD_connect,
        Self::CMD_const,
        Self::CMD_copen,
        Self::CMD_cprevious,
        Self::CMD_cpfile,
        Self::CMD_cquit,
        Self::CMD_crewind,
        Self::CMD_cunmap,
        Self::CMD_cunabbrev,
        Self::CMD_cunmenu,
        Self::CMD_cwindow,
        Self::CMD_delete,
        Self::CMD_delmarks,
        Self::CMD_debug,
        Self::CMD_debuggreedy,
        Self::CMD_defer,
        Self::CMD_delcommand,
        Self::CMD_delfunction,
        Self::CMD_detach,
        Self::CMD_display,
        Self::CMD_diffupdate,
        Self::CMD_diffget,
        Self::CMD_diffoff,
        Self::CMD_diffpatch,
        Self::CMD_diffput,
        Self::CMD_diffsplit,
        Self::CMD_diffthis,
        Self::CMD_digraphs,
        Self::CMD_djump,
        Self::CMD_dlist,
        Self::CMD_doautocmd,
        Self::CMD_doautoall,
        Self::CMD_drop,
        Self::CMD_dsearch,
        Self::CMD_dsplit,
        Self::CMD_edit,
        Self::CMD_earlier,
        Self::CMD_echo,
        Self::CMD_echoerr,
        Self::CMD_echohl,
        Self::CMD_echomsg,
        Self::CMD_echon,
        Self::CMD_else,
        Self::CMD_elseif,
        Self::CMD_emenu,
        Self::CMD_endif,
        Self::CMD_endfunction,
        Self::CMD_endfor,
        Self::CMD_endtry,
        Self::CMD_endwhile,
        Self::CMD_enew,
        Self::CMD_eval,
        Self::CMD_ex,
        Self::CMD_execute,
        Self::CMD_exit,
        Self::CMD_exmode,
        Self::CMD_exusage,
        Self::CMD_file,
        Self::CMD_files,
        Self::CMD_filetype,
        Self::CMD_filter,
        Self::CMD_find,
        Self::CMD_finally,
        Self::CMD_finish,
        Self::CMD_first,
        Self::CMD_fold,
        Self::CMD_foldclose,
        Self::CMD_folddoopen,
        Self::CMD_folddoclosed,
        Self::CMD_foldopen,
        Self::CMD_for,
        Self::CMD_function,
        Self::CMD_fclose,
        Self::CMD_global,
        Self::CMD_goto,
        Self::CMD_grep,
        Self::CMD_grepadd,
        Self::CMD_gui,
        Self::CMD_gvim,
        Self::CMD_help,
        Self::CMD_helpclose,
        Self::CMD_helpgrep,
        Self::CMD_helptags,
        Self::CMD_highlight,
        Self::CMD_hide,
        Self::CMD_history,
        Self::CMD_horizontal,
        Self::CMD_insert,
        Self::CMD_iabbrev,
        Self::CMD_iabclear,
        Self::CMD_if,
        Self::CMD_ijump,
        Self::CMD_ilist,
        Self::CMD_imap,
        Self::CMD_imapclear,
        Self::CMD_imenu,
        Self::CMD_inoremap,
        Self::CMD_inoreabbrev,
        Self::CMD_inoremenu,
        Self::CMD_intro,
        Self::CMD_iput,
        Self::CMD_isearch,
        Self::CMD_isplit,
        Self::CMD_iunmap,
        Self::CMD_iunabbrev,
        Self::CMD_iunmenu,
        Self::CMD_join,
        Self::CMD_jumps,
        Self::CMD_k,
        Self::CMD_keepmarks,
        Self::CMD_keepjumps,
        Self::CMD_keeppatterns,
        Self::CMD_keepalt,
        Self::CMD_list,
        Self::CMD_lNext,
        Self::CMD_lNfile,
        Self::CMD_last,
        Self::CMD_labove,
        Self::CMD_language,
        Self::CMD_laddexpr,
        Self::CMD_laddbuffer,
        Self::CMD_laddfile,
        Self::CMD_lafter,
        Self::CMD_later,
        Self::CMD_lbuffer,
        Self::CMD_lbefore,
        Self::CMD_lbelow,
        Self::CMD_lbottom,
        Self::CMD_lcd,
        Self::CMD_lchdir,
        Self::CMD_lclose,
        Self::CMD_ldo,
        Self::CMD_left,
        Self::CMD_leftabove,
        Self::CMD_let,
        Self::CMD_lexpr,
        Self::CMD_lfile,
        Self::CMD_lfdo,
        Self::CMD_lfirst,
        Self::CMD_lgetfile,
        Self::CMD_lgetbuffer,
        Self::CMD_lgetexpr,
        Self::CMD_lgrep,
        Self::CMD_lgrepadd,
        Self::CMD_lhelpgrep,
        Self::CMD_lhistory,
        Self::CMD_ll,
        Self::CMD_llast,
        Self::CMD_llist,
        Self::CMD_lmap,
        Self::CMD_lmapclear,
        Self::CMD_lmake,
        Self::CMD_lnoremap,
        Self::CMD_lnext,
        Self::CMD_lnewer,
        Self::CMD_lnfile,
        Self::CMD_loadview,
        Self::CMD_loadkeymap,
        Self::CMD_lockmarks,
        Self::CMD_lockvar,
        Self::CMD_log,
        Self::CMD_lolder,
        Self::CMD_lopen,
        Self::CMD_lprevious,
        Self::CMD_lpfile,
        Self::CMD_lrewind,
        Self::CMD_ltag,
        Self::CMD_lunmap,
        Self::CMD_lua,
        Self::CMD_luado,
        Self::CMD_luafile,
        Self::CMD_lvimgrep,
        Self::CMD_lvimgrepadd,
        Self::CMD_lwindow,
        Self::CMD_ls,
        Self::CMD_lsp,
        Self::CMD_move,
        Self::CMD_mark,
        Self::CMD_make,
        Self::CMD_map,
        Self::CMD_mapclear,
        Self::CMD_marks,
        Self::CMD_match,
        Self::CMD_menu,
        Self::CMD_menutranslate,
        Self::CMD_messages,
        Self::CMD_mkexrc,
        Self::CMD_mksession,
        Self::CMD_mkspell,
        Self::CMD_mkvimrc,
        Self::CMD_mkview,
        Self::CMD_mode,
        Self::CMD_mzscheme,
        Self::CMD_mzfile,
        Self::CMD_next,
        Self::CMD_new,
        Self::CMD_nmap,
        Self::CMD_nmapclear,
        Self::CMD_nmenu,
        Self::CMD_nnoremap,
        Self::CMD_nnoremenu,
        Self::CMD_noremap,
        Self::CMD_noautocmd,
        Self::CMD_nohlsearch,
        Self::CMD_noreabbrev,
        Self::CMD_noremenu,
        Self::CMD_noswapfile,
        Self::CMD_normal,
        Self::CMD_number,
        Self::CMD_nunmap,
        Self::CMD_nunmenu,
        Self::CMD_oldfiles,
        Self::CMD_omap,
        Self::CMD_omapclear,
        Self::CMD_omenu,
        Self::CMD_only,
        Self::CMD_onoremap,
        Self::CMD_onoremenu,
        Self::CMD_options,
        Self::CMD_ounmap,
        Self::CMD_ounmenu,
        Self::CMD_ownsyntax,
        Self::CMD_print,
        Self::CMD_packadd,
        Self::CMD_packdel,
        Self::CMD_packloadall,
        Self::CMD_packupdate,
        Self::CMD_pbuffer,
        Self::CMD_pclose,
        Self::CMD_perl,
        Self::CMD_perldo,
        Self::CMD_perlfile,
        Self::CMD_pedit,
        Self::CMD_pop,
        Self::CMD_popup,
        Self::CMD_ppop,
        Self::CMD_preserve,
        Self::CMD_previous,
        Self::CMD_profile,
        Self::CMD_profdel,
        Self::CMD_psearch,
        Self::CMD_ptag,
        Self::CMD_ptNext,
        Self::CMD_ptfirst,
        Self::CMD_ptjump,
        Self::CMD_ptlast,
        Self::CMD_ptnext,
        Self::CMD_ptprevious,
        Self::CMD_ptrewind,
        Self::CMD_ptselect,
        Self::CMD_put,
        Self::CMD_pwd,
        Self::CMD_python,
        Self::CMD_pydo,
        Self::CMD_pyfile,
        Self::CMD_py3,
        Self::CMD_py3do,
        Self::CMD_python3,
        Self::CMD_py3file,
        Self::CMD_pyx,
        Self::CMD_pyxdo,
        Self::CMD_pythonx,
        Self::CMD_pyxfile,
        Self::CMD_quit,
        Self::CMD_quitall,
        Self::CMD_qall,
        Self::CMD_read,
        Self::CMD_recover,
        Self::CMD_redo,
        Self::CMD_redir,
        Self::CMD_redraw,
        Self::CMD_redrawstatus,
        Self::CMD_redrawtabline,
        Self::CMD_registers,
        Self::CMD_resize,
        Self::CMD_restart,
        Self::CMD_retab,
        Self::CMD_return,
        Self::CMD_rewind,
        Self::CMD_right,
        Self::CMD_rightbelow,
        Self::CMD_rshada,
        Self::CMD_runtime,
        Self::CMD_rundo,
        Self::CMD_ruby,
        Self::CMD_rubydo,
        Self::CMD_rubyfile,
        Self::CMD_rviminfo,
        Self::CMD_substitute,
        Self::CMD_sNext,
        Self::CMD_sargument,
        Self::CMD_sall,
        Self::CMD_sandbox,
        Self::CMD_saveas,
        Self::CMD_sbuffer,
        Self::CMD_sbNext,
        Self::CMD_sball,
        Self::CMD_sbfirst,
        Self::CMD_sblast,
        Self::CMD_sbmodified,
        Self::CMD_sbnext,
        Self::CMD_sbprevious,
        Self::CMD_sbrewind,
        Self::CMD_scriptnames,
        Self::CMD_scriptencoding,
        Self::CMD_set,
        Self::CMD_setfiletype,
        Self::CMD_setglobal,
        Self::CMD_setlocal,
        Self::CMD_sfind,
        Self::CMD_sfirst,
        Self::CMD_simalt,
        Self::CMD_sign,
        Self::CMD_silent,
        Self::CMD_sleep,
        Self::CMD_slast,
        Self::CMD_smagic,
        Self::CMD_smap,
        Self::CMD_smapclear,
        Self::CMD_smenu,
        Self::CMD_snext,
        Self::CMD_snomagic,
        Self::CMD_snoremap,
        Self::CMD_snoremenu,
        Self::CMD_source,
        Self::CMD_sort,
        Self::CMD_split,
        Self::CMD_spellgood,
        Self::CMD_spelldump,
        Self::CMD_spellinfo,
        Self::CMD_spellrepall,
        Self::CMD_spellrare,
        Self::CMD_spellundo,
        Self::CMD_spellwrong,
        Self::CMD_sprevious,
        Self::CMD_srewind,
        Self::CMD_stop,
        Self::CMD_stag,
        Self::CMD_startinsert,
        Self::CMD_startgreplace,
        Self::CMD_startreplace,
        Self::CMD_stopinsert,
        Self::CMD_stjump,
        Self::CMD_stselect,
        Self::CMD_sunhide,
        Self::CMD_sunmap,
        Self::CMD_sunmenu,
        Self::CMD_suspend,
        Self::CMD_sview,
        Self::CMD_swapname,
        Self::CMD_syntax,
        Self::CMD_syntime,
        Self::CMD_syncbind,
        Self::CMD_t,
        Self::CMD_tcd,
        Self::CMD_tchdir,
        Self::CMD_tNext,
        Self::CMD_tag,
        Self::CMD_tags,
        Self::CMD_tab,
        Self::CMD_tabclose,
        Self::CMD_tabdo,
        Self::CMD_tabedit,
        Self::CMD_tabfind,
        Self::CMD_tabfirst,
        Self::CMD_tabmove,
        Self::CMD_tablast,
        Self::CMD_tabnext,
        Self::CMD_tabnew,
        Self::CMD_tabonly,
        Self::CMD_tabprevious,
        Self::CMD_tabNext,
        Self::CMD_tabrewind,
        Self::CMD_tabs,
        Self::CMD_tcl,
        Self::CMD_tcldo,
        Self::CMD_tclfile,
        Self::CMD_terminal,
        Self::CMD_tfirst,
        Self::CMD_throw,
        Self::CMD_tjump,
        Self::CMD_tlast,
        Self::CMD_tlmenu,
        Self::CMD_tlnoremenu,
        Self::CMD_tlunmenu,
        Self::CMD_tmenu,
        Self::CMD_tmap,
        Self::CMD_tmapclear,
        Self::CMD_tnext,
        Self::CMD_tnoremap,
        Self::CMD_topleft,
        Self::CMD_tprevious,
        Self::CMD_trewind,
        Self::CMD_trust,
        Self::CMD_try,
        Self::CMD_tselect,
        Self::CMD_tunmenu,
        Self::CMD_tunmap,
        Self::CMD_undo,
        Self::CMD_undojoin,
        Self::CMD_undolist,
        Self::CMD_unabbreviate,
        Self::CMD_unhide,
        Self::CMD_uniq,
        Self::CMD_unlet,
        Self::CMD_unlockvar,
        Self::CMD_unmap,
        Self::CMD_unmenu,
        Self::CMD_unsilent,
        Self::CMD_update,
        Self::CMD_uptime,
        Self::CMD_vglobal,
        Self::CMD_version,
        Self::CMD_verbose,
        Self::CMD_vertical,
        Self::CMD_visual,
        Self::CMD_view,
        Self::CMD_vimgrep,
        Self::CMD_vimgrepadd,
        Self::CMD_viusage,
        Self::CMD_vmap,
        Self::CMD_vmapclear,
        Self::CMD_vmenu,
        Self::CMD_vnoremap,
        Self::CMD_vnew,
        Self::CMD_vnoremenu,
        Self::CMD_vsplit,
        Self::CMD_vunmap,
        Self::CMD_vunmenu,
        Self::CMD_write,
        Self::CMD_wNext,
        Self::CMD_wall,
        Self::CMD_while,
        Self::CMD_winsize,
        Self::CMD_wincmd,
        Self::CMD_windo,
        Self::CMD_winpos,
        Self::CMD_wnext,
        Self::CMD_wprevious,
        Self::CMD_wq,
        Self::CMD_wqall,
        Self::CMD_wshada,
        Self::CMD_wundo,
        Self::CMD_wviminfo,
        Self::CMD_xit,
        Self::CMD_xall,
        Self::CMD_xmap,
        Self::CMD_xmapclear,
        Self::CMD_xmenu,
        Self::CMD_xnoremap,
        Self::CMD_xnoremenu,
        Self::CMD_xunmap,
        Self::CMD_xunmenu,
        Self::CMD_yank,
        Self::CMD_z,
        Self::CMD_bang,
        Self::CMD_pound,
        Self::CMD_and,
        Self::CMD_lshift,
        Self::CMD_equal,
        Self::CMD_rshift,
        Self::CMD_at,
        Self::CMD_tilde,
        Self::CMD_Next,
    ];

    #[inline]
    pub const fn from_raw(idx: c_int) -> Self {
        match idx {
            -1 => Self::CMD_USER,
            -2 => Self::CMD_USER_BUF,
            i if i >= 0 && (i as usize) < Self::ALL.len() => Self::ALL[i as usize],
            _ => Self::CMD_SIZE,
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
    pub start: c_int,
    pub end: c_int,
    pub hl_id: c_int,
}

pub type CmdlineColors = Vec<CmdlineColorChunk>;

#[repr(C)]
#[derive(Clone)]
pub struct ColoredCmdline {
    pub prompt_id: c_uint,
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
    pub cmdbufflen: c_int,
    pub cmdlen: c_int,
    pub cmdpos: c_int,
    pub cmdspos: c_int,
    pub cmdfirstc: c_int,
    pub cmdindent: c_int,
    pub cmdprompt: Option<CString>,
    pub hl_id: c_int,
    pub overstrike: c_int,
    pub xpc: *mut expand_T,
    pub xp_context: c_int,
    pub xp_arg: Option<CString>,
    pub input_fn: c_int,
    pub cmdbuff_replaced: bool,
    pub prompt_id: c_uint,
    pub highlight_callback: Callback,
    pub last_colors: ColoredCmdline,
    pub level: c_int,
    pub prev_ccline: Option<Box<CmdlineInfo>>,
    pub special_char: c_char,
    pub special_shift: bool,
    pub redraw_state: c_int,
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

pub(crate) const Ctrl_N: c_int = 14;
pub(crate) const Ctrl_P: c_int = 16;

pub(crate) const MB_MAXBYTES: usize = 30;

#[cfg(windows)]
const ENV_SEPCHAR: c_int = b';' as c_int;
#[cfg(not(windows))]
const ENV_SEPCHAR: c_int = b':' as c_int;

const PATHSEPSTR: &[u8] = b"/";

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

struct CmdexpandState {
    cmd_showtail: bool,
    may_expand_pattern: bool,
    pre_incsearch_pos: pos_T,
    compl_match_array: Vec<pumitem_T>,
    compl_startcol: c_int,
    compl_selected: c_int,
    cmdline_orig: Option<Vec<u8>>,
    filetype_expand_what: FiletypeExpandWhat,
    breakpt_expand_what: BreakptExpandWhat,
    HEALTHCHECK_NAMES: Object,
    HEALTHCHECK_LAST_GEN: c_uint,
    ARG1_LUA_NAMES: Object,
    ARG1_LUA_LAST_XP_LINE: Option<CString>,
    ARG1_LUA_LAST_GEN: c_uint,
    first_match: c_int,
}

impl CmdexpandState {
    const fn new() -> Self {
        Self {
            cmd_showtail: false,
            may_expand_pattern: false,
            pre_incsearch_pos: pos_T {
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
            HEALTHCHECK_NAMES: Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData::Integer(0),
            },
            HEALTHCHECK_LAST_GEN: 0,
            ARG1_LUA_NAMES: Object {
                obj_type: ObjectType::kObjectTypeNil,
                data: ObjectData::Integer(0),
            },
            ARG1_LUA_LAST_XP_LINE: None,
            ARG1_LUA_LAST_GEN: 0,
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
fn c_strchr(mut s: *const c_char, c: c_char) -> *mut c_char {
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
fn show_match(matches: &[CString], m: usize, showtail: bool) -> *mut c_char {
    let item = matches[m].as_ptr().cast_mut();
        if showtail {
            showmatches_gettail(item, false)
        } else {
            item
        }
}

/// Returns true if fuzzy completion is supported for a given cmdline completion context.
fn cmdline_fuzzy_completion_supported(xp: &expand_T) -> bool {
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
            _ => (opt_vars().wop_flags & kOptWopFlagFuzzy) != 0,
        }
}

/// Returns true if fuzzy completion for cmdline completion is enabled and
/// "fuzzystr" is not empty.
pub fn cmdline_fuzzy_complete(fuzzystr: *const c_char) -> bool {
    unsafe { (opt_vars().wop_flags & kOptWopFlagFuzzy) != 0 && *fuzzystr != NUL }
}

/// Sort function for the completion matches.
/// <SNR> functions should be sorted to the end.
fn sort_func_compare(p1: &CString, p2: &CString) -> c_int {
    let b1 = p1.as_bytes().first().copied().unwrap_or(0);
    let b2 = p2.as_bytes().first().copied().unwrap_or(0);
    if b1 != b'<' && b2 == b'<' {
        return -1;
    }
    if b1 == b'<' && b2 != b'<' {
        return 1;
    }
     crate::memory::cstr_as_bytes(p1.as_ptr()).cmp(crate::memory::cstr_as_bytes(p2.as_ptr())) as c_int
}

/// Escape special characters in the cmdline completion matches.
fn wildescape(xp: &mut expand_T, str_arg: *const c_char, files: &mut [CString]) {
    unsafe {
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
                    *file_slot = vim_strsave_escaped(std::ffi::CStr::from_ptr(file_slot.as_ptr()), std::ffi::CStr::from_ptr(pat));
                    #[cfg(windows)]
                    {
                        *file_slot = vim_strsave_escaped(std::ffi::CStr::from_ptr(file_slot.as_ptr()), c" ");
                    }
                } else if (xp.xp_backslash & XP_BS_COMMA) != 0
                    && file_slot.as_bytes().contains(&b',')
                {
                    *file_slot = vim_strsave_escaped(std::ffi::CStr::from_ptr(file_slot.as_ptr()), c",");
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

                if *str_arg == b'\\' as c_char
                    && *str_arg.add(1) == b'~' as c_char
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
                *file_slot = vim_strsave_escaped(std::ffi::CStr::from_ptr(file_slot.as_ptr()), c"\\|\"");
            }
        }
    }
}

/// Escape special characters in the cmdline completion matches.
fn ExpandEscape(
    xp: &mut expand_T,
    str_arg: *mut c_char,
    files: &mut [CString],
    options: c_int,
) {
    if (options & WILD_HOME_REPLACE) != 0 && !files.is_empty() {
            tilde_replace(str_arg, files);
        }

        if (options & WILD_ESCAPE) != 0 {
            wildescape(xp, str_arg, files);
        }
}

/// Return FAIL if this is not an appropriate context in which to do
/// completion of anything, return OK if it is (even if there are no matches).
pub fn nextwild(xp: &mut expand_T, type_arg: c_int, options: c_int, escape: bool) -> bool {
    unsafe {
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
            if (*ccline).input_fn != 0 && (*ccline).xp_context == EXPAND_COMMANDS {
                set_cmd_context(
                    xp,
                    (*ccline).cmdbuff,
                    (*ccline).cmdlen,
                    (*ccline).cmdpos,
                    false as c_int,
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

        let i = xp.xp_pattern.offset_from((*ccline).cmdbuff) as c_int;
        debug_assert!((*ccline).cmdpos >= i);
        xp.xp_pattern_len = ((*ccline).cmdpos as usize) - (i as usize);

        if from_wildtrigger_func && xp.xp_context == EXPAND_COMMANDS && xp.xp_pattern_len == 0
        {
            return false;
        }

        if !g_st().cmd_silent
            && !from_wildtrigger_func
            && !wild_navigate
            && !(ui_has(kUICmdline) || ui_has(kUIWildmenu))
        {
            msg_puts(c"...".as_ptr());
            ui_flush();
        }

        if wild_navigate {
            p = ExpandOne(xp, ptr::null_mut(), None, 0, type_arg);
        } else {
            let tmp = if cmdline_fuzzy_completion_supported(xp)
                || xp.xp_context == EXPAND_PATTERN_IN_BUF
            {
                cstr_from_bytes(std::slice::from_raw_parts(
                    xp.xp_pattern.cast::<u8>(),
                    xp.xp_pattern_len,
                ))
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
            let orig = cstr_from_bytes(std::slice::from_raw_parts(
                (*ccline).cmdbuff.add(i as usize).cast::<u8>(),
                xp.xp_pattern_len,
            ));
            p = ExpandOne(
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
                    let c = *(*ccline).cmdbuff.add((i as usize) + j);
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

        if !wild_navigate && !(*ccline).cmdbuff.is_null() {
            let slice =
                std::slice::from_raw_parts((*ccline).cmdbuff.cast::<u8>(), (*ccline).cmdlen as usize);
            let len = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            cmdexp().cmdline_orig = Some(slice[..len].to_vec());
        }

        if let Some(ref p_str) = p
            && !g_st().got_int
            && (options & (WILD_NOSELECT | WILD_NOINSERT)) == 0
        {
            let plen = p_str.to_bytes().len();
            let difflen = (plen as c_int) - (xp.xp_pattern_len as c_int);
            if (*ccline).cmdlen + difflen + 4 > (*ccline).cmdbufflen {
                realloc_cmdbuff((*ccline).cmdlen + difflen + 4);
                xp.xp_pattern = (*ccline).cmdbuff.add(i as usize);
            }
            debug_assert!((*ccline).cmdpos <= (*ccline).cmdlen);
            ptr::copy(
                (*ccline).cmdbuff.add((*ccline).cmdpos as usize),
                (*ccline).cmdbuff.add(((*ccline).cmdpos + difflen) as usize),
                ((*ccline).cmdlen as usize) - ((*ccline).cmdpos as usize) + 1,
            );
            ptr::copy(p_str.as_ptr(), (*ccline).cmdbuff.add(i as usize), plen);
            (*ccline).cmdlen += difflen;
            (*ccline).cmdpos += difflen;
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
            ExpandOne(xp, ptr::null_mut(), None, 0, WILD_FREE);
        }

        true
    }
}

/// Create completion popup menu with items from "matches".
fn cmdline_pum_create(
    ccline: &CmdlineInfo,
    xp: &expand_T,
    matches: &[CString],
    showtail: bool,
    cmdline_unchanged: bool,
) {
    unsafe {
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
            items.push(pumitem_T {
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
        if ui_has(kUICmdline) && g_st().cmdline_win.is_null() {
            cmdexp().compl_startcol = endpos.offset_from(ccline.cmdbuff) as c_int;
        } else {
            cmdexp().compl_startcol = cmd_screencol(endpos.offset_from(ccline.cmdbuff) as c_int);
        }
    }
}

pub fn cmdline_pum_display(changed_array: bool) {
    unsafe {
        let arr = &raw mut cmdexp().compl_match_array;
        pum_display(
            (*arr).as_mut_ptr(),
            (*arr).len() as c_int,
            cmdexp().compl_selected,
            changed_array,
            cmdexp().compl_startcol,
        );
    }
}

/// Returns true if the cmdline completion popup menu is being displayed.
pub fn cmdline_pum_active() -> bool {
    unsafe {
        let arr = &raw const cmdexp().compl_match_array;
        pum_visible() && !(*arr).is_empty()
    }
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
pub fn cmdline_compl_pattern() -> *mut c_char {
    unsafe {
        let xp = (*get_cmdline_info()).xpc;
        if false {
            ptr::null_mut()
        } else {
            (*xp)
                .xp_orig
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut())
        }
    }
}

/// Returns true if fuzzy cmdline completion is active, false otherwise.
pub fn cmdline_compl_is_fuzzy() -> bool {
    unsafe {
        let xp = (*get_cmdline_info()).xpc;
        !xp.is_null() && cmdline_fuzzy_completion_supported(&*xp)
    }
}

/// Checks whether popup menu should be used for cmdline completion wildmenu.
fn cmdline_compl_use_pum(need_wildmenu: bool) -> bool {
    (need_wildmenu
            && (opt_vars().wop_flags & kOptWopFlagPum) != 0
            && !(ui_has(kUICmdline) && g_st().cmdline_win.is_null()))
            || ui_has(kUIWildmenu)
            || (ui_has(kUICmdline) && ui_has(kUIPopupmenu))
}

/// Return the number of characters that should be skipped in the wildmenu.
fn skip_wildmenu_char(xp: &expand_T, s: *mut c_char) -> c_int {
    unsafe {
        if (rem_backslash(s)
            && xp.xp_context != EXPAND_HELP
            && xp.xp_context != EXPAND_PATTERN_IN_BUF)
            || ((xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES)
                && (*s == b'\t' as c_char || (*s == b'\\' as c_char && *s.add(1) != NUL)))
        {
            #[cfg(not(windows))]
            if xp.xp_shell
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
fn wildmenu_match_len(xp: &expand_T, mut s: *mut c_char) -> c_int {
    unsafe {
        let mut len: c_int = 0;
        let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;

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
fn redraw_wildmenu(
    xp: &expand_T,
    matches: &[CString],
    mut match_idx: c_int,
    showtail: bool,
) {
    unsafe {
        let mut highlight = true;
        let mut selstart: *mut c_char = ptr::null_mut();
        let mut selstart_col: c_int = 0;
        let mut selend: *mut c_char = ptr::null_mut();
        let mut add_left = false;

        if matches.is_empty() {
            return;
        }
        let num_matches = matches.len() as c_int;

        let mut buf_vec = vec![0 as c_char; (g_st().Columns as usize) * MB_MAXBYTES + 1];
        let buf = buf_vec.as_mut_ptr();

        if match_idx == -1 {
            match_idx = 0;
            highlight = false;
        }
        let mut clen =
            wildmenu_match_len(xp, show_match(matches, match_idx as usize, showtail)) + 3;
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
            if clen > g_st().Columns {
                cmdexp().first_match = match_idx;
                clen = 2;
                let mut i = match_idx;
                while i < num_matches {
                    clen += wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2;
                    if clen >= g_st().Columns {
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
                if clen >= g_st().Columns {
                    break;
                }
                cmdexp().first_match -= 1;
            }
        }

        let mut group: hlf_T = 0;
        let fillchar = fillchar_status(&mut group, *curwin.as_ptr());
        let attr = win_hl_attr(*curwin.as_ptr(), group);

        let mut len: usize;
        if cmdexp().first_match == 0 {
            *buf = NUL;
            len = 0;
        } else {
            crate::memory::cstr_cpy(buf, c"< ".as_ptr());
            len = 2;
        }
        clen = len as c_int;

        let mut i = cmdexp().first_match;
        while clen + wildmenu_match_len(xp, show_match(matches, i as usize, showtail)) + 2 < g_st().Columns
        {
            if i == match_idx {
                selstart = buf.add(len);
                selstart_col = clen;
            }

            let mut s = show_match(matches, i as usize, showtail);
            let emenu = xp.xp_context == EXPAND_MENUS || xp.xp_context == EXPAND_MENUNAMES;
            if emenu && menu_is_separator(s) {
                crate::memory::cstr_cpy(buf.add(len), transchar(b'|' as c_int));
                let l = crate::memory::cstr_len(buf.add(len));
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
                        crate::memory::cstr_cpy(buf.add(len), transchar_byte(c_int::from(*s as u8)));
                        len += crate::memory::cstr_len(buf.add(len));
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

        let mut row = g_st().cmdline_row - 1;
        if row >= 0 {
            if g_st().wild_menu_showing == 0 {
                if g_st().msg_scrolled > 0 {
                    if g_st().cmdline_row == g_st().Rows - 1 {
                        msg_scroll_up(false, false);
                        g_st().msg_scrolled += 1;
                    } else {
                        g_st().cmdline_row += 1;
                        row += 1;
                    }
                    g_st().wild_menu_showing = WM_SCROLLED;
                } else {
                    if (*g_st().lastwin).w_status_height == 0 && global_stl_height() == 0 {
                        g_st().save_p_ls = opt_vars().p_ls as c_int;
                        g_st().save_p_wmh = opt_vars().p_wmh as c_int;
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
                *selend = NUL;
                grid_line_puts(selstart_col, selstart, -1, hl_attr(HLF_WM));
            }

            grid_line_fill(clen, g_st().Columns, fillchar, attr);

            grid_line_flush();
        }

        win_redraw_last_status(g_st().topframe);
    }
}

/// Get the next or prev cmdline completion match.
fn get_next_or_prev_match(mode: c_int, xp: &mut expand_T) -> Option<CString> {
    unsafe {
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
            let arr = &raw const cmdexp().compl_match_array;
            if !(*arr).is_empty() {
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
}

/// Start the command-line expansion and get the matches.
fn ExpandOne_start(
    mode: c_int,
    xp: &mut expand_T,
    str_arg: *mut c_char,
    options: c_int,
) -> Option<CString> {
    let mut ss: Option<CString> = None;

        let mut xp_files = std::mem::take(&mut xp.xp_files);
        let expand_res = ExpandFromContext(xp, str_arg, &mut xp_files, options);
        xp.xp_files = xp_files;
        if !expand_res {
            xp.xp_numfiles = 0;
            #[cfg(windows)]
            if (options & WILD_SILENT) == 0 && (options & WILD_LIST_NOTFOUND) != 0 {
                semsg!(gettext(e_nomatch2.as_ptr()), str_arg);
            }
        } else {
            xp.xp_numfiles = xp.xp_files.len() as c_int;
            if xp.xp_numfiles == 0 {
                if (options & WILD_SILENT) == 0 {
                    semsg!(gettext(e_nomatch2.as_ptr()), str_arg);
                }
            } else {
                let nfiles = xp.xp_numfiles as usize;
                let mut xp_files = std::mem::take(&mut xp.xp_files);
                ExpandEscape(
                    xp,
                    str_arg,
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
                            if match_suffix(xp.xp_files.as_slice()[i].as_ptr().cast_mut()) {
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
                        ss = Some(xp.xp_files.as_slice()[0].clone());
                    }
                }
            }
        }

        ss
}

/// Return the longest common part in the list of cmdline completion matches.
fn find_longest_match(xp: &mut expand_T, options: c_int) -> CString {
    unsafe {
        let mut len: usize = 0;
        let files = xp.xp_files.as_slice();
        let first_file = files[0].as_ptr();

        while *first_file.add(len) != NUL {
            let mb_len = utfc_ptr2len(first_file.add(len)) as usize;
            let c0 = utf_ptr2char(first_file.add(len));
            let mut i: usize = 1;
            while (i as c_int) < xp.xp_numfiles {
                let ci = utf_ptr2char(files[i].as_ptr().add(len));

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
            if (i as c_int) < xp.xp_numfiles {
                if (options & WILD_NO_BEEP) == 0 {
                    vim_beep(kOptBoFlagWildmode);
                }
                break;
            }
            len += mb_len;
        }

        cstr_from_bytes(std::slice::from_raw_parts(first_file.cast::<u8>(), len))
    }
}

pub fn free_xp_files_extra(xp: &mut expand_T) {
    xp.xp_files_abbr.clear();
    xp.xp_files_kind.clear();
    xp.xp_files_menu.clear();
    xp.xp_files_info.clear();
}

/// Do wildcard expansion on the string "str".
pub fn ExpandOne(
    xp: &mut expand_T,
    str_arg: *mut c_char,
    orig: Option<CString>,
    options: c_int,
    mode: c_int,
) -> Option<CString> {
    unsafe {
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

            let arr = &raw const cmdexp().compl_match_array;
            if !(*arr).is_empty() {
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

            ss = ExpandOne_start(mode, xp, str_arg, options);
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
                let i = idx as c_int;
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
            ExpandCleanup(xp);
        }

        ss
    }
}

/// Prepare an expand structure for use.
pub fn ExpandInit(xp: &mut expand_T) {
    *xp = expand_T::new();
}

/// Cleanup an expand structure after use.
pub fn ExpandCleanup(xp: &mut expand_T) {
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
    xp: &expand_T,
    matches: &[CString],
    lines: c_int,
    linenr: c_int,
    maxlen: c_int,
    showtail: bool,
) {
    let numMatches = matches.len() as c_int;
        let mut lastlen: c_int = 999;
        let mut j = linenr;
        while j < numMatches {
            if xp.xp_context == EXPAND_TAGS_LISTFILES {
                let m = matches[j as usize].as_ptr();
                msg_outtrans(m, HLF_D, false);
                if let Some((kind_str, fname_str)) = crate::tag::tag_listfiles_extra(j as usize) {
                    msg_advance(maxlen + 1);
                    msg_puts(kind_str);
                    msg_advance(maxlen + 3);
                    msg_outtrans_long(fname_str.cast_mut(), HLF_D);
                }
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
                    let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();
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
        if g_st().msg_col > 0 {
            msg_clr_eos();
            msg_putchar(b'\n' as c_int);
        }
}

#[inline]
fn show_pum_matches(
    ccline: &mut CmdlineInfo,
    xp: &expand_T,
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
    xp: &mut expand_T,
    display_wildmenu: bool,
    display_list: bool,
    wim_flags_arg: c_int,
) -> c_int {
    unsafe {
        let ccline = get_cmdline_info();
        let mut temp_matches: Vec<CString> = Vec::new();
        let showtail: bool;
        let noselect = (wim_flags_arg & c_int::from(kOptWimFlagNoselect)) != 0;
        let noinsert = (wim_flags_arg & c_int::from(kOptWimFlagNoinsert)) != 0;
        let cmdline_unchanged = noselect || noinsert;

        let matches: &[CString] = if xp.xp_numfiles == -1 {
            set_expand_context(xp);
            if xp.xp_context == EXPAND_LUA {
                nlua_expand_pat(xp);
            }
            let retval = expand_cmdline(xp, (*ccline).cmdbuff, (*ccline).cmdpos, &mut temp_matches);
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
            show_pum_matches(&mut *ccline, xp, matches, showtail, noselect, cmdline_unchanged);
            if xp.xp_numfiles == -1 {
                free_xp_files_extra(xp);
            }
            return EXPAND_OK;
        }

        if display_list {
            g_st().msg_didany = false;
            msg_start();
            if !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }
            ui_flush();
            g_st().cmdline_row = g_st().msg_row;
            g_st().msg_didany = false;
            msg_ext_set_kind(c"wildlist".as_ptr());
            msg_start();
        }

        if g_st().got_int {
            g_st().got_int = false;
        } else if display_wildmenu && !display_list {
            redraw_wildmenu(xp, matches, if noselect { -1 } else { 0 }, showtail);
        } else if display_list {
            let numMatches = matches.len() as c_int;
            let mut maxlen: c_int = 0;
            for (i, match_i) in matches.iter().enumerate() {
                let len = if !showtail
                    && (xp.xp_context == EXPAND_FILES
                        || xp.xp_context == EXPAND_SHELLCMD
                        || xp.xp_context == EXPAND_BUFFERS)
                {
                    let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();
                    home_replace(ptr::null(), match_i.as_ptr(), name_buff, MAXPATHL, true);
                    vim_strsize(name_buff)
                } else {
                    vim_strsize(show_match(matches, i, showtail))
                };
                maxlen = maxlen.max(len);
            }

            let lines = if xp.xp_context == EXPAND_TAGS_LISTFILES {
                numMatches
            } else {
                maxlen += 2;
                let mut columns = (g_st().Columns + 2) / maxlen;
                if columns < 1 {
                    columns = 1;
                }
                (numMatches + columns - 1) / columns
            };

            if xp.xp_context == EXPAND_TAGS_LISTFILES {
                msg_puts_hl(gettext(c"tagname".as_ptr()), HLF_T, false);
                msg_clr_eos();
                msg_advance(maxlen - 3);
                msg_puts_hl(gettext(c" kind file\n".as_ptr()), HLF_T, false);
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
                show_pum_matches(&mut *ccline, xp, matches, showtail, noselect, cmdline_unchanged);
            } else {
                redraw_wildmenu(xp, matches, if noselect { -1 } else { 0 }, showtail);
            }
        }

        if xp.xp_numfiles == -1 {
            free_xp_files_extra(xp);
        }

        EXPAND_OK
    }
}

/// path_tail() version for showmatches() and redraw_wildmenu().
fn showmatches_gettail(s: *mut c_char, eager: bool) -> *mut c_char {
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
fn expand_showtail(xp: &mut expand_T) -> bool {
    unsafe {
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
pub fn addstar(fname: *mut c_char, mut len: usize, context: c_int) -> CString {
    unsafe {
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
                cstr_from_bytes(std::slice::from_raw_parts(fname.cast::<u8>(), len))
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
                let mut retval = vec![0u8; new_len];
                retval[0] = b'^';
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
                    retval[j] = *fname.add(i) as u8;
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
            cstr_from_bytes(std::slice::from_raw_parts(retval.cast::<u8>(), len))
        }
    }
}

/// Must parse the command line so far to work out what context we are in.
pub fn set_expand_context(xp: &mut expand_T) {
    unsafe {
        let ccline = get_cmdline_info();

        if ((*ccline).cmdfirstc == b'/' as c_int || (*ccline).cmdfirstc == b'?' as c_int)
            && cmdexp().may_expand_pattern
        {
            xp.xp_context = EXPAND_PATTERN_IN_BUF;
            xp.xp_search_dir = if (*ccline).cmdfirstc == b'/' as c_int {
                FORWARD
            } else {
                BACKWARD
            };
            xp.xp_pattern = (*ccline).cmdbuff;
            xp.xp_pattern_len = (*ccline).cmdpos as usize;
            g_st().Search.first_line = 0;
            return;
        }

        if (*ccline).cmdfirstc != b':' as c_int
            && (*ccline).cmdfirstc != b'>' as c_int
            && (*ccline).cmdfirstc != b'=' as c_int
            && (*ccline).input_fn == 0
        {
            xp.xp_context = EXPAND_NOTHING;
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
fn set_cmd_index(
    cmd: *const c_char,
    eap: &mut exarg_T,
    xp: &mut expand_T,
    complp: &mut c_int,
) -> *const c_char {
    unsafe {
        let mut p: *const c_char;
        let fuzzy = cmdline_fuzzy_complete(cmd);

        if !fuzzy && opt_vars().p_ic == 0 && *cmd == b'k' as c_char && *cmd.add(1) != b'e' as c_char {
            eap.cmdidx = cmdidx_T::CMD_k as c_int;
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
                xp.xp_context = EXPAND_UNSUCCESSFUL;
                return ptr::null();
            }

            eap.cmdidx = excmd_get_cmdidx(cmd, len) as c_int;

            if (*cmd >= b'A' as c_char && *cmd <= b'Z' as c_char)
                || (fuzzy && eap.cmdidx != cmdidx_T::CMD_bang as c_int as c_int && *p != NUL)
            {
                while ascii_isalnum(*p) || *p == b'*' as c_char {
                    p = p.add(1);
                }
            }
        }

        if *p == NUL && ascii_isalnum(*p.sub(1)) {
            return ptr::null();
        }

        if eap.cmdidx == cmdidx_T::CMD_SIZE as c_int as c_int {
            if *cmd == b's' as c_char
                && !vim_strchr(c"cgriI".as_ptr(), *cmd.add(1) as u8 as c_int).is_null()
            {
                eap.cmdidx = cmdidx_T::CMD_substitute as c_int;
                p = cmd.add(1);
            } else if *cmd >= b'A' as c_char && *cmd <= b'Z' as c_char {
                eap.cmd = cmd.cast_mut();
                p = find_ucmd(eap, p.cast_mut(), None, Some(&mut *xp), Some(&mut *complp));
                if p.is_null() {
                    eap.cmdidx = cmdidx_T::CMD_SIZE as c_int;
                }
            }
        }
        if eap.cmdidx == cmdidx_T::CMD_SIZE as c_int as c_int {
            xp.xp_context = EXPAND_UNSUCCESSFUL;
            return ptr::null();
        }

        p
    }
}

/// Set the completion context for a command argument with wild card characters.
fn set_context_for_wildcard_arg(
    eap: Option<&exarg_T>,
    arg: *const c_char,
    usefilter: bool,
    xp: &mut expand_T,
    complp: &mut c_int,
) {
    unsafe {
        let mut in_quote = false;
        let mut bow: *const c_char = ptr::null();
        let mut len: usize;

        xp.xp_pattern = skipwhite(arg);
        let mut p: *const c_char = xp.xp_pattern;
        while *p != NUL {
            let mut c = utf_ptr2char(p);
            if c == b'\\' as c_int && *p.add(1) != NUL {
                p = p.add(1);
            } else if c == b'`' as c_int {
                if !in_quote {
                    xp.xp_pattern = p.cast_mut();
                    bow = p.add(1);
                }
                in_quote = !in_quote;
            } else if c == b'|' as c_int
                || c == b'\n' as c_int
                || c == b'"' as c_int
                || (ascii_iswhite(c)
                    && (!eap.is_some_and(|eap| (eap.argt & EX_NOSPC) != 0) || usefilter))
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
                    xp.xp_pattern = p.cast_mut();
                }
                p = p.sub(len);
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }

        if !bow.is_null() && in_quote {
            xp.xp_pattern = bow.cast_mut();
        }
        xp.xp_context = EXPAND_FILES;

        if usefilter
            || eap.is_some_and(|eap| {
                eap.cmdidx == cmdidx_T::CMD_bang as c_int
                    || eap.cmdidx == cmdidx_T::CMD_terminal as c_int
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

        if *xp.xp_pattern == b'$' as c_char {
            p = xp.xp_pattern.add(1);
            p = p.add(env_name_len(p));
            if *p == NUL {
                xp.xp_context = EXPAND_ENV_VARS;
                xp.xp_pattern = xp.xp_pattern.add(1);
                if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST {
                    *complp = EXPAND_ENV_VARS;
                }
            }
        }

        if *xp.xp_pattern == b'~' as c_char {
            p = xp.xp_pattern.add(1);
            while *p != NUL && *p != b'/' as c_char {
                p = p.add(1);
            }
            if *p == NUL
                && (p as usize) > (xp.xp_pattern.add(1) as usize)
                && match_user(xp.xp_pattern.add(1)) >= 1
            {
                xp.xp_context = EXPAND_USER;
                xp.xp_pattern = xp.xp_pattern.add(1);
            }
        }
    }
}

/// Set the completion context for the "++opt=arg" argument. Always returns NULL.
fn set_context_in_argopt(xp: &mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        let p = vim_strchr(arg, b'=' as c_int);
        if p.is_null() {
            xp.xp_pattern = arg.cast_mut();
        } else {
            xp.xp_pattern = p.add(1);
        }

        xp.xp_context = EXPAND_ARGOPT;
        ptr::null()
    }
}

/// Set the completion context for the :filter command.
fn set_context_in_filter_cmd(xp: &mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        if *arg != NUL {
            arg = skip_vimgrep_pat(arg.cast_mut(), ptr::null_mut(), None);
        }
        if arg.is_null() || *arg == NUL {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }
        skipwhite(arg)
    }
}

/// Set the completion context for the :match command.
fn set_context_in_match_cmd(xp: &mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        if *arg == NUL || ends_excmd(*arg as c_int) == 0 {
            set_context_in_echohl_cmd(xp, arg);
            arg = skipwhite(skiptowhite(arg));
            if *arg != NUL {
                xp.xp_context = EXPAND_NOTHING;
                arg = skip_regexp(arg.add(1).cast_mut(), *arg as u8 as c_int, opt_vars().p_magic);
            }
        }
        find_nextcmd(arg)
    }
}

/// Returns a pointer to the next command after a :global or a :v command.
fn find_cmd_after_global_cmd(mut arg: *const c_char) -> *const c_char {
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
fn find_cmd_after_substitute_cmd(mut arg: *const c_char) -> *const c_char {
    unsafe {
        let delim = *arg as u8 as c_int;
        if delim != 0 {
            arg = arg.add(1);
            arg = skip_regexp(arg.cast_mut(), delim, opt_vars().p_magic);

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
fn find_cmd_after_isearch_cmd(xp: &mut expand_T, mut arg: *const c_char) -> *const c_char {
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
                xp.xp_context = EXPAND_NOTHING;
            } else {
                return arg;
            }
        }

        ptr::null()
    }
}

/// Set the completion context for the :unlet command. Always returns NULL.
fn set_context_in_unlet_cmd(xp: &mut expand_T, mut arg: *const c_char) -> *const c_char {
    unsafe {
        while {
            xp.xp_pattern = c_strchr(arg, b' ' as c_char);
            !xp.xp_pattern.is_null()
        } {
            arg = xp.xp_pattern.add(1);
        }

        xp.xp_context = EXPAND_USER_VARS;
        xp.xp_pattern = arg.cast_mut();

        if *xp.xp_pattern == b'$' as c_char {
            xp.xp_context = EXPAND_ENV_VARS;
            xp.xp_pattern = xp.xp_pattern.add(1);
        }

        ptr::null()
    }
}

/// Set the completion context for the :language command. Always returns NULL.
fn set_context_in_lang_cmd(xp: &mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        let p = skiptowhite(arg);
        if *p == NUL {
            xp.xp_context = EXPAND_LANGUAGE;
            xp.xp_pattern = arg.cast_mut();
        } else {
            let n = p.offset_from(arg) as usize;
            if (crate::memory::cstr_prefix_bytes(arg, n) == crate::memory::cstr_prefix_bytes(c"messages".as_ptr(), n))
                || (crate::memory::cstr_prefix_bytes(arg, n) == crate::memory::cstr_prefix_bytes(c"ctype".as_ptr(), n))
                || (crate::memory::cstr_prefix_bytes(arg, n) == crate::memory::cstr_prefix_bytes(c"time".as_ptr(), n))
                || (crate::memory::cstr_prefix_bytes(arg, n) == crate::memory::cstr_prefix_bytes(c"collate".as_ptr(), n))
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
    xp: &mut expand_T,
    arg: *const c_char,
    cmdidx: cmdidx_T,
) -> *const c_char {
    unsafe {
        xp.xp_context = EXPAND_BREAKPOINT;
        xp.xp_pattern = arg.cast_mut();

        if cmdidx == cmdidx_T::CMD_breakadd {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Add;
        } else if cmdidx == cmdidx_T::CMD_breakdel {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Del;
        } else {
            cmdexp().breakpt_expand_what = BreakptExpandWhat::Profdel;
        }

        let mut p: *const c_char = skipwhite(arg);
        if *p == NUL {
            return ptr::null();
        }
        let subcmd_start = p;

        if (crate::memory::cstr_prefix_bytes(c"file ".as_ptr(), 5) == crate::memory::cstr_prefix_bytes(p, 5)) || (crate::memory::cstr_prefix_bytes(c"func ".as_ptr(), 5) == crate::memory::cstr_prefix_bytes(p, 5)) {
            p = p.add(4);
            p = skipwhite(p);

            if ascii_isdigit(*p as c_int) {
                p = skipdigits(p);
                if *p != b' ' as c_char {
                    xp.xp_context = EXPAND_NOTHING;
                    return ptr::null();
                }
                p = skipwhite(p);
            }
            if crate::memory::cstr_prefix_bytes(c"file".as_ptr(), 4) == crate::memory::cstr_prefix_bytes(subcmd_start, 4) {
                xp.xp_context = EXPAND_FILES;
            } else {
                xp.xp_context = EXPAND_USER_FUNC;
            }
            xp.xp_pattern = p.cast_mut();
        } else if crate::memory::cstr_prefix_bytes(c"expr ".as_ptr(), 5) == crate::memory::cstr_prefix_bytes(p, 5) {
            xp.xp_context = EXPAND_EXPRESSION;
            xp.xp_pattern = skipwhite(p.add(5));
        }

        ptr::null()
    }
}

fn set_context_in_scriptnames_cmd(xp: &mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        xp.xp_context = EXPAND_NOTHING;
        xp.xp_pattern = ptr::null_mut();

        let p = skipwhite(arg);
        if ascii_isdigit(*p as c_int) {
            return ptr::null();
        }

        xp.xp_context = EXPAND_SCRIPTNAMES;
        xp.xp_pattern = p;

        ptr::null()
    }
}

/// Set the completion context for the :filetype command. Always returns NULL.
fn set_context_in_filetype_cmd(xp: &mut expand_T, arg: *const c_char) -> *const c_char {
    unsafe {
        xp.xp_context = EXPAND_FILETYPECMD;
        xp.xp_pattern = arg.cast_mut();
        cmdexp().filetype_expand_what = FiletypeExpandWhat::All;

        let mut p = skipwhite(arg);
        if *p == NUL {
            return ptr::null();
        }

        let mut val: c_int = 0;

        loop {
            if (crate::memory::cstr_prefix_bytes(p, 6) == &b"plugin"[..6]) && ascii_iswhite(*p.add(6) as c_int) {
                val |= EXPAND_FILETYPECMD_PLUGIN;
                p = skipwhite(p.add(6));
                continue;
            }
            if (crate::memory::cstr_prefix_bytes(p, 6) == &b"indent"[..6]) && ascii_iswhite(*p.add(6) as c_int) {
                val |= EXPAND_FILETYPECMD_INDENT;
                p = skipwhite(p.add(6));
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
fn set_context_with_pattern(xp: &mut expand_T) {
    unsafe {
        let ccline = get_cmdline_info();

        g_st().emsg_off += 1;
        let mut skiplen: c_int = 0;
        let mut dummy: c_int = 0;
        let mut patlen: c_int = 0;
        let mut magic = opt_vars().p_magic != 0;
        let retval = parse_pattern_and_range(
            &mut cmdexp().pre_incsearch_pos,
            &mut dummy,
            &mut skiplen,
            &mut patlen,
            &mut magic,
        );
        g_st().emsg_off -= 1;

        if !retval || (*ccline).cmdpos <= skiplen || (*ccline).cmdpos > skiplen + patlen {
            return;
        }

        xp.xp_pattern = (*ccline).cmdbuff.add(skiplen as usize);
        xp.xp_pattern_len = ((*ccline).cmdpos - skiplen) as usize;
        xp.xp_context = EXPAND_PATTERN_IN_BUF;
        xp.xp_search_dir = FORWARD;
    }
}

/// Set the completion context in "xp" for command "cmd" with index "cmdidx".
fn set_context_by_cmdname(
    cmd: *const c_char,
    cmdidx: cmdidx_T,
    xp: &mut expand_T,
    mut arg: *const c_char,
    argt: u32,
    context: c_int,
    forceit: bool,
) -> *const c_char {
    unsafe {
        match cmdidx {
            cmdidx_T::CMD_find | cmdidx_T::CMD_sfind | cmdidx_T::CMD_tabfind => {
                if xp.xp_context == EXPAND_FILES {
                    xp.xp_context = if (*get_findfunc()).cb_type() != 0 {
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
                if xp.xp_context == EXPAND_FILES {
                    xp.xp_context = EXPAND_DIRS_IN_CDPATH;
                }
            }
            cmdidx_T::CMD_help => {
                xp.xp_context = EXPAND_HELP;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_USER_COMMANDS;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_global | cmdidx_T::CMD_vglobal => {
                let nextcmd = find_cmd_after_global_cmd(arg);
                if nextcmd.is_null() && cmdexp().may_expand_pattern {
                    set_context_with_pattern(xp);
                }
                return nextcmd;
            }
            cmdidx_T::CMD_and | cmdidx_T::CMD_substitute => {
                let nextcmd = find_cmd_after_substitute_cmd(arg);
                if nextcmd.is_null() && cmdexp().may_expand_pattern {
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
                if (opt_vars().wop_flags & kOptWopFlagTagfile) != 0 {
                    xp.xp_context = EXPAND_TAGS_LISTFILES;
                } else {
                    xp.xp_context = EXPAND_TAGS;
                }
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_augroup => {
                xp.xp_context = EXPAND_AUGROUP;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_USER_FUNC;
                xp.xp_pattern = arg.cast_mut();
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
                    xp.xp_pattern = c_strchr(arg, b' ' as c_char);
                    !xp.xp_pattern.is_null()
                } {
                    arg = xp.xp_pattern.add(1);
                }
                xp.xp_context = EXPAND_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_buffer
            | cmdidx_T::CMD_sbuffer
            | cmdidx_T::CMD_pbuffer
            | cmdidx_T::CMD_checktime => {
                xp.xp_context = EXPAND_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_diffget | cmdidx_T::CMD_diffput => {
                xp.xp_context = EXPAND_DIFF_BUFFERS;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_MAPCLEAR;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_COLORS;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_compiler => {
                xp.xp_context = EXPAND_COMPILER;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_ownsyntax => {
                xp.xp_context = EXPAND_OWNSYNTAX;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_setfiletype => {
                xp.xp_context = EXPAND_FILETYPE;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_packadd => {
                xp.xp_context = EXPAND_PACKADD;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_CHECKHEALTH;
            }
            cmdidx_T::CMD_marks => {
                xp.xp_context = EXPAND_MARKS;
            }
            cmdidx_T::CMD_log => {
                xp.xp_context = EXPAND_LOG;
            }
            cmdidx_T::CMD_lsp => {
                xp.xp_context = EXPAND_LSP;
            }
            cmdidx_T::CMD_packdel => {
                xp.xp_context = EXPAND_PACKDEL;
            }
            cmdidx_T::CMD_packupdate => {
                xp.xp_context = EXPAND_PACKUPDATE;
            }
            cmdidx_T::CMD_retab => {
                xp.xp_context = EXPAND_RETAB;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_messages => {
                xp.xp_context = EXPAND_MESSAGES;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_history => {
                xp.xp_context = EXPAND_HISTORY;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_syntime => {
                xp.xp_context = EXPAND_SYNTIME;
                xp.xp_pattern = arg.cast_mut();
            }
            cmdidx_T::CMD_argdelete => {
                while {
                    xp.xp_pattern = vim_strchr(arg, b' ' as c_int);
                    !xp.xp_pattern.is_null()
                } {
                    arg = xp.xp_pattern.add(1);
                }
                xp.xp_context = EXPAND_ARGLIST;
                xp.xp_pattern = arg.cast_mut();
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
                xp.xp_context = EXPAND_LUA;
            }
            _ => {}
        }
        ptr::null()
    }
}

/// Parse one command from `buff` and set completion context in `xp`.
fn set_one_cmd_context(xp: &mut expand_T, buff: *const c_char) -> *const c_char {
    unsafe {
        let mut ea: exarg_T = exarg_T::new();
        let mut context: c_int = EXPAND_NOTHING;
        let mut forceit = false;
        let mut usefilter = false;

        ExpandInit(xp);
        xp.xp_pattern = buff.cast_mut();
        xp.xp_line = buff.cast_mut();
        xp.xp_context = EXPAND_COMMANDS;
        ea.argt = 0;

        let mut cmd = buff;
        while !vim_strchr(c" \t:|".as_ptr(), *cmd as u8 as c_int).is_null() {
            cmd = cmd.add(1);
        }
        xp.xp_pattern = cmd.cast_mut();

        if *cmd == NUL {
            return ptr::null();
        }
        if *cmd == b'"' as c_char {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        cmd = skip_range(cmd, Some(&mut xp.xp_context));
        xp.xp_pattern = cmd.cast_mut();
        if *cmd == NUL {
            return ptr::null();
        }
        if *cmd == b'"' as c_char {
            xp.xp_context = EXPAND_NOTHING;
            return ptr::null();
        }

        if *cmd == b'|' as c_char || *cmd == b'\n' as c_char {
            return cmd.add(1);
        }

        let mut p = set_cmd_index(cmd, &mut ea, xp, &mut context);
        if p.is_null() {
            return ptr::null();
        }

        xp.xp_context = EXPAND_NOTHING;

        if *p == b'!' as c_char {
            forceit = true;
            p = p.add(1);
        }

        if !is_user_cmdidx(cmdidx_T::from_raw(ea.cmdidx)) {
            ea.argt = excmd_get_argt(cmdidx_T::from_raw(ea.cmdidx));
        }

        let mut arg: *const c_char = skipwhite(p);

        if (ea.argt & EX_ARGOPT) != 0 {
            while *arg != NUL && (crate::memory::cstr_prefix_bytes(arg, 2) == &b"++"[..2]) {
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
        xp.xp_pattern = p.cast_mut();
        let len = crate::memory::cstr_len(buff);
        while *p != NUL && (p as usize) < (buff.add(len) as usize) {
            if *p == b' ' as c_char || *p == TAB {
                p = p.add(1);
                xp.xp_pattern = p.cast_mut();
            } else {
                if *p == b'\\' as c_char && *p.add(1) != NUL {
                    p = p.add(1);
                }
                p = p.add(utfc_ptr2len(p) as usize);
            }
        }

        if (ea.argt & EX_XFILE) != 0 {
            set_context_for_wildcard_arg(Some(&ea), arg, usefilter, xp, &mut context);
        }

        set_context_by_cmdname(
            cmd,
            cmdidx_T::from_raw(ea.cmdidx),
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
    xp: &mut expand_T,
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

        cmdexp().filetype_expand_what = FiletypeExpandWhat::All;
        cmdexp().breakpt_expand_what = BreakptExpandWhat::Add;

        if use_ccline != 0 && (*ccline).cmdfirstc == b'=' as c_int {
            set_context_for_expression(xp, str_arg, cmdidx_T::CMD_SIZE);
        } else if use_ccline != 0 && (*ccline).input_fn != 0 {
            xp.xp_context = (*ccline).xp_context;
            xp.xp_pattern = (*ccline).cmdbuff;
            xp.xp_arg = (*ccline)
                .xp_arg
                .as_ref()
                .map_or(ptr::null_mut(), |a| a.as_ptr().cast_mut());
            if xp.xp_context == EXPAND_SHELLCMDLINE {
                let mut context = xp.xp_context;
                set_context_for_wildcard_arg(
                    None,
                    xp.xp_pattern,
                    false,
                    xp,
                    &mut context,
                );
            }
        } else {
            while !nextcomm.is_null() {
                nextcomm = set_one_cmd_context(xp, nextcomm);
            }
        }

        xp.xp_line = str_arg;
        xp.xp_col = col;

        if col < len {
            *str_arg.add(col as usize) = old_char;
        }
    }
}

/// Expand the command line "str" from context "xp".
pub fn expand_cmdline(
    xp: &mut expand_T,
    str_arg: *const c_char,
    col: c_int,
    matches: &mut Vec<CString>,
) -> c_int {
    unsafe {
        let mut options = WILD_ADD_SLASH | WILD_SILENT;

        if xp.xp_context == EXPAND_UNSUCCESSFUL {
            beep_flush();
            return EXPAND_UNSUCCESSFUL;
        }
        if xp.xp_context == EXPAND_NOTHING {
            return EXPAND_NOTHING;
        }

        debug_assert!(str_arg.add(col as usize).offset_from(xp.xp_pattern) >= 0);
        xp.xp_pattern_len = str_arg.add(col as usize).offset_from(xp.xp_pattern) as usize;
        let file_str = if cmdline_fuzzy_completion_supported(xp) {
            cstr_to_cstring(xp.xp_pattern)
        } else {
            addstar(xp.xp_pattern, xp.xp_pattern_len, xp.xp_context)
        };

        if opt_vars().p_wic != 0 {
            options += WILD_ICASE;
        }

        if !ExpandFromContext(xp, file_str.as_ptr().cast_mut(), matches, options) {
            matches.clear();
        }

        EXPAND_OK
    }
}

/// Expand file or directory names.
fn expand_files_and_dirs(
    xp: &mut expand_T,
    mut pat: *mut c_char,
    matches: &mut Vec<CString>,
    mut flags: c_int,
    options: c_int,
) -> bool {
    unsafe {
        let mut pat_owned: Option<Vec<c_char>> = None;

        if xp.xp_backslash != XP_BS_NONE {
            let pat_len = crate::memory::cstr_len(pat);
            let mut pat_vec = vec![0 as c_char; pat_len + 1];
            crate::memory::cstr_ncpy(pat_vec.as_mut_ptr(), pat, pat_len);
            pat = pat_vec.as_mut_ptr();
            pat_owned = Some(pat_vec);

            let mut pat_end = pat.add(pat_len);
            let mut p = pat;
            while *p != NUL {
                if *p == b'\\' as c_char {
                    if (xp.xp_backslash & XP_BS_THREE) != 0
                        && *p.add(1) == b'\\' as c_char
                        && *p.add(2) == b'\\' as c_char
                        && *p.add(3) == b' ' as c_char
                    {
                        let from = p.add(3);
                        ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                        pat_end = pat_end.sub(3);
                    } else if (xp.xp_backslash & XP_BS_ONE) != 0 && *p.add(1) == b' ' as c_char {
                        let from = p.add(1);
                        ptr::copy(from, p, (pat_end.offset_from(from) as usize) + 1);
                        pat_end = pat_end.sub(1);
                    } else if (xp.xp_backslash & XP_BS_COMMA) != 0 {
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
                && ((*opt_vars().p_csl == NUL && opt_vars().p_ssl == 0) || *opt_vars().p_csl == b'b' as c_char))
        {
            for slot in matches.iter_mut() {
                if slot.as_bytes().contains(&(PATHSEP as u8)) {
                    let bytes: Vec<u8> = slot
                        .as_bytes()
                        .iter()
                        .map(|&b| if b == PATHSEP as u8 { b'\\' } else { b })
                        .collect();
                    *slot = cstr_from_bytes(&bytes);
                }
            }
        }
        ret
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":filetype".
fn get_filetypecmd_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
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
fn get_breakadd_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
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
fn get_scriptnames_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if !script_id_valid(idx + 1) {
            return ptr::null_mut();
        }

        let si = script_item(idx + 1);
        let name_buff = (&raw mut g_st().NameBuff).cast::<c_char>();
        home_replace(ptr::null(), (*si).name_ptr(), name_buff, MAXPATHL, true);
        name_buff
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":retab".
fn get_retab_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"-indentonly".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Function given to ExpandGeneric() to obtain the possible arguments of ":messages".
fn get_messages_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"clear".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

fn get_mapclear_arg(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    if idx == 0 {
        c"<buffer>".as_ptr().cast_mut()
    } else {
        ptr::null_mut()
    }
}

/// Completion for |:checkhealth| command.
fn get_healthcheck_names(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let prompt_id = get_cmdline_last_prompt_id();
        if cmdexp().HEALTHCHECK_LAST_GEN != prompt_id || cmdexp().HEALTHCHECK_LAST_GEN == 0 {
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
                kRetObject,
                None,
                &mut err,
            );
            api_clear_error(&mut err);
            api_free_object(cmdexp().HEALTHCHECK_NAMES);
            cmdexp().HEALTHCHECK_NAMES = res;
            cmdexp().HEALTHCHECK_LAST_GEN = prompt_id;
        }

        if cmdexp().HEALTHCHECK_NAMES.obj_type == ObjectType::kObjectTypeArray
            && idx >= 0
            && (idx as usize) < cmdexp().HEALTHCHECK_NAMES.data.array().size
        {
            let item = *cmdexp().HEALTHCHECK_NAMES.data.array().items.add(idx as usize);
            if item.obj_type == ObjectType::kObjectTypeString {
                return item.data.string().data;
            }
        }
        ptr::null_mut()
    }
}

/// Completes arg1 via Lua.
fn get_arg1_from_lua(lua: &std::ffi::CStr, xp: &mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        let prompt_id = get_cmdline_last_prompt_id();
        let last_line_ptr = &raw const cmdexp().ARG1_LUA_LAST_XP_LINE;
        if (*last_line_ptr)
            .as_ref()
            .is_none_or(|s| crate::memory::cstr_as_bytes(s.as_ptr()) != crate::memory::cstr_as_bytes(xp.xp_line))
            || cmdexp().ARG1_LUA_LAST_GEN != prompt_id
        {
            cmdexp().ARG1_LUA_LAST_XP_LINE = Some(cstr_to_cstring(xp.xp_line));
            let mut args_items = [
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData::String(cstr_as_string(xp.xp_pattern)),
                },
                Object {
                    obj_type: ObjectType::kObjectTypeString,
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
                kRetObject,
                None,
                &mut err,
            );
            api_clear_error(&mut err);
            api_free_object(cmdexp().ARG1_LUA_NAMES);
            cmdexp().ARG1_LUA_NAMES = res;
            cmdexp().ARG1_LUA_LAST_GEN = prompt_id;
        }

        if cmdexp().ARG1_LUA_NAMES.obj_type == ObjectType::kObjectTypeArray
            && idx >= 0
            && (idx as usize) < cmdexp().ARG1_LUA_NAMES.data.array().size
        {
            let item = *cmdexp().ARG1_LUA_NAMES.data.array().items.add(idx as usize);
            if item.obj_type == ObjectType::kObjectTypeString {
                return item.data.string().data;
            }
        }
        ptr::null_mut()
    }
}

fn get_marks_arg(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    get_arg1_from_lua(
            c"return require'vim._core.marks'.complete(...)",
            xp,
            idx,
        )
}

fn get_log_arg(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.log_complete(...)",
            xp,
            idx,
        )
}

fn get_lsp_arg(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.lsp_complete(...)",
            xp,
            idx,
        )
}

fn get_packdel_arg(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.packdel_complete(...)",
            xp,
            idx,
        )
}

fn get_packupdate_arg(xp: &mut expand_T, idx: c_int) -> *mut c_char {
    get_arg1_from_lua(
            c"return require'vim._core.ex_cmd'.packupdate_complete(...)",
            xp,
            idx,
        )
}

/// Do the expansion based on xp->xp_context and "rmp".
fn ExpandOther(
    pat: *mut c_char,
    xp: &mut expand_T,
    rmp: &mut regmatch_T,
    matches: &mut Vec<CString>,
) -> bool {
    struct ExpGen {
        context: c_int,
        func: fn(&mut expand_T, c_int) -> *mut c_char,
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
                ExpandGeneric(pat, xp, rmp, matches, Some(entry.func), entry.escaped);
                return true;
            }
        }
        false
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
fn ExpandFromContext(
    xp: &mut expand_T,
    mut pat: *mut c_char,
    matches: &mut Vec<CString>,
    options: c_int,
) -> bool {
    unsafe {
        let mut regmatch: regmatch_T = regmatch_T::new();
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
                if *pat == NUL { c"help".as_ptr() } else { pat },
                matches,
                false,
            )
            {
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
            return ExpandOldSetting(matches);
        }
        if xp.xp_context == EXPAND_BUFFERS {
            return ExpandBufnames(pat, matches, options);
        }
        if xp.xp_context == EXPAND_DIFF_BUFFERS {
            return ExpandBufnames(pat, matches, options | BUF_DIFF_FILTER);
        }
        if xp.xp_context == EXPAND_TAGS || xp.xp_context == EXPAND_TAGS_LISTFILES {
            return expand_tags(xp.xp_context == EXPAND_TAGS, pat, matches);
        }
        if xp.xp_context == EXPAND_COLORS {
            let mut directories: [*mut c_char; 2] =
                [c"colors".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, DIP_START + DIP_OPT, matches, directories.as_mut_ptr());
        }
        if xp.xp_context == EXPAND_COMPILER {
            let mut directories: [*mut c_char; 2] =
                [c"compiler".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, matches, directories.as_mut_ptr());
        }
        if xp.xp_context == EXPAND_OWNSYNTAX {
            let mut directories: [*mut c_char; 2] =
                [c"syntax".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, matches, directories.as_mut_ptr());
        }
        if xp.xp_context == EXPAND_FILETYPE {
            let mut directories: [*mut c_char; 4] = [
                c"syntax".as_ptr().cast_mut(),
                c"indent".as_ptr().cast_mut(),
                c"ftplugin".as_ptr().cast_mut(),
                ptr::null_mut(),
            ];
            return ExpandRTDir(pat, 0, matches, directories.as_mut_ptr());
        }
        if xp.xp_context == EXPAND_KEYMAP {
            let mut directories: [*mut c_char; 2] =
                [c"keymap".as_ptr().cast_mut(), ptr::null_mut()];
            return ExpandRTDir(pat, 0, matches, directories.as_mut_ptr());
        }
        if xp.xp_context == EXPAND_USER_LIST {
            return ExpandUserList(xp, matches);
        }
        if xp.xp_context == EXPAND_USER_LUA {
            return ExpandUserLua(xp, matches);
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
        if xp.xp_context == EXPAND_USER_FUNC && (crate::memory::cstr_prefix_bytes(pat, 3) == &b"^s:"[..3]) {
            let len = crate::memory::cstr_len(pat) + 20;
            let mut buf = vec![0 as c_char; len];
            vim_snprintf!(buf.as_mut_ptr(), len, c"^<SNR>\\d\\+_%s".as_ptr(), pat.add(3));
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

        let ret = if xp.xp_context == EXPAND_SETTINGS || xp.xp_context == EXPAND_BOOL_SETTINGS
        {
            ExpandSettings(xp, &mut regmatch, pat, matches, fuzzy)
        } else if xp.xp_context == EXPAND_STRING_SETTING {
            ExpandStringSetting(xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_SETTING_SUBTRACT {
            ExpandSettingSubtract(xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_MAPPINGS {
            ExpandMappings(pat, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_ARGOPT {
            expand_argopt(pat, xp, &mut regmatch, matches)
        } else if xp.xp_context == EXPAND_USER_DEFINED {
            ExpandUserDefined(pat, xp, &mut regmatch, matches)
        } else {
            ExpandOther(pat, xp, &mut regmatch, matches)
        };

        if !fuzzy {
            vim_regfree(regmatch.regprog);
        }
        drop(tofree);

        ret
    }
}

/// Expand a list of names.
pub fn ExpandGeneric(
    pat: *const c_char,
    xp: &mut expand_T,
    regmatch: &mut regmatch_T,
    matches: &mut Vec<CString>,
    func: CompleteListItemGetter,
    escaped: bool,
) {
    unsafe {
        let fuzzy = cmdline_fuzzy_complete(pat);
        matches.clear();

        let Some(func_fn) = func else {
            return;
        };

        let mut str_ga: Vec<CString> = Vec::new();
        let mut fuz_ga: Vec<fuzmatch_str_T> = Vec::new();

        let mut i: c_int = 0;
        loop {
            let str_item = func_fn(xp, i);
            i += 1;
            if str_item.is_null() {
                break;
            }
            if *str_item == NUL {
                continue;
            }

            let match_ok: bool;
            let mut score: c_int = 0;
            if *xp.xp_pattern != NUL {
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
                vim_strsave_escaped(std::ffi::CStr::from_ptr(str_item), c" \t\\.")
            } else {
                cstr_to_cstring(str_item)
            };

            if ptr::fn_addr_eq(
                func_fn,
                get_menu_names as fn(&mut expand_T, c_int) -> *mut c_char,
            ) && item_cstr.as_bytes().last() == Some(&1)
            {
                let mut bytes = item_cstr.into_bytes();
                if let Some(last) = bytes.last_mut() {
                    *last = b'.';
                }
                item_cstr = CString::new(bytes).unwrap_or_default();
            }

            if fuzzy {
                fuz_ga.push(fuzmatch_str_T {
                    idx: fuz_ga.len() as c_int,
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
    mut pathed_pattern: *mut c_char,
    pathlen: usize,
    flags: c_int,
    found_ht: &mut std::collections::HashSet<Vec<u8>>,
    gap: &mut Vec<CString>,
) {
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
fn expand_shellcmd(filepat: *mut c_char, matches: &mut Vec<CString>, flagsarg: c_int) {
    unsafe {
        matches.clear();
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();
        let mut flags = flagsarg;
        let mut did_curdir = false;

        let mut patlen = crate::memory::cstr_len(filepat);
        let mut pat_vec = vec![0 as c_char; patlen + 1];
        crate::memory::cstr_ncpy(pat_vec.as_mut_ptr(), filepat, patlen);
        let pat = pat_vec.as_mut_ptr();
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

        let mut path_owned: Option<std::ffi::CString> = None;
        let path: *mut c_char = if *pat == b'.' as c_char
            && (vim_ispathsep(*pat.add(1) as c_int)
                || (*pat.add(1) == b'.' as c_char && vim_ispathsep(*pat.add(2) as c_int)))
        {
            c".".as_ptr().cast_mut()
        } else {
            if !path_is_absolute(pat) {
                path_owned = vim_getenv(c"PATH".as_ptr());
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
                    e = s.add(crate::memory::cstr_len(s));
                }

                pathlen = e.offset_from(s) as usize;
                if crate::memory::cstr_prefix_bytes(s, pathlen) == crate::memory::cstr_prefix_bytes(c".".as_ptr(), pathlen) {
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
                        crate::memory::cstr_ncpy(buf.add(pathlen), PATHSEPSTR.as_ptr().cast(), seplen);
                        pathlen += seplen;
                    }
                }
                crate::memory::cstr_ncpy(buf.add(pathlen), pat, patlen);

                expand_shellcmd_onedir(buf, pathlen, flags, &mut found_ht, matches);
            }

            if *e != NUL {
                e = e.add(1);
            }
            s = e;
        }
    }
}

/// Call "user_expand_func()" to invoke a user defined Vim script function.
fn call_user_expand_func<R>(
    user_expand_func: fn(*const c_char, c_int, *mut typval_T) -> R,
    xp: *mut expand_T,
    default: R,
) -> R {
    unsafe {
        let ccline = get_cmdline_info();
        let mut keep: c_char = 0;
        let mut args: [typval_T; 4] = [typval_T::new(); 4];
        let save_current_sctx = g_st().current_sctx;

        if (*xp).xp_arg.is_null() || *(*xp).xp_arg == NUL || (*xp).xp_line.is_null() {
            return default;
        }

        if !(*ccline).cmdbuff.is_null() {
            keep = *(*ccline).cmdbuff.add((*ccline).cmdlen as usize);
            *(*ccline).cmdbuff.add((*ccline).cmdlen as usize) = 0;
        }

        let pat = cstr_from_bytes(std::slice::from_raw_parts(
            (*xp).xp_pattern.cast::<u8>(),
            (*xp).xp_pattern_len,
        ));
        args[0].v_type = VarType::VAR_STRING;
        args[1].v_type = VarType::VAR_STRING;
        args[2].v_type = VarType::VAR_NUMBER;
        args[3].v_type = VarType::VAR_UNKNOWN;
        args[0].vval = crate::eval::typval::TypvalVval::String(pat.as_ptr().cast_mut());
        args[1].vval = crate::eval::typval::TypvalVval::String((*xp).xp_line);
        args[2].vval = crate::eval::typval::TypvalVval::Number(i64::from((*xp).xp_col));

        g_st().current_sctx = (*xp).xp_script_ctx;

        let ret = user_expand_func((*xp).xp_arg, 3, args.as_mut_ptr());

        g_st().current_sctx = save_current_sctx;
        if !(*ccline).cmdbuff.is_null() {
            *(*ccline).cmdbuff.add((*ccline).cmdlen as usize) = keep;
        }

        ret
    }
}

/// Expand names with a function defined by the user (EXPAND_USER_DEFINED).
fn ExpandUserDefined(
    pat: *const c_char,
    xp: &mut expand_T,
    regmatch: &mut regmatch_T,
    matches: &mut Vec<CString>,
) -> bool {
    unsafe {
        let fuzzy = cmdline_fuzzy_complete(pat);
        matches.clear();

        let Some(retstr_cstr) = call_user_expand_func(call_func_retstr, xp, None) else {
            return false;
        };
        let mut retstr_bytes = retstr_cstr.into_bytes_with_nul();
        let retstr = retstr_bytes.as_mut_ptr().cast::<c_char>();

        let mut str_ga: Vec<CString> = Vec::new();
        let mut fuz_ga: Vec<fuzmatch_str_T> = Vec::new();

        let mut s = retstr;
        while *s != NUL {
            let mut e = vim_strchr(s, b'\n' as c_int);
            if e.is_null() {
                e = s.add(crate::memory::cstr_len(s));
            }
            let keep = *e;
            *e = NUL;

            let match_ok: bool;
            let mut score: c_int = 0;
            if *xp.xp_pattern != NUL {
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
                let p = cstr_from_bytes(std::slice::from_raw_parts(
                    s.cast::<u8>(),
                    e.offset_from(s) as usize,
                ));

                if !fuzzy {
                    str_ga.push(p);
                } else {
                    fuz_ga.push(fuzmatch_str_T {
                        idx: fuz_ga.len() as c_int,
                        str_cstr: p,
                        score,
                    });
                }
            }

            if *e != NUL {
                e = e.add(1);
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
    retlist: *mut list_T,
    matches: &mut Vec<CString>,
    xp: &mut expand_T,
) {
    unsafe {
        matches.clear();
        let mut ga_abbr: Vec<Option<std::ffi::CString>> = Vec::new();
        let mut ga_kind: Vec<Option<std::ffi::CString>> = Vec::new();
        let mut ga_menu: Vec<Option<std::ffi::CString>> = Vec::new();
        let mut ga_info: Vec<Option<std::ffi::CString>> = Vec::new();
        let mut have_extra = false;

        if !retlist.is_null() {
            let mut li = (*retlist).lv_first;
            while !li.is_null() {
                let tv = &raw const (*li).li_tv;
                let p: CString;
                let mut abbr: Option<std::ffi::CString> = None;
                let mut kind: Option<std::ffi::CString> = None;
                let mut menu: Option<std::ffi::CString> = None;
                let mut info: Option<std::ffi::CString> = None;

                if (*tv).v_type == VarType::VAR_STRING {
                    if (*tv).vval.v_string().is_null() {
                        li = (*li).li_next;
                        continue;
                    }
                    p = cstr_to_cstring((*tv).vval.v_string());
                } else if (*tv).v_type == VarType::VAR_DICT && !(*tv).vval.v_dict().is_null() {
                    let d = (*tv).vval.v_dict();
                    let word = tv_dict_get_string(d, c"word".as_ptr());
                    if word.is_null() {
                        li = (*li).li_next;
                        continue;
                    }
                    p = cstr_to_cstring(word);
                    abbr = crate::eval::typval::tv_dict_get_cstring(d, c"abbr");
                    kind = crate::eval::typval::tv_dict_get_cstring(d, c"kind");
                    menu = crate::eval::typval::tv_dict_get_cstring(d, c"menu");
                    info = crate::eval::typval::tv_dict_get_cstring(d, c"info");
                    if abbr.is_some() || kind.is_some() || menu.is_some() || info.is_some() {
                        have_extra = true;
                    }
                } else {
                    li = (*li).li_next;
                    continue;
                }

                matches.push(p);
                ga_abbr.push(abbr);
                ga_kind.push(kind);
                ga_menu.push(menu);
                ga_info.push(info);

                li = (*li).li_next;
            }
        }

        if have_extra && !matches.is_empty() {
            xp.xp_files_abbr = ga_abbr;
            xp.xp_files_kind = ga_kind;
            xp.xp_files_menu = ga_menu;
            xp.xp_files_info = ga_info;
        }
    }
}

/// Expand names with a list returned by a function defined by the user.
fn ExpandUserList(xp: &mut expand_T, matches: &mut Vec<CString>) -> bool {
    matches.clear();
        let retlist = call_user_expand_func(call_func_retlist, xp, ptr::null_mut());
        if retlist.is_null() {
            return false;
        }

        expand_process_user_list(retlist, matches, xp);
        tv_list_unref(retlist);
        true
}

fn ExpandUserLua(xp: &mut expand_T, matches: &mut Vec<CString>) -> bool {
    let mut rettv: typval_T = typval_T::new();
        nlua_call_user_expand_func(xp, &mut rettv);
        if rettv.v_type != VarType::VAR_LIST {
            tv_clear(&raw mut rettv);
            return false;
        }

        let retlist = rettv.vval.v_list();
        expand_process_user_list(retlist, matches, xp);
        tv_list_unref(retlist);
        true
}

/// Expand `file` for all comma-separated directories in `path`.
pub fn globpath(
    mut path: *mut c_char,
    file: *mut c_char,
    ga: &mut Vec<CString>,
    expand_options: c_int,
    dirs: bool,
) {
    unsafe {
        let mut buf_vec = vec![0 as c_char; MAXPATHL];
        let buf = buf_vec.as_mut_ptr();

        let mut xpc: expand_T = expand_T::new();
        xpc.xp_context = if dirs {
            EXPAND_DIRECTORIES
        } else {
            EXPAND_FILES
        };

        let filelen = crate::memory::cstr_len(file);

        while *path != NUL {
            let mut pathlen = copy_option_part(&raw mut path, buf, MAXPATHL, c",".as_ptr());
            let seplen = if *buf != NUL && after_pathsep(buf, buf.add(pathlen)) == 0 {
                PATHSEPSTR.len()
            } else {
                0
            };

            if pathlen + seplen + filelen < MAXPATHL {
                if seplen > 0 {
                    crate::memory::cstr_ncpy(buf.add(pathlen), PATHSEPSTR.as_ptr().cast(), seplen);
                    pathlen += seplen;
                }
                crate::memory::cstr_ncpy(buf.add(pathlen), file, filelen);

                let mut p: Vec<CString> = Vec::new();
                ExpandFromContext(&mut xpc, buf, &mut p, WILD_SILENT | expand_options);
                if !p.is_empty() {
                    ExpandEscape(&mut xpc, buf, &mut p, WILD_SILENT | expand_options);
                    ga.extend(p);
                }
            }
        }
    }
}

/// Translate some keys pressed when 'wildmenu' is used.
pub fn wildmenu_translate_key(
    cclp: &mut CmdlineInfo,
    key: c_int,
    xp: &mut expand_T,
    did_wild_list: bool,
) -> c_int {
    unsafe {
        let mut c = key;

        if cmdline_pum_active() || did_wild_list || g_st().wild_menu_showing != 0 {
            if c == K_LEFT {
                c = Ctrl_P;
            } else if c == K_RIGHT {
                c = Ctrl_N;
            }
        }

        if xp.xp_context == EXPAND_MENUNAMES
            && cclp.cmdpos > 1
            && *cclp.cmdbuff.add((cclp.cmdpos - 1) as usize) == b'.' as c_char
            && *cclp.cmdbuff.add((cclp.cmdpos - 2) as usize) != b'\\' as c_char
            && (c == b'\n' as c_int || c == b'\r' as c_int || c == K_KENTER)
        {
            c = K_DOWN;
        }

        c
    }
}

/// Delete characters on the command line, from "from" to the current position.
fn cmdline_del(cclp: &mut CmdlineInfo, from: c_int) {
    unsafe {
        debug_assert!(cclp.cmdpos <= cclp.cmdlen);
        ptr::copy(
            cclp.cmdbuff.add(cclp.cmdpos as usize),
            cclp.cmdbuff.add(from as usize),
            (cclp.cmdlen as usize) - (cclp.cmdpos as usize) + 1,
        );
        cclp.cmdlen -= cclp.cmdpos - from;
        cclp.cmdpos = from;
    }
}

/// Handle a key pressed when the wild menu for the menu names (EXPAND_MENUNAMES) is displayed.
fn wildmenu_process_key_menunames(
    cclp: &mut CmdlineInfo,
    mut key: c_int,
    xp: &mut expand_T,
) -> c_int {
    unsafe {
        if key == K_DOWN
            && cclp.cmdpos > 0
            && *cclp.cmdbuff.add((cclp.cmdpos - 1) as usize) == b'.' as c_char
        {
            key = opt_vars().p_wc as c_int;
            g_st().KeyTyped = true;
        } else if key == K_UP {
            let mut found = false;

            let mut j = xp.xp_pattern.offset_from(cclp.cmdbuff) as c_int;
            let mut i: c_int = 0;
            while {
                j -= 1;
                j > 0
            } {
                if *cclp.cmdbuff.add(j as usize) == b' ' as c_char
                    && *cclp.cmdbuff.add((j - 1) as usize) != b'\\' as c_char
                {
                    i = j + 1;
                    break;
                }
                if *cclp.cmdbuff.add(j as usize) == b'.' as c_char
                    && *cclp.cmdbuff.add((j - 1) as usize) != b'\\' as c_char
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
            key = opt_vars().p_wc as c_int;
            g_st().KeyTyped = true;
            xp.xp_context = EXPAND_NOTHING;
        }

        key
    }
}

/// Handle a key pressed when the wild menu for file/dir/shellcmd names is displayed.
fn wildmenu_process_key_filenames(
    cclp: &mut CmdlineInfo,
    mut key: c_int,
    xp: &mut expand_T,
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
            && cclp.cmdpos > 0
            && *cclp.cmdbuff.add((cclp.cmdpos - 1) as usize) == PATHSEP as c_char
            && (cclp.cmdpos < 3
                || *cclp.cmdbuff.add((cclp.cmdpos - 2) as usize) != b'.' as c_char
                || *cclp.cmdbuff.add((cclp.cmdpos - 3) as usize) != b'.' as c_char)
        {
            key = opt_vars().p_wc as c_int;
            g_st().KeyTyped = true;
        } else if (crate::memory::cstr_prefix_bytes(xp.xp_pattern, 3) == crate::memory::cstr_prefix_bytes(upseg.as_ptr().add(1), 3)) && key == K_DOWN {
            let mut found = false;

            let mut j = cclp.cmdpos;
            let i = xp.xp_pattern.offset_from(cclp.cmdbuff) as c_int;
            while {
                j -= 1;
                j > i
            } {
                j -= utf_head_off(cclp.cmdbuff, cclp.cmdbuff.add(j as usize));
                if vim_ispathsep(*cclp.cmdbuff.add(j as usize) as c_int) {
                    found = true;
                    break;
                }
            }
            if found
                && *cclp.cmdbuff.add((j - 1) as usize) == b'.' as c_char
                && *cclp.cmdbuff.add((j - 2) as usize) == b'.' as c_char
                && (vim_ispathsep(*cclp.cmdbuff.add((j - 3) as usize) as c_int) || j == i + 2)
            {
                cmdline_del(cclp, j - 2);
                key = opt_vars().p_wc as c_int;
                g_st().KeyTyped = true;
            }
        } else if key == K_UP {
            let mut found = false;

            let mut j = cclp.cmdpos - 1;
            let mut i = xp.xp_pattern.offset_from(cclp.cmdbuff) as c_int;
            while {
                j -= 1;
                j > i
            } {
                j -= utf_head_off(cclp.cmdbuff, cclp.cmdbuff.add(j as usize));
                #[cfg(windows)]
                let is_sep = vim_ispathsep(*cclp.cmdbuff.add(j as usize) as c_int)
                    && vim_strchr(
                        c" *?[{`$%#".as_ptr(),
                        *cclp.cmdbuff.add((j + 1) as usize) as u8 as c_int,
                    )
                    .is_null();
                #[cfg(not(windows))]
                let is_sep = vim_ispathsep(*cclp.cmdbuff.add(j as usize) as c_int);

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
            } else if crate::memory::cstr_prefix_bytes(cclp.cmdbuff.add(j as usize), 4) == crate::memory::cstr_prefix_bytes(upseg.as_ptr(), 4) {
                j += 4;
            } else if (crate::memory::cstr_prefix_bytes(cclp.cmdbuff.add(j as usize), 3) == crate::memory::cstr_prefix_bytes(upseg.as_ptr().add(1), 3))
                && j == i
            {
                j += 3;
            } else {
                j = 0;
            }

            if j > 0 {
                cmdline_del(cclp, j);
                put_on_cmdline(upseg.as_ptr().add(1), 3, false);
            } else if cclp.cmdpos > i {
                cmdline_del(cclp, i);
            }

            key = opt_vars().p_wc as c_int;
            g_st().KeyTyped = true;
        }

        key
    }
}

/// Handle a key pressed when wild menu is displayed.
pub fn wildmenu_process_key(cclp: &mut CmdlineInfo, key: c_int, xp: &mut expand_T) -> c_int {
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

        let skt = g_st().KeyTyped;
        let old_RedrawingDisabled = g_st().RedrawingDisabled;

        if cclp.input_fn != 0 {
            g_st().RedrawingDisabled = 0;
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
        g_st().KeyTyped = skt;
        if cclp.input_fn != 0 {
            g_st().RedrawingDisabled = old_RedrawingDisabled;
        }
}

/// "getcompletion()" function.
pub fn f_getcompletion(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let mut xpc: expand_T = expand_T::new();
        let mut filtered = false;
        let mut options =
            WILD_SILENT | WILD_USE_NL | WILD_ADD_SLASH | WILD_NO_BEEP | WILD_HOME_REPLACE;

        if !tv_check_for_string_arg(argvars, 1) {
            return;
        }
        let type_str = tv_get_string(&argvars[1]);

        if argvars[2].v_type != VarType::VAR_UNKNOWN {
            filtered = tv_get_bool_chk(&argvars[2], None) != 0;
        }

        if opt_vars().p_wic != 0 {
            options |= WILD_ICASE;
        }

        if !filtered {
            options |= WILD_KEEP_ALL;
        }

        if argvars[0].v_type != VarType::VAR_STRING {
            if !tv_get_string_chk(&argvars[0]).is_null() {
                emsg(gettext(e_invarg.as_ptr()));
            }
            return;
        }
        let pattern = tv_get_string(&argvars[0]);
        let mut pattern_start = pattern;

        if crate::memory::cstr_as_bytes(type_str) == b"cmdline" {
            let cmdline_len = crate::memory::cstr_len(pattern) as c_int;
            set_cmd_context(
                &mut xpc,
                pattern.cast_mut(),
                cmdline_len,
                cmdline_len,
                false as c_int,
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
                    semsg!(gettext(e_invarg2.as_ptr()), type_str);
                    return;
                }
                EXPAND_USER_DEFINED => {
                    if !(crate::memory::cstr_prefix_bytes(type_str, 7) == &b"custom,"[..7]) {
                        semsg!(gettext(e_invarg2.as_ptr()), type_str);
                        return;
                    }
                    xpc.xp_arg = type_str.add(7).cast_mut();
                }
                EXPAND_USER_LIST => {
                    if !(crate::memory::cstr_prefix_bytes(type_str, 11) == &b"customlist,"[..11]) {
                        semsg!(gettext(e_invarg2.as_ptr()), type_str);
                        return;
                    }
                    xpc.xp_arg = type_str.add(11).cast_mut();
                }
                EXPAND_MENUS => {
                    let pat_p = xpc.xp_pattern;
                    set_context_in_menu_cmd(&mut xpc, c"menu".as_ptr(), pat_p, false);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_SIGN => {
                    let pat_p = xpc.xp_pattern;
                    set_context_in_sign_cmd(&mut xpc, pat_p);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_RUNTIME => {
                    let pat_p = xpc.xp_pattern;
                    set_context_in_runtime_cmd(&mut xpc, pat_p);
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
                }
                EXPAND_SHELLCMDLINE => {
                    let mut context = EXPAND_SHELLCMDLINE;
                    let pat_p = xpc.xp_pattern;
                    set_context_for_wildcard_arg(
                        None,
                        pat_p,
                        false,
                        &mut xpc,
                        &mut context,
                    );
                    xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
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
            xpc.xp_col = crate::memory::cstr_len(xpc.xp_line) as c_int;
            nlua_expand_pat(&mut xpc);
            xpc.xp_pattern_len -= xpc.xp_pattern.offset_from(pattern_start) as usize;
        }
        let pat = if cmdline_fuzzy_completion_supported(&xpc) {
            cstr_from_bytes(std::slice::from_raw_parts(
                xpc.xp_pattern.cast::<u8>(),
                xpc.xp_pattern_len,
            ))
        } else {
            addstar(xpc.xp_pattern, xpc.xp_pattern_len, xpc.xp_context)
        };

        ExpandOne(
            &mut xpc,
            pat.as_ptr().cast_mut(),
            None,
            options,
            WILD_ALL_KEEP,
        );
        tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);

        for f in xpc.xp_files.iter().take(xpc.xp_numfiles.max(0) as usize) {
            tv_list_append_string(rettv.vval.v_list(), f.as_ptr(), f.as_bytes().len() as isize);
        }
    }
}

/// "getcompletiontype()" function.
pub fn f_getcompletiontype(argvars: &mut [typval_T], rettv: &mut typval_T,
    _fptr: EvalFuncData,){

     {
        rettv.v_type = VarType::VAR_STRING;
        rettv.vval = crate::eval::typval::TypvalVval::String(ptr::null_mut());

        if !tv_check_for_string_arg(argvars, 0) {
            return;
        }

        let pat = tv_get_string(&argvars[0]);
        let mut xpc: expand_T = expand_T::new();

        let cmdline_len = crate::memory::cstr_len(pat) as c_int;
        set_cmd_context(
            &mut xpc,
            pat.cast_mut(),
            cmdline_len,
            cmdline_len,
            false as c_int,
        );
        crate::eval::typval::tv_set_string(
            rettv,
            cmdcomplete_type_to_str(xpc.xp_context, xpc.xp_arg)
                .as_ref()
                .map_or(ptr::null(), |s| s.as_ptr()),
        );
    }
}

/// "cmdcomplete_info()" function.
pub fn f_cmdcomplete_info(_argvars: &mut [typval_T], rettv: &mut typval_T,
    _fptr: EvalFuncData,){
    unsafe {
        let ccline = get_cmdline_info();

        tv_dict_alloc_ret(rettv);
        if ccline.is_null() || (*ccline).xpc.is_null() || (*(*ccline).xpc).xp_files.capacity() == 0
        {
            return;
        }
        let orig_ptr = &raw const cmdexp().cmdline_orig;
        let orig_ref = (*orig_ptr).as_ref();

        let retdict = rettv.vval.v_dict();
        let mut ret = tv_dict_add_str_len(
            retdict,
            c"cmdline_orig".as_ptr(),
            12,
            orig_ref.map_or(ptr::null(), |v| v.as_ptr().cast::<c_char>()),
            orig_ref.map_or(0, |v| v.len() as c_int),
        );
        if ret {
            ret = tv_dict_add_nr(
                retdict,
                c"pum_visible".as_ptr(),
                11,
                i64::from(pum_visible()),
            );
        }
        if ret {
            ret = tv_dict_add_nr(
                retdict,
                c"selected".as_ptr(),
                8,
                i64::from((*(*ccline).xpc).xp_selected),
            );
        }
        if ret {
            let li = tv_list_alloc((*(*ccline).xpc).xp_numfiles as isize);
            ret = tv_dict_add_list(retdict, c"matches".as_ptr(), 7, li);
            let files = (*(*ccline).xpc).xp_files.as_slice();
            let mut idx: usize = 0;
            while ret && (idx as c_int) < (*(*ccline).xpc).xp_numfiles {
                tv_list_append_string(
                    li,
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
    start: &mut pos_T,
    end: &mut pos_T,
    match_end: &mut pos_T,
) -> Option<CString> {
    unsafe {
        let exacttext = (opt_vars().wop_flags & kOptWopFlagExacttext) != 0;

        if start.lnum > end.lnum
            || (start.lnum == end.lnum && start.col >= end.col)
        {
            return None;
        }

        let mut ga: Vec<u8> = Vec::with_capacity(128);

        let start_line = ml_get(start.lnum);
        let start_ptr = start_line.add(start.col as usize);
        let is_single_line = start.lnum == end.lnum;

        let mut segment_len = if is_single_line {
            end.col - start.col
        } else {
            ml_get_len(start.lnum) - start.col
        };
        ga.extend_from_slice(std::slice::from_raw_parts(
            start_ptr.cast::<u8>(),
            segment_len as usize,
        ));
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
                ga.extend_from_slice(std::slice::from_raw_parts(
                    line.cast::<u8>(),
                    linelen as usize,
                ));
                if exacttext {
                    ga.extend_from_slice(b"\\n");
                } else {
                    ga.push(b'\n');
                }
                lnum += 1;
            }
        }

        let end_line = ml_get(end.lnum);
        let word_end = find_word_end(end_line.add(end.col as usize));
        segment_len = word_end.offset_from(end_line) as c_int;
        let start_off = if is_single_line { end.col } else { 0 };
        ga.extend_from_slice(std::slice::from_raw_parts(
            end_line.add(start_off as usize).cast::<u8>(),
            (segment_len - start_off) as usize,
        ));

        match_end.lnum = end.lnum;
        match_end.col = segment_len;

        Some(cstr_from_bytes(&ga))
    }
}

/// Returns true if the given string `str` matches the regex pattern `pat`.
fn is_regex_match(pat: *const c_char, str_arg: *const c_char) -> bool {
     {
        if crate::memory::cstr_as_bytes(pat) == crate::memory::cstr_as_bytes(str_arg) {
            return true;
        }

        let mut regmatch: regmatch_T = regmatch_T::new();

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
    pat: *mut c_char,
    pat_len: c_int,
    end_match_pos: &mut pos_T,
    lowercase: bool,
) -> CString {
    unsafe {
        let line = ml_get(end_match_pos.lnum);
        let word_end = find_word_end(line.add(end_match_pos.col as usize));
        let match_len = word_end.offset_from(line.add(end_match_pos.col as usize)) as c_int;
        let mut out = Vec::with_capacity((match_len as usize) + (pat_len as usize));
        out.extend_from_slice(std::slice::from_raw_parts(
            pat.cast::<u8>(),
            pat_len as usize,
        ));
        if match_len > 0 {
            let match_slice = std::slice::from_raw_parts(
                line.add(end_match_pos.col as usize).cast::<u8>(),
                match_len as usize,
            );
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
    pat: *mut c_char,
    dir: Direction,
    matches: &mut Vec<CString>,
) -> bool {
    unsafe {
        let exacttext = (opt_vars().wop_flags & kOptWopFlagExacttext) != 0;
        let has_range = g_st().Search.first_line != 0;

        matches.clear();

        if pat.is_null() || *pat == NUL {
            return false;
        }

        let pat_len = crate::memory::cstr_len(pat) as c_int;
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
            cur_match_pos.lnum = g_st().Search.first_line;
        } else {
            cur_match_pos = cmdexp().pre_incsearch_pos;
        }

        let search_flags = SEARCH_OPT
            | SEARCH_NOOF
            | SEARCH_PEEK
            | SEARCH_NFMSG
            | if has_range { SEARCH_START } else { 0 };

        let mut end_match_pos: pos_T = pos_T::new();
        let mut word_end_pos: pos_T = pos_T::new();
        let mut looped_around = false;
        let mut compl_started = false;

        loop {
            g_st().emsg_off += 1;
            g_st().msg_silent += 1;
            let found_new_match = searchit(
                ptr::null_mut(),
                g_st().curbuf,
                &raw mut cur_match_pos,
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
                && (cur_match_pos.lnum < g_st().Search.first_line || cur_match_pos.lnum > g_st().Search.last_line)
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

            if end_match_pos.lnum > (*g_st().curbuf).b_ml.ml_line_count {
                cur_match_pos.lnum = 1;
                cur_match_pos.col = 0;
                cur_match_pos.coladd = 0;
                continue;
            }

            let Some(full_match) = copy_substring_from_pos(
                &mut cur_match_pos,
                &mut end_match_pos,
                &mut word_end_pos,
            ) else {
                break;
            };

            let match_cstr = if exacttext {
                full_match
            } else {
                let mut m =
                    concat_pattern_with_buffer_match(pat, pat_len, &mut end_match_pos, false);

                if !is_regex_match(m.as_ptr(), full_match.as_ptr()) {
                    m = concat_pattern_with_buffer_match(
                        pat,
                        pat_len,
                        &mut end_match_pos,
                        true,
                    );
                    if !is_regex_match(m.as_ptr(), full_match.as_ptr()) {
                        continue;
                    }
                }
                m
            };

            if !matches.contains(&match_cstr) {
                matches.push(match_cstr);
                if matches.len() as c_int > TAG_MANY {
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
        let mut xp: expand_T = expand_T::new();
            ExpandInit(&mut xp);
            assert_eq!(xp.xp_context, EXPAND_NOTHING);
            assert_eq!(xp.xp_backslash, XP_BS_NONE);
            ExpandCleanup(&mut xp);
            assert!(!cmdline_pum_active());
    }

    #[test]
    fn cmdidx_from_raw_roundtrips_all_variants() {
        for (i, &c) in cmdidx_T::ALL.iter().enumerate() {
            assert_eq!(c as c_int, i as c_int);
            assert!(cmdidx_T::from_raw(i as c_int) == c);
        }
        assert!(cmdidx_T::from_raw(-1) == cmdidx_T::CMD_USER);
        assert!(cmdidx_T::from_raw(-2) == cmdidx_T::CMD_USER_BUF);
        assert!(cmdidx_T::from_raw(cmdidx_T::CMD_SIZE as c_int) == cmdidx_T::CMD_SIZE);
    }
}
