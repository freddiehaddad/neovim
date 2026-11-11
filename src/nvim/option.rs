#![forbid(unsafe_code)]

//! User-settable options (`src/nvim/option.c`).

use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    api_free_string, api_set_error, arena_dict, boolean_obj, copy_object, cstr_as_obj,
    cstr_as_string, cstr_to_obj, dict_obj, error_set, integer_obj, luaref_obj, nil_obj, put_c,
    string_obj,
};
use crate::api::private::validate::{
    Dict, Error, ErrorType, NvimString, Object, ObjectData, ObjectType,
};
use crate::autocmd::{
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_OPTIONSET, EVENT_SYNTAX, K_OPT_MODIFIED, OPT_NOWIN,
    apply_autocmds, aucmd_defer_modified, do_filetype_autocmd,
};
use crate::buffer::{
    BCO_ALWAYS, FileBuf, KEYMAP_INIT, NO_LOCAL_UNDOLEVEL, OPT_MODELINE, bt_prompt, buf_is_empty,
    do_autochdir, free_buf_options, maketitle,
};
use crate::change::{HLF_W, save_file_ff};
#[cfg(windows)]
use crate::charset::vim_isfilec;
use crate::charset::{
    EOL_MAC, ascii_isalnum, ascii_iswhite, buf_init_isk_chartab, init_isf_chartab,
    init_isi_chartab, is_special, trans_characters, transchar, vim_str2nr, vim_strsize,
};
use crate::cmdexpand::{
    Cmdidx, EXPAND_BOOL_SETTINGS, EXPAND_DIRECTORIES, EXPAND_FILES, EXPAND_FILETYPE, EXPAND_KEYMAP,
    EXPAND_NOTHING, EXPAND_OLD_SETTING, EXPAND_OWNSYNTAX, EXPAND_SETTING_SUBTRACT, EXPAND_SETTINGS,
    EXPAND_STRING_SETTING, EXPAND_UNSUCCESSFUL, XP_BS_COMMA, XP_BS_ONE, XP_BS_THREE, XP_PREFIX_INV,
    XP_PREFIX_NO, cmdline_fuzzy_complete,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor_shape::{SHAPE_CURSOR, parse_shape_opt};
use crate::decoration_provider::with_decor_provider_mut;
use crate::diff::{K_OPT_DIFF, K_OPT_FOLDMETHOD, diff_buf_adjust};
use crate::drawscreen::{
    UPD_CLEAR, UPD_NOT_VALID, UPD_SOME_VALID, check_screensize, comp_col, redraw_all_later,
    redraw_buf_later, redraw_buf_status_later, redraw_later, screen_resize, showmode,
    status_redraw_all, status_redraw_curbuf,
};
use crate::errors::*;
use crate::eval::encode::encode_tv2string;
use crate::eval::typval::{
    Callback, TypVal, callback_copy, callback_free, callback_put, tv_clear, tv_dict_add_tv,
    tv_dict_alloc,
};
use crate::eval::userfunc::get_scriptlocal_funcname;
use crate::eval::vars::{
    K_OPT_FLAG_FUNC, Scid, VV_OPTION_COMMAND, VV_OPTION_NEW, VV_OPTION_OLD, VV_OPTION_OLDGLOBAL,
    VV_OPTION_OLDLOCAL, VV_OPTION_TYPE, VV_WARNINGMSG, get_vim_var_str, opt_to_tv,
    reset_v_option_vars, set_vim_var_string, set_vim_var_tv, tristate_from_int,
};
use crate::eval::{STR2NR_ALL, callback_from_typval, eval_expr, last_set_msg};
use crate::ex_docmd::{
    CMOD_NOSWAPFILE, Exarg, K_OPT_FINDFUNC, UNDO_HASH_SIZE, free_findfunc_option, set_no_hlsearch,
};
use crate::ex_getln::{
    B_IMODE_NONE, B_IMODE_USE_INSERT, check_opt_wim, did_set_cedit, gotocmdline,
};
use crate::ex_session::{OPT_SKIPRTP, put_eol, put_line};
#[cfg(unix)]
use crate::fileio::getuid;
use crate::fileio::{EOL_DOS, EOL_UNIX, K_OPT_FILEENCODING, fprintf};
use crate::fold::{
    fold_update_all, foldmethod_is_diff, foldmethod_is_indent, foldmethod_is_syntax, new_fold_level,
};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, FuzmatchStr, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::highlight::{HL_GLOBAL, hl_invalidate_blends, ns_hl_def};
use crate::highlight_group::{highlight_changed, syn_check_group};
use crate::indent::{
    K_BUF_OPT_INDENTEXPR, K_OPT_BREAKINDENTOPT, K_OPT_INDENTEXPR, K_OPT_TABSTOP, K_OPT_VARTABSTOP,
    briopt_check, tabstop_set,
};
use crate::indent_c::parse_cino;
use crate::input_cmdatom::atom_follow_changed;
use crate::insexpand::{K_OPT_COMPLETE, set_buflocal_cpt_callbacks};
use crate::keycodes::{
    FSK_KEEP_X_KEY, FSK_KEYCODE, FSK_SIMPLIFY, K_KENTER, K_ZERO, find_special_key,
    find_special_key_in_table, get_special_key_code, get_special_key_name, termcap2key,
};
use crate::log::logmsg;

use crate::highlight::HlAttrs;
use crate::lua::executor::{api_free_luaref, api_new_luaref, nlua_funcref_str, nlua_set_sctx};
use crate::mapping::{langmap_init, put_escstr};
use crate::mbyte::enc_locale;
use crate::mcursor::{K_OPT_FOLLOW, mc_ins_replay_can_join};
use crate::memfile::mf_close_file;
use crate::memline::{ml_open_file, ml_open_files};
use crate::memory::Arena;
use crate::message::{
    emsg, iemsg, message_filtered, msg, msg_advance, msg_ext_set_kind, msg_outtrans, msg_putchar,
    msg_puts, msg_puts_title, msg_source,
};
use crate::mouse::setmouse;
use crate::r#move::changed_window_setting;
use crate::normal::{K_OPT_FOLDENABLE, K_OPT_FOLDLEVEL, do_check_scrollbind, get_vtopline};
use crate::ops::free_operatorfunc_option;
use crate::option_vars::*;
use crate::optionstr::{
    CharsOption, OptIndex, OptScopeFlags, OptVar, Optexpand, Optset, Vimoption, check_buf_options,
    check_illegal_path_names, check_signcolumn, did_set_breakat, didset_string_options,
    opt_dict_schema, opt_strings_check, set_chars_option,
};
use crate::os::env::{
    ExpandCtx, expand_env_esc, home_replace, os_env_exists, os_getenv_cstr, os_setenv, vim_getenv,
};
use crate::os::input::os_breakcheck;
use crate::os::lang::{get_mess_lang, lang_init};
use crate::os::stdpaths::{stdpaths_user_conf_subpath, stdpaths_user_state_subpath};
use crate::path::{
    FullName_save, after_pathsep, invocation_path_tail, path_tail, vim_ispathlistsep,
};
use crate::popupmenu::{pum_drawn, pum_redraw};
use crate::quickfix::{ll_resize_stack, qf_resize_stack};
use crate::regexp::{MAXCOL, NUMBUFLEN, Regmatch, vim_regexec};
use crate::runtime::{
    DIP_ALL, SID_NONE, Sctx, runtimepath_default, source_runtime_vim_lua, sourcing_lnum,
};
use crate::spell::{compile_cap_prog, did_set_spell_option, init_spell_chartab, parse_spelllang};
use crate::spellsuggest::{spell_apply_sps, spell_check_sps};
use crate::strings::{format_to_buf, format_to_buf_safelen, gettext, tr, vim_strsave_escaped};
use crate::tag::free_tagfunc_option;
use crate::terminal::on_scrollback_option_changed;
use crate::ui::ui_call_option_set;
use crate::undo::{buf_is_changed_fn, curbuf_is_changed_fn, u_compute_hash, u_read_undo, u_sync};
use crate::window::{
    MIN_COLUMNS, STATUS_HEIGHT, WinInfoInfo, Winopt, check_colorcolumn, command_height,
    frame_new_height, global_stl_height, last_status, min_rows, min_rows_for_all_tabpages,
    set_winbar, set_winbar_all, set_winbar_win, tabline_height, unuse_tabpage, use_tabpage,
    win_comp_pos, win_comp_scroll, win_default_scroll, win_equal, win_find_tabpage,
    win_new_screen_rows, win_setheight, win_setwidth,
};
use crate::winfloat::win_float_update_statusline;
use std::ffi::CStr;
use std::ptr;

pub type OptInt = i64;
pub type OptScope = i32;
pub type SetOp = i32;
pub type SetPrefix = i32;

pub(crate) const OPT_GLOBAL: i32 = 0x01;
pub(crate) const OPT_LOCAL: i32 = 0x02;
pub(crate) const PREFIX_NONE: SetPrefix = 1;
pub(crate) const K_OPT_SCOPE_GLOBAL: OptScope = 0;
pub(crate) const K_OPT_SCOPE_WIN: OptScope = 1;
pub(crate) const K_OPT_SCOPE_BUF: OptScope = 2;
pub(crate) const K_OPT_SCOPE_TAB: OptScope = 3;
pub(crate) const K_OPT_INVALID: OptIndex = -1;
pub(crate) const OP_NONE: SetOp = 0;
pub(crate) const OP_ADDING: SetOp = 1;
pub(crate) const OP_PREPENDING: SetOp = 2;
pub(crate) const OP_REMOVING: SetOp = 3;
pub(crate) const K_OPT_BUFHIDDEN: OptIndex = 27;
pub(crate) const K_OPT_BUFTYPE: OptIndex = 29;
pub(crate) const K_OPT_FILETYPE: OptIndex = 97;
pub(crate) const K_OPT_MODELINE: OptIndex = 192;
pub(crate) const K_OPT_SWAPFILE: OptIndex = 301;

use crate::api::ui::NO_SCREEN;
use crate::api::vim::ERR_BUFLEN;
use crate::api::win_config::K_CTX_NO_DISPLAY;
use crate::buffer::{BCO_ENTER, BCO_NOHELP};
use crate::drawline::{
    K_OPT_CULOPT_FLAG_LINE, K_OPT_CULOPT_FLAG_NUMBER, K_OPT_CULOPT_FLAG_SCREENLINE,
    K_OPT_FLAG_INSECURE,
};
use crate::eval::typval::{
    K_CALLBACK_EXPR, K_CALLBACK_FUNCREF, K_CALLBACK_LUA, K_CALLBACK_NONE_CONST, K_CALLBACK_PARTIAL,
};
use crate::ex_cmds::{K_OPT_FOLDCOLUMN, OPT_WINONLY};
use crate::ex_cmds2::BF_SYN_SET;
use crate::extmark::{K_FALSE, K_NONE, K_TRUE, TriState};
use crate::file_search::{K_BUF_OPT_INCLUDEEXPR, K_OPT_INCLUDEEXPR};
use crate::help::K_OPT_ISKEYWORD;
use crate::insert::{BS_EOL, BS_NOSTOP, BS_START};
use crate::os::fs::MAXPATHL;
use crate::state::MODE_TERMINAL;

const OPT_ONECOLUMN: i32 = 0x20;

const PREFIX_NO: SetPrefix = 0;
const PREFIX_INV: SetPrefix = 2;

const K_OPT_SCOPE_SIZE: OptScope = 4;

const K_OPT_FLAG_EXPAND: u32 = 1 << 0;
const K_OPT_FLAG_NO_DEF_EXP: u32 = 1 << 1;
const K_OPT_FLAG_NO_DEFAULT: u32 = 1 << 2;
const K_OPT_FLAG_WAS_SET: u32 = 1 << 3;
const K_OPT_FLAG_NO_MKRC: u32 = 1 << 4;
const K_OPT_FLAG_UIOPTION: u32 = 1 << 5;
const K_OPT_FLAG_REDR_TABL: u32 = 1 << 6;
const K_OPT_FLAG_REDR_STAT: u32 = 1 << 7;
const K_OPT_FLAG_REDR_WIN: u32 = 1 << 8;
const K_OPT_FLAG_REDR_BUF: u32 = 1 << 9;
const K_OPT_FLAG_REDR_ALL: u32 = K_OPT_FLAG_REDR_BUF | K_OPT_FLAG_REDR_WIN;
pub const K_OPT_FLAG_COMMA: u32 = 1 << 10;
pub(crate) const K_OPT_FLAG_ONE_COMMA: u32 = (1 << 11) | K_OPT_FLAG_COMMA;
const K_OPT_FLAG_NO_DUP: u32 = 1 << 12;
pub const K_OPT_FLAG_FLAG_LIST: u32 = 1 << 13;
const K_OPT_FLAG_SECURE: u32 = 1 << 14;
const K_OPT_FLAG_GETTEXT: u32 = 1 << 15;
const K_OPT_FLAG_NO_GLOB: u32 = 1 << 16;
pub(crate) const K_OPT_FLAG_NFNAME: u32 = 1 << 17;
const K_OPT_FLAG_PRI_MKRC: u32 = 1 << 19;
const K_OPT_FLAG_CURSWANT: u32 = 1 << 20;
pub(crate) const K_OPT_FLAG_NDNAME: u32 = 1 << 21;
const K_OPT_FLAG_HLONLY: u32 = 1 << 22;
const K_OPT_FLAG_MLE: u32 = 1 << 23;
pub const K_OPT_FLAG_COLON: u32 = 1 << 25;
const K_OPT_FLAG_EXPR: u32 = 1 << 26;

pub(crate) const K_OPT_ARABIC: OptIndex = 3;
const K_OPT_AUTOCOMPLETE: OptIndex = 6;
const K_OPT_AUTOINDENT: OptIndex = 9;
const K_OPT_AUTOREAD: OptIndex = 10;
pub(crate) const K_OPT_BACKUPCOPY: OptIndex = 16;
const K_OPT_BACKUPDIR: OptIndex = 17;
const K_OPT_BACKUPSKIP: OptIndex = 19;
const K_OPT_BINARY: OptIndex = 21;
const K_OPT_BOMB: OptIndex = 22;
const K_OPT_BREAKINDENT: OptIndex = 24;
pub(crate) const K_OPT_BUFLISTED: OptIndex = 28;
const K_OPT_BUSY: OptIndex = 30;
const K_OPT_CDPATH: OptIndex = 33;
const K_OPT_CHANNEL: OptIndex = 35;
const K_OPT_CHISTORY: OptIndex = 37;
const K_OPT_CINDENT: OptIndex = 38;
const K_OPT_CINKEYS: OptIndex = 39;
const K_OPT_CINOPTIONS: OptIndex = 40;
const K_OPT_CINSCOPEDECLS: OptIndex = 41;
const K_OPT_CINWORDS: OptIndex = 42;
pub(crate) const K_OPT_CMDHEIGHT: OptIndex = 44;
const K_OPT_CMDWINHEIGHT: OptIndex = 45;
const K_OPT_COLORCOLUMN: OptIndex = 46;
const K_OPT_COLUMNS: OptIndex = 47;
pub(crate) const K_OPT_COMMENTS: OptIndex = 48;
const K_OPT_COMMENTSTRING: OptIndex = 49;
const K_OPT_COMPLETEFUNC: OptIndex = 52;
pub(crate) const K_OPT_COMPLETEOPT: OptIndex = 54;
#[cfg(windows)]
const K_OPT_COMPLETESLASH: OptIndex = 55;
const K_OPT_CONCEALCURSOR: OptIndex = 57;
const K_OPT_CONCEALLEVEL: OptIndex = 58;
const K_OPT_COPYINDENT: OptIndex = 60;
const K_OPT_CURSORBIND: OptIndex = 62;
const K_OPT_CURSORCOLUMN: OptIndex = 63;
const K_OPT_CURSORLINE: OptIndex = 64;
const K_OPT_CURSORLINEOPT: OptIndex = 65;
const K_OPT_DEFINE: OptIndex = 67;
const K_OPT_DICTIONARY: OptIndex = 69;
const K_OPT_DIFFANCHORS: OptIndex = 71;
const K_OPT_DIRECTORY: OptIndex = 75;
const K_OPT_ENDOFFILE: OptIndex = 81;
const K_OPT_ENDOFLINE: OptIndex = 82;
const K_OPT_EQUALPRG: OptIndex = 84;
const K_OPT_ERRORFORMAT: OptIndex = 87;
pub(crate) const K_OPT_EVENTIGNOREWIN: OptIndex = 89;
const K_OPT_EXPANDTAB: OptIndex = 90;
pub(crate) const K_OPT_FILEFORMAT: OptIndex = 94;
pub(crate) const K_OPT_FILEFORMATS: OptIndex = 95;
const K_OPT_FILLCHARS: OptIndex = 98;
const K_OPT_FIXENDOFLINE: OptIndex = 100;
pub(crate) const K_OPT_FOLDEXPR: OptIndex = 104;
const K_OPT_FOLDIGNORE: OptIndex = 105;
const K_OPT_FOLDMARKER: OptIndex = 108;
const K_OPT_FOLDMINLINES: OptIndex = 110;
const K_OPT_FOLDNESTMAX: OptIndex = 111;
pub(crate) const K_OPT_FOLDTEXT: OptIndex = 113;
pub(crate) const K_OPT_FORMATEXPR: OptIndex = 115;
const K_OPT_FORMATLISTPAT: OptIndex = 116;
const K_OPT_FORMATOPTIONS: OptIndex = 117;
const K_OPT_FORMATPRG: OptIndex = 118;
const K_OPT_FSYNC: OptIndex = 119;
const K_OPT_GREPFORMAT: OptIndex = 121;
const K_OPT_GREPPRG: OptIndex = 122;
const K_OPT_HELPHEIGHT: OptIndex = 130;
const K_OPT_HELPLANG: OptIndex = 131;
const K_OPT_HISTORY: OptIndex = 134;
const K_OPT_ICON: OptIndex = 138;
const K_OPT_IMINSERT: OptIndex = 143;
const K_OPT_IMSEARCH: OptIndex = 144;
const K_OPT_INCLUDE: OptIndex = 146;
const K_OPT_INDENTKEYS: OptIndex = 150;
const K_OPT_INFERCASE: OptIndex = 151;
pub(crate) const K_OPT_KEYMAP: OptIndex = 159;
const K_OPT_KEYWORDPRG: OptIndex = 161;
const K_OPT_LHISTORY: OptIndex = 168;
const K_OPT_LINEBREAK: OptIndex = 169;
const K_OPT_LINES: OptIndex = 170;
const K_OPT_LISP: OptIndex = 172;
const K_OPT_LISPOPTIONS: OptIndex = 173;
const K_OPT_LISPWORDS: OptIndex = 174;
const K_OPT_LIST: OptIndex = 175;
pub(crate) const K_OPT_LISTCHARS: OptIndex = 176;
const K_OPT_MAKEENCODING: OptIndex = 180;
const K_OPT_MAKEPRG: OptIndex = 181;
const K_OPT_MATCHPAIRS: OptIndex = 182;
const K_OPT_MAXCOMBINE: OptIndex = 184;
const K_OPT_MAXSEARCHCOUNT: OptIndex = 188;
const K_OPT_MODIFIABLE: OptIndex = 195;
const K_OPT_NRFORMATS: OptIndex = 206;
const K_OPT_NUMBER: OptIndex = 207;
const K_OPT_NUMBERWIDTH: OptIndex = 208;
const K_OPT_OMNIFUNC: OptIndex = 209;
const K_OPT_PACKLOCKFILE: OptIndex = 212;
const K_OPT_PACKPATH: OptIndex = 213;
const K_OPT_PATH: OptIndex = 219;
const K_OPT_PRESERVEINDENT: OptIndex = 220;
const K_OPT_PREVIEWWINDOW: OptIndex = 223;
const K_OPT_PUMBLEND: OptIndex = 225;
const K_OPT_PYXVERSION: OptIndex = 230;
const K_OPT_QUOTEESCAPE: OptIndex = 232;
const K_OPT_READONLY: OptIndex = 233;
const K_OPT_REGEXPENGINE: OptIndex = 236;
const K_OPT_RELATIVENUMBER: OptIndex = 237;
const K_OPT_REPORT: OptIndex = 239;
const K_OPT_REVINS: OptIndex = 240;
pub(crate) const K_OPT_RIGHTLEFT: OptIndex = 241;
const K_OPT_RIGHTLEFTCMD: OptIndex = 242;
const K_OPT_RULER: OptIndex = 243;
pub(crate) const K_OPT_RUNTIMEPATH: OptIndex = 245;
const K_OPT_SCROLL: OptIndex = 246;
const K_OPT_SCROLLBACK: OptIndex = 247;
const K_OPT_SCROLLBIND: OptIndex = 248;
const K_OPT_SCROLLJUMP: OptIndex = 249;
const K_OPT_SCROLLOFF: OptIndex = 250;
const K_OPT_SCROLLOFFPAD: OptIndex = 251;
const K_OPT_SHELL: OptIndex = 260;
const K_OPT_SHELLCMDFLAG: OptIndex = 261;
const K_OPT_SHELLPIPE: OptIndex = 262;
const K_OPT_SHELLREDIR: OptIndex = 264;
#[cfg(windows)]
const K_OPT_SHELLSLASH: OptIndex = 265;
const K_OPT_SHELLXQUOTE: OptIndex = 268;
const K_OPT_SHIFTWIDTH: OptIndex = 270;
const K_OPT_SHOWBREAK: OptIndex = 272;
const K_OPT_SHOWMATCH: OptIndex = 276;
const K_OPT_SIDESCROLL: OptIndex = 279;
const K_OPT_SIDESCROLLOFF: OptIndex = 280;
const K_OPT_SIGNCOLUMN: OptIndex = 281;
const K_OPT_SMARTINDENT: OptIndex = 283;
const K_OPT_SMARTTAB: OptIndex = 284;
const K_OPT_SMOOTHSCROLL: OptIndex = 285;
const K_OPT_SOFTTABSTOP: OptIndex = 286;
pub(crate) const K_OPT_SPELL: OptIndex = 287;
const K_OPT_SPELLCAPCHECK: OptIndex = 288;
pub(crate) const K_OPT_SPELLFILE: OptIndex = 289;
pub(crate) const K_OPT_SPELLLANG: OptIndex = 290;
const K_OPT_SPELLOPTIONS: OptIndex = 291;
pub(crate) const K_OPT_STATUSCOLUMN: OptIndex = 297;
pub(crate) const K_OPT_STATUSLINE: OptIndex = 298;
const K_OPT_SUFFIXESADD: OptIndex = 300;
const K_OPT_SYNMAXCOL: OptIndex = 303;
const K_OPT_SYNTAX: OptIndex = 304;
pub(crate) const K_OPT_TAGCASE: OptIndex = 310;
const K_OPT_TAGFUNC: OptIndex = 311;
const K_OPT_TAGS: OptIndex = 314;
const K_OPT_TERMBIDI: OptIndex = 316;
const K_OPT_TEXTWIDTH: OptIndex = 322;
const K_OPT_THESAURUS: OptIndex = 323;
const K_OPT_THESAURUSFUNC: OptIndex = 324;
const K_OPT_TIMEOUTLEN: OptIndex = 327;
const K_OPT_TITLE: OptIndex = 328;
const K_OPT_TITLELEN: OptIndex = 329;
const K_OPT_TTYFAST: OptIndex = 334;
const K_OPT_UNDODIR: OptIndex = 335;
const K_OPT_UNDOFILE: OptIndex = 336;
const K_OPT_UNDOLEVELS: OptIndex = 337;
const K_OPT_UPDATECOUNT: OptIndex = 339;
const K_OPT_UPDATETIME: OptIndex = 340;
pub(crate) const K_OPT_VARSOFTTABSTOP: OptIndex = 341;
const K_OPT_VIEWDIR: OptIndex = 345;
pub(crate) const K_OPT_VIRTUALEDIT: OptIndex = 347;
const K_OPT_WILDCHAR: OptIndex = 351;
const K_OPT_WILDCHARM: OptIndex = 352;
pub(crate) const K_OPT_WINBAR: OptIndex = 359;
const K_OPT_WINBLEND: OptIndex = 360;
pub(crate) const K_OPT_WINDOW: OptIndex = 362;
const K_OPT_WINFIXBUF: OptIndex = 363;
const K_OPT_WINFIXHEIGHT: OptIndex = 364;
const K_OPT_WINFIXWIDTH: OptIndex = 365;
const K_OPT_WINHEIGHT: OptIndex = 366;
const K_OPT_WINHIGHLIGHT: OptIndex = 367;
const K_OPT_WINMINHEIGHT: OptIndex = 368;
const K_OPT_WINMINWIDTH: OptIndex = 369;
const K_OPT_WINPINNED: OptIndex = 370;
const K_OPT_WINWIDTH: OptIndex = 371;
const K_OPT_WRAP: OptIndex = 372;
const K_OPT_WRAPMARGIN: OptIndex = 373;
const K_OPT_WRITEDELAY: OptIndex = 378;
const K_OPT_COUNT: OptIndex = 379;

const K_BUF_OPT_AUTOINDENT: usize = 1;
const K_BUF_OPT_BINARY: usize = 4;
const K_BUF_OPT_BOMB: usize = 5;
const K_BUF_OPT_CINDENT: usize = 11;
const K_BUF_OPT_CINKEYS: usize = 12;
const K_BUF_OPT_CINOPTIONS: usize = 13;
const K_BUF_OPT_CINSCOPEDECLS: usize = 14;
const K_BUF_OPT_CINWORDS: usize = 15;
const K_BUF_OPT_COMMENTS: usize = 16;
const K_BUF_OPT_COMMENTSTRING: usize = 17;
const K_BUF_OPT_COMPLETE: usize = 18;
const K_BUF_OPT_COMPLETEFUNC: usize = 19;
#[cfg(windows)]
const K_BUF_OPT_COMPLETESLASH: usize = 21;
const K_BUF_OPT_COPYINDENT: usize = 22;
const K_BUF_OPT_EXPANDTAB: usize = 30;
const K_BUF_OPT_FIXENDOFLINE: usize = 35;
const K_BUF_OPT_FOLLOW: usize = 36;
pub(crate) const K_BUF_OPT_FORMATEXPR: usize = 37;
const K_BUF_OPT_FORMATLISTPAT: usize = 38;
const K_BUF_OPT_FORMATOPTIONS: usize = 39;
const K_BUF_OPT_IMINSERT: usize = 44;
const K_BUF_OPT_IMSEARCH: usize = 45;
const K_BUF_OPT_INDENTKEYS: usize = 49;
const K_BUF_OPT_INFERCASE: usize = 50;
const K_BUF_OPT_ISKEYWORD: usize = 51;
const K_BUF_OPT_KEYMAP: usize = 52;
const K_BUF_OPT_LISP: usize = 54;
const K_BUF_OPT_LISPOPTIONS: usize = 55;
const K_BUF_OPT_MATCHPAIRS: usize = 59;
const K_BUF_OPT_MODELINE: usize = 60;
const K_BUF_OPT_MODIFIABLE: usize = 61;
const K_BUF_OPT_NRFORMATS: usize = 63;
const K_BUF_OPT_OMNIFUNC: usize = 64;
const K_BUF_OPT_PRESERVEINDENT: usize = 66;
const K_BUF_OPT_QUOTEESCAPE: usize = 67;
const K_BUF_OPT_SCROLLBACK: usize = 69;
const K_BUF_OPT_SHIFTWIDTH: usize = 70;
const K_BUF_OPT_SMARTINDENT: usize = 71;
const K_BUF_OPT_SOFTTABSTOP: usize = 72;
const K_BUF_OPT_SPELLCAPCHECK: usize = 73;
const K_BUF_OPT_SPELLFILE: usize = 74;
const K_BUF_OPT_SPELLLANG: usize = 75;
const K_BUF_OPT_SPELLOPTIONS: usize = 76;
const K_BUF_OPT_SUFFIXESADD: usize = 77;
const K_BUF_OPT_SWAPFILE: usize = 78;
const K_BUF_OPT_SYNMAXCOL: usize = 79;
const K_BUF_OPT_TABSTOP: usize = 81;
const K_BUF_OPT_TAGFUNC: usize = 83;
const K_BUF_OPT_TEXTWIDTH: usize = 85;
const K_BUF_OPT_UNDOFILE: usize = 88;
const K_BUF_OPT_VARSOFTTABSTOP: usize = 90;
const K_BUF_OPT_VARTABSTOP: usize = 91;
const K_BUF_OPT_WRAPMARGIN: usize = 92;

const K_CPO_BUFOPT: i32 = b's' as i32;
const K_CPO_BUFOPTGLOB: i32 = b'S' as i32;

const K_OPT_VE_FLAG_NONE: u32 = 0x10;
const K_OPT_VE_FLAG_NONE_U: u32 = 0x20;

pub(crate) const SB_MAX: OptInt = 1_000_000;
const MAX_NUMBERWIDTH: OptInt = 20;
const MAX_MCO: OptInt = 6;

const B_IMODE_LAST: OptInt = 1;

const CTRL_C_CONST_CONST: OptInt = 3;

#[cfg(unix)]
pub(crate) const ROOT_UID: u32 = 0;

const PATHSEPSTR: &CStr = c"/";

const SHM_ALL_ABBREVIATIONS: &CStr = c"rmlw";

const E_UNKNOWN_OPTION: &CStr = c"E518: Unknown option";
const E_NOT_ALLOWED_IN_MODELINE: &CStr = c"E520: Not allowed in a modeline";
const E_NOT_ALLOWED_IN_MODELINE_WHEN_MODELINEEXPR_IS_OFF: &CStr =
    c"E992: Not allowed in a modeline when 'modelineexpr' is off";
const E_NUMBER_REQUIRED_AFTER_EQUAL: &CStr = c"E521: Number required after =";
const E_PREVIEW_WINDOW_ALREADY_EXISTS: &CStr = c"E590: A preview window already exists";
const E_CANNOT_HAVE_NEGATIVE_OR_ZERO_NUMBER_OF_QUICKFIX: &CStr =
    c"E1542: Cannot have a negative or zero number of quickfix/location lists";
const E_CANNOT_HAVE_MORE_THAN_HUNDRED_QUICKFIX: &CStr =
    c"E1543: Cannot have more than a hundred quickfix/location lists";
const W_ARABIC: &CStr = c"W17: Arabic requires UTF-8, do ':set encoding=utf-8'";

struct OptionModuleState {
    p_et_nobin: std::cell::Cell<bool>,
    p_ml_nobin: std::cell::Cell<bool>,
    p_tw_nobin: std::cell::Cell<OptInt>,
    p_wm_nobin: std::cell::Cell<OptInt>,
    p_ai_nopaste: std::cell::Cell<bool>,
    p_et_nopaste: std::cell::Cell<bool>,
    p_sts_nopaste: std::cell::Cell<OptInt>,
    p_tw_nopaste: std::cell::Cell<OptInt>,
    p_wm_nopaste: std::cell::Cell<OptInt>,
    old_p_paste: std::cell::Cell<bool>,
    save_sm: std::cell::Cell<bool>,
    save_sta: std::cell::Cell<bool>,
    save_ru: std::cell::Cell<bool>,
    save_ri: std::cell::Cell<bool>,
    syn_recursive: std::cell::Cell<i32>,
    expand_option_idx: std::cell::Cell<OptIndex>,
    expand_option_start_col: std::cell::Cell<i32>,
    expand_option_name: std::cell::Cell<[u8; 5]>,
    expand_option_flags: std::cell::Cell<i32>,
    expand_option_append: std::cell::Cell<bool>,
}

impl OptionModuleState {
    const fn new() -> Self {
        Self {
            p_et_nobin: std::cell::Cell::new(false),
            p_ml_nobin: std::cell::Cell::new(false),
            p_tw_nobin: std::cell::Cell::new(0),
            p_wm_nobin: std::cell::Cell::new(0),
            p_ai_nopaste: std::cell::Cell::new(false),
            p_et_nopaste: std::cell::Cell::new(false),
            p_sts_nopaste: std::cell::Cell::new(0),
            p_tw_nopaste: std::cell::Cell::new(0),
            p_wm_nopaste: std::cell::Cell::new(0),
            old_p_paste: std::cell::Cell::new(false),
            save_sm: std::cell::Cell::new(false),
            save_sta: std::cell::Cell::new(false),
            save_ru: std::cell::Cell::new(false),
            save_ri: std::cell::Cell::new(false),
            syn_recursive: std::cell::Cell::new(0),
            expand_option_idx: std::cell::Cell::new(K_OPT_INVALID),
            expand_option_start_col: std::cell::Cell::new(0),
            expand_option_name: std::cell::Cell::new([b't', b'_', 0, 0, 0]),
            expand_option_flags: std::cell::Cell::new(0),
            expand_option_append: std::cell::Cell::new(false),
        }
    }
}

thread_local! {
    static OPT_STATE: OptionModuleState = const { OptionModuleState::new() };
    static P_TERM: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
    static P_TTYTYPE: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
    static P_VSTS_NOPASTE: std::cell::RefCell<Option<std::ffi::CString>> = const {
        std::cell::RefCell::new(None)
    };
}

const P_BIN_DEP_OPTS: &[i32] = &[
    K_OPT_TEXTWIDTH,
    K_OPT_WRAPMARGIN,
    K_OPT_MODELINE,
    K_OPT_EXPANDTAB,
];

const P_PASTE_DEP_OPTS: &[i32] = &[
    K_OPT_AUTOINDENT,
    K_OPT_EXPANDTAB,
    K_OPT_RULER,
    K_OPT_SHOWMATCH,
    K_OPT_SMARTTAB,
    K_OPT_SOFTTABSTOP,
    K_OPT_TEXTWIDTH,
    K_OPT_WRAPMARGIN,
    K_OPT_REVINS,
    K_OPT_VARSOFTTABSTOP,
];

struct ShellRule {
    pat: &'static CStr,
    shcf: Option<&'static CStr>,
    sp: Option<&'static CStr>,
    srr: Option<&'static CStr>,
    sxq: Option<&'static CStr>,
}

#[cfg(windows)]
const SHELL_RULES: [ShellRule; 4] = [
    ShellRule {
        pat: c"cmd",
        shcf: Some(c"/s /c"),
        sp: None,
        srr: None,
        sxq: Some(c"\""),
    },
    ShellRule {
        pat: c"powershell",
        shcf: Some(c"-Command"),
        sp: None,
        srr: None,
        sxq: None,
    },
    ShellRule {
        pat: c"csh",
        shcf: None,
        sp: Some(c"|& tee"),
        srr: Some(c">&"),
        sxq: None,
    },
    ShellRule {
        pat: c"sh",
        shcf: None,
        sp: Some(c"2>&1| tee"),
        srr: Some(c">%s 2>&1"),
        sxq: None,
    },
];

#[cfg(not(windows))]
const SHELL_RULES: [ShellRule; 2] = [
    ShellRule {
        pat: c"csh",
        shcf: None,
        sp: Some(c"|& tee"),
        srr: Some(c">&"),
        sxq: None,
    },
    ShellRule {
        pat: c"sh",
        shcf: None,
        sp: Some(c"2>&1| tee"),
        srr: Some(c">%s 2>&1"),
        sxq: None,
    },
];

use crate::os::lang::bind_textdomain_codeset;

pub fn opt_error<'a>(
    errbuf: Option<&'a mut [u8]>,
    fmt: &std::ffi::CStr,
    arg: &std::ffi::CStr,
) -> Option<&'a CStr> {
    let Some(errbuf) = errbuf else {
        return Some(c"");
    };
    format_to_buf!(errbuf, gettext(fmt), arg);
    Some(crate::memory::bytes_as_cstr(errbuf))
}

pub fn opt_error_int<'a>(
    errbuf: Option<&'a mut [u8]>,
    fmt: &std::ffi::CStr,
    arg: i32,
) -> Option<&'a CStr> {
    let Some(errbuf) = errbuf else {
        return Some(c"");
    };
    format_to_buf!(errbuf, gettext(fmt), arg);
    Some(crate::memory::bytes_as_cstr(errbuf))
}

#[inline]
const fn unset_obj() -> Object {
    Object {
        obj_type: ObjectType::ObjectTypeUnset,
        data: ObjectData::Integer(0),
    }
}

use crate::keycodes::{ctrl_chr, key2termcap0, key2termcap1};

#[inline]
fn optval_type_name(t: ObjectType) -> &'static std::ffi::CStr {
    match t {
        ObjectType::ObjectTypeBoolean => c"boolean",
        ObjectType::ObjectTypeInteger => c"number",
        ObjectType::ObjectTypeString => c"string",
        _ => std::process::abort(),
    }
}

#[inline]
fn join_cstrings(items: &[std::ffi::CString], sep: &[u8]) -> std::ffi::CString {
    if items.is_empty() {
        return std::ffi::CString::default();
    }
    let mut total = sep.len() * (items.len() - 1);
    for item in items {
        total += item.to_bytes().len();
    }
    let mut out: Vec<u8> = Vec::with_capacity(total);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(sep);
        }
        out.extend_from_slice(item.to_bytes());
    }
    crate::memory::bytes_to_cstring(&out)
}

#[inline]
fn get_opt_idx(opt: Vimoption) -> OptIndex {
    find_option(opt.fullname.to_bytes())
}

#[inline]
fn get_varp(p: Vimoption) -> OptVar {
    get_varp_from(p, curbuf_mut(), Some(curwin_mut()))
}

pub fn set_init_tablocal() {
    // cmdheight calls itself OPT_GLOBAL but is really tablocal!
    opt_vars()
        .p_ch
        .set(get_option(K_OPT_CMDHEIGHT).def_val.data.integer());
}

/// Initialize the 'shell' option to a default value.
fn set_init_default_shell() {
    if let Some(shell) = os_getenv_cstr(c"SHELL") {
        if crate::strings::vim_haschr(shell.as_c_str(), b' ' as i32) {
            let len = shell.as_bytes().len() + 3;
            let mut cmd = vec![0u8; len];
            format_to_buf!(&mut cmd, c"\"%s\"", &shell);
            set_string_default(K_OPT_SHELL, crate::memory::bytes_as_cstr(&cmd));
        } else {
            set_string_default(K_OPT_SHELL, shell.as_c_str());
        }
    }
}

/// Set the default for 'backupskip' to include environment variables for temp files.
fn set_init_default_backupskip() {
    #[cfg(unix)]
    let names: [&CStr; 4] = [c"", c"TMPDIR", c"TEMP", c"TMP"];
    #[cfg(not(unix))]
    let names: [&CStr; 3] = [c"TMPDIR", c"TEMP", c"TMP"];

    let mut buf: Vec<u8> = Vec::new();
    let opt_idx = K_OPT_BACKUPSKIP;

    for &name in &names {
        let env_owned: Option<std::ffi::CString>;
        #[cfg(unix)]
        let env_cstr: Option<&CStr> = if name.to_bytes().is_empty() {
            #[cfg(target_os = "macos")]
            {
                Some(c"/private/tmp")
            }
            #[cfg(not(target_os = "macos"))]
            {
                Some(c"/tmp")
            }
        } else {
            env_owned = vim_getenv(name);
            env_owned.as_deref()
        };
        #[cfg(not(unix))]
        let env_cstr: Option<&CStr> = {
            env_owned = vim_getenv(name);
            env_owned.as_deref()
        };

        if let Some(env_cstr) = env_cstr
            && !env_cstr.is_empty()
        {
            let p_bytes = env_cstr.to_bytes();
            let plen = p_bytes.len();
            let has_trailing_path_sep = after_pathsep(p_bytes) != 0;

            let itemsize = plen + usize::from(!has_trailing_path_sep) + 2;
            let mut item = vec![0u8; itemsize];

            let itemlen = format_to_buf!(
                &mut item,
                c"%s%s*",
                env_cstr,
                if has_trailing_path_sep {
                    c""
                } else {
                    PATHSEPSTR
                },
            ) as usize;

            if find_dup_item(
                crate::memory::bytes_until_nul(&buf),
                &item[..itemlen],
                get_option(opt_idx).flags,
            )
            .is_none()
            {
                if !buf.is_empty() {
                    // Replace trailing NUL with ','
                    let last = buf.len() - 1;
                    buf[last] = b',';
                }
                buf.extend_from_slice(&item[..itemlen]);
                buf.push(0);
            }
        }
    }
    if !buf.is_empty() {
        set_string_default(K_OPT_BACKUPSKIP, crate::memory::bytes_as_cstr(&buf));
    }
}

/// Initialize the 'cdpath' option to a default value.
fn set_init_default_cdpath() {
    let Some(cdpath_cstr) = vim_getenv(c"CDPATH") else {
        return;
    };
    let cdpath = cdpath_cstr.as_bytes();

    let mut buf_vec = vec![0u8; 2 * cdpath.len() + 2];
    buf_vec[0] = b',';
    let mut j: usize = 1;
    for &ch in cdpath {
        if vim_ispathlistsep(i32::from(ch)) {
            buf_vec[j] = b',';
            j += 1;
        } else {
            if ch == b' ' || ch == b',' {
                buf_vec[j] = b'\\';
                j += 1;
            }
            buf_vec[j] = ch;
            j += 1;
        }
    }
    buf_vec[j] = 0;
    change_option_default(
        K_OPT_CDPATH,
        cstr_to_obj(Some(crate::memory::bytes_as_cstr(&buf_vec))),
    );
}

/// Expand environment variables and things like "~" for the defaults.
fn set_init_expand_env() {
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        if (opt.flags & K_OPT_FLAG_NO_DEF_EXP) != 0 {
            continue;
        }
        let mut p: Option<&CStr> = None;
        if (opt.flags & K_OPT_FLAG_GETTEXT) != 0 && opt.var().is_some() {
            if let Some(cstr) = opt.var().opt_cstr() {
                p = Some(tr(cstr));
            }
        } else if (opt.flags & K_OPT_FLAG_EXPAND) != 0 {
            p = option_expand(opt_idx, opt.def_val.data.string().as_opt_cstr());
        }
        if let Some(p) = p {
            let ov = cstr_to_obj(Some(p));
            if opt.var().is_some() {
                set_option_varp(opt_idx, opt.var(), ov.clone());
            }
            change_option_default(opt_idx, ov);
        }
    }
}

/// Initialize the encoding used for "default" in 'fileencodings'.
fn set_init_fenc_default() {
    g_st()
        .fenc_default
        .set(Some(enc_locale().unwrap_or_else(|| c"utf-8".to_owned())));
}

/// Initialize the options, first part.
pub fn set_init_1(clean_arg: bool) {
    langmap_init();

    alloc_options_default();

    set_init_default_shell();
    set_init_default_backupskip();
    set_init_default_cdpath();

    let raw_backupdir = stdpaths_user_state_subpath(c"backup", 2, true);
    let mut backupdir_bytes = Vec::with_capacity(raw_backupdir.as_bytes().len() + 2);
    backupdir_bytes.extend_from_slice(b".,");
    backupdir_bytes.extend_from_slice(raw_backupdir.as_bytes());
    let backupdir = crate::memory::bytes_to_cstring(&backupdir_bytes);
    set_string_default(K_OPT_BACKUPDIR, backupdir.as_c_str());
    set_string_default(
        K_OPT_VIEWDIR,
        stdpaths_user_state_subpath(c"view", 2, false).as_c_str(),
    );
    set_string_default(
        K_OPT_DIRECTORY,
        stdpaths_user_state_subpath(c"swap", 2, true).as_c_str(),
    );
    set_string_default(
        K_OPT_UNDODIR,
        stdpaths_user_state_subpath(c"undo", 2, true).as_c_str(),
    );
    set_string_default(
        K_OPT_PACKLOCKFILE,
        stdpaths_user_conf_subpath(c"nvim-pack-lock.json").as_c_str(),
    );
    if let Some(rtp) = runtimepath_default(clean_arg) {
        set_string_default(K_OPT_RUNTIMEPATH, rtp.as_c_str());
        set_string_default(K_OPT_PACKPATH, rtp.as_c_str());
    }

    set_options_default(0);

    curbuf_mut().b_p_initialized.set(true);
    curbuf_mut().b_p_ac.set(None);
    curbuf_mut().b_p_ar.set(None);
    curbuf_mut().b_p_fs.set(None);
    curbuf_mut().b_p_ul.set(NO_LOCAL_UNDOLEVEL);
    check_buf_options(curbuf_mut());
    check_win_options(curwin_mut());
    check_options();

    last_status(false);

    didset_options();

    init_spell_chartab();

    set_init_expand_env();

    if os_env_exists(c"NVIM_NOTTYFAST", false) {
        set_option_value_give_err(K_OPT_TTYFAST, boolean_obj(false), 0);
    }

    save_file_ff(curbuf_mut());

    if os_env_exists(c"MLTERM", false) {
        set_option_value_give_err(K_OPT_TERMBIDI, boolean_obj(true), 0);
    }

    didset_options2();

    lang_init();
    set_init_fenc_default();

    bind_textdomain_codeset(c"nvim", Some(opt_vars().p_enc.as_cstr()));

    set_helplang_default(get_mess_lang());
}

/// Get default value for option, based on the option's type and scope.
pub fn get_option_default(opt_idx: OptIndex, opt_flags: i32) -> Object {
    let opt = get_option(opt_idx);
    let is_global_local_option = option_is_global_local(opt_idx);

    #[cfg(unix)]
    if opt_idx == K_OPT_MODELINE && getuid() == ROOT_UID {
        return boolean_obj(false);
    }

    if (opt_flags & OPT_LOCAL) != 0 && is_global_local_option {
        return unset_obj();
    }

    let mut def_val = opt.def_val;
    if option_has_type(opt_idx, ObjectType::ObjectTypeString)
        && (opt.flags & K_OPT_FLAG_NO_DEF_EXP) == 0
        && let Some(s) = option_expand(opt_idx, def_val.data.as_opt_cstr())
    {
        def_val = cstr_as_obj(Some(s));
    }
    if optval_is_owned(opt_idx) {
        copy_object(def_val, None)
    } else {
        def_val
    }
}

/// Allocate the default values for all options by copying them from the stack.
fn alloc_options_default() {
    for opt_idx in 0..K_OPT_COUNT {
        let def_val = get_option(opt_idx).def_val;
        let copied = copy_object(def_val, None);
        OPTIONS.with(|opts| opts.borrow_mut()[opt_idx as usize].def_val = copied);
    }
}

/// Change the default value for an option.
fn change_option_default(opt_idx: OptIndex, value: Object) {
    let old_def_val = get_option(opt_idx).def_val;
    optval_free(old_def_val);
    OPTIONS.with(|opts| opts.borrow_mut()[opt_idx as usize].def_val = value);
}

/// Set an option to its default value.
fn set_option_default(opt_idx: OptIndex, opt_flags: i32) {
    let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
    let def_val = get_option_default(opt_idx, opt_flags);
    set_option_direct(
        opt_idx,
        def_val.clone(),
        opt_flags,
        g_st().current_sctx.get().sc_sid,
    );
    optval_free_read(opt_idx, def_val);

    if opt_idx == K_OPT_SCROLL {
        win_comp_scroll(curwin_mut());
    }

    set_insecure_flag(
        Some(crate::globals::curwin_mut()),
        opt_idx,
        opt_flags,
        false,
    );
    if both {
        set_insecure_flag(
            Some(crate::globals::curwin_mut()),
            opt_idx,
            OPT_LOCAL,
            false,
        );
    }
}

/// Set all options (except terminal options) to their default value.
fn set_options_default(opt_flags: i32) {
    for opt_idx in 0..K_OPT_COUNT {
        if (get_option(opt_idx).flags & K_OPT_FLAG_NO_DEFAULT) == 0 {
            set_option_default(opt_idx, opt_flags);
        }
    }

    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_mut(&tp) {
            win_comp_scroll(wp);
        }
    }

    parse_cino(curbuf_mut());
}

/// Set the Vi-default value of a string option.
fn set_string_default(opt_idx: OptIndex, val: &std::ffi::CStr) {
    assert!(opt_idx != K_OPT_INVALID);
    change_option_default(opt_idx, cstr_to_obj(Some(val)));
}

/// For an option value that contains comma separated items, find "newval" in "origval".
fn find_dup_item(orig_bytes: &[u8], new_bytes: &[u8], flags: u32) -> Option<usize> {
    let comma_flag = (flags & K_OPT_FLAG_COMMA) != 0;
    let newvallen = new_bytes.len();
    let mut bs: i32 = 0;
    for idx in 0..orig_bytes.len() {
        if (!comma_flag || idx == 0 || (orig_bytes[idx - 1] == b',' && (bs & 1) == 0))
            && orig_bytes[idx..].starts_with(new_bytes)
            && (!comma_flag || orig_bytes.get(idx + newvallen).is_none_or(|&b| b == b','))
        {
            return Some(idx);
        }
        if (idx > 1 && orig_bytes[idx - 1] == b'\\' && orig_bytes[idx - 2] != b',')
            || (idx == 1 && orig_bytes[0] == b'\\')
        {
            bs += 1;
        } else {
            bs = 0;
        }
    }
    None
}

/// Free all options.
pub fn free_all_options() {
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        let hidden = is_option_hidden(opt_idx);

        if !hidden
            && (option_is_global_only(opt_idx) || !option_is_window_local(opt_idx))
            && opt.var().is_some()
        {
            if is_callback_option(opt_idx) {
                callback_free(opt.var().callback_mut());
            } else if option_has_type(opt_idx, ObjectType::ObjectTypeString) {
                opt.var().clear_str();
            }
        }
        optval_free(opt.def_val);
    }
    free_operatorfunc_option();
    free_tagfunc_option();
    free_findfunc_option();
    g_st().fenc_default.set(None);
    P_TERM.with(|cell| *cell.borrow_mut() = None);
    P_TTYTYPE.with(|cell| *cell.borrow_mut() = None);
}

/// Initialize the options, part two: After getting Rows and Columns.
pub fn set_init_2(_headless: bool) {
    logmsg!(
        1,
        None,
        Some(c"set_init_2"),
        592,
        true,
        c"startup runtimepath/packpath value: %s",
        &opt_vars().p_rtp,
    );

    if (get_option(K_OPT_SCROLL).flags & K_OPT_FLAG_WAS_SET) == 0 {
        set_option_default(K_OPT_SCROLL, OPT_LOCAL);
    }
    comp_col();

    if !option_was_set(K_OPT_WINDOW) {
        opt_vars()
            .p_window
            .set(OptInt::from(g_st().rows_global.get() - 1));
    }
    change_option_default(
        K_OPT_WINDOW,
        integer_obj(OptInt::from(g_st().rows_global.get() - 1)),
    );
}

fn change_option_and_default_if_unset(idx: OptIndex, val: Option<&'static CStr>) {
    let Some(val) = val else {
        return;
    };
    if (get_option(idx).flags & K_OPT_FLAG_WAS_SET) != 0 {
        return;
    }
    let optval = cstr_as_obj(Some(val));
    set_option_direct(idx, optval.clone(), 0, SID_NONE);
    change_option_default(idx, copy_object(optval, None));
}

/// Initialize the options, part three: After reading the .vimrc
pub fn set_init_3() {
    parse_shape_opt(SHAPE_CURSOR);

    let mut len: usize = 0;
    let p_sh = opt_vars().p_sh.to_owned();
    let p = invocation_path_tail(&p_sh, Some(&mut len));
    let p_bytes = p.to_bytes();
    let name_slice = &p_bytes[..len.min(p_bytes.len())];
    for (i, rule) in SHELL_RULES.iter().enumerate() {
        let pat = rule.pat.to_bytes();
        if !pat.is_empty() && !name_slice.windows(pat.len()).any(|w| w == pat) {
            continue;
        }
        change_option_and_default_if_unset(K_OPT_SHELLCMDFLAG, rule.shcf);
        change_option_and_default_if_unset(K_OPT_SHELLPIPE, rule.sp);
        change_option_and_default_if_unset(K_OPT_SHELLREDIR, rule.srr);
        change_option_and_default_if_unset(K_OPT_SHELLXQUOTE, rule.sxq);
        #[cfg(windows)]
        if i > 0 && (get_option(K_OPT_SHELLSLASH).flags & K_OPT_FLAG_WAS_SET) == 0 {
            set_option_direct(K_OPT_SHELLSLASH, boolean_obj(true), 0, SID_NONE);
            change_option_default(K_OPT_SHELLSLASH, boolean_obj(true));
        }
        let _ = i;
        break;
    }

    if buf_is_empty(curbuf_mut()) && (get_option(K_OPT_FILEFORMATS).flags & K_OPT_FLAG_WAS_SET) != 0
    {
        set_fileformat(default_fileformat(), OPT_LOCAL);
    }

    set_title_defaults();
}

/// When 'helplang' is still at its default value, set it to "lang".
pub fn set_helplang_default(lang: Option<&std::ffi::CStr>) {
    let Some(lang) = lang else {
        return;
    };

    let lang_bytes = lang.to_bytes();
    if lang_bytes.len() < 2 || (get_option(K_OPT_HELPLANG).flags & K_OPT_FLAG_WAS_SET) != 0 {
        return;
    }

    let mut hlg_bytes = [lang_bytes[0], lang_bytes[1]];
    if lang_bytes.len() >= 5 && hlg_bytes.eq_ignore_ascii_case(b"zh") && lang_bytes[2] == b'_' {
        hlg_bytes[0] = lang_bytes[3].to_ascii_lowercase();
        hlg_bytes[1] = lang_bytes[4].to_ascii_lowercase();
    } else if lang_bytes[0] == b'C' {
        hlg_bytes = *b"en";
    }
    opt_vars().p_hlg.set_option_bytes(&hlg_bytes);
}

pub fn set_title_defaults() {
    if (get_option(K_OPT_TITLE).flags & K_OPT_FLAG_WAS_SET) == 0 {
        change_option_default(K_OPT_TITLE, boolean_obj(false));
        opt_vars().p_title.set(false);
    }
    if (get_option(K_OPT_ICON).flags & K_OPT_FLAG_WAS_SET) == 0 {
        change_option_default(K_OPT_ICON, boolean_obj(false));
        opt_vars().p_icon.set(false);
    }
}

pub fn ex_set(eap: &mut Exarg) {
    let mut flags: i32 = 0;

    if eap.cmdidx == Cmdidx::CmdSetlocal {
        flags = OPT_LOCAL;
    } else if eap.cmdidx == Cmdidx::CmdSetglobal {
        flags = OPT_GLOBAL;
    }
    if eap.forceit {
        flags |= OPT_ONECOLUMN;
    }
    do_set(eap.arg.as_cstr(), flags);
}

/// Copy the new string value into allocated memory for the option.
fn stropt_copy_value(
    origval: Option<&std::ffi::CStr>,
    argp: &mut &[u8],
    op: SetOp,
    flags: u32,
) -> Vec<u8> {
    let arg_bytes = crate::memory::bytes_until_nul(argp);
    let mut rem = arg_bytes;

    let mut newlen = arg_bytes.len() + 1;
    if op != OP_NONE {
        newlen += origval.map_or(0, |s| s.to_bytes().len()) + 1;
    }
    let mut newval_vec: Vec<u8> = vec![0; newlen];
    let mut pos: usize = 0;

    while let Some(&b0) = rem.first() {
        if ascii_iswhite(i32::from(b0)) {
            break;
        }
        #[cfg(windows)]
        let remove_bs = b0 == b'\\'
            && rem.len() >= 2
            && !((flags & K_OPT_FLAG_EXPAND) != 0
                && vim_isfilec(i32::from(rem[1]))
                && !ascii_iswhite(i32::from(rem[1]))
                && (rem[1] != b'\\' || (pos == 0 && rem.get(2) != Some(&b'\\'))));
        #[cfg(not(windows))]
        let remove_bs = {
            let _ = flags;
            b0 == b'\\' && rem.len() >= 2
        };
        if remove_bs {
            rem = &rem[1..];
        }
        let i = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
        newval_vec[pos..pos + i].copy_from_slice(&rem[..i]);
        rem = &rem[i..];
        pos += i;
    }
    newval_vec[pos] = 0;

    let consumed = arg_bytes.len() - rem.len();
    *argp = &argp[consumed..];
    newval_vec
}

/// Expand environment variables and ~ in string option value 'newval'.
fn stropt_expand_envvar(
    opt_idx: OptIndex,
    origval: Option<&std::ffi::CStr>,
    newval: &mut Vec<u8>,
    op: SetOp,
) {
    let Some(s) = option_expand(opt_idx, Some(crate::memory::bytes_as_cstr(newval))) else {
        return;
    };

    let mut newlen = (s.to_bytes().len() as u32) + 1;
    if op != OP_NONE {
        newlen += (origval.map_or(0, |o| o.to_bytes().len()) as u32) + 1;
    }
    *newval = vec![0; newlen as usize];
    let s_bytes = s.to_bytes_with_nul();
    newval[..s_bytes.len()].copy_from_slice(s_bytes);
}

/// Concatenate the original and new values of a string option, adding a "," if needed.
fn stropt_concat_with_comma(
    origval: Option<&std::ffi::CStr>,
    newval: &mut [u8],
    op: SetOp,
    flags: u32,
) {
    let orig = origval.map_or(&[][..], CStr::to_bytes);
    let new_len = crate::memory::bytes_until_nul(newval).len();
    let comma = usize::from((flags & K_OPT_FLAG_COMMA) != 0 && !orig.is_empty() && new_len > 0);
    let mut len: usize;
    if op == OP_ADDING {
        len = orig.len();
        if comma != 0
            && len > 1
            && (flags & K_OPT_FLAG_ONE_COMMA) == K_OPT_FLAG_ONE_COMMA
            && orig[len - 1] == b','
            && orig[len - 2] != b'\\'
        {
            len -= 1;
        }
        newval.copy_within(0..=new_len, len + comma);
        newval[..len].copy_from_slice(&orig[..len]);
    } else {
        len = new_len;
        newval[len + comma..len + comma + orig.len()].copy_from_slice(orig);
        newval[len + comma + orig.len()] = 0;
    }
    if comma != 0 {
        newval[len] = b',';
    }
}

/// Remove a value from a string option.
fn stropt_remove_val(
    origval: Option<&std::ffi::CStr>,
    newval: &mut [u8],
    flags: u32,
    dup_idx: Option<usize>,
    mut len: usize,
) {
    let orig_bytes = origval.unwrap_or(c"").to_bytes_with_nul();
    newval[..orig_bytes.len()].copy_from_slice(orig_bytes);
    let orig_slice = &orig_bytes[..orig_bytes.len() - 1];
    if let Some(mut idx) = dup_idx
        && idx < orig_slice.len()
    {
        if (flags & K_OPT_FLAG_COMMA) != 0 {
            if idx == 0 {
                if orig_slice.get(len) == Some(&b',') {
                    len += 1;
                }
            } else {
                idx -= 1;
                len += 1;
            }
        }
        let src_bytes = &orig_bytes[(idx + len).min(orig_bytes.len() - 1)..];
        newval[idx..idx + src_bytes.len()].copy_from_slice(src_bytes);
    }
}

/// Find a comma-separated item in "src" (starting at `start`) that matches `key_prefix`.
fn find_key_item(src: &[u8], start: usize, key_prefix: &[u8]) -> Option<(usize, usize)> {
    let bytes = crate::memory::bytes_until_nul(src);
    let mut i = start;
    while i < bytes.len() {
        if (i == start || bytes[i - 1] == b',') && bytes[i..].starts_with(key_prefix) {
            let item_len = bytes[i..]
                .iter()
                .position(|&b| b == b',')
                .unwrap_or(bytes.len() - i);
            return Some((i, item_len));
        }
        i += 1;
    }
    None
}

/// Remove one item of length `itemlen` at offset `item_off` from comma-separated buffer `str_arg` in-place.
fn remove_comma_item(str_arg: &mut [u8], item_off: usize, itemlen: usize) {
    let total_len = crate::memory::bytes_until_nul(str_arg).len();
    if str_arg.get(item_off + itemlen) == Some(&b',') {
        str_arg.copy_within(item_off + itemlen + 1..=total_len, item_off);
    } else if item_off > 0 && str_arg[item_off - 1] == b',' {
        str_arg.copy_within(item_off + itemlen..=total_len, item_off - 1);
    } else {
        str_arg[item_off] = 0;
    }
}

/// Remove all items matching `key_prefix` (with ':') from comma-separated string `str_arg` in-place.
fn remove_key_item(str_arg: &mut [u8], key_prefix: &[u8], skip_off: Option<usize>) {
    while let Some((mut found_off, mut itemlen)) = find_key_item(str_arg, 0, key_prefix) {
        if skip_off == Some(found_off) {
            let mut next_start = found_off + itemlen;
            if str_arg.get(next_start) == Some(&b',') {
                next_start += 1;
            }
            let Some((next_found, next_len)) = find_key_item(str_arg, next_start, key_prefix)
            else {
                break;
            };
            found_off = next_found;
            itemlen = next_len;
        }

        remove_comma_item(str_arg, found_off, itemlen);
    }
}

/// Append a comma-separated item to the end of "str" in-place.
fn append_item(str_arg: &mut [u8], item_bytes: &[u8]) {
    let mut len = crate::memory::bytes_until_nul(str_arg).len();
    if len > 0 {
        str_arg[len] = b',';
        len += 1;
    }
    str_arg[len..len + item_bytes.len()].copy_from_slice(item_bytes);
    str_arg[len + item_bytes.len()] = 0;
}

/// Prepend a comma-separated item to the beginning of "str" in-place.
fn prepend_item(str_arg: &mut [u8], item_bytes: &[u8]) {
    let len = crate::memory::bytes_until_nul(str_arg).len();
    let comma = usize::from(len > 0);

    str_arg.copy_within(0..=len, item_bytes.len() + comma);
    str_arg[..item_bytes.len()].copy_from_slice(item_bytes);
    if comma != 0 {
        str_arg[item_bytes.len()] = b',';
    }
}

/// For a P_COMMA option: process "key:value" items in "newval" individually.
fn stropt_handle_keymatch(
    origval: Option<&std::ffi::CStr>,
    newval_slice: &mut [u8],
    op: SetOp,
    _flags: u32,
) -> bool {
    let newval_bytes = crate::memory::bytes_until_nul(newval_slice);
    if !newval_bytes.contains(&b':') && !newval_bytes.contains(&b',') {
        return false;
    }

    let newval_copy = newval_bytes.to_vec();
    let orig_bytes = origval.unwrap_or(c"").to_bytes_with_nul();
    newval_slice[..orig_bytes.len()].copy_from_slice(orig_bytes);

    for item in newval_copy.split(|&b| b == b',') {
        if item.is_empty() {
            continue;
        }
        if let Some(colon_idx) = item.iter().position(|&b| b == b':') {
            let key_prefix = &item[..=colon_idx];

            if op == OP_ADDING || op == OP_PREPENDING {
                if let Some((found_off, old_itemlen)) = find_key_item(newval_slice, 0, key_prefix) {
                    if old_itemlen == item.len()
                        && &newval_slice[found_off..found_off + item.len()] == item
                    {
                        remove_key_item(newval_slice, key_prefix, Some(found_off));
                    } else {
                        remove_key_item(newval_slice, key_prefix, None);
                        if op == OP_PREPENDING {
                            prepend_item(newval_slice, item);
                        } else {
                            append_item(newval_slice, item);
                        }
                    }
                } else if op == OP_PREPENDING {
                    prepend_item(newval_slice, item);
                } else {
                    append_item(newval_slice, item);
                }
            } else if op == OP_REMOVING {
                remove_key_item(newval_slice, key_prefix, None);
            }
        } else if op == OP_ADDING || op == OP_PREPENDING {
            let found = find_dup_item(
                crate::memory::bytes_until_nul(newval_slice),
                item,
                K_OPT_FLAG_COMMA,
            );
            if found.is_none() {
                if op == OP_PREPENDING {
                    prepend_item(newval_slice, item);
                } else {
                    append_item(newval_slice, item);
                }
            }
        } else if op == OP_REMOVING {
            let found = find_dup_item(
                crate::memory::bytes_until_nul(newval_slice),
                item,
                K_OPT_FLAG_COMMA,
            );
            if let Some(found_off) = found {
                remove_comma_item(newval_slice, found_off, item.len());
            }
        }
    }

    true
}

/// Remove flags that appear twice in the string option value 'newval'.
fn stropt_remove_dupflags(newval: &mut [u8], flags: u32) {
    let mut i = 0;
    while newval[i] != 0 {
        let ch = newval[i];
        let rest_end = i + crate::memory::bytes_until_nul(&newval[i..]).len();
        if (flags & K_OPT_FLAG_ONE_COMMA) != 0 {
            if ch != b',' && newval[i + 1] == b',' && newval[i + 2..rest_end].contains(&ch) {
                newval.copy_within(i + 2..=rest_end, i);
                continue;
            }
        } else if ((flags & K_OPT_FLAG_COMMA) == 0 || ch != b',')
            && newval[i + 1..rest_end].contains(&ch)
        {
            newval.copy_within(i + 1..=rest_end, i);
            continue;
        }
        i += 1;
    }
}

/// Get the string value specified for a ":set" command.
fn stropt_get_newval(
    opt_idx: OptIndex,
    argp: &mut &[u8],
    varp: OptVar,
    origval: Option<&std::ffi::CStr>,
    op_arg: &mut SetOp,
) -> std::ffi::CString {
    let mut op = *op_arg;
    let flags = get_option(opt_idx).flags;

    let mut arg = &argp[usize::from(!argp.is_empty())..];
    let mut save_arg: Option<&[u8]> = None;

    if varp.ptr_eq(&opt_vars().p_kp)
        && (crate::memory::bytes_until_nul(arg).is_empty() || arg.first() == Some(&b' '))
    {
        save_arg = Some(arg);
        arg = b":help";
    }

    let mut newval_vec = stropt_copy_value(origval, &mut arg, op, flags);

    if op == OP_NONE || (flags & K_OPT_FLAG_COMMA) != 0 {
        stropt_expand_envvar(opt_idx, origval, &mut newval_vec, op);
    }

    if (flags & K_OPT_FLAG_COMMA) != 0
        && (flags & K_OPT_FLAG_COLON) != 0
        && op != OP_NONE
        && stropt_handle_keymatch(origval, &mut newval_vec, op, flags)
    {
        // fully handled
    } else {
        let mut len: usize = 0;
        let mut dup_idx: Option<usize> = None;
        if op == OP_REMOVING || (flags & K_OPT_FLAG_NO_DUP) != 0 {
            let newval_bytes = crate::memory::bytes_until_nul(&newval_vec);
            len = newval_bytes.len();
            if let Some(orig) = origval {
                dup_idx = find_dup_item(orig.to_bytes(), newval_bytes, flags);
            }

            if (op == OP_ADDING || op == OP_PREPENDING) && dup_idx.is_some() {
                op = OP_NONE;
                let orig_bytes = origval.unwrap_or(c"").to_bytes_with_nul();
                newval_vec[..orig_bytes.len()].copy_from_slice(orig_bytes);
            }
        }

        if op == OP_ADDING || op == OP_PREPENDING {
            stropt_concat_with_comma(origval, &mut newval_vec, op, flags);
        } else if op == OP_REMOVING {
            stropt_remove_val(origval, &mut newval_vec, flags, dup_idx, len);
        }
    }

    if (flags & K_OPT_FLAG_FLAG_LIST) != 0 {
        stropt_remove_dupflags(&mut newval_vec, flags);
    }

    if let Some(saved) = save_arg {
        arg = saved;
    }

    *argp = arg;
    *op_arg = op;

    crate::memory::cstring_until_nul(&newval_vec)
}

fn get_op(bytes: &[u8]) -> SetOp {
    if bytes.get(1) == Some(&b'=') {
        match bytes[0] {
            b'+' => OP_ADDING,
            b'^' => OP_PREPENDING,
            b'-' => OP_REMOVING,
            _ => OP_NONE,
        }
    } else {
        OP_NONE
    }
}

fn get_option_prefix(argp: &mut &[u8]) -> SetPrefix {
    let bytes = *argp;
    if bytes.starts_with(b"no") {
        *argp = &bytes[2..];
        PREFIX_NO
    } else if bytes.starts_with(b"inv") {
        *argp = &bytes[3..];
        PREFIX_INV
    } else {
        PREFIX_NONE
    }
}

fn validate_opt_idx(
    win: &mut WinInfoInfo,
    opt_idx: OptIndex,
    opt_flags: i32,
    flags: u32,
    prefix: SetPrefix,
    errmsg: &mut Option<&'static CStr>,
) -> bool {
    if !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) && prefix != PREFIX_NONE {
        *errmsg = Some(E_INVARG);
        return false;
    }

    if (opt_flags & OPT_WINONLY) != 0 && !option_is_window_local(opt_idx) {
        return false;
    }

    if (opt_flags & OPT_NOWIN) != 0 && option_is_window_local(opt_idx) {
        return false;
    }

    if (opt_flags & OPT_MODELINE) != 0 {
        if (flags & K_OPT_FLAG_SECURE) != 0 {
            *errmsg = Some(E_NOT_ALLOWED_IN_MODELINE);
            return false;
        }
        if (flags & K_OPT_FLAG_MLE) != 0 && !opt_vars().p_mle.get() {
            *errmsg = Some(E_NOT_ALLOWED_IN_MODELINE_WHEN_MODELINEEXPR_IS_OFF);
            return false;
        }
        if win.w_onebuf_opt.wo_diff.get() && (opt_idx == K_OPT_FOLDMETHOD || opt_idx == K_OPT_WRAP)
        {
            return false;
        }
    }

    if g_st().sandbox.get() != 0 && (flags & K_OPT_FLAG_SECURE) != 0 {
        *errmsg = Some(E_SANDBOX);
        return false;
    }

    true
}

/// Skip over the name of a TTY option or keycode option.
fn find_tty_option_len(bytes: &[u8]) -> Option<usize> {
    let bytes = crate::memory::bytes_until_nul(bytes);
    if bytes == b"term" {
        return Some("term".len());
    } else if bytes == b"ttytype" {
        return Some("ttytype".len());
    }

    let mut rem = bytes;
    let mut delimit = false;
    if let Some(after_lt) = rem.strip_prefix(b"<") {
        delimit = true;
        rem = after_lt;
    }
    if rem.len() >= 4 && rem.starts_with(b"t_") {
        rem = &rem[4..];
    } else if delimit {
        let pos = rem.iter().position(|&b| b == b'>').unwrap_or(rem.len());
        rem = &rem[pos..];
    }
    if delimit {
        rem = rem.strip_prefix(b">")?;
    }

    (rem.len() != bytes.len()).then_some(bytes.len() - rem.len())
}

/// Skip over the name of an option.
pub fn find_option_end(arg: &[u8], opt_idxp: &mut OptIndex) -> Option<usize> {
    let bytes = crate::memory::bytes_until_nul(arg);
    if let Some(tty_len) = find_tty_option_len(bytes) {
        *opt_idxp = K_OPT_INVALID;
        return Some(tty_len);
    }
    let alpha_len = bytes
        .iter()
        .take_while(|&&b| b.is_ascii_alphabetic())
        .count();
    if alpha_len == 0 {
        *opt_idxp = K_OPT_INVALID;
        return None;
    }

    *opt_idxp = find_option_len(bytes, alpha_len);
    Some(alpha_len)
}

/// Get new option value from argp. Allocated Object must be freed by caller.
pub fn get_option_newval(
    opt_idx: OptIndex,
    opt_flags: i32,
    prefix: SetPrefix,
    argp: &mut &[u8],
    nextchar: i32,
    mut op: SetOp,
    varp: OptVar,
    mut oldval_override: Option<&mut Object>,
    errmsg: &mut Option<&'static CStr>,
) -> Object {
    assert!(varp.is_some());
    let opt = get_option(opt_idx);
    let mut arg = crate::memory::bytes_until_nul(argp);
    let mut newval = nil_obj();

    if nextchar == b'&' as i32 {
        return optval_own(opt_idx, get_option_default(opt_idx, OPT_GLOBAL));
    } else if nextchar == b'<' as i32 {
        if option_is_global_local(opt_idx) && (opt_flags & OPT_LOCAL) == 0 {
            unset_option_local_value(opt_idx);
        }
        return get_option_value(opt_idx, OPT_GLOBAL);
    }

    let oldval = if oldval_override.is_some() {
        oldval_override.as_deref_mut().unwrap().clone()
    } else {
        let oldval_is_global = option_is_global_local(opt_idx) && (opt_flags & OPT_LOCAL) != 0;
        optval_own(
            opt_idx,
            opt_from_varp(
                opt_idx,
                if oldval_is_global {
                    get_varp(opt)
                } else {
                    varp
                },
            ),
        )
    };

    match oldval.obj_type {
        ObjectType::ObjectTypeNil => std::process::abort(),
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeBoolean => {
            let newval_bool: TriState = if nextchar == b'!' as i32 {
                if oldval.obj_type == ObjectType::ObjectTypeUnset {
                    K_NONE
                } else if oldval.data.boolean() {
                    K_FALSE
                } else {
                    K_TRUE
                }
            } else if prefix == PREFIX_INV {
                varp.bool_tristate() ^ 1
            } else if prefix == PREFIX_NO {
                0
            } else {
                1
            };

            newval = opt_from_tristate(newval_bool);
        }
        ObjectType::ObjectTypeInteger => {
            let oldval_num = oldval.data.integer();
            let mut newval_num: OptInt = 0;

            arg = &arg[usize::from(!arg.is_empty())..];
            let arg_bytes = arg;
            if (varp.ptr_eq(&opt_vars().p_wc) || varp.ptr_eq(&opt_vars().p_wcm))
                && (matches!(arg_bytes.first(), Some(b'<' | b'^'))
                    || (!arg_bytes.is_empty()
                        && arg_bytes.get(1).is_none_or(|&b| ascii_iswhite(b as i32))
                        && !arg_bytes[0].is_ascii_digit()))
            {
                newval_num = OptInt::from(string_to_key(arg_bytes));
                if newval_num == 0 {
                    *errmsg = Some(E_INVARG);
                    if oldval_override.is_none() {
                        optval_free(oldval);
                    }
                    return newval;
                }
            } else if arg_bytes
                .first()
                .is_some_and(|&b| b == b'-' || b.is_ascii_digit())
            {
                let mut i: i32 = 0;
                vim_str2nr(
                    arg_bytes,
                    None,
                    Some(&mut i),
                    STR2NR_ALL,
                    Some(&mut newval_num),
                    None,
                    0,
                    true,
                    None,
                );
                if i == 0
                    || arg_bytes
                        .get(i as usize)
                        .is_some_and(|&b| !ascii_iswhite(b as i32))
                {
                    *errmsg = Some(E_NUMBER_REQUIRED_AFTER_EQUAL);
                    if oldval_override.is_none() {
                        optval_free(oldval);
                    }
                    return newval;
                }
            } else {
                *errmsg = Some(E_NUMBER_REQUIRED_AFTER_EQUAL);
                if oldval_override.is_none() {
                    optval_free(oldval);
                }
                return newval;
            }

            if op == OP_ADDING {
                newval_num += oldval_num;
            }
            if op == OP_PREPENDING {
                newval_num *= oldval_num;
            }
            if op == OP_REMOVING {
                newval_num = oldval_num - newval_num;
            }

            newval = integer_obj(newval_num);
        }
        ObjectType::ObjectTypeString => {
            let oldval_str = oldval.data.as_opt_cstr();
            let newval_str = stropt_get_newval(opt_idx, argp, varp, oldval_str, &mut op);
            newval = cstr_to_obj(Some(&*newval_str));
        }
        ObjectType::ObjectTypeLuaRef => {
            let newval_str = stropt_get_newval(opt_idx, argp, varp, Some(c""), &mut op);
            newval = cstr_to_obj(Some(&*newval_str));
        }
        _ => std::process::abort(),
    }

    if oldval_override.is_none() {
        optval_free(oldval);
    }
    newval
}

fn do_one_set_option<'a>(
    opt_flags: i32,
    argp: &mut &[u8],
    did_show: &mut bool,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let prefix = get_option_prefix(argp);
    let arg = *argp;

    let mut opt_idx: OptIndex = K_OPT_INVALID;
    let option_end_off = find_option_end(arg, &mut opt_idx);

    if opt_idx != K_OPT_INVALID {
        assert!(option_end_off.is_some());
    } else if is_tty_option(arg) {
        return None;
    } else {
        return Some(E_UNKNOWN_OPTION);
    }

    let end_off = option_end_off.unwrap();
    let afterchar = arg.get(end_off).copied().unwrap_or(0);
    let mut p = &arg[end_off..];

    while p.first().is_some_and(|&b| ascii_iswhite(i32::from(b))) {
        p = &p[1..];
    }

    let op = get_op(p);
    if op != OP_NONE {
        p = &p[1..];
    }

    let nextchar = i32::from(p.first().copied().unwrap_or(0));
    let opt = get_option(opt_idx);
    let flags = opt.flags;
    let varp = get_varp_scope(opt.clone(), opt_flags);

    let mut errmsg: Option<&'static CStr> = None;
    if !validate_opt_idx(curwin_mut(), opt_idx, opt_flags, flags, prefix, &mut errmsg) {
        return errmsg;
    }

    if crate::strings::vim_haschr(c"?=:!&<", nextchar) {
        *argp = p;
        let arg_bytes = *argp;

        if arg_bytes.starts_with(b"&vi") {
            let skip = if arg_bytes.starts_with(b"&vim") { 3 } else { 2 };
            *argp = &arg_bytes[skip..];
        }
        let arg_bytes = *argp;
        if matches!(nextchar as u8, b'?' | b'!' | b'&' | b'<')
            && arg_bytes.get(1).is_some_and(|&b| !ascii_iswhite(b as i32))
        {
            return Some(E_TRAILING);
        }
    }

    if nextchar == b'?' as i32
        || (prefix == PREFIX_NONE
            && !crate::strings::vim_haschr(c"=:&<", nextchar)
            && !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean))
    {
        if *did_show {
            msg_putchar(b'\n' as i32);
        } else {
            msg_ext_set_kind(c"list_cmd");
            gotocmdline(true);
            *did_show = true;
        }
        showoneopt(opt.clone(), opt_flags);

        if opt_vars().p_verbose.get() > 0 {
            if varp.is_some() && varp == opt.var() {
                last_set_msg(opt.script_ctx);
            } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
                last_set_msg(
                    curwin_mut().w_onebuf_opt.wo_script_ctx
                        [option_scope_idx(opt_idx, K_OPT_SCOPE_WIN) as usize]
                        .get(),
                );
            } else if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
                last_set_msg(
                    curbuf_mut().b_p_script_ctx
                        [option_scope_idx(opt_idx, K_OPT_SCOPE_BUF) as usize]
                        .get(),
                );
            }
        }

        if nextchar != b'?' as i32 && nextchar != 0 && !ascii_iswhite(afterchar) {
            return Some(E_TRAILING);
        }
        return None;
    }

    if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
        if crate::strings::vim_haschr(c"=:", nextchar) {
            return Some(E_INVARG);
        }

        if !crate::strings::vim_haschr(c"!&<", nextchar)
            && nextchar != 0
            && !ascii_iswhite(afterchar)
        {
            return Some(E_TRAILING);
        }
    } else if !crate::strings::vim_haschr(c"=:&<", nextchar) {
        return Some(E_INVARG);
    }

    let newval = get_option_newval(
        opt_idx,
        opt_flags,
        prefix,
        argp,
        nextchar,
        op,
        varp,
        None,
        &mut errmsg,
    );

    if newval.obj_type == ObjectType::ObjectTypeNil || errmsg.is_some() {
        return errmsg;
    }

    set_option(opt_idx, newval, opt_flags, 0, false, op == OP_NONE, errbuf)
}

/// Parse 'arg' for option settings.
pub fn do_set(arg: &std::ffi::CStr, opt_flags: i32) -> bool {
    let mut arg = arg.to_bytes();
    let mut did_show = false;

    if arg.is_empty() {
        showoptions(false, opt_flags);
        did_show = true;
    } else {
        while !arg.is_empty() {
            if arg.starts_with(b"all")
                && !arg.get(3).is_some_and(u8::is_ascii_alphabetic)
                && (opt_flags & OPT_MODELINE) == 0
            {
                arg = &arg[3..];
                if arg.first() == Some(&b'&') {
                    arg = &arg[1..];
                    set_options_default(opt_flags);
                    didset_options_all();
                    didset_options();
                    didset_options2();
                    ui_refresh_options();
                    redraw_all_later(UPD_CLEAR);
                } else {
                    showoptions(true, opt_flags);
                    did_show = true;
                }
            } else {
                let startarg = arg;
                let mut errbuf_storage = [0u8; ERR_BUFLEN];
                let errmsg = do_one_set_option(
                    opt_flags,
                    &mut arg,
                    &mut did_show,
                    Some(&mut errbuf_storage),
                );

                for _ in 0..2 {
                    let mut idx = 0;
                    while idx < arg.len() {
                        if arg[idx] == b'\\' && idx + 1 < arg.len() {
                            idx += 2;
                            continue;
                        }
                        if ascii_iswhite(i32::from(arg[idx])) {
                            break;
                        }
                        idx += 1;
                    }
                    arg = &arg[idx..];
                    while arg.first().is_some_and(|&b| ascii_iswhite(i32::from(b))) {
                        arg = &arg[1..];
                    }
                    if arg.first() != Some(&b'=') {
                        break;
                    }
                }

                if let Some(errmsg) = errmsg {
                    {
                        let iobuf = &mut *g_st().iobuff_bytes_mut();
                        let msg_bytes = tr(errmsg).to_bytes();
                        let mut pos = msg_bytes.len().min(IOSIZE - 1);
                        iobuf[..pos].copy_from_slice(&msg_bytes[..pos]);
                        let arg_len = startarg.len() - arg.len();
                        if pos + 2 + arg_len < IOSIZE {
                            iobuf[pos..pos + 2].copy_from_slice(b": ");
                            pos += 2;
                            iobuf[pos..pos + arg_len].copy_from_slice(&startarg[..arg_len]);
                            pos += arg_len;
                        }
                        iobuf[pos] = 0;
                        trans_characters(iobuf);
                    }

                    g_st().no_wait_return.set(g_st().no_wait_return.get() + (1));
                    emsg(g_st().iobuff_cstr());
                    g_st().no_wait_return.set(g_st().no_wait_return.get() - (1));

                    return false;
                }
            }

            while arg.first().is_some_and(|&b| ascii_iswhite(i32::from(b))) {
                arg = &arg[1..];
            }
        }
    }

    if g_st().silent_mode.get() && did_show {
        g_st().silent_mode.set(false);
        g_st().info_message.set(true);
        msg_putchar(b'\n' as i32);
        g_st().silent_mode.set(true);
        g_st().info_message.set(false);
    }

    true
}

fn find_key_len(arg_with_lt: &[u8], len: usize, has_lt: bool) -> i32 {
    let mut key: i32 = 0;
    let arg_b = if has_lt {
        &arg_with_lt[usize::from(!arg_with_lt.is_empty())..]
    } else {
        arg_with_lt
    };
    let bytes = &arg_b[..len.saturating_add(1).min(arg_b.len())];

    if len >= 4 && bytes.starts_with(b"t_") && bytes.len() >= 4 {
        if !has_lt || bytes.get(4) == Some(&b'>') {
            key = termcap2key(i32::from(bytes[2]), i32::from(bytes[3]));
        }
    } else if has_lt {
        let mut modifiers: i32 = 0;
        let mut arg_slice = &arg_with_lt[..(len + 1).min(arg_with_lt.len())];
        key = find_special_key(
            &mut arg_slice,
            &mut modifiers,
            FSK_KEYCODE | FSK_KEEP_X_KEY | FSK_SIMPLIFY,
            None,
        );
        if modifiers != 0 {
            key = 0;
        }
    }
    key
}

/// Convert a key name or string into a key value.
pub fn string_to_key(arg: &[u8]) -> i32 {
    let bytes = crate::memory::bytes_until_nul(arg);
    if bytes.len() >= 2 && bytes[0] == b'<' {
        return find_key_len(bytes, bytes.len(), true);
    }
    if bytes.len() >= 2 && bytes[0] == b'^' {
        let mut key = ctrl_chr(i32::from(bytes[1]));
        if key == 0 {
            key = K_ZERO;
        }
        return key;
    }
    bytes.first().copied().map_or(0, i32::from)
}

pub fn did_set_title() {
    if starting_val() != NO_SCREEN {
        maketitle();
    }
}

/// set_options_bin - called when 'bin' changes value.
pub fn set_options_bin(oldval: i32, newval: i32, opt_flags: i32) {
    if newval != 0 {
        if oldval == 0 {
            if (opt_flags & OPT_GLOBAL) == 0 {
                curbuf_mut().b_p_tw_nobin.set(curbuf_mut().b_p_tw.get());
                curbuf_mut().b_p_wm_nobin.set(curbuf_mut().b_p_wm.get());
                curbuf_mut().b_p_ml_nobin.set(curbuf_mut().b_p_ml.get());
                curbuf_mut().b_p_et_nobin.set(curbuf_mut().b_p_et.get());
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                OPT_STATE.with(|s| s.p_tw_nobin.set(opt_vars().p_tw.get()));
                OPT_STATE.with(|s| s.p_wm_nobin.set(opt_vars().p_wm.get()));
                OPT_STATE.with(|s| s.p_ml_nobin.set(opt_vars().p_ml.get()));
                OPT_STATE.with(|s| s.p_et_nobin.set(opt_vars().p_et.get()));
            }
        }

        if (opt_flags & OPT_GLOBAL) == 0 {
            curbuf_mut().b_p_tw.set(0);
            curbuf_mut().b_p_wm.set(0);
            curbuf_mut().b_p_ml.set(false);
            curbuf_mut().b_p_et.set(false);
        }
        if (opt_flags & OPT_LOCAL) == 0 {
            opt_vars().p_tw.set(0);
            opt_vars().p_wm.set(0);
            opt_vars().p_ml.set(false);
            opt_vars().p_et.set(false);
            opt_vars().p_bin.set(true);
        }
    } else if oldval != 0 {
        if (opt_flags & OPT_GLOBAL) == 0 {
            curbuf_mut().b_p_tw.set(curbuf_mut().b_p_tw_nobin.get());
            curbuf_mut().b_p_wm.set(curbuf_mut().b_p_wm_nobin.get());
            curbuf_mut().b_p_ml.set(curbuf_mut().b_p_ml_nobin.get());
            curbuf_mut().b_p_et.set(curbuf_mut().b_p_et_nobin.get());
        }
        if (opt_flags & OPT_LOCAL) == 0 {
            opt_vars().p_tw.set(OPT_STATE.with(|s| s.p_tw_nobin.get()));
            opt_vars().p_wm.set(OPT_STATE.with(|s| s.p_wm_nobin.get()));
            opt_vars().p_ml.set(OPT_STATE.with(|s| s.p_ml_nobin.get()));
            opt_vars().p_et.set(OPT_STATE.with(|s| s.p_et_nobin.get()));
        }
    }

    didset_options_sctx(opt_flags, P_BIN_DEP_OPTS);
}

/// Expand environment variables for some string options.
fn option_expand(opt_idx: OptIndex, val: Option<&std::ffi::CStr>) -> Option<&'static CStr> {
    let opt = get_option(opt_idx);
    if (opt.flags & K_OPT_FLAG_EXPAND) == 0 || is_option_hidden(opt_idx) {
        return None;
    }

    let val_cstr = val.or_else(|| opt.var().opt_cstr())?;
    if val_cstr.to_bytes().len() > MAXPATHL {
        return None;
    }

    let var = opt.var();
    let esc = var.ptr_eq(&opt_vars().p_tags) || var.ptr_eq(&opt_vars().p_path);
    expand_env_esc(
        val_cstr,
        &mut *g_st().name_buff_bytes_mut(),
        esc.then_some(c" \t"),
        false,
        var.ptr_eq(&opt_vars().p_sps).then_some(c"file:"),
    );
    let namebuf_cstr = g_st().name_buff_cstr();
    if namebuf_cstr.to_bytes() == val_cstr.to_bytes() {
        return None;
    }

    Some(namebuf_cstr)
}

/// After setting various option values: recompute variables that depend on option values.
fn didset_options() {
    buf_init_isk_chartab(curbuf_mut());
    init_isf_chartab();
    init_isi_chartab();

    didset_string_options();

    let mut sps_flags: i32 = 0;
    let mut sps_limit: i32 = 0;
    if spell_check_sps(opt_vars().p_sps.as_cstr(), &mut sps_flags, &mut sps_limit) {
        spell_apply_sps(sps_flags, sps_limit);
    }
    let ws = curwin_mut().w_s_ptr();
    compile_cap_prog(ws.b_p_spc.as_cstr(), &mut ws.b_cap_prog);
    did_set_spell_option();
    let mut dummy = Optset::new();
    did_set_cedit(&mut dummy);
    did_set_breakat(&mut dummy);
    didset_window_options(curwin_mut(), true);
}

fn didset_options2() {
    highlight_changed();

    set_chars_option(
        curwin_mut(),
        curwin_mut().w_onebuf_opt.wo_fcs.as_cstr(),
        CharsOption::Fillchars,
        true,
        None,
    );
    set_chars_option(
        curwin_mut(),
        curwin_mut().w_onebuf_opt.wo_lcs.as_cstr(),
        CharsOption::Listchars,
        true,
        None,
    );

    let mut wim_flags = [0u8; 4];
    check_opt_wim(opt_vars().p_wim.as_bytes(), &mut wim_flags);
    for (dst, src) in g_st().wim_flags.iter().zip(wim_flags) {
        dst.set(src);
    }
    tabstop_set(
        curbuf_mut().b_p_vsts.as_bytes(),
        &mut curbuf_mut().b_p_vsts_array,
        None,
    );
    tabstop_set(
        curbuf_mut().b_p_vts.as_bytes(),
        &mut curbuf_mut().b_p_vts_array,
        None,
    );
}

/// Repair UI state after `:set all&`.
fn didset_options_all() {
    let errmsg = parse_shape_opt(SHAPE_CURSOR);
    assert!(errmsg.is_none());
    last_status(false);
    win_float_update_statusline();
    win_new_screen_rows();
}

/// Check for string options that are NULL (normally only termcap options).
pub fn check_options() {
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        if option_has_type(opt_idx, ObjectType::ObjectTypeString)
            && !is_callback_option(opt_idx)
            && !opt.immutable
        {
            let varp = get_varp(opt);
            varp.check_str();
        }
    }
}

/// Check if option was set insecurely.
pub fn was_set_insecurely(wp: Option<&mut WinInfoInfo>, opt_idx: OptIndex, opt_flags: i32) -> i32 {
    assert!(opt_idx != K_OPT_INVALID);
    let flags = if (opt_flags & OPT_LOCAL) != 0 {
        let wp = wp.unwrap();
        match opt_idx {
            K_OPT_WRAP => Some(wp.w_onebuf_opt.wo_wrap_flags.get()),
            K_OPT_STATUSLINE => Some(wp.w_onebuf_opt.wo_stl_flags.get()),
            K_OPT_WINBAR => Some(wp.w_onebuf_opt.wo_wbr_flags.get()),
            K_OPT_FOLDEXPR => Some(wp.w_onebuf_opt.wo_fde_flags.get()),
            K_OPT_FOLDTEXT => Some(wp.w_onebuf_opt.wo_fdt_flags.get()),
            K_OPT_INDENTEXPR => Some(wp.buf_mut().b_p_inde_flags.get()),
            K_OPT_FORMATEXPR => Some(wp.buf_mut().b_p_fex_flags.get()),
            K_OPT_INCLUDEEXPR => Some(wp.buf_mut().b_p_inex_flags.get()),
            K_OPT_COMPLETE => Some(wp.buf_mut().b_p_cpt_flags.get()),
            _ => None,
        }
    } else if matches!(opt_idx, K_OPT_WRAP | K_OPT_FOLDEXPR | K_OPT_FOLDTEXT) {
        let wp = wp.unwrap();
        match opt_idx {
            K_OPT_WRAP => Some(wp.w_allbuf_opt.wo_wrap_flags.get()),
            K_OPT_FOLDEXPR => Some(wp.w_allbuf_opt.wo_fde_flags.get()),
            K_OPT_FOLDTEXT => Some(wp.w_allbuf_opt.wo_fdt_flags.get()),
            _ => None,
        }
    } else {
        None
    }
    .unwrap_or_else(|| get_option(opt_idx).flags);
    i32::from((flags & K_OPT_FLAG_INSECURE) != 0)
}

fn set_insecure_flag(
    wp: Option<&mut WinInfoInfo>,
    opt_idx: OptIndex,
    opt_flags: i32,
    insecure: bool,
) {
    let update_cell = |f: &std::cell::Cell<u32>| {
        if insecure {
            f.set(f.get() | K_OPT_FLAG_INSECURE);
        } else {
            f.set(f.get() & !K_OPT_FLAG_INSECURE);
        }
    };
    if (opt_flags & OPT_LOCAL) != 0 {
        let wp = wp.unwrap();
        match opt_idx {
            K_OPT_WRAP => return update_cell(&wp.w_onebuf_opt.wo_wrap_flags),
            K_OPT_STATUSLINE => return update_cell(&wp.w_onebuf_opt.wo_stl_flags),
            K_OPT_WINBAR => return update_cell(&wp.w_onebuf_opt.wo_wbr_flags),
            K_OPT_FOLDEXPR => return update_cell(&wp.w_onebuf_opt.wo_fde_flags),
            K_OPT_FOLDTEXT => return update_cell(&wp.w_onebuf_opt.wo_fdt_flags),
            K_OPT_INDENTEXPR => return update_cell(&wp.buf_mut().b_p_inde_flags),
            K_OPT_FORMATEXPR => return update_cell(&wp.buf_mut().b_p_fex_flags),
            K_OPT_INCLUDEEXPR => return update_cell(&wp.buf_mut().b_p_inex_flags),
            K_OPT_COMPLETE => return update_cell(&wp.buf_mut().b_p_cpt_flags),
            _ => {}
        }
    } else if matches!(opt_idx, K_OPT_WRAP | K_OPT_FOLDEXPR | K_OPT_FOLDTEXT) {
        let wp = wp.unwrap();
        match opt_idx {
            K_OPT_WRAP => return update_cell(&wp.w_allbuf_opt.wo_wrap_flags),
            K_OPT_FOLDEXPR => return update_cell(&wp.w_allbuf_opt.wo_fde_flags),
            K_OPT_FOLDTEXT => return update_cell(&wp.w_allbuf_opt.wo_fdt_flags),
            _ => {}
        }
    }
    OPTIONS.with(|opts| {
        let f = &mut opts.borrow_mut()[opt_idx as usize].flags;
        if insecure {
            *f |= K_OPT_FLAG_INSECURE;
        } else {
            *f &= !K_OPT_FLAG_INSECURE;
        }
    });
}

/// Redraw the window title and/or tab page text later.
pub fn redraw_titles() {
    g_st().need_maketitle.set(true);
    g_st().redraw_tabline.set(true);
}

/// Return true if "val" is a valid name.
pub fn valid_name(val: &[u8], allowed: &[u8]) -> bool {
    for &s_byte in val {
        if !ascii_isalnum(i32::from(s_byte)) && !allowed.contains(&s_byte) {
            return false;
        }
    }
    true
}

pub fn check_blending(wp: &mut WinInfoInfo) {
    wp.w_grid_alloc
        .blending
        .set(wp.w_onebuf_opt.wo_winbl.get() > 0 || (wp.w_floating.get() && wp.w_config.shadow));
}

/// Handle setting 'winhighlight' in window "wp".
pub fn parse_winhl_opt(winhl: Option<&CStr>, mut wp: Option<&mut WinInfoInfo>) -> bool {
    let winhl_owned = if winhl.is_none()
        && let Some(ref w) = wp
    {
        Some(w.w_onebuf_opt.wo_winhl.to_owned())
    } else {
        None
    };
    let p_cstr = winhl.or(winhl_owned.as_deref()).unwrap_or(c"");

    if p_cstr.to_bytes().is_empty() {
        if let Some(ref mut w) = wp
            && w.w_ns_hl_winhl.get() > 0
            && w.w_ns_hl.get() == w.w_ns_hl_winhl.get()
        {
            w.w_ns_hl.set(0);
            w.w_hl_needs_update.set(1);
        }
        return true;
    }

    let mut ns_hl: i32 = 0;
    if let Some(ref mut w) = wp {
        if w.w_ns_hl_winhl.get() == 0 {
            w.w_ns_hl_winhl
                .set(nvim_create_namespace(NvimString::new()) as i32);
        } else {
            with_decor_provider_mut(w.w_ns_hl_winhl.get(), true, |dp| dp.hl_valid += 1);
        }
        ns_hl = w.w_ns_hl_winhl.get();
        if w.w_ns_hl.get() <= 0 {
            w.w_ns_hl.set(w.w_ns_hl_winhl.get());
        }
    }

    let mut p = p_cstr.to_bytes();
    while let Some((key, val)) = option_next_keyval(&mut p) {
        let Some(val) = val else {
            return false;
        };
        let hl_id = if !val.is_empty() {
            syn_check_group(val)
        } else {
            -1
        };
        if hl_id == 0 {
            return false;
        }
        let hl_id_link = if !key.is_empty() {
            syn_check_group(key)
        } else {
            0
        };
        if hl_id_link == 0 {
            return false;
        }

        if wp.is_some() {
            let attrs = HlAttrs {
                rgb_ae_attr: HL_GLOBAL,
                cterm_ae_attr: 0,
                rgb_fg_color: -1,
                rgb_bg_color: -1,
                rgb_sp_color: -1,
                cterm_fg_color: 0,
                cterm_bg_color: 0,
                hl_blend: -1,
                url: -1,
                font: -1,
            };
            ns_hl_def(ns_hl, hl_id_link, attrs, hl_id, None);
        }
    }

    if let Some(wp) = wp {
        wp.w_hl_needs_update.set(1);
    }
    true
}

/// Get the script context of global option at index opt_idx.
pub fn get_option_sctx(opt_idx: OptIndex) -> Sctx {
    assert!(opt_idx != K_OPT_INVALID);
    get_option(opt_idx).script_ctx
}

/// Set the script_ctx for an option.
pub fn set_option_sctx(opt_idx: OptIndex, opt_flags: i32, mut script_ctx: Sctx) {
    let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;

    if (opt_flags & OPT_MODELINE) == 0 {
        script_ctx.sc_lnum += sourcing_lnum();
    }
    nlua_set_sctx(&mut script_ctx);

    if both || (opt_flags & OPT_GLOBAL) != 0 || option_is_global_only(opt_idx) {
        OPTIONS.with(|opts| opts.borrow_mut()[opt_idx as usize].script_ctx = script_ctx);
    }
    if both || (opt_flags & OPT_LOCAL) != 0 {
        if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
            curbuf_mut().b_p_script_ctx[option_scope_idx(opt_idx, K_OPT_SCOPE_BUF) as usize]
                .set(script_ctx);
        } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
            let win_idx = option_scope_idx(opt_idx, K_OPT_SCOPE_WIN) as usize;
            curwin_mut().w_onebuf_opt.wo_script_ctx[win_idx].set(script_ctx);
            if both {
                curwin_mut().w_allbuf_opt.wo_script_ctx[win_idx].set(script_ctx);
            }
        }
    }
}

/// Execute OptionSet autocmd now (not deferred).
pub fn apply_optionset_autocmd_now(
    opt_idx: OptIndex,
    opt_flags: i32,
    oldval: Object,
    oldval_g: Object,
    oldval_l: Object,
    newval: Object,
) {
    if starting_val() != 0 || !get_vim_var_str(VV_OPTION_TYPE).is_empty() {
        return;
    }

    let mut buf_type = [0u8; 7];
    let mut oldval_tv = opt_to_tv(oldval, false);
    let mut oldval_g_tv = opt_to_tv(oldval_g, false);
    let mut oldval_l_tv = opt_to_tv(oldval_l, false);
    let mut newval_tv = opt_to_tv(newval, false);

    set_vim_var_tv(VV_OPTION_OLD, &mut oldval_tv);
    set_vim_var_tv(VV_OPTION_NEW, &mut newval_tv);
    let typelen = format_to_buf_safelen!(
        &mut buf_type,
        c"%s",
        if (opt_flags & OPT_LOCAL) != 0 {
            c"local"
        } else {
            c"global"
        },
    );
    set_vim_var_string(
        VV_OPTION_TYPE,
        Some(crate::memory::bytes_as_cstr(&buf_type)),
        typelen as isize,
    );
    if (opt_flags & OPT_LOCAL) != 0 {
        set_vim_var_string(
            VV_OPTION_COMMAND,
            Some(c"setlocal"),
            "setlocal".len() as isize,
        );
        set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_tv);
    }
    if (opt_flags & OPT_GLOBAL) != 0 {
        set_vim_var_string(
            VV_OPTION_COMMAND,
            Some(c"setglobal"),
            "setglobal".len() as isize,
        );
        set_vim_var_tv(VV_OPTION_OLDGLOBAL, &mut oldval_tv);
    }
    if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0 {
        set_vim_var_string(VV_OPTION_COMMAND, Some(c"set"), "set".len() as isize);
        set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_l_tv);
        set_vim_var_tv(VV_OPTION_OLDGLOBAL, &mut oldval_g_tv);
    }
    if (opt_flags & OPT_MODELINE) != 0 {
        set_vim_var_string(
            VV_OPTION_COMMAND,
            Some(c"modeline"),
            "modeline".len() as isize,
        );
        set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_tv);
    }
    apply_autocmds(
        EVENT_OPTIONSET,
        Some(get_option(opt_idx).fullname),
        None,
        false,
        None,
    );
    reset_v_option_vars();

    tv_clear(&mut oldval_tv);
    tv_clear(&mut oldval_g_tv);
    tv_clear(&mut oldval_l_tv);
    tv_clear(&mut newval_tv);
}

/// For 'modified', the event is deferred.
fn apply_optionset_autocmd(
    opt_idx: OptIndex,
    opt_flags: i32,
    oldval: Object,
    oldval_g: Object,
    oldval_l: Object,
    newval: Object,
) {
    if starting_val() != 0 {
        return;
    }
    if opt_idx == K_OPT_MODIFIED {
        aucmd_defer_modified(curbuf_mut(), newval.data.boolean());
        return;
    }
    apply_optionset_autocmd_now(opt_idx, opt_flags, oldval, oldval_g, oldval_l, newval);
}

/// Process the updated 'arabic' option value.
pub fn did_set_arabic<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let mut errmsg: Option<&CStr> = None;

    if win.w_onebuf_opt.wo_arab.get() {
        if !opt_vars().p_tbidi.get() {
            if !win.w_onebuf_opt.wo_rl.get() {
                win.w_onebuf_opt.wo_rl.set(true);
                changed_window_setting(&mut *win);
            }

            if !opt_vars().p_arshape.get() {
                opt_vars().p_arshape.set(true);
                redraw_all_later(UPD_NOT_VALID);
            }
        }

        if opt_vars().p_enc.as_bytes() != b"utf-8" {
            msg_source(HLF_W);
            msg(tr(W_ARABIC), HLF_W);
            set_vim_var_string(VV_WARNINGMSG, Some(tr(W_ARABIC)), -1);
        }

        opt_vars().p_deco.set(true);

        errmsg = set_option_value(
            K_OPT_KEYMAP,
            string_obj(NvimString::from_cstr(c"arabic")),
            OPT_LOCAL,
            true,
            args.os_errbuf.take(),
        );
    } else {
        if !opt_vars().p_tbidi.get() && win.w_onebuf_opt.wo_rl.get() {
            win.w_onebuf_opt.wo_rl.set(false);
            changed_window_setting(&mut *win);
        }

        win.buf_mut().b_p_iminsert.set(B_IMODE_NONE);
        win.buf_mut().b_p_imsearch.set(B_IMODE_USE_INSERT);
    }

    errmsg
}

/// Process the updated 'autochdir' option value.
pub fn did_set_autochdir<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    do_autochdir();
    None
}

/// Process the updated 'binary' option value.
pub fn did_set_binary<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    set_options_bin(
        i32::from(args.os_oldval.data.boolean()),
        i32::from(args.buf_mut().b_p_bin.get()),
        args.os_flags,
    );
    redraw_titles();
    None
}

/// Process the updated 'buflisted' option value.
pub fn did_set_buflisted<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();

    if args.os_oldval.data.boolean() != buf.b_p_bl.get() {
        apply_autocmds(
            if buf.b_p_bl.get() {
                EVENT_BUFADD
            } else {
                EVENT_BUFDELETE
            },
            None,
            None,
            true,
            args.os_buf.opt_mut(),
        );
    }
    None
}

/// Process the new 'cmdheight' option value.
pub fn did_set_cmdheight<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let old_value = args.os_oldval.data.integer();

    if opt_vars().p_ch.get() > OptInt::from(g_st().rows_global.get() - min_rows(&curtab_mut()) + 1)
    {
        opt_vars().p_ch.set(OptInt::from(
            g_st().rows_global.get() - min_rows(&curtab_mut()) + 1,
        ));
    }

    if (opt_vars().p_ch.get() != old_value
        || OptInt::from(
            tabline_height() + global_stl_height() + crate::globals::topframe_mut().fr_height.get(),
        ) != OptInt::from(g_st().rows_global.get()) - opt_vars().p_ch.get())
        && g_st().full_screen.get()
    {
        command_height();
    }

    None
}

/// Process the updated 'diff' option value.
pub fn did_set_diff<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    diff_buf_adjust(&mut *win);
    if foldmethod_is_diff(&mut *win) {
        fold_update_all(&mut *win);
    }
    None
}

/// Process the updated 'endoffile' or 'endofline' or 'fixendofline' or 'bomb' option value.
pub fn did_set_eof_eol_fixeol_bomb<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    redraw_titles();
    None
}

/// Process the updated 'equalalways' option value.
pub fn did_set_equalalways<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_vars().p_ea.get() && !args.os_oldval.data.boolean() {
        win_equal(args.os_win.opt(), false, 0);
    }
    None
}

/// Process the new 'foldenable' option value.
pub fn did_set_foldenable<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if args.os_varp.ptr_eq(&win.w_onebuf_opt.wo_fen.get())
        && args.os_oldval.data.boolean() != win.w_onebuf_opt.wo_fen.get()
        && foldmethod_is_diff(&mut *win)
        && win.w_onebuf_opt.wo_scb.get()
    {
        for wp in for_all_windows_ptr() {
            if !std::ptr::eq(wp, win)
                && foldmethod_is_diff(&mut *wp)
                && wp.w_onebuf_opt.wo_scb.get()
            {
                wp.w_onebuf_opt.wo_fen.set(win.w_onebuf_opt.wo_fen.get());
                changed_window_setting(&mut *wp);
            }
        }
    }
    None
}

/// Process the new 'foldlevel' option value.
pub fn did_set_foldlevel<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    new_fold_level();
    None
}

/// Process the new 'foldminlines' option value.
pub fn did_set_foldminlines<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    fold_update_all(args.win_mut());
    None
}

/// Process the new 'foldnestmax' option value.
pub fn did_set_foldnestmax<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if foldmethod_is_syntax(&mut *win) || foldmethod_is_indent(&mut *win) {
        fold_update_all(&mut *win);
    }
    None
}

/// Multicursor: handle updates to 'follow' option.
pub fn did_set_follow<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if args.os_buf == g_st().curbuf.get() {
        atom_follow_changed();
    }
    None
}

/// Process the new 'helpheight' option value.
pub fn did_set_helpheight<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !firstwin_mut().w_next.get().is_null()
        && curbuf_mut().b_help.get()
        && OptInt::from(curwin_mut().w_height.get()) < opt_vars().p_hh.get()
    {
        win_setheight(opt_vars().p_hh.get() as i32);
    }
    None
}

/// Process the updated 'hlsearch' option value.
pub fn did_set_hlsearch<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    set_no_hlsearch(false);
    None
}

/// Process the updated 'ignorecase' option value.
pub fn did_set_ignorecase<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_vars().p_hls.get() {
        redraw_all_later(UPD_SOME_VALID);
    }
    None
}

/// Process the new 'iminsert' option value.
pub fn did_set_iminsert<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    showmode();
    status_redraw_curbuf();
    None
}

/// Process the updated 'langnoremap' option value.
pub fn did_set_langnoremap<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    opt_vars().p_lrm.set(!opt_vars().p_lnr.get());
    None
}

/// Process the updated 'langremap' option value.
pub fn did_set_langremap<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    opt_vars().p_lnr.set(!opt_vars().p_lrm.get());
    None
}

/// Process the new 'laststatus' option value.
pub fn did_set_laststatus<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let old_value = args.os_oldval.data.integer();
    let value = args.os_newval.data.integer();

    if value == 3 && old_value != 3 {
        let new_h = crate::globals::topframe_mut().fr_height.get() - STATUS_HEIGHT;
        frame_new_height(&crate::globals::topframe_mut(), new_h, false, false, false);
        win_comp_pos();
        g_st().clear_cmdline.set(true);
    }
    if old_value == 3 && value != 3 {
        let new_h = crate::globals::topframe_mut().fr_height.get() + STATUS_HEIGHT;
        frame_new_height(&crate::globals::topframe_mut(), new_h, false, false, false);
        win_comp_pos();
    }

    status_redraw_curbuf();
    last_status(false);
    win_float_update_statusline();
    None
}

/// Process the updated 'lines' or 'columns' option value.
pub fn did_set_lines_or_columns<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_vars().p_lines.get() != OptInt::from(g_st().rows_global.get())
        || opt_vars().p_columns.get() != OptInt::from(g_st().columns_global.get())
    {
        if g_st().updating_screen.get() {
            let oldval = integer_obj(args.os_oldval.data.integer());
            set_option_varp(args.os_idx, args.os_varp, oldval);
        } else if g_st().full_screen.get() {
            screen_resize(
                opt_vars().p_columns.get() as i32,
                opt_vars().p_lines.get() as i32,
            );
        } else {
            g_st().rows_global.set(opt_vars().p_lines.get() as i32);
            g_st().columns_global.set(opt_vars().p_columns.get() as i32);
            check_screensize();
            let new_row =
                (OptInt::from(g_st().rows_global.get()) - opt_vars().p_ch.get().max(1)) as i32;
            if g_st().cmdline_row.get() > new_row
                && OptInt::from(g_st().rows_global.get()) > opt_vars().p_ch.get()
            {
                g_st().cmdline_row.set(new_row);
            }
        }
        if opt_vars().p_window.get() >= OptInt::from(g_st().rows_global.get())
            || !option_was_set(K_OPT_WINDOW)
        {
            opt_vars()
                .p_window
                .set(OptInt::from(g_st().rows_global.get() - 1));
        }
    }

    if opt_vars().p_sj.get() >= OptInt::from(g_st().rows_global.get()) && g_st().full_screen.get() {
        opt_vars()
            .p_sj
            .set(OptInt::from(g_st().rows_global.get() / 2));
    }

    None
}

/// Process the updated 'lisp' option value.
pub fn did_set_lisp<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    buf_init_isk_chartab(args.buf_mut());
    None
}

/// Process the updated 'modifiable' option value.
pub fn did_set_modifiable<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    redraw_titles();
    None
}

/// Process the updated 'modified' option value.
pub fn did_set_modified<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let new_bool = args.os_newval.data.boolean();
    if !new_bool {
        save_file_ff(buf);
    }
    redraw_titles();
    buf.b_modified_was_set.set(new_bool);
    None
}

/// Process the updated 'number' or 'relativenumber' option value.
pub fn did_set_number_relativenumber<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if !win.w_onebuf_opt.wo_stc.is_empty() {
        win.w_nrwidth_line_count.set(0);
    }
    check_signcolumn(None::<&CStr>, Some(win));
    None
}

/// Process the new 'numberwidth' option value.
pub fn did_set_numberwidth<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    args.win_mut().w_nrwidth_line_count.set(0);
    None
}

/// Process the updated 'paste' option value.
pub fn did_set_paste<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_vars().p_paste.get() {
        if !OPT_STATE.with(|s| s.old_p_paste.get()) {
            for buf in for_all_buffers_mut() {
                buf.b_p_tw_nopaste.set(buf.b_p_tw.get());
                buf.b_p_wm_nopaste.set(buf.b_p_wm.get());
                buf.b_p_sts_nopaste.set(buf.b_p_sts.get());
                buf.b_p_ai_nopaste.set(buf.b_p_ai.get());
                buf.b_p_et_nopaste.set(buf.b_p_et.get());
                buf.b_p_vsts_nopaste
                    .set((!buf.b_p_vsts.is_empty()).then(|| buf.b_p_vsts.to_owned()));
            }

            OPT_STATE.with(|s| s.save_sm.set(opt_vars().p_sm.get()));
            OPT_STATE.with(|s| s.save_sta.set(opt_vars().p_sta.get()));
            OPT_STATE.with(|s| s.save_ru.set(opt_vars().p_ru.get()));
            OPT_STATE.with(|s| s.save_ri.set(opt_vars().p_ri.get()));
            OPT_STATE.with(|s| s.p_ai_nopaste.set(opt_vars().p_ai.get()));
            OPT_STATE.with(|s| s.p_et_nopaste.set(opt_vars().p_et.get()));
            OPT_STATE.with(|s| s.p_sts_nopaste.set(opt_vars().p_sts.get()));
            OPT_STATE.with(|s| s.p_tw_nopaste.set(opt_vars().p_tw.get()));
            OPT_STATE.with(|s| s.p_wm_nopaste.set(opt_vars().p_wm.get()));
            P_VSTS_NOPASTE.with(|cell| {
                *cell.borrow_mut() =
                    (!opt_vars().p_vsts.is_empty()).then(|| opt_vars().p_vsts.to_owned());
            });
        }

        for buf in for_all_buffers_mut() {
            buf.b_p_tw.set(0);
            buf.b_p_wm.set(0);
            buf.b_p_sts.set(0);
            buf.b_p_ai.set(false);
            buf.b_p_et.set(false);
            buf.b_p_vsts.clear_option_str();
            buf.b_p_vsts_array.clear();
        }

        opt_vars().p_sm.set(false);
        opt_vars().p_sta.set(false);
        if opt_vars().p_ru.get() {
            status_redraw_all();
        }
        opt_vars().p_ru.set(false);
        opt_vars().p_ri.set(false);
        opt_vars().p_tw.set(0);
        opt_vars().p_wm.set(0);
        opt_vars().p_sts.set(0);
        opt_vars().p_ai.set(false);
        opt_vars().p_et.set(false);
        opt_vars().p_vsts.clear_option_str();
    } else if OPT_STATE.with(|s| s.old_p_paste.get()) {
        for buf in for_all_buffers_mut() {
            buf.b_p_tw.set(buf.b_p_tw_nopaste.get());
            buf.b_p_wm.set(buf.b_p_wm_nopaste.get());
            buf.b_p_sts.set(buf.b_p_sts_nopaste.get());
            buf.b_p_ai.set(buf.b_p_ai_nopaste.get());
            buf.b_p_et.set(buf.b_p_et_nopaste.get());
            buf.b_p_vsts = buf
                .b_p_vsts_nopaste
                .get()
                .as_deref()
                .map_or_else(OptString::new, OptString::from_cstr);
            if !buf.b_p_vsts.is_empty() {
                tabstop_set(buf.b_p_vsts.as_bytes(), &mut buf.b_p_vsts_array, None);
            } else {
                buf.b_p_vsts_array.clear();
            }
        }

        opt_vars().p_sm.set(OPT_STATE.with(|s| s.save_sm.get()));
        opt_vars().p_sta.set(OPT_STATE.with(|s| s.save_sta.get()));
        if opt_vars().p_ru.get() != OPT_STATE.with(|s| s.save_ru.get()) {
            status_redraw_all();
        }
        opt_vars().p_ru.set(OPT_STATE.with(|s| s.save_ru.get()));
        opt_vars().p_ri.set(OPT_STATE.with(|s| s.save_ri.get()));
        opt_vars()
            .p_ai
            .set(OPT_STATE.with(|s| s.p_ai_nopaste.get()));
        opt_vars()
            .p_et
            .set(OPT_STATE.with(|s| s.p_et_nopaste.get()));
        opt_vars()
            .p_sts
            .set(OPT_STATE.with(|s| s.p_sts_nopaste.get()));
        opt_vars()
            .p_tw
            .set(OPT_STATE.with(|s| s.p_tw_nopaste.get()));
        opt_vars()
            .p_wm
            .set(OPT_STATE.with(|s| s.p_wm_nopaste.get()));
        opt_vars().p_vsts.set(P_VSTS_NOPASTE.with(|cell| {
            cell.borrow()
                .as_deref()
                .map_or_else(OptString::new, OptString::from_cstr)
        }));
    }

    OPT_STATE.with(|s| s.old_p_paste.set(opt_vars().p_paste.get()));

    didset_options_sctx(OPT_LOCAL | OPT_GLOBAL, P_PASTE_DEP_OPTS);

    None
}

/// Validate the 'previewwindow' option.
pub fn validate_previewwindow<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    if (args.os_flags & OPT_GLOBAL) != 0 || !args.os_newval.data.boolean() {
        return None;
    }
    let win = args.os_win;
    let tab = if win == curwin_ptr() {
        g_st().curtab.get().clone()
    } else {
        crate::globals::TabpagePtr::from(win_find_tabpage(win.borrow()))
    };
    for wp in for_all_windows_in_tab_ptr(tab.borrow()) {
        if wp != win && wp.w_onebuf_opt.wo_pvw.get() {
            return Some(E_PREVIEW_WINDOW_ALREADY_EXISTS);
        }
    }
    None
}

/// Process the new 'pumblend' option value.
pub fn did_set_pumblend<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    hl_invalidate_blends();
    if pum_drawn() {
        pum_redraw();
    }
    None
}

/// Process the updated 'readonly' option value.
pub fn did_set_readonly<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();

    if !buf.b_p_ro.get() && (args.os_flags & OPT_LOCAL) == 0 {
        g_st().readonlymode.set(false);
    }

    if buf.b_p_ro.get() {
        buf.b_did_warn.set(false);
    }

    redraw_titles();
    None
}

/// Process the new 'scrollback' option value.
pub fn did_set_scrollback<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let old_value = args.os_oldval.data.integer();
    let value = args.os_newval.data.integer();

    if let Some(term) = buf.terminal_opt()
        && value < old_value
    {
        on_scrollback_option_changed(term);
    }
    None
}

/// Process the updated 'scrollbind' option value.
pub fn did_set_scrollbind<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();

    if !win.w_onebuf_opt.wo_scb.get() {
        return None;
    }
    do_check_scrollbind(false);
    let vtop = get_vtopline(&mut *win);
    win.w_scbind_pos.set(vtop);
    None
}

/// Process the new 'shiftwidth' or the 'tabstop' option value.
pub fn did_set_shiftwidth_tabstop<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    let win = args.win_mut();

    if foldmethod_is_indent(win) {
        fold_update_all(win);
    }
    if args.os_varp.ptr_eq(&buf.b_p_sw.get()) || buf.b_p_sw.get() == 0 {
        parse_cino(buf);
    }
    None
}

/// Process the new 'showtabline' option value.
pub fn did_set_showtabline<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    win_new_screen_rows();
    None
}

/// Process the updated 'smoothscroll' option value.
pub fn did_set_smoothscroll<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if !win.w_onebuf_opt.wo_sms.get() {
        win.w_skipcol.set(0);
    }
    None
}

/// Process the updated 'spell' option value.
pub fn did_set_spell<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if win.w_onebuf_opt.wo_spell.get() {
        return parse_spelllang(win);
    }
    None
}

/// Process the updated 'swapfile' option value.
pub fn did_set_swapfile<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();
    if buf.b_p_swf.get() && opt_vars().p_uc.get() != 0 {
        ml_open_file(buf);
    } else {
        mf_close_file(buf, true);
    }
    None
}

/// Process the new 'textwidth' option value.
pub fn did_set_textwidth<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    for tp in for_all_tabpages_ptr() {
        for wp in for_all_windows_in_tab_ptr(&tp) {
            check_colorcolumn(None::<&CStr>, Some(&mut *wp));
        }
    }
    None
}

/// Process the updated 'title' or the 'icon' option value.
pub fn did_set_title_icon<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    did_set_title();
    None
}

/// Process the new 'titlelen' option value.
pub fn did_set_titlelen<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let old_value = args.os_oldval.data.integer();
    if starting_val() != NO_SCREEN && old_value != opt_vars().p_titlelen.get() {
        g_st().need_maketitle.set(true);
    }
    None
}

/// Process the updated 'undofile' option value.
pub fn did_set_undofile<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.os_buf.as_ptr();

    if !args.buf_mut().b_p_udf.get() && !opt_vars().p_udf.get() {
        return None;
    }

    let mut hash = [0u8; UNDO_HASH_SIZE];

    for bp in for_all_buffers_ptr() {
        if (ptr::eq(buf, bp) || (args.os_flags & OPT_GLOBAL) != 0 || args.os_flags == 0)
            && !buf_is_changed_fn(&mut *bp)
            && bp.b_ml.ml_mfp.is_some()
        {
            u_compute_hash(&mut *bp, &mut hash);
            u_read_undo(None, &hash, bp.fname_opt());
        }
    }

    None
}

/// Process the new global 'undolevels' option value.
pub fn did_set_global_undolevels(value: OptInt, old_value: OptInt) -> Option<&'static CStr> {
    opt_vars().p_ul.set(old_value);
    u_sync(true);
    opt_vars().p_ul.set(value);
    None
}

/// Process the new buffer local 'undolevels' option value.
pub fn did_set_buflocal_undolevels(
    buf: &mut FileBuf,
    value: OptInt,
    old_value: OptInt,
) -> Option<&'static CStr> {
    buf.b_p_ul.set(old_value);
    u_sync(true);
    buf.b_p_ul.set(value);
    None
}

/// Process the new 'undolevels' option value.
pub fn did_set_undolevels<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let buf = args.buf_mut();

    if args.os_varp.ptr_eq(&opt_vars().p_ul) {
        did_set_global_undolevels(args.os_newval.data.integer(), args.os_oldval.data.integer());
    } else if args.os_varp.ptr_eq(&buf.b_p_ul.get()) {
        did_set_buflocal_undolevels(
            buf,
            args.os_newval.data.integer(),
            args.os_oldval.data.integer(),
        );
    }

    None
}

/// Process the new 'updatecount' option value.
pub fn did_set_updatecount<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let old_value = args.os_oldval.data.integer();
    if opt_vars().p_uc.get() != 0 && old_value == 0 {
        ml_open_files();
    }
    None
}

/// Validate the 'wildchar' or 'wildcharm' option.
pub fn validate_wildchar<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let c = args.os_newval.data.integer();
    if c == CTRL_C_CONST_CONST
        || c == OptInt::from(b'\n')
        || c == OptInt::from(b'\r')
        || c == OptInt::from(K_KENTER)
    {
        return Some(E_INVARG);
    }
    None
}

/// Process the new 'winblend' option value.
pub fn did_set_winblend<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    let old_value = args.os_oldval.data.integer();
    let value = args.os_newval.data.integer();

    if value != old_value {
        win.w_onebuf_opt
            .wo_winbl
            .set(win.w_onebuf_opt.wo_winbl.get().clamp(0, 100));
        win.w_hl_needs_update.set(1);
        check_blending(&mut *win);
    }
    None
}

/// Process the new 'window' option value.
pub fn did_set_window<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if opt_vars().p_window.get() < 1
        || opt_vars().p_window.get() >= OptInt::from(g_st().rows_global.get())
    {
        opt_vars()
            .p_window
            .set(OptInt::from(g_st().rows_global.get() - 1));
    }
    None
}

/// Process the new 'winheight' value.
pub fn did_set_winheight<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !firstwin_mut().w_next.get().is_null()
        && OptInt::from(curwin_mut().w_height.get()) < opt_vars().p_wh.get()
    {
        win_setheight(opt_vars().p_wh.get() as i32);
    }
    None
}

/// Process the new 'winwidth' option value.
pub fn did_set_winwidth<'a>(_args: &mut Optset<'a>) -> Option<&'a CStr> {
    if !firstwin_mut().w_next.get().is_null()
        && OptInt::from(curwin_mut().w_width.get()) < opt_vars().p_wiw.get()
    {
        win_setwidth(opt_vars().p_wiw.get() as i32);
    }
    None
}

/// Process the updated 'wrap' option value.
pub fn did_set_wrap<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let win = args.win_mut();
    if win.w_onebuf_opt.wo_wrap.get() {
        win.w_leftcol.set(0);
    } else {
        win.w_skipcol.set(0);
    }
    None
}

/// Process the new 'chistory' or 'lhistory' option value.
pub fn did_set_xhistory<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let is_p_chi = args.os_varp.ptr_eq(&opt_vars().p_chi);

    if is_p_chi {
        qf_resize_stack(opt_vars().p_chi.get() as i32);
    } else {
        let val = args.os_varp.int_val() as i32;
        ll_resize_stack(args.win_mut(), val);
    }

    None
}

fn do_syntax_autocmd(buf: &mut FileBuf, value_changed: bool) {
    OPT_STATE.with(|s| s.syn_recursive.set(s.syn_recursive.get() + (1)));
    buf.b_flags.set(buf.b_flags.get() | (BF_SYN_SET));
    let syn_name = buf.b_p_syn.to_owned();
    let buf_fname = buf.fname_opt().map(CStr::to_owned);
    apply_autocmds(
        EVENT_SYNTAX,
        Some(&syn_name),
        buf_fname.as_deref(),
        value_changed || OPT_STATE.with(|s| s.syn_recursive.get()) == 1,
        Some(buf),
    );
    OPT_STATE.with(|s| s.syn_recursive.set(s.syn_recursive.get() - (1)));
}

fn do_spelllang_source(win: &mut WinInfoInfo) {
    let mut fname = [0u8; 200];
    let ws = win.w_s_ptr();
    let mut q = ws.b_p_spl.as_cstr();

    if q.to_bytes().starts_with(b"cjk,") {
        q = &q[4..];
    }

    let len = q
        .to_bytes()
        .iter()
        .take_while(|&&b| ascii_isalnum(i32::from(b)) || b == b'-')
        .count();
    if len > 0 {
        format_to_buf!(&mut fname, c"spell/%.*s.*", len as i32, q);
        source_runtime_vim_lua(crate::memory::bytes_as_cstr(&fname), DIP_ALL);
    }
}

/// Check the bounds of numeric options.
fn check_num_option_bounds<'a>(
    opt_idx: OptIndex,
    newval: &mut OptInt,
    win: &mut WinInfoInfo,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let mut errmsg: Option<&'a CStr> = None;

    match opt_idx {
        K_OPT_LINES => {
            let min_r = min_rows_for_all_tabpages();
            if *newval < OptInt::from(min_r) && g_st().full_screen.get() {
                errmsg = opt_error_int(errbuf, c"E593: Need at least %d lines", min_r);
                *newval = OptInt::from(min_r);
            }
            *newval = (*newval).min(OptInt::from(i32::MAX));
        }
        K_OPT_COLUMNS => {
            if *newval < OptInt::from(MIN_COLUMNS) && g_st().full_screen.get() {
                errmsg = opt_error_int(errbuf, c"E594: Need at least %d columns", MIN_COLUMNS);
                *newval = OptInt::from(MIN_COLUMNS);
            }
            *newval = (*newval).min(OptInt::from(i32::MAX));
        }
        K_OPT_PUMBLEND => {
            *newval = (*newval).clamp(0, 100);
        }
        K_OPT_SCROLLJUMP => {
            if (*newval < -100 || *newval >= OptInt::from(g_st().rows_global.get()))
                && g_st().full_screen.get()
            {
                errmsg = Some(E_SCROLL);
                *newval = 1;
            }
        }
        K_OPT_SCROLL
            if (*newval <= 0
                || (*newval > OptInt::from(win.w_view_height.get())
                    && win.w_view_height.get() > 0))
                && g_st().full_screen.get() =>
        {
            if *newval != 0 {
                errmsg = Some(E_SCROLL);
            }
            *newval = OptInt::from(win_default_scroll(win));
        }
        _ => {}
    }

    errmsg
}

/// Validate and bound check option value.
fn validate_num_option<'a>(
    opt_idx: OptIndex,
    newval: &mut OptInt,
    win: &mut WinInfoInfo,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    let value = *newval;

    if value < OptInt::from(i32::MIN) || value > OptInt::from(i32::MAX) {
        return Some(E_INVARG);
    }

    const MAX_SEARCH_COUNT: OptInt = 9999;

    match opt_idx {
        K_OPT_HELPHEIGHT | K_OPT_TITLELEN | K_OPT_UPDATECOUNT | K_OPT_REPORT | K_OPT_UPDATETIME
        | K_OPT_SIDESCROLL | K_OPT_FOLDLEVEL | K_OPT_SHIFTWIDTH | K_OPT_TEXTWIDTH
        | K_OPT_WRITEDELAY | K_OPT_TIMEOUTLEN => {
            if value < 0 {
                return Some(E_POSITIVE);
            }
        }
        K_OPT_WINHEIGHT => {
            if value < 1 {
                return Some(E_POSITIVE);
            } else if opt_vars().p_wmh.get() > value {
                return Some(E_WINHEIGHT);
            }
        }
        K_OPT_WINMINHEIGHT => {
            if value < 0 {
                return Some(E_POSITIVE);
            } else if value > opt_vars().p_wh.get() {
                return Some(E_WINHEIGHT);
            }
        }
        K_OPT_WINWIDTH => {
            if value < 1 {
                return Some(E_POSITIVE);
            } else if opt_vars().p_wmw.get() > value {
                return Some(E_WINWIDTH);
            }
        }
        K_OPT_WINMINWIDTH => {
            if value < 0 {
                return Some(E_POSITIVE);
            } else if value > opt_vars().p_wiw.get() {
                return Some(E_WINWIDTH);
            }
        }
        K_OPT_MAXCOMBINE => {
            *newval = MAX_MCO;
        }
        K_OPT_CMDHEIGHT => {
            if value < 0 {
                return Some(E_POSITIVE);
            }
        }
        K_OPT_HISTORY => {
            if value < 0 {
                return Some(E_POSITIVE);
            } else if value > 10000 {
                return Some(E_INVARG);
            }
        }
        K_OPT_PYXVERSION => {
            if value == 0 {
                *newval = 3;
            } else if value != 3 {
                return Some(E_INVARG);
            }
        }
        K_OPT_REGEXPENGINE => {
            if !(0..=2).contains(&value) {
                return Some(E_INVARG);
            }
        }
        K_OPT_SCROLLOFF => {
            if value < 0 && g_st().full_screen.get() {
                return Some(E_POSITIVE);
            }
        }
        K_OPT_SCROLLOFFPAD => {
            if value < 0 {
                return Some(E_INVARG);
            }
        }
        K_OPT_SIDESCROLLOFF => {
            if value < 0 && g_st().full_screen.get() {
                return Some(E_POSITIVE);
            }
        }
        K_OPT_CMDWINHEIGHT => {
            if value < 1 {
                return Some(E_POSITIVE);
            }
        }
        K_OPT_CONCEALLEVEL => {
            if value < 0 {
                return Some(E_POSITIVE);
            } else if value > 3 {
                return Some(E_INVARG);
            }
        }
        K_OPT_NUMBERWIDTH => {
            if value < 1 {
                return Some(E_POSITIVE);
            } else if value > MAX_NUMBERWIDTH {
                return Some(E_INVARG);
            }
        }
        K_OPT_IMINSERT => {
            if !(0..=B_IMODE_LAST).contains(&value) {
                return Some(E_INVARG);
            }
        }
        K_OPT_IMSEARCH => {
            if !(-1..=B_IMODE_LAST).contains(&value) {
                return Some(E_INVARG);
            }
        }
        K_OPT_CHANNEL => {
            return Some(E_INVARG);
        }
        K_OPT_SCROLLBACK => {
            if !(-1..=SB_MAX).contains(&value) {
                return Some(E_INVARG);
            }
        }
        K_OPT_TABSTOP => {
            if value < 1 {
                return Some(E_POSITIVE);
            } else if value > OptInt::from(crate::indent::TABSTOP_MAX) {
                return Some(E_INVARG);
            }
        }
        K_OPT_CHISTORY | K_OPT_LHISTORY => {
            if value < 1 {
                return Some(E_CANNOT_HAVE_NEGATIVE_OR_ZERO_NUMBER_OF_QUICKFIX);
            } else if value > 100 {
                return Some(E_CANNOT_HAVE_MORE_THAN_HUNDRED_QUICKFIX);
            }
        }
        K_OPT_MAXSEARCHCOUNT => {
            if value <= 0 {
                return Some(E_POSITIVE);
            } else if value > MAX_SEARCH_COUNT {
                return Some(E_INVARG);
            }
        }
        _ => {}
    }

    check_num_option_bounds(opt_idx, newval, win, errbuf)
}

/// Called after an option changed: check if something needs to be redrawn.
pub fn check_redraw_for(buf: &FileBuf, win: &mut WinInfoInfo, flags: u32) {
    let all = (flags & K_OPT_FLAG_REDR_ALL) == K_OPT_FLAG_REDR_ALL;

    if (flags & K_OPT_FLAG_REDR_STAT) != 0 || all {
        status_redraw_all();
    }

    if (flags & K_OPT_FLAG_REDR_TABL) != 0 || all {
        g_st().redraw_tabline.set(true);
    }

    if (flags & K_OPT_FLAG_REDR_BUF) != 0 || (flags & K_OPT_FLAG_REDR_WIN) != 0 || all {
        if (flags & K_OPT_FLAG_HLONLY) != 0 {
            redraw_later(Some(&mut *win), UPD_NOT_VALID);
        } else {
            changed_window_setting(win);
        }
    }
    if (flags & K_OPT_FLAG_REDR_BUF) != 0 {
        redraw_buf_later(buf, UPD_NOT_VALID);
    }
    if all {
        redraw_all_later(UPD_NOT_VALID);
    }
}

pub fn check_redraw(flags: u32) {
    check_redraw_for(curbuf_mut(), curwin_mut(), flags);
}

pub fn is_tty_option(name: &[u8]) -> bool {
    find_tty_option_len(name).is_some()
}

/// Get value of TTY option.
pub fn get_tty_option(name: &[u8]) -> Object {
    let name = crate::memory::bytes_until_nul(name);
    if name == b"t_Co" {
        if g_st().t_colors.get() <= 1 {
            cstr_to_obj(Some(c""))
        } else {
            let mut value = [0u8; NUMBUFLEN];
            format_to_buf!(&mut value, c"%d", g_st().t_colors.get());
            cstr_to_obj(Some(crate::memory::bytes_as_cstr(&value)))
        }
    } else if name == b"term" {
        P_TERM.with(|cell| cstr_to_obj(Some(cell.borrow().as_deref().unwrap_or(c"nvim"))))
    } else if name == b"ttytype" {
        P_TTYTYPE.with(|cell| cstr_to_obj(Some(cell.borrow().as_deref().unwrap_or(c"nvim"))))
    } else if is_tty_option(name) {
        cstr_to_obj(Some(c""))
    } else {
        nil_obj()
    }
}

pub fn set_tty_option(name: &std::ffi::CStr, value: Option<std::ffi::CString>) -> bool {
    if Some(name.to_bytes()) == Some(b"term") {
        P_TERM.with(|cell| *cell.borrow_mut() = value);
        return true;
    }

    if Some(name.to_bytes()) == Some(b"ttytype") {
        P_TTYTYPE.with(|cell| *cell.borrow_mut() = value);
        return true;
    }

    false
}

const CPO_VIM_CSTR: &CStr = c"aABceFs_";
const HIGHLIGHT_INIT_CSTR: &CStr = c"8:SpecialKey,~:EndOfBuffer,z:TermCursor,@:NonText,d:Directory,e:ErrorMsg,i:IncSearch,l:Search,y:CurSearch,m:MoreMsg,M:ModeMsg,n:LineNr,a:LineNrAbove,b:LineNrBelow,N:CursorLineNr,G:CursorLineSign,O:CursorLineFold,r:Question,s:StatusLine,S:StatusLineNC,c:VertSplit,t:Title,v:Visual,V:VisualNOS,w:WarningMsg,W:WildMenu,f:Folded,F:FoldColumn,A:DiffAdd,C:DiffChange,D:DiffDelete,T:DiffText,E:DiffTextAdd,>:SignColumn,-:Conceal,B:SpellBad,P:SpellCap,R:SpellRare,L:SpellLocal,+:Pmenu,=:PmenuSel,k:PmenuMatch,<:PmenuMatchSel,[:PmenuKind,]:PmenuKindSel,{:PmenuExtra,}:PmenuExtraSel,x:PmenuSbar,X:PmenuThumb,*:TabLine,#:TabLineSel,_:TabLineFill,!:CursorColumn,.:CursorLine,o:ColorColumn,q:QuickFixLine,z:StatusLineTerm,Z:StatusLineTermNC,g:MsgArea,h:ComplMatchIns,0:Whitespace,I:PreInsert";
const LISPWORD_VALUE_CSTR: &CStr = c"defun,define,defmacro,set!,lambda,if,case,let,flet,let*,letrec,do,do*,define-syntax,let-syntax,letrec-syntax,destructuring-bind,defpackage,defparameter,defstruct,deftype,defvar,do-all-symbols,do-external-symbols,do-symbols,dolist,dotimes,ecase,etypecase,eval-when,labels,macrolet,multiple-value-bind,multiple-value-call,multiple-value-prog1,multiple-value-setq,prog1,progv,typecase,unless,unwind-protect,when,with-input-from-string,with-open-file,with-open-stream,with-output-to-string,with-package-iterator,define-condition,handler-bind,handler-case,restart-bind,restart-case,with-simple-restart,store-value,use-value,muffle-warning,abort,continue,with-slots,with-slots*,with-accessors,with-accessors*,defclass,defmethod,print-unreadable-object";
#[cfg(windows)]
const DFLT_EFM_CSTR: &CStr = c"%f(%l): %t%*\\D%n: %m,%f(%l\\,%c): %t%*\\D%n: %m,%f(%l) \\=: %t%*\\D%n: %m,%*[^\"]\"%f\"%*\\D%l: %m,%f(%l) \\=: %m,%*[^ ] %f %l: %m,%f:%l:%c:%m,%f(%l):%m,%f:%l:%m,%f|%l| %m";
#[cfg(not(windows))]
const DFLT_EFM_CSTR: &CStr = c"%*[^\"]\"%f\"%*\\D%l: %m,\"%f\"%*\\D%l: %m,%-Gg%\\?make[%*\\d]: *** [%f:%l:%m,%-Gg%\\?make: *** [%f:%l:%m,%-G%f:%l: (Each undeclared identifier is reported only once,%-G%f:%l: for each function it appears in.),%-GIn file included from %f:%l:%c:,%-GIn file included from %f:%l:%c\\,,%-GIn file included from %f:%l:%c,%-GIn file included from %f:%l,%-G%*[ ]from %f:%l:%c,%-G%*[ ]from %f:%l:,%-G%*[ ]from %f:%l\\,,%-G%*[ ]from %f:%l,%f:%l:%c:%m,%f(%l):%m,%f:%l:%m,\"%f\"\\, line %l%*\\D%c%*[^ ] %m,%D%*\\a[%*\\d]: Entering directory %*[`']%f',%X%*\\a[%*\\d]: Leaving directory %*[`']%f',%D%*\\a: Entering directory %*[`']%f',%X%*\\a: Leaving directory %*[`']%f',%DMaking %*\\a in %f,%f|%l| %m";
#[cfg(windows)]
const DFLT_GFN_CSTR: &CStr = c"Cascadia Code,Cascadia Mono,Consolas,Courier New,monospace";
#[cfg(target_os = "macos")]
const DFLT_GFN_CSTR: &CStr = c"SF Mono,Menlo,Monaco,Courier New,monospace";
#[cfg(target_os = "linux")]
const DFLT_GFN_CSTR: &CStr = c"Source Code Pro,DejaVu Sans Mono,Courier New,monospace";
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
const DFLT_GFN_CSTR: &CStr = c"DejaVu Sans Mono,Courier New,monospace";

macro_rules! cstr_def_val {
    ($s:expr) => {{
        const S: &CStr = $s;
        Object {
            obj_type: ObjectType::ObjectTypeString,
            data: ObjectData::String(NvimString::from_static_cstr(S)),
        }
    }};
}

macro_rules! opt_cb {
    ($cb:path) => {{ Some($cb) }};
}

macro_rules! opt_var {
    ($field:ident) => {
        Some(|| OptVar::from(crate::optionstr::OptRef::Global(|v| &v.$field)))
    };
}

macro_rules! buf_var {
    ($buf:expr, $field:ident) => {
        OptVar::from(crate::optionstr::OptRef::Buf(
            crate::globals::BufPtr::from(&*$buf),
            |b| &b.$field,
        ))
    };
}

macro_rules! win_onebuf_var {
    ($win:expr, $field:ident) => {
        OptVar::from(crate::optionstr::OptRef::Win(
            crate::globals::WinPtr::from(&*$win),
            |w| &w.w_onebuf_opt.$field,
        ))
    };
}

macro_rules! winopt_var {
    ($win:expr, $allbuf:expr, $field:ident) => {
        if $allbuf {
            OptVar::from(crate::optionstr::OptRef::Win(
                crate::globals::WinPtr::from(&*$win),
                |w| &w.w_allbuf_opt.$field,
            ))
        } else {
            win_onebuf_var!($win, $field)
        }
    };
}

macro_rules! opt_flags {
    ($field:ident) => {
        Some(|| &opt_vars().$field)
    };
}

macro_rules! opt_def_var {
    ($idx:literal) => {
        Some(|| OptVar::DefVal($idx))
    };
}

pub(crate) const fn init_options() -> [Vimoption; 379] {
    [
    Vimoption { fullname: c"aleph", shortname: Some(c"al"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(0), flags_var: None, scope_idx: [0, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(224) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"allowrevins", shortname: Some(c"ari"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ari), flags_var: None, scope_idx: [1, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ambiwidth", shortname: Some(c"ambw"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ambw), flags_var: None, scope_idx: [2, -1, -1, -1], immutable: false, values: &OPT_AMBW_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_ambiwidth), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"single"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"arabic", shortname: Some(c"arab"), flags: K_OPT_FLAG_CURSWANT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 0, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_arabic), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"arabicshape", shortname: Some(c"arshape"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_arshape), flags_var: None, scope_idx: [3, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autochdir", shortname: Some(c"acd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_acd), flags_var: None, scope_idx: [4, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_autochdir), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocomplete", shortname: Some(c"ac"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ac), flags_var: None, scope_idx: [5, -1, 0, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocompletedelay", shortname: Some(c"acl"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_acl), flags_var: None, scope_idx: [6, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocompletetimeout", shortname: Some(c"act"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_act), flags_var: None, scope_idx: [7, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autoindent", shortname: Some(c"ai"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ai), flags_var: None, scope_idx: [-1, -1, 1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autoread", shortname: Some(c"ar"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ar), flags_var: None, scope_idx: [8, -1, 2, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autowrite", shortname: Some(c"aw"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_aw), flags_var: None, scope_idx: [9, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autowriteall", shortname: Some(c"awa"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_awa), flags_var: None, scope_idx: [10, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"background", shortname: Some(c"bg"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bg), flags_var: None, scope_idx: [11, -1, -1, -1], immutable: false, values: &OPT_BG_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_background), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dark"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backspace", shortname: Some(c"bs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bs), flags_var: None, scope_idx: [12, -1, -1, -1], immutable: false, values: &OPT_BS_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backspace), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"indent,eol,start"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backup", shortname: Some(c"bk"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bk), flags_var: None, scope_idx: [13, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(unix)]
    Vimoption { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_bkc), flags_var: opt_flags!(bkc_flags), scope_idx: [14, -1, 3, -1], immutable: false, values: &OPT_BKC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(unix))]
    Vimoption { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_bkc), flags_var: opt_flags!(bkc_flags), scope_idx: [14, -1, 3, -1], immutable: false, values: &OPT_BKC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupdir", shortname: Some(c"bdir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bdir), flags_var: None, scope_idx: [15, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupext", shortname: Some(c"bex"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bex), flags_var: None, scope_idx: [16, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"~"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupskip", shortname: Some(c"bsk"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bsk), flags_var: None, scope_idx: [17, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"belloff", shortname: Some(c"bo"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bo), flags_var: opt_flags!(bo_flags), scope_idx: [18, -1, -1, -1], immutable: false, values: &OPT_BO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"all"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"binary", shortname: Some(c"bin"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bin), flags_var: None, scope_idx: [-1, -1, 4, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_binary), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"bomb", shortname: None, flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bomb), flags_var: None, scope_idx: [-1, -1, 5, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakat", shortname: Some(c"brk"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_breakat), flags_var: None, scope_idx: [19, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakat), opt_expand_cb: None, def_val: cstr_def_val!(c" \t!@*-+;:,./?"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakindent", shortname: Some(c"bri"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakindentopt", shortname: Some(c"briopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 2, -1, -1], immutable: false, values: &OPT_BRIOPT_VALUES, schema: &OPT_BRIOPT_SCHEMA, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakindentopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"browsedir", shortname: Some(c"bsdir"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(26), flags_var: None, scope_idx: [20, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"bufhidden", shortname: Some(c"bh"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bh), flags_var: None, scope_idx: [-1, -1, 6, -1], immutable: false, values: &OPT_BH_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"buflisted", shortname: Some(c"bl"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bl), flags_var: None, scope_idx: [-1, -1, 7, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_buflisted), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"buftype", shortname: Some(c"bt"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bt), flags_var: None, scope_idx: [-1, -1, 8, -1], immutable: false, values: &OPT_BT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_buftype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_buftype), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"busy", shortname: None, flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_busy), flags_var: None, scope_idx: [-1, -1, 9, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"casemap", shortname: Some(c"cmp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cmp), flags_var: opt_flags!(cmp_flags), scope_idx: [21, -1, -1, -1], immutable: false, values: &OPT_CMP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"internal,keepascii"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"cdhome", shortname: Some(c"cdh"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdh), flags_var: None, scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"cdhome", shortname: Some(c"cdh"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdh), flags_var: None, scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cdpath", shortname: Some(c"cd"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdpath), flags_var: None, scope_idx: [23, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c",,"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cedit", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cedit), flags_var: None, scope_idx: [24, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::ex_getln::validate_cedit), opt_did_set_cb: opt_cb!(crate::ex_getln::did_set_cedit), opt_expand_cb: None, def_val: cstr_def_val!(c"\x06"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"channel", shortname: None, flags: K_OPT_FLAG_NO_MKRC|K_OPT_FLAG_NO_DEFAULT, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_channel), flags_var: None, scope_idx: [-1, -1, 10, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"charconvert", shortname: Some(c"ccv"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ccv), flags_var: None, scope_idx: [25, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"chistory", shortname: Some(c"chi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_chi), flags_var: None, scope_idx: [26, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cindent", shortname: Some(c"cin"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cin), flags_var: None, scope_idx: [-1, -1, 11, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinkeys", shortname: Some(c"cink"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cink), flags_var: None, scope_idx: [-1, -1, 12, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinoptions", shortname: Some(c"cino"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cino), flags_var: None, scope_idx: [-1, -1, 13, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cinoptions), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinscopedecls", shortname: Some(c"cinsd"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cinsd), flags_var: None, scope_idx: [-1, -1, 14, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"public,protected,private"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinwords", shortname: Some(c"cinw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cinw), flags_var: None, scope_idx: [-1, -1, 15, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"if,else,while,do,for,switch"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"clipboard", shortname: Some(c"cb"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cb), flags_var: opt_flags!(cb_flags), scope_idx: [27, -1, -1, -1], immutable: false, values: &OPT_CB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cmdheight", shortname: Some(c"ch"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_TAB)) as u8, var: opt_var!(p_ch), flags_var: None, scope_idx: [28, -1, -1, 0], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_cmdheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cmdwinheight", shortname: Some(c"cwh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cwh), flags_var: None, scope_idx: [29, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(7) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"colorcolumn", shortname: Some(c"cc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 3, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_colorcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_colorcolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"columns", shortname: Some(c"co"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_columns), flags_var: None, scope_idx: [30, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"comments", shortname: Some(c"com"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_com), flags_var: None, scope_idx: [-1, -1, 16, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_comments), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-,fb:•"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"commentstring", shortname: Some(c"cms"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cms), flags_var: None, scope_idx: [-1, -1, 17, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_commentstring), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"compatible", shortname: Some(c"cp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(50), flags_var: None, scope_idx: [31, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"complete", shortname: Some(c"cpt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cpt), flags_var: None, scope_idx: [-1, -1, 18, -1], immutable: false, values: &OPT_CPT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_complete), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_complete), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c".,w,b,u,t"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completefunc", shortname: Some(c"cfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cfu), flags_var: None, scope_idx: [-1, -1, 19, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completeitemalign", shortname: Some(c"cia"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cia), flags_var: None, scope_idx: [32, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_completeitemalign), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeitemalign), opt_expand_cb: None, def_val: cstr_def_val!(c"abbr,kind,menu"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completeopt", shortname: Some(c"cot"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_cot), flags_var: opt_flags!(cot_flags), scope_idx: [33, -1, 20, -1], immutable: false, values: &OPT_COT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu,popup"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_csl), flags_var: None, scope_idx: [-1, -1, 21, -1], immutable: false, values: &OPT_CSL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeslash), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_def_var!(55), flags_var: None, scope_idx: [-1, -1, 21, -1], immutable: true, values: &OPT_CSL_VALUES, schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completetimeout", shortname: Some(c"cto"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cto), flags_var: None, scope_idx: [34, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"concealcursor", shortname: Some(c"cocu"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 4, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_concealcursor), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_concealcursor), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"conceallevel", shortname: Some(c"cole"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 5, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"confirm", shortname: Some(c"cf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_confirm), flags_var: None, scope_idx: [35, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"copyindent", shortname: Some(c"ci"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ci), flags_var: None, scope_idx: [-1, -1, 22, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cpoptions", shortname: Some(c"cpo"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cpo), flags_var: None, scope_idx: [36, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cpoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_cpoptions), def_val: cstr_def_val!(CPO_VIM_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorbind", shortname: Some(c"crb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 6, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorcolumn", shortname: Some(c"cuc"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 7, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorline", shortname: Some(c"cul"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 8, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorlineopt", shortname: Some(c"culopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 9, -1, -1], immutable: false, values: &OPT_CULOPT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cursorlineopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cursorlineopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"debug", shortname: None, flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_debug), flags_var: None, scope_idx: [37, -1, -1, -1], immutable: false, values: &OPT_DEBUG_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"define", shortname: Some(c"def"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_def), flags_var: None, scope_idx: [38, -1, 23, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"delcombine", shortname: Some(c"deco"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_deco), flags_var: None, scope_idx: [39, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"dictionary", shortname: Some(c"dict"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_NDNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_dict), flags_var: None, scope_idx: [40, -1, 24, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diff", shortname: None, flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 10, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_diff), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffanchors", shortname: Some(c"dia"), flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_dia), flags_var: None, scope_idx: [41, -1, 25, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffanchors), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffexpr", shortname: Some(c"dex"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dex), flags_var: None, scope_idx: [42, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffopt", shortname: Some(c"dip"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dip), flags_var: None, scope_idx: [43, -1, -1, -1], immutable: false, values: &OPT_DIP_VALUES, schema: &OPT_DIP_SCHEMA, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_diffopt), def_val: cstr_def_val!(c"internal,filler,closeoff,indent-heuristic,inline:char,linematch:40"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"digraph", shortname: Some(c"dg"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dg), flags_var: None, scope_idx: [44, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"directory", shortname: Some(c"dir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dir), flags_var: None, scope_idx: [45, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"display", shortname: Some(c"dy"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dy), flags_var: opt_flags!(dy_flags), scope_idx: [46, -1, -1, -1], immutable: false, values: &OPT_DY_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_display), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"lastline"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eadirection", shortname: Some(c"ead"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ead), flags_var: None, scope_idx: [47, -1, -1, -1], immutable: false, values: &OPT_EAD_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"edcompatible", shortname: Some(c"ed"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(78), flags_var: None, scope_idx: [48, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"emoji", shortname: Some(c"emo"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_emoji), flags_var: None, scope_idx: [49, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_emoji), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"encoding", shortname: Some(c"enc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_enc), flags_var: None, scope_idx: [50, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: None, def_val: cstr_def_val!(c"utf-8"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"endoffile", shortname: Some(c"eof"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_eof), flags_var: None, scope_idx: [-1, -1, 26, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"endofline", shortname: Some(c"eol"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_eol), flags_var: None, scope_idx: [-1, -1, 27, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"equalalways", shortname: Some(c"ea"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ea), flags_var: None, scope_idx: [51, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_equalalways), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"equalprg", shortname: Some(c"ep"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ep), flags_var: None, scope_idx: [52, -1, 28, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorbells", shortname: Some(c"eb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_eb), flags_var: None, scope_idx: [53, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorfile", shortname: Some(c"ef"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ef), flags_var: None, scope_idx: [54, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"errors.err"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorformat", shortname: Some(c"efm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_efm), flags_var: None, scope_idx: [55, -1, 29, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_EFM_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eventignore", shortname: Some(c"ei"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ei), flags_var: None, scope_idx: [56, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eventignorewin", shortname: Some(c"eiw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 11, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"expandtab", shortname: Some(c"et"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_et), flags_var: None, scope_idx: [-1, -1, 30, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"exrc", shortname: Some(c"ex"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_exrc), flags_var: None, scope_idx: [57, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fileencoding", shortname: Some(c"fenc"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fenc), flags_var: None, scope_idx: [-1, -1, 31, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fileencodings", shortname: Some(c"fencs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fencs), flags_var: None, scope_idx: [58, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c"ucs-bom,utf-8,default,latin1"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"fileformat", shortname: Some(c"ff"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ff), flags_var: None, scope_idx: [-1, -1, 32, -1], immutable: false, values: &OPT_FF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"fileformat", shortname: Some(c"ff"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ff), flags_var: None, scope_idx: [-1, -1, 32, -1], immutable: false, values: &OPT_FF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"fileformats", shortname: Some(c"ffs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ffs), flags_var: None, scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos,unix"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"fileformats", shortname: Some(c"ffs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ffs), flags_var: None, scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix,dos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(any(windows, target_os = "macos"))]
    Vimoption { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fic), flags_var: None, scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(any(windows, target_os = "macos")))]
    Vimoption { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fic), flags_var: None, scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"filetype", shortname: Some(c"ft"), flags: K_OPT_FLAG_NO_GLOB|K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ft), flags_var: None, scope_idx: [-1, -1, 33, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fillchars", shortname: Some(c"fcs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_fcs), flags_var: None, scope_idx: [61, 12, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"findfunc", shortname: Some(c"ffu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ffu), flags_var: None, scope_idx: [62, -1, 34, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fixendofline", shortname: Some(c"fixeol"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fixeol), flags_var: None, scope_idx: [-1, -1, 35, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldclose", shortname: Some(c"fcl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fcl), flags_var: None, scope_idx: [63, -1, -1, -1], immutable: false, values: &OPT_FCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldcolumn", shortname: Some(c"fdc"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 13, -1, -1], immutable: false, values: &OPT_FDC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"0"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldenable", shortname: Some(c"fen"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 14, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldenable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldexpr", shortname: Some(c"fde"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 15, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldexpr), opt_expand_cb: None, def_val: cstr_def_val!(c"0"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldignore", shortname: Some(c"fdi"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 16, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldignore), opt_expand_cb: None, def_val: cstr_def_val!(c"#"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldlevel", shortname: Some(c"fdl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 17, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldlevel), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldlevelstart", shortname: Some(c"fdls"), flags: K_OPT_FLAG_CURSWANT, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fdls), flags_var: None, scope_idx: [64, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldmarker", shortname: Some(c"fmr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 18, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_foldmarker), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmarker), opt_expand_cb: None, def_val: cstr_def_val!(c"{{{,}}}"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldmethod", shortname: Some(c"fdm"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 19, -1, -1], immutable: false, values: &OPT_FDM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmethod), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"manual"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldminlines", shortname: Some(c"fml"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 20, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldminlines), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldnestmax", shortname: Some(c"fdn"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 21, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldnestmax), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldopen", shortname: Some(c"fdo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_CURSWANT|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fdo), flags_var: opt_flags!(fdo_flags), scope_idx: [65, -1, -1, -1], immutable: false, values: &OPT_FDO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"block,hor,mark,percent,quickfix,search,tag,undo"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldtext", shortname: Some(c"fdt"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 22, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"foldtext()"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"follow", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_follow), flags_var: None, scope_idx: [-1, -1, 36, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_follow), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatexpr", shortname: Some(c"fex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fex), flags_var: None, scope_idx: [-1, -1, 37, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatlistpat", shortname: Some(c"flp"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_flp), flags_var: None, scope_idx: [-1, -1, 38, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"^\\s*\\d\\+[\\]:.)}\\t ]\\s*"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatoptions", shortname: Some(c"fo"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fo), flags_var: None, scope_idx: [-1, -1, 39, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_formatoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_formatoptions), def_val: cstr_def_val!(c"tcqj"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatprg", shortname: Some(c"fp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_fp), flags_var: None, scope_idx: [66, -1, 40, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fsync", shortname: Some(c"fs"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_fs), flags_var: None, scope_idx: [67, -1, 41, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"gdefault", shortname: Some(c"gd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_gd), flags_var: None, scope_idx: [68, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"grepformat", shortname: Some(c"gfm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gefm), flags_var: None, scope_idx: [69, -1, 42, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"%f:%l:%m,%f:%l%m,%f  %l%m"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"grepprg", shortname: Some(c"gp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gp), flags_var: None, scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"findstr /n $* nul"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"grepprg", shortname: Some(c"gp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gp), flags_var: None, scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"grep -HIn $* /dev/null"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guicursor", shortname: Some(c"gcr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guicursor), flags_var: None, scope_idx: [71, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_guicursor), opt_expand_cb: None, def_val: cstr_def_val!(c"n-v-c-sm:block,i-ci-ve:ver25,r-cr-o:hor20,t:block-blinkon500-blinkoff500-TermCursor"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guifont", shortname: Some(c"gfn"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_UIOPTION|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guifont), flags_var: None, scope_idx: [72, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_GFN_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guifontwide", shortname: Some(c"gfw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_UIOPTION|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guifontwide), flags_var: None, scope_idx: [73, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guioptions", shortname: Some(c"go"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(126), flags_var: None, scope_idx: [74, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guitablabel", shortname: Some(c"gtl"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(127), flags_var: None, scope_idx: [75, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guitabtooltip", shortname: Some(c"gtt"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(128), flags_var: None, scope_idx: [76, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helpfile", shortname: Some(c"hf"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hf), flags_var: None, scope_idx: [77, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_helpfile), opt_expand_cb: None, def_val: cstr_def_val!(c"$VIMRUNTIME/doc/help.txt"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helpheight", shortname: Some(c"hh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hh), flags_var: None, scope_idx: [78, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_helpheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helplang", shortname: Some(c"hlg"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hlg), flags_var: None, scope_idx: [79, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_helplang), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hidden", shortname: Some(c"hid"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hid), flags_var: None, scope_idx: [80, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"highlight", shortname: Some(c"hl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hl), flags_var: None, scope_idx: [81, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_highlight), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(HIGHLIGHT_INIT_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"history", shortname: Some(c"hi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hi), flags_var: None, scope_idx: [82, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hkmap", shortname: Some(c"hk"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(135), flags_var: None, scope_idx: [83, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hkmapp", shortname: Some(c"hkp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(136), flags_var: None, scope_idx: [84, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hlsearch", shortname: Some(c"hls"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hls), flags_var: None, scope_idx: [85, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_hlsearch), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"icon", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_icon), flags_var: None, scope_idx: [86, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iconstring", shortname: None, flags: K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_iconstring), flags_var: None, scope_idx: [87, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iconstring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ignorecase", shortname: Some(c"ic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ic), flags_var: None, scope_idx: [88, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_ignorecase), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imcmdline", shortname: Some(c"imc"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(141), flags_var: None, scope_idx: [89, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imdisable", shortname: Some(c"imd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(142), flags_var: None, scope_idx: [90, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iminsert", shortname: Some(c"imi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_iminsert), flags_var: None, scope_idx: [-1, -1, 44, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_iminsert), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imsearch", shortname: Some(c"ims"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_imsearch), flags_var: None, scope_idx: [-1, -1, 45, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"inccommand", shortname: Some(c"icm"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_icm), flags_var: None, scope_idx: [91, -1, -1, -1], immutable: false, values: &OPT_ICM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_inccommand), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"nosplit"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"include", shortname: Some(c"inc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_inc), flags_var: None, scope_idx: [92, -1, 46, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"includeexpr", shortname: Some(c"inex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inex), flags_var: None, scope_idx: [-1, -1, 47, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"incsearch", shortname: Some(c"is"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_is), flags_var: None, scope_idx: [93, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"indentexpr", shortname: Some(c"inde"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inde), flags_var: None, scope_idx: [-1, -1, 48, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"indentkeys", shortname: Some(c"indk"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_indk), flags_var: None, scope_idx: [-1, -1, 49, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"infercase", shortname: Some(c"inf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inf), flags_var: None, scope_idx: [-1, -1, 50, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"insertmode", shortname: Some(c"im"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(152), flags_var: None, scope_idx: [94, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"isfname", shortname: Some(c"isf"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isf), flags_var: None, scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,\\,.,-,_,+,,,#,$,%,{,},[,],@-@,!,~,="), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"isfname", shortname: Some(c"isf"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isf), flags_var: None, scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,.,-,_,+,,,#,$,%,~,="), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"isident", shortname: Some(c"isi"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isi), flags_var: None, scope_idx: [96, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iskeyword", shortname: Some(c"isk"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_isk), flags_var: None, scope_idx: [-1, -1, 51, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iskeyword), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"isprint", shortname: Some(c"isp"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(156), flags_var: None, scope_idx: [97, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"@,161-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"joinspaces", shortname: Some(c"js"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_js), flags_var: None, scope_idx: [98, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"jumpoptions", shortname: Some(c"jop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_jop), flags_var: opt_flags!(jop_flags), scope_idx: [99, -1, -1, -1], immutable: false, values: &OPT_JOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"clean"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"keymap", shortname: Some(c"kmp"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NFNAME|K_OPT_FLAG_PRI_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_keymap), flags_var: None, scope_idx: [-1, -1, 52, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"keymodel", shortname: Some(c"km"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_km), flags_var: None, scope_idx: [100, -1, -1, -1], immutable: false, values: &OPT_KM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymodel), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"keywordprg", shortname: Some(c"kp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_kp), flags_var: None, scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":help!"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"keywordprg", shortname: Some(c"kp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_kp), flags_var: None, scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":Man"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langmap", shortname: Some(c"lmap"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_langmap), flags_var: None, scope_idx: [102, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::mapping::did_set_langmap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langmenu", shortname: Some(c"lm"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lm), flags_var: None, scope_idx: [103, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langnoremap", shortname: Some(c"lnr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lnr), flags_var: None, scope_idx: [104, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langnoremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langremap", shortname: Some(c"lrm"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lrm), flags_var: None, scope_idx: [105, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"laststatus", shortname: Some(c"ls"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ls), flags_var: None, scope_idx: [106, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_laststatus), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lazyredraw", shortname: Some(c"lz"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lz), flags_var: None, scope_idx: [107, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lhistory", shortname: Some(c"lhi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 23, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"linebreak", shortname: Some(c"lbr"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 24, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lines", shortname: None, flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lines), flags_var: None, scope_idx: [108, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(24) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"linespace", shortname: Some(c"lsp"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_linespace), flags_var: None, scope_idx: [109, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lisp", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_lisp), flags_var: None, scope_idx: [-1, -1, 54, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lisp), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lispoptions", shortname: Some(c"lop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_lop), flags_var: None, scope_idx: [-1, -1, 55, -1], immutable: false, values: &OPT_LOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_lispoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lispwords", shortname: Some(c"lw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_lispwords), flags_var: None, scope_idx: [110, -1, 56, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(LISPWORD_VALUE_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"list", shortname: None, flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 25, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"listchars", shortname: Some(c"lcs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_lcs), flags_var: None, scope_idx: [111, 26, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c"tab:> ,trail:-,nbsp:+"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"loadplugins", shortname: Some(c"lpl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lpl), flags_var: None, scope_idx: [112, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"magic", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_magic), flags_var: None, scope_idx: [113, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeef", shortname: Some(c"mef"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mef), flags_var: None, scope_idx: [114, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeencoding", shortname: Some(c"menc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_menc), flags_var: None, scope_idx: [115, -1, 57, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeprg", shortname: Some(c"mp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_mp), flags_var: None, scope_idx: [116, -1, 58, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"make"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"matchpairs", shortname: Some(c"mps"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_mps), flags_var: None, scope_idx: [-1, -1, 59, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_matchpairs), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"(:),{:},[:]"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"matchtime", shortname: Some(c"mat"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mat), flags_var: None, scope_idx: [117, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxcombine", shortname: Some(c"mco"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mco), flags_var: None, scope_idx: [118, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(6) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxfuncdepth", shortname: Some(c"mfd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mfd), flags_var: None, scope_idx: [119, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(100) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxmapdepth", shortname: Some(c"mmd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mmd), flags_var: None, scope_idx: [120, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxmempattern", shortname: Some(c"mmp"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mmp), flags_var: None, scope_idx: [121, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxsearchcount", shortname: Some(c"msc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_msc), flags_var: None, scope_idx: [122, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(999) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"menuitems", shortname: Some(c"mis"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mis), flags_var: None, scope_idx: [123, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(25) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"messagesopt", shortname: Some(c"mopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mopt), flags_var: None, scope_idx: [124, -1, -1, -1], immutable: false, values: &OPT_MOPT_VALUES, schema: &OPT_MOPT_SCHEMA, opt_validate_cb: opt_cb!(crate::message::validate_messagesopt), opt_did_set_cb: opt_cb!(crate::message::did_set_messagesopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"hit-enter,history:500,progress:c"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mkspellmem", shortname: Some(c"msm"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_msm), flags_var: None, scope_idx: [125, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mkspellmem), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"460000,2000,500"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modeline", shortname: Some(c"ml"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ml), flags_var: None, scope_idx: [-1, -1, 60, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modelineexpr", shortname: Some(c"mle"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mle), flags_var: None, scope_idx: [126, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modelines", shortname: Some(c"mls"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mls), flags_var: None, scope_idx: [127, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modifiable", shortname: Some(c"ma"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ma), flags_var: None, scope_idx: [-1, -1, 61, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modifiable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modified", shortname: Some(c"mod"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_mod), flags_var: None, scope_idx: [-1, -1, 62, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modified), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"more", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_more), flags_var: None, scope_idx: [128, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mouse", shortname: None, flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mouse), flags_var: None, scope_idx: [129, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mouse), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_mouse), def_val: cstr_def_val!(c"nvi"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousefocus", shortname: Some(c"mousef"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousef), flags_var: None, scope_idx: [130, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousehide", shortname: Some(c"mh"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mh), flags_var: None, scope_idx: [131, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousemodel", shortname: Some(c"mousem"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousem), flags_var: None, scope_idx: [132, -1, -1, -1], immutable: false, values: &OPT_MOUSEM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"popup_setpos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousemoveevent", shortname: Some(c"mousemev"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousemev), flags_var: None, scope_idx: [133, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousescroll", shortname: None, flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousescroll), flags_var: None, scope_idx: [134, -1, -1, -1], immutable: false, values: &OPT_MOUSESCROLL_VALUES, schema: &OPT_MOUSESCROLL_SCHEMA, opt_validate_cb: opt_cb!(crate::optionstr::validate_mousescroll), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_mousescroll), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver:3,hor:6"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mouseshape", shortname: Some(c"mouses"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(204), flags_var: None, scope_idx: [135, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousetime", shortname: Some(c"mouset"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mouset), flags_var: None, scope_idx: [136, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(500) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"nrformats", shortname: Some(c"nf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_nf), flags_var: None, scope_idx: [-1, -1, 63, -1], immutable: false, values: &OPT_NF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"bin,hex"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"number", shortname: Some(c"nu"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 27, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"numberwidth", shortname: Some(c"nuw"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 28, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_numberwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(4) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"omnifunc", shortname: Some(c"ofu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ofu), flags_var: None, scope_idx: [-1, -1, 64, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"opendevice", shortname: Some(c"odev"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(210), flags_var: None, scope_idx: [137, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"operatorfunc", shortname: Some(c"opfunc"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_opfunc), flags_var: None, scope_idx: [138, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"packlockfile", shortname: Some(c"plf"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_plf), flags_var: None, scope_idx: [139, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"$XDG_CONFIG_HOME/nvim/nvim-pack-lock.json"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"packpath", shortname: Some(c"pp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pp), flags_var: None, scope_idx: [140, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"paragraphs", shortname: Some(c"para"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_para), flags_var: None, scope_idx: [141, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"IPLPPPQPP TPHPLIPpLpItpplpipbp"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"paste", shortname: None, flags: K_OPT_FLAG_PRI_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_paste), flags_var: None, scope_idx: [142, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_paste), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pastetoggle", shortname: Some(c"pt"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(216), flags_var: None, scope_idx: [143, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"patchexpr", shortname: Some(c"pex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pex), flags_var: None, scope_idx: [144, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"patchmode", shortname: Some(c"pm"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pm), flags_var: None, scope_idx: [145, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"path", shortname: Some(c"pa"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_path), flags_var: None, scope_idx: [146, -1, 65, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".,,"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"preserveindent", shortname: Some(c"pi"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_pi), flags_var: None, scope_idx: [-1, -1, 66, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewheight", shortname: Some(c"pvh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pvh), flags_var: None, scope_idx: [147, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(12) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewpopup", shortname: Some(c"pvp"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_COLON, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pvp), flags_var: None, scope_idx: [148, -1, -1, -1], immutable: false, values: &OPT_PVP_VALUES, schema: &OPT_PVP_SCHEMA, opt_validate_cb: opt_cb!(crate::optionstr::validate_previewpopup), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_popupoption), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewwindow", shortname: Some(c"pvw"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 29, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_previewwindow), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"prompt", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(224), flags_var: None, scope_idx: [149, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumblend", shortname: Some(c"pb"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pb), flags_var: None, scope_idx: [150, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_pumblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumborder", shortname: None, flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pumborder), flags_var: None, scope_idx: [151, -1, -1, -1], immutable: false, values: &OPT_PUMBORDER_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumheight", shortname: Some(c"ph"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ph), flags_var: None, scope_idx: [152, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pummaxwidth", shortname: Some(c"pmw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pmw), flags_var: None, scope_idx: [153, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumwidth", shortname: Some(c"pw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pw), flags_var: None, scope_idx: [154, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(15) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pyxversion", shortname: Some(c"pyx"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pyx), flags_var: None, scope_idx: [155, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(3) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"quickfixtextfunc", shortname: Some(c"qftf"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_qftf), flags_var: None, scope_idx: [156, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"quoteescape", shortname: Some(c"qe"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_qe), flags_var: None, scope_idx: [-1, -1, 67, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"\\"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"readonly", shortname: Some(c"ro"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ro), flags_var: None, scope_idx: [-1, -1, 68, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_readonly), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"redrawdebug", shortname: Some(c"rdb"), flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rdb), flags_var: opt_flags!(rdb_flags), scope_idx: [157, -1, -1, -1], immutable: false, values: &OPT_RDB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"redrawtime", shortname: Some(c"rdt"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rdt), flags_var: None, scope_idx: [158, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"regexpengine", shortname: Some(c"re"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_re), flags_var: None, scope_idx: [159, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"relativenumber", shortname: Some(c"rnu"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 30, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"remap", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(238), flags_var: None, scope_idx: [160, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"report", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_report), flags_var: None, scope_idx: [161, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"revins", shortname: Some(c"ri"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ri), flags_var: None, scope_idx: [162, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rightleft", shortname: Some(c"rl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 31, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rightleftcmd", shortname: Some(c"rlc"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 32, -1, -1], immutable: false, values: &OPT_RLC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"search"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ruler", shortname: Some(c"ru"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ru), flags_var: None, scope_idx: [163, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rulerformat", shortname: Some(c"ruf"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ruf), flags_var: None, scope_idx: [164, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_rulerformat), opt_expand_cb: None, def_val: cstr_def_val!(c"%18(%l,%c%V%= %P%)%<"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"runtimepath", shortname: Some(c"rtp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rtp), flags_var: None, scope_idx: [165, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scroll", shortname: Some(c"scr"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 33, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollback", shortname: Some(c"scbk"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_scbk), flags_var: None, scope_idx: [-1, -1, 69, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollback), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollbind", shortname: Some(c"scb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 34, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollbind), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolljump", shortname: Some(c"sj"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sj), flags_var: None, scope_idx: [166, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolloff", shortname: Some(c"so"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_so), flags_var: None, scope_idx: [167, 35, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolloffpad", shortname: Some(c"sop"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_sop), flags_var: None, scope_idx: [168, 36, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollopt", shortname: Some(c"sbo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sbo), flags_var: None, scope_idx: [169, -1, -1, -1], immutable: false, values: &OPT_SBO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver,jump"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sections", shortname: Some(c"sect"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sections), flags_var: None, scope_idx: [170, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"SHNHH HUnhsh"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"secure", shortname: None, flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_secure), flags_var: None, scope_idx: [171, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"selection", shortname: Some(c"sel"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sel), flags_var: None, scope_idx: [172, -1, -1, -1], immutable: false, values: &OPT_SEL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_selection), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"inclusive"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"selectmode", shortname: Some(c"slm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_slm), flags_var: None, scope_idx: [173, -1, -1, -1], immutable: false, values: &OPT_SLM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sessionoptions", shortname: Some(c"ssop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ssop), flags_var: opt_flags!(ssop_flags), scope_idx: [174, -1, -1, -1], immutable: false, values: &OPT_SSOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_sessionoptions), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"blank,buffers,curdir,folds,help,tabpages,winsize,terminal"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shada", shortname: Some(c"sd"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shada), flags_var: None, scope_idx: [175, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shada), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"!,'100,<50,s10,h,r/tmp/,r/private/"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shadafile", shortname: Some(c"sdf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shadafile), flags_var: None, scope_idx: [176, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shell", shortname: Some(c"sh"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sh), flags_var: None, scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"cmd.exe"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shell", shortname: Some(c"sh"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sh), flags_var: None, scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"sh"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellcmdflag", shortname: Some(c"shcf"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shcf), flags_var: None, scope_idx: [178, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"-c"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellpipe", shortname: Some(c"sp"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sp), flags_var: None, scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"2>&1| tee"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellpipe", shortname: Some(c"sp"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sp), flags_var: None, scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"| tee"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellquote", shortname: Some(c"shq"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shq), flags_var: None, scope_idx: [180, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellredir", shortname: Some(c"srr"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_srr), flags_var: None, scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">%s 2>&1"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellredir", shortname: Some(c"srr"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_srr), flags_var: None, scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ssl), flags_var: None, scope_idx: [182, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(265), flags_var: None, scope_idx: [182, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shelltemp", shortname: Some(c"stmp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_stmp), flags_var: None, scope_idx: [183, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellxescape", shortname: Some(c"sxe"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sxe), flags_var: None, scope_idx: [184, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellxquote", shortname: Some(c"sxq"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sxq), flags_var: None, scope_idx: [185, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shiftround", shortname: Some(c"sr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sr), flags_var: None, scope_idx: [186, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shiftwidth", shortname: Some(c"sw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sw), flags_var: None, scope_idx: [-1, -1, 70, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shortmess", shortname: Some(c"shm"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shm), flags_var: None, scope_idx: [187, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shortmess), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_shortmess), def_val: cstr_def_val!(c"ltToOCF"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showbreak", shortname: Some(c"sbr"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_sbr), flags_var: None, scope_idx: [188, 37, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_showbreak), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showcmd", shortname: Some(c"sc"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sc), flags_var: None, scope_idx: [189, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showcmdloc", shortname: Some(c"sloc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sloc), flags_var: None, scope_idx: [190, -1, -1, -1], immutable: false, values: &OPT_SLOC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_showcmdloc), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"last"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showfulltag", shortname: Some(c"sft"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sft), flags_var: None, scope_idx: [191, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showmatch", shortname: Some(c"sm"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sm), flags_var: None, scope_idx: [192, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showmode", shortname: Some(c"smd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_smd), flags_var: None, scope_idx: [193, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showtabline", shortname: Some(c"stal"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_stal), flags_var: None, scope_idx: [194, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_showtabline), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sidescroll", shortname: Some(c"ss"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ss), flags_var: None, scope_idx: [195, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sidescrolloff", shortname: Some(c"siso"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_siso), flags_var: None, scope_idx: [196, 38, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"signcolumn", shortname: Some(c"scl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 39, -1, -1], immutable: false, values: &OPT_SCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_signcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_signcolumn), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smartcase", shortname: Some(c"scs"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_scs), flags_var: None, scope_idx: [197, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smartindent", shortname: Some(c"si"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_si), flags_var: None, scope_idx: [-1, -1, 71, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smarttab", shortname: Some(c"sta"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sta), flags_var: None, scope_idx: [198, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smoothscroll", shortname: Some(c"sms"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 40, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_smoothscroll), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"softtabstop", shortname: Some(c"sts"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sts), flags_var: None, scope_idx: [-1, -1, 72, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spell", shortname: None, flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 41, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_spell), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellcapcheck", shortname: Some(c"spc"), flags: K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spc), flags_var: None, scope_idx: [-1, -1, 73, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellcapcheck), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellcapcheck), opt_expand_cb: None, def_val: cstr_def_val!(c"[.?!]\\_[\\])'\"\\t ]\\+"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellfile", shortname: Some(c"spf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spf), flags_var: None, scope_idx: [-1, -1, 74, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellfile), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellfile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spelllang", shortname: Some(c"spl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spl), flags_var: None, scope_idx: [-1, -1, 75, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spelllang), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelllang), opt_expand_cb: None, def_val: cstr_def_val!(c"en"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spelloptions", shortname: Some(c"spo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spo), flags_var: None, scope_idx: [-1, -1, 76, -1], immutable: false, values: &OPT_SPO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelloptions), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellsuggest", shortname: Some(c"sps"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sps), flags_var: None, scope_idx: [199, -1, -1, -1], immutable: false, values: &OPT_SPS_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellsuggest), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellsuggest), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"best"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitbelow", shortname: Some(c"sb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sb), flags_var: None, scope_idx: [200, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitkeep", shortname: Some(c"spk"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_spk), flags_var: None, scope_idx: [201, -1, -1, -1], immutable: false, values: &OPT_SPK_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_splitkeep), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"cursor"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitright", shortname: Some(c"spr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_spr), flags_var: None, scope_idx: [202, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"startofline", shortname: Some(c"sol"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sol), flags_var: None, scope_idx: [203, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"statuscolumn", shortname: Some(c"stc"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 42, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statuscolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"statusline", shortname: Some(c"stl"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_stl), flags_var: None, scope_idx: [204, 43, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statusline), opt_expand_cb: None, def_val: cstr_def_val!(c"%(%#(%#StatusLineRemote# @%.30(%{ v:lua.require('vim._core.util').remote_host() }%<%) %#) %)%<%f%( %h%w%m%r%{ v:lua.require('vim._core.util').term_exitcode() }%)%= %(%-10S %)%{ &busy > 0 ? '◐ ' : '' }%(%{ luaeval('(package.loaded[''vim.ui''] and vim.api.nvim_get_current_win() == tonumber(vim.g.actual_curwin or -1) and vim.ui.progress_status()) or '''' ')} %)%{% luaeval('(package.loaded[''vim.diagnostic''] and next(vim.diagnostic.count(0)) and vim.diagnostic.status() .. '' '') or '''' ') %}%(%k %)%{% &ruler ? &rulerformat : '' %}"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"suffixes", shortname: Some(c"su"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_su), flags_var: None, scope_idx: [205, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".bak,~,.o,.h,.info,.swp,.obj"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"suffixesadd", shortname: Some(c"sua"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sua), flags_var: None, scope_idx: [-1, -1, 77, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"swapfile", shortname: Some(c"swf"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_swf), flags_var: None, scope_idx: [-1, -1, 78, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_swapfile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"switchbuf", shortname: Some(c"swb"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_swb), flags_var: opt_flags!(swb_flags), scope_idx: [206, -1, -1, -1], immutable: false, values: &OPT_SWB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"uselast"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"synmaxcol", shortname: Some(c"smc"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_smc), flags_var: None, scope_idx: [-1, -1, 79, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(3000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"syntax", shortname: Some(c"syn"), flags: K_OPT_FLAG_NO_GLOB|K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_syn), flags_var: None, scope_idx: [-1, -1, 80, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabclose", shortname: Some(c"tcl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tcl), flags_var: opt_flags!(tcl_flags), scope_idx: [207, -1, -1, -1], immutable: false, values: &OPT_TCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabline", shortname: Some(c"tal"), flags: K_OPT_FLAG_REDR_TABL|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tal), flags_var: None, scope_idx: [208, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tabline), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabpagemax", shortname: Some(c"tpm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tpm), flags_var: None, scope_idx: [209, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabstop", shortname: Some(c"ts"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ts), flags_var: None, scope_idx: [-1, -1, 81, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagbsearch", shortname: Some(c"tbs"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tbs), flags_var: None, scope_idx: [210, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagcase", shortname: Some(c"tc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tc), flags_var: opt_flags!(tc_flags), scope_idx: [211, -1, 82, -1], immutable: false, values: &OPT_TC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tagcase), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"followic"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagfunc", shortname: Some(c"tfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_tfu), flags_var: None, scope_idx: [-1, -1, 83, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"taglength", shortname: Some(c"tl"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tl), flags_var: None, scope_idx: [212, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagrelative", shortname: Some(c"tr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tr), flags_var: None, scope_idx: [213, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tags", shortname: Some(c"tag"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tags), flags_var: None, scope_idx: [214, -1, 84, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"./tags;,tags"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagstack", shortname: Some(c"tgst"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tgst), flags_var: None, scope_idx: [215, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termbidi", shortname: Some(c"tbidi"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tbidi), flags_var: None, scope_idx: [216, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termencoding", shortname: Some(c"tenc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(317), flags_var: None, scope_idx: [217, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termguicolors", shortname: Some(c"tgc"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tgc), flags_var: None, scope_idx: [218, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termpastefilter", shortname: Some(c"tpf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tpf), flags_var: opt_flags!(tpf_flags), scope_idx: [219, -1, -1, -1], immutable: false, values: &OPT_TPF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"BS,ESC,DEL"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termsync", shortname: None, flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_termsync), flags_var: None, scope_idx: [220, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"terse", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(321), flags_var: None, scope_idx: [221, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"textwidth", shortname: Some(c"tw"), flags: K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_tw), flags_var: None, scope_idx: [-1, -1, 85, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_textwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"thesaurus", shortname: Some(c"tsr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_NDNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tsr), flags_var: None, scope_idx: [222, -1, 86, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"thesaurusfunc", shortname: Some(c"tsrfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tsrfu), flags_var: None, scope_idx: [223, -1, 87, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tildeop", shortname: Some(c"top"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_to), flags_var: None, scope_idx: [224, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"timeout", shortname: Some(c"to"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_timeout), flags_var: None, scope_idx: [225, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"timeoutlen", shortname: Some(c"tm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tm), flags_var: None, scope_idx: [226, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"title", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_title), flags_var: None, scope_idx: [227, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titlelen", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titlelen), flags_var: None, scope_idx: [228, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_titlelen), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(85) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titleold", shortname: None, flags: K_OPT_FLAG_NO_MKRC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titleold), flags_var: None, scope_idx: [229, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titlestring", shortname: None, flags: K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titlestring), flags_var: None, scope_idx: [230, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_titlestring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttimeout", shortname: None, flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ttimeout), flags_var: None, scope_idx: [231, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttimeoutlen", shortname: Some(c"ttm"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ttm), flags_var: None, scope_idx: [232, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttyfast", shortname: Some(c"tf"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tf), flags_var: None, scope_idx: [233, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undodir", shortname: Some(c"udir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_udir), flags_var: None, scope_idx: [234, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undofile", shortname: Some(c"udf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_udf), flags_var: None, scope_idx: [-1, -1, 88, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undofile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undolevels", shortname: Some(c"ul"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ul), flags_var: None, scope_idx: [235, -1, 89, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undolevels), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undoreload", shortname: Some(c"ur"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ur), flags_var: None, scope_idx: [236, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"updatecount", shortname: Some(c"uc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_uc), flags_var: None, scope_idx: [237, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_updatecount), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(200) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"updatetime", shortname: Some(c"ut"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ut), flags_var: None, scope_idx: [238, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(4000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"varsofttabstop", shortname: Some(c"vsts"), flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_vsts), flags_var: None, scope_idx: [-1, -1, 90, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"vartabstop", shortname: Some(c"vts"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_vts), flags_var: None, scope_idx: [-1, -1, 91, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"verbose", shortname: Some(c"vbs"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_verbose), flags_var: None, scope_idx: [239, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"verbosefile", shortname: Some(c"vfile"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vfile), flags_var: None, scope_idx: [240, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_verbosefile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"viewdir", shortname: Some(c"vdir"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vdir), flags_var: None, scope_idx: [241, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"viewoptions", shortname: Some(c"vop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vop), flags_var: opt_flags!(vop_flags), scope_idx: [242, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"folds,cursor,curdir"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"virtualedit", shortname: Some(c"ve"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_CURSWANT|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_ve), flags_var: opt_flags!(ve_flags), scope_idx: [243, 44, -1, -1], immutable: false, values: &OPT_VE_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_virtualedit), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"visualbell", shortname: Some(c"vb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vb), flags_var: None, scope_idx: [244, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"warn", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_warn), flags_var: None, scope_idx: [245, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"whichwrap", shortname: Some(c"ww"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ww), flags_var: None, scope_idx: [246, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_whichwrap), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_whichwrap), def_val: cstr_def_val!(c"b,s"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildchar", shortname: Some(c"wc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wc), flags_var: None, scope_idx: [247, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(9) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildcharm", shortname: Some(c"wcm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wcm), flags_var: None, scope_idx: [248, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildignore", shortname: Some(c"wig"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wig), flags_var: None, scope_idx: [249, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildignorecase", shortname: Some(c"wic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wic), flags_var: None, scope_idx: [250, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildmenu", shortname: Some(c"wmnu"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmnu), flags_var: None, scope_idx: [251, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildmode", shortname: Some(c"wim"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wim), flags_var: None, scope_idx: [252, -1, -1, -1], immutable: false, values: &OPT_WIM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_wildmode), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_wildmode), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"full"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildoptions", shortname: Some(c"wop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wop), flags_var: opt_flags!(wop_flags), scope_idx: [253, -1, -1, -1], immutable: false, values: &OPT_WOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"pum,tagfile"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winaltkeys", shortname: Some(c"wak"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wak), flags_var: None, scope_idx: [254, -1, -1, -1], immutable: false, values: &OPT_WAK_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winbar", shortname: Some(c"wbr"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_wbr), flags_var: None, scope_idx: [255, 45, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winbar), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winblend", shortname: Some(c"winbl"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 46, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winborder", shortname: None, flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_winborder), flags_var: None, scope_idx: [256, -1, -1, -1], immutable: false, values: &OPT_WINBORDER_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"window", shortname: Some(c"wi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_window), flags_var: None, scope_idx: [257, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_window), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixbuf", shortname: Some(c"wfb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 47, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixheight", shortname: Some(c"wfh"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 48, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixwidth", shortname: Some(c"wfw"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 49, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winheight", shortname: Some(c"wh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wh), flags_var: None, scope_idx: [258, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winhighlight", shortname: Some(c"winhl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 50, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winhighlight), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_winhighlight), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winminheight", shortname: Some(c"wmh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmh), flags_var: None, scope_idx: [259, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winminwidth", shortname: Some(c"wmw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmw), flags_var: None, scope_idx: [260, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winpinned", shortname: Some(c"wp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 51, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winwidth", shortname: Some(c"wiw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wiw), flags_var: None, scope_idx: [261, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrap", shortname: None, flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: None, flags_var: None, scope_idx: [-1, 52, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_wrap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrapmargin", shortname: Some(c"wm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_wm), flags_var: None, scope_idx: [-1, -1, 92, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrapscan", shortname: Some(c"ws"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ws), flags_var: None, scope_idx: [262, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"write", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_write), flags_var: None, scope_idx: [263, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writeany", shortname: Some(c"wa"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wa), flags_var: None, scope_idx: [264, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writebackup", shortname: Some(c"wb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wb), flags_var: None, scope_idx: [265, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writedelay", shortname: Some(c"wd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wd), flags_var: None, scope_idx: [266, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    ]
}

thread_local! {
    static OPTIONS: std::cell::RefCell<[Vimoption; 379]> = const {
        std::cell::RefCell::new(init_options())
    };
}

/// Return information for option at `opt_idx` (`get_option`).
pub fn get_option(opt_idx: OptIndex) -> Vimoption {
    assert!(opt_idx != K_OPT_INVALID);
    OPTIONS.with(|opts| opts.borrow()[opt_idx as usize].clone())
}

const OPTION_NAME_MAP: &[(&[u8], OptIndex)] = &[
    (b"ac", 6),
    (b"acd", 5),
    (b"acl", 7),
    (b"act", 8),
    (b"ai", 9),
    (b"al", 0),
    (b"aleph", 0),
    (b"allowrevins", 1),
    (b"ambiwidth", 2),
    (b"ambw", 2),
    (b"ar", 10),
    (b"arab", 3),
    (b"arabic", 3),
    (b"arabicshape", 4),
    (b"ari", 1),
    (b"arshape", 4),
    (b"autochdir", 5),
    (b"autocomplete", 6),
    (b"autocompletedelay", 7),
    (b"autocompletetimeout", 8),
    (b"autoindent", 9),
    (b"autoread", 10),
    (b"autowrite", 11),
    (b"autowriteall", 12),
    (b"aw", 11),
    (b"awa", 12),
    (b"background", 13),
    (b"backspace", 14),
    (b"backup", 15),
    (b"backupcopy", 16),
    (b"backupdir", 17),
    (b"backupext", 18),
    (b"backupskip", 19),
    (b"bdir", 17),
    (b"belloff", 20),
    (b"bex", 18),
    (b"bg", 13),
    (b"bh", 27),
    (b"bin", 21),
    (b"binary", 21),
    (b"bk", 15),
    (b"bkc", 16),
    (b"bl", 28),
    (b"bo", 20),
    (b"bomb", 22),
    (b"breakat", 23),
    (b"breakindent", 24),
    (b"breakindentopt", 25),
    (b"bri", 24),
    (b"briopt", 25),
    (b"brk", 23),
    (b"browsedir", 26),
    (b"bs", 14),
    (b"bsdir", 26),
    (b"bsk", 19),
    (b"bt", 29),
    (b"bufhidden", 27),
    (b"buflisted", 28),
    (b"buftype", 29),
    (b"busy", 30),
    (b"casemap", 31),
    (b"cb", 43),
    (b"cc", 46),
    (b"ccv", 36),
    (b"cd", 33),
    (b"cdh", 32),
    (b"cdhome", 32),
    (b"cdpath", 33),
    (b"cedit", 34),
    (b"cf", 59),
    (b"cfu", 52),
    (b"ch", 44),
    (b"channel", 35),
    (b"charconvert", 36),
    (b"chi", 37),
    (b"chistory", 37),
    (b"ci", 60),
    (b"cia", 53),
    (b"cin", 38),
    (b"cindent", 38),
    (b"cink", 39),
    (b"cinkeys", 39),
    (b"cino", 40),
    (b"cinoptions", 40),
    (b"cinscopedecls", 41),
    (b"cinsd", 41),
    (b"cinw", 42),
    (b"cinwords", 42),
    (b"clipboard", 43),
    (b"cmdheight", 44),
    (b"cmdwinheight", 45),
    (b"cmp", 31),
    (b"cms", 49),
    (b"co", 47),
    (b"cocu", 57),
    (b"cole", 58),
    (b"colorcolumn", 46),
    (b"columns", 47),
    (b"com", 48),
    (b"comments", 48),
    (b"commentstring", 49),
    (b"compatible", 50),
    (b"complete", 51),
    (b"completefunc", 52),
    (b"completeitemalign", 53),
    (b"completeopt", 54),
    (b"completeslash", 55),
    (b"completetimeout", 56),
    (b"concealcursor", 57),
    (b"conceallevel", 58),
    (b"confirm", 59),
    (b"copyindent", 60),
    (b"cot", 54),
    (b"cp", 50),
    (b"cpo", 61),
    (b"cpoptions", 61),
    (b"cpt", 51),
    (b"crb", 62),
    (b"csl", 55),
    (b"cto", 56),
    (b"cuc", 63),
    (b"cul", 64),
    (b"culopt", 65),
    (b"cursorbind", 62),
    (b"cursorcolumn", 63),
    (b"cursorline", 64),
    (b"cursorlineopt", 65),
    (b"cwh", 45),
    (b"debug", 66),
    (b"deco", 68),
    (b"def", 67),
    (b"define", 67),
    (b"delcombine", 68),
    (b"dex", 72),
    (b"dg", 74),
    (b"dia", 71),
    (b"dict", 69),
    (b"dictionary", 69),
    (b"diff", 70),
    (b"diffanchors", 71),
    (b"diffexpr", 72),
    (b"diffopt", 73),
    (b"digraph", 74),
    (b"dip", 73),
    (b"dir", 75),
    (b"directory", 75),
    (b"display", 76),
    (b"dy", 76),
    (b"ea", 83),
    (b"ead", 77),
    (b"eadirection", 77),
    (b"eb", 85),
    (b"ed", 78),
    (b"edcompatible", 78),
    (b"ef", 86),
    (b"efm", 87),
    (b"ei", 88),
    (b"eiw", 89),
    (b"emo", 79),
    (b"emoji", 79),
    (b"enc", 80),
    (b"encoding", 80),
    (b"endoffile", 81),
    (b"endofline", 82),
    (b"eof", 81),
    (b"eol", 82),
    (b"ep", 84),
    (b"equalalways", 83),
    (b"equalprg", 84),
    (b"errorbells", 85),
    (b"errorfile", 86),
    (b"errorformat", 87),
    (b"et", 90),
    (b"eventignore", 88),
    (b"eventignorewin", 89),
    (b"ex", 91),
    (b"expandtab", 90),
    (b"exrc", 91),
    (b"fcl", 101),
    (b"fcs", 98),
    (b"fdc", 102),
    (b"fde", 104),
    (b"fdi", 105),
    (b"fdl", 106),
    (b"fdls", 107),
    (b"fdm", 109),
    (b"fdn", 111),
    (b"fdo", 112),
    (b"fdt", 113),
    (b"fen", 103),
    (b"fenc", 92),
    (b"fencs", 93),
    (b"fex", 115),
    (b"ff", 94),
    (b"ffs", 95),
    (b"ffu", 99),
    (b"fic", 96),
    (b"fileencoding", 92),
    (b"fileencodings", 93),
    (b"fileformat", 94),
    (b"fileformats", 95),
    (b"fileignorecase", 96),
    (b"filetype", 97),
    (b"fillchars", 98),
    (b"findfunc", 99),
    (b"fixendofline", 100),
    (b"fixeol", 100),
    (b"flp", 116),
    (b"fml", 110),
    (b"fmr", 108),
    (b"fo", 117),
    (b"foldclose", 101),
    (b"foldcolumn", 102),
    (b"foldenable", 103),
    (b"foldexpr", 104),
    (b"foldignore", 105),
    (b"foldlevel", 106),
    (b"foldlevelstart", 107),
    (b"foldmarker", 108),
    (b"foldmethod", 109),
    (b"foldminlines", 110),
    (b"foldnestmax", 111),
    (b"foldopen", 112),
    (b"foldtext", 113),
    (b"follow", 114),
    (b"formatexpr", 115),
    (b"formatlistpat", 116),
    (b"formatoptions", 117),
    (b"formatprg", 118),
    (b"fp", 118),
    (b"fs", 119),
    (b"fsync", 119),
    (b"ft", 97),
    (b"gcr", 123),
    (b"gd", 120),
    (b"gdefault", 120),
    (b"gfm", 121),
    (b"gfn", 124),
    (b"gfw", 125),
    (b"go", 126),
    (b"gp", 122),
    (b"grepformat", 121),
    (b"grepprg", 122),
    (b"gtl", 127),
    (b"gtt", 128),
    (b"guicursor", 123),
    (b"guifont", 124),
    (b"guifontwide", 125),
    (b"guioptions", 126),
    (b"guitablabel", 127),
    (b"guitabtooltip", 128),
    (b"helpfile", 129),
    (b"helpheight", 130),
    (b"helplang", 131),
    (b"hf", 129),
    (b"hh", 130),
    (b"hi", 134),
    (b"hid", 132),
    (b"hidden", 132),
    (b"highlight", 133),
    (b"history", 134),
    (b"hk", 135),
    (b"hkmap", 135),
    (b"hkmapp", 136),
    (b"hkp", 136),
    (b"hl", 133),
    (b"hlg", 131),
    (b"hls", 137),
    (b"hlsearch", 137),
    (b"ic", 140),
    (b"icm", 145),
    (b"icon", 138),
    (b"iconstring", 139),
    (b"ignorecase", 140),
    (b"im", 152),
    (b"imc", 141),
    (b"imcmdline", 141),
    (b"imd", 142),
    (b"imdisable", 142),
    (b"imi", 143),
    (b"iminsert", 143),
    (b"ims", 144),
    (b"imsearch", 144),
    (b"inc", 146),
    (b"inccommand", 145),
    (b"include", 146),
    (b"includeexpr", 147),
    (b"incsearch", 148),
    (b"inde", 149),
    (b"indentexpr", 149),
    (b"indentkeys", 150),
    (b"indk", 150),
    (b"inex", 147),
    (b"inf", 151),
    (b"infercase", 151),
    (b"insertmode", 152),
    (b"is", 148),
    (b"isf", 153),
    (b"isfname", 153),
    (b"isi", 154),
    (b"isident", 154),
    (b"isk", 155),
    (b"iskeyword", 155),
    (b"isp", 156),
    (b"isprint", 156),
    (b"joinspaces", 157),
    (b"jop", 158),
    (b"js", 157),
    (b"jumpoptions", 158),
    (b"keymap", 159),
    (b"keymodel", 160),
    (b"keywordprg", 161),
    (b"km", 160),
    (b"kmp", 159),
    (b"kp", 161),
    (b"langmap", 162),
    (b"langmenu", 163),
    (b"langnoremap", 164),
    (b"langremap", 165),
    (b"laststatus", 166),
    (b"lazyredraw", 167),
    (b"lbr", 169),
    (b"lcs", 176),
    (b"lhi", 168),
    (b"lhistory", 168),
    (b"linebreak", 169),
    (b"lines", 170),
    (b"linespace", 171),
    (b"lisp", 172),
    (b"lispoptions", 173),
    (b"lispwords", 174),
    (b"list", 175),
    (b"listchars", 176),
    (b"lm", 163),
    (b"lmap", 162),
    (b"lnr", 164),
    (b"loadplugins", 177),
    (b"lop", 173),
    (b"lpl", 177),
    (b"lrm", 165),
    (b"ls", 166),
    (b"lsp", 171),
    (b"lw", 174),
    (b"lz", 167),
    (b"ma", 195),
    (b"magic", 178),
    (b"makeef", 179),
    (b"makeencoding", 180),
    (b"makeprg", 181),
    (b"mat", 183),
    (b"matchpairs", 182),
    (b"matchtime", 183),
    (b"maxcombine", 184),
    (b"maxfuncdepth", 185),
    (b"maxmapdepth", 186),
    (b"maxmempattern", 187),
    (b"maxsearchcount", 188),
    (b"mco", 184),
    (b"mef", 179),
    (b"menc", 180),
    (b"menuitems", 189),
    (b"messagesopt", 190),
    (b"mfd", 185),
    (b"mh", 200),
    (b"mis", 189),
    (b"mkspellmem", 191),
    (b"ml", 192),
    (b"mle", 193),
    (b"mls", 194),
    (b"mmd", 186),
    (b"mmp", 187),
    (b"mod", 196),
    (b"modeline", 192),
    (b"modelineexpr", 193),
    (b"modelines", 194),
    (b"modifiable", 195),
    (b"modified", 196),
    (b"mopt", 190),
    (b"more", 197),
    (b"mouse", 198),
    (b"mousef", 199),
    (b"mousefocus", 199),
    (b"mousehide", 200),
    (b"mousem", 201),
    (b"mousemev", 202),
    (b"mousemodel", 201),
    (b"mousemoveevent", 202),
    (b"mouses", 204),
    (b"mousescroll", 203),
    (b"mouseshape", 204),
    (b"mouset", 205),
    (b"mousetime", 205),
    (b"mp", 181),
    (b"mps", 182),
    (b"msc", 188),
    (b"msm", 191),
    (b"nf", 206),
    (b"nrformats", 206),
    (b"nu", 207),
    (b"number", 207),
    (b"numberwidth", 208),
    (b"nuw", 208),
    (b"odev", 210),
    (b"ofu", 209),
    (b"omnifunc", 209),
    (b"opendevice", 210),
    (b"operatorfunc", 211),
    (b"opfunc", 211),
    (b"pa", 219),
    (b"packlockfile", 212),
    (b"packpath", 213),
    (b"para", 214),
    (b"paragraphs", 214),
    (b"paste", 215),
    (b"pastetoggle", 216),
    (b"patchexpr", 217),
    (b"patchmode", 218),
    (b"path", 219),
    (b"pb", 225),
    (b"pex", 217),
    (b"ph", 227),
    (b"pi", 220),
    (b"plf", 212),
    (b"pm", 218),
    (b"pmw", 228),
    (b"pp", 213),
    (b"preserveindent", 220),
    (b"previewheight", 221),
    (b"previewpopup", 222),
    (b"previewwindow", 223),
    (b"prompt", 224),
    (b"pt", 216),
    (b"pumblend", 225),
    (b"pumborder", 226),
    (b"pumheight", 227),
    (b"pummaxwidth", 228),
    (b"pumwidth", 229),
    (b"pvh", 221),
    (b"pvp", 222),
    (b"pvw", 223),
    (b"pw", 229),
    (b"pyx", 230),
    (b"pyxversion", 230),
    (b"qe", 232),
    (b"qftf", 231),
    (b"quickfixtextfunc", 231),
    (b"quoteescape", 232),
    (b"rdb", 234),
    (b"rdt", 235),
    (b"re", 236),
    (b"readonly", 233),
    (b"redrawdebug", 234),
    (b"redrawtime", 235),
    (b"regexpengine", 236),
    (b"relativenumber", 237),
    (b"remap", 238),
    (b"report", 239),
    (b"revins", 240),
    (b"ri", 240),
    (b"rightleft", 241),
    (b"rightleftcmd", 242),
    (b"rl", 241),
    (b"rlc", 242),
    (b"rnu", 237),
    (b"ro", 233),
    (b"rtp", 245),
    (b"ru", 243),
    (b"ruf", 244),
    (b"ruler", 243),
    (b"rulerformat", 244),
    (b"runtimepath", 245),
    (b"sb", 293),
    (b"sbo", 252),
    (b"sbr", 272),
    (b"sc", 273),
    (b"scb", 248),
    (b"scbk", 247),
    (b"scl", 281),
    (b"scr", 246),
    (b"scroll", 246),
    (b"scrollback", 247),
    (b"scrollbind", 248),
    (b"scrolljump", 249),
    (b"scrolloff", 250),
    (b"scrolloffpad", 251),
    (b"scrollopt", 252),
    (b"scs", 282),
    (b"sd", 258),
    (b"sdf", 259),
    (b"sect", 253),
    (b"sections", 253),
    (b"secure", 254),
    (b"sel", 255),
    (b"selection", 255),
    (b"selectmode", 256),
    (b"sessionoptions", 257),
    (b"sft", 275),
    (b"sh", 260),
    (b"shada", 258),
    (b"shadafile", 259),
    (b"shcf", 261),
    (b"shell", 260),
    (b"shellcmdflag", 261),
    (b"shellpipe", 262),
    (b"shellquote", 263),
    (b"shellredir", 264),
    (b"shellslash", 265),
    (b"shelltemp", 266),
    (b"shellxescape", 267),
    (b"shellxquote", 268),
    (b"shiftround", 269),
    (b"shiftwidth", 270),
    (b"shm", 271),
    (b"shortmess", 271),
    (b"showbreak", 272),
    (b"showcmd", 273),
    (b"showcmdloc", 274),
    (b"showfulltag", 275),
    (b"showmatch", 276),
    (b"showmode", 277),
    (b"showtabline", 278),
    (b"shq", 263),
    (b"si", 283),
    (b"sidescroll", 279),
    (b"sidescrolloff", 280),
    (b"signcolumn", 281),
    (b"siso", 280),
    (b"sj", 249),
    (b"slm", 256),
    (b"sloc", 274),
    (b"sm", 276),
    (b"smartcase", 282),
    (b"smartindent", 283),
    (b"smarttab", 284),
    (b"smc", 303),
    (b"smd", 277),
    (b"smoothscroll", 285),
    (b"sms", 285),
    (b"so", 250),
    (b"softtabstop", 286),
    (b"sol", 296),
    (b"sop", 251),
    (b"sp", 262),
    (b"spc", 288),
    (b"spell", 287),
    (b"spellcapcheck", 288),
    (b"spellfile", 289),
    (b"spelllang", 290),
    (b"spelloptions", 291),
    (b"spellsuggest", 292),
    (b"spf", 289),
    (b"spk", 294),
    (b"spl", 290),
    (b"splitbelow", 293),
    (b"splitkeep", 294),
    (b"splitright", 295),
    (b"spo", 291),
    (b"spr", 295),
    (b"sps", 292),
    (b"sr", 269),
    (b"srr", 264),
    (b"ss", 279),
    (b"ssl", 265),
    (b"ssop", 257),
    (b"sta", 284),
    (b"stal", 278),
    (b"startofline", 296),
    (b"statuscolumn", 297),
    (b"statusline", 298),
    (b"stc", 297),
    (b"stl", 298),
    (b"stmp", 266),
    (b"sts", 286),
    (b"su", 299),
    (b"sua", 300),
    (b"suffixes", 299),
    (b"suffixesadd", 300),
    (b"sw", 270),
    (b"swapfile", 301),
    (b"swb", 302),
    (b"swf", 301),
    (b"switchbuf", 302),
    (b"sxe", 267),
    (b"sxq", 268),
    (b"syn", 304),
    (b"synmaxcol", 303),
    (b"syntax", 304),
    (b"tabclose", 305),
    (b"tabline", 306),
    (b"tabpagemax", 307),
    (b"tabstop", 308),
    (b"tag", 314),
    (b"tagbsearch", 309),
    (b"tagcase", 310),
    (b"tagfunc", 311),
    (b"taglength", 312),
    (b"tagrelative", 313),
    (b"tags", 314),
    (b"tagstack", 315),
    (b"tal", 306),
    (b"tbidi", 316),
    (b"tbs", 309),
    (b"tc", 310),
    (b"tcl", 305),
    (b"tenc", 317),
    (b"termbidi", 316),
    (b"termencoding", 317),
    (b"termguicolors", 318),
    (b"termpastefilter", 319),
    (b"termsync", 320),
    (b"terse", 321),
    (b"textwidth", 322),
    (b"tf", 334),
    (b"tfu", 311),
    (b"tgc", 318),
    (b"tgst", 315),
    (b"thesaurus", 323),
    (b"thesaurusfunc", 324),
    (b"tildeop", 325),
    (b"timeout", 326),
    (b"timeoutlen", 327),
    (b"title", 328),
    (b"titlelen", 329),
    (b"titleold", 330),
    (b"titlestring", 331),
    (b"tl", 312),
    (b"tm", 327),
    (b"to", 326),
    (b"top", 325),
    (b"tpf", 319),
    (b"tpm", 307),
    (b"tr", 313),
    (b"ts", 308),
    (b"tsr", 323),
    (b"tsrfu", 324),
    (b"ttimeout", 332),
    (b"ttimeoutlen", 333),
    (b"ttm", 333),
    (b"ttyfast", 334),
    (b"tw", 322),
    (b"uc", 339),
    (b"udf", 336),
    (b"udir", 335),
    (b"ul", 337),
    (b"undodir", 335),
    (b"undofile", 336),
    (b"undolevels", 337),
    (b"undoreload", 338),
    (b"updatecount", 339),
    (b"updatetime", 340),
    (b"ur", 338),
    (b"ut", 340),
    (b"varsofttabstop", 341),
    (b"vartabstop", 342),
    (b"vb", 348),
    (b"vbs", 343),
    (b"vdir", 345),
    (b"ve", 347),
    (b"verbose", 343),
    (b"verbosefile", 344),
    (b"vfile", 344),
    (b"vi", 258),
    (b"viewdir", 345),
    (b"viewoptions", 346),
    (b"vif", 259),
    (b"viminfo", 258),
    (b"viminfofile", 259),
    (b"virtualedit", 347),
    (b"visualbell", 348),
    (b"vop", 346),
    (b"vsts", 341),
    (b"vts", 342),
    (b"wa", 376),
    (b"wak", 358),
    (b"warn", 349),
    (b"wb", 377),
    (b"wbr", 359),
    (b"wc", 351),
    (b"wcm", 352),
    (b"wd", 378),
    (b"wfb", 363),
    (b"wfh", 364),
    (b"wfw", 365),
    (b"wh", 366),
    (b"whichwrap", 350),
    (b"wi", 362),
    (b"wic", 354),
    (b"wig", 353),
    (b"wildchar", 351),
    (b"wildcharm", 352),
    (b"wildignore", 353),
    (b"wildignorecase", 354),
    (b"wildmenu", 355),
    (b"wildmode", 356),
    (b"wildoptions", 357),
    (b"wim", 356),
    (b"winaltkeys", 358),
    (b"winbar", 359),
    (b"winbl", 360),
    (b"winblend", 360),
    (b"winborder", 361),
    (b"window", 362),
    (b"winfixbuf", 363),
    (b"winfixheight", 364),
    (b"winfixwidth", 365),
    (b"winheight", 366),
    (b"winhighlight", 367),
    (b"winhl", 367),
    (b"winminheight", 368),
    (b"winminwidth", 369),
    (b"winpinned", 370),
    (b"winwidth", 371),
    (b"wiw", 371),
    (b"wm", 373),
    (b"wmh", 368),
    (b"wmnu", 355),
    (b"wmw", 369),
    (b"wop", 357),
    (b"wp", 370),
    (b"wrap", 372),
    (b"wrapmargin", 373),
    (b"wrapscan", 374),
    (b"write", 375),
    (b"writeany", 376),
    (b"writebackup", 377),
    (b"writedelay", 378),
    (b"ws", 374),
    (b"ww", 350),
];

/// Find index for an option. Don't go beyond `len` length (`find_option_len`).
pub fn find_option_len(name: &[u8], len: usize) -> OptIndex {
    let slice = crate::memory::bytes_until_nul(&name[..len.min(name.len())]);
    match OPTION_NAME_MAP.binary_search_by_key(&slice, |&(k, _)| k) {
        Ok(i) => OPTION_NAME_MAP[i].1,
        Err(_) => K_OPT_INVALID,
    }
}

/// Find index for an option.
pub fn find_option(name: &[u8]) -> OptIndex {
    let slice = crate::memory::bytes_until_nul(name);
    find_option_len(slice, slice.len())
}

/// True if `opt_idx` is a dict option (`schema.dict` in options.lua, e.g. 'diffopt').
pub fn is_dict_option(opt_idx: OptIndex) -> bool {
    opt_dict_schema(opt_idx).is_some()
}

/// Release an option value read from opt_from_varp() (and similar).
pub fn optval_free_read(opt_idx: OptIndex, value: Object) {
    if optval_is_owned(opt_idx) {
        optval_free(value);
    }
}

/// Free an allocated option value.
pub fn optval_free(o: Object) {
    match o.obj_type {
        ObjectType::ObjectTypeString => {
            api_free_string(o.data.string());
        }
        ObjectType::ObjectTypeLuaRef => {
            api_free_luaref(o.data.luaref());
        }
        ObjectType::ObjectTypeUnset
        | ObjectType::ObjectTypeNil
        | ObjectType::ObjectTypeBoolean
        | ObjectType::ObjectTypeInteger => {}
        _ => std::process::abort(),
    }
}

/// Check if two option values are equal.
pub fn option_equal(o1: Object, o2: Object) -> bool {
    if o1.obj_type != o2.obj_type {
        return false;
    }

    match o1.obj_type {
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => true,
        ObjectType::ObjectTypeBoolean => o1.data.boolean() == o2.data.boolean(),
        ObjectType::ObjectTypeInteger => o1.data.integer() == o2.data.integer(),
        ObjectType::ObjectTypeString => o1.data.string().as_bytes() == o2.data.string().as_bytes(),
        ObjectType::ObjectTypeLuaRef => false,
        _ => std::process::abort(),
    }
}

/// Convert a tri-state boolean option to Object.
pub fn opt_from_tristate(t: TriState) -> Object {
    if t == K_NONE {
        unset_obj()
    } else {
        boolean_obj(t == K_TRUE)
    }
}

fn optval_type(o: Object) -> ObjectType {
    match o.obj_type {
        ObjectType::ObjectTypeNil => ObjectType::ObjectTypeNil,
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeBoolean => {
            ObjectType::ObjectTypeBoolean
        }
        ObjectType::ObjectTypeInteger => ObjectType::ObjectTypeInteger,
        ObjectType::ObjectTypeString => ObjectType::ObjectTypeString,
        ObjectType::ObjectTypeLuaRef => ObjectType::ObjectTypeLuaRef,
        _ => std::process::abort(),
    }
}

fn is_callback_option(opt_idx: OptIndex) -> bool {
    (get_option(opt_idx).flags & (K_OPT_FLAG_FUNC | K_OPT_FLAG_EXPR)) != 0
}

fn optval_is_owned(opt_idx: OptIndex) -> bool {
    is_callback_option(opt_idx)
}

pub fn optval_own(opt_idx: OptIndex, value: Object) -> Object {
    if optval_is_owned(opt_idx) {
        value
    } else {
        copy_object(value, None)
    }
}

fn optval_snapshot(value: Object) -> Object {
    if value.obj_type == ObjectType::ObjectTypeLuaRef {
        let s = nlua_funcref_str(value.data.luaref(), true);
        return cstr_to_obj(Some(s.as_c_str()));
    }
    copy_object(value, None)
}

fn optval_snapshot_varp(opt_idx: OptIndex, varp: OptVar) -> Object {
    let value = opt_from_varp(opt_idx, varp);
    let snapshot = optval_snapshot(value.clone());
    optval_free_read(opt_idx, value);
    snapshot
}

fn opt_from_callback(cb: &Callback) -> Object {
    match cb.cb_type() {
        K_CALLBACK_LUA => {
            let luaref = cb.data.as_luaref();
            return luaref_obj(api_new_luaref(luaref));
        }
        K_CALLBACK_EXPR | K_CALLBACK_FUNCREF => {
            return cstr_to_obj(cb.data.as_funcref());
        }
        K_CALLBACK_PARTIAL => {
            let mut tv = TypVal::new();
            callback_put(cb, &mut tv);
            let str_ = encode_tv2string(&mut tv, None);
            tv_clear(&mut tv);
            return cstr_to_obj(Some(&*str_));
        }
        K_CALLBACK_NONE_CONST => {}
        _ => {}
    }
    cstr_to_obj(Some(c""))
}

fn opt_to_callback(value: Object, is_expr: bool) -> Callback {
    let mut cb = Callback::NONE;
    match value.obj_type {
        ObjectType::ObjectTypeLuaRef => {
            cb.data
                .set(crate::eval::typval::CallbackData::lua(api_new_luaref(
                    value.data.luaref(),
                )));
        }
        ObjectType::ObjectTypeString => {
            if !is_expr {
                option_set_callback_func(value.data.string().as_bytes(), &mut cb);
            } else if value.data.string().size > 0 {
                let s_val = value.data.string();
                let funcref = if let Some(name) = get_scriptlocal_funcname(s_val.as_opt_cstr()) {
                    name
                } else {
                    s_val.to_cstring()
                };
                cb.data
                    .set(crate::eval::typval::CallbackData::expr(funcref));
            }
        }
        _ => {}
    }
    cb
}

/// Creates Object from var pointer.
pub fn opt_from_varp(opt_idx: OptIndex, varp: OptVar) -> Object {
    let opt = get_option(opt_idx);
    if opt.immutable {
        return if opt.opt_type == ObjectType::ObjectTypeString {
            string_obj(cstr_as_string(opt.def_val.data.string().as_opt_cstr()))
        } else {
            opt.def_val
        };
    }

    assert!(varp.is_some());
    if varp.ptr_eq(&curbuf_mut().b_changed.get()) {
        return boolean_obj(curbuf_is_changed_fn());
    }

    if is_callback_option(opt_idx) {
        return opt_from_callback(varp.callback_mut());
    }

    match opt.opt_type {
        ObjectType::ObjectTypeBoolean => {
            opt_from_tristate(tristate_from_int(OptInt::from(varp.bool_tristate())))
        }
        ObjectType::ObjectTypeInteger => integer_obj(varp.int_val()),
        ObjectType::ObjectTypeString => string_obj(cstr_as_string(varp.opt_cstr())),
        _ => std::process::abort(),
    }
}

/// Stores an option value at its "varp", freeing the previously-stored value.
fn set_option_varp(opt_idx: OptIndex, varp: OptVar, value: Object) {
    let opt = get_option(opt_idx);
    if opt.immutable {
        return;
    }

    if is_callback_option(opt_idx) {
        let cb = varp.callback_mut();
        callback_free(cb);
        cb.data.set(
            opt_to_callback(value, (opt.flags & K_OPT_FLAG_EXPR) != 0)
                .data
                .get(),
        );
        return;
    }

    match value.obj_type {
        ObjectType::ObjectTypeUnset => {
            assert!(option_is_global_local(opt_idx));
            if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
                varp.set_bool(None);
            } else if option_has_type(opt_idx, ObjectType::ObjectTypeString) {
                varp.clear_str();
            } else if opt_idx == K_OPT_UNDOLEVELS {
                varp.set_int(NO_LOCAL_UNDOLEVEL);
            } else {
                assert!(
                    opt_idx == K_OPT_SCROLLOFF
                        || opt_idx == K_OPT_SCROLLOFFPAD
                        || opt_idx == K_OPT_SIDESCROLLOFF
                );
                varp.set_int(-1);
            }
        }
        ObjectType::ObjectTypeBoolean => {
            assert!(option_has_type(opt_idx, ObjectType::ObjectTypeBoolean));
            varp.set_bool(Some(value.data.boolean()));
        }
        ObjectType::ObjectTypeInteger => {
            assert!(option_has_type(opt_idx, ObjectType::ObjectTypeInteger));
            varp.set_int(value.data.integer());
        }
        ObjectType::ObjectTypeString => {
            assert!(option_has_type(opt_idx, ObjectType::ObjectTypeString));
            varp.set_bytes(value.data.string().as_bytes());
        }
        _ => std::process::abort(),
    }
}

/// Gets the string form of an option value.
fn optval_to_cstr(o: Object, quote: bool) -> std::ffi::CString {
    match o.obj_type {
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => std::ffi::CString::default(),
        ObjectType::ObjectTypeBoolean => {
            if o.data.boolean() {
                c"true".to_owned()
            } else {
                c"false".to_owned()
            }
        }
        ObjectType::ObjectTypeInteger => {
            let s = format!("{}", o.data.integer());
            crate::memory::bytes_to_cstring(s.as_bytes())
        }
        ObjectType::ObjectTypeString => {
            let slice = if o.data.string().data.is_null() || o.data.string().size == 0 {
                &[][..]
            } else {
                o.data.as_bytes()
            };
            if !quote {
                return crate::memory::bytes_to_cstring(slice);
            }
            let mut out = Vec::with_capacity(slice.len() + 2);
            out.push(b'"');
            out.extend_from_slice(slice);
            out.push(b'"');
            crate::memory::bytes_to_cstring(&out)
        }
        ObjectType::ObjectTypeLuaRef => nlua_funcref_str(o.data.luaref(), false),
        _ => std::process::abort(),
    }
}

/// Converts a structured option (non-scalar Object) to the internal scalar (:set-style) form.
pub fn optval_from_obj(opt_idx: OptIndex, o: Object, op: SetOp, error: &mut bool) -> Object {
    if o.obj_type == ObjectType::ObjectTypeNil {
        return nil_obj();
    }

    let flags = get_option(opt_idx).flags;
    let is_list = (flags & (K_OPT_FLAG_COMMA | K_OPT_FLAG_FLAG_LIST)) != 0;
    let is_map = (flags & K_OPT_FLAG_COLON) != 0 || is_dict_option(opt_idx);
    let is_flaglist = (flags & K_OPT_FLAG_FLAG_LIST) != 0;
    let is_comma = (flags & K_OPT_FLAG_COMMA) != 0;
    let allow_dup = (flags & K_OPT_FLAG_NO_DUP) == 0;

    let type_ok = match o.obj_type {
        ObjectType::ObjectTypeBoolean => option_has_type(opt_idx, ObjectType::ObjectTypeBoolean),
        ObjectType::ObjectTypeInteger => option_has_type(opt_idx, ObjectType::ObjectTypeInteger),
        ObjectType::ObjectTypeString => {
            option_has_type(opt_idx, ObjectType::ObjectTypeString)
                || opt_idx == K_OPT_WILDCHAR
                || opt_idx == K_OPT_WILDCHARM
        }
        ObjectType::ObjectTypeArray => is_list,
        ObjectType::ObjectTypeDict => is_map || is_flaglist,
        ObjectType::ObjectTypeLuaRef => is_callback_option(opt_idx),
        _ => false,
    };
    if !type_ok {
        *error = true;
        return nil_obj();
    }

    match o.obj_type {
        ObjectType::ObjectTypeBoolean | ObjectType::ObjectTypeInteger => return o,
        ObjectType::ObjectTypeLuaRef => return luaref_obj(api_new_luaref(o.data.luaref())),
        _ => {}
    }

    let mut str_cstr: std::ffi::CString = if o.obj_type == ObjectType::ObjectTypeString {
        o.data.string().to_cstring()
    } else if o.obj_type == ObjectType::ObjectTypeArray {
        let mut items: Vec<std::ffi::CString> = Vec::with_capacity(o.data.array().size);
        for ai in 0..o.data.array().size {
            let item = o.data.array()[ai].clone();
            if item.obj_type != ObjectType::ObjectTypeString {
                *error = true;
                return nil_obj();
            }
            let s_cstr = item.data.as_cstr();
            if !allow_dup && items.iter().any(|existing| existing.as_c_str() == s_cstr) {
                continue;
            }
            items.push(s_cstr.to_owned());
        }
        join_cstrings(&items, if is_flaglist && !is_comma { b"" } else { b"," })
    } else {
        let dict = o.data.dict();
        let mut items: Vec<std::ffi::CString> = Vec::with_capacity(dict.size);
        for di in 0..dict.size {
            let k = &dict[di].key;
            let v = &dict[di].value;
            if is_flaglist {
                if !(v.obj_type == ObjectType::ObjectTypeNil
                    || (v.obj_type == ObjectType::ObjectTypeBoolean && !v.data.boolean()))
                {
                    items.push(k.to_cstring());
                }
            } else if v.obj_type == ObjectType::ObjectTypeString {
                let k_bytes = k.cstr_bytes();
                let v_bytes = v.data.cstr_bytes();
                let mut kv = Vec::with_capacity(k_bytes.len() + 1 + v_bytes.len());
                kv.extend_from_slice(k_bytes);
                kv.push(b':');
                kv.extend_from_slice(v_bytes);
                items.push(crate::memory::bytes_to_cstring(&kv));
            } else if v.obj_type == ObjectType::ObjectTypeInteger {
                let k_bytes = k.cstr_bytes();
                let num_s = format!(":{}", v.data.integer());
                let mut kv = Vec::with_capacity(k_bytes.len() + num_s.len());
                kv.extend_from_slice(k_bytes);
                kv.extend_from_slice(num_s.as_bytes());
                items.push(crate::memory::bytes_to_cstring(&kv));
            } else if v.obj_type == ObjectType::ObjectTypeBoolean
                || v.obj_type == ObjectType::ObjectTypeNil
            {
                if v.obj_type == ObjectType::ObjectTypeBoolean && v.data.boolean() {
                    items.push(k.to_cstring());
                }
            } else {
                *error = true;
                return nil_obj();
            }
        }
        if !items.is_empty() && (is_map || is_comma) {
            items.sort_unstable();
        }
        join_cstrings(&items, if is_flaglist && !is_comma { b"" } else { b"," })
    };

    if op == OP_REMOVING && is_map && !str_cstr.as_bytes().contains(&b':') {
        let mut bytes = str_cstr.into_bytes();
        bytes.push(b':');
        str_cstr = std::ffi::CString::new(bytes).unwrap();
    }

    cstr_to_obj(Some(&*str_cstr))
}

/// Check if option is hidden.
pub fn is_option_hidden(opt_idx: OptIndex) -> bool {
    opt_idx != K_OPT_INVALID && get_option(opt_idx).immutable
}

/// Check if option supports a specific type.
pub fn option_has_type(opt_idx: OptIndex, opt_type: ObjectType) -> bool {
    opt_idx != K_OPT_INVALID && get_option(opt_idx).opt_type == opt_type
}

/// Check if option supports a specific scope.
pub fn option_has_scope(opt_idx: OptIndex, scope: OptScope) -> bool {
    assert!((K_OPT_SCOPE_GLOBAL..K_OPT_SCOPE_SIZE).contains(&scope));
    (get_option(opt_idx).scope_flags & (1u8 << scope)) != 0
}

#[inline]
fn option_is_global_local(opt_idx: OptIndex) -> bool {
    if opt_idx == K_OPT_INVALID {
        return false;
    }
    let bw: OptScopeFlags = (1 << K_OPT_SCOPE_BUF) | (1 << K_OPT_SCOPE_WIN);
    (get_option(opt_idx).scope_flags & bw) != 0 && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL)
}

#[inline]
fn option_is_global_only(opt_idx: OptIndex) -> bool {
    if opt_idx == K_OPT_INVALID {
        return false;
    }
    let bw: OptScopeFlags = (1 << K_OPT_SCOPE_BUF) | (1 << K_OPT_SCOPE_WIN);
    (get_option(opt_idx).scope_flags & bw) == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL)
}

#[inline]
fn option_is_window_local(opt_idx: OptIndex) -> bool {
    if opt_idx == K_OPT_INVALID {
        return false;
    }
    let exclude: OptScopeFlags = (1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF);
    (get_option(opt_idx).scope_flags & exclude) == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_WIN)
}

/// Get option index for scope.
pub fn option_scope_idx(opt_idx: OptIndex, scope: OptScope) -> isize {
    get_option(opt_idx).scope_idx[scope as usize]
}

/// Gets the value for an option.
pub fn get_option_value(opt_idx: OptIndex, opt_flags: i32) -> Object {
    if opt_idx == K_OPT_INVALID {
        return nil_obj();
    }
    let opt = get_option(opt_idx);
    optval_own(
        opt_idx,
        opt_from_varp(opt_idx, get_varp_scope(opt, opt_flags)),
    )
}

fn optval_is_unset_local(opt_idx: OptIndex, value: Object) -> bool {
    if !option_is_global_local(opt_idx) {
        return false;
    }
    if value.obj_type == ObjectType::ObjectTypeUnset {
        return true;
    }
    if option_has_type(opt_idx, ObjectType::ObjectTypeString) {
        return value.obj_type == ObjectType::ObjectTypeString && value.data.string().size == 0;
    }
    if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean)
        || value.obj_type != ObjectType::ObjectTypeInteger
    {
        return false;
    }
    match opt_idx {
        K_OPT_SCROLLOFF | K_OPT_SCROLLOFFPAD | K_OPT_SIDESCROLLOFF => value.data.integer() == -1,
        K_OPT_UNDOLEVELS => value.data.integer() == NO_LOCAL_UNDOLEVEL,
        _ => std::process::abort(),
    }
}

fn is_option_local_value_unset(opt_idx: OptIndex) -> bool {
    if !option_is_global_local(opt_idx) {
        return false;
    }
    let opt = get_option(opt_idx);
    let local_value = opt_from_varp(opt_idx, get_varp_scope(opt, OPT_LOCAL));
    let is_unset = optval_is_unset_local(opt_idx, local_value.clone());
    optval_free_read(opt_idx, local_value);
    is_unset
}

/// Handle side-effects of setting an option.
fn did_set_option<'a>(
    args: &mut Optset<'a>,
    set_sid: Scid,
    direct: bool,
    value_replaced: bool,
    value_untrusted: bool,
) -> Option<&'a CStr> {
    let opt_idx = args.os_idx;
    let varp = args.os_varp;
    let opt_flags = args.os_flags;
    let opt = get_option(opt_idx);
    let mut errmsg: Option<&'a CStr> = None;

    if direct {
        // Don't do any extra processing if setting directly.
    } else if (g_st().secure.get() != 0 || g_st().sandbox.get() != 0)
        && (opt.flags & K_OPT_FLAG_SECURE) != 0
    {
        errmsg = Some(E_SECURE);
    } else if args.os_newval.obj_type == ObjectType::ObjectTypeString
        && !is_callback_option(opt_idx)
        && (opt.flags & (K_OPT_FLAG_NFNAME | K_OPT_FLAG_NDNAME)) != 0
        && check_illegal_path_names(args.var_cstr().to_bytes(), opt.flags)
    {
        errmsg = Some(E_INVARG);
    } else if let Some(cb) = opt.opt_did_set_cb {
        errmsg = cb(args);
    }

    if errmsg.is_some() {
        set_option_varp(opt_idx, varp, args.os_oldval.clone());
        optval_free(args.os_oldval.clone());
        return errmsg;
    }

    let new_value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));

    if set_sid != SID_NONE {
        let script_ctx = if set_sid == 0 {
            g_st().current_sctx.get()
        } else {
            Sctx {
                sc_sid: set_sid,
                sc_seq: 0,
                sc_lnum: 0,
                sc_chan: 0,
            }
        };
        set_option_sctx(opt_idx, opt_flags, script_ctx);
    }

    optval_free(args.os_oldval.clone());

    let scope_both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;

    if scope_both {
        if option_is_global_local(opt_idx) {
            let varp_local = get_varp_scope(opt.clone(), OPT_LOCAL);
            if varp_local.is_some() {
                set_option_varp(opt_idx, varp_local, unset_obj());
            }
        } else {
            let varp_global = get_varp_scope(opt.clone(), OPT_GLOBAL);
            if varp_global.is_some() && varp_global != varp {
                set_option_varp(opt_idx, varp_global, new_value.clone());
            }
        }
    }

    optval_free(new_value);

    if direct {
        return errmsg;
    }

    let ws = curwin_mut().w_s_ptr();
    if varp.ptr_eq(&curbuf_mut().b_p_syn) {
        do_syntax_autocmd(curbuf_mut(), args.os_value_changed);
    } else if varp.ptr_eq(&curbuf_mut().b_p_ft) {
        if (opt_flags & OPT_MODELINE) == 0 || args.os_value_changed {
            do_filetype_autocmd(curbuf_mut(), args.os_value_changed);
        }
    } else if varp.ptr_eq(&ws.b_p_spl) {
        do_spelllang_source(curwin_mut());
    }

    comp_col();

    if varp.ptr_eq(&opt_vars().p_mouse) {
        setmouse();
    } else if (varp.ptr_eq(&opt_vars().p_flp) || varp.ptr_eq(&curbuf_mut().b_p_flp))
        && curwin_mut().w_briopt_list.get() != 0
    {
        redraw_all_later(UPD_NOT_VALID);
    } else if varp.ptr_eq(&opt_vars().p_wbr) || varp.ptr_eq(&curwin_mut().w_onebuf_opt.wo_wbr) {
        if varp.ptr_eq(&opt_vars().p_wbr) {
            set_winbar_all(true);
        } else {
            set_winbar(true);
        }
    }

    if curwin_mut().w_curswant.get() != MAXCOL
        && (opt.flags & (K_OPT_FLAG_CURSWANT | K_OPT_FLAG_REDR_ALL)) != 0
        && (opt.flags & K_OPT_FLAG_HLONLY) == 0
    {
        curwin_mut().w_set_curswant.set(true);
    }

    check_redraw(opt.flags);

    if errmsg.is_none() {
        OPTIONS.with(|opts| opts.borrow_mut()[opt_idx as usize].flags |= K_OPT_FLAG_WAS_SET);

        if !args.os_value_checked && value_untrusted {
            set_insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, opt_flags, true);
            if scope_both {
                set_insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, OPT_LOCAL, true);
            }
        } else if value_replaced {
            set_insecure_flag(
                Some(crate::globals::curwin_mut()),
                opt_idx,
                opt_flags,
                false,
            );
            if scope_both {
                set_insecure_flag(
                    Some(crate::globals::curwin_mut()),
                    opt_idx,
                    OPT_LOCAL,
                    false,
                );
            }
        }
    }

    errmsg
}

/// Validates an option value and prepares any data needed to apply it, without side effects.
pub fn validate_and_prepare_option_value<'a>(args: &mut Optset<'a>) -> Option<&'a CStr> {
    let opt_idx = args.os_idx;
    let opt_flags = args.os_flags;
    let buf = args.buf_mut();
    let win = args.win_mut();
    let newval = &mut args.os_newval;
    let mut errmsg: Option<&'a CStr> = None;
    let opt = get_option(opt_idx);

    if (opt_flags & OPT_LOCAL) != 0 && optval_is_unset_local(opt_idx, newval.clone()) {
        return None;
    }

    if newval.obj_type == ObjectType::ObjectTypeNil {
        if opt_flags == OPT_GLOBAL {
            errmsg = Some(tr(c"Cannot unset global option value"));
        } else {
            let varp = get_varp_scope_from(opt.clone(), OPT_GLOBAL, buf, Some(&mut *win));
            *newval = if option_is_global_local(opt_idx) {
                unset_obj()
            } else {
                optval_own(opt_idx, opt_from_varp(opt_idx, varp))
            };
        }
    } else if newval.obj_type == ObjectType::ObjectTypeLuaRef {
        assert!(is_callback_option(opt_idx));
    } else if !option_has_type(opt_idx, optval_type(newval.clone())) {
        let rep = optval_to_cstr(newval.clone(), true);
        let type_str = optval_type_name(opt.opt_type);
        errmsg = if let Some(errbuf) = args.os_errbuf.take() {
            format_to_buf!(
                errbuf,
                gettext(c"Invalid value for option '%s': expected %s, got %s %s"),
                opt.fullname,
                type_str,
                optval_type_name(optval_type(newval.clone())),
                &rep,
            );
            Some(crate::memory::bytes_as_cstr(errbuf))
        } else {
            Some(c"")
        };
    } else if newval.obj_type == ObjectType::ObjectTypeInteger {
        errmsg = validate_num_option(
            opt_idx,
            newval.data.integer_mut(),
            win,
            args.os_errbuf.take(),
        );
    }
    if errmsg.is_some() {
        return errmsg;
    }

    if opt.immutable {
        let oldval = opt_from_varp(
            opt_idx,
            get_varp_scope_from(opt.clone(), opt_flags, buf, Some(&mut *win)),
        );
        let unchanged = option_equal(oldval.clone(), newval.clone());
        optval_free_read(opt_idx, oldval);
        if !unchanged {
            return Some(E_UNSUPPORTEDOPTION);
        }
    }
    let schema = opt.schema;
    if newval.obj_type == ObjectType::ObjectTypeString && !schema.is_empty() {
        errmsg = opt_strings_check(newval.data.as_bytes(), schema, args.os_errbuf.take());
        if errmsg.is_some() {
            return errmsg;
        }
    }
    if newval.obj_type != ObjectType::ObjectTypeUnset
        && let Some(validate_cb) = opt.opt_validate_cb
    {
        let scope_both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
        let varp = if scope_both && option_is_global_local(opt_idx) {
            opt.var()
        } else {
            get_varp_scope_from(opt, opt_flags, buf, Some(&mut *win))
        };
        args.os_varp = varp;
        args.os_oldval = opt_from_varp(opt_idx, varp);
        let validate_err = validate_cb(args);
        optval_free_read(opt_idx, args.os_oldval.clone());
        args.os_oldval = nil_obj();
        return validate_err;
    }
    None
}

/// Set the value of an option using an Object.
fn set_option<'a>(
    opt_idx: OptIndex,
    mut value: Object,
    opt_flags: i32,
    set_sid: Scid,
    direct: bool,
    value_replaced: bool,
    errbuf: Option<&'a mut [u8]>,
) -> Option<&'a CStr> {
    assert!(opt_idx != K_OPT_INVALID);

    let mut errmsg: Option<&'a CStr> = None;
    let mut args = Optset {
        os_varp: OptVar::None,
        os_idx: opt_idx,
        os_flags: opt_flags,
        os_oldval: nil_obj(),
        os_newval: value.clone(),
        os_prep: crate::optionstr::OptSetPrep::None,
        os_value_checked: false,
        os_value_changed: false,
        os_restore_chartab: false,
        os_errbuf: errbuf,
        os_win: crate::globals::curwin_ptr(),
        os_buf: g_st().curbuf.get(),
    };

    let opt = get_option(opt_idx);
    if !direct {
        if (g_st().secure.get() != 0 || g_st().sandbox.get() != 0)
            && (opt.flags & K_OPT_FLAG_SECURE) != 0
        {
            optval_free(value);
            return Some(E_SECURE);
        }

        errmsg = validate_and_prepare_option_value(&mut args);
        value = args.os_newval.clone();

        if errmsg.is_none()
            && value.obj_type == ObjectType::ObjectTypeString
            && (opt.flags & K_OPT_FLAG_FUNC) != 0
            && value.data.string().size > 0
        {
            let mut cb = Callback::NONE;
            if !option_set_callback_func(value.data.as_bytes(), &mut cb) {
                errmsg = Some(E_INVARG);
            }
            callback_free(&cb);
        }
    } else if value.obj_type == ObjectType::ObjectTypeString && is_dict_option(opt_idx) {
        errmsg = opt_strings_check(
            value.data.as_bytes(),
            opt_dict_schema(opt_idx).unwrap().schema,
            args.os_errbuf.take(),
        );
    }

    if errmsg.is_some() {
        optval_free(value);
        return errmsg;
    }

    #[cfg(windows)]
    {
        let flags = opt.flags;
        if value.obj_type == ObjectType::ObjectTypeString
            && (flags & K_OPT_FLAG_EXPAND) != 0
            && opt_idx != K_OPT_EQUALPRG
            && opt_idx != K_OPT_FORMATPRG
            && opt_idx != K_OPT_GREPPRG
            && opt_idx != K_OPT_KEYWORDPRG
            && opt_idx != K_OPT_MAKEPRG
            && opt_idx != K_OPT_SHELL
        {
            let allow_comma = (flags & K_OPT_FLAG_COMMA) != 0;
            let allow_space =
                opt_idx == K_OPT_CDPATH || opt_idx == K_OPT_PATH || opt_idx == K_OPT_TAGS;
            let str_slice = value.data.string_mut().as_bytes_with_nul_mut();
            let mut i = 0;
            while i < str_slice.len() && str_slice[i] != 0 {
                if str_slice[i] != b'\\'
                    || (str_slice.get(i + 1) == Some(&b',') && allow_comma)
                    || (str_slice.get(i + 1) == Some(&b' ') && allow_space)
                {
                    i += 1;
                    continue;
                }
                str_slice[i] = b'/';
                i += 1;
            }
        }
    }

    let scope_local = (opt_flags & OPT_LOCAL) != 0;
    let scope_global = (opt_flags & OPT_GLOBAL) != 0;
    let scope_both = !scope_local && !scope_global;
    let is_opt_local_unset = is_option_local_value_unset(opt_idx);

    let varp = if scope_both && option_is_global_local(opt_idx) {
        opt.var()
    } else {
        get_varp_scope(opt.clone(), opt_flags)
    };
    let varp_local = get_varp_scope(opt.clone(), OPT_LOCAL);
    let varp_global = get_varp_scope(opt.clone(), OPT_GLOBAL);

    let old_value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));

    let mut saved_used_value = nil_obj();
    let mut saved_old_global_value = nil_obj();
    let mut saved_old_local_value = nil_obj();
    let mut saved_new_value = nil_obj();
    if !direct {
        saved_old_global_value = optval_snapshot_varp(opt_idx, varp_global);
        saved_old_local_value = if is_opt_local_unset {
            copy_object(saved_old_global_value.clone(), None)
        } else {
            optval_snapshot_varp(opt_idx, varp_local)
        };
        saved_used_value = if scope_local && is_opt_local_unset {
            optval_snapshot_varp(opt_idx, get_varp(opt.clone()))
        } else {
            optval_snapshot(old_value.clone())
        };
    }

    let secure_saved = g_st().secure.get();

    let value_untrusted = g_st().secure.get() != 0
        || g_st().sandbox.get() != 0
        || (opt_flags & OPT_MODELINE) != 0
        || (!value_replaced
            && was_set_insecurely(Some(crate::globals::curwin_mut()), opt_idx, opt_flags) != 0);
    if g_st().secure.get() == 0 && value_untrusted {
        g_st().secure.set(1);
    }

    if varp.is_some() {
        set_option_varp(opt_idx, varp, value.clone());
    }
    if value.obj_type == ObjectType::ObjectTypeUnset {
        value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));
    }
    if !direct {
        saved_new_value = optval_snapshot(value.clone());
    }
    args.os_varp = varp;
    args.os_oldval = old_value;
    args.os_newval = value.clone();
    args.os_buf = g_st().curbuf.get();
    args.os_win = crate::globals::curwin_ptr();
    errmsg = did_set_option(&mut args, set_sid, direct, value_replaced, value_untrusted);
    args.os_prep = crate::optionstr::OptSetPrep::None;

    g_st().secure.set(secure_saved);

    if errmsg.is_none() && !direct {
        if starting_val() == 0 {
            apply_optionset_autocmd(
                opt_idx,
                opt_flags,
                saved_used_value.clone(),
                saved_old_global_value.clone(),
                saved_old_local_value.clone(),
                saved_new_value.clone(),
            );
        }
        if (opt.flags & K_OPT_FLAG_UIOPTION) != 0 {
            ui_call_option_set(cstr_as_string(Some(opt.fullname)), saved_new_value.clone());
        }
    }

    optval_free(saved_used_value);
    optval_free(saved_old_local_value);
    optval_free(saved_old_global_value);
    optval_free(saved_new_value);
    optval_free(value);

    errmsg
}

/// Set option value directly, without processing any side effects.
pub fn set_option_direct(opt_idx: OptIndex, value: Object, opt_flags: i32, set_sid: Scid) {
    if is_option_hidden(opt_idx) {
        return;
    }

    let errmsg = set_option(
        opt_idx,
        copy_object(value, None),
        opt_flags,
        set_sid,
        true,
        true,
        None,
    );
    assert!(errmsg.is_none());
}

/// Set option value directly for buffer / window, without processing any side effects.
pub fn set_option_direct_for(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: i32,
    set_sid: Scid,
    target: OptTarget,
) {
    let save_curbuf = g_st().curbuf.get();
    let save_curwin = curwin_ptr();

    match target {
        OptTarget::Global => {}
        OptTarget::Win(win) => {
            set_curwin(win);
            g_st().curbuf.set(curwin_mut().w_buffer.get());
        }
        OptTarget::Buf(buf) => {
            g_st().curbuf.set(buf);
        }
        OptTarget::Tab(_) => std::process::abort(),
    }

    set_option_direct(opt_idx, value, opt_flags, set_sid);

    set_curwin(save_curwin);
    g_st().curbuf.set(save_curbuf);
}

/// Sets the value of an (non-tty) option.
pub fn set_option_value(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: i32,
    value_replaced: bool,
    errbuf: Option<&mut [u8]>,
) -> Option<&CStr> {
    assert!(opt_idx != K_OPT_INVALID);

    let flags = get_option(opt_idx).flags;
    if g_st().sandbox.get() > 0 && (flags & K_OPT_FLAG_SECURE) != 0 {
        return Some(tr(E_SANDBOX));
    }

    set_option(
        opt_idx,
        copy_object(value, None),
        opt_flags,
        0,
        false,
        value_replaced,
        errbuf,
    )
}

#[inline]
fn unset_option_local_value(opt_idx: OptIndex) -> Option<&'static CStr> {
    assert!(option_is_global_local(opt_idx));
    set_option_value(opt_idx, unset_obj(), OPT_LOCAL, true, None)
}

/// Call set_option_value() and when an error is returned, report it.
pub fn set_option_value_give_err(opt_idx: OptIndex, value: Object, opt_flags: i32) {
    let mut errbuf_storage = [0u8; IOSIZE];
    let errmsg = set_option_value(opt_idx, value, opt_flags, true, Some(&mut errbuf_storage));

    if let Some(errmsg) = errmsg {
        emsg(tr(errmsg));
    }
}

#[derive(Clone)]
pub enum OptTarget {
    Global,
    Win(WinPtr),
    Buf(BufPtr),
    Tab(TabpagePtr),
}

enum OptSavedCtx {
    None,
    Switch(CtxSwitch),
    Tab(TabpagePtr),
}

/// Switch current context to get/set option value for the target window/buffer/tabpage.
fn switch_option_context(
    ctx: &mut OptSavedCtx,
    scope: OptScope,
    from: OptTarget,
    err: &mut Error,
) -> bool {
    match scope {
        K_OPT_SCOPE_GLOBAL => false,
        K_OPT_SCOPE_WIN => {
            let OptTarget::Win(win) = from else {
                return false;
            };
            if win == curwin_ptr() {
                return false;
            }
            let mut cs = CtxSwitch::new();
            let tab = win.opt().and_then(win_find_tabpage);
            if !ctx_switch(
                &mut cs,
                win.opt_mut(),
                tab.as_deref(),
                None,
                K_CTX_NO_DISPLAY,
            ) {
                ctx_restore(&mut cs);
                if !error_set(err) {
                    api_set_error!(
                        err,
                        ErrorType::ErrorTypeException,
                        c"Problem while switching windows",
                    );
                }
                return false;
            }
            *ctx = OptSavedCtx::Switch(cs);
            true
        }
        K_OPT_SCOPE_BUF => {
            let OptTarget::Buf(buf) = from else {
                return false;
            };
            if buf == g_st().curbuf.get().as_ptr() {
                return false;
            }
            let mut cs = CtxSwitch::new();
            ctx_switch(&mut cs, None, None, buf.opt_mut(), 0);
            *ctx = OptSavedCtx::Switch(cs);
            true
        }
        K_OPT_SCOPE_TAB => {
            let OptTarget::Tab(tab) = from else {
                return false;
            };
            if tab == g_st().curtab.get() {
                return false;
            }
            *ctx = OptSavedCtx::Tab(g_st().curtab.get().clone());
            unuse_tabpage(&curtab_mut());
            use_tabpage(tab.borrow());
            true
        }
        _ => unreachable!(),
    }
}

fn restore_option_context(ctx: &mut OptSavedCtx, scope: OptScope) {
    match (scope, ctx) {
        (K_OPT_SCOPE_GLOBAL, _) => {}
        (K_OPT_SCOPE_WIN | K_OPT_SCOPE_BUF, OptSavedCtx::Switch(cs)) => {
            ctx_restore(cs);
        }
        (K_OPT_SCOPE_TAB, OptSavedCtx::Tab(saved_curtab)) => {
            unuse_tabpage(&curtab_mut());
            use_tabpage(saved_curtab.borrow());
        }
        _ => {}
    }
}

/// Get option value for buffer / window.
pub fn get_option_value_for(
    opt_idx: OptIndex,
    opt_flags: i32,
    scope: OptScope,
    from: OptTarget,
    err: &mut Error,
) -> Object {
    let mut ctx = OptSavedCtx::None;

    let switched = switch_option_context(&mut ctx, scope, from, err);
    if error_set(err) {
        return nil_obj();
    }

    let retv = get_option_value(opt_idx, opt_flags);

    if switched {
        restore_option_context(&mut ctx, scope);
    }

    retv
}

/// Set option value for buffer / window.
pub fn set_option_value_for(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: i32,
    scope: OptScope,
    from: OptTarget,
    value_replaced: bool,
    err: &mut Error,
) {
    if let (K_OPT_SCOPE_TAB, OptTarget::Tab(tab)) = (scope, &from)
        && *tab != g_st().curtab.get()
    {
        assert!(opt_idx == K_OPT_CMDHEIGHT);
        if value.obj_type != ObjectType::ObjectTypeInteger {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"'cmdheight' requires a Number",
            );
            return;
        }

        tab.tp_ch_used.set(value.data.integer());

        return;
    }

    let mut ctx = OptSavedCtx::None;

    let switched = switch_option_context(&mut ctx, scope, from, err);
    if error_set(err) {
        return;
    }

    let mut errbuf_storage = [0u8; IOSIZE];
    let errmsg = set_option_value(
        opt_idx,
        value,
        opt_flags,
        value_replaced,
        Some(&mut errbuf_storage),
    );
    if let Some(errmsg) = errmsg {
        api_set_error!(err, ErrorType::ErrorTypeException, c"%s", errmsg);
    }

    if switched {
        restore_option_context(&mut ctx, scope);
    }
}

fn showoptions(all: bool, opt_flags: i32) {
    const INC: i32 = 20;
    const GAP: i32 = 3;

    let mut items: Vec<Vimoption> = Vec::with_capacity(K_OPT_COUNT as usize);

    msg_ext_set_kind(c"list_cmd");
    if (opt_flags & OPT_GLOBAL) != 0 {
        msg_puts_title(tr(c"\n--- Global option values ---"));
    } else if (opt_flags & OPT_LOCAL) != 0 {
        msg_puts_title(tr(c"\n--- Local option values ---"));
    } else {
        msg_puts_title(tr(c"\n--- Options ---"));
    }

    let mut run = 1;
    while run <= 2 && !g_st().got_int.get() {
        items.clear();
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            if message_filtered(opt.fullname) {
                continue;
            }

            let mut varp = OptVar::None;
            if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) != 0 {
                if !option_is_global_only(opt_idx) {
                    varp = get_varp_scope(opt.clone(), opt_flags);
                }
            } else {
                varp = get_varp(opt.clone());
            }
            if varp.is_some() && (all || optval_default(opt_idx, varp) == 0) {
                let len = if (opt_flags & OPT_ONECOLUMN) != 0 {
                    g_st().columns_global.get()
                } else if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
                    1
                } else {
                    optval_fmt(opt_idx, opt_flags);
                    (opt.fullname.to_bytes().len() as i32)
                        + vim_strsize(g_st().name_buff_cstr())
                        + 1
                };
                if (len <= INC - GAP && run == 1) || (len > INC - GAP && run == 2) {
                    items.push(opt);
                }
            }
        }

        let item_count = items.len();
        let rows = if run == 1 {
            let mut cols = (g_st().columns_global.get() + GAP - 3) / INC;
            if cols == 0 {
                cols = 1;
            }
            (item_count as i32 + cols - 1) / cols
        } else {
            item_count as i32
        };

        let mut row = 0;
        while row < rows && !g_st().got_int.get() {
            msg_putchar(b'\n' as i32);
            if g_st().got_int.get() {
                break;
            }
            let mut col: i32 = 0;
            let mut i = row as usize;
            while i < item_count {
                msg_advance(col);
                showoneopt(items[i].clone(), opt_flags);
                col += INC;
                i += rows as usize;
            }
            os_breakcheck();
            row += 1;
        }
        run += 1;
    }
}

fn optval_default(opt_idx: OptIndex, varp: OptVar) -> i32 {
    if is_option_hidden(opt_idx) {
        return 1;
    }

    let current_val = opt_from_varp(opt_idx, varp);
    let default_val = get_option(opt_idx).def_val;

    let is_default = option_equal(current_val.clone(), default_val);
    optval_free_read(opt_idx, current_val);
    i32::from(is_default)
}

/// Send update to UIs with values of UI relevant options.
pub fn ui_refresh_options() {
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        if (opt.flags & K_OPT_FLAG_UIOPTION) == 0 {
            continue;
        }
        let name = cstr_as_string(Some(opt.fullname));
        let value = opt_from_varp(opt_idx, opt.var());
        ui_call_option_set(name, value);
    }
    if !opt_vars().p_mouse.is_null() {
        setmouse();
    }
}

fn showoneopt(opt: Vimoption, opt_flags: i32) {
    let save_silent = g_st().silent_mode.get();

    g_st().silent_mode.set(false);
    g_st().info_message.set(true);

    let opt_idx = get_opt_idx(opt.clone());
    let varp = get_varp_scope(opt.clone(), opt_flags);

    if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
        let val_int = if opt.immutable {
            i32::from(opt.def_val.data.boolean())
        } else if varp.ptr_eq(&curbuf_mut().b_changed.get()) {
            i32::from(curbuf_is_changed_fn())
        } else {
            varp.bool_tristate()
        };
        if val_int == 0 {
            msg_puts(c"no");
        } else if val_int < 0 {
            msg_puts(c"--");
        } else {
            msg_puts(c"  ");
        }
    } else {
        msg_puts(c"  ");
    }
    msg_puts(opt.fullname);
    if !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
        msg_putchar(b'=' as i32);
        optval_fmt(opt_idx, opt_flags);
        let namebuf_cstr = g_st().name_buff_cstr();
        if !namebuf_cstr.to_bytes().is_empty() {
            msg_outtrans(namebuf_cstr, 0, false);
        }
    }

    g_st().silent_mode.set(save_silent);
    g_st().info_message.set(false);
}

/// Write modified options as ":set" commands to a file.
pub fn makeset(fd: &crate::os::fs::CFile, opt_flags: i32, local_only: i32) -> bool {
    for pri in (0..=1).rev() {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);

            if (opt.flags & K_OPT_FLAG_NO_MKRC) == 0
                && ((pri == 1) == ((opt.flags & K_OPT_FLAG_PRI_MKRC) != 0))
            {
                if option_is_global_only(opt_idx) && (opt_flags & OPT_GLOBAL) == 0 {
                    continue;
                }

                if (opt_flags & OPT_GLOBAL) != 0 && (opt.flags & K_OPT_FLAG_NO_GLOB) != 0 {
                    continue;
                }

                let mut varp = get_varp_scope(opt.clone(), opt_flags);
                if varp.is_none() {
                    continue;
                }
                if (opt_flags & OPT_GLOBAL) != 0 && optval_default(opt_idx, varp) != 0 {
                    continue;
                }

                if (opt_flags & OPT_SKIPRTP) != 0
                    && (opt.var().ptr_eq(&opt_vars().p_rtp) || opt.var().ptr_eq(&opt_vars().p_pp))
                {
                    continue;
                }

                let mut round: i32 = 2;
                let mut varp_local = OptVar::None;
                if option_is_window_local(opt_idx) {
                    if (opt_flags & OPT_LOCAL) == 0 {
                        continue;
                    }
                    if (opt_flags & OPT_GLOBAL) == 0 && local_only == 0 {
                        let varp_fresh = get_varp_scope(opt.clone(), OPT_GLOBAL);
                        if optval_default(opt_idx, varp_fresh) == 0 {
                            round = 1;
                            varp_local = varp;
                            varp = varp_fresh;
                        }
                    }
                }

                while round <= 2 {
                    let cmd = if round == 1 || (opt_flags & OPT_GLOBAL) != 0 {
                        c"set"
                    } else {
                        c"setlocal"
                    };

                    let mut do_endif = false;
                    if opt_idx == K_OPT_SYNTAX || opt_idx == K_OPT_FILETYPE {
                        if fprintf!(fd, c"if &%s != '%s'", opt.fullname, varp.as_cstr(),) < 0
                            || !put_eol(fd)
                        {
                            return false;
                        }
                        do_endif = true;
                    }
                    if !put_set(fd, cmd, opt_idx, varp) {
                        return false;
                    }
                    if do_endif && !put_line(fd, c"endif") {
                        return false;
                    }

                    varp = std::mem::take(&mut varp_local);
                    round += 1;
                }
            }
        }
    }
    true
}

/// Generate set commands for the local fold options only.
pub fn makefoldset(fd: &crate::os::fs::CFile) -> bool {
    let cmd = c"setlocal";
    if !put_set(
        fd,
        cmd,
        K_OPT_FOLDMETHOD,
        win_onebuf_var!(curwin_mut(), wo_fdm),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDEXPR,
        win_onebuf_var!(curwin_mut(), wo_fde),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDMARKER,
        win_onebuf_var!(curwin_mut(), wo_fmr),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDIGNORE,
        win_onebuf_var!(curwin_mut(), wo_fdi),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDLEVEL,
        win_onebuf_var!(curwin_mut(), wo_fdl),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDMINLINES,
        win_onebuf_var!(curwin_mut(), wo_fml),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDNESTMAX,
        win_onebuf_var!(curwin_mut(), wo_fdn),
    ) || !put_set(
        fd,
        cmd,
        K_OPT_FOLDENABLE,
        win_onebuf_var!(curwin_mut(), wo_fen),
    ) {
        return false;
    }
    true
}

fn put_set_value(
    fd: &crate::os::fs::CFile,
    cmd: &std::ffi::CStr,
    opt_idx: OptIndex,
    varp: OptVar,
    value: Object,
) -> bool {
    let opt = get_option(opt_idx);
    let name = opt.fullname;
    let flags = opt.flags;

    if varp != opt.var() && optval_is_unset_local(opt_idx, value.clone()) {
        return true;
    }

    match value.obj_type {
        ObjectType::ObjectTypeBoolean => {
            let value_bool = value.data.boolean();
            if fprintf!(
                fd,
                c"%s %s%s",
                cmd,
                if value_bool { c"" } else { c"no" },
                name,
            ) < 0
            {
                return false;
            }
        }
        ObjectType::ObjectTypeInteger => {
            if fprintf!(fd, c"%s %s=", cmd, name) < 0 {
                return false;
            }
            let value_num = value.data.integer();
            let mut wc: OptInt = 0;
            if wc_use_keyname(varp, &mut wc) != 0 {
                if fd.puts(&get_special_key_name(wc as i32, 0)) < 0 {
                    return false;
                }
            } else {
                let num_s = format!("{value_num}\0");
                if fd.puts(crate::memory::bytes_as_cstr(num_s.as_bytes())) < 0 {
                    return false;
                }
            }
        }
        ObjectType::ObjectTypeString => {
            if fprintf!(fd, c"%s %s=", cmd, name) < 0 {
                return false;
            }
            let value_str = value.data.string();
            if !value_str.data.is_null() {
                if (flags & K_OPT_FLAG_EXPAND) != 0 {
                    let size = value_str.cstr_bytes().len() + 1;
                    let mut buf_vec = vec![0u8; size];
                    home_replace(None, value_str.as_opt_cstr(), &mut buf_vec, false);

                    if size >= MAXPATHL
                        && (flags & K_OPT_FLAG_COMMA) != 0
                        && crate::strings::vim_haschr(value_str.as_cstr(), b',' as i32)
                    {
                        let mut part_vec = vec![0u8; size];
                        if !put_eol(fd) {
                            return false;
                        }
                        let mut p = crate::memory::bytes_until_nul(&buf_vec);
                        while !p.is_empty() {
                            if fprintf!(fd, c"%s %s+=", cmd, name) < 0 {
                                return false;
                            }
                            copy_option_part(&mut p, &mut part_vec, c",");
                            if !put_escstr(fd, crate::memory::bytes_as_cstr(&part_vec), 2)
                                || !put_eol(fd)
                            {
                                return false;
                            }
                        }
                        return true;
                    }
                    if !put_escstr(fd, crate::memory::bytes_as_cstr(&buf_vec), 2) {
                        return false;
                    }
                } else if !put_escstr(fd, value_str.as_cstr(), 2) {
                    return false;
                }
            }
        }
        _ => std::process::abort(),
    }

    if !put_eol(fd) {
        return false;
    }
    true
}

fn put_set(
    fd: &crate::os::fs::CFile,
    cmd: &std::ffi::CStr,
    opt_idx: OptIndex,
    varp: OptVar,
) -> bool {
    let value = opt_from_varp(opt_idx, varp);
    let ret = if value.obj_type == ObjectType::ObjectTypeLuaRef {
        true
    } else {
        put_set_value(fd, cmd, opt_idx, varp, value.clone())
    };
    optval_free_read(opt_idx, value);
    ret
}

fn get_winopt_var(win: &WinInfoInfo, allbuf: bool, opt_idx: OptIndex) -> OptVar {
    match opt_idx {
        K_OPT_ARABIC => winopt_var!(win, allbuf, wo_arab),
        K_OPT_LIST => winopt_var!(win, allbuf, wo_list),
        K_OPT_SPELL => winopt_var!(win, allbuf, wo_spell),
        K_OPT_CURSORCOLUMN => winopt_var!(win, allbuf, wo_cuc),
        K_OPT_CURSORLINE => winopt_var!(win, allbuf, wo_cul),
        K_OPT_CURSORLINEOPT => winopt_var!(win, allbuf, wo_culopt),
        K_OPT_COLORCOLUMN => winopt_var!(win, allbuf, wo_cc),
        K_OPT_DIFF => winopt_var!(win, allbuf, wo_diff),
        K_OPT_EVENTIGNOREWIN => winopt_var!(win, allbuf, wo_eiw),
        K_OPT_FOLDCOLUMN => winopt_var!(win, allbuf, wo_fdc),
        K_OPT_FOLDENABLE => winopt_var!(win, allbuf, wo_fen),
        K_OPT_FOLDIGNORE => winopt_var!(win, allbuf, wo_fdi),
        K_OPT_FOLDLEVEL => winopt_var!(win, allbuf, wo_fdl),
        K_OPT_FOLDMETHOD => winopt_var!(win, allbuf, wo_fdm),
        K_OPT_FOLDMINLINES => winopt_var!(win, allbuf, wo_fml),
        K_OPT_FOLDNESTMAX => winopt_var!(win, allbuf, wo_fdn),
        K_OPT_FOLDEXPR => winopt_var!(win, allbuf, wo_fde),
        K_OPT_FOLDTEXT => winopt_var!(win, allbuf, wo_fdt),
        K_OPT_FOLDMARKER => winopt_var!(win, allbuf, wo_fmr),
        K_OPT_NUMBER => winopt_var!(win, allbuf, wo_nu),
        K_OPT_RELATIVENUMBER => winopt_var!(win, allbuf, wo_rnu),
        K_OPT_NUMBERWIDTH => winopt_var!(win, allbuf, wo_nuw),
        K_OPT_WINFIXBUF => winopt_var!(win, allbuf, wo_wfb),
        K_OPT_WINFIXHEIGHT => winopt_var!(win, allbuf, wo_wfh),
        K_OPT_WINFIXWIDTH => winopt_var!(win, allbuf, wo_wfw),
        K_OPT_WINPINNED => winopt_var!(win, allbuf, wo_wp),
        K_OPT_PREVIEWWINDOW => winopt_var!(win, allbuf, wo_pvw),
        K_OPT_LHISTORY => winopt_var!(win, allbuf, wo_lhi),
        K_OPT_RIGHTLEFT => winopt_var!(win, allbuf, wo_rl),
        K_OPT_RIGHTLEFTCMD => winopt_var!(win, allbuf, wo_rlc),
        K_OPT_SCROLL => winopt_var!(win, allbuf, wo_scr),
        K_OPT_SMOOTHSCROLL => winopt_var!(win, allbuf, wo_sms),
        K_OPT_WRAP => winopt_var!(win, allbuf, wo_wrap),
        K_OPT_LINEBREAK => winopt_var!(win, allbuf, wo_lbr),
        K_OPT_BREAKINDENT => winopt_var!(win, allbuf, wo_bri),
        K_OPT_BREAKINDENTOPT => winopt_var!(win, allbuf, wo_briopt),
        K_OPT_SCROLLBIND => winopt_var!(win, allbuf, wo_scb),
        K_OPT_CURSORBIND => winopt_var!(win, allbuf, wo_crb),
        K_OPT_CONCEALCURSOR => winopt_var!(win, allbuf, wo_cocu),
        K_OPT_CONCEALLEVEL => winopt_var!(win, allbuf, wo_cole),
        K_OPT_SIGNCOLUMN => winopt_var!(win, allbuf, wo_scl),
        K_OPT_WINHIGHLIGHT => winopt_var!(win, allbuf, wo_winhl),
        K_OPT_WINBLEND => winopt_var!(win, allbuf, wo_winbl),
        K_OPT_STATUSCOLUMN => winopt_var!(win, allbuf, wo_stc),
        _ => OptVar::None,
    }
}

pub fn get_varp_scope_from(
    p: Vimoption,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: Option<&mut WinInfoInfo>,
) -> OptVar {
    let opt_idx = get_opt_idx(p.clone());

    if (opt_flags & OPT_GLOBAL) != 0 && !option_is_global_only(opt_idx) {
        if option_is_window_local(opt_idx) {
            return get_winopt_var(win.unwrap(), true, opt_idx);
        }
        return p.var();
    }

    if (opt_flags & OPT_LOCAL) != 0 && option_is_global_local(opt_idx) {
        return match opt_idx {
            K_OPT_FORMATPRG => buf_var!(buf, b_p_fp),
            K_OPT_FSYNC => buf_var!(buf, b_p_fs),
            K_OPT_FINDFUNC => buf_var!(buf, b_p_ffu),
            K_OPT_ERRORFORMAT => buf_var!(buf, b_p_efm),
            K_OPT_GREPFORMAT => buf_var!(buf, b_p_gefm),
            K_OPT_GREPPRG => buf_var!(buf, b_p_gp),
            K_OPT_MAKEPRG => buf_var!(buf, b_p_mp),
            K_OPT_EQUALPRG => buf_var!(buf, b_p_ep),
            K_OPT_KEYWORDPRG => buf_var!(buf, b_p_kp),
            K_OPT_PATH => buf_var!(buf, b_p_path),
            K_OPT_AUTOCOMPLETE => buf_var!(buf, b_p_ac),
            K_OPT_AUTOREAD => buf_var!(buf, b_p_ar),
            K_OPT_TAGS => buf_var!(buf, b_p_tags),
            K_OPT_TAGCASE => buf_var!(buf, b_p_tc),
            K_OPT_SIDESCROLLOFF => win_onebuf_var!(win.as_deref().unwrap(), wo_siso),
            K_OPT_SCROLLOFF => win_onebuf_var!(win.as_deref().unwrap(), wo_so),
            K_OPT_SCROLLOFFPAD => win_onebuf_var!(win.as_deref().unwrap(), wo_sop),
            K_OPT_DEFINE => buf_var!(buf, b_p_def),
            K_OPT_INCLUDE => buf_var!(buf, b_p_inc),
            K_OPT_COMPLETEOPT => buf_var!(buf, b_p_cot),
            K_OPT_DICTIONARY => buf_var!(buf, b_p_dict),
            K_OPT_DIFFANCHORS => buf_var!(buf, b_p_dia),
            K_OPT_THESAURUS => buf_var!(buf, b_p_tsr),
            K_OPT_THESAURUSFUNC => buf_var!(buf, b_p_tsrfu),
            K_OPT_TAGFUNC => buf_var!(buf, b_p_tfu),
            K_OPT_SHOWBREAK => win_onebuf_var!(win.as_deref().unwrap(), wo_sbr),
            K_OPT_STATUSLINE => win_onebuf_var!(win.as_deref().unwrap(), wo_stl),
            K_OPT_WINBAR => win_onebuf_var!(win.as_deref().unwrap(), wo_wbr),
            K_OPT_UNDOLEVELS => buf_var!(buf, b_p_ul),
            K_OPT_LISPWORDS => buf_var!(buf, b_p_lw),
            K_OPT_BACKUPCOPY => buf_var!(buf, b_p_bkc),
            K_OPT_MAKEENCODING => buf_var!(buf, b_p_menc),
            K_OPT_FILLCHARS => win_onebuf_var!(win.as_deref().unwrap(), wo_fcs),
            K_OPT_LISTCHARS => win_onebuf_var!(win.as_deref().unwrap(), wo_lcs),
            K_OPT_VIRTUALEDIT => win_onebuf_var!(win.as_deref().unwrap(), wo_ve),
            _ => std::process::abort(),
        };
    }

    get_varp_from(p, buf, win)
}

/// Get pointer to option variable, depending on local or global scope.
pub fn get_varp_scope(p: Vimoption, opt_flags: i32) -> OptVar {
    get_varp_scope_from(p, opt_flags, curbuf_mut(), Some(curwin_mut()))
}

/// Get pointer to option variable at 'opt_idx', depending on local or global scope.
pub fn get_option_varp_scope_from(
    opt_idx: OptIndex,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: Option<&mut WinInfoInfo>,
) -> OptVar {
    get_varp_scope_from(get_option(opt_idx), opt_flags, buf, win)
}

pub fn get_varp_from(p: Vimoption, buf: &mut FileBuf, mut win: Option<&mut WinInfoInfo>) -> OptVar {
    let opt_idx = get_opt_idx(p.clone());

    if is_option_hidden(opt_idx) || option_is_global_only(opt_idx) {
        return p.var();
    }

    if let Some(ref w) = win {
        let win_var = get_winopt_var(w, false, opt_idx);
        if win_var.is_some() {
            return win_var;
        }
    }

    match opt_idx {
        K_OPT_EQUALPRG => {
            if !buf.b_p_ep.is_empty() {
                buf_var!(buf, b_p_ep)
            } else {
                p.var()
            }
        }
        K_OPT_KEYWORDPRG => {
            if !buf.b_p_kp.is_empty() {
                buf_var!(buf, b_p_kp)
            } else {
                p.var()
            }
        }
        K_OPT_PATH => {
            if !buf.b_p_path.is_empty() {
                buf_var!(buf, b_p_path)
            } else {
                p.var()
            }
        }
        K_OPT_AUTOCOMPLETE => {
            if buf.b_p_ac.get().is_some() {
                buf_var!(buf, b_p_ac)
            } else {
                p.var()
            }
        }
        K_OPT_AUTOREAD => {
            if buf.b_p_ar.get().is_some() {
                buf_var!(buf, b_p_ar)
            } else {
                p.var()
            }
        }
        K_OPT_TAGS => {
            if !buf.b_p_tags.is_empty() {
                buf_var!(buf, b_p_tags)
            } else {
                p.var()
            }
        }
        K_OPT_TAGCASE => {
            if !buf.b_p_tc.is_empty() {
                buf_var!(buf, b_p_tc)
            } else {
                p.var()
            }
        }
        K_OPT_SIDESCROLLOFF => {
            let w = win.as_deref().unwrap();
            if w.w_onebuf_opt.wo_siso.get() >= 0 {
                win_onebuf_var!(w, wo_siso)
            } else {
                p.var()
            }
        }
        K_OPT_SCROLLOFF => {
            let w = win.as_deref().unwrap();
            if w.w_onebuf_opt.wo_so.get() >= 0 {
                win_onebuf_var!(w, wo_so)
            } else {
                p.var()
            }
        }
        K_OPT_SCROLLOFFPAD => {
            let w = win.as_deref().unwrap();
            if w.w_onebuf_opt.wo_sop.get() >= 0 {
                win_onebuf_var!(w, wo_sop)
            } else {
                p.var()
            }
        }
        K_OPT_BACKUPCOPY => {
            if !buf.b_p_bkc.is_empty() {
                buf_var!(buf, b_p_bkc)
            } else {
                p.var()
            }
        }
        K_OPT_DEFINE => {
            if !buf.b_p_def.is_empty() {
                buf_var!(buf, b_p_def)
            } else {
                p.var()
            }
        }
        K_OPT_INCLUDE => {
            if !buf.b_p_inc.is_empty() {
                buf_var!(buf, b_p_inc)
            } else {
                p.var()
            }
        }
        K_OPT_COMPLETEOPT => {
            if !buf.b_p_cot.is_empty() {
                buf_var!(buf, b_p_cot)
            } else {
                p.var()
            }
        }
        K_OPT_DICTIONARY => {
            if !buf.b_p_dict.is_empty() {
                buf_var!(buf, b_p_dict)
            } else {
                p.var()
            }
        }
        K_OPT_DIFFANCHORS => {
            if !buf.b_p_dia.is_empty() {
                buf_var!(buf, b_p_dia)
            } else {
                p.var()
            }
        }
        K_OPT_THESAURUS => {
            if !buf.b_p_tsr.is_empty() {
                buf_var!(buf, b_p_tsr)
            } else {
                p.var()
            }
        }
        K_OPT_THESAURUSFUNC => {
            if buf.b_p_tsrfu.cb_type() != K_CALLBACK_NONE_CONST {
                buf_var!(buf, b_p_tsrfu)
            } else {
                p.var()
            }
        }
        K_OPT_FORMATPRG => {
            if !buf.b_p_fp.is_empty() {
                buf_var!(buf, b_p_fp)
            } else {
                p.var()
            }
        }
        K_OPT_FSYNC => {
            if buf.b_p_fs.get().is_some() {
                buf_var!(buf, b_p_fs)
            } else {
                p.var()
            }
        }
        K_OPT_FINDFUNC => {
            if buf.b_p_ffu.cb_type() != K_CALLBACK_NONE_CONST {
                buf_var!(buf, b_p_ffu)
            } else {
                p.var()
            }
        }
        K_OPT_ERRORFORMAT => {
            if !buf.b_p_efm.is_empty() {
                buf_var!(buf, b_p_efm)
            } else {
                p.var()
            }
        }
        K_OPT_GREPFORMAT => {
            if !buf.b_p_gefm.is_empty() {
                buf_var!(buf, b_p_gefm)
            } else {
                p.var()
            }
        }
        K_OPT_GREPPRG => {
            if !buf.b_p_gp.is_empty() {
                buf_var!(buf, b_p_gp)
            } else {
                p.var()
            }
        }
        K_OPT_MAKEPRG => {
            if !buf.b_p_mp.is_empty() {
                buf_var!(buf, b_p_mp)
            } else {
                p.var()
            }
        }
        K_OPT_SHOWBREAK => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_sbr.is_empty() {
                win_onebuf_var!(w, wo_sbr)
            } else {
                p.var()
            }
        }
        K_OPT_STATUSLINE => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_stl.is_empty() {
                win_onebuf_var!(w, wo_stl)
            } else {
                p.var()
            }
        }
        K_OPT_WINBAR => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_wbr.is_empty() {
                win_onebuf_var!(w, wo_wbr)
            } else {
                p.var()
            }
        }
        K_OPT_UNDOLEVELS => {
            if buf.b_p_ul.get() != NO_LOCAL_UNDOLEVEL {
                buf_var!(buf, b_p_ul)
            } else {
                p.var()
            }
        }
        K_OPT_LISPWORDS => {
            if !buf.b_p_lw.is_empty() {
                buf_var!(buf, b_p_lw)
            } else {
                p.var()
            }
        }
        K_OPT_MAKEENCODING => {
            if !buf.b_p_menc.is_empty() {
                buf_var!(buf, b_p_menc)
            } else {
                p.var()
            }
        }
        K_OPT_FILLCHARS => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_fcs.is_empty() {
                win_onebuf_var!(w, wo_fcs)
            } else {
                p.var()
            }
        }
        K_OPT_LISTCHARS => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_lcs.is_empty() {
                win_onebuf_var!(w, wo_lcs)
            } else {
                p.var()
            }
        }
        K_OPT_VIRTUALEDIT => {
            let w = win.as_deref().unwrap();
            if !w.w_onebuf_opt.wo_ve.is_empty() {
                win_onebuf_var!(w, wo_ve)
            } else {
                p.var()
            }
        }

        K_OPT_AUTOINDENT => buf_var!(buf, b_p_ai),
        K_OPT_BINARY => buf_var!(buf, b_p_bin),
        K_OPT_BOMB => buf_var!(buf, b_p_bomb),
        K_OPT_BUFHIDDEN => buf_var!(buf, b_p_bh),
        K_OPT_BUFTYPE => buf_var!(buf, b_p_bt),
        K_OPT_BUFLISTED => buf_var!(buf, b_p_bl),
        K_OPT_BUSY => buf_var!(buf, b_p_busy),
        K_OPT_CHANNEL => buf_var!(buf, b_p_channel),
        K_OPT_COPYINDENT => buf_var!(buf, b_p_ci),
        K_OPT_CINDENT => buf_var!(buf, b_p_cin),
        K_OPT_CINKEYS => buf_var!(buf, b_p_cink),
        K_OPT_CINOPTIONS => buf_var!(buf, b_p_cino),
        K_OPT_CINSCOPEDECLS => buf_var!(buf, b_p_cinsd),
        K_OPT_CINWORDS => buf_var!(buf, b_p_cinw),
        K_OPT_COMMENTS => buf_var!(buf, b_p_com),
        K_OPT_COMMENTSTRING => buf_var!(buf, b_p_cms),
        K_OPT_COMPLETE => buf_var!(buf, b_p_cpt),
        #[cfg(windows)]
        K_OPT_COMPLETESLASH => buf_var!(buf, b_p_csl),
        K_OPT_COMPLETEFUNC => buf_var!(buf, b_p_cfu),
        K_OPT_OMNIFUNC => buf_var!(buf, b_p_ofu),
        K_OPT_ENDOFFILE => buf_var!(buf, b_p_eof),
        K_OPT_ENDOFLINE => buf_var!(buf, b_p_eol),
        K_OPT_FIXENDOFLINE => buf_var!(buf, b_p_fixeol),
        K_OPT_EXPANDTAB => buf_var!(buf, b_p_et),
        K_OPT_FILEENCODING => buf_var!(buf, b_p_fenc),
        K_OPT_FILEFORMAT => buf_var!(buf, b_p_ff),
        K_OPT_FILETYPE => buf_var!(buf, b_p_ft),
        K_OPT_FOLLOW => buf_var!(buf, b_p_follow),
        K_OPT_FORMATOPTIONS => buf_var!(buf, b_p_fo),
        K_OPT_FORMATLISTPAT => buf_var!(buf, b_p_flp),
        K_OPT_IMINSERT => buf_var!(buf, b_p_iminsert),
        K_OPT_IMSEARCH => buf_var!(buf, b_p_imsearch),
        K_OPT_INFERCASE => buf_var!(buf, b_p_inf),
        K_OPT_ISKEYWORD => buf_var!(buf, b_p_isk),
        K_OPT_INCLUDEEXPR => buf_var!(buf, b_p_inex),
        K_OPT_INDENTEXPR => buf_var!(buf, b_p_inde),
        K_OPT_INDENTKEYS => buf_var!(buf, b_p_indk),
        K_OPT_FORMATEXPR => buf_var!(buf, b_p_fex),
        K_OPT_LISP => buf_var!(buf, b_p_lisp),
        K_OPT_LISPOPTIONS => buf_var!(buf, b_p_lop),
        K_OPT_MODELINE => buf_var!(buf, b_p_ml),
        K_OPT_MATCHPAIRS => buf_var!(buf, b_p_mps),
        K_OPT_MODIFIABLE => buf_var!(buf, b_p_ma),
        K_OPT_MODIFIED => buf_var!(buf, b_changed),
        K_OPT_NRFORMATS => buf_var!(buf, b_p_nf),
        K_OPT_PRESERVEINDENT => buf_var!(buf, b_p_pi),
        K_OPT_QUOTEESCAPE => buf_var!(buf, b_p_qe),
        K_OPT_READONLY => buf_var!(buf, b_p_ro),
        K_OPT_SCROLLBACK => buf_var!(buf, b_p_scbk),
        K_OPT_SMARTINDENT => buf_var!(buf, b_p_si),
        K_OPT_SOFTTABSTOP => buf_var!(buf, b_p_sts),
        K_OPT_SUFFIXESADD => buf_var!(buf, b_p_sua),
        K_OPT_SWAPFILE => buf_var!(buf, b_p_swf),
        K_OPT_SYNMAXCOL => buf_var!(buf, b_p_smc),
        K_OPT_SYNTAX => buf_var!(buf, b_p_syn),
        K_OPT_SPELLCAPCHECK => {
            if let Some(ref mut w) = win {
                OptVar::from(crate::optionstr::OptRef::Win(
                    crate::globals::WinPtr::from(&**w),
                    |w| &w.w_s_ptr().b_p_spc,
                ))
            } else {
                OptVar::from(crate::optionstr::OptRef::Buf(
                    crate::globals::BufPtr::from(&*buf),
                    |b| &b.b_s.b_p_spc,
                ))
            }
        }
        K_OPT_SPELLFILE => {
            if let Some(ref mut w) = win {
                OptVar::from(crate::optionstr::OptRef::Win(
                    crate::globals::WinPtr::from(&**w),
                    |w| &w.w_s_ptr().b_p_spf,
                ))
            } else {
                OptVar::from(crate::optionstr::OptRef::Buf(
                    crate::globals::BufPtr::from(&*buf),
                    |b| &b.b_s.b_p_spf,
                ))
            }
        }
        K_OPT_SPELLLANG => {
            if let Some(ref mut w) = win {
                OptVar::from(crate::optionstr::OptRef::Win(
                    crate::globals::WinPtr::from(&**w),
                    |w| &w.w_s_ptr().b_p_spl,
                ))
            } else {
                OptVar::from(crate::optionstr::OptRef::Buf(
                    crate::globals::BufPtr::from(&*buf),
                    |b| &b.b_s.b_p_spl,
                ))
            }
        }
        K_OPT_SPELLOPTIONS => {
            if let Some(ref mut w) = win {
                OptVar::from(crate::optionstr::OptRef::Win(
                    crate::globals::WinPtr::from(&**w),
                    |w| &w.w_s_ptr().b_p_spo,
                ))
            } else {
                OptVar::from(crate::optionstr::OptRef::Buf(
                    crate::globals::BufPtr::from(&*buf),
                    |b| &b.b_s.b_p_spo,
                ))
            }
        }
        K_OPT_SHIFTWIDTH => buf_var!(buf, b_p_sw),
        K_OPT_TAGFUNC => buf_var!(buf, b_p_tfu),
        K_OPT_TABSTOP => buf_var!(buf, b_p_ts),
        K_OPT_TEXTWIDTH => buf_var!(buf, b_p_tw),
        K_OPT_UNDOFILE => buf_var!(buf, b_p_udf),
        K_OPT_WRAPMARGIN => buf_var!(buf, b_p_wm),
        K_OPT_VARSOFTTABSTOP => buf_var!(buf, b_p_vsts),
        K_OPT_VARTABSTOP => buf_var!(buf, b_p_vts),
        K_OPT_KEYMAP => buf_var!(buf, b_p_keymap),
        _ => {
            iemsg(tr(c"E356: get_varp ERROR"));
            buf_var!(buf, b_p_wm)
        }
    }
}

/// Get the value of 'equalprg', either the buffer-local one or the global one.
pub fn get_equalprg() -> std::ffi::CString {
    if curbuf_mut().b_p_ep.is_empty() {
        opt_vars().p_ep.to_owned()
    } else {
        curbuf_mut().b_p_ep.to_owned()
    }
}

/// Get the value of 'findfunc', either the buffer-local one or the global one.
pub fn get_findfunc() -> &'static Callback {
    if curbuf_mut().b_p_ffu.cb_type() != K_CALLBACK_NONE_CONST {
        &curbuf_mut().b_p_ffu
    } else {
        &opt_vars().p_ffu
    }
}

/// Copy options from one window to another.
pub fn win_copy_options(wp_from: &mut WinInfoInfo, wp_to: &mut WinInfoInfo) {
    copy_winopt(&wp_from.w_onebuf_opt, &wp_to.w_onebuf_opt);
    copy_winopt(&wp_from.w_allbuf_opt, &wp_to.w_allbuf_opt);
    didset_window_options(&mut *wp_to, true);
}

/// Copy the options from one Winopt to another.
pub fn copy_winopt(from: &Winopt, to: &Winopt) {
    to.wo_arab.set(from.wo_arab.get());
    to.wo_list.set(from.wo_list.get());
    to.wo_lcs.set(from.wo_lcs.clone());
    to.wo_fcs.set(from.wo_fcs.clone());
    to.wo_nu.set(from.wo_nu.get());
    to.wo_rnu.set(from.wo_rnu.get());
    to.wo_ve.set(from.wo_ve.clone());
    to.wo_ve_flags.set(from.wo_ve_flags.get());
    to.wo_nuw.set(from.wo_nuw.get());
    to.wo_rl.set(from.wo_rl.get());
    to.wo_rlc.set(from.wo_rlc.clone());
    to.wo_sbr.set(from.wo_sbr.clone());
    to.wo_stl.set(from.wo_stl.clone());
    to.wo_wbr.set(from.wo_wbr.clone());
    to.wo_wrap.set(from.wo_wrap.get());
    to.wo_wrap_save.set(from.wo_wrap_save.get());
    to.wo_lbr.set(from.wo_lbr.get());
    to.wo_bri.set(from.wo_bri.get());
    to.wo_briopt.set(from.wo_briopt.clone());
    to.wo_scb.set(from.wo_scb.get());
    to.wo_scb_save.set(from.wo_scb_save.get());
    to.wo_sms.set(from.wo_sms.get());
    to.wo_crb.set(from.wo_crb.get());
    to.wo_crb_save.set(from.wo_crb_save.get());
    to.wo_siso.set(from.wo_siso.get());
    to.wo_so.set(from.wo_so.get());
    to.wo_sop.set(from.wo_sop.get());
    to.wo_spell.set(from.wo_spell.get());
    to.wo_cuc.set(from.wo_cuc.get());
    to.wo_cul.set(from.wo_cul.get());
    to.wo_culopt.set(from.wo_culopt.clone());
    to.wo_cc.set(from.wo_cc.clone());
    to.wo_diff.set(from.wo_diff.get());
    to.wo_diff_saved.set(from.wo_diff_saved.get());
    to.wo_eiw.set(from.wo_eiw.clone());
    to.wo_cocu.set(from.wo_cocu.clone());
    to.wo_cole.set(from.wo_cole.get());
    to.wo_fdc.set(from.wo_fdc.clone());
    to.wo_fdc_save.set(if from.wo_diff_saved.get() {
        from.wo_fdc_save.clone()
    } else {
        OptString::new()
    });
    to.wo_fen.set(from.wo_fen.get());
    to.wo_fen_save.set(from.wo_fen_save.get());
    to.wo_fdi.set(from.wo_fdi.clone());
    to.wo_fml.set(from.wo_fml.get());
    to.wo_fdl.set(from.wo_fdl.get());
    to.wo_fdl_save.set(from.wo_fdl_save.get());
    to.wo_fdm.set(from.wo_fdm.clone());
    to.wo_fdm_save.set(if from.wo_diff_saved.get() {
        from.wo_fdm_save.clone()
    } else {
        OptString::new()
    });
    to.wo_fdn.set(from.wo_fdn.get());
    callback_copy(&to.wo_fde, &from.wo_fde);
    callback_copy(&to.wo_fdt, &from.wo_fdt);
    to.wo_fmr.set(from.wo_fmr.clone());
    to.wo_scl.set(from.wo_scl.clone());
    to.wo_lhi.set(from.wo_lhi.get());
    to.wo_winhl.set(from.wo_winhl.clone());
    to.wo_winbl.set(from.wo_winbl.get());
    to.wo_stc.set(from.wo_stc.clone());

    to.wo_wrap_flags.set(from.wo_wrap_flags.get());
    to.wo_stl_flags.set(from.wo_stl_flags.get());
    to.wo_wbr_flags.set(from.wo_wbr_flags.get());
    to.wo_fde_flags.set(from.wo_fde_flags.get());
    to.wo_fdt_flags.set(from.wo_fdt_flags.get());

    for (dst, src) in to.wo_script_ctx.iter().zip(from.wo_script_ctx.iter()) {
        dst.set(src.get());
    }
    check_winopt(to);
}

fn check_win_options(win: &WinInfoInfo) {
    check_winopt(&win.w_onebuf_opt);
    check_winopt(&win.w_allbuf_opt);
}

fn check_winopt(wop: &Winopt) {
    wop.wo_fdc.check_option_str();
    wop.wo_fdc_save.check_option_str();
    wop.wo_fdi.check_option_str();
    wop.wo_fdm.check_option_str();
    wop.wo_fdm_save.check_option_str();
    wop.wo_fmr.check_option_str();
    wop.wo_eiw.check_option_str();
    wop.wo_scl.check_option_str();
    wop.wo_rlc.check_option_str();
    wop.wo_sbr.check_option_str();
    wop.wo_stl.check_option_str();
    wop.wo_culopt.check_option_str();
    wop.wo_cc.check_option_str();
    wop.wo_cocu.check_option_str();
    wop.wo_briopt.check_option_str();
    wop.wo_winhl.check_option_str();
    wop.wo_lcs.check_option_str();
    wop.wo_fcs.check_option_str();
    wop.wo_ve.check_option_str();
    wop.wo_wbr.check_option_str();
    wop.wo_stc.check_option_str();
}

/// Free the allocated memory inside a Winopt.
pub fn clear_winopt(wop: &Winopt) {
    wop.wo_fdc.clear_option_str();
    wop.wo_fdc_save.clear_option_str();
    wop.wo_fdi.clear_option_str();
    wop.wo_fdm.clear_option_str();
    wop.wo_fdm_save.clear_option_str();
    callback_free(&wop.wo_fde);
    callback_free(&wop.wo_fdt);
    wop.wo_fmr.clear_option_str();
    wop.wo_eiw.clear_option_str();
    wop.wo_scl.clear_option_str();
    wop.wo_rlc.clear_option_str();
    wop.wo_sbr.clear_option_str();
    wop.wo_stl.clear_option_str();
    wop.wo_culopt.clear_option_str();
    wop.wo_cc.clear_option_str();
    wop.wo_cocu.clear_option_str();
    wop.wo_briopt.clear_option_str();
    wop.wo_winhl.clear_option_str();
    wop.wo_lcs.clear_option_str();
    wop.wo_fcs.clear_option_str();
    wop.wo_ve.clear_option_str();
    wop.wo_wbr.clear_option_str();
    wop.wo_stc.clear_option_str();
}

pub fn didset_window_options(wp: &mut WinInfoInfo, valid_cursor: bool) {
    if wp.w_onebuf_opt.wo_wrap.get() {
        wp.w_leftcol.set(0);
    } else {
        wp.w_skipcol.set(0);
    }
    check_colorcolumn(None::<&CStr>, Some(&mut *wp));
    briopt_check(Some(&mut *wp));
    let mut culopt_flags: u32 = 0;
    if fill_culopt_flags(wp.w_onebuf_opt.wo_culopt.as_bytes(), &mut culopt_flags) {
        wp.w_p_culopt_flags.set(culopt_flags as u8);
    }
    let wo_fcs = wp.w_onebuf_opt.wo_fcs.to_owned();
    set_chars_option(wp, &wo_fcs, CharsOption::Fillchars, true, None);
    let wo_lcs = wp.w_onebuf_opt.wo_lcs.to_owned();
    set_chars_option(wp, &wo_lcs, CharsOption::Listchars, true, None);
    parse_winhl_opt(None, Some(&mut *wp));
    check_blending(wp);
    set_winbar_win(wp, false, valid_cursor);
    check_signcolumn(None, Some(&mut *wp));
    wp.w_grid_alloc
        .blending
        .set(wp.w_onebuf_opt.wo_winbl.get() > 0);
}

#[inline]
fn copy_opt_sctx(buf: &FileBuf, bv: usize) {
    buf.b_p_script_ctx[bv].set(get_option(BUF_OPT_IDX[bv]).script_ctx);
}

#[inline]
fn copy_opt_insecure(bv: usize) -> u32 {
    get_option(BUF_OPT_IDX[bv]).flags & K_OPT_FLAG_INSECURE
}

/// Copy global option values to local options for one buffer.
pub fn buf_copy_options(buf: &mut FileBuf, flags: i32) {
    let mut should_copy = true;
    let mut save_p_isk = OptString::new();
    let mut did_isk = false;
    let empty_opt = OptString::new();

    if !opt_vars().p_cpo.is_null() {
        if (!crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_BUFOPTGLOB)
            || (flags & BCO_ENTER) == 0)
            && (buf.b_p_initialized.get()
                || ((flags & BCO_ENTER) == 0
                    && crate::strings::vim_haschr(opt_vars().p_cpo.as_cstr(), K_CPO_BUFOPT)))
        {
            should_copy = false;
        }

        if should_copy || (flags & BCO_ALWAYS) != 0 {
            for slot in &buf.b_p_script_ctx {
                slot.set(Sctx {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                });
            }
            let dont_do_help =
                ((flags & BCO_NOHELP) != 0 && buf.b_help.get()) || buf.b_p_initialized.get();
            if dont_do_help {
                save_p_isk = buf.b_p_isk.take();
            }
            if !buf.b_p_initialized.get() {
                free_buf_options(buf, true);
                buf.b_p_ro.set(false);
                buf.b_p_fenc.set(opt_vars().p_fenc.clone());
                match opt_vars().p_ffs.first_byte() {
                    b'm' => buf.b_p_ff.set_option_cstr(c"mac"),
                    b'd' => buf.b_p_ff.set_option_cstr(c"dos"),
                    b'u' => buf.b_p_ff.set_option_cstr(c"unix"),
                    _ => buf.b_p_ff.set(opt_vars().p_ff.clone()),
                }
                buf.b_p_bh.set(empty_opt.clone());
                buf.b_p_bt.set(empty_opt.clone());
            } else {
                free_buf_options(buf, false);
            }

            buf.b_p_ai.set(opt_vars().p_ai.get());
            copy_opt_sctx(buf, K_BUF_OPT_AUTOINDENT);
            buf.b_p_ai_nopaste
                .set(OPT_STATE.with(|s| s.p_ai_nopaste.get()));
            buf.b_p_sw.set(opt_vars().p_sw.get());
            copy_opt_sctx(buf, K_BUF_OPT_SHIFTWIDTH);
            buf.b_p_scbk.set(opt_vars().p_scbk.get());
            copy_opt_sctx(buf, K_BUF_OPT_SCROLLBACK);
            buf.b_p_tw.set(opt_vars().p_tw.get());
            copy_opt_sctx(buf, K_BUF_OPT_TEXTWIDTH);
            buf.b_p_tw_nopaste
                .set(OPT_STATE.with(|s| s.p_tw_nopaste.get()));
            buf.b_p_tw_nobin.set(OPT_STATE.with(|s| s.p_tw_nobin.get()));
            buf.b_p_wm.set(opt_vars().p_wm.get());
            copy_opt_sctx(buf, K_BUF_OPT_WRAPMARGIN);
            buf.b_p_wm_nopaste
                .set(OPT_STATE.with(|s| s.p_wm_nopaste.get()));
            buf.b_p_wm_nobin.set(OPT_STATE.with(|s| s.p_wm_nobin.get()));
            buf.b_p_bin.set(opt_vars().p_bin.get());
            copy_opt_sctx(buf, K_BUF_OPT_BINARY);
            buf.b_p_bomb.set(opt_vars().p_bomb.get());
            copy_opt_sctx(buf, K_BUF_OPT_BOMB);
            buf.b_p_et.set(opt_vars().p_et.get());
            copy_opt_sctx(buf, K_BUF_OPT_EXPANDTAB);
            buf.b_p_fixeol.set(opt_vars().p_fixeol.get());
            copy_opt_sctx(buf, K_BUF_OPT_FIXENDOFLINE);
            buf.b_p_follow.set(opt_vars().p_follow.get());
            copy_opt_sctx(buf, K_BUF_OPT_FOLLOW);
            buf.b_p_et_nobin.set(OPT_STATE.with(|s| s.p_et_nobin.get()));
            buf.b_p_et_nopaste
                .set(OPT_STATE.with(|s| s.p_et_nopaste.get()));
            buf.b_p_ml.set(opt_vars().p_ml.get());
            copy_opt_sctx(buf, K_BUF_OPT_MODELINE);
            buf.b_p_ml_nobin.set(OPT_STATE.with(|s| s.p_ml_nobin.get()));
            buf.b_p_inf.set(opt_vars().p_inf.get());
            copy_opt_sctx(buf, K_BUF_OPT_INFERCASE);
            if (g_st().cmdmod.borrow().cmod_flags & CMOD_NOSWAPFILE) != 0 {
                buf.b_p_swf.set(false);
            } else {
                buf.b_p_swf.set(opt_vars().p_swf.get());
                copy_opt_sctx(buf, K_BUF_OPT_SWAPFILE);
            }
            buf.b_p_cpt.set_option_cstr(opt_vars().p_cpt.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_COMPLETE);
            buf.b_p_cpt_flags.set(copy_opt_insecure(K_BUF_OPT_COMPLETE));
            set_buflocal_cpt_callbacks(Some(&mut *buf));
            #[cfg(windows)]
            {
                buf.b_p_csl.set(opt_vars().p_csl.clone());
                copy_opt_sctx(buf, K_BUF_OPT_COMPLETESLASH);
            }
            callback_copy(&buf.b_p_cfu, &opt_vars().p_cfu);
            copy_opt_sctx(buf, K_BUF_OPT_COMPLETEFUNC);
            callback_copy(&buf.b_p_ofu, &opt_vars().p_ofu);
            copy_opt_sctx(buf, K_BUF_OPT_OMNIFUNC);
            callback_copy(&buf.b_p_tfu, &opt_vars().p_tfu);
            copy_opt_sctx(buf, K_BUF_OPT_TAGFUNC);
            buf.b_p_sts.set(opt_vars().p_sts.get());
            copy_opt_sctx(buf, K_BUF_OPT_SOFTTABSTOP);
            buf.b_p_sts_nopaste
                .set(OPT_STATE.with(|s| s.p_sts_nopaste.get()));
            buf.b_p_vsts.set_option_cstr(opt_vars().p_vsts.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_VARSOFTTABSTOP);
            if !opt_vars().p_vsts.is_empty() {
                tabstop_set(opt_vars().p_vsts.as_bytes(), &mut buf.b_p_vsts_array, None);
            } else {
                buf.b_p_vsts_array.clear();
            }
            buf.b_p_vsts_nopaste
                .set(P_VSTS_NOPASTE.with(|cell| cell.borrow().clone()));
            buf.b_p_com.set_option_cstr(opt_vars().p_com.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_COMMENTS);
            buf.b_p_cms.set_option_cstr(opt_vars().p_cms.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_COMMENTSTRING);
            buf.b_p_fo.set_option_cstr(opt_vars().p_fo.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_FORMATOPTIONS);
            buf.b_p_flp.set_option_cstr(opt_vars().p_flp.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_FORMATLISTPAT);
            buf.b_p_nf.set_option_cstr(opt_vars().p_nf.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_NRFORMATS);
            buf.b_p_mps.set_option_cstr(opt_vars().p_mps.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_MATCHPAIRS);
            buf.b_p_si.set(opt_vars().p_si.get());
            copy_opt_sctx(buf, K_BUF_OPT_SMARTINDENT);
            buf.b_p_channel.set(0);
            buf.b_p_ci.set(opt_vars().p_ci.get());
            copy_opt_sctx(buf, K_BUF_OPT_COPYINDENT);
            buf.b_p_cin.set(opt_vars().p_cin.get());
            copy_opt_sctx(buf, K_BUF_OPT_CINDENT);
            buf.b_p_cink.set_option_cstr(opt_vars().p_cink.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_CINKEYS);
            buf.b_p_cino.set_option_cstr(opt_vars().p_cino.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_CINOPTIONS);
            buf.b_p_cinsd.set_option_cstr(opt_vars().p_cinsd.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_CINSCOPEDECLS);
            buf.b_p_lop.set_option_cstr(opt_vars().p_lop.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_LISPOPTIONS);

            buf.b_p_ft.set(empty_opt.clone());
            buf.b_p_pi.set(opt_vars().p_pi.get());
            copy_opt_sctx(buf, K_BUF_OPT_PRESERVEINDENT);
            buf.b_p_cinw.set_option_cstr(opt_vars().p_cinw.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_CINWORDS);
            buf.b_p_lisp.set(opt_vars().p_lisp.get());
            copy_opt_sctx(buf, K_BUF_OPT_LISP);
            buf.b_p_syn.set(empty_opt.clone());
            buf.b_p_smc.set(opt_vars().p_smc.get());
            copy_opt_sctx(buf, K_BUF_OPT_SYNMAXCOL);
            buf.b_s.b_syn_isk = None;
            buf.b_s.b_p_spc.set_option_cstr(opt_vars().p_spc.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_SPELLCAPCHECK);
            compile_cap_prog(buf.b_s.b_p_spc.as_cstr(), &mut buf.b_s.b_cap_prog);
            buf.b_s.b_p_spf.set_option_cstr(opt_vars().p_spf.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_SPELLFILE);
            buf.b_s.b_p_spl.set_option_cstr(opt_vars().p_spl.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_SPELLLANG);
            buf.b_s.b_p_spo.set_option_cstr(opt_vars().p_spo.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_SPELLOPTIONS);
            buf.b_s.b_p_spo_flags = opt_vars().spo_flags.get();
            callback_copy(&buf.b_p_inde, &opt_vars().p_inde);
            copy_opt_sctx(buf, K_BUF_OPT_INDENTEXPR);
            buf.b_p_inde_flags
                .set(copy_opt_insecure(K_BUF_OPT_INDENTEXPR));
            buf.b_p_indk.set_option_cstr(opt_vars().p_indk.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_INDENTKEYS);
            buf.b_p_fp.set(empty_opt.clone());
            callback_copy(&buf.b_p_fex, &opt_vars().p_fex);
            copy_opt_sctx(buf, K_BUF_OPT_FORMATEXPR);
            buf.b_p_fex_flags
                .set(copy_opt_insecure(K_BUF_OPT_FORMATEXPR));
            buf.b_p_sua.set_option_cstr(opt_vars().p_sua.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_SUFFIXESADD);
            buf.b_p_keymap
                .set_option_cstr(opt_vars().p_keymap.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_KEYMAP);
            buf.b_kmap_state.set(buf.b_kmap_state.get() | (KEYMAP_INIT));
            buf.b_p_iminsert.set(opt_vars().p_iminsert.get());
            copy_opt_sctx(buf, K_BUF_OPT_IMINSERT);
            buf.b_p_imsearch.set(opt_vars().p_imsearch.get());
            copy_opt_sctx(buf, K_BUF_OPT_IMSEARCH);

            buf.b_p_ac.set(None);
            buf.b_p_ar.set(None);
            buf.b_p_fs.set(None);
            buf.b_p_ul.set(NO_LOCAL_UNDOLEVEL);
            buf.b_p_bkc.set(empty_opt.clone());
            buf.b_bkc_flags.set(0);
            buf.b_p_gefm.set(empty_opt.clone());
            buf.b_p_gp.set(empty_opt.clone());
            buf.b_p_mp.set(empty_opt.clone());
            buf.b_p_efm.set(empty_opt.clone());
            buf.b_p_ep.set(empty_opt.clone());
            buf.b_p_ffu
                .data
                .set(crate::eval::typval::CallbackData::NONE);
            buf.b_p_kp.set(empty_opt.clone());
            buf.b_p_path.set(empty_opt.clone());
            buf.b_p_tags.set(empty_opt.clone());
            buf.b_p_tc.set(empty_opt.clone());
            buf.b_tc_flags.set(0);
            buf.b_p_def.set(empty_opt.clone());
            buf.b_p_inc.set(empty_opt.clone());
            callback_copy(&buf.b_p_inex, &opt_vars().p_inex);
            copy_opt_sctx(buf, K_BUF_OPT_INCLUDEEXPR);
            buf.b_p_inex_flags
                .set(copy_opt_insecure(K_BUF_OPT_INCLUDEEXPR));
            buf.b_p_cot.set(empty_opt.clone());
            buf.b_cot_flags.set(0);
            buf.b_p_dict.set(empty_opt.clone());
            buf.b_p_dia.set(empty_opt.clone());
            buf.b_p_tsr.set(empty_opt.clone());
            buf.b_p_tsrfu
                .data
                .set(crate::eval::typval::CallbackData::NONE);
            buf.b_p_qe.set_option_cstr(opt_vars().p_qe.as_cstr());
            copy_opt_sctx(buf, K_BUF_OPT_QUOTEESCAPE);
            buf.b_p_udf.set(opt_vars().p_udf.get());
            copy_opt_sctx(buf, K_BUF_OPT_UNDOFILE);
            buf.b_p_lw.set(empty_opt.clone());
            buf.b_p_menc = empty_opt;

            if dont_do_help {
                buf.b_p_isk = save_p_isk;
                if !opt_vars().p_vts.is_null()
                    && !opt_vars().p_vts.is_empty()
                    && buf.b_p_vts_array.is_empty()
                {
                    tabstop_set(opt_vars().p_vts.as_bytes(), &mut buf.b_p_vts_array, None);
                } else {
                    buf.b_p_vts_array.clear();
                }
            } else {
                buf.b_p_isk = opt_vars().p_isk.clone();
                copy_opt_sctx(buf, K_BUF_OPT_ISKEYWORD);
                did_isk = true;
                buf.b_p_ts.set(opt_vars().p_ts.get());
                copy_opt_sctx(buf, K_BUF_OPT_TABSTOP);
                buf.b_p_vts = opt_vars().p_vts.clone();
                copy_opt_sctx(buf, K_BUF_OPT_VARTABSTOP);
                if !opt_vars().p_vts.is_null()
                    && !opt_vars().p_vts.is_empty()
                    && buf.b_p_vts_array.is_empty()
                {
                    tabstop_set(opt_vars().p_vts.as_bytes(), &mut buf.b_p_vts_array, None);
                } else {
                    buf.b_p_vts_array.clear();
                }
                buf.b_help.set(false);
                if buf.b_p_bt.first_byte() == b'h' {
                    buf.b_p_bt.clear_option_str();
                }
                buf.b_p_ma.set(opt_vars().p_ma.get());
                copy_opt_sctx(buf, K_BUF_OPT_MODIFIABLE);
            }
        }

        if should_copy {
            buf.b_p_initialized.set(true);
        }
    }

    check_buf_options(buf);
    if did_isk {
        buf_init_isk_chartab(buf);
    }
}

/// Reset the 'modifiable' option and its default value.
pub fn reset_modifiable() {
    curbuf_mut().b_p_ma.set(false);
    opt_vars().p_ma.set(false);
    change_option_default(K_OPT_MODIFIABLE, boolean_obj(false));
}

/// Set the global value for 'iminsert' to the local value.
pub fn set_iminsert_global(buf: &mut FileBuf) {
    opt_vars().p_iminsert.set(buf.b_p_iminsert.get());
}

/// Set the global value for 'imsearch' to the local value.
pub fn set_imsearch_global(buf: &mut FileBuf) {
    opt_vars().p_imsearch.set(buf.b_p_imsearch.get());
}

pub fn set_context_in_set_cmd(xp: &mut ExpandCtx, arg_cstr: &std::ffi::CStr, opt_flags: i32) {
    let mut arg = arg_cstr;
    OPT_STATE.with(|s| s.expand_option_flags.set(opt_flags));

    xp.xp_context = EXPAND_SETTINGS;
    let arg_bytes = arg_cstr.to_bytes();
    if arg_bytes.is_empty() {
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
        return;
    }
    let argend = crate::memory::CStrSubslice::advance(arg, arg_bytes.len());
    let mut p_idx = arg_bytes.len() - 1;
    if arg_bytes[p_idx] == b' ' && (p_idx == 0 || arg_bytes[p_idx - 1] != b'\\') {
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(
            crate::memory::CStrSubslice::advance(arg, p_idx + 1),
        );
        return;
    }
    while p_idx > 0 {
        let mut s_idx = p_idx;
        if arg_bytes[p_idx] == b' ' || arg_bytes[p_idx] == b',' {
            while s_idx > 0 && arg_bytes[s_idx - 1] == b'\\' {
                s_idx -= 1;
            }
        }
        if arg_bytes[p_idx] == b' ' && ((p_idx - s_idx) & 1) == 0 {
            p_idx += 1;
            break;
        }
        p_idx -= 1;
    }
    if arg_bytes[p_idx..].starts_with(b"no") {
        xp.xp_context = EXPAND_BOOL_SETTINGS;
        xp.xp_prefix = XP_PREFIX_NO;
        p_idx += 2;
    } else if arg_bytes[p_idx..].starts_with(b"inv") {
        xp.xp_context = EXPAND_BOOL_SETTINGS;
        xp.xp_prefix = XP_PREFIX_INV;
        p_idx += 3;
    }
    let mut p = crate::memory::CStrSubslice::advance(arg, p_idx);
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(p);
    let p_bytes = &arg_bytes[p_idx..];
    arg = p;

    let mut nextchar: u8;
    let mut flags: u32 = 0;
    let mut opt_idx: OptIndex = 0;
    let mut is_term_option = false;

    if p_bytes.first() == Some(&b'<') {
        let Some(gt_off) = p_bytes.iter().position(|&b| b == b'>') else {
            return;
        };
        p = crate::memory::CStrSubslice::advance(p, gt_off + 1);
        let key = get_special_key_code(&arg.to_bytes()[1..]);
        if key == 0 {
            xp.xp_context = EXPAND_NOTHING;
            return;
        }
        nextchar = p_bytes.get(gt_off + 1).copied().unwrap_or(0);
        is_term_option = true;
        OPT_STATE.with(|s| s.expand_option_name.get())[2] = key2termcap0(key) as u8;
        OPT_STATE.with(|s| s.expand_option_name.get())[3] = key2termcap1(key) as u8;
    } else if p_bytes.starts_with(b"t_") {
        if p_bytes.len() < 4 {
            return;
        }
        p = crate::memory::CStrSubslice::advance(p, 4);
        nextchar = p_bytes.get(4).copied().unwrap_or(0);
        is_term_option = true;
        OPT_STATE.with(|s| s.expand_option_name.get())[2] = p_bytes[2];
        OPT_STATE.with(|s| s.expand_option_name.get())[3] = p_bytes[3];
    } else {
        let name_len = p_bytes
            .iter()
            .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'*')
            .count();
        if name_len == p_bytes.len() {
            return;
        }
        p = crate::memory::CStrSubslice::advance(p, name_len);
        nextchar = p_bytes[name_len];
        opt_idx = find_option_len(p_bytes, name_len);
        if opt_idx == K_OPT_INVALID || is_option_hidden(opt_idx) {
            xp.xp_context = EXPAND_NOTHING;
            return;
        }
        flags = get_option(opt_idx).flags;
        if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
            xp.xp_context = EXPAND_NOTHING;
            return;
        }
    }

    OPT_STATE.with(|s| s.expand_option_append.set(false));
    let mut expand_option_subtract = false;
    if matches!(nextchar, b'-' | b'+' | b'^') && p.to_bytes().get(1) == Some(&b'=') {
        if nextchar == b'-' {
            expand_option_subtract = true;
        }
        if nextchar == b'+' || nextchar == b'^' {
            OPT_STATE.with(|s| s.expand_option_append.set(true));
        }
        p = crate::memory::CStrSubslice::advance(p, 1);
        nextchar = b'=';
    }
    if (nextchar != b'=' && nextchar != b':') || xp.xp_context == EXPAND_BOOL_SETTINGS {
        xp.xp_context = EXPAND_UNSUCCESSFUL;
        return;
    }

    if is_term_option {
        OPT_STATE.with(|s| s.expand_option_idx.set(K_OPT_INVALID));
    } else {
        OPT_STATE.with(|s| s.expand_option_idx.set(opt_idx));
    }

    let p_after_eq = crate::memory::CStrSubslice::advance(p, 1);
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(p_after_eq);
    OPT_STATE.with(|s| {
        s.expand_option_start_col
            .set(((p_after_eq.as_ptr() as usize).saturating_sub(xp.xp_line.addr()) as isize) as i32)
    });

    let opt_var = get_option(opt_idx).var();
    if opt_var.ptr_eq(&opt_vars().p_syn) {
        xp.xp_context = EXPAND_OWNSYNTAX;
        return;
    }
    if opt_var.ptr_eq(&opt_vars().p_ft) {
        xp.xp_context = EXPAND_FILETYPE;
        return;
    }
    if opt_var.ptr_eq(&opt_vars().p_keymap) {
        xp.xp_context = EXPAND_KEYMAP;
        return;
    }

    if expand_option_subtract {
        xp.xp_context = EXPAND_SETTING_SUBTRACT;
        return;
    } else if OPT_STATE.with(|s| s.expand_option_idx.get()) != K_OPT_INVALID
        && get_option(OPT_STATE.with(|s| s.expand_option_idx.get()))
            .opt_expand_cb
            .is_some()
    {
        xp.xp_context = EXPAND_STRING_SETTING;
    } else if xp.xp_pattern.is_empty() {
        xp.xp_context = EXPAND_OLD_SETTING;
        return;
    } else {
        xp.xp_context = EXPAND_NOTHING;
    }

    if is_term_option || option_has_type(opt_idx, ObjectType::ObjectTypeInteger) {
        return;
    }

    if (flags & K_OPT_FLAG_EXPAND) != 0 {
        let v = get_option(opt_idx).var();
        if v.ptr_eq(&opt_vars().p_bdir)
            || v.ptr_eq(&opt_vars().p_dir)
            || v.ptr_eq(&opt_vars().p_path)
            || v.ptr_eq(&opt_vars().p_pp)
            || v.ptr_eq(&opt_vars().p_rtp)
            || v.ptr_eq(&opt_vars().p_cdpath)
            || v.ptr_eq(&opt_vars().p_vdir)
        {
            xp.xp_context = EXPAND_DIRECTORIES;
            if v.ptr_eq(&opt_vars().p_path) || v.ptr_eq(&opt_vars().p_cdpath) {
                xp.xp_backslash = XP_BS_THREE;
            } else {
                xp.xp_backslash = XP_BS_ONE;
            }
        } else {
            xp.xp_context = EXPAND_FILES;
            if v.ptr_eq(&opt_vars().p_tags) {
                xp.xp_backslash = XP_BS_THREE;
            } else {
                xp.xp_backslash = XP_BS_ONE;
            }
        }
        if (flags & K_OPT_FLAG_COMMA) != 0 {
            xp.xp_backslash |= XP_BS_COMMA;
        }
    }

    if (flags & K_OPT_FLAG_EXPAND) != 0
        || (flags & K_OPT_FLAG_COMMA) != 0
        || (flags & K_OPT_FLAG_COLON) != 0
    {
        let pat_start_idx = xp.xp_pattern.offset_from(arg_cstr);
        let mut idx = arg_bytes.len();
        while idx > pat_start_idx + 1 {
            idx -= 1;
            let b = arg_bytes[idx];
            if b == b' ' || b == b',' || (b == b':' && (flags & K_OPT_FLAG_COLON) != 0) {
                let mut s_idx = idx;
                while s_idx > pat_start_idx && arg_bytes[s_idx - 1] == b'\\' {
                    s_idx -= 1;
                }
                let bsl_cnt = (idx - s_idx) as isize;
                let comma_unesc = b == b','
                    && (flags & K_OPT_FLAG_COMMA) != 0
                    && bsl_cnt < if cfg!(windows) { 1 } else { 2 };

                if (b == b' ' && ((xp.xp_backslash & XP_BS_THREE) != 0 && bsl_cnt < 3))
                    || comma_unesc
                    || (b == b':' && (flags & K_OPT_FLAG_COLON) != 0)
                {
                    xp.xp_pattern =
                        crate::strings::OwnedSlice::from_cstr(arg_cstr).wrapping_add(idx + 1);
                    break;
                }
            }
        }
    }

    if (flags & K_OPT_FLAG_FLAG_LIST) != 0 {
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(argend);
    }

    if get_option(opt_idx).var().ptr_eq(&opt_vars().p_sps) {
        if xp.xp_pattern.as_bytes().starts_with(b"file:") {
            xp.xp_pattern = xp.xp_pattern.wrapping_add(5);
        } else if get_option(OPT_STATE.with(|s| s.expand_option_idx.get()))
            .opt_expand_cb
            .is_some()
        {
            xp.xp_context = EXPAND_STRING_SETTING;
        }
    }
}

fn match_str(
    str_arg: &std::ffi::CStr,
    regmatch: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
    fuzzy: bool,
    fuzzystr: Option<&std::ffi::CStr>,
    fuzmatch: &mut Vec<FuzmatchStr>,
) -> bool {
    if !fuzzy {
        if vim_regexec(regmatch, str_arg, 0) {
            matches.push(str_arg.to_owned());
            return true;
        }
    } else {
        let score = fuzzy_match_str(Some(str_arg.to_bytes()), fuzzystr.map(CStr::to_bytes));
        if score != FUZZY_SCORE_NONE {
            let idx = fuzmatch.len() as i32;
            fuzmatch.push(FuzmatchStr {
                idx,
                str_cstr: str_arg.to_owned(),
                score,
            });
            return true;
        }
    }
    false
}

pub fn expand_settings(
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    fuzzystr: Option<&std::ffi::CStr>,
    matches: &mut Vec<std::ffi::CString>,
    can_fuzzy: bool,
) -> bool {
    let names: [&CStr; 1] = [c"all"];
    let ic = regmatch.rm_ic;

    matches.clear();
    let mut fuzmatch: Vec<FuzmatchStr> = Vec::new();
    let fuzzy = can_fuzzy && cmdline_fuzzy_complete(fuzzystr.unwrap_or(c""));

    regmatch.rm_ic = ic;
    if xp.xp_context != EXPAND_BOOL_SETTINGS {
        for &name in &names {
            match_str(name, regmatch, matches, fuzzy, fuzzystr, &mut fuzmatch);
        }
    }
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        if is_option_hidden(opt_idx) {
            continue;
        }
        if xp.xp_context == EXPAND_BOOL_SETTINGS
            && !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean)
        {
            continue;
        }

        if !match_str(
            opt.fullname,
            regmatch,
            matches,
            fuzzy,
            fuzzystr,
            &mut fuzmatch,
        ) && !fuzzy
            && opt.shortname.is_some_and(|s| vim_regexec(regmatch, s, 0))
        {
            matches.push(opt.fullname.to_owned());
        }
    }

    if fuzzy {
        fuzzymatches_slice_to_strmatches(&mut fuzmatch, matches, false);
    }

    true
}

/// Escape an option value that can be used on the command-line with :set.
pub fn escape_option_str_cmdline(var_arg: &[u8]) -> std::ffi::CString {
    let buf_cstr = vim_strsave_escaped(
        crate::memory::bytes_until_nul(var_arg),
        ESCAPE_CHARS.to_bytes(),
    );

    #[cfg(windows)]
    {
        let src = buf_cstr.to_bytes();
        let mut out = Vec::with_capacity(src.len());
        let expand_ok = OPT_STATE.with(|s| s.expand_option_idx.get()) != K_OPT_INVALID
            && (get_option(OPT_STATE.with(|s| s.expand_option_idx.get())).flags
                & K_OPT_FLAG_EXPAND)
                != 0;
        let mut idx = 0usize;
        while idx < src.len() {
            if expand_ok
                && src[idx..].starts_with(b"\\\\")
                && src.get(idx + 2).is_some_and(|&b| vim_isfilec(i32::from(b)))
                && (src[idx + 2] != b'\\' || (idx == 0 && src.get(idx + 4) != Some(&b'\\')))
            {
                idx += 1;
            }
            let clen = crate::mbyte::utfc_slice2len(&src[idx..])
                .max(1)
                .min(src.len() - idx);
            out.extend_from_slice(&src[idx..idx + clen]);
            idx += clen;
        }
        crate::memory::bytes_to_cstring(&out)
    }
    #[cfg(not(windows))]
    {
        buf_cstr
    }
}

/// Expansion handler for :set= when we just want to fill in with the existing value.
pub fn expand_old_setting(matches: &mut Vec<std::ffi::CString>) -> bool {
    matches.clear();

    if OPT_STATE.with(|s| s.expand_option_idx.get()) == K_OPT_INVALID {
        OPT_STATE.with(|s| {
            s.expand_option_idx
                .set(find_option(&OPT_STATE.with(|s| s.expand_option_name.get())))
        });
    }

    let var = if OPT_STATE.with(|s| s.expand_option_idx.get()) != K_OPT_INVALID {
        let o = opt_from_varp(
            OPT_STATE.with(|s| s.expand_option_idx.get()),
            get_varp_scope(
                get_option(OPT_STATE.with(|s| s.expand_option_idx.get())),
                OPT_STATE.with(|s| s.expand_option_flags.get()),
            ),
        );
        let is_luafn = o.obj_type == ObjectType::ObjectTypeLuaRef;
        optval_free_read(OPT_STATE.with(|s| s.expand_option_idx.get()), o);
        if is_luafn {
            return true;
        }
        optval_fmt(
            OPT_STATE.with(|s| s.expand_option_idx.get()),
            OPT_STATE.with(|s| s.expand_option_flags.get()),
        );
        g_st().name_buff_cstr().to_bytes()
    } else {
        b""
    };

    matches.push(escape_option_str_cmdline(var));
    true
}

/// Expansion handler for :set=/:set+= when the option has a custom expansion handler.
pub fn expand_string_setting(
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    if OPT_STATE.with(|s| s.expand_option_idx.get()) == K_OPT_INVALID {
        return false;
    }
    let opt = get_option(OPT_STATE.with(|s| s.expand_option_idx.get()));
    let Some(expand_cb) = opt.opt_expand_cb else {
        return false;
    };

    let set_arg = xp
        .xp_line
        .wrapping_add(OPT_STATE.with(|s| s.expand_option_start_col.get()) as usize);
    optval_fmt(
        OPT_STATE.with(|s| s.expand_option_idx.get()),
        OPT_STATE.with(|s| s.expand_option_flags.get()),
    );
    let buf = escape_option_str_cmdline(&*g_st().name_buff_bytes_mut());
    let mut args = Optexpand {
        oe_idx: OPT_STATE.with(|s| s.expand_option_idx.get()),
        oe_opt_value: buf.as_c_str(),
        oe_append: OPT_STATE.with(|s| s.expand_option_append.get()),
        oe_include_orig_val: !OPT_STATE.with(|s| s.expand_option_append.get())
            && set_arg.is_empty(),
        oe_regmatch: regmatch,
        oe_xp: xp,
        oe_set_arg: set_arg,
    };

    expand_cb(&mut args, matches)
}

/// Expansion handler for :set-=
pub fn expand_setting_subtract(
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    if OPT_STATE.with(|s| s.expand_option_idx.get()) == K_OPT_INVALID
        || option_has_type(
            OPT_STATE.with(|s| s.expand_option_idx.get()),
            ObjectType::ObjectTypeInteger,
        )
        || is_callback_option(OPT_STATE.with(|s| s.expand_option_idx.get()))
        || is_option_hidden(OPT_STATE.with(|s| s.expand_option_idx.get()))
    {
        return expand_old_setting(matches);
    }

    let option_val_cstr = get_option_varp_scope_from(
        OPT_STATE.with(|s| s.expand_option_idx.get()),
        OPT_STATE.with(|s| s.expand_option_flags.get()),
        curbuf_mut(),
        Some(curwin_mut()),
    )
    .as_cstr();

    let option_flags = get_option(OPT_STATE.with(|s| s.expand_option_idx.get())).flags;

    if (option_flags & K_OPT_FLAG_COMMA) != 0 {
        if option_val_cstr.is_empty() {
            return false;
        }

        let option_copy = option_val_cstr.to_owned().into_bytes_with_nul();
        let copy_len = option_copy.len() - 1;
        let mut start = 0usize;

        matches.clear();

        loop {
            let mut end = start;
            while end < copy_len {
                if option_copy[end] == b',' && (end == start || option_copy[end - 1] != b'\\') {
                    break;
                }
                end += 1;
            }

            let item_bytes = &option_copy[start..end];
            if !item_bytes.is_empty() {
                let item_cstr = crate::memory::bytes_to_cstring(item_bytes);
                if vim_regexec(regmatch, &item_cstr, 0) {
                    matches.push(escape_option_str_cmdline(item_bytes));
                }
            }

            if end >= copy_len {
                break;
            }
            start = end + 1;
        }

        return true;
    } else if (option_flags & K_OPT_FLAG_FLAG_LIST) != 0 {
        if !xp.xp_pattern.is_empty() {
            return false;
        }

        let num_flags = option_val_cstr.to_bytes().len();
        if num_flags == 0 {
            return false;
        }

        matches.clear();
        matches.reserve_exact(num_flags + 1);
        matches.push(option_val_cstr.to_owned());

        if num_flags > 1 {
            for &flag_byte in option_val_cstr.to_bytes() {
                matches.push(crate::memory::bytes_to_cstring(&[flag_byte]));
            }
        }

        return true;
    }

    expand_old_setting(matches)
}

fn optval_fmt(opt_idx: OptIndex, opt_flags: i32) {
    let opt = get_option(opt_idx);
    let vp = get_varp_scope(opt.clone(), opt_flags);
    assert!(vp.is_some());

    let mut wc: OptInt = 0;
    if wc_use_keyname(vp, &mut wc) != 0 {
        crate::memory::copy_bytes_to_c_buf(
            &mut *g_st().name_buff_bytes_mut(),
            get_special_key_name(wc as i32, 0).to_bytes(),
        );
        return;
    }
    if wc != 0 {
        crate::memory::copy_bytes_to_c_buf(
            &mut *g_st().name_buff_bytes_mut(),
            transchar(wc as i32).to_bytes(),
        );
        return;
    }
    if (opt.flags & K_OPT_FLAG_EXPAND) != 0
        && !is_callback_option(opt_idx)
        && option_has_type(opt_idx, ObjectType::ObjectTypeString)
    {
        home_replace(
            None,
            Some(vp.as_cstr()),
            &mut *g_st().name_buff_bytes_mut(),
            false,
        );
        return;
    }

    let o = opt_from_varp(opt_idx, vp);
    let s = optval_to_cstr(o.clone(), false);
    crate::memory::copy_bytes_to_c_buf(&mut *g_st().name_buff_bytes_mut(), s.as_bytes());
    optval_free_read(opt_idx, o);
}

fn wc_use_keyname(varp: OptVar, wcp: &mut OptInt) -> i32 {
    if varp.ptr_eq(&opt_vars().p_wc) || varp.ptr_eq(&opt_vars().p_wcm) {
        *wcp = varp.int_val();
        if is_special(*wcp as i32) || find_special_key_in_table(*wcp as i32) >= 0 {
            return 1;
        }
    }
    0
}

pub fn shortmess(x: i32) -> bool {
    !opt_vars().p_shm.is_null()
        && (crate::strings::vim_haschr(opt_vars().p_shm.as_cstr(), x)
            || (crate::strings::vim_haschr(opt_vars().p_shm.as_cstr(), b'a' as i32)
                && crate::strings::vim_haschr(SHM_ALL_ABBREVIATIONS, x)))
}

pub fn vimrc_found(fname: Option<&std::ffi::CStr>, envname: &std::ffi::CStr) {
    if let Some(fname) = fname
        && vim_getenv(envname).is_none()
        && let Some(p) = FullName_save(Some(fname), false)
    {
        os_setenv(envname, p.as_c_str(), 1);
    }
}

/// Check whether global option has been set.
pub fn option_was_set(opt_idx: OptIndex) -> bool {
    assert!(opt_idx != K_OPT_INVALID);
    (get_option(opt_idx).flags & K_OPT_FLAG_WAS_SET) != 0
}

/// Reset the flag indicating option "name" was set.
pub fn reset_option_was_set(opt_idx: OptIndex) {
    assert!(opt_idx != K_OPT_INVALID);
    OPTIONS.with(|opts| opts.borrow_mut()[opt_idx as usize].flags &= !K_OPT_FLAG_WAS_SET);
}

/// Parse 'cursorlineopt', writing flags only when the value is valid.
pub fn fill_culopt_flags(val: &[u8], flags: &mut u32) -> bool {
    let mut rem = crate::memory::bytes_until_nul(val);
    let mut culopt_flags_new: u8 = 0;

    while !rem.is_empty() {
        if let Some(rest) = rem.strip_prefix(b"line") {
            rem = rest;
            culopt_flags_new |= K_OPT_CULOPT_FLAG_LINE;
        } else if let Some(rest) = rem.strip_prefix(b"both") {
            rem = rest;
            culopt_flags_new |= K_OPT_CULOPT_FLAG_LINE | K_OPT_CULOPT_FLAG_NUMBER;
        } else if let Some(rest) = rem.strip_prefix(b"number") {
            rem = rest;
            culopt_flags_new |= K_OPT_CULOPT_FLAG_NUMBER;
        } else if let Some(rest) = rem.strip_prefix(b"screenline") {
            rem = rest;
            culopt_flags_new |= K_OPT_CULOPT_FLAG_SCREENLINE;
        }

        if let Some(after_comma) = rem.strip_prefix(b",") {
            rem = after_comma;
        } else if !rem.is_empty() {
            return false;
        }
    }

    if (culopt_flags_new & K_OPT_CULOPT_FLAG_LINE) != 0
        && (culopt_flags_new & K_OPT_CULOPT_FLAG_SCREENLINE) != 0
    {
        return false;
    }
    *flags = u32::from(culopt_flags_new);

    true
}

/// Set the callback function value for an option that accepts a function name, lambda, et al.
pub fn option_set_callback_func(optval: &[u8], optcb: &mut Callback) -> bool {
    let bytes = crate::memory::bytes_until_nul(optval);
    if bytes.is_empty() {
        callback_free(optcb);
        return true;
    }

    let optval_cstr = crate::memory::bytes_to_cstring(bytes);
    let mut tv: TypVal =
        if bytes[0] == b'{' || bytes.starts_with(b"function(") || bytes.starts_with(b"funcref(") {
            let Some(evaluated) = eval_expr(&optval_cstr, None) else {
                return false;
            };
            evaluated
        } else {
            let mut allocated = TypVal::new();
            crate::eval::typval::tv_set_string(&mut allocated, Some(optval_cstr.as_c_str()));
            allocated
        };

    let cb = Callback::NONE;
    if !callback_from_typval(&cb, &tv) || cb.cb_type() == K_CALLBACK_NONE_CONST {
        tv_clear(&mut tv);
        return false;
    }

    callback_free(optcb);
    *optcb = cb;
    tv_clear(&mut tv);
    true
}

fn didset_options_sctx(opt_flags: i32, opts: &[i32]) {
    for &opt_idx in opts {
        set_option_sctx(opt_idx, opt_flags, g_st().current_sctx.get());
    }
}

/// Check if backspacing over something is allowed.
pub fn can_bs(what: i32) -> bool {
    if what == BS_START && bt_prompt(Some(crate::globals::curbuf_mut())) {
        return false;
    }

    if what == BS_EOL && !mc_ins_replay_can_join() {
        return false;
    }

    if opt_vars().p_bs.first_byte() == b'2' {
        return what != BS_NOSTOP;
    }

    crate::strings::vim_haschr(opt_vars().p_bs.as_cstr(), what)
}

/// Get the local or global value of 'backupcopy' flags.
pub fn get_bkc_flags(buf: &mut FileBuf) -> u32 {
    if buf.b_bkc_flags.get() != 0 {
        buf.b_bkc_flags.get()
    } else {
        opt_vars().bkc_flags.get()
    }
}

/// Get the local or global value of 'formatlistpat'.
pub fn get_flp_value(buf: &FileBuf) -> std::ffi::CString {
    if buf.b_p_flp.is_empty() {
        opt_vars().p_flp.to_owned()
    } else {
        buf.b_p_flp.to_owned()
    }
}

/// Get the local or global value of 'virtualedit' flags.
pub fn get_ve_flags(wp: &mut WinInfoInfo) -> u32 {
    let f = if wp.w_onebuf_opt.wo_ve_flags.get() != 0 {
        wp.w_onebuf_opt.wo_ve_flags.get()
    } else {
        opt_vars().ve_flags.get()
    };
    f & !(K_OPT_VE_FLAG_NONE | K_OPT_VE_FLAG_NONE_U)
}

/// Get the local or global value of 'showbreak'.
pub fn get_showbreak_value(win: &WinInfoInfo) -> std::ffi::CString {
    let sbr = &win.w_onebuf_opt.wo_sbr;
    if sbr.is_empty() {
        return opt_vars().p_sbr.to_owned();
    }
    if sbr.as_bytes() == b"NONE" {
        return std::ffi::CString::default();
    }
    sbr.to_owned()
}

/// Return the current end-of-line type: EOL_DOS, EOL_UNIX or EOL_MAC.
pub fn get_fileformat(buf: &FileBuf) -> i32 {
    let c = buf.b_p_ff.first_byte();
    if buf.b_p_bin.get() || c == b'u' {
        return EOL_UNIX;
    }
    if c == b'm' {
        return EOL_MAC;
    }
    EOL_DOS
}

/// Like get_fileformat(), but override 'fileformat' with "p" for "++opt=val" argument.
pub fn get_fileformat_force(buf: &FileBuf, eap: Option<&Exarg>) -> i32 {
    let c = if let Some(eap) = eap
        && eap.force_ff != 0
    {
        eap.force_ff as u8
    } else {
        let is_bin = if let Some(eap) = eap
            && let Some(bin) = eap.force_bin
        {
            bin
        } else {
            buf.b_p_bin.get()
        };
        if is_bin {
            return EOL_UNIX;
        }
        buf.b_p_ff.first_byte()
    };
    if c == b'u' {
        return EOL_UNIX;
    }
    if c == b'm' {
        return EOL_MAC;
    }
    EOL_DOS
}

/// Return the default fileformat from 'fileformats'.
pub fn default_fileformat() -> i32 {
    match opt_vars().p_ffs.first_byte() {
        b'm' => EOL_MAC,
        b'd' => EOL_DOS,
        _ => EOL_UNIX,
    }
}

/// Set the current end-of-line type to EOL_UNIX, EOL_MAC, or EOL_DOS.
pub fn set_fileformat(eol_style: i32, opt_flags: i32) {
    let p: Option<&CStr> = match eol_style {
        EOL_UNIX => Some(c"unix"),
        EOL_MAC => Some(c"mac"),
        EOL_DOS => Some(c"dos"),
        _ => None,
    };

    if let Some(p) = p {
        set_option_direct(K_OPT_FILEFORMAT, cstr_as_obj(Some(p)), opt_flags, 0);
    }

    redraw_buf_status_later(curbuf_mut());
    g_st().redraw_tabline.set(true);
    g_st().need_maketitle.set(true);
}

/// Skip to next part of an option argument: skip space and comma.
#[inline]
pub fn skip_to_option_part_slice(mut p: &[u8]) -> &[u8] {
    if let Some((&b',', rest)) = p.split_first() {
        p = rest;
    }
    while let Some((&b' ', rest)) = p.split_first() {
        p = rest;
    }
    p
}

/// Isolate one part of a string option separated by `sep_chars`.
pub fn copy_option_part(option: &mut &[u8], out: &mut [u8], sep_chars: &std::ffi::CStr) -> usize {
    let mut len: usize = 0;
    let maxlen = out.len();
    let p_bytes = crate::memory::bytes_until_nul(option);
    let mut rem = p_bytes;
    let sep = sep_chars.to_bytes();

    if let Some((&b'.', rest)) = rem.split_first() {
        if len < maxlen.saturating_sub(1) {
            out[len] = b'.';
            len += 1;
        }
        rem = rest;
    }
    while let Some((&b, rest)) = rem.split_first() {
        if sep.contains(&b) {
            break;
        }
        let mut ch = b;
        rem = rest;
        if b == b'\\'
            && let Some((&b1, rest2)) = rem.split_first()
            && sep.contains(&b1)
        {
            ch = b1;
            rem = rest2;
        }
        if len < maxlen.saturating_sub(1) {
            out[len] = ch;
            len += 1;
        }
    }
    if maxlen > 0 {
        out[len] = 0;
    }

    if !rem.is_empty() && rem[0] != b',' {
        rem = &rem[1..];
    }
    rem = skip_to_option_part_slice(rem);

    let consumed = p_bytes.len() - rem.len();
    *option = &option[consumed..];

    len
}

/// Iterates comma-separated parts of option, splitting each part at its first ':' into key/value.
pub fn option_next_keyval<'a>(p: &mut &'a [u8]) -> Option<(&'a [u8], Option<&'a [u8]>)> {
    let bytes = *p;
    if bytes.is_empty() {
        return None;
    }

    let comma_off = bytes.iter().position(|&b| b == b',');
    let end_off = comma_off.unwrap_or(bytes.len());
    let part = &bytes[..end_off];

    let (key, val) = if let Some(colon_off) = part.iter().position(|&b| b == b':') {
        (&part[..colon_off], Some(&part[colon_off + 1..]))
    } else {
        (part, None)
    };

    *p = if let Some(c_off) = comma_off {
        &bytes[c_off + 1..]
    } else {
        &bytes[end_off..]
    };
    Some((key, val))
}

/// Return true when 'shell' has "csh" in the tail.
pub fn csh_like_shell() -> i32 {
    i32::from(
        path_tail(opt_vars().p_sh.as_cstr())
            .to_bytes()
            .windows(3)
            .any(|w| w == b"csh"),
    )
}

/// Return true when 'shell' has "fish" in the tail.
pub fn fish_like_shell() -> bool {
    path_tail(opt_vars().p_sh.as_cstr())
        .to_bytes()
        .windows(4)
        .any(|w| w == b"fish")
}

/// Get window or buffer local options.
pub fn get_winbuf_options(bufopt: i32) -> crate::eval::typval::DictId {
    let d = tv_dict_alloc();

    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);

        if (bufopt != 0 && option_has_scope(opt_idx, K_OPT_SCOPE_BUF))
            || (bufopt == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_WIN))
        {
            let varp = get_varp(opt.clone());
            if varp.is_some() {
                let ov = opt_from_varp(opt_idx, varp);
                let mut opt_tv = opt_to_tv(ov.clone(), true);
                tv_dict_add_tv(&d, opt.fullname.to_bytes(), &mut opt_tv);
                tv_clear(&mut opt_tv);
                optval_free_read(opt_idx, ov);
            }
        }
    }

    d
}

/// Return the effective 'scrolloff' value for the current window.
pub fn get_scrolloff_value(wp: &mut WinInfoInfo) -> i64 {
    if (g_st().state_global.get() & MODE_TERMINAL) != 0 && !wp.buf_mut().terminal.get().is_null() {
        return 0;
    }
    if wp.w_onebuf_opt.wo_so.get() < 0 {
        opt_vars().p_so.get()
    } else {
        wp.w_onebuf_opt.wo_so.get()
    }
}

/// Return the effective 'scrolloffpad' value for the current window.
pub fn get_scrolloffpad_value(wp: &mut WinInfoInfo) -> i64 {
    if wp.w_onebuf_opt.wo_sop.get() == -1 {
        opt_vars().p_sop.get()
    } else {
        wp.w_onebuf_opt.wo_sop.get()
    }
}

/// Return the effective 'sidescrolloff' value for the current window.
pub fn get_sidescrolloff_value(wp: &mut WinInfoInfo) -> i64 {
    if wp.w_onebuf_opt.wo_siso.get() < 0 {
        opt_vars().p_siso.get()
    } else {
        wp.w_onebuf_opt.wo_siso.get()
    }
}

/// Metadata Dict for option `name`; errors if `name` is unknown.
pub fn get_option_info(
    name: NvimString,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: &mut WinInfoInfo,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
    let opt_idx = find_option_len(name.as_bytes(), name.size);
    if opt_idx == K_OPT_INVALID {
        api_set_error!(
            err,
            ErrorType::ErrorTypeValidation,
            c"Invalid option (not found): '%s'",
            name,
        );
        return Dict::new();
    }

    option_info_dict(get_option(opt_idx), opt_flags, buf, win, arena)
}

/// Metadata Dict for every option, keyed by full name.
pub fn get_all_options_info(arena: &mut Arena) -> Dict {
    let mut retval = arena_dict(arena, K_OPT_COUNT as usize);
    for opt_idx in 0..K_OPT_COUNT {
        let opt = get_option(opt_idx);
        let fullname = opt.fullname;
        let opt_dict = option_info_dict(opt, OPT_GLOBAL, curbuf_mut(), curwin_mut(), arena);
        put_c(&mut retval, fullname, dict_obj(opt_dict));
    }
    retval
}

fn option_info_dict(
    opt: Vimoption,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: &mut WinInfoInfo,
    arena: &mut Arena,
) -> Dict {
    let opt_idx = get_opt_idx(opt.clone());
    let mut dict = arena_dict(arena, 13);

    put_c(&mut dict, c"name", cstr_as_obj(Some(opt.fullname)));
    put_c(&mut dict, c"shortname", cstr_as_obj(opt.shortname));

    let scope = if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
        c"buf"
    } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
        c"win"
    } else if option_has_scope(opt_idx, K_OPT_SCOPE_TAB) {
        c"tab"
    } else {
        c"global"
    };

    put_c(&mut dict, c"scope", cstr_as_obj(Some(scope)));

    put_c(
        &mut dict,
        c"global_local",
        boolean_obj(option_is_global_local(opt_idx)),
    );
    put_c(
        &mut dict,
        c"commalist",
        boolean_obj((opt.flags & K_OPT_FLAG_COMMA) != 0),
    );
    put_c(
        &mut dict,
        c"flaglist",
        boolean_obj((opt.flags & K_OPT_FLAG_FLAG_LIST) != 0),
    );

    put_c(
        &mut dict,
        c"was_set",
        boolean_obj((opt.flags & K_OPT_FLAG_WAS_SET) != 0),
    );

    let mut script_ctx = Sctx {
        sc_sid: 0,
        sc_seq: 0,
        sc_lnum: 0,
        sc_chan: 0,
    };
    if opt_flags == OPT_GLOBAL {
        script_ctx = opt.script_ctx;
    } else {
        if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
            script_ctx = buf.b_p_script_ctx[opt.scope_idx[K_OPT_SCOPE_BUF as usize] as usize].get();
        }
        if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
            script_ctx = win.w_onebuf_opt.wo_script_ctx
                [opt.scope_idx[K_OPT_SCOPE_WIN as usize] as usize]
                .get();
        }
        if opt_flags != OPT_LOCAL && script_ctx.sc_sid == 0 {
            script_ctx = opt.script_ctx;
        }
    }

    put_c(
        &mut dict,
        c"last_set_sid",
        integer_obj(i64::from(script_ctx.sc_sid)),
    );
    put_c(
        &mut dict,
        c"last_set_linenr",
        integer_obj(i64::from(script_ctx.sc_lnum)),
    );
    put_c(
        &mut dict,
        c"last_set_chan",
        integer_obj(script_ctx.sc_chan as i64),
    );

    put_c(
        &mut dict,
        c"type",
        cstr_as_obj(Some(optval_type_name(
            get_option(get_opt_idx(opt.clone())).opt_type,
        ))),
    );
    put_c(&mut dict, c"default", opt.def_val);
    put_c(
        &mut dict,
        c"allows_duplicates",
        boolean_obj((opt.flags & K_OPT_FLAG_NO_DUP) == 0),
    );

    dict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_option_and_find_option_len_resolve_names_and_abbreviations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(find_option(b"number"), K_OPT_NUMBER);
        assert_eq!(find_option(b"nu"), K_OPT_NUMBER);
        assert_eq!(find_option_len(b"tabstop=4", 7), K_OPT_TABSTOP);
        assert_eq!(find_option(b"nonexistent_option"), K_OPT_INVALID);

        assert!(option_has_type(K_OPT_NUMBER, ObjectType::ObjectTypeBoolean));
        assert!(option_has_type(
            K_OPT_TABSTOP,
            ObjectType::ObjectTypeInteger
        ));
        assert!(option_has_scope(K_OPT_NUMBER, K_OPT_SCOPE_WIN));
        assert!(option_has_scope(K_OPT_TABSTOP, K_OPT_SCOPE_BUF));
        assert!(!is_option_hidden(K_OPT_NUMBER));
        assert!(is_option_hidden(0));
    }
}
