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
    EOL_MAC, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_iswhite, buf_init_isk_chartab,
    init_isf_chartab, init_isi_chartab, is_special, skiptowhite_esc, skipwhite, trans_characters,
    transchar, vim_str2nr, vim_strsize,
};
use crate::cmdexpand::{
    Cmdidx, EXPAND_BOOL_SETTINGS, EXPAND_DIRECTORIES, EXPAND_FILES, EXPAND_FILETYPE, EXPAND_KEYMAP,
    EXPAND_NOTHING, EXPAND_OLD_SETTING, EXPAND_OWNSYNTAX, EXPAND_SETTING_SUBTRACT, EXPAND_SETTINGS,
    EXPAND_STRING_SETTING, EXPAND_UNSUCCESSFUL, XP_BS_COMMA, XP_BS_ONE, XP_BS_THREE, XP_PREFIX_INV,
    XP_PREFIX_NO, cmdline_fuzzy_complete,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor_shape::{SHAPE_CURSOR, parse_shape_opt};
use crate::decoration_provider::get_decor_provider;
use crate::diff::{K_OPT_DIFF, K_OPT_FOLDMETHOD, diff_buf_adjust};
use crate::drawscreen::{
    UPD_CLEAR, UPD_NOT_VALID, UPD_SOME_VALID, check_screensize, comp_col, redraw_all_later,
    redraw_buf_later, redraw_buf_status_later, redraw_later, screen_resize, showmode,
    status_redraw_all, status_redraw_curbuf,
};
use crate::errors::*;
use crate::eval::encode::encode_tv2string;
use crate::eval::typval::{
    Callback, TvDict, TypVal, callback_copy, callback_free, callback_put, tv_clear, tv_dict_add_tv,
    tv_dict_alloc,
};
use crate::eval::userfunc::get_scriptlocal_funcname;
use crate::eval::vars::{
    FORCE_BIN, K_OPT_FLAG_FUNC, Scid, VV_OPTION_COMMAND, VV_OPTION_NEW, VV_OPTION_OLD,
    VV_OPTION_OLDGLOBAL, VV_OPTION_OLDLOCAL, VV_OPTION_TYPE, VV_WARNINGMSG, get_vim_var_str,
    opt_to_tv, reset_v_option_vars, set_vim_var_string, set_vim_var_tv, tristate_from_int,
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
use crate::fileio::{EOL_DOS, EOL_UNIX, FILE, K_OPT_FILEENCODING, fprintf, fputs};
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
use crate::mbyte::{enc_locale, utfc_ptr2len};
use crate::mcursor::{K_OPT_FOLLOW, mc_ins_replay_can_join};
use crate::memfile::mf_close_file;
use crate::memline::{ml_open_file, ml_open_files};
use crate::memory::{Arena, xstrlcpy};
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
    CharBuf, CharsOption, OptIndex, OptScopeFlags, Optexpand, Optset, Vimoption,
    alloc_string_option, check_buf_options, check_illegal_path_names, check_signcolumn,
    check_string_option, clear_string_option, did_set_breakat, didset_string_options,
    free_string_option, opt_dict_schema, opt_strings_check, set_chars_option,
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
use crate::strings::{
    gettext, tr, vim_snprintf, vim_snprintf_safelen, vim_strchr, vim_strsave_escaped,
};
use crate::tag::free_tagfunc_option;
use crate::terminal::on_scrollback_option_changed;
use crate::ui::ui_call_option_set;
use crate::undo::{buf_is_changed_fn, curbuf_is_changed_fn, u_compute_hash, u_read_undo, u_sync};
use crate::window::{
    MIN_COLUMNS, STATUS_HEIGHT, TabPageInfo, WinInfoInfo, Winopt, check_colorcolumn,
    command_height, frame_new_height, global_stl_height, last_status, min_rows,
    min_rows_for_all_tabpages, set_winbar, set_winbar_all, set_winbar_win, tabline_height,
    unuse_tabpage, use_tabpage, win_comp_pos, win_comp_scroll, win_default_scroll, win_equal,
    win_find_tabpage, win_new_screen_rows, win_setheight, win_setwidth,
};
use crate::winfloat::win_float_update_statusline;
use std::ffi::{CStr, c_char, c_void};
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
use crate::charset::NUL;
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

const PATHSEPSTR: *const c_char = c"/".as_ptr();

const SHM_ALL_ABBREVIATIONS: *const c_char = c"rmlw".as_ptr();

const E_UNKNOWN_OPTION: *const c_char = c"E518: Unknown option".as_ptr();
const E_NOT_ALLOWED_IN_MODELINE: *const c_char = c"E520: Not allowed in a modeline".as_ptr();
const E_NOT_ALLOWED_IN_MODELINE_WHEN_MODELINEEXPR_IS_OFF: *const c_char =
    c"E992: Not allowed in a modeline when 'modelineexpr' is off".as_ptr();
const E_NUMBER_REQUIRED_AFTER_EQUAL: *const c_char = c"E521: Number required after =".as_ptr();
const E_PREVIEW_WINDOW_ALREADY_EXISTS: *const c_char =
    c"E590: A preview window already exists".as_ptr();
const E_CANNOT_HAVE_NEGATIVE_OR_ZERO_NUMBER_OF_QUICKFIX: *const c_char =
    c"E1542: Cannot have a negative or zero number of quickfix/location lists".as_ptr();
const E_CANNOT_HAVE_MORE_THAN_HUNDRED_QUICKFIX: *const c_char =
    c"E1543: Cannot have more than a hundred quickfix/location lists".as_ptr();
const W_ARABIC: *const c_char = c"W17: Arabic requires UTF-8, do ':set encoding=utf-8'".as_ptr();

struct OptionModuleState {
    p_term: Option<std::ffi::CString>,
    p_ttytype: Option<std::ffi::CString>,
    p_et_nobin: i32,
    p_ml_nobin: i32,
    p_tw_nobin: OptInt,
    p_wm_nobin: OptInt,
    p_ai_nopaste: i32,
    p_et_nopaste: i32,
    p_sts_nopaste: OptInt,
    p_tw_nopaste: OptInt,
    p_wm_nopaste: OptInt,
    p_vsts_nopaste: Option<std::ffi::CString>,
    old_p_paste: i32,
    save_sm: i32,
    save_sta: i32,
    save_ru: i32,
    save_ri: i32,
    syn_recursive: i32,
    expand_option_idx: OptIndex,
    expand_option_start_col: i32,
    expand_option_name: [c_char; 5],
    expand_option_flags: i32,
    expand_option_append: bool,
}

impl OptionModuleState {
    const fn new() -> Self {
        Self {
            p_term: None,
            p_ttytype: None,
            p_et_nobin: 0,
            p_ml_nobin: 0,
            p_tw_nobin: 0,
            p_wm_nobin: 0,
            p_ai_nopaste: 0,
            p_et_nopaste: 0,
            p_sts_nopaste: 0,
            p_tw_nopaste: 0,
            p_wm_nopaste: 0,
            p_vsts_nopaste: None,
            old_p_paste: 0,
            save_sm: 0,
            save_sta: 0,
            save_ru: 0,
            save_ri: 0,
            syn_recursive: 0,
            expand_option_idx: K_OPT_INVALID,
            expand_option_start_col: 0,
            expand_option_name: [b't' as c_char, b'_' as c_char, 0, 0, 0],
            expand_option_flags: 0,
            expand_option_append: false,
        }
    }
}

static OPT_STATE: GlobalState<OptionModuleState> = GlobalState::new(OptionModuleState::new());

#[inline]
fn opt_st() -> &'static mut OptionModuleState {
    unsafe { &mut *OPT_STATE.as_ptr() }
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

pub fn opt_error(
    errbuf: Option<CharBuf>,
    fmt: impl crate::strings::IntoCCharPtr,
    arg: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    let fmt = fmt.into_c_char_ptr();
    let Some(errbuf) = errbuf else {
        return c"".as_ptr();
    };
    vim_snprintf!(errbuf.data, errbuf.size, gettext(fmt), arg);
    errbuf.data.cast_const()
}

pub fn opt_error_int(
    errbuf: Option<CharBuf>,
    fmt: impl crate::strings::IntoCCharPtr,
    arg: i32,
) -> *const c_char {
    let fmt = fmt.into_c_char_ptr();
    let Some(errbuf) = errbuf else {
        return c"".as_ptr();
    };
    vim_snprintf!(errbuf.data, errbuf.size, gettext(fmt), arg);
    errbuf.data.cast_const()
}

#[inline]
fn c_str_contains(haystack: impl crate::strings::IntoCCharPtr, needle: &[u8]) -> bool {
    let haystack = haystack.into_c_char_ptr();
    {
        needle.is_empty()
            || crate::memory::cstr_as_bytes(haystack)
                .windows(needle.len())
                .any(|w| w == needle)
    }
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
    crate::memory::cstr_from_bytes(&out)
}

#[inline]
fn get_opt_idx(opt: *mut Vimoption) -> OptIndex {
    crate::memory::ptr_offset_from(opt, get_option(0)) as OptIndex
}

#[inline]
fn get_varp(p: &mut Vimoption) -> *mut c_void {
    get_varp_from(p, curbuf_mut(), curwin_mut())
}

pub fn set_init_tablocal() {
    {
        // cmdheight calls itself OPT_GLOBAL but is really tablocal!
        opt_vars().p_ch = get_option(K_OPT_CMDHEIGHT).def_val.data.integer();
    }
}

/// Initialize the 'shell' option to a default value.
fn set_init_default_shell() {
    if let Some(shell) = os_getenv_cstr(c"SHELL") {
        let shell_ptr = shell.as_ptr();
        if crate::strings::vim_haschr(shell_ptr, b' ' as i32) {
            let len = shell.as_bytes().len() + 3;
            let mut cmd = vec![0 as c_char; len];
            vim_snprintf!(cmd.as_mut_ptr(), len, c"\"%s\"", shell_ptr);
            set_string_default(K_OPT_SHELL, cmd.as_ptr());
        } else {
            set_string_default(K_OPT_SHELL, shell_ptr);
        }
    }
}

/// Set the default for 'backupskip' to include environment variables for temp files.
fn set_init_default_backupskip() {
    {
        #[cfg(unix)]
        let names: [*const c_char; 4] = [
            c"".as_ptr(),
            c"TMPDIR".as_ptr(),
            c"TEMP".as_ptr(),
            c"TMP".as_ptr(),
        ];
        #[cfg(not(unix))]
        let names: [*const c_char; 3] = [c"TMPDIR".as_ptr(), c"TEMP".as_ptr(), c"TMP".as_ptr()];

        let mut buf: Vec<c_char> = Vec::new();
        let opt_idx = K_OPT_BACKUPSKIP;

        for &name in &names {
            #[cfg(unix)]
            let (p_owned, p, mut plen): (
                Option<std::ffi::CString>,
                *const c_char,
                usize,
            ) = if crate::memory::cstr_is_empty(name) {
                #[cfg(target_os = "macos")]
                {
                    (None, c"/private/tmp".as_ptr(), "/private/tmp".len())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    (None, c"/tmp".as_ptr(), "/tmp".len())
                }
            } else {
                let env = vim_getenv(name);
                let ptr = env.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                (env, ptr, 0)
            };
            #[cfg(not(unix))]
            let (p_owned, p, mut plen): (
                Option<std::ffi::CString>,
                *const c_char,
                usize,
            ) = {
                let env = vim_getenv(name);
                let ptr = env.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                (env, ptr, 0)
            };
            let _ = &p_owned;

            if !crate::memory::cstr_is_empty(p) {
                let mut has_trailing_path_sep = false;

                if plen == 0 {
                    plen = crate::memory::cstr_len(p);
                    if after_pathsep(p, p.wrapping_add(plen)) != 0 {
                        has_trailing_path_sep = true;
                    }
                }

                let itemsize = plen + usize::from(!has_trailing_path_sep) + 2;
                let mut item = vec![0 as c_char; itemsize];

                let itemlen = vim_snprintf!(
                    item.as_mut_ptr(),
                    itemsize,
                    c"%s%s*",
                    p,
                    if has_trailing_path_sep {
                        c"".as_ptr()
                    } else {
                        PATHSEPSTR
                    },
                ) as usize;

                if find_dup_item(
                    if buf.is_empty() {
                        ptr::null()
                    } else {
                        buf.as_ptr()
                    },
                    item.as_ptr(),
                    itemlen,
                    get_option(opt_idx).flags,
                )
                .is_null()
                {
                    if !buf.is_empty() {
                        // Replace trailing NUL with ','
                        let last = buf.len() - 1;
                        buf[last] = b',' as c_char;
                    }
                    buf.extend_from_slice(&item[..itemlen]);
                    buf.push(NUL);
                }
            }
        }
        if !buf.is_empty() {
            set_string_default(K_OPT_BACKUPSKIP, buf.as_ptr());
        }
    }
}

/// Initialize the 'cdpath' option to a default value.
fn set_init_default_cdpath() {
    unsafe {
        let Some(cdpath_cstr) = vim_getenv(c"CDPATH") else {
            return;
        };
        let cdpath = cdpath_cstr.as_ptr();

        let mut buf_vec = vec![0 as c_char; 2 * cdpath_cstr.as_bytes().len() + 2];
        let buf = buf_vec.as_mut_ptr();
        *buf = b',' as c_char;
        let mut j: usize = 1;
        let mut i: usize = 0;
        while crate::memory::cstr_byte_at(cdpath, i) != 0 {
            let ch = *cdpath.wrapping_add(i);
            if vim_ispathlistsep(ch as i32) {
                *buf.wrapping_add(j) = b',' as c_char;
                j += 1;
            } else {
                if ch == b' ' as c_char || ch == b',' as c_char {
                    *buf.wrapping_add(j) = b'\\' as c_char;
                    j += 1;
                }
                *buf.wrapping_add(j) = ch;
                j += 1;
            }
            i += 1;
        }
        *buf.wrapping_add(j) = NUL;
        change_option_default(K_OPT_CDPATH, cstr_to_obj(buf));
    }
}

/// Expand environment variables and things like "~" for the defaults.
fn set_init_expand_env() {
    unsafe {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            if (opt.flags & K_OPT_FLAG_NO_DEF_EXP) != 0 {
                continue;
            }
            let mut p: *mut c_char = ptr::null_mut();
            if (opt.flags & K_OPT_FLAG_GETTEXT) != 0 && !opt.var.is_null() {
                p = tr(*(opt.var.cast::<*mut c_char>()));
            } else if (opt.flags & K_OPT_FLAG_EXPAND) != 0 {
                p = option_expand(opt_idx, opt.def_val.data.string().data);
            }
            if !p.is_null() {
                let ov = cstr_to_obj(p);
                set_option_varp(opt_idx, opt.var, ov);
                change_option_default(opt_idx, ov);
            }
        }
    }
}

/// Initialize the encoding used for "default" in 'fileencodings'.
fn set_init_fenc_default() {
    g_st().fenc_default = Some(enc_locale().unwrap_or_else(|| c"utf-8".to_owned()));
}

/// Initialize the options, first part.
pub fn set_init_1(clean_arg: bool) {
    {
        langmap_init();

        alloc_options_default();

        set_init_default_shell();
        set_init_default_backupskip();
        set_init_default_cdpath();

        let raw_backupdir = stdpaths_user_state_subpath(c"backup", 2, true);
        let mut backupdir_bytes = Vec::with_capacity(raw_backupdir.as_bytes().len() + 2);
        backupdir_bytes.extend_from_slice(b".,");
        backupdir_bytes.extend_from_slice(raw_backupdir.as_bytes());
        let backupdir = crate::memory::cstr_from_bytes(&backupdir_bytes);
        set_string_default(K_OPT_BACKUPDIR, backupdir.as_ptr());
        set_string_default(
            K_OPT_VIEWDIR,
            stdpaths_user_state_subpath(c"view", 2, false).as_ptr(),
        );
        set_string_default(
            K_OPT_DIRECTORY,
            stdpaths_user_state_subpath(c"swap", 2, true).as_ptr(),
        );
        set_string_default(
            K_OPT_UNDODIR,
            stdpaths_user_state_subpath(c"undo", 2, true).as_ptr(),
        );
        set_string_default(
            K_OPT_PACKLOCKFILE,
            stdpaths_user_conf_subpath(c"nvim-pack-lock.json").as_ptr(),
        );
        if let Some(rtp) = runtimepath_default(clean_arg) {
            set_string_default(K_OPT_RUNTIMEPATH, rtp.as_ptr());
            set_string_default(K_OPT_PACKPATH, rtp.as_ptr());
        }

        set_options_default(0);

        curbuf_mut().b_p_initialized = true;
        curbuf_mut().b_p_ac = -1;
        curbuf_mut().b_p_ar = -1;
        curbuf_mut().b_p_fs = -1;
        curbuf_mut().b_p_ul = NO_LOCAL_UNDOLEVEL;
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

        let _ = bind_textdomain_codeset(c"nvim".as_ptr(), opt_vars().p_enc);

        set_helplang_default(get_mess_lang());
    }
}

/// Get default value for option, based on the option's type and scope.
pub fn get_option_default(opt_idx: OptIndex, opt_flags: i32) -> Object {
    {
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
        {
            let s = option_expand(opt_idx, def_val.data.string().data);
            if !s.is_null() {
                def_val = cstr_as_obj(s);
            }
        }
        if optval_is_owned(opt_idx) {
            copy_object(def_val, None)
        } else {
            def_val
        }
    }
}

/// Allocate the default values for all options by copying them from the stack.
fn alloc_options_default() {
    {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            opt.def_val = copy_object(opt.def_val, None);
        }
    }
}

/// Change the default value for an option.
fn change_option_default(opt_idx: OptIndex, value: Object) {
    {
        let opt = get_option(opt_idx);
        optval_free(opt.def_val);
        opt.def_val = value;
    }
}

/// Set an option to its default value.
fn set_option_default(opt_idx: OptIndex, opt_flags: i32) {
    {
        let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
        let def_val = get_option_default(opt_idx, opt_flags);
        set_option_direct(opt_idx, def_val, opt_flags, g_st().current_sctx.sc_sid);
        optval_free_read(opt_idx, def_val);

        if opt_idx == K_OPT_SCROLL {
            win_comp_scroll(curwin_mut());
        }

        let mut flagsp = insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, opt_flags);
        *flagsp &= !K_OPT_FLAG_INSECURE;
        if both {
            flagsp = insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, OPT_LOCAL);
            *flagsp &= !K_OPT_FLAG_INSECURE;
        }
    }
}

/// Set all options (except terminal options) to their default value.
fn set_options_default(opt_flags: i32) {
    {
        for opt_idx in 0..K_OPT_COUNT {
            if (get_option(opt_idx).flags & K_OPT_FLAG_NO_DEFAULT) == 0 {
                set_option_default(opt_idx, opt_flags);
            }
        }

        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_mut(&*tp) {
                win_comp_scroll(wp);
            }
        }

        parse_cino(curbuf_mut());
    }
}

/// Set the Vi-default value of a string option.
fn set_string_default(opt_idx: OptIndex, val: impl crate::strings::IntoCCharPtr) {
    let val = val.into_c_char_ptr();
    assert!(opt_idx != K_OPT_INVALID);
    change_option_default(opt_idx, cstr_to_obj(val));
}

/// For an option value that contains comma separated items, find "newval" in "origval".
fn find_dup_item(
    origval: impl crate::strings::IntoCCharPtr,
    newval: impl crate::strings::IntoCCharPtr,
    newvallen: usize,
    flags: u32,
) -> *const c_char {
    let origval = origval.into_c_char_ptr();
    let newval = newval.into_c_char_ptr();
    unsafe {
        if origval.is_null() {
            return ptr::null();
        }

        let mut bs: i32 = 0;
        let mut s = origval;
        while !crate::memory::cstr_is_empty(s) {
            if ((flags & K_OPT_FLAG_COMMA) == 0
                || s == origval
                || (crate::memory::cstr_first_byte(s.wrapping_offset(-1)) == b',' && (bs & 1) == 0))
                && (crate::memory::cstr_prefix_bytes(s, newvallen)
                    == crate::memory::cstr_prefix_bytes(newval, newvallen))
                && ((flags & K_OPT_FLAG_COMMA) == 0
                    || crate::memory::cstr_byte_at(s, newvallen) == b','
                    || crate::memory::cstr_byte_at(s, newvallen) == 0)
            {
                return s;
            }
            if (s > origval.wrapping_add(1)
                && *s.wrapping_offset(-1) == b'\\' as c_char
                && *s.wrapping_offset(-2) != b',' as c_char)
                || (s == origval.wrapping_add(1) && *s.wrapping_offset(-1) == b'\\' as c_char)
            {
                bs += 1;
            } else {
                bs = 0;
            }
            s = s.wrapping_add(1);
        }
        ptr::null()
    }
}

/// Free all options.
pub fn free_all_options() {
    unsafe {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            let hidden = is_option_hidden(opt_idx);

            if !hidden
                && !opt.var.is_null()
                && (option_is_global_only(opt_idx) || !option_is_window_local(opt_idx))
            {
                if is_callback_option(opt_idx) {
                    callback_free(&mut *opt.var.cast::<Callback>());
                } else if option_has_type(opt_idx, ObjectType::ObjectTypeString) {
                    crate::optionstr::clear_string_option(opt.var.cast::<*mut c_char>());
                }
            }
            optval_free(opt.def_val);
        }
        free_operatorfunc_option();
        free_tagfunc_option();
        free_findfunc_option();
        g_st().fenc_default = None;
        opt_st().p_term = None;
        opt_st().p_ttytype = None;
    }
}

/// Initialize the options, part two: After getting Rows and Columns.
pub fn set_init_2(_headless: bool) {
    {
        logmsg!(
            1,
            ptr::null(),
            c"set_init_2",
            592,
            true,
            c"startup runtimepath/packpath value: %s",
            opt_vars().p_rtp,
        );

        if (get_option(K_OPT_SCROLL).flags & K_OPT_FLAG_WAS_SET) == 0 {
            set_option_default(K_OPT_SCROLL, OPT_LOCAL);
        }
        comp_col();

        if !option_was_set(K_OPT_WINDOW) {
            opt_vars().p_window = OptInt::from(g_st().rows_global - 1);
        }
        change_option_default(
            K_OPT_WINDOW,
            integer_obj(OptInt::from(g_st().rows_global - 1)),
        );
    }
}

fn change_option_and_default_if_unset(idx: OptIndex, val: Option<&'static CStr>) {
    {
        let Some(val) = val else {
            return;
        };
        if (get_option(idx).flags & K_OPT_FLAG_WAS_SET) != 0 {
            return;
        }
        let optval = cstr_as_obj(val.as_ptr());
        set_option_direct(idx, optval, 0, SID_NONE);
        change_option_default(idx, copy_object(optval, None));
    }
}

/// Initialize the options, part three: After reading the .vimrc
pub fn set_init_3() {
    {
        parse_shape_opt(SHAPE_CURSOR);

        let mut len: usize = 0;
        let mut name = [0 as c_char; MAXPATHL];
        let p = invocation_path_tail(opt_vars().p_sh, Some(&mut len));
        crate::memory::cstr_ncpy(name.as_mut_ptr(), p, len);
        for (i, rule) in SHELL_RULES.iter().enumerate() {
            if !c_str_contains(name.as_ptr(), rule.pat.to_bytes()) {
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

        if buf_is_empty(curbuf_mut())
            && (get_option(K_OPT_FILEFORMATS).flags & K_OPT_FLAG_WAS_SET) != 0
        {
            set_fileformat(default_fileformat(), OPT_LOCAL);
        }

        set_title_defaults();
    }
}

/// When 'helplang' is still at its default value, set it to "lang".
pub fn set_helplang_default(lang: impl crate::strings::IntoCCharPtr) {
    let lang = lang.into_c_char_ptr();
    {
        if lang.is_null() {
            return;
        }

        let lang_len = crate::memory::cstr_len(lang);
        if lang_len < 2 {
            return;
        }
        if (get_option(K_OPT_HELPLANG).flags & K_OPT_FLAG_WAS_SET) != 0 {
            return;
        }

        free_string_option(opt_vars().p_hlg);
        let mut hlg_bytes = [
            crate::memory::cstr_first_byte(lang),
            crate::memory::cstr_byte_at(lang, 1),
        ];
        if lang_len >= 5
            && hlg_bytes[0].eq_ignore_ascii_case(&b'z')
            && hlg_bytes[1].eq_ignore_ascii_case(&b'h')
            && crate::memory::cstr_byte_at(lang, 2) == b'_'
        {
            hlg_bytes[0] = (crate::memory::cstr_byte_at(lang, 3)).to_ascii_lowercase();
            hlg_bytes[1] = (crate::memory::cstr_byte_at(lang, 4)).to_ascii_lowercase();
        } else if lang_len > 0 && crate::memory::cstr_first_byte(lang) == b'C' {
            hlg_bytes[0] = b'e';
            hlg_bytes[1] = b'n';
        }
        opt_vars().p_hlg = crate::optionstr::alloc_string_option_bytes(&hlg_bytes);
    }
}

pub fn set_title_defaults() {
    {
        if (get_option(K_OPT_TITLE).flags & K_OPT_FLAG_WAS_SET) == 0 {
            change_option_default(K_OPT_TITLE, boolean_obj(false));
            opt_vars().p_title = 0;
        }
        if (get_option(K_OPT_ICON).flags & K_OPT_FLAG_WAS_SET) == 0 {
            change_option_default(K_OPT_ICON, boolean_obj(false));
            opt_vars().p_icon = 0;
        }
    }
}

pub fn ex_set(eap: &mut Exarg) {
    let mut flags: i32 = 0;

    if eap.cmdidx == Cmdidx::CmdSetlocal as i32 {
        flags = OPT_LOCAL;
    } else if eap.cmdidx == Cmdidx::CmdSetglobal as i32 {
        flags = OPT_GLOBAL;
    }
    if eap.forceit != 0 {
        flags |= OPT_ONECOLUMN;
    }
    do_set(eap.arg, flags);
}

/// Copy the new string value into allocated memory for the option.
fn stropt_copy_value(
    origval: impl crate::strings::IntoCCharPtr,
    argp: *mut *mut c_char,
    op: SetOp,
    flags: u32,
) -> Vec<c_char> {
    let origval = origval.into_c_char_ptr();
    unsafe {
        let mut arg = *argp;

        let mut newlen = crate::memory::cstr_len(arg) + 1;
        if op != OP_NONE {
            newlen += crate::memory::cstr_len(origval) + 1;
        }
        let mut newval_vec: Vec<c_char> = vec![0; newlen];
        let newval = newval_vec.as_mut_ptr();
        let mut s = newval;

        while !crate::memory::cstr_is_empty(arg)
            && !ascii_iswhite(crate::memory::cstr_first_char(arg))
        {
            #[cfg(windows)]
            let remove_bs = crate::memory::cstr_first_byte(arg) == b'\\'
                && crate::memory::cstr_byte_at(arg, 1) != 0
                && !((flags & K_OPT_FLAG_EXPAND) != 0
                    && vim_isfilec(crate::memory::cstr_byte_at(arg, 1) as i32)
                    && !ascii_iswhite(crate::memory::cstr_char_at(arg, 1))
                    && (crate::memory::cstr_byte_at(arg, 1) != b'\\'
                        || (s == newval && crate::memory::cstr_byte_at(arg, 2) != b'\\')));
            #[cfg(not(windows))]
            let remove_bs = {
                let _ = flags;
                crate::memory::cstr_first_byte(arg) == b'\\'
                    && crate::memory::cstr_byte_at(arg, 1) != 0
            };
            if remove_bs {
                arg = arg.wrapping_add(1);
            }
            let i = utfc_ptr2len(arg);
            if i > 1 {
                ptr::copy(arg, s, i as usize);
                arg = arg.wrapping_add(i as usize);
                s = s.wrapping_add(i as usize);
            } else {
                *s = *arg;
                s = s.wrapping_add(1);
                arg = arg.wrapping_add(1);
            }
        }
        *s = NUL;

        *argp = arg;
        newval_vec
    }
}

/// Expand environment variables and ~ in string option value 'newval'.
fn stropt_expand_envvar(
    opt_idx: OptIndex,
    origval: impl crate::strings::IntoCCharPtr,
    newval: &mut Vec<c_char>,
    op: SetOp,
) {
    let origval = origval.into_c_char_ptr();
    {
        let s = option_expand(opt_idx, newval.as_mut_ptr());
        if s.is_null() {
            return;
        }

        let mut newlen = (crate::memory::cstr_len(s) as u32) + 1;
        if op != OP_NONE {
            newlen += (crate::memory::cstr_len(origval) as u32) + 1;
        }
        *newval = vec![0; newlen as usize];
        crate::memory::cstr_cpy(newval.as_mut_ptr(), s);
    }
}

/// Concatenate the original and new values of a string option, adding a "," if needed.
fn stropt_concat_with_comma(
    origval: impl crate::strings::IntoCCharPtr,
    newval: *mut c_char,
    op: SetOp,
    flags: u32,
) {
    let origval = origval.into_c_char_ptr();
    unsafe {
        let mut len: usize;
        let comma = usize::from(
            (flags & K_OPT_FLAG_COMMA) != 0
                && !crate::memory::cstr_is_empty(origval)
                && !crate::memory::cstr_is_empty(newval),
        );
        if op == OP_ADDING {
            len = crate::memory::cstr_len(origval);
            if comma != 0
                && len > 1
                && (flags & K_OPT_FLAG_ONE_COMMA) == K_OPT_FLAG_ONE_COMMA
                && crate::memory::cstr_byte_at(origval, len - 1) == b','
                && crate::memory::cstr_byte_at(origval, len - 2) != b'\\'
            {
                len -= 1;
            }
            ptr::copy(
                newval,
                newval.wrapping_add(len + comma),
                crate::memory::cstr_len(newval) + 1,
            );
            ptr::copy(origval, newval, len);
        } else {
            len = crate::memory::cstr_len(newval);
            ptr::copy(
                origval,
                newval.wrapping_add(len + comma),
                crate::memory::cstr_len(origval) + 1,
            );
        }
        if comma != 0 {
            *newval.wrapping_add(len) = b',' as c_char;
        }
    }
}

/// Remove a value from a string option.
fn stropt_remove_val(
    origval: impl crate::strings::IntoCCharPtr,
    newval: *mut c_char,
    flags: u32,
    strval: impl crate::strings::IntoCCharPtr,
    mut len: i32,
) {
    let origval = origval.into_c_char_ptr();
    let mut strval = strval.into_c_char_ptr();
    unsafe {
        crate::memory::cstr_cpy(newval, origval);
        if !crate::memory::cstr_is_empty(strval) {
            if (flags & K_OPT_FLAG_COMMA) != 0 {
                if strval == origval {
                    if crate::memory::cstr_byte_at(strval, len as usize) == b',' {
                        len += 1;
                    }
                } else {
                    strval = strval.wrapping_offset(-1);
                    len += 1;
                }
            }
            let src = strval.wrapping_add(len as usize);
            let dst_off = crate::memory::ptr_offset_from(strval, origval) as usize;
            ptr::copy(
                src,
                newval.wrapping_add(dst_off),
                crate::memory::cstr_len(src) + 1,
            );
        }
    }
}

/// Find a comma-separated item in "src" that matches the key part of "key".
fn find_key_item(
    src: impl crate::strings::IntoCCharPtr,
    key: impl crate::strings::IntoCCharPtr,
    keylen: isize,
    itemlenp: &mut isize,
) -> *mut c_char {
    let src = src.into_c_char_ptr().cast_mut();
    let key = key.into_c_char_ptr();
    {
        let mut p = src;

        while !crate::memory::cstr_is_empty(p) {
            if (p == src || crate::memory::cstr_first_byte(p.wrapping_offset(-1)) == b',')
                && (crate::memory::cstr_prefix_bytes(p, keylen as usize)
                    == crate::memory::cstr_prefix_bytes(key, keylen as usize))
            {
                let mut end = vim_strchr(p, b',' as i32);
                if end.is_null() {
                    end = p.wrapping_add(crate::memory::cstr_len(p));
                }
                *itemlenp = crate::memory::ptr_offset_from(end, p);
                return p;
            }
            p = p.wrapping_add(1);
        }
        ptr::null_mut()
    }
}

/// Remove one item of length "itemlen" at position "item" from comma-separated string "str" in-place.
fn remove_comma_item(
    str_arg: impl crate::strings::IntoCCharPtr,
    item: *mut c_char,
    itemlen: isize,
) {
    let str_arg = str_arg.into_c_char_ptr();
    unsafe {
        if crate::memory::cstr_first_byte(item.wrapping_offset(itemlen)) == b',' {
            let src = item.wrapping_offset(itemlen + 1);
            ptr::copy(src, item, crate::memory::cstr_len(src) + 1);
        } else if (item as *const c_char) > str_arg && *item.wrapping_offset(-1) == b',' as c_char {
            let src = item.wrapping_offset(itemlen);
            ptr::copy(
                src,
                item.wrapping_offset(-1),
                crate::memory::cstr_len(src) + 1,
            );
        } else {
            *item = NUL;
        }
    }
}

/// Remove all items matching "key" (with ':') from comma-separated string "str" in-place.
fn remove_key_item(
    str_arg: impl crate::strings::IntoCCharPtr,
    key: impl crate::strings::IntoCCharPtr,
    keylen: isize,
    skip: impl crate::strings::IntoCCharPtr,
) {
    let str_arg = str_arg.into_c_char_ptr().cast_mut();
    let key = key.into_c_char_ptr().cast_mut();
    let skip = skip.into_c_char_ptr();
    {
        let mut itemlen: isize = 0;
        loop {
            let mut found = find_key_item(str_arg, key, keylen, &mut itemlen);
            if found.is_null() {
                break;
            }
            if ptr::eq(found, skip) {
                let mut next = found.wrapping_offset(itemlen);
                if crate::memory::cstr_first_byte(next) == b',' {
                    next = next.wrapping_add(1);
                }
                found = find_key_item(next, key, keylen, &mut itemlen);
                if found.is_null() {
                    break;
                }
            }

            remove_comma_item(str_arg, found, itemlen);
        }
    }
}

/// Append a comma-separated item to the end of "str" in-place.
fn append_item(str_arg: *mut c_char, item: *mut c_char, item_len: isize) {
    unsafe {
        let mut len = crate::memory::cstr_len(str_arg) as isize;
        if len > 0 {
            *str_arg.wrapping_offset(len) = b',' as c_char;
            len += 1;
        }
        ptr::copy(item, str_arg.wrapping_offset(len), item_len as usize);
        *str_arg.wrapping_offset(len + item_len) = NUL;
    }
}

/// Prepend a comma-separated item to the beginning of "str" in-place.
fn prepend_item(str_arg: *mut c_char, item: *mut c_char, item_len: isize) {
    unsafe {
        let len = crate::memory::cstr_len(str_arg) as isize;
        let comma: isize = if len > 0 { 1 } else { 0 };

        ptr::copy(
            str_arg,
            str_arg.wrapping_offset(item_len + comma),
            (len + 1) as usize,
        );
        ptr::copy(item, str_arg, item_len as usize);
        if comma != 0 {
            *str_arg.wrapping_offset(item_len) = b',' as c_char;
        }
    }
}

/// For a P_COMMA option: process "key:value" items in "newval" individually.
fn stropt_handle_keymatch(
    origval: impl crate::strings::IntoCCharPtr,
    newval: *mut c_char,
    op: SetOp,
    _flags: u32,
) -> bool {
    let origval = origval.into_c_char_ptr();
    {
        if !crate::strings::vim_haschr(newval, b':' as i32)
            && !crate::strings::vim_haschr(newval, b',' as i32)
        {
            return false;
        }

        let newval_copy = crate::memory::cstr_to_cstring(newval);
        crate::memory::cstr_cpy(newval, origval);

        let mut item_start = newval_copy.as_ptr().cast_mut();
        loop {
            let p = vim_strchr(item_start, b',' as i32);
            let item_len = if p.is_null() {
                crate::memory::cstr_len(item_start) as isize
            } else {
                crate::memory::ptr_offset_from(p, item_start)
            };

            if item_len > 0 {
                let colon = vim_strchr(item_start, b':' as i32);
                if !colon.is_null() && colon < item_start.wrapping_offset(item_len) {
                    let keylen = crate::memory::ptr_offset_from(colon, item_start) + 1;

                    if op == OP_ADDING || op == OP_PREPENDING {
                        let mut old_itemlen: isize = 0;
                        let found = find_key_item(newval, item_start, keylen, &mut old_itemlen);
                        if !found.is_null() {
                            if old_itemlen == item_len
                                && (crate::memory::cstr_prefix_bytes(found, item_len as usize)
                                    == crate::memory::cstr_prefix_bytes(
                                        item_start,
                                        item_len as usize,
                                    ))
                            {
                                remove_key_item(newval, item_start, keylen, found);
                            } else {
                                remove_key_item(newval, item_start, keylen, ptr::null());
                                if op == OP_PREPENDING {
                                    prepend_item(newval, item_start, item_len);
                                } else {
                                    append_item(newval, item_start, item_len);
                                }
                            }
                        } else if op == OP_PREPENDING {
                            prepend_item(newval, item_start, item_len);
                        } else {
                            append_item(newval, item_start, item_len);
                        }
                    } else if op == OP_REMOVING {
                        remove_key_item(newval, item_start, keylen, ptr::null());
                    }
                } else if op == OP_ADDING || op == OP_PREPENDING {
                    let found =
                        find_dup_item(newval, item_start, item_len as usize, K_OPT_FLAG_COMMA);
                    if found.is_null() {
                        if op == OP_PREPENDING {
                            prepend_item(newval, item_start, item_len);
                        } else {
                            append_item(newval, item_start, item_len);
                        }
                    }
                } else if op == OP_REMOVING {
                    let found =
                        find_dup_item(newval, item_start, item_len as usize, K_OPT_FLAG_COMMA)
                            .cast_mut();
                    if !found.is_null() {
                        remove_comma_item(newval, found, item_len);
                    }
                }
            }

            if p.is_null() {
                break;
            }
            item_start = p.wrapping_add(1);
        }

        true
    }
}

/// Remove flags that appear twice in the string option value 'newval'.
fn stropt_remove_dupflags(newval: impl crate::strings::IntoCCharPtr, flags: u32) {
    let newval = newval.into_c_char_ptr().cast_mut();
    unsafe {
        let mut s = newval;
        while !crate::memory::cstr_is_empty(s) {
            if (flags & K_OPT_FLAG_ONE_COMMA) != 0 {
                if crate::memory::cstr_first_byte(s) != b','
                    && crate::memory::cstr_byte_at(s, 1) == b','
                    && crate::strings::vim_haschr(
                        s.wrapping_add(2),
                        crate::memory::cstr_first_byte(s) as i32,
                    )
                {
                    let src = s.wrapping_add(2);
                    ptr::copy(src, s, crate::memory::cstr_len(src) + 1);
                    continue;
                }
            } else if ((flags & K_OPT_FLAG_COMMA) == 0 || crate::memory::cstr_first_byte(s) != b',')
                && crate::strings::vim_haschr(
                    s.wrapping_add(1),
                    crate::memory::cstr_first_byte(s) as i32,
                )
            {
                let src = s.wrapping_add(1);
                ptr::copy(src, s, crate::memory::cstr_len(src) + 1);
                continue;
            }
            s = s.wrapping_add(1);
        }
    }
}

/// Get the string value specified for a ":set" command.
fn stropt_get_newval(
    opt_idx: OptIndex,
    argp: *mut *mut c_char,
    varp: *mut c_void,
    origval: impl crate::strings::IntoCCharPtr,
    op_arg: &mut SetOp,
) -> std::ffi::CString {
    let origval = origval.into_c_char_ptr();
    unsafe {
        let mut arg = *argp;
        let mut op = *op_arg;
        let mut save_arg: *mut c_char = ptr::null_mut();
        let mut s: *const c_char = ptr::null();
        let flags = get_option(opt_idx).flags;

        arg = arg.wrapping_add(1);

        if varp == (&raw mut opt_vars().p_kp).cast::<c_void>()
            && (crate::memory::cstr_is_empty(arg) || crate::memory::cstr_first_byte(arg) == b' ')
        {
            save_arg = arg;
            arg = c":help".as_ptr().cast_mut();
        }

        let mut newval_vec = stropt_copy_value(origval, &raw mut arg, op, flags);

        if op == OP_NONE || (flags & K_OPT_FLAG_COMMA) != 0 {
            stropt_expand_envvar(opt_idx, origval, &mut newval_vec, op);
        }
        let newval = newval_vec.as_mut_ptr();

        if (flags & K_OPT_FLAG_COMMA) != 0
            && (flags & K_OPT_FLAG_COLON) != 0
            && op != OP_NONE
            && stropt_handle_keymatch(origval, newval, op, flags)
        {
            // fully handled
        } else {
            let mut len: i32 = 0;
            if op == OP_REMOVING || (flags & K_OPT_FLAG_NO_DUP) != 0 {
                len = crate::memory::cstr_len(newval) as i32;
                s = find_dup_item(origval, newval, len as usize, flags);

                if (op == OP_ADDING || op == OP_PREPENDING) && !s.is_null() {
                    op = OP_NONE;
                    crate::memory::cstr_cpy(newval, origval);
                }

                if s.is_null() {
                    s = origval.wrapping_add(crate::memory::cstr_len(origval));
                }
            }

            if op == OP_ADDING || op == OP_PREPENDING {
                stropt_concat_with_comma(origval, newval, op, flags);
            } else if op == OP_REMOVING {
                stropt_remove_val(origval, newval, flags, s, len);
            }
        }

        if (flags & K_OPT_FLAG_FLAG_LIST) != 0 {
            stropt_remove_dupflags(newval, flags);
        }

        if !save_arg.is_null() {
            arg = save_arg;
        }
        *argp = arg;
        *op_arg = op;

        crate::memory::cstr_to_cstring(newval)
    }
}

fn get_op(arg: impl crate::strings::IntoCCharPtr) -> SetOp {
    let arg = arg.into_c_char_ptr();
    {
        let mut op = OP_NONE;
        if !crate::memory::cstr_is_empty(arg) && crate::memory::cstr_byte_at(arg, 1) == b'=' {
            if crate::memory::cstr_first_byte(arg) == b'+' {
                op = OP_ADDING;
            } else if crate::memory::cstr_first_byte(arg) == b'^' {
                op = OP_PREPENDING;
            } else if crate::memory::cstr_first_byte(arg) == b'-' {
                op = OP_REMOVING;
            }
        }
        op
    }
}

fn get_option_prefix(argp: *mut *mut c_char) -> SetPrefix {
    unsafe {
        if crate::memory::cstr_prefix_bytes(*argp, 2) == &b"no"[..2] {
            *argp = (*argp).wrapping_add(2);
            PREFIX_NO
        } else if crate::memory::cstr_prefix_bytes(*argp, 3) == &b"inv"[..3] {
            *argp = (*argp).wrapping_add(3);
            PREFIX_INV
        } else {
            PREFIX_NONE
        }
    }
}

fn validate_opt_idx(
    win: &mut WinInfoInfo,
    opt_idx: OptIndex,
    opt_flags: i32,
    flags: u32,
    prefix: SetPrefix,
    errmsg: *mut *const c_char,
) -> bool {
    unsafe {
        if !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) && prefix != PREFIX_NONE {
            *errmsg = E_INVARG.as_ptr();
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
                *errmsg = E_NOT_ALLOWED_IN_MODELINE;
                return false;
            }
            if (flags & K_OPT_FLAG_MLE) != 0 && opt_vars().p_mle == 0 {
                *errmsg = E_NOT_ALLOWED_IN_MODELINE_WHEN_MODELINEEXPR_IS_OFF;
                return false;
            }
            if win.w_onebuf_opt.wo_diff != 0
                && (opt_idx == K_OPT_FOLDMETHOD || opt_idx == K_OPT_WRAP)
            {
                return false;
            }
        }

        if g_st().sandbox != 0 && (flags & K_OPT_FLAG_SECURE) != 0 {
            *errmsg = E_SANDBOX.as_ptr();
            return false;
        }

        true
    }
}

/// Skip over the name of a TTY option or keycode option.
fn find_tty_option_end(arg: impl crate::strings::IntoCCharPtr) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        if crate::memory::cstr_as_opt_bytes(arg) == Some(b"term") {
            return arg.wrapping_add("term".len());
        } else if crate::memory::cstr_as_opt_bytes(arg) == Some(b"ttytype") {
            return arg.wrapping_add("ttytype".len());
        }

        let mut p = arg;
        let mut delimit = false;

        if crate::memory::cstr_first_byte(arg) == b'<' {
            delimit = true;
            p = p.wrapping_add(1);
        }
        if crate::memory::cstr_first_byte(p) == b't'
            && crate::memory::cstr_byte_at(p, 1) == b'_'
            && crate::memory::cstr_byte_at(p, 2) != 0
            && crate::memory::cstr_byte_at(p, 3) != 0
        {
            p = p.wrapping_add(4);
        } else if delimit {
            while !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b'>' {
                p = p.wrapping_add(1);
            }
        }
        if delimit {
            if crate::memory::cstr_first_byte(p) != b'>' {
                return ptr::null();
            }
            p = p.wrapping_add(1);
        }

        if arg == p { ptr::null() } else { p }
    }
}

/// Skip over the name of an option.
pub fn find_option_end(
    arg: impl crate::strings::IntoCCharPtr,
    opt_idxp: &mut OptIndex,
) -> *const c_char {
    let arg = arg.into_c_char_ptr();
    {
        let mut p = find_tty_option_end(arg);
        if !p.is_null() {
            *opt_idxp = K_OPT_INVALID;
            return p;
        }
        p = arg;

        if !ascii_isalpha(crate::memory::cstr_first_char(p)) {
            *opt_idxp = K_OPT_INVALID;
            return ptr::null();
        }
        while ascii_isalpha(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
        }

        *opt_idxp = find_option_len(arg, crate::memory::ptr_offset_from(p, arg) as usize);
        p
    }
}

/// Get new option value from argp. Allocated Object must be freed by caller.
pub fn get_option_newval(
    opt_idx: OptIndex,
    opt_flags: i32,
    prefix: SetPrefix,
    argp: *mut *mut c_char,
    nextchar: i32,
    mut op: SetOp,
    varp: *mut c_void,
    mut oldval_override: Option<&mut Object>,
    errmsg: *mut *const c_char,
) -> Object {
    unsafe {
        assert!(!varp.is_null());

        let opt = get_option(opt_idx);
        let mut arg = *argp;
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
            *oldval_override.as_deref_mut().unwrap()
        } else {
            let oldval_is_global = option_is_global_local(opt_idx) && (opt_flags & OPT_LOCAL) != 0;
            optval_own(
                opt_idx,
                opt_from_varp(
                    opt_idx,
                    if oldval_is_global {
                        get_varp(&mut *opt)
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
                    *(varp.cast::<i32>()) ^ 1
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

                arg = arg.wrapping_add(1);
                if (varp.cast::<OptInt>() == &raw mut opt_vars().p_wc
                    || varp.cast::<OptInt>() == &raw mut opt_vars().p_wcm)
                    && (crate::memory::cstr_first_byte(arg) == b'<'
                        || crate::memory::cstr_first_byte(arg) == b'^'
                        || (!crate::memory::cstr_is_empty(arg)
                            && (crate::memory::cstr_byte_at(arg, 1) == 0
                                || ascii_iswhite(crate::memory::cstr_char_at(arg, 1)))
                            && !ascii_isdigit(crate::memory::cstr_first_char(arg))))
                {
                    newval_num = OptInt::from(string_to_key(arg));
                    if newval_num == 0 {
                        *errmsg = E_INVARG.as_ptr();
                        if oldval_override.is_none() {
                            optval_free(oldval);
                        }
                        return newval;
                    }
                } else if crate::memory::cstr_first_byte(arg) == b'-'
                    || ascii_isdigit(crate::memory::cstr_first_char(arg))
                {
                    let mut i: i32 = 0;
                    vim_str2nr(
                        arg,
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
                        || (crate::memory::cstr_byte_at(arg, i as usize) != 0
                            && !ascii_iswhite(crate::memory::cstr_char_at(arg, i as usize)))
                    {
                        *errmsg = E_NUMBER_REQUIRED_AFTER_EQUAL;
                        if oldval_override.is_none() {
                            optval_free(oldval);
                        }
                        return newval;
                    }
                } else {
                    *errmsg = E_NUMBER_REQUIRED_AFTER_EQUAL;
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
                let oldval_str = oldval.data.string().data;
                let newval_str = stropt_get_newval(opt_idx, argp, varp, oldval_str, &mut op);
                newval = cstr_to_obj(newval_str.as_ptr());
            }
            ObjectType::ObjectTypeLuaRef => {
                let newval_str = stropt_get_newval(opt_idx, argp, varp, c"", &mut op);
                newval = cstr_to_obj(newval_str.as_ptr());
            }
            _ => std::process::abort(),
        }

        if oldval_override.is_none() {
            optval_free(oldval);
        }
        newval
    }
}

fn do_one_set_option(
    opt_flags: i32,
    argp: *mut *mut c_char,
    did_show: &mut bool,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let prefix = get_option_prefix(argp);
        let arg = *argp;

        let mut opt_idx: OptIndex = K_OPT_INVALID;
        let option_end = find_option_end(arg, &mut opt_idx);

        if opt_idx != K_OPT_INVALID {
            assert!(option_end >= arg as *const c_char);
        } else if is_tty_option(arg) {
            return ptr::null();
        } else {
            return E_UNKNOWN_OPTION;
        }

        let afterchar = *option_end;
        let mut p = option_end.cast_mut();

        while ascii_iswhite(crate::memory::cstr_first_char(p)) {
            p = p.wrapping_add(1);
        }

        let op = get_op(p);
        if op != OP_NONE {
            p = p.wrapping_add(1);
        }

        let nextchar = crate::memory::cstr_first_byte(p) as i32;
        let opt = get_option(opt_idx);
        let flags = opt.flags;
        let varp = get_varp_scope(opt, opt_flags);

        let mut errmsg: *const c_char = ptr::null();
        if !validate_opt_idx(curwin_mut(), opt_idx, opt_flags, flags, prefix, &mut errmsg) {
            return errmsg;
        }

        if crate::strings::vim_haschr(c"?=:!&<", nextchar) {
            *argp = p;

            if nextchar == b'&' as i32
                && crate::memory::cstr_byte_at(*argp, 1) == b'v'
                && crate::memory::cstr_byte_at(*argp, 2) == b'i'
            {
                if crate::memory::cstr_byte_at(*argp, 3) == b'm' {
                    *argp = (*argp).wrapping_add(3);
                } else {
                    *argp = (*argp).wrapping_add(2);
                }
            }
            if crate::strings::vim_haschr(c"?!&<", nextchar)
                && crate::memory::cstr_byte_at(*argp, 1) != 0
                && !ascii_iswhite(crate::memory::cstr_char_at(*argp, 1))
            {
                return E_TRAILING.as_ptr();
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
            showoneopt(&mut *opt, opt_flags);

            if opt_vars().p_verbose > 0 {
                if varp == opt.var {
                    last_set_msg(opt.script_ctx);
                } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
                    last_set_msg(
                        curwin_mut().w_onebuf_opt.wo_script_ctx
                            [option_scope_idx(opt_idx, K_OPT_SCOPE_WIN) as usize],
                    );
                } else if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
                    last_set_msg(
                        curbuf_mut().b_p_script_ctx
                            [option_scope_idx(opt_idx, K_OPT_SCOPE_BUF) as usize],
                    );
                }
            }

            if nextchar != b'?' as i32 && nextchar != NUL as i32 && !ascii_iswhite(afterchar) {
                return E_TRAILING.as_ptr();
            }
            return ptr::null();
        }

        if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
            if crate::strings::vim_haschr(c"=:", nextchar) {
                return E_INVARG.as_ptr();
            }

            if !crate::strings::vim_haschr(c"!&<", nextchar)
                && nextchar != NUL as i32
                && !ascii_iswhite(afterchar)
            {
                return E_TRAILING.as_ptr();
            }
        } else if !crate::strings::vim_haschr(c"=:&<", nextchar) {
            return E_INVARG.as_ptr();
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

        if newval.obj_type == ObjectType::ObjectTypeNil || !errmsg.is_null() {
            return errmsg;
        }

        set_option(opt_idx, newval, opt_flags, 0, false, op == OP_NONE, errbuf)
    }
}

/// Parse 'arg' for option settings.
pub fn do_set(mut arg: *mut c_char, opt_flags: i32) -> bool {
    unsafe {
        let mut did_show = false;

        if crate::memory::cstr_is_empty(arg) {
            showoptions(false, opt_flags);
            did_show = true;
        } else {
            while !crate::memory::cstr_is_empty(arg) {
                if (crate::memory::cstr_prefix_bytes(arg, 3) == &b"all"[..3])
                    && !ascii_isalpha(crate::memory::cstr_char_at(arg, 3))
                    && (opt_flags & OPT_MODELINE) == 0
                {
                    arg = arg.wrapping_add(3);
                    if crate::memory::cstr_first_byte(arg) == b'&' {
                        arg = arg.wrapping_add(1);
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
                    let mut errbuf_storage = [0 as c_char; ERR_BUFLEN];
                    let errbuf = CharBuf {
                        data: errbuf_storage.as_mut_ptr(),
                        size: ERR_BUFLEN,
                    };
                    let errmsg =
                        do_one_set_option(opt_flags, &raw mut arg, &mut did_show, Some(errbuf));

                    for _ in 0..2 {
                        arg = skiptowhite_esc(arg);
                        arg = skipwhite(arg);
                        if crate::memory::cstr_first_byte(arg) != b'=' {
                            break;
                        }
                    }

                    if !errmsg.is_null() {
                        let iobuf_ptr = g_st().iobuff.as_mut_ptr();
                        let i = (vim_snprintf!(iobuf_ptr, IOSIZE, c"%s", tr(errmsg)) + 2) as isize;
                        let arg_len = crate::memory::ptr_offset_from(arg, startarg);
                        if (i + arg_len) < IOSIZE as isize {
                            xstrlcpy(
                                iobuf_ptr.wrapping_offset(i - 2),
                                c": ",
                                (IOSIZE as isize - i + 2) as usize,
                            );
                            assert!(arg >= startarg);
                            ptr::copy(startarg, iobuf_ptr.wrapping_offset(i), arg_len as usize);
                            *iobuf_ptr.wrapping_offset(i + arg_len) = NUL;
                        }
                        trans_characters(iobuf_ptr, IOSIZE as i32);

                        g_st().no_wait_return += 1;
                        emsg(iobuf_ptr);
                        g_st().no_wait_return -= 1;

                        return false;
                    }
                }

                arg = skipwhite(arg);
            }
        }

        if g_st().silent_mode && did_show {
            g_st().silent_mode = false;
            g_st().info_message = true;
            msg_putchar(b'\n' as i32);
            g_st().silent_mode = true;
            g_st().info_message = false;
        }

        true
    }
}

fn find_key_len(arg_arg: impl crate::strings::IntoCCharPtr, len: usize, has_lt: bool) -> i32 {
    let arg_arg = arg_arg.into_c_char_ptr();
    {
        let mut key: i32 = 0;
        let mut arg = arg_arg;

        if len >= 4
            && crate::memory::cstr_first_byte(arg) == b't'
            && crate::memory::cstr_byte_at(arg, 1) == b'_'
        {
            if !has_lt || crate::memory::cstr_byte_at(arg, 4) == b'>' {
                key = termcap2key(
                    crate::memory::cstr_byte_at(arg, 2) as i32,
                    crate::memory::cstr_byte_at(arg, 3) as i32,
                );
            }
        } else if has_lt {
            arg = arg.wrapping_offset(-1);
            let mut modifiers: i32 = 0;
            key = find_special_key(
                &raw mut arg,
                len + 1,
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
}

/// Convert a key name or string into a key value.
pub fn string_to_key(arg: impl crate::strings::IntoCCharPtr) -> i32 {
    let arg = arg.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(arg) == b'<' && crate::memory::cstr_byte_at(arg, 1) != 0 {
            return find_key_len(arg.wrapping_add(1), crate::memory::cstr_len(arg), true);
        }
        if crate::memory::cstr_first_byte(arg) == b'^' && crate::memory::cstr_byte_at(arg, 1) != 0 {
            let mut key = ctrl_chr(i32::from(crate::memory::cstr_byte_at(arg, 1)));
            if key == 0 {
                key = K_ZERO;
            }
            return key;
        }
        crate::memory::cstr_first_byte(arg) as i32
    }
}

pub fn did_set_title() {
    {
        if starting_val() != NO_SCREEN {
            maketitle();
        }
    }
}

/// set_options_bin - called when 'bin' changes value.
pub fn set_options_bin(oldval: i32, newval: i32, opt_flags: i32) {
    {
        if newval != 0 {
            if oldval == 0 {
                if (opt_flags & OPT_GLOBAL) == 0 {
                    curbuf_mut().b_p_tw_nobin = curbuf_mut().b_p_tw;
                    curbuf_mut().b_p_wm_nobin = curbuf_mut().b_p_wm;
                    curbuf_mut().b_p_ml_nobin = curbuf_mut().b_p_ml;
                    curbuf_mut().b_p_et_nobin = curbuf_mut().b_p_et;
                }
                if (opt_flags & OPT_LOCAL) == 0 {
                    opt_st().p_tw_nobin = opt_vars().p_tw;
                    opt_st().p_wm_nobin = opt_vars().p_wm;
                    opt_st().p_ml_nobin = opt_vars().p_ml;
                    opt_st().p_et_nobin = opt_vars().p_et;
                }
            }

            if (opt_flags & OPT_GLOBAL) == 0 {
                curbuf_mut().b_p_tw = 0;
                curbuf_mut().b_p_wm = 0;
                curbuf_mut().b_p_ml = 0;
                curbuf_mut().b_p_et = 0;
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                opt_vars().p_tw = 0;
                opt_vars().p_wm = 0;
                opt_vars().p_ml = 0;
                opt_vars().p_et = 0;
                opt_vars().p_bin = 1;
            }
        } else if oldval != 0 {
            if (opt_flags & OPT_GLOBAL) == 0 {
                curbuf_mut().b_p_tw = curbuf_mut().b_p_tw_nobin;
                curbuf_mut().b_p_wm = curbuf_mut().b_p_wm_nobin;
                curbuf_mut().b_p_ml = curbuf_mut().b_p_ml_nobin;
                curbuf_mut().b_p_et = curbuf_mut().b_p_et_nobin;
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                opt_vars().p_tw = opt_st().p_tw_nobin;
                opt_vars().p_wm = opt_st().p_wm_nobin;
                opt_vars().p_ml = opt_st().p_ml_nobin;
                opt_vars().p_et = opt_st().p_et_nobin;
            }
        }

        didset_options_sctx(opt_flags, P_BIN_DEP_OPTS);
    }
}

/// Expand environment variables for some string options.
fn option_expand(opt_idx: OptIndex, val: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut val = val.into_c_char_ptr();
    unsafe {
        let opt = get_option(opt_idx);
        if (opt.flags & K_OPT_FLAG_EXPAND) == 0 || is_option_hidden(opt_idx) {
            return ptr::null_mut();
        }

        if val.is_null() {
            val = *(opt.var.cast::<*const c_char>());
        }

        if val.is_null() || crate::memory::cstr_len(val) > MAXPATHL {
            return ptr::null_mut();
        }

        let var = opt.var.cast::<*mut c_char>();
        let esc = var == &raw mut opt_vars().p_tags || var == &raw mut opt_vars().p_path;
        let namebuf_ptr = g_st().name_buff.as_mut_ptr();
        expand_env_esc(
            val,
            namebuf_ptr,
            MAXPATHL as i32,
            if esc {
                c" \t".as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
            false,
            if var == &raw mut opt_vars().p_sps {
                c"file:".as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
        );
        if crate::memory::cstr_as_bytes(namebuf_ptr) == crate::memory::cstr_as_bytes(val) {
            return ptr::null_mut();
        }

        namebuf_ptr
    }
}

/// After setting various option values: recompute variables that depend on option values.
fn didset_options() {
    {
        buf_init_isk_chartab(curbuf_mut());
        init_isf_chartab();
        init_isi_chartab();

        didset_string_options();

        let mut sps_flags: i32 = 0;
        let mut sps_limit: i32 = 0;
        if spell_check_sps(opt_vars().p_sps, &mut sps_flags, &mut sps_limit) {
            spell_apply_sps(sps_flags, sps_limit);
        }
        let ws = curwin_mut().w_s_ptr();
        compile_cap_prog(ws.b_p_spc, &mut ws.b_cap_prog);
        did_set_spell_option();
        let mut dummy = Optset::new();
        did_set_cedit(&mut dummy);
        did_set_breakat(&mut dummy);
        didset_window_options(curwin_mut(), true);
    }
}

fn didset_options2() {
    {
        highlight_changed();

        set_chars_option(
            curwin_mut(),
            curwin_mut().w_onebuf_opt.wo_fcs,
            CharsOption::Fillchars,
            true,
            None,
        );
        set_chars_option(
            curwin_mut(),
            curwin_mut().w_onebuf_opt.wo_lcs,
            CharsOption::Listchars,
            true,
            None,
        );

        check_opt_wim(opt_vars().p_wim, (&raw mut g_st().wim_flags).cast::<u8>());
        tabstop_set(
            curbuf_mut().b_p_vsts,
            &mut curbuf_mut().b_p_vsts_array,
            None,
        );
        tabstop_set(curbuf_mut().b_p_vts, &mut curbuf_mut().b_p_vts_array, None);
    }
}

/// Repair UI state after `:set all&`.
fn didset_options_all() {
    let errmsg = parse_shape_opt(SHAPE_CURSOR);
    assert!(errmsg.is_null());
    last_status(false);
    win_float_update_statusline();
    win_new_screen_rows();
}

/// Check for string options that are NULL (normally only termcap options).
pub fn check_options() {
    {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            if option_has_type(opt_idx, ObjectType::ObjectTypeString)
                && !is_callback_option(opt_idx)
                && !opt.immutable
                && !opt.var.is_null()
            {
                check_string_option(get_varp(&mut *opt).cast::<*mut c_char>());
            }
        }
    }
}

/// Check if option was set insecurely.
pub fn was_set_insecurely(wp: Option<&mut WinInfoInfo>, opt_idx: OptIndex, opt_flags: i32) -> i32 {
    {
        assert!(opt_idx != K_OPT_INVALID);
        let flagp = insecure_flag(wp, opt_idx, opt_flags);
        i32::from((*flagp & K_OPT_FLAG_INSECURE) != 0)
    }
}

/// Get a pointer to the flags used for the K_OPT_FLAG_INSECURE flag of option "opt_idx".
pub fn insecure_flag(
    wp: Option<&mut WinInfoInfo>,
    opt_idx: OptIndex,
    opt_flags: i32,
) -> &'static mut u32 {
    unsafe {
        if (opt_flags & OPT_LOCAL) != 0 {
            let wp: *mut WinInfoInfo = wp.unwrap();
            let wp = { &mut *wp };
            match opt_idx {
                K_OPT_WRAP => return &mut wp.w_onebuf_opt.wo_wrap_flags,
                K_OPT_STATUSLINE => return &mut wp.w_onebuf_opt.wo_stl_flags,
                K_OPT_WINBAR => return &mut wp.w_onebuf_opt.wo_wbr_flags,
                K_OPT_FOLDEXPR => return &mut wp.w_onebuf_opt.wo_fde_flags,
                K_OPT_FOLDTEXT => return &mut wp.w_onebuf_opt.wo_fdt_flags,
                K_OPT_INDENTEXPR => return &mut (*wp.w_buffer).b_p_inde_flags,
                K_OPT_FORMATEXPR => return &mut (*wp.w_buffer).b_p_fex_flags,
                K_OPT_INCLUDEEXPR => return &mut (*wp.w_buffer).b_p_inex_flags,
                K_OPT_COMPLETE => return &mut (*wp.w_buffer).b_p_cpt_flags,
                _ => {}
            }
        } else {
            match opt_idx {
                K_OPT_WRAP => {
                    let wp: *mut WinInfoInfo = wp.unwrap();
                    let wp = { &mut *wp };
                    return &mut wp.w_allbuf_opt.wo_wrap_flags;
                }
                K_OPT_FOLDEXPR => {
                    let wp: *mut WinInfoInfo = wp.unwrap();
                    return &mut (*wp).w_allbuf_opt.wo_fde_flags;
                }
                K_OPT_FOLDTEXT => {
                    let wp: *mut WinInfoInfo = wp.unwrap();
                    return &mut (*wp).w_allbuf_opt.wo_fdt_flags;
                }
                _ => {}
            }
        }
        &mut get_option(opt_idx).flags
    }
}

/// Redraw the window title and/or tab page text later.
pub fn redraw_titles() {
    g_st().need_maketitle = true;
    g_st().redraw_tabline = true;
}

/// Return true if "val" is a valid name.
pub fn valid_name(val: impl crate::strings::IntoCCharPtr, allowed: &std::ffi::CStr) -> bool {
    let val = val.into_c_char_ptr();
    {
        let s = val;
        for &s_byte in crate::memory::cstr_as_bytes(s) {
            if !ascii_isalnum(s_byte as c_char as i32)
                && !crate::strings::vim_haschr(allowed.as_ptr(), s_byte as i32)
            {
                return false;
            }
        }
        true
    }
}

pub fn check_blending(wp: &mut WinInfoInfo) {
    wp.w_grid_alloc.blending =
        wp.w_onebuf_opt.wo_winbl > 0 || (wp.w_floating && wp.w_config.shadow);
}

/// Handle setting 'winhighlight' in window "wp".
pub fn parse_winhl_opt(
    winhl: impl crate::strings::IntoCCharPtr,
    mut wp: Option<&mut WinInfoInfo>,
) -> bool {
    let winhl = winhl.into_c_char_ptr();
    unsafe {
        let mut p: *const c_char = EMPTY_STRING_OPTION.as_ptr().cast();
        if !winhl.is_null() {
            p = winhl;
        } else if wp.is_some() {
            p = wp.as_deref_mut().unwrap().w_onebuf_opt.wo_winhl;
        }

        if crate::memory::cstr_is_empty(p) {
            if wp.is_some()
                && wp.as_deref_mut().unwrap().w_ns_hl_winhl > 0
                && wp.as_deref_mut().unwrap().w_ns_hl == wp.as_deref_mut().unwrap().w_ns_hl_winhl
            {
                wp.as_deref_mut().unwrap().w_ns_hl = 0;
                wp.as_deref_mut().unwrap().w_hl_needs_update = 1;
            }
            return true;
        }

        let mut ns_hl: i32 = 0;
        if wp.is_some() {
            if wp.as_deref_mut().unwrap().w_ns_hl_winhl == 0 {
                wp.as_deref_mut().unwrap().w_ns_hl_winhl = nvim_create_namespace(NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                }) as i32;
            } else {
                let dp = get_decor_provider(wp.as_deref_mut().unwrap().w_ns_hl_winhl, true);
                (*dp).hl_valid += 1;
            }
            ns_hl = wp.as_deref_mut().unwrap().w_ns_hl_winhl;
            if wp.as_deref_mut().unwrap().w_ns_hl <= 0 {
                wp.as_deref_mut().unwrap().w_ns_hl = wp.as_deref_mut().unwrap().w_ns_hl_winhl;
            }
        }

        let mut key: *const c_char = ptr::null();
        let mut val: *const c_char = ptr::null();
        let mut keylen: usize = 0;
        let mut vallen: usize = 0;
        while option_next_keyval(
            &raw mut p,
            &raw mut key,
            &mut keylen,
            &raw mut val,
            &mut vallen,
        ) {
            if val.is_null() {
                return false;
            }
            let hl_id = if vallen != 0 {
                syn_check_group(val, vallen)
            } else {
                -1
            };
            if hl_id == 0 {
                return false;
            }
            let hl_id_link = if keylen != 0 {
                syn_check_group(key, keylen)
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
            wp.w_hl_needs_update = 1;
        }
        true
    }
}

/// Get the script context of global option at index opt_idx.
pub fn get_option_sctx(opt_idx: OptIndex) -> &'static mut Sctx {
    assert!(opt_idx != K_OPT_INVALID);
    &mut get_option(opt_idx).script_ctx
}

/// Set the script_ctx for an option.
pub fn set_option_sctx(opt_idx: OptIndex, opt_flags: i32, mut script_ctx: Sctx) {
    {
        let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;

        if (opt_flags & OPT_MODELINE) == 0 {
            script_ctx.sc_lnum += sourcing_lnum();
        }
        nlua_set_sctx(&mut script_ctx);

        if both || (opt_flags & OPT_GLOBAL) != 0 || option_is_global_only(opt_idx) {
            get_option(opt_idx).script_ctx = script_ctx;
        }
        if both || (opt_flags & OPT_LOCAL) != 0 {
            if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
                curbuf_mut().b_p_script_ctx[option_scope_idx(opt_idx, K_OPT_SCOPE_BUF) as usize] =
                    script_ctx;
            } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
                let win_idx = option_scope_idx(opt_idx, K_OPT_SCOPE_WIN) as usize;
                curwin_mut().w_onebuf_opt.wo_script_ctx[win_idx] = script_ctx;
                if both {
                    curwin_mut().w_allbuf_opt.wo_script_ctx[win_idx] = script_ctx;
                }
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
    {
        if starting_val() != 0 || !crate::memory::cstr_is_empty(get_vim_var_str(VV_OPTION_TYPE)) {
            return;
        }

        let mut buf_type = [0 as c_char; 7];
        let mut oldval_tv = opt_to_tv(oldval, false);
        let mut oldval_g_tv = opt_to_tv(oldval_g, false);
        let mut oldval_l_tv = opt_to_tv(oldval_l, false);
        let mut newval_tv = opt_to_tv(newval, false);

        set_vim_var_tv(VV_OPTION_OLD, &mut oldval_tv);
        set_vim_var_tv(VV_OPTION_NEW, &mut newval_tv);
        let typelen = vim_snprintf_safelen!(
            buf_type.as_mut_ptr(),
            buf_type.len(),
            c"%s",
            if (opt_flags & OPT_LOCAL) != 0 {
                c"local".as_ptr()
            } else {
                c"global".as_ptr()
            },
        );
        set_vim_var_string(VV_OPTION_TYPE, buf_type.as_ptr(), typelen as isize);
        if (opt_flags & OPT_LOCAL) != 0 {
            set_vim_var_string(VV_OPTION_COMMAND, c"setlocal", "setlocal".len() as isize);
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_tv);
        }
        if (opt_flags & OPT_GLOBAL) != 0 {
            set_vim_var_string(VV_OPTION_COMMAND, c"setglobal", "setglobal".len() as isize);
            set_vim_var_tv(VV_OPTION_OLDGLOBAL, &mut oldval_tv);
        }
        if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0 {
            set_vim_var_string(VV_OPTION_COMMAND, c"set", "set".len() as isize);
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_l_tv);
            set_vim_var_tv(VV_OPTION_OLDGLOBAL, &mut oldval_g_tv);
        }
        if (opt_flags & OPT_MODELINE) != 0 {
            set_vim_var_string(VV_OPTION_COMMAND, c"modeline", "modeline".len() as isize);
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &mut oldval_tv);
        }
        apply_autocmds(
            EVENT_OPTIONSET,
            get_option(opt_idx).fullname.as_ptr().cast_mut(),
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        reset_v_option_vars();

        tv_clear(&mut oldval_tv);
        tv_clear(&mut oldval_g_tv);
        tv_clear(&mut oldval_l_tv);
        tv_clear(&mut newval_tv);
    }
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
    {
        if starting_val() != 0 {
            return;
        }
        if opt_idx == K_OPT_MODIFIED {
            aucmd_defer_modified(curbuf_mut(), newval.data.boolean());
            return;
        }
        apply_optionset_autocmd_now(opt_idx, opt_flags, oldval, oldval_g, oldval_l, newval);
    }
}

/// Process the updated 'arabic' option value.
pub fn did_set_arabic(args: &mut Optset) -> *const c_char {
    {
        let win = args.os_win;
        let win = unsafe { &mut *win };
        let mut errmsg: *const c_char = ptr::null();

        if win.w_onebuf_opt.wo_arab != 0 {
            if opt_vars().p_tbidi == 0 {
                if win.w_onebuf_opt.wo_rl == 0 {
                    win.w_onebuf_opt.wo_rl = 1;
                    changed_window_setting(&mut *win);
                }

                if opt_vars().p_arshape == 0 {
                    opt_vars().p_arshape = 1;
                    redraw_all_later(UPD_NOT_VALID);
                }
            }

            if crate::memory::cstr_as_bytes(opt_vars().p_enc) != b"utf-8" {
                msg_source(HLF_W);
                msg(tr(W_ARABIC), HLF_W);
                set_vim_var_string(VV_WARNINGMSG, tr(W_ARABIC), -1);
            }

            opt_vars().p_deco = 1;

            errmsg = set_option_value(
                K_OPT_KEYMAP,
                string_obj(NvimString {
                    data: c"arabic".as_ptr().cast_mut(),
                    size: "arabic".len(),
                }),
                OPT_LOCAL,
                true,
                args.os_errbuf,
            );
        } else {
            if opt_vars().p_tbidi == 0 && win.w_onebuf_opt.wo_rl != 0 {
                win.w_onebuf_opt.wo_rl = 0;
                changed_window_setting(&mut *win);
            }

            (*win).buf_mut().b_p_iminsert = B_IMODE_NONE;
            (*win).buf_mut().b_p_imsearch = B_IMODE_USE_INSERT;
        }

        errmsg
    }
}

/// Process the updated 'autochdir' option value.
pub fn did_set_autochdir(_args: &mut Optset) -> *const c_char {
    do_autochdir();
    ptr::null()
}

/// Process the updated 'binary' option value.
pub fn did_set_binary(args: &mut Optset) -> *const c_char {
    unsafe {
        set_options_bin(
            i32::from(args.os_oldval.data.boolean()),
            (*args.os_buf).b_p_bin,
            args.os_flags,
        );
        redraw_titles();
        ptr::null()
    }
}

/// Process the updated 'buflisted' option value.
pub fn did_set_buflisted(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;

        if i32::from(args.os_oldval.data.boolean()) != (*buf).b_p_bl {
            apply_autocmds(
                if (*buf).b_p_bl != 0 {
                    EVENT_BUFADD
                } else {
                    EVENT_BUFDELETE
                },
                ptr::null_mut(),
                ptr::null_mut(),
                true,
                buf,
            );
        }
        ptr::null()
    }
}

/// Process the new 'cmdheight' option value.
pub fn did_set_cmdheight(args: &mut Optset) -> *const c_char {
    {
        let old_value = args.os_oldval.data.integer();

        if opt_vars().p_ch > OptInt::from(g_st().rows_global - min_rows(curtab_mut()) + 1) {
            opt_vars().p_ch = OptInt::from(g_st().rows_global - min_rows(curtab_mut()) + 1);
        }

        if (opt_vars().p_ch != old_value
            || OptInt::from(
                tabline_height() + global_stl_height() + crate::globals::topframe_mut().fr_height,
            ) != OptInt::from(g_st().rows_global) - opt_vars().p_ch)
            && g_st().full_screen
        {
            command_height();
        }

        ptr::null()
    }
}

/// Process the updated 'diff' option value.
pub fn did_set_diff(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;
        diff_buf_adjust(&mut *win);
        if foldmethod_is_diff(&mut *win) {
            fold_update_all(&mut *win);
        }
        ptr::null()
    }
}

/// Process the updated 'endoffile' or 'endofline' or 'fixendofline' or 'bomb' option value.
pub fn did_set_eof_eol_fixeol_bomb(_args: &mut Optset) -> *const c_char {
    redraw_titles();
    ptr::null()
}

/// Process the updated 'equalalways' option value.
pub fn did_set_equalalways(args: &mut Optset) -> *const c_char {
    if opt_vars().p_ea != 0 && !args.os_oldval.data.boolean() {
        win_equal(args.os_win, false, 0);
    }
    ptr::null()
}

/// Process the new 'foldenable' option value.
pub fn did_set_foldenable(args: &mut Optset) -> *const c_char {
    {
        let win = args.os_win;
        let win = unsafe { &mut *win };
        if args.os_varp == (&raw mut win.w_onebuf_opt.wo_fen).cast::<c_void>()
            && i32::from(args.os_oldval.data.boolean()) != win.w_onebuf_opt.wo_fen
            && foldmethod_is_diff(&mut *win)
            && win.w_onebuf_opt.wo_scb != 0
        {
            for wp in for_all_windows_ptr() {
                if !std::ptr::eq(wp, win)
                    && foldmethod_is_diff(&mut *wp)
                    && wp.w_onebuf_opt.wo_scb != 0
                {
                    wp.w_onebuf_opt.wo_fen = win.w_onebuf_opt.wo_fen;
                    changed_window_setting(&mut *wp);
                }
            }
        }
        ptr::null()
    }
}

/// Process the new 'foldlevel' option value.
pub fn did_set_foldlevel(_args: &mut Optset) -> *const c_char {
    new_fold_level();
    ptr::null()
}

/// Process the new 'foldminlines' option value.
pub fn did_set_foldminlines(args: &mut Optset) -> *const c_char {
    unsafe {
        fold_update_all(&mut *args.os_win);
        ptr::null()
    }
}

/// Process the new 'foldnestmax' option value.
pub fn did_set_foldnestmax(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if foldmethod_is_syntax(&mut *win) || foldmethod_is_indent(&mut *win) {
            fold_update_all(&mut *win);
        }
        ptr::null()
    }
}

/// Multicursor: handle updates to 'follow' option.
pub fn did_set_follow(args: &mut Optset) -> *const c_char {
    if args.os_buf == g_st().curbuf {
        atom_follow_changed();
    }
    ptr::null()
}

/// Process the new 'helpheight' option value.
pub fn did_set_helpheight(_args: &mut Optset) -> *const c_char {
    {
        if !firstwin_mut().w_next.is_null()
            && curbuf_mut().b_help
            && OptInt::from(curwin_mut().w_height) < opt_vars().p_hh
        {
            win_setheight(opt_vars().p_hh as i32);
        }
        ptr::null()
    }
}

/// Process the updated 'hlsearch' option value.
pub fn did_set_hlsearch(_args: &mut Optset) -> *const c_char {
    set_no_hlsearch(false);
    ptr::null()
}

/// Process the updated 'ignorecase' option value.
pub fn did_set_ignorecase(_args: &mut Optset) -> *const c_char {
    if opt_vars().p_hls != 0 {
        redraw_all_later(UPD_SOME_VALID);
    }
    ptr::null()
}

/// Process the new 'iminsert' option value.
pub fn did_set_iminsert(_args: &mut Optset) -> *const c_char {
    showmode();
    status_redraw_curbuf();
    ptr::null()
}

/// Process the updated 'langnoremap' option value.
pub fn did_set_langnoremap(_args: &mut Optset) -> *const c_char {
    opt_vars().p_lrm = i32::from(opt_vars().p_lnr == 0);
    ptr::null()
}

/// Process the updated 'langremap' option value.
pub fn did_set_langremap(_args: &mut Optset) -> *const c_char {
    opt_vars().p_lnr = i32::from(opt_vars().p_lrm == 0);
    ptr::null()
}

/// Process the new 'laststatus' option value.
pub fn did_set_laststatus(args: &mut Optset) -> *const c_char {
    unsafe {
        let old_value = args.os_oldval.data.integer();
        let value = args.os_newval.data.integer();

        if value == 3 && old_value != 3 {
            frame_new_height(
                &mut *(g_st().topframe),
                crate::globals::topframe_mut().fr_height - STATUS_HEIGHT,
                false,
                false,
                false,
            );
            win_comp_pos();
            g_st().clear_cmdline = true;
        }
        if old_value == 3 && value != 3 {
            frame_new_height(
                &mut *(g_st().topframe),
                crate::globals::topframe_mut().fr_height + STATUS_HEIGHT,
                false,
                false,
                false,
            );
            win_comp_pos();
        }

        status_redraw_curbuf();
        last_status(false);
        win_float_update_statusline();
        ptr::null()
    }
}

/// Process the updated 'lines' or 'columns' option value.
pub fn did_set_lines_or_columns(args: &mut Optset) -> *const c_char {
    if opt_vars().p_lines != OptInt::from(g_st().rows_global)
        || opt_vars().p_columns != OptInt::from(g_st().columns_global)
    {
        if g_st().updating_screen {
            let oldval = integer_obj(args.os_oldval.data.integer());
            set_option_varp(args.os_idx, args.os_varp, oldval);
        } else if g_st().full_screen {
            screen_resize(opt_vars().p_columns as i32, opt_vars().p_lines as i32);
        } else {
            g_st().rows_global = opt_vars().p_lines as i32;
            g_st().columns_global = opt_vars().p_columns as i32;
            check_screensize();
            let new_row = (OptInt::from(g_st().rows_global) - opt_vars().p_ch.max(1)) as i32;
            if g_st().cmdline_row > new_row && OptInt::from(g_st().rows_global) > opt_vars().p_ch {
                g_st().cmdline_row = new_row;
            }
        }
        if opt_vars().p_window >= OptInt::from(g_st().rows_global) || !option_was_set(K_OPT_WINDOW)
        {
            opt_vars().p_window = OptInt::from(g_st().rows_global - 1);
        }
    }

    if opt_vars().p_sj >= OptInt::from(g_st().rows_global) && g_st().full_screen {
        opt_vars().p_sj = OptInt::from(g_st().rows_global / 2);
    }

    ptr::null()
}

/// Process the updated 'lisp' option value.
pub fn did_set_lisp(args: &mut Optset) -> *const c_char {
    unsafe {
        buf_init_isk_chartab(&mut *args.os_buf);
        ptr::null()
    }
}

/// Process the updated 'modifiable' option value.
pub fn did_set_modifiable(_args: &mut Optset) -> *const c_char {
    redraw_titles();
    ptr::null()
}

/// Process the updated 'modified' option value.
pub fn did_set_modified(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        if !args.os_newval.data.boolean() {
            save_file_ff(&mut *buf);
        }
        redraw_titles();
        (*buf).b_modified_was_set = args.os_newval.data.boolean();
        ptr::null()
    }
}

/// Process the updated 'number' or 'relativenumber' option value.
pub fn did_set_number_relativenumber(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if !crate::memory::cstr_is_empty((*win).w_onebuf_opt.wo_stc) {
            (*win).w_nrwidth_line_count = 0;
        }
        check_signcolumn(ptr::null_mut(), win.as_mut());
        ptr::null()
    }
}

/// Process the new 'numberwidth' option value.
pub fn did_set_numberwidth(args: &mut Optset) -> *const c_char {
    unsafe {
        (*args.os_win).w_nrwidth_line_count = 0;
        ptr::null()
    }
}

/// Process the updated 'paste' option value.
pub fn did_set_paste(_args: &mut Optset) -> *const c_char {
    {
        let empty_opt = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
        if opt_vars().p_paste != 0 {
            if opt_st().old_p_paste == 0 {
                for buf in for_all_buffers_mut() {
                    buf.b_p_tw_nopaste = buf.b_p_tw;
                    buf.b_p_wm_nopaste = buf.b_p_wm;
                    buf.b_p_sts_nopaste = buf.b_p_sts;
                    buf.b_p_ai_nopaste = buf.b_p_ai;
                    buf.b_p_et_nopaste = buf.b_p_et;
                    buf.b_p_vsts_nopaste = if !buf.b_p_vsts.is_null() && buf.b_p_vsts != empty_opt {
                        Some(crate::memory::cstr_to_cstring(buf.b_p_vsts))
                    } else {
                        None
                    };
                }

                opt_st().save_sm = opt_vars().p_sm;
                opt_st().save_sta = opt_vars().p_sta;
                opt_st().save_ru = opt_vars().p_ru;
                opt_st().save_ri = opt_vars().p_ri;
                opt_st().p_ai_nopaste = opt_vars().p_ai;
                opt_st().p_et_nopaste = opt_vars().p_et;
                opt_st().p_sts_nopaste = opt_vars().p_sts;
                opt_st().p_tw_nopaste = opt_vars().p_tw;
                opt_st().p_wm_nopaste = opt_vars().p_wm;
                opt_st().p_vsts_nopaste =
                    if !opt_vars().p_vsts.is_null() && opt_vars().p_vsts != empty_opt {
                        Some(crate::memory::cstr_to_cstring(opt_vars().p_vsts))
                    } else {
                        None
                    };
            }

            for buf in for_all_buffers_mut() {
                buf.b_p_tw = 0;
                buf.b_p_wm = 0;
                buf.b_p_sts = 0;
                buf.b_p_ai = 0;
                buf.b_p_et = 0;
                if !buf.b_p_vsts.is_null() {
                    free_string_option(buf.b_p_vsts);
                }
                buf.b_p_vsts = empty_opt;
                buf.b_p_vsts_array.clear();
            }

            opt_vars().p_sm = 0;
            opt_vars().p_sta = 0;
            if opt_vars().p_ru != 0 {
                status_redraw_all();
            }
            opt_vars().p_ru = 0;
            opt_vars().p_ri = 0;
            opt_vars().p_tw = 0;
            opt_vars().p_wm = 0;
            opt_vars().p_sts = 0;
            opt_vars().p_ai = 0;
            opt_vars().p_et = 0;
            if !opt_vars().p_vsts.is_null() {
                free_string_option(opt_vars().p_vsts);
            }
            opt_vars().p_vsts = empty_opt;
        } else if opt_st().old_p_paste != 0 {
            for buf in for_all_buffers_mut() {
                buf.b_p_tw = buf.b_p_tw_nopaste;
                buf.b_p_wm = buf.b_p_wm_nopaste;
                buf.b_p_sts = buf.b_p_sts_nopaste;
                buf.b_p_ai = buf.b_p_ai_nopaste;
                buf.b_p_et = buf.b_p_et_nopaste;
                if !buf.b_p_vsts.is_null() {
                    free_string_option(buf.b_p_vsts);
                }
                buf.b_p_vsts = buf
                    .b_p_vsts_nopaste
                    .as_ref()
                    .map_or(empty_opt, |s| alloc_string_option(s.as_ptr()));
                if !buf.b_p_vsts.is_null() && buf.b_p_vsts != empty_opt {
                    tabstop_set(buf.b_p_vsts, &mut buf.b_p_vsts_array, None);
                } else {
                    buf.b_p_vsts_array.clear();
                }
            }

            opt_vars().p_sm = opt_st().save_sm;
            opt_vars().p_sta = opt_st().save_sta;
            if opt_vars().p_ru != opt_st().save_ru {
                status_redraw_all();
            }
            opt_vars().p_ru = opt_st().save_ru;
            opt_vars().p_ri = opt_st().save_ri;
            opt_vars().p_ai = opt_st().p_ai_nopaste;
            opt_vars().p_et = opt_st().p_et_nopaste;
            opt_vars().p_sts = opt_st().p_sts_nopaste;
            opt_vars().p_tw = opt_st().p_tw_nopaste;
            opt_vars().p_wm = opt_st().p_wm_nopaste;
            if !opt_vars().p_vsts.is_null() {
                free_string_option(opt_vars().p_vsts);
            }
            opt_vars().p_vsts = opt_st()
                .p_vsts_nopaste
                .as_ref()
                .map_or(empty_opt, |s| alloc_string_option(s.as_ptr()));
        }

        opt_st().old_p_paste = opt_vars().p_paste;

        didset_options_sctx(OPT_LOCAL | OPT_GLOBAL, P_PASTE_DEP_OPTS);

        ptr::null()
    }
}

/// Validate the 'previewwindow' option.
pub fn validate_previewwindow(args: &mut Optset) -> *const c_char {
    unsafe {
        if (args.os_flags & OPT_GLOBAL) != 0 || !args.os_newval.data.boolean() {
            return ptr::null();
        }
        let win = args.os_win;
        let tab = if win == curwin_ptr() {
            g_st().curtab
        } else {
            win_find_tabpage(win)
        };
        for wp in for_all_windows_in_tab_ptr(&*tab) {
            if !std::ptr::eq(wp, win) && wp.w_onebuf_opt.wo_pvw != 0 {
                return E_PREVIEW_WINDOW_ALREADY_EXISTS;
            }
        }
        ptr::null()
    }
}

/// Process the new 'pumblend' option value.
pub fn did_set_pumblend(_args: &mut Optset) -> *const c_char {
    hl_invalidate_blends();
    if pum_drawn() {
        pum_redraw();
    }
    ptr::null()
}

/// Process the updated 'readonly' option value.
pub fn did_set_readonly(args: &mut Optset) -> *const c_char {
    {
        let buf = args.os_buf;
        let buf = unsafe { &mut *buf };

        if buf.b_p_ro == 0 && (args.os_flags & OPT_LOCAL) == 0 {
            g_st().readonlymode = false;
        }

        if buf.b_p_ro != 0 {
            buf.b_did_warn = false;
        }

        redraw_titles();
        ptr::null()
    }
}

/// Process the new 'scrollback' option value.
pub fn did_set_scrollback(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let old_value = args.os_oldval.data.integer();
        let value = args.os_newval.data.integer();

        if !(*buf).terminal.is_null() && value < old_value {
            on_scrollback_option_changed(&mut *(*buf).terminal.cast());
        }
        ptr::null()
    }
}

/// Process the updated 'scrollbind' option value.
pub fn did_set_scrollbind(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;

        if (*win).w_onebuf_opt.wo_scb == 0 {
            return ptr::null();
        }
        do_check_scrollbind(false);
        (*win).w_scbind_pos = get_vtopline(&mut *win);
        ptr::null()
    }
}

/// Process the new 'shiftwidth' or the 'tabstop' option value.
pub fn did_set_shiftwidth_tabstop(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let win = args.os_win;
        let pp = args.os_varp.cast::<OptInt>();

        if foldmethod_is_indent(&mut *win) {
            fold_update_all(&mut *win);
        }
        if pp == &raw mut (*buf).b_p_sw || (*buf).b_p_sw == 0 {
            parse_cino(&mut *buf);
        }
        ptr::null()
    }
}

/// Process the new 'showtabline' option value.
pub fn did_set_showtabline(_args: &mut Optset) -> *const c_char {
    win_new_screen_rows();
    ptr::null()
}

/// Process the updated 'smoothscroll' option value.
pub fn did_set_smoothscroll(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if (*win).w_onebuf_opt.wo_sms == 0 {
            (*win).w_skipcol = 0;
        }
        ptr::null()
    }
}

/// Process the updated 'spell' option value.
pub fn did_set_spell(args: &mut Optset) -> *const c_char {
    unsafe {
        let win = args.os_win;
        if (*win).w_onebuf_opt.wo_spell != 0 {
            return parse_spelllang(&mut *win);
        }
        ptr::null()
    }
}

/// Process the updated 'swapfile' option value.
pub fn did_set_swapfile(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        if (*buf).b_p_swf != 0 && opt_vars().p_uc != 0 {
            ml_open_file(&mut *buf);
        } else {
            mf_close_file(&mut *buf, true);
        }
        ptr::null()
    }
}

/// Process the new 'textwidth' option value.
pub fn did_set_textwidth(_args: &mut Optset) -> *const c_char {
    {
        for tp in for_all_tabpages_ptr() {
            for wp in for_all_windows_in_tab_ptr(&*tp) {
                check_colorcolumn(ptr::null_mut(), Some(&mut *wp));
            }
        }
        ptr::null()
    }
}

/// Process the updated 'title' or the 'icon' option value.
pub fn did_set_title_icon(_args: &mut Optset) -> *const c_char {
    did_set_title();
    ptr::null()
}

/// Process the new 'titlelen' option value.
pub fn did_set_titlelen(args: &mut Optset) -> *const c_char {
    {
        let old_value = args.os_oldval.data.integer();
        if starting_val() != NO_SCREEN && old_value != opt_vars().p_titlelen {
            g_st().need_maketitle = true;
        }
        ptr::null()
    }
}

/// Process the updated 'undofile' option value.
pub fn did_set_undofile(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;

        if (*buf).b_p_udf == 0 && opt_vars().p_udf == 0 {
            return ptr::null();
        }

        let mut hash = [0u8; UNDO_HASH_SIZE];

        for bp in for_all_buffers_ptr() {
            if (buf == bp || (args.os_flags & OPT_GLOBAL) != 0 || args.os_flags == 0)
                && !buf_is_changed_fn(&mut *bp)
                && bp.b_ml.ml_mfp.is_some()
            {
                u_compute_hash(&mut *bp, hash.as_mut_ptr());
                u_read_undo(ptr::null_mut(), hash.as_ptr(), bp.b_fname);
            }
        }

        ptr::null()
    }
}

/// Process the new global 'undolevels' option value.
pub fn did_set_global_undolevels(value: OptInt, old_value: OptInt) -> *const c_char {
    opt_vars().p_ul = old_value;
    u_sync(true);
    opt_vars().p_ul = value;
    ptr::null()
}

/// Process the new buffer local 'undolevels' option value.
pub fn did_set_buflocal_undolevels(
    buf: &mut FileBuf,
    value: OptInt,
    old_value: OptInt,
) -> *const c_char {
    buf.b_p_ul = old_value;
    u_sync(true);
    buf.b_p_ul = value;
    ptr::null()
}

/// Process the new 'undolevels' option value.
pub fn did_set_undolevels(args: &mut Optset) -> *const c_char {
    unsafe {
        let buf = args.os_buf;
        let pp = args.os_varp.cast::<OptInt>();

        if pp == &raw mut opt_vars().p_ul {
            did_set_global_undolevels(args.os_newval.data.integer(), args.os_oldval.data.integer());
        } else if pp == &raw mut (*buf).b_p_ul {
            did_set_buflocal_undolevels(
                &mut *buf,
                args.os_newval.data.integer(),
                args.os_oldval.data.integer(),
            );
        }

        ptr::null()
    }
}

/// Process the new 'updatecount' option value.
pub fn did_set_updatecount(args: &mut Optset) -> *const c_char {
    let old_value = args.os_oldval.data.integer();
    if opt_vars().p_uc != 0 && old_value == 0 {
        ml_open_files();
    }
    ptr::null()
}

/// Validate the 'wildchar' or 'wildcharm' option.
pub fn validate_wildchar(args: &mut Optset) -> *const c_char {
    let c = args.os_newval.data.integer();
    if c == CTRL_C_CONST_CONST
        || c == OptInt::from(b'\n')
        || c == OptInt::from(b'\r')
        || c == OptInt::from(K_KENTER)
    {
        return E_INVARG.as_ptr();
    }
    ptr::null()
}

/// Process the new 'winblend' option value.
pub fn did_set_winblend(args: &mut Optset) -> *const c_char {
    {
        let win = args.os_win;
        let win = unsafe { &mut *win };
        let old_value = args.os_oldval.data.integer();
        let value = args.os_newval.data.integer();

        if value != old_value {
            win.w_onebuf_opt.wo_winbl = win.w_onebuf_opt.wo_winbl.clamp(0, 100);
            win.w_hl_needs_update = 1;
            check_blending(&mut *win);
        }
        ptr::null()
    }
}

/// Process the new 'window' option value.
pub fn did_set_window(_args: &mut Optset) -> *const c_char {
    if opt_vars().p_window < 1 || opt_vars().p_window >= OptInt::from(g_st().rows_global) {
        opt_vars().p_window = OptInt::from(g_st().rows_global - 1);
    }
    ptr::null()
}

/// Process the new 'winheight' value.
pub fn did_set_winheight(_args: &mut Optset) -> *const c_char {
    {
        if !firstwin_mut().w_next.is_null() && OptInt::from(curwin_mut().w_height) < opt_vars().p_wh
        {
            win_setheight(opt_vars().p_wh as i32);
        }
        ptr::null()
    }
}

/// Process the new 'winwidth' option value.
pub fn did_set_winwidth(_args: &mut Optset) -> *const c_char {
    {
        if !firstwin_mut().w_next.is_null() && OptInt::from(curwin_mut().w_width) < opt_vars().p_wiw
        {
            win_setwidth(opt_vars().p_wiw as i32);
        }
        ptr::null()
    }
}

/// Process the updated 'wrap' option value.
pub fn did_set_wrap(args: &mut Optset) -> *const c_char {
    {
        let win = args.os_win;
        let win = unsafe { &mut *win };
        if win.w_onebuf_opt.wo_wrap != 0 {
            win.w_leftcol = 0;
        } else {
            win.w_skipcol = 0;
        }
        ptr::null()
    }
}

/// Process the new 'chistory' or 'lhistory' option value.
pub fn did_set_xhistory(args: &mut Optset) -> *const c_char {
    unsafe {
        let is_p_chi = args.os_varp.cast::<OptInt>() == &raw mut opt_vars().p_chi;
        let arg = if is_p_chi {
            &raw mut opt_vars().p_chi
        } else {
            args.os_varp.cast::<OptInt>()
        };

        if is_p_chi {
            qf_resize_stack(*arg as i32);
        } else {
            ll_resize_stack(&mut *args.os_win, *arg as i32);
        }

        ptr::null()
    }
}

fn do_syntax_autocmd(buf: &mut FileBuf, value_changed: bool) {
    opt_st().syn_recursive += 1;
    buf.b_flags |= BF_SYN_SET;
    apply_autocmds(
        EVENT_SYNTAX,
        buf.b_p_syn,
        buf.b_fname,
        value_changed || opt_st().syn_recursive == 1,
        &raw mut *buf,
    );
    opt_st().syn_recursive -= 1;
}

fn do_spelllang_source(win: &mut WinInfoInfo) {
    {
        let mut fname = [0 as c_char; 200];
        let ws = win.w_s_ptr();
        let mut q = ws.b_p_spl;

        if crate::memory::cstr_prefix_bytes(q, 4) == &b"cjk,"[..4] {
            q = q.wrapping_add(4);
        }

        let mut p = q;
        while !crate::memory::cstr_is_empty(p) {
            if !ascii_isalnum(crate::memory::cstr_first_char(p))
                && crate::memory::cstr_first_byte(p) != b'-'
            {
                break;
            }
            p = p.wrapping_add(1);
        }
        if p > q {
            vim_snprintf!(
                fname.as_mut_ptr(),
                fname.len(),
                c"spell/%.*s.*",
                crate::memory::ptr_offset_from(p, q) as i32,
                q,
            );
            source_runtime_vim_lua(fname.as_mut_ptr(), DIP_ALL);
        }
    }
}

/// Check the bounds of numeric options.
fn check_num_option_bounds(
    opt_idx: OptIndex,
    newval: *mut OptInt,
    win: &mut WinInfoInfo,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let mut errmsg: *const c_char = ptr::null();

        match opt_idx {
            K_OPT_LINES => {
                let min_r = min_rows_for_all_tabpages();
                if *newval < OptInt::from(min_r) && g_st().full_screen {
                    errmsg = opt_error_int(errbuf, c"E593: Need at least %d lines", min_r);
                    *newval = OptInt::from(min_r);
                }
                *newval = (*newval).min(OptInt::from(i32::MAX));
            }
            K_OPT_COLUMNS => {
                if *newval < OptInt::from(MIN_COLUMNS) && g_st().full_screen {
                    errmsg = opt_error_int(errbuf, c"E594: Need at least %d columns", MIN_COLUMNS);
                    *newval = OptInt::from(MIN_COLUMNS);
                }
                *newval = (*newval).min(OptInt::from(i32::MAX));
            }
            K_OPT_PUMBLEND => {
                *newval = (*newval).clamp(0, 100);
            }
            K_OPT_SCROLLJUMP => {
                if (*newval < -100 || *newval >= OptInt::from(g_st().rows_global))
                    && g_st().full_screen
                {
                    errmsg = E_SCROLL.as_ptr();
                    *newval = 1;
                }
            }
            K_OPT_SCROLL
                if (*newval <= 0
                    || (*newval > OptInt::from(win.w_view_height) && win.w_view_height > 0))
                    && g_st().full_screen =>
            {
                if *newval != 0 {
                    errmsg = E_SCROLL.as_ptr();
                }
                *newval = OptInt::from(win_default_scroll(win));
            }
            _ => {}
        }

        errmsg
    }
}

/// Validate and bound check option value.
fn validate_num_option(
    opt_idx: OptIndex,
    newval: *mut OptInt,
    win: &mut WinInfoInfo,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        let value = *newval;

        if value < OptInt::from(i32::MIN) || value > OptInt::from(i32::MAX) {
            return E_INVARG.as_ptr();
        }

        const MAX_SEARCH_COUNT: OptInt = 9999;

        match opt_idx {
            K_OPT_HELPHEIGHT | K_OPT_TITLELEN | K_OPT_UPDATECOUNT | K_OPT_REPORT
            | K_OPT_UPDATETIME | K_OPT_SIDESCROLL | K_OPT_FOLDLEVEL | K_OPT_SHIFTWIDTH
            | K_OPT_TEXTWIDTH | K_OPT_WRITEDELAY | K_OPT_TIMEOUTLEN => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                }
            }
            K_OPT_WINHEIGHT => {
                if value < 1 {
                    return E_POSITIVE.as_ptr();
                } else if opt_vars().p_wmh > value {
                    return E_WINHEIGHT.as_ptr();
                }
            }
            K_OPT_WINMINHEIGHT => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                } else if value > opt_vars().p_wh {
                    return E_WINHEIGHT.as_ptr();
                }
            }
            K_OPT_WINWIDTH => {
                if value < 1 {
                    return E_POSITIVE.as_ptr();
                } else if opt_vars().p_wmw > value {
                    return E_WINWIDTH.as_ptr();
                }
            }
            K_OPT_WINMINWIDTH => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                } else if value > opt_vars().p_wiw {
                    return E_WINWIDTH.as_ptr();
                }
            }
            K_OPT_MAXCOMBINE => {
                *newval = MAX_MCO;
            }
            K_OPT_CMDHEIGHT => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                }
            }
            K_OPT_HISTORY => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                } else if value > 10000 {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_PYXVERSION => {
                if value == 0 {
                    *newval = 3;
                } else if value != 3 {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_REGEXPENGINE => {
                if !(0..=2).contains(&value) {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_SCROLLOFF => {
                if value < 0 && g_st().full_screen {
                    return E_POSITIVE.as_ptr();
                }
            }
            K_OPT_SCROLLOFFPAD => {
                if value < 0 {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_SIDESCROLLOFF => {
                if value < 0 && g_st().full_screen {
                    return E_POSITIVE.as_ptr();
                }
            }
            K_OPT_CMDWINHEIGHT => {
                if value < 1 {
                    return E_POSITIVE.as_ptr();
                }
            }
            K_OPT_CONCEALLEVEL => {
                if value < 0 {
                    return E_POSITIVE.as_ptr();
                } else if value > 3 {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_NUMBERWIDTH => {
                if value < 1 {
                    return E_POSITIVE.as_ptr();
                } else if value > MAX_NUMBERWIDTH {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_IMINSERT => {
                if !(0..=B_IMODE_LAST).contains(&value) {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_IMSEARCH => {
                if !(-1..=B_IMODE_LAST).contains(&value) {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_CHANNEL => {
                return E_INVARG.as_ptr();
            }
            K_OPT_SCROLLBACK => {
                if !(-1..=SB_MAX).contains(&value) {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_TABSTOP => {
                if value < 1 {
                    return E_POSITIVE.as_ptr();
                } else if value > OptInt::from(crate::indent::TABSTOP_MAX) {
                    return E_INVARG.as_ptr();
                }
            }
            K_OPT_CHISTORY | K_OPT_LHISTORY => {
                if value < 1 {
                    return E_CANNOT_HAVE_NEGATIVE_OR_ZERO_NUMBER_OF_QUICKFIX;
                } else if value > 100 {
                    return E_CANNOT_HAVE_MORE_THAN_HUNDRED_QUICKFIX;
                }
            }
            K_OPT_MAXSEARCHCOUNT => {
                if value <= 0 {
                    return E_POSITIVE.as_ptr();
                } else if value > MAX_SEARCH_COUNT {
                    return E_INVARG.as_ptr();
                }
            }
            _ => {}
        }

        check_num_option_bounds(opt_idx, newval, win, errbuf)
    }
}

/// Called after an option changed: check if something needs to be redrawn.
pub fn check_redraw_for(buf: *mut FileBuf, win: &mut WinInfoInfo, flags: u32) {
    {
        let all = (flags & K_OPT_FLAG_REDR_ALL) == K_OPT_FLAG_REDR_ALL;

        if (flags & K_OPT_FLAG_REDR_STAT) != 0 || all {
            status_redraw_all();
        }

        if (flags & K_OPT_FLAG_REDR_TABL) != 0 || all {
            g_st().redraw_tabline = true;
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
}

pub fn check_redraw(flags: u32) {
    {
        check_redraw_for(g_st().curbuf, curwin_mut(), flags);
    }
}

pub fn is_tty_option(name: impl crate::strings::IntoCCharPtr) -> bool {
    let name = name.into_c_char_ptr();
    !find_tty_option_end(name).is_null()
}

/// Get value of TTY option.
pub fn get_tty_option(name: impl crate::strings::IntoCCharPtr) -> Object {
    let name = name.into_c_char_ptr();
    {
        if crate::memory::cstr_as_opt_bytes(name) == Some(b"t_Co") {
            if g_st().t_colors <= 1 {
                cstr_to_obj(c"")
            } else {
                let mut value = [0 as c_char; NUMBUFLEN];
                vim_snprintf!(value.as_mut_ptr(), NUMBUFLEN, c"%d", g_st().t_colors);
                cstr_to_obj(value.as_ptr())
            }
        } else if crate::memory::cstr_as_opt_bytes(name) == Some(b"term") {
            cstr_to_obj(
                opt_st()
                    .p_term
                    .as_ref()
                    .map_or(c"nvim".as_ptr(), |s| s.as_ptr()),
            )
        } else if crate::memory::cstr_as_opt_bytes(name) == Some(b"ttytype") {
            cstr_to_obj(
                opt_st()
                    .p_ttytype
                    .as_ref()
                    .map_or(c"nvim".as_ptr(), |s| s.as_ptr()),
            )
        } else if is_tty_option(name) {
            cstr_to_obj(c"")
        } else {
            nil_obj()
        }
    }
}

pub fn set_tty_option(name: &std::ffi::CStr, value: Option<std::ffi::CString>) -> bool {
    if Some(name.to_bytes()) == Some(b"term") {
        opt_st().p_term = value;
        return true;
    }

    if Some(name.to_bytes()) == Some(b"ttytype") {
        opt_st().p_ttytype = value;
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
            data: ObjectData::String(NvimString {
                data: S.as_ptr().cast_mut(),
                size: S.to_bytes().len(),
            }),
        }
    }};
}

macro_rules! opt_cb {
    ($cb:path) => {{ Some($cb) }};
}

macro_rules! opt_var {
    ($field:ident) => {
        unsafe { (&raw mut (*OPT_VARS.as_ptr()).$field).cast() }
    };
}

macro_rules! opt_flags {
    ($field:ident) => {
        unsafe { &raw mut (*OPT_VARS.as_ptr()).$field }
    };
}

macro_rules! opt_def_var {
    ($idx:literal) => {
        unsafe { (&raw mut (*OPTIONS.as_ptr())[$idx].def_val).cast() }
    };
}

static OPTIONS: GlobalState<[Vimoption; 379]> = GlobalState::new([
    Vimoption { fullname: c"aleph", shortname: Some(c"al"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(0), flags_var: ptr::null_mut(), scope_idx: [0, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(224) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"allowrevins", shortname: Some(c"ari"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ari), flags_var: ptr::null_mut(), scope_idx: [1, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ambiwidth", shortname: Some(c"ambw"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ambw), flags_var: ptr::null_mut(), scope_idx: [2, -1, -1, -1], immutable: false, values: &OPT_AMBW_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_ambiwidth), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"single"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"arabic", shortname: Some(c"arab"), flags: K_OPT_FLAG_CURSWANT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 0, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_arabic), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"arabicshape", shortname: Some(c"arshape"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_arshape), flags_var: ptr::null_mut(), scope_idx: [3, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autochdir", shortname: Some(c"acd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_acd), flags_var: ptr::null_mut(), scope_idx: [4, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_autochdir), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocomplete", shortname: Some(c"ac"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ac), flags_var: ptr::null_mut(), scope_idx: [5, -1, 0, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocompletedelay", shortname: Some(c"acl"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_acl), flags_var: ptr::null_mut(), scope_idx: [6, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autocompletetimeout", shortname: Some(c"act"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_act), flags_var: ptr::null_mut(), scope_idx: [7, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autoindent", shortname: Some(c"ai"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ai), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autoread", shortname: Some(c"ar"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ar), flags_var: ptr::null_mut(), scope_idx: [8, -1, 2, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autowrite", shortname: Some(c"aw"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_aw), flags_var: ptr::null_mut(), scope_idx: [9, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"autowriteall", shortname: Some(c"awa"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_awa), flags_var: ptr::null_mut(), scope_idx: [10, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"background", shortname: Some(c"bg"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bg), flags_var: ptr::null_mut(), scope_idx: [11, -1, -1, -1], immutable: false, values: &OPT_BG_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_background), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dark"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backspace", shortname: Some(c"bs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bs), flags_var: ptr::null_mut(), scope_idx: [12, -1, -1, -1], immutable: false, values: &OPT_BS_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backspace), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"indent,eol,start"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backup", shortname: Some(c"bk"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bk), flags_var: ptr::null_mut(), scope_idx: [13, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(unix)]
    Vimoption { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_bkc), flags_var: opt_flags!(bkc_flags), scope_idx: [14, -1, 3, -1], immutable: false, values: &OPT_BKC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(unix))]
    Vimoption { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_bkc), flags_var: opt_flags!(bkc_flags), scope_idx: [14, -1, 3, -1], immutable: false, values: &OPT_BKC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupdir", shortname: Some(c"bdir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bdir), flags_var: ptr::null_mut(), scope_idx: [15, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupext", shortname: Some(c"bex"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bex), flags_var: ptr::null_mut(), scope_idx: [16, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"~"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"backupskip", shortname: Some(c"bsk"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bsk), flags_var: ptr::null_mut(), scope_idx: [17, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"belloff", shortname: Some(c"bo"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_bo), flags_var: opt_flags!(bo_flags), scope_idx: [18, -1, -1, -1], immutable: false, values: &OPT_BO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"all"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"binary", shortname: Some(c"bin"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bin), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 4, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_binary), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"bomb", shortname: None, flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bomb), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 5, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakat", shortname: Some(c"brk"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_breakat), flags_var: ptr::null_mut(), scope_idx: [19, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakat), opt_expand_cb: None, def_val: cstr_def_val!(c" \t!@*-+;:,./?"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakindent", shortname: Some(c"bri"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"breakindentopt", shortname: Some(c"briopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 2, -1, -1], immutable: false, values: &OPT_BRIOPT_VALUES, schema: &OPT_BRIOPT_SCHEMA, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakindentopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"browsedir", shortname: Some(c"bsdir"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(26), flags_var: ptr::null_mut(), scope_idx: [20, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"bufhidden", shortname: Some(c"bh"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bh), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 6, -1], immutable: false, values: &OPT_BH_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"buflisted", shortname: Some(c"bl"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bl), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 7, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_buflisted), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"buftype", shortname: Some(c"bt"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_bt), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 8, -1], immutable: false, values: &OPT_BT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_buftype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_buftype), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"busy", shortname: None, flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_busy), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 9, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"casemap", shortname: Some(c"cmp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cmp), flags_var: opt_flags!(cmp_flags), scope_idx: [21, -1, -1, -1], immutable: false, values: &OPT_CMP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"internal,keepascii"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"cdhome", shortname: Some(c"cdh"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdh), flags_var: ptr::null_mut(), scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"cdhome", shortname: Some(c"cdh"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdh), flags_var: ptr::null_mut(), scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cdpath", shortname: Some(c"cd"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cdpath), flags_var: ptr::null_mut(), scope_idx: [23, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c",,"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cedit", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cedit), flags_var: ptr::null_mut(), scope_idx: [24, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::ex_getln::validate_cedit), opt_did_set_cb: opt_cb!(crate::ex_getln::did_set_cedit), opt_expand_cb: None, def_val: cstr_def_val!(c"\x06"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"channel", shortname: None, flags: K_OPT_FLAG_NO_MKRC|K_OPT_FLAG_NO_DEFAULT, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_channel), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 10, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"charconvert", shortname: Some(c"ccv"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ccv), flags_var: ptr::null_mut(), scope_idx: [25, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"chistory", shortname: Some(c"chi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_chi), flags_var: ptr::null_mut(), scope_idx: [26, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cindent", shortname: Some(c"cin"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cin), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 11, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinkeys", shortname: Some(c"cink"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cink), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 12, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinoptions", shortname: Some(c"cino"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cino), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 13, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cinoptions), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinscopedecls", shortname: Some(c"cinsd"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cinsd), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 14, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"public,protected,private"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cinwords", shortname: Some(c"cinw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cinw), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 15, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"if,else,while,do,for,switch"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"clipboard", shortname: Some(c"cb"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cb), flags_var: opt_flags!(cb_flags), scope_idx: [27, -1, -1, -1], immutable: false, values: &OPT_CB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cmdheight", shortname: Some(c"ch"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_TAB)) as u8, var: opt_var!(p_ch), flags_var: ptr::null_mut(), scope_idx: [28, -1, -1, 0], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_cmdheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cmdwinheight", shortname: Some(c"cwh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cwh), flags_var: ptr::null_mut(), scope_idx: [29, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(7) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"colorcolumn", shortname: Some(c"cc"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 3, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_colorcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_colorcolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"columns", shortname: Some(c"co"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_columns), flags_var: ptr::null_mut(), scope_idx: [30, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"comments", shortname: Some(c"com"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_com), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 16, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_comments), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-,fb:•"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"commentstring", shortname: Some(c"cms"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cms), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 17, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_commentstring), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"compatible", shortname: Some(c"cp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(50), flags_var: ptr::null_mut(), scope_idx: [31, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"complete", shortname: Some(c"cpt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cpt), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 18, -1], immutable: false, values: &OPT_CPT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_complete), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_complete), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c".,w,b,u,t"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completefunc", shortname: Some(c"cfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_cfu), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 19, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completeitemalign", shortname: Some(c"cia"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cia), flags_var: ptr::null_mut(), scope_idx: [32, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_completeitemalign), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeitemalign), opt_expand_cb: None, def_val: cstr_def_val!(c"abbr,kind,menu"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completeopt", shortname: Some(c"cot"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_cot), flags_var: opt_flags!(cot_flags), scope_idx: [33, -1, 20, -1], immutable: false, values: &OPT_COT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu,popup"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_csl), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 21, -1], immutable: false, values: &OPT_CSL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeslash), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_def_var!(55), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 21, -1], immutable: true, values: &OPT_CSL_VALUES, schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"completetimeout", shortname: Some(c"cto"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cto), flags_var: ptr::null_mut(), scope_idx: [34, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"concealcursor", shortname: Some(c"cocu"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 4, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_concealcursor), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_concealcursor), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"conceallevel", shortname: Some(c"cole"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 5, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"confirm", shortname: Some(c"cf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_confirm), flags_var: ptr::null_mut(), scope_idx: [35, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"copyindent", shortname: Some(c"ci"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ci), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 22, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cpoptions", shortname: Some(c"cpo"), flags: K_OPT_FLAG_FLAG_LIST|K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_cpo), flags_var: ptr::null_mut(), scope_idx: [36, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cpoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_cpoptions), def_val: cstr_def_val!(CPO_VIM_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorbind", shortname: Some(c"crb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 6, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorcolumn", shortname: Some(c"cuc"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 7, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorline", shortname: Some(c"cul"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 8, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"cursorlineopt", shortname: Some(c"culopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 9, -1, -1], immutable: false, values: &OPT_CULOPT_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cursorlineopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cursorlineopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"debug", shortname: None, flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_debug), flags_var: ptr::null_mut(), scope_idx: [37, -1, -1, -1], immutable: false, values: &OPT_DEBUG_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"define", shortname: Some(c"def"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_def), flags_var: ptr::null_mut(), scope_idx: [38, -1, 23, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"delcombine", shortname: Some(c"deco"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_deco), flags_var: ptr::null_mut(), scope_idx: [39, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"dictionary", shortname: Some(c"dict"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_NDNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_dict), flags_var: ptr::null_mut(), scope_idx: [40, -1, 24, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diff", shortname: None, flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 10, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_diff), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffanchors", shortname: Some(c"dia"), flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_dia), flags_var: ptr::null_mut(), scope_idx: [41, -1, 25, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffanchors), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffexpr", shortname: Some(c"dex"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dex), flags_var: ptr::null_mut(), scope_idx: [42, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"diffopt", shortname: Some(c"dip"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dip), flags_var: ptr::null_mut(), scope_idx: [43, -1, -1, -1], immutable: false, values: &OPT_DIP_VALUES, schema: &OPT_DIP_SCHEMA, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_diffopt), def_val: cstr_def_val!(c"internal,filler,closeoff,indent-heuristic,inline:char,linematch:40"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"digraph", shortname: Some(c"dg"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dg), flags_var: ptr::null_mut(), scope_idx: [44, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"directory", shortname: Some(c"dir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dir), flags_var: ptr::null_mut(), scope_idx: [45, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"display", shortname: Some(c"dy"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_dy), flags_var: opt_flags!(dy_flags), scope_idx: [46, -1, -1, -1], immutable: false, values: &OPT_DY_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_display), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"lastline"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eadirection", shortname: Some(c"ead"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ead), flags_var: ptr::null_mut(), scope_idx: [47, -1, -1, -1], immutable: false, values: &OPT_EAD_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"edcompatible", shortname: Some(c"ed"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(78), flags_var: ptr::null_mut(), scope_idx: [48, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"emoji", shortname: Some(c"emo"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_emoji), flags_var: ptr::null_mut(), scope_idx: [49, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_emoji), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"encoding", shortname: Some(c"enc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_enc), flags_var: ptr::null_mut(), scope_idx: [50, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: None, def_val: cstr_def_val!(c"utf-8"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"endoffile", shortname: Some(c"eof"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_eof), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 26, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"endofline", shortname: Some(c"eol"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_eol), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 27, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"equalalways", shortname: Some(c"ea"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ea), flags_var: ptr::null_mut(), scope_idx: [51, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_equalalways), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"equalprg", shortname: Some(c"ep"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ep), flags_var: ptr::null_mut(), scope_idx: [52, -1, 28, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorbells", shortname: Some(c"eb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_eb), flags_var: ptr::null_mut(), scope_idx: [53, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorfile", shortname: Some(c"ef"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ef), flags_var: ptr::null_mut(), scope_idx: [54, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"errors.err"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"errorformat", shortname: Some(c"efm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_efm), flags_var: ptr::null_mut(), scope_idx: [55, -1, 29, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_EFM_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eventignore", shortname: Some(c"ei"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ei), flags_var: ptr::null_mut(), scope_idx: [56, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"eventignorewin", shortname: Some(c"eiw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 11, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"expandtab", shortname: Some(c"et"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_et), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 30, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"exrc", shortname: Some(c"ex"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_exrc), flags_var: ptr::null_mut(), scope_idx: [57, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fileencoding", shortname: Some(c"fenc"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fenc), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 31, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fileencodings", shortname: Some(c"fencs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fencs), flags_var: ptr::null_mut(), scope_idx: [58, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c"ucs-bom,utf-8,default,latin1"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"fileformat", shortname: Some(c"ff"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ff), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 32, -1], immutable: false, values: &OPT_FF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"fileformat", shortname: Some(c"ff"), flags: K_OPT_FLAG_CURSWANT|K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ff), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 32, -1], immutable: false, values: &OPT_FF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"fileformats", shortname: Some(c"ffs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ffs), flags_var: ptr::null_mut(), scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos,unix"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"fileformats", shortname: Some(c"ffs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ffs), flags_var: ptr::null_mut(), scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix,dos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(any(windows, target_os = "macos"))]
    Vimoption { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fic), flags_var: ptr::null_mut(), scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(any(windows, target_os = "macos")))]
    Vimoption { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fic), flags_var: ptr::null_mut(), scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"filetype", shortname: Some(c"ft"), flags: K_OPT_FLAG_NO_GLOB|K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ft), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 33, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fillchars", shortname: Some(c"fcs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_fcs), flags_var: ptr::null_mut(), scope_idx: [61, 12, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"findfunc", shortname: Some(c"ffu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ffu), flags_var: ptr::null_mut(), scope_idx: [62, -1, 34, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fixendofline", shortname: Some(c"fixeol"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fixeol), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 35, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldclose", shortname: Some(c"fcl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fcl), flags_var: ptr::null_mut(), scope_idx: [63, -1, -1, -1], immutable: false, values: &OPT_FCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldcolumn", shortname: Some(c"fdc"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 13, -1, -1], immutable: false, values: &OPT_FDC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"0"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldenable", shortname: Some(c"fen"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 14, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldenable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldexpr", shortname: Some(c"fde"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 15, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldexpr), opt_expand_cb: None, def_val: cstr_def_val!(c"0"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldignore", shortname: Some(c"fdi"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 16, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldignore), opt_expand_cb: None, def_val: cstr_def_val!(c"#"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldlevel", shortname: Some(c"fdl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 17, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldlevel), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldlevelstart", shortname: Some(c"fdls"), flags: K_OPT_FLAG_CURSWANT, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fdls), flags_var: ptr::null_mut(), scope_idx: [64, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldmarker", shortname: Some(c"fmr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 18, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_foldmarker), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmarker), opt_expand_cb: None, def_val: cstr_def_val!(c"{{{,}}}"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldmethod", shortname: Some(c"fdm"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 19, -1, -1], immutable: false, values: &OPT_FDM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmethod), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"manual"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldminlines", shortname: Some(c"fml"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 20, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldminlines), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldnestmax", shortname: Some(c"fdn"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 21, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldnestmax), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldopen", shortname: Some(c"fdo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_CURSWANT|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_fdo), flags_var: opt_flags!(fdo_flags), scope_idx: [65, -1, -1, -1], immutable: false, values: &OPT_FDO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"block,hor,mark,percent,quickfix,search,tag,undo"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"foldtext", shortname: Some(c"fdt"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 22, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"foldtext()"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"follow", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_follow), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 36, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_follow), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatexpr", shortname: Some(c"fex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fex), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 37, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatlistpat", shortname: Some(c"flp"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_flp), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 38, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"^\\s*\\d\\+[\\]:.)}\\t ]\\s*"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatoptions", shortname: Some(c"fo"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_fo), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 39, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_formatoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_formatoptions), def_val: cstr_def_val!(c"tcqj"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"formatprg", shortname: Some(c"fp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_fp), flags_var: ptr::null_mut(), scope_idx: [66, -1, 40, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"fsync", shortname: Some(c"fs"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_fs), flags_var: ptr::null_mut(), scope_idx: [67, -1, 41, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"gdefault", shortname: Some(c"gd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_gd), flags_var: ptr::null_mut(), scope_idx: [68, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"grepformat", shortname: Some(c"gfm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gefm), flags_var: ptr::null_mut(), scope_idx: [69, -1, 42, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"%f:%l:%m,%f:%l%m,%f  %l%m"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"grepprg", shortname: Some(c"gp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gp), flags_var: ptr::null_mut(), scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"findstr /n $* nul"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"grepprg", shortname: Some(c"gp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_gp), flags_var: ptr::null_mut(), scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"grep -HIn $* /dev/null"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guicursor", shortname: Some(c"gcr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guicursor), flags_var: ptr::null_mut(), scope_idx: [71, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_guicursor), opt_expand_cb: None, def_val: cstr_def_val!(c"n-v-c-sm:block,i-ci-ve:ver25,r-cr-o:hor20,t:block-blinkon500-blinkoff500-TermCursor"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guifont", shortname: Some(c"gfn"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_UIOPTION|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guifont), flags_var: ptr::null_mut(), scope_idx: [72, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_GFN_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guifontwide", shortname: Some(c"gfw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_UIOPTION|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_guifontwide), flags_var: ptr::null_mut(), scope_idx: [73, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guioptions", shortname: Some(c"go"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(126), flags_var: ptr::null_mut(), scope_idx: [74, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guitablabel", shortname: Some(c"gtl"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(127), flags_var: ptr::null_mut(), scope_idx: [75, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"guitabtooltip", shortname: Some(c"gtt"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(128), flags_var: ptr::null_mut(), scope_idx: [76, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helpfile", shortname: Some(c"hf"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hf), flags_var: ptr::null_mut(), scope_idx: [77, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_helpfile), opt_expand_cb: None, def_val: cstr_def_val!(c"$VIMRUNTIME/doc/help.txt"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helpheight", shortname: Some(c"hh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hh), flags_var: ptr::null_mut(), scope_idx: [78, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_helpheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"helplang", shortname: Some(c"hlg"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hlg), flags_var: ptr::null_mut(), scope_idx: [79, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_helplang), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hidden", shortname: Some(c"hid"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hid), flags_var: ptr::null_mut(), scope_idx: [80, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"highlight", shortname: Some(c"hl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hl), flags_var: ptr::null_mut(), scope_idx: [81, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_highlight), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(HIGHLIGHT_INIT_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"history", shortname: Some(c"hi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hi), flags_var: ptr::null_mut(), scope_idx: [82, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hkmap", shortname: Some(c"hk"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(135), flags_var: ptr::null_mut(), scope_idx: [83, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hkmapp", shortname: Some(c"hkp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(136), flags_var: ptr::null_mut(), scope_idx: [84, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"hlsearch", shortname: Some(c"hls"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_hls), flags_var: ptr::null_mut(), scope_idx: [85, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_hlsearch), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"icon", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_icon), flags_var: ptr::null_mut(), scope_idx: [86, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iconstring", shortname: None, flags: K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_iconstring), flags_var: ptr::null_mut(), scope_idx: [87, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iconstring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ignorecase", shortname: Some(c"ic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ic), flags_var: ptr::null_mut(), scope_idx: [88, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_ignorecase), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imcmdline", shortname: Some(c"imc"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(141), flags_var: ptr::null_mut(), scope_idx: [89, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imdisable", shortname: Some(c"imd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(142), flags_var: ptr::null_mut(), scope_idx: [90, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iminsert", shortname: Some(c"imi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_iminsert), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 44, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_iminsert), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"imsearch", shortname: Some(c"ims"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_imsearch), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 45, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"inccommand", shortname: Some(c"icm"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_icm), flags_var: ptr::null_mut(), scope_idx: [91, -1, -1, -1], immutable: false, values: &OPT_ICM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_inccommand), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"nosplit"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"include", shortname: Some(c"inc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_inc), flags_var: ptr::null_mut(), scope_idx: [92, -1, 46, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"includeexpr", shortname: Some(c"inex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inex), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 47, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"incsearch", shortname: Some(c"is"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_is), flags_var: ptr::null_mut(), scope_idx: [93, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"indentexpr", shortname: Some(c"inde"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inde), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 48, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"indentkeys", shortname: Some(c"indk"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_indk), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 49, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"infercase", shortname: Some(c"inf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_inf), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 50, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"insertmode", shortname: Some(c"im"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(152), flags_var: ptr::null_mut(), scope_idx: [94, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"isfname", shortname: Some(c"isf"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isf), flags_var: ptr::null_mut(), scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,\\,.,-,_,+,,,#,$,%,{,},[,],@-@,!,~,="), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"isfname", shortname: Some(c"isf"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isf), flags_var: ptr::null_mut(), scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,.,-,_,+,,,#,$,%,~,="), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"isident", shortname: Some(c"isi"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_isi), flags_var: ptr::null_mut(), scope_idx: [96, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"iskeyword", shortname: Some(c"isk"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_isk), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 51, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iskeyword), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"isprint", shortname: Some(c"isp"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(156), flags_var: ptr::null_mut(), scope_idx: [97, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"@,161-255"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"joinspaces", shortname: Some(c"js"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_js), flags_var: ptr::null_mut(), scope_idx: [98, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"jumpoptions", shortname: Some(c"jop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_jop), flags_var: opt_flags!(jop_flags), scope_idx: [99, -1, -1, -1], immutable: false, values: &OPT_JOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"clean"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"keymap", shortname: Some(c"kmp"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_NFNAME|K_OPT_FLAG_PRI_MKRC, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_keymap), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 52, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"keymodel", shortname: Some(c"km"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_km), flags_var: ptr::null_mut(), scope_idx: [100, -1, -1, -1], immutable: false, values: &OPT_KM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymodel), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"keywordprg", shortname: Some(c"kp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_kp), flags_var: ptr::null_mut(), scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":help!"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"keywordprg", shortname: Some(c"kp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_kp), flags_var: ptr::null_mut(), scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":Man"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langmap", shortname: Some(c"lmap"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_langmap), flags_var: ptr::null_mut(), scope_idx: [102, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::mapping::did_set_langmap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langmenu", shortname: Some(c"lm"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lm), flags_var: ptr::null_mut(), scope_idx: [103, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langnoremap", shortname: Some(c"lnr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lnr), flags_var: ptr::null_mut(), scope_idx: [104, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langnoremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"langremap", shortname: Some(c"lrm"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lrm), flags_var: ptr::null_mut(), scope_idx: [105, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"laststatus", shortname: Some(c"ls"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ls), flags_var: ptr::null_mut(), scope_idx: [106, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_laststatus), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lazyredraw", shortname: Some(c"lz"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lz), flags_var: ptr::null_mut(), scope_idx: [107, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lhistory", shortname: Some(c"lhi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 23, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"linebreak", shortname: Some(c"lbr"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 24, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lines", shortname: None, flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lines), flags_var: ptr::null_mut(), scope_idx: [108, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(24) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"linespace", shortname: Some(c"lsp"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_linespace), flags_var: ptr::null_mut(), scope_idx: [109, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lisp", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_lisp), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 54, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lisp), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lispoptions", shortname: Some(c"lop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_lop), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 55, -1], immutable: false, values: &OPT_LOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_lispoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"lispwords", shortname: Some(c"lw"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_lispwords), flags_var: ptr::null_mut(), scope_idx: [110, -1, 56, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(LISPWORD_VALUE_CSTR), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"list", shortname: None, flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 25, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"listchars", shortname: Some(c"lcs"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_lcs), flags_var: ptr::null_mut(), scope_idx: [111, 26, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c"tab:> ,trail:-,nbsp:+"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"loadplugins", shortname: Some(c"lpl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_lpl), flags_var: ptr::null_mut(), scope_idx: [112, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"magic", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_magic), flags_var: ptr::null_mut(), scope_idx: [113, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeef", shortname: Some(c"mef"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mef), flags_var: ptr::null_mut(), scope_idx: [114, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeencoding", shortname: Some(c"menc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_menc), flags_var: ptr::null_mut(), scope_idx: [115, -1, 57, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"makeprg", shortname: Some(c"mp"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_mp), flags_var: ptr::null_mut(), scope_idx: [116, -1, 58, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"make"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"matchpairs", shortname: Some(c"mps"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_mps), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 59, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_matchpairs), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"(:),{:},[:]"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"matchtime", shortname: Some(c"mat"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mat), flags_var: ptr::null_mut(), scope_idx: [117, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxcombine", shortname: Some(c"mco"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mco), flags_var: ptr::null_mut(), scope_idx: [118, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(6) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxfuncdepth", shortname: Some(c"mfd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mfd), flags_var: ptr::null_mut(), scope_idx: [119, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(100) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxmapdepth", shortname: Some(c"mmd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mmd), flags_var: ptr::null_mut(), scope_idx: [120, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxmempattern", shortname: Some(c"mmp"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mmp), flags_var: ptr::null_mut(), scope_idx: [121, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"maxsearchcount", shortname: Some(c"msc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_msc), flags_var: ptr::null_mut(), scope_idx: [122, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(999) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"menuitems", shortname: Some(c"mis"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mis), flags_var: ptr::null_mut(), scope_idx: [123, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(25) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"messagesopt", shortname: Some(c"mopt"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mopt), flags_var: ptr::null_mut(), scope_idx: [124, -1, -1, -1], immutable: false, values: &OPT_MOPT_VALUES, schema: &OPT_MOPT_SCHEMA, opt_validate_cb: opt_cb!(crate::message::validate_messagesopt), opt_did_set_cb: opt_cb!(crate::message::did_set_messagesopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"hit-enter,history:500,progress:c"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mkspellmem", shortname: Some(c"msm"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_msm), flags_var: ptr::null_mut(), scope_idx: [125, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mkspellmem), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"460000,2000,500"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modeline", shortname: Some(c"ml"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ml), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 60, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modelineexpr", shortname: Some(c"mle"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mle), flags_var: ptr::null_mut(), scope_idx: [126, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modelines", shortname: Some(c"mls"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mls), flags_var: ptr::null_mut(), scope_idx: [127, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modifiable", shortname: Some(c"ma"), flags: K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ma), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 61, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modifiable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"modified", shortname: Some(c"mod"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_mod), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 62, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modified), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"more", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_more), flags_var: ptr::null_mut(), scope_idx: [128, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mouse", shortname: None, flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mouse), flags_var: ptr::null_mut(), scope_idx: [129, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mouse), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_mouse), def_val: cstr_def_val!(c"nvi"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousefocus", shortname: Some(c"mousef"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousef), flags_var: ptr::null_mut(), scope_idx: [130, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousehide", shortname: Some(c"mh"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mh), flags_var: ptr::null_mut(), scope_idx: [131, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousemodel", shortname: Some(c"mousem"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousem), flags_var: ptr::null_mut(), scope_idx: [132, -1, -1, -1], immutable: false, values: &OPT_MOUSEM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"popup_setpos"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousemoveevent", shortname: Some(c"mousemev"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousemev), flags_var: ptr::null_mut(), scope_idx: [133, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousescroll", shortname: None, flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mousescroll), flags_var: ptr::null_mut(), scope_idx: [134, -1, -1, -1], immutable: false, values: &OPT_MOUSESCROLL_VALUES, schema: &OPT_MOUSESCROLL_SCHEMA, opt_validate_cb: opt_cb!(crate::optionstr::validate_mousescroll), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_mousescroll), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver:3,hor:6"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mouseshape", shortname: Some(c"mouses"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(204), flags_var: ptr::null_mut(), scope_idx: [135, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"mousetime", shortname: Some(c"mouset"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_mouset), flags_var: ptr::null_mut(), scope_idx: [136, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(500) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"nrformats", shortname: Some(c"nf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_nf), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 63, -1], immutable: false, values: &OPT_NF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"bin,hex"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"number", shortname: Some(c"nu"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 27, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"numberwidth", shortname: Some(c"nuw"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 28, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_numberwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(4) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"omnifunc", shortname: Some(c"ofu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ofu), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 64, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"opendevice", shortname: Some(c"odev"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(210), flags_var: ptr::null_mut(), scope_idx: [137, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"operatorfunc", shortname: Some(c"opfunc"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_opfunc), flags_var: ptr::null_mut(), scope_idx: [138, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"packlockfile", shortname: Some(c"plf"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_plf), flags_var: ptr::null_mut(), scope_idx: [139, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"$XDG_CONFIG_HOME/nvim/nvim-pack-lock.json"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"packpath", shortname: Some(c"pp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pp), flags_var: ptr::null_mut(), scope_idx: [140, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"paragraphs", shortname: Some(c"para"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_para), flags_var: ptr::null_mut(), scope_idx: [141, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"IPLPPPQPP TPHPLIPpLpItpplpipbp"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"paste", shortname: None, flags: K_OPT_FLAG_PRI_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_paste), flags_var: ptr::null_mut(), scope_idx: [142, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_paste), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pastetoggle", shortname: Some(c"pt"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(216), flags_var: ptr::null_mut(), scope_idx: [143, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"patchexpr", shortname: Some(c"pex"), flags: K_OPT_FLAG_EXPR|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pex), flags_var: ptr::null_mut(), scope_idx: [144, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"patchmode", shortname: Some(c"pm"), flags: K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pm), flags_var: ptr::null_mut(), scope_idx: [145, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"path", shortname: Some(c"pa"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_path), flags_var: ptr::null_mut(), scope_idx: [146, -1, 65, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".,,"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"preserveindent", shortname: Some(c"pi"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_pi), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 66, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewheight", shortname: Some(c"pvh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pvh), flags_var: ptr::null_mut(), scope_idx: [147, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(12) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewpopup", shortname: Some(c"pvp"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_COLON, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pvp), flags_var: ptr::null_mut(), scope_idx: [148, -1, -1, -1], immutable: false, values: &OPT_PVP_VALUES, schema: &OPT_PVP_SCHEMA, opt_validate_cb: opt_cb!(crate::optionstr::validate_previewpopup), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_popupoption), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"previewwindow", shortname: Some(c"pvw"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 29, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_previewwindow), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"prompt", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(224), flags_var: ptr::null_mut(), scope_idx: [149, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumblend", shortname: Some(c"pb"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pb), flags_var: ptr::null_mut(), scope_idx: [150, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_pumblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumborder", shortname: None, flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pumborder), flags_var: ptr::null_mut(), scope_idx: [151, -1, -1, -1], immutable: false, values: &OPT_PUMBORDER_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumheight", shortname: Some(c"ph"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ph), flags_var: ptr::null_mut(), scope_idx: [152, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pummaxwidth", shortname: Some(c"pmw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pmw), flags_var: ptr::null_mut(), scope_idx: [153, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pumwidth", shortname: Some(c"pw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pw), flags_var: ptr::null_mut(), scope_idx: [154, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(15) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"pyxversion", shortname: Some(c"pyx"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_pyx), flags_var: ptr::null_mut(), scope_idx: [155, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(3) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"quickfixtextfunc", shortname: Some(c"qftf"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_qftf), flags_var: ptr::null_mut(), scope_idx: [156, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"quoteescape", shortname: Some(c"qe"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_qe), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 67, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"\\"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"readonly", shortname: Some(c"ro"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_NO_GLOB, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ro), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 68, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_readonly), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"redrawdebug", shortname: Some(c"rdb"), flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rdb), flags_var: opt_flags!(rdb_flags), scope_idx: [157, -1, -1, -1], immutable: false, values: &OPT_RDB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"redrawtime", shortname: Some(c"rdt"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rdt), flags_var: ptr::null_mut(), scope_idx: [158, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"regexpengine", shortname: Some(c"re"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_re), flags_var: ptr::null_mut(), scope_idx: [159, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"relativenumber", shortname: Some(c"rnu"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 30, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"remap", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(238), flags_var: ptr::null_mut(), scope_idx: [160, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"report", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_report), flags_var: ptr::null_mut(), scope_idx: [161, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"revins", shortname: Some(c"ri"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ri), flags_var: ptr::null_mut(), scope_idx: [162, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rightleft", shortname: Some(c"rl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 31, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rightleftcmd", shortname: Some(c"rlc"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 32, -1, -1], immutable: false, values: &OPT_RLC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"search"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ruler", shortname: Some(c"ru"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ru), flags_var: ptr::null_mut(), scope_idx: [163, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"rulerformat", shortname: Some(c"ruf"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ruf), flags_var: ptr::null_mut(), scope_idx: [164, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_rulerformat), opt_expand_cb: None, def_val: cstr_def_val!(c"%18(%l,%c%V%= %P%)%<"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"runtimepath", shortname: Some(c"rtp"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_rtp), flags_var: ptr::null_mut(), scope_idx: [165, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scroll", shortname: Some(c"scr"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 33, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollback", shortname: Some(c"scbk"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_scbk), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 69, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollback), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollbind", shortname: Some(c"scb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 34, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollbind), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolljump", shortname: Some(c"sj"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sj), flags_var: ptr::null_mut(), scope_idx: [166, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolloff", shortname: Some(c"so"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_so), flags_var: ptr::null_mut(), scope_idx: [167, 35, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrolloffpad", shortname: Some(c"sop"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_sop), flags_var: ptr::null_mut(), scope_idx: [168, 36, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"scrollopt", shortname: Some(c"sbo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sbo), flags_var: ptr::null_mut(), scope_idx: [169, -1, -1, -1], immutable: false, values: &OPT_SBO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver,jump"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sections", shortname: Some(c"sect"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sections), flags_var: ptr::null_mut(), scope_idx: [170, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"SHNHH HUnhsh"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"secure", shortname: None, flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_secure), flags_var: ptr::null_mut(), scope_idx: [171, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"selection", shortname: Some(c"sel"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sel), flags_var: ptr::null_mut(), scope_idx: [172, -1, -1, -1], immutable: false, values: &OPT_SEL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_selection), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"inclusive"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"selectmode", shortname: Some(c"slm"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_slm), flags_var: ptr::null_mut(), scope_idx: [173, -1, -1, -1], immutable: false, values: &OPT_SLM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sessionoptions", shortname: Some(c"ssop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ssop), flags_var: opt_flags!(ssop_flags), scope_idx: [174, -1, -1, -1], immutable: false, values: &OPT_SSOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_sessionoptions), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"blank,buffers,curdir,folds,help,tabpages,winsize,terminal"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shada", shortname: Some(c"sd"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shada), flags_var: ptr::null_mut(), scope_idx: [175, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shada), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"!,'100,<50,s10,h,r/tmp/,r/private/"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shadafile", shortname: Some(c"sdf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shadafile), flags_var: ptr::null_mut(), scope_idx: [176, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shell", shortname: Some(c"sh"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sh), flags_var: ptr::null_mut(), scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"cmd.exe"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shell", shortname: Some(c"sh"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sh), flags_var: ptr::null_mut(), scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"sh"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellcmdflag", shortname: Some(c"shcf"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shcf), flags_var: ptr::null_mut(), scope_idx: [178, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"-c"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellpipe", shortname: Some(c"sp"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sp), flags_var: ptr::null_mut(), scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"2>&1| tee"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellpipe", shortname: Some(c"sp"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sp), flags_var: ptr::null_mut(), scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"| tee"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellquote", shortname: Some(c"shq"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shq), flags_var: ptr::null_mut(), scope_idx: [180, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellredir", shortname: Some(c"srr"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_srr), flags_var: ptr::null_mut(), scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">%s 2>&1"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellredir", shortname: Some(c"srr"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_srr), flags_var: ptr::null_mut(), scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    Vimoption { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ssl), flags_var: ptr::null_mut(), scope_idx: [182, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    Vimoption { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(265), flags_var: ptr::null_mut(), scope_idx: [182, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shelltemp", shortname: Some(c"stmp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_stmp), flags_var: ptr::null_mut(), scope_idx: [183, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellxescape", shortname: Some(c"sxe"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sxe), flags_var: ptr::null_mut(), scope_idx: [184, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shellxquote", shortname: Some(c"sxq"), flags: K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sxq), flags_var: ptr::null_mut(), scope_idx: [185, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shiftround", shortname: Some(c"sr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sr), flags_var: ptr::null_mut(), scope_idx: [186, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shiftwidth", shortname: Some(c"sw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sw), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 70, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"shortmess", shortname: Some(c"shm"), flags: K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_shm), flags_var: ptr::null_mut(), scope_idx: [187, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shortmess), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_shortmess), def_val: cstr_def_val!(c"ltToOCF"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showbreak", shortname: Some(c"sbr"), flags: K_OPT_FLAG_REDR_ALL, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_sbr), flags_var: ptr::null_mut(), scope_idx: [188, 37, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_showbreak), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showcmd", shortname: Some(c"sc"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sc), flags_var: ptr::null_mut(), scope_idx: [189, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showcmdloc", shortname: Some(c"sloc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sloc), flags_var: ptr::null_mut(), scope_idx: [190, -1, -1, -1], immutable: false, values: &OPT_SLOC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_showcmdloc), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"last"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showfulltag", shortname: Some(c"sft"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sft), flags_var: ptr::null_mut(), scope_idx: [191, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showmatch", shortname: Some(c"sm"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sm), flags_var: ptr::null_mut(), scope_idx: [192, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showmode", shortname: Some(c"smd"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_smd), flags_var: ptr::null_mut(), scope_idx: [193, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"showtabline", shortname: Some(c"stal"), flags: K_OPT_FLAG_REDR_ALL|K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_stal), flags_var: ptr::null_mut(), scope_idx: [194, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_showtabline), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sidescroll", shortname: Some(c"ss"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ss), flags_var: ptr::null_mut(), scope_idx: [195, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"sidescrolloff", shortname: Some(c"siso"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_siso), flags_var: ptr::null_mut(), scope_idx: [196, 38, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"signcolumn", shortname: Some(c"scl"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 39, -1, -1], immutable: false, values: &OPT_SCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_signcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_signcolumn), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smartcase", shortname: Some(c"scs"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_scs), flags_var: ptr::null_mut(), scope_idx: [197, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smartindent", shortname: Some(c"si"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_si), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 71, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smarttab", shortname: Some(c"sta"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sta), flags_var: ptr::null_mut(), scope_idx: [198, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"smoothscroll", shortname: Some(c"sms"), flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 40, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_smoothscroll), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"softtabstop", shortname: Some(c"sts"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sts), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 72, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spell", shortname: None, flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 41, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_spell), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellcapcheck", shortname: Some(c"spc"), flags: K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spc), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 73, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellcapcheck), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellcapcheck), opt_expand_cb: None, def_val: cstr_def_val!(c"[.?!]\\_[\\])'\"\\t ]\\+"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellfile", shortname: Some(c"spf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spf), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 74, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellfile), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellfile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spelllang", shortname: Some(c"spl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spl), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 75, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spelllang), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelllang), opt_expand_cb: None, def_val: cstr_def_val!(c"en"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spelloptions", shortname: Some(c"spo"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_spo), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 76, -1], immutable: false, values: &OPT_SPO_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelloptions), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"spellsuggest", shortname: Some(c"sps"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sps), flags_var: ptr::null_mut(), scope_idx: [199, -1, -1, -1], immutable: false, values: &OPT_SPS_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellsuggest), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellsuggest), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"best"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitbelow", shortname: Some(c"sb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sb), flags_var: ptr::null_mut(), scope_idx: [200, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitkeep", shortname: Some(c"spk"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_spk), flags_var: ptr::null_mut(), scope_idx: [201, -1, -1, -1], immutable: false, values: &OPT_SPK_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_splitkeep), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"cursor"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"splitright", shortname: Some(c"spr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_spr), flags_var: ptr::null_mut(), scope_idx: [202, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"startofline", shortname: Some(c"sol"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_sol), flags_var: ptr::null_mut(), scope_idx: [203, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"statuscolumn", shortname: Some(c"stc"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 42, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statuscolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"statusline", shortname: Some(c"stl"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_stl), flags_var: ptr::null_mut(), scope_idx: [204, 43, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statusline), opt_expand_cb: None, def_val: cstr_def_val!(c"%<%f%( %h%w%m%r%{ v:lua.require('vim._core.util').term_exitcode() }%)%= %(%-10S %)%{ &busy > 0 ? '◐ ' : '' }%(%{ luaeval('(package.loaded[''vim.ui''] and vim.api.nvim_get_current_win() == tonumber(vim.g.actual_curwin or -1) and vim.ui.progress_status()) or '''' ')} %)%{% luaeval('(package.loaded[''vim.diagnostic''] and next(vim.diagnostic.count(0)) and vim.diagnostic.status() .. '' '') or '''' ') %}%(%k %)%{% &ruler ? &rulerformat : '' %}"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"suffixes", shortname: Some(c"su"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_su), flags_var: ptr::null_mut(), scope_idx: [205, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".bak,~,.o,.h,.info,.swp,.obj"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"suffixesadd", shortname: Some(c"sua"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_sua), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 77, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"swapfile", shortname: Some(c"swf"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_swf), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 78, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_swapfile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"switchbuf", shortname: Some(c"swb"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_swb), flags_var: opt_flags!(swb_flags), scope_idx: [206, -1, -1, -1], immutable: false, values: &OPT_SWB_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"uselast"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"synmaxcol", shortname: Some(c"smc"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_smc), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 79, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(3000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"syntax", shortname: Some(c"syn"), flags: K_OPT_FLAG_NO_GLOB|K_OPT_FLAG_NFNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_syn), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 80, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabclose", shortname: Some(c"tcl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tcl), flags_var: opt_flags!(tcl_flags), scope_idx: [207, -1, -1, -1], immutable: false, values: &OPT_TCL_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabline", shortname: Some(c"tal"), flags: K_OPT_FLAG_REDR_TABL|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tal), flags_var: ptr::null_mut(), scope_idx: [208, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tabline), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabpagemax", shortname: Some(c"tpm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tpm), flags_var: ptr::null_mut(), scope_idx: [209, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tabstop", shortname: Some(c"ts"), flags: K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_ts), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 81, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagbsearch", shortname: Some(c"tbs"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tbs), flags_var: ptr::null_mut(), scope_idx: [210, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagcase", shortname: Some(c"tc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tc), flags_var: opt_flags!(tc_flags), scope_idx: [211, -1, 82, -1], immutable: false, values: &OPT_TC_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tagcase), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"followic"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagfunc", shortname: Some(c"tfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_tfu), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 83, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"taglength", shortname: Some(c"tl"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tl), flags_var: ptr::null_mut(), scope_idx: [212, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagrelative", shortname: Some(c"tr"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tr), flags_var: ptr::null_mut(), scope_idx: [213, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tags", shortname: Some(c"tag"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tags), flags_var: ptr::null_mut(), scope_idx: [214, -1, 84, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"./tags;,tags"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tagstack", shortname: Some(c"tgst"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tgst), flags_var: ptr::null_mut(), scope_idx: [215, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termbidi", shortname: Some(c"tbidi"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tbidi), flags_var: ptr::null_mut(), scope_idx: [216, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termencoding", shortname: Some(c"tenc"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(317), flags_var: ptr::null_mut(), scope_idx: [217, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termguicolors", shortname: Some(c"tgc"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tgc), flags_var: ptr::null_mut(), scope_idx: [218, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termpastefilter", shortname: Some(c"tpf"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tpf), flags_var: opt_flags!(tpf_flags), scope_idx: [219, -1, -1, -1], immutable: false, values: &OPT_TPF_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"BS,ESC,DEL"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"termsync", shortname: None, flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_termsync), flags_var: ptr::null_mut(), scope_idx: [220, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"terse", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_def_var!(321), flags_var: ptr::null_mut(), scope_idx: [221, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"textwidth", shortname: Some(c"tw"), flags: K_OPT_FLAG_REDR_BUF|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_tw), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 85, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_textwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"thesaurus", shortname: Some(c"tsr"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_NDNAME, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tsr), flags_var: ptr::null_mut(), scope_idx: [222, -1, 86, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"thesaurusfunc", shortname: Some(c"tsrfu"), flags: K_OPT_FLAG_FUNC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_tsrfu), flags_var: ptr::null_mut(), scope_idx: [223, -1, 87, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"tildeop", shortname: Some(c"top"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_to), flags_var: ptr::null_mut(), scope_idx: [224, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"timeout", shortname: Some(c"to"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_timeout), flags_var: ptr::null_mut(), scope_idx: [225, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"timeoutlen", shortname: Some(c"tm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tm), flags_var: ptr::null_mut(), scope_idx: [226, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"title", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_title), flags_var: ptr::null_mut(), scope_idx: [227, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titlelen", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titlelen), flags_var: ptr::null_mut(), scope_idx: [228, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_titlelen), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(85) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titleold", shortname: None, flags: K_OPT_FLAG_NO_MKRC|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titleold), flags_var: ptr::null_mut(), scope_idx: [229, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"titlestring", shortname: None, flags: K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_titlestring), flags_var: ptr::null_mut(), scope_idx: [230, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_titlestring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttimeout", shortname: None, flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ttimeout), flags_var: ptr::null_mut(), scope_idx: [231, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttimeoutlen", shortname: Some(c"ttm"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ttm), flags_var: ptr::null_mut(), scope_idx: [232, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"ttyfast", shortname: Some(c"tf"), flags: K_OPT_FLAG_NO_MKRC, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_tf), flags_var: ptr::null_mut(), scope_idx: [233, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undodir", shortname: Some(c"udir"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_NO_DUP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_udir), flags_var: ptr::null_mut(), scope_idx: [234, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undofile", shortname: Some(c"udf"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_udf), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 88, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undofile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undolevels", shortname: Some(c"ul"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF)) as u8, var: opt_var!(p_ul), flags_var: ptr::null_mut(), scope_idx: [235, -1, 89, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undolevels), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"undoreload", shortname: Some(c"ur"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ur), flags_var: ptr::null_mut(), scope_idx: [236, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"updatecount", shortname: Some(c"uc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_uc), flags_var: ptr::null_mut(), scope_idx: [237, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_updatecount), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(200) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"updatetime", shortname: Some(c"ut"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ut), flags_var: ptr::null_mut(), scope_idx: [238, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(4000) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"varsofttabstop", shortname: Some(c"vsts"), flags: K_OPT_FLAG_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_vsts), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 90, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"vartabstop", shortname: Some(c"vts"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_REDR_BUF, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_vts), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 91, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"verbose", shortname: Some(c"vbs"), flags: K_OPT_FLAG_UIOPTION, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_verbose), flags_var: ptr::null_mut(), scope_idx: [239, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"verbosefile", shortname: Some(c"vfile"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vfile), flags_var: ptr::null_mut(), scope_idx: [240, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_verbosefile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"viewdir", shortname: Some(c"vdir"), flags: K_OPT_FLAG_EXPAND|K_OPT_FLAG_NO_DEF_EXP|K_OPT_FLAG_SECURE, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vdir), flags_var: ptr::null_mut(), scope_idx: [241, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"viewoptions", shortname: Some(c"vop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vop), flags_var: opt_flags!(vop_flags), scope_idx: [242, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"folds,cursor,curdir"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"virtualedit", shortname: Some(c"ve"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_CURSWANT|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_ve), flags_var: opt_flags!(ve_flags), scope_idx: [243, 44, -1, -1], immutable: false, values: &OPT_VE_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_virtualedit), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"visualbell", shortname: Some(c"vb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_vb), flags_var: ptr::null_mut(), scope_idx: [244, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"warn", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_warn), flags_var: ptr::null_mut(), scope_idx: [245, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"whichwrap", shortname: Some(c"ww"), flags: K_OPT_FLAG_COMMA|K_OPT_FLAG_FLAG_LIST, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ww), flags_var: ptr::null_mut(), scope_idx: [246, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_whichwrap), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_whichwrap), def_val: cstr_def_val!(c"b,s"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildchar", shortname: Some(c"wc"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wc), flags_var: ptr::null_mut(), scope_idx: [247, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(9) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildcharm", shortname: Some(c"wcm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wcm), flags_var: ptr::null_mut(), scope_idx: [248, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildignore", shortname: Some(c"wig"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wig), flags_var: ptr::null_mut(), scope_idx: [249, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildignorecase", shortname: Some(c"wic"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wic), flags_var: ptr::null_mut(), scope_idx: [250, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildmenu", shortname: Some(c"wmnu"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmnu), flags_var: ptr::null_mut(), scope_idx: [251, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildmode", shortname: Some(c"wim"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wim), flags_var: ptr::null_mut(), scope_idx: [252, -1, -1, -1], immutable: false, values: &OPT_WIM_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_wildmode), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_wildmode), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"full"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wildoptions", shortname: Some(c"wop"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wop), flags_var: opt_flags!(wop_flags), scope_idx: [253, -1, -1, -1], immutable: false, values: &OPT_WOP_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"pum,tagfile"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winaltkeys", shortname: Some(c"wak"), flags: 0, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wak), flags_var: ptr::null_mut(), scope_idx: [254, -1, -1, -1], immutable: false, values: &OPT_WAK_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu"), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winbar", shortname: Some(c"wbr"), flags: K_OPT_FLAG_REDR_STAT|K_OPT_FLAG_MLE, opt_type: ObjectType::ObjectTypeString, scope_flags: ((1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_WIN)) as u8, var: opt_var!(p_wbr), flags_var: ptr::null_mut(), scope_idx: [255, 45, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winbar), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winblend", shortname: Some(c"winbl"), flags: K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 46, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winborder", shortname: None, flags: K_OPT_FLAG_ONE_COMMA, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_winborder), flags_var: ptr::null_mut(), scope_idx: [256, -1, -1, -1], immutable: false, values: &OPT_WINBORDER_VALUES, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"window", shortname: Some(c"wi"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_window), flags_var: ptr::null_mut(), scope_idx: [257, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_window), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixbuf", shortname: Some(c"wfb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 47, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixheight", shortname: Some(c"wfh"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 48, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winfixwidth", shortname: Some(c"wfw"), flags: K_OPT_FLAG_REDR_STAT, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 49, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winheight", shortname: Some(c"wh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wh), flags_var: ptr::null_mut(), scope_idx: [258, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winhighlight", shortname: Some(c"winhl"), flags: K_OPT_FLAG_ONE_COMMA|K_OPT_FLAG_COLON|K_OPT_FLAG_REDR_WIN|K_OPT_FLAG_HLONLY|K_OPT_FLAG_NO_DUP, opt_type: ObjectType::ObjectTypeString, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 50, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winhighlight), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_winhighlight), def_val: cstr_def_val!(c""), script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winminheight", shortname: Some(c"wmh"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmh), flags_var: ptr::null_mut(), scope_idx: [259, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winminwidth", shortname: Some(c"wmw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wmw), flags_var: ptr::null_mut(), scope_idx: [260, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winpinned", shortname: Some(c"wp"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 51, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"winwidth", shortname: Some(c"wiw"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wiw), flags_var: ptr::null_mut(), scope_idx: [261, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrap", shortname: None, flags: K_OPT_FLAG_REDR_WIN, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_WIN) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 52, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_wrap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrapmargin", shortname: Some(c"wm"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_BUF) as u8, var: opt_var!(p_wm), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 92, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"wrapscan", shortname: Some(c"ws"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_ws), flags_var: ptr::null_mut(), scope_idx: [262, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"write", shortname: None, flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_write), flags_var: ptr::null_mut(), scope_idx: [263, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writeany", shortname: Some(c"wa"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wa), flags_var: ptr::null_mut(), scope_idx: [264, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writebackup", shortname: Some(c"wb"), flags: 0, opt_type: ObjectType::ObjectTypeBoolean, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wb), flags_var: ptr::null_mut(), scope_idx: [265, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    Vimoption { fullname: c"writedelay", shortname: Some(c"wd"), flags: 0, opt_type: ObjectType::ObjectTypeInteger, scope_flags: (1 << K_OPT_SCOPE_GLOBAL) as u8, var: opt_var!(p_wd), flags_var: ptr::null_mut(), scope_idx: [266, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::ObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: Sctx { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
]);

/// Return information for option at `opt_idx` (`get_option`).
pub fn get_option(opt_idx: OptIndex) -> &'static mut Vimoption {
    assert!(opt_idx != K_OPT_INVALID);
    unsafe { &mut (*OPTIONS.as_ptr())[opt_idx as usize] }
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
pub fn find_option_len(name: impl crate::strings::IntoCCharPtr, len: usize) -> OptIndex {
    let name = name.into_c_char_ptr();
    if name.is_null() {
        return K_OPT_INVALID;
    }
    let slice = { crate::memory::cstr_slice(name, len) };
    match OPTION_NAME_MAP.binary_search_by_key(&slice, |&(k, _)| k) {
        Ok(i) => OPTION_NAME_MAP[i].1,
        Err(_) => K_OPT_INVALID,
    }
}

/// Find index for an option.
pub fn find_option(name: impl crate::strings::IntoCCharPtr) -> OptIndex {
    let name = name.into_c_char_ptr();
    find_option_len(name, crate::memory::cstr_len(name))
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
            if o.data.string().data != EMPTY_STRING_OPTION.as_ptr().cast::<c_char>() {
                api_free_string(o.data.string());
            }
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
        return cstr_to_obj(s.as_ptr());
    }
    copy_object(value, None)
}

fn optval_snapshot_varp(opt_idx: OptIndex, varp: *mut c_void) -> Object {
    let value = opt_from_varp(opt_idx, varp);
    let snapshot = optval_snapshot(value);
    optval_free_read(opt_idx, value);
    snapshot
}

fn opt_from_callback(cb: &mut Callback) -> Object {
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
            return cstr_to_obj(str_.as_ptr());
        }
        K_CALLBACK_NONE_CONST => {}
        _ => {}
    }
    cstr_to_obj(c"")
}

fn opt_to_callback(value: Object, is_expr: bool) -> Callback {
    {
        let mut cb = Callback::NONE;
        match value.obj_type {
            ObjectType::ObjectTypeLuaRef => {
                cb.data =
                    crate::eval::typval::CallbackData::lua(api_new_luaref(value.data.luaref()));
            }
            ObjectType::ObjectTypeString => {
                if !is_expr {
                    option_set_callback_func(value.data.string().data, &mut cb);
                } else if value.data.string().size > 0 {
                    let funcref =
                        if let Some(name) = get_scriptlocal_funcname(value.data.string().data) {
                            name
                        } else {
                            crate::memory::cstr_to_cstring(value.data.string().data)
                        };
                    cb.data = crate::eval::typval::CallbackData::expr(funcref);
                }
            }
            _ => {}
        }
        cb
    }
}

/// Creates Object from var pointer.
pub fn opt_from_varp(opt_idx: OptIndex, varp: *mut c_void) -> Object {
    unsafe {
        let opt = get_option(opt_idx);
        if opt.immutable {
            return if opt.opt_type == ObjectType::ObjectTypeString {
                string_obj(cstr_as_string(opt.def_val.data.string().data))
            } else {
                opt.def_val
            };
        }

        if varp.cast::<i32>() == &raw mut curbuf_mut().b_changed {
            return boolean_obj(curbuf_is_changed_fn());
        }

        if is_callback_option(opt_idx) {
            return opt_from_callback(&mut *varp.cast::<Callback>());
        }

        match opt.opt_type {
            ObjectType::ObjectTypeBoolean => {
                opt_from_tristate(tristate_from_int(OptInt::from(*(varp.cast::<i32>()))))
            }
            ObjectType::ObjectTypeInteger => integer_obj(*(varp.cast::<OptInt>())),
            ObjectType::ObjectTypeString => {
                string_obj(cstr_as_string(*(varp.cast::<*mut c_char>())))
            }
            _ => std::process::abort(),
        }
    }
}

/// Stores an option value at its "varp", freeing the previously-stored value.
fn set_option_varp(opt_idx: OptIndex, varp: *mut c_void, value: Object) {
    unsafe {
        let opt = get_option(opt_idx);
        if opt.immutable {
            return;
        }

        if is_callback_option(opt_idx) {
            let cb = varp.cast::<Callback>();
            callback_free(&mut *cb);
            *cb = opt_to_callback(value, (opt.flags & K_OPT_FLAG_EXPR) != 0);
            return;
        }

        match value.obj_type {
            ObjectType::ObjectTypeUnset => {
                assert!(option_is_global_local(opt_idx));
                if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
                    *(varp.cast::<i32>()) = K_NONE;
                } else if option_has_type(opt_idx, ObjectType::ObjectTypeString) {
                    let pp = varp.cast::<*mut c_char>();
                    free_string_option(*pp);
                    *pp = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
                } else if opt_idx == K_OPT_UNDOLEVELS {
                    *(varp.cast::<OptInt>()) = NO_LOCAL_UNDOLEVEL;
                } else {
                    assert!(
                        opt_idx == K_OPT_SCROLLOFF
                            || opt_idx == K_OPT_SCROLLOFFPAD
                            || opt_idx == K_OPT_SIDESCROLLOFF
                    );
                    *(varp.cast::<OptInt>()) = -1;
                }
            }
            ObjectType::ObjectTypeBoolean => {
                assert!(option_has_type(opt_idx, ObjectType::ObjectTypeBoolean));
                *(varp.cast::<i32>()) = i32::from(value.data.boolean());
            }
            ObjectType::ObjectTypeInteger => {
                assert!(option_has_type(opt_idx, ObjectType::ObjectTypeInteger));
                *(varp.cast::<OptInt>()) = value.data.integer();
            }
            ObjectType::ObjectTypeString => {
                assert!(option_has_type(opt_idx, ObjectType::ObjectTypeString));
                let pp = varp.cast::<*mut c_char>();
                free_string_option(*pp);
                *pp = copy_option_val(value.data.string().data);
            }
            _ => std::process::abort(),
        }
    }
}

/// Gets the string form of an option value.
fn optval_to_cstr(o: Object, quote: bool) -> std::ffi::CString {
    {
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
                crate::memory::cstr_from_bytes(s.as_bytes())
            }
            ObjectType::ObjectTypeString => {
                let slice = if o.data.string().data.is_null() || o.data.string().size == 0 {
                    &[][..]
                } else {
                    crate::memory::cstr_slice(o.data.string().data, o.data.string().size)
                };
                if !quote {
                    return crate::memory::cstr_from_bytes(slice);
                }
                let mut out = Vec::with_capacity(slice.len() + 2);
                out.push(b'"');
                out.extend_from_slice(slice);
                out.push(b'"');
                crate::memory::cstr_from_bytes(&out)
            }
            ObjectType::ObjectTypeLuaRef => nlua_funcref_str(o.data.luaref(), false),
            _ => std::process::abort(),
        }
    }
}

/// Converts a structured option (non-scalar Object) to the internal scalar (:set-style) form.
pub fn optval_from_obj(opt_idx: OptIndex, o: Object, op: SetOp, error: &mut bool) -> Object {
    {
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
            ObjectType::ObjectTypeBoolean => {
                option_has_type(opt_idx, ObjectType::ObjectTypeBoolean)
            }
            ObjectType::ObjectTypeInteger => {
                option_has_type(opt_idx, ObjectType::ObjectTypeInteger)
            }
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
            crate::memory::cstr_to_cstring(o.data.string().data)
        } else if o.obj_type == ObjectType::ObjectTypeArray {
            let mut items: Vec<std::ffi::CString> = Vec::with_capacity(o.data.array().size);
            for ai in 0..o.data.array().size {
                let item = o.data.array()[ai];
                if item.obj_type != ObjectType::ObjectTypeString {
                    *error = true;
                    return nil_obj();
                }
                let s_cstr = crate::memory::cstr_from_ptr(item.data.string().data);
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
                let k = dict[di].key;
                let v = dict[di].value;
                if is_flaglist {
                    if !(v.obj_type == ObjectType::ObjectTypeNil
                        || (v.obj_type == ObjectType::ObjectTypeBoolean && !v.data.boolean()))
                    {
                        items.push(crate::memory::cstr_to_cstring(k.data));
                    }
                } else if v.obj_type == ObjectType::ObjectTypeString {
                    let k_bytes = crate::memory::cstr_as_bytes(k.data);
                    let v_bytes = crate::memory::cstr_as_bytes(v.data.string().data);
                    let mut kv = Vec::with_capacity(k_bytes.len() + 1 + v_bytes.len());
                    kv.extend_from_slice(k_bytes);
                    kv.push(b':');
                    kv.extend_from_slice(v_bytes);
                    items.push(crate::memory::cstr_from_bytes(&kv));
                } else if v.obj_type == ObjectType::ObjectTypeInteger {
                    let k_bytes = crate::memory::cstr_as_bytes(k.data);
                    let num_s = format!(":{}", v.data.integer());
                    let mut kv = Vec::with_capacity(k_bytes.len() + num_s.len());
                    kv.extend_from_slice(k_bytes);
                    kv.extend_from_slice(num_s.as_bytes());
                    items.push(crate::memory::cstr_from_bytes(&kv));
                } else if v.obj_type == ObjectType::ObjectTypeBoolean
                    || v.obj_type == ObjectType::ObjectTypeNil
                {
                    if v.obj_type == ObjectType::ObjectTypeBoolean && v.data.boolean() {
                        items.push(crate::memory::cstr_to_cstring(k.data));
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

        cstr_to_obj(str_cstr.as_ptr())
    }
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
    {
        assert!((K_OPT_SCOPE_GLOBAL..K_OPT_SCOPE_SIZE).contains(&scope));
        (get_option(opt_idx).scope_flags & (1u8 << scope)) != 0
    }
}

#[inline]
fn option_is_global_local(opt_idx: OptIndex) -> bool {
    {
        if opt_idx == K_OPT_INVALID {
            return false;
        }
        let bw: OptScopeFlags = (1 << K_OPT_SCOPE_BUF) | (1 << K_OPT_SCOPE_WIN);
        (get_option(opt_idx).scope_flags & bw) != 0 && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL)
    }
}

#[inline]
fn option_is_global_only(opt_idx: OptIndex) -> bool {
    {
        if opt_idx == K_OPT_INVALID {
            return false;
        }
        let bw: OptScopeFlags = (1 << K_OPT_SCOPE_BUF) | (1 << K_OPT_SCOPE_WIN);
        (get_option(opt_idx).scope_flags & bw) == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_GLOBAL)
    }
}

#[inline]
fn option_is_window_local(opt_idx: OptIndex) -> bool {
    {
        if opt_idx == K_OPT_INVALID {
            return false;
        }
        let exclude: OptScopeFlags = (1 << K_OPT_SCOPE_GLOBAL) | (1 << K_OPT_SCOPE_BUF);
        (get_option(opt_idx).scope_flags & exclude) == 0
            && option_has_scope(opt_idx, K_OPT_SCOPE_WIN)
    }
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
    let is_unset = optval_is_unset_local(opt_idx, local_value);
    optval_free_read(opt_idx, local_value);
    is_unset
}

/// Handle side-effects of setting an option.
fn did_set_option(
    args: &mut Optset,
    set_sid: Scid,
    direct: bool,
    value_replaced: bool,
    value_untrusted: bool,
) -> *const c_char {
    unsafe {
        let opt_idx = args.os_idx;
        let varp = args.os_varp;
        let opt_flags = args.os_flags;
        let opt = get_option(opt_idx);
        let mut errmsg: *const c_char = ptr::null();

        if direct {
            // Don't do any extra processing if setting directly.
        } else if (g_st().secure != 0 || g_st().sandbox != 0)
            && (opt.flags & K_OPT_FLAG_SECURE) != 0
        {
            errmsg = E_SECURE.as_ptr();
        } else if args.os_newval.obj_type == ObjectType::ObjectTypeString
            && !is_callback_option(opt_idx)
            && check_illegal_path_names(*(varp.cast::<*mut c_char>()), opt.flags)
        {
            errmsg = E_INVARG.as_ptr();
        } else if let Some(cb) = opt.opt_did_set_cb {
            errmsg = cb(args);
        }

        if !errmsg.is_null() {
            set_option_varp(opt_idx, varp, args.os_oldval);
            optval_free(args.os_oldval);
            return errmsg;
        }

        let new_value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));

        if set_sid != SID_NONE {
            let script_ctx = if set_sid == 0 {
                g_st().current_sctx
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

        optval_free(args.os_oldval);

        let scope_both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;

        if scope_both {
            if option_is_global_local(opt_idx) {
                set_option_varp(opt_idx, get_varp_scope(opt, OPT_LOCAL), unset_obj());
            } else {
                let varp_global = get_varp_scope(opt, OPT_GLOBAL);
                if varp_global != varp {
                    set_option_varp(opt_idx, varp_global, new_value);
                }
            }
        }

        optval_free(new_value);

        if direct {
            return errmsg;
        }

        let ws = curwin_mut().w_s_ptr();
        if varp == (&raw mut curbuf_mut().b_p_syn).cast::<c_void>() {
            do_syntax_autocmd(curbuf_mut(), args.os_value_changed);
        } else if varp == (&raw mut curbuf_mut().b_p_ft).cast::<c_void>() {
            if (opt_flags & OPT_MODELINE) == 0 || args.os_value_changed {
                do_filetype_autocmd(curbuf_mut(), args.os_value_changed);
            }
        } else if varp == (&raw mut ws.b_p_spl).cast::<c_void>() {
            do_spelllang_source(curwin_mut());
        }

        comp_col();

        if varp == (&raw mut opt_vars().p_mouse).cast::<c_void>() {
            setmouse();
        } else if (varp == (&raw mut opt_vars().p_flp).cast::<c_void>()
            || varp == (&raw mut curbuf_mut().b_p_flp).cast::<c_void>())
            && curwin_mut().w_briopt_list != 0
        {
            redraw_all_later(UPD_NOT_VALID);
        } else if varp == (&raw mut opt_vars().p_wbr).cast::<c_void>()
            || varp == (&raw mut curwin_mut().w_onebuf_opt.wo_wbr).cast::<c_void>()
        {
            if varp == (&raw mut opt_vars().p_wbr).cast::<c_void>() {
                set_winbar_all(true);
            } else {
                set_winbar(true);
            }
        }

        if curwin_mut().w_curswant != MAXCOL
            && (opt.flags & (K_OPT_FLAG_CURSWANT | K_OPT_FLAG_REDR_ALL)) != 0
            && (opt.flags & K_OPT_FLAG_HLONLY) == 0
        {
            curwin_mut().w_set_curswant = true;
        }

        check_redraw(opt.flags);

        if errmsg.is_null() {
            opt.flags |= K_OPT_FLAG_WAS_SET;

            let flagsp = insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, opt_flags);
            let flagsp_local = if scope_both {
                insecure_flag(Some(crate::globals::curwin_mut()), opt_idx, OPT_LOCAL)
            } else {
                ptr::null_mut()
            };
            if !args.os_value_checked && value_untrusted {
                *flagsp |= K_OPT_FLAG_INSECURE;
                if !flagsp_local.is_null() {
                    *flagsp_local |= K_OPT_FLAG_INSECURE;
                }
            } else if value_replaced {
                *flagsp &= !K_OPT_FLAG_INSECURE;
                if !flagsp_local.is_null() {
                    *flagsp_local &= !K_OPT_FLAG_INSECURE;
                }
            }
        }

        errmsg
    }
}

/// Validates an option value and prepares any data needed to apply it, without side effects.
pub fn validate_and_prepare_option_value(args: &mut Optset) -> *const c_char {
    unsafe {
        let opt_idx = args.os_idx;
        let newval = &mut args.os_newval;
        let opt_flags = args.os_flags;
        let buf = args.os_buf;
        let win = args.os_win;
        let errbuf = args.os_errbuf;
        let mut errmsg: *const c_char = ptr::null();
        let opt = get_option(opt_idx);

        if (opt_flags & OPT_LOCAL) != 0 && optval_is_unset_local(opt_idx, *newval) {
            return ptr::null();
        }

        if newval.obj_type == ObjectType::ObjectTypeNil {
            if opt_flags == OPT_GLOBAL {
                errmsg = tr(c"Cannot unset global option value");
            } else {
                let varp = get_varp_scope_from(opt, OPT_GLOBAL, &mut *buf, &mut *win);
                *newval = if option_is_global_local(opt_idx) {
                    unset_obj()
                } else {
                    optval_own(opt_idx, opt_from_varp(opt_idx, varp))
                };
            }
        } else if newval.obj_type == ObjectType::ObjectTypeLuaRef {
            assert!(is_callback_option(opt_idx));
        } else if !option_has_type(opt_idx, optval_type(*newval)) {
            let rep = optval_to_cstr(*newval, true);
            let type_str = optval_type_name(opt.opt_type);
            errmsg = if let Some(errbuf) = errbuf {
                vim_snprintf!(
                    errbuf.data,
                    errbuf.size,
                    gettext(c"Invalid value for option '%s': expected %s, got %s %s"),
                    opt.fullname.as_ptr(),
                    type_str,
                    optval_type_name(optval_type(*newval)),
                    rep.as_ptr(),
                );
                errbuf.data.cast_const()
            } else {
                c"".as_ptr()
            };
        } else if newval.obj_type == ObjectType::ObjectTypeInteger {
            errmsg = validate_num_option(opt_idx, newval.data.integer_mut(), &mut *win, errbuf);
        }
        if !errmsg.is_null() {
            return errmsg;
        }

        if opt.immutable {
            let oldval = opt_from_varp(
                opt_idx,
                get_varp_scope_from(opt, opt_flags, &mut *buf, &mut *win),
            );
            let unchanged = option_equal(oldval, *newval);
            optval_free_read(opt_idx, oldval);
            if !unchanged {
                return E_UNSUPPORTEDOPTION.as_ptr();
            }
        }
        let schema = opt.schema;
        if newval.obj_type == ObjectType::ObjectTypeString && !schema.is_empty() {
            errmsg = opt_strings_check(newval.data.string().data, schema, errbuf);
            if !errmsg.is_null() {
                return errmsg;
            }
        }
        if newval.obj_type != ObjectType::ObjectTypeUnset
            && let Some(validate_cb) = opt.opt_validate_cb
        {
            let scope_both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
            args.os_varp = if scope_both && option_is_global_local(opt_idx) {
                opt.var
            } else {
                get_varp_scope_from(opt, opt_flags, &mut *buf, &mut *win)
            };
            args.os_oldval = opt_from_varp(opt_idx, args.os_varp);
            errmsg = validate_cb(args);
            optval_free_read(opt_idx, args.os_oldval);
            args.os_oldval = nil_obj();
            return errmsg;
        }
        ptr::null()
    }
}

/// Set the value of an option using an Object.
fn set_option(
    opt_idx: OptIndex,
    mut value: Object,
    opt_flags: i32,
    set_sid: Scid,
    direct: bool,
    value_replaced: bool,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    unsafe {
        assert!(opt_idx != K_OPT_INVALID);

        let mut errmsg: *const c_char = ptr::null();
        let mut args = Optset {
            os_varp: ptr::null_mut(),
            os_idx: opt_idx,
            os_flags: opt_flags,
            os_oldval: nil_obj(),
            os_newval: value,
            os_prep: crate::optionstr::OptSetPrep::None,
            os_value_checked: false,
            os_value_changed: false,
            os_restore_chartab: false,
            os_errbuf: errbuf,
            os_win: curwin_ptr(),
            os_buf: g_st().curbuf,
        };

        let opt = get_option(opt_idx);
        if !direct {
            if (g_st().secure != 0 || g_st().sandbox != 0) && (opt.flags & K_OPT_FLAG_SECURE) != 0 {
                optval_free(value);
                return E_SECURE.as_ptr();
            }

            errmsg = validate_and_prepare_option_value(&mut args);
            value = args.os_newval;

            if errmsg.is_null()
                && value.obj_type == ObjectType::ObjectTypeString
                && (opt.flags & K_OPT_FLAG_FUNC) != 0
                && value.data.string().size > 0
            {
                let mut cb = Callback::NONE;
                if !option_set_callback_func(value.data.string().data, &mut cb) {
                    errmsg = E_INVARG.as_ptr();
                }
                callback_free(&mut cb);
            }
        } else if value.obj_type == ObjectType::ObjectTypeString && is_dict_option(opt_idx) {
            errmsg = opt_strings_check(
                value.data.string().data,
                opt_dict_schema(opt_idx).unwrap().schema,
                errbuf,
            );
        }

        if !errmsg.is_null() {
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
                let mut p = value.data.string().data;
                while !crate::memory::cstr_is_empty(p) {
                    if crate::memory::cstr_first_byte(p) != b'\\'
                        || (crate::memory::cstr_byte_at(p, 1) == b',' && allow_comma)
                        || (crate::memory::cstr_byte_at(p, 1) == b' ' && allow_space)
                    {
                        p = p.wrapping_add(1);
                        continue;
                    }
                    *p = b'/' as c_char;
                    p = p.wrapping_add(1);
                }
            }
        }

        let scope_local = (opt_flags & OPT_LOCAL) != 0;
        let scope_global = (opt_flags & OPT_GLOBAL) != 0;
        let scope_both = !scope_local && !scope_global;
        let is_opt_local_unset = is_option_local_value_unset(opt_idx);

        let varp = if scope_both && option_is_global_local(opt_idx) {
            opt.var
        } else {
            get_varp_scope(opt, opt_flags)
        };
        let varp_local = get_varp_scope(opt, OPT_LOCAL);
        let varp_global = get_varp_scope(opt, OPT_GLOBAL);

        let old_value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));

        let mut saved_used_value = nil_obj();
        let mut saved_old_global_value = nil_obj();
        let mut saved_old_local_value = nil_obj();
        let mut saved_new_value = nil_obj();
        if !direct {
            saved_old_global_value = optval_snapshot_varp(opt_idx, varp_global);
            saved_old_local_value = if is_opt_local_unset {
                copy_object(saved_old_global_value, None)
            } else {
                optval_snapshot_varp(opt_idx, varp_local)
            };
            saved_used_value = if scope_local && is_opt_local_unset {
                optval_snapshot_varp(opt_idx, get_varp(&mut *opt))
            } else {
                optval_snapshot(old_value)
            };
        }

        let secure_saved = g_st().secure;

        let value_untrusted = g_st().secure != 0
            || g_st().sandbox != 0
            || (opt_flags & OPT_MODELINE) != 0
            || (!value_replaced
                && was_set_insecurely(Some(crate::globals::curwin_mut()), opt_idx, opt_flags) != 0);
        if g_st().secure == 0 && value_untrusted {
            g_st().secure = 1;
        }

        set_option_varp(opt_idx, varp, value);
        if value.obj_type == ObjectType::ObjectTypeUnset {
            value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));
        }
        if !direct {
            saved_new_value = optval_snapshot(value);
        }
        args.os_varp = varp;
        args.os_oldval = old_value;
        args.os_newval = value;
        args.os_buf = g_st().curbuf;
        args.os_win = curwin_ptr();
        errmsg = did_set_option(&mut args, set_sid, direct, value_replaced, value_untrusted);
        args.os_prep = crate::optionstr::OptSetPrep::None;

        g_st().secure = secure_saved;

        if errmsg.is_null() && !direct {
            if starting_val() == 0 {
                apply_optionset_autocmd(
                    opt_idx,
                    opt_flags,
                    saved_used_value,
                    saved_old_global_value,
                    saved_old_local_value,
                    saved_new_value,
                );
            }
            if (opt.flags & K_OPT_FLAG_UIOPTION) != 0 {
                ui_call_option_set(cstr_as_string(opt.fullname.as_ptr()), saved_new_value);
            }
        }

        optval_free(saved_used_value);
        optval_free(saved_old_local_value);
        optval_free(saved_old_global_value);
        optval_free(saved_new_value);
        optval_free(value);

        errmsg
    }
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
    assert!(errmsg.is_null());
}

/// Set option value directly for buffer / window, without processing any side effects.
pub fn set_option_direct_for(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: i32,
    set_sid: Scid,
    target: OptTarget,
) {
    {
        let save_curbuf = g_st().curbuf;
        let save_curwin = curwin_ptr();

        match target {
            OptTarget::Global => {}
            OptTarget::Win(win) => {
                set_curwin(win);
                g_st().curbuf = curwin_mut().w_buffer;
            }
            OptTarget::Buf(buf) => {
                g_st().curbuf = buf;
            }
            OptTarget::Tab(_) => std::process::abort(),
        }

        set_option_direct(opt_idx, value, opt_flags, set_sid);

        set_curwin(save_curwin);
        g_st().curbuf = save_curbuf;
    }
}

/// Sets the value of an (non-tty) option.
pub fn set_option_value(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: i32,
    value_replaced: bool,
    errbuf: Option<CharBuf>,
) -> *const c_char {
    {
        assert!(opt_idx != K_OPT_INVALID);

        let flags = get_option(opt_idx).flags;
        if g_st().sandbox > 0 && (flags & K_OPT_FLAG_SECURE) != 0 {
            return tr(E_SANDBOX);
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
}

#[inline]
fn unset_option_local_value(opt_idx: OptIndex) -> *const c_char {
    assert!(option_is_global_local(opt_idx));
    set_option_value(opt_idx, unset_obj(), OPT_LOCAL, true, None)
}

/// Call set_option_value() and when an error is returned, report it.
pub fn set_option_value_give_err(opt_idx: OptIndex, value: Object, opt_flags: i32) {
    let mut errbuf_storage = [0 as c_char; IOSIZE];
    let errbuf = CharBuf {
        data: errbuf_storage.as_mut_ptr(),
        size: IOSIZE,
    };
    let errmsg = set_option_value(opt_idx, value, opt_flags, true, Some(errbuf));

    if !errmsg.is_null() {
        emsg(tr(errmsg));
    }
}

#[derive(Clone, Copy)]
pub enum OptTarget {
    Global,
    Win(*mut WinInfoInfo),
    Buf(*mut FileBuf),
    Tab(*mut TabPageInfo),
}

enum OptSavedCtx {
    None,
    Switch(CtxSwitch),
    Tab(*mut TabPageInfo),
}

/// Switch current context to get/set option value for the target window/buffer/tabpage.
fn switch_option_context(
    ctx: &mut OptSavedCtx,
    scope: OptScope,
    from: OptTarget,
    err: &mut Error,
) -> bool {
    {
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
                if !ctx_switch(
                    &mut cs,
                    win,
                    win_find_tabpage(win),
                    ptr::null_mut(),
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
                if buf == g_st().curbuf {
                    return false;
                }
                let mut cs = CtxSwitch::new();
                ctx_switch(&mut cs, ptr::null_mut(), ptr::null_mut(), buf, 0);
                *ctx = OptSavedCtx::Switch(cs);
                true
            }
            K_OPT_SCOPE_TAB => {
                let OptTarget::Tab(tab) = from else {
                    return false;
                };
                if tab == g_st().curtab {
                    return false;
                }
                *ctx = OptSavedCtx::Tab(g_st().curtab);
                unuse_tabpage(curtab_mut());
                use_tabpage(tab);
                true
            }
            _ => unreachable!(),
        }
    }
}

fn restore_option_context(ctx: &mut OptSavedCtx, scope: OptScope) {
    {
        match (scope, ctx) {
            (K_OPT_SCOPE_GLOBAL, _) => {}
            (K_OPT_SCOPE_WIN | K_OPT_SCOPE_BUF, OptSavedCtx::Switch(cs)) => {
                ctx_restore(cs);
            }
            (K_OPT_SCOPE_TAB, OptSavedCtx::Tab(saved_curtab)) => {
                unuse_tabpage(curtab_mut());
                use_tabpage(*saved_curtab);
            }
            _ => {}
        }
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
    unsafe {
        if let (K_OPT_SCOPE_TAB, OptTarget::Tab(tab)) = (scope, from)
            && tab != g_st().curtab
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
            (*tab).tp_ch_used = value.data.integer();
            return;
        }

        let mut ctx = OptSavedCtx::None;

        let switched = switch_option_context(&mut ctx, scope, from, err);
        if error_set(err) {
            return;
        }

        let mut errbuf_storage = [0 as c_char; IOSIZE];
        let errbuf = CharBuf {
            data: errbuf_storage.as_mut_ptr(),
            size: IOSIZE,
        };
        let errmsg = set_option_value(opt_idx, value, opt_flags, value_replaced, Some(errbuf));
        if !errmsg.is_null() {
            api_set_error!(err, ErrorType::ErrorTypeException, c"%s", errmsg);
        }

        if switched {
            restore_option_context(&mut ctx, scope);
        }
    }
}

fn showoptions(all: bool, opt_flags: i32) {
    unsafe {
        const INC: i32 = 20;
        const GAP: i32 = 3;

        let mut items: Vec<*mut Vimoption> = Vec::with_capacity(K_OPT_COUNT as usize);

        msg_ext_set_kind(c"list_cmd");
        if (opt_flags & OPT_GLOBAL) != 0 {
            msg_puts_title(tr(c"\n--- Global option values ---"));
        } else if (opt_flags & OPT_LOCAL) != 0 {
            msg_puts_title(tr(c"\n--- Local option values ---"));
        } else {
            msg_puts_title(tr(c"\n--- Options ---"));
        }

        let mut run = 1;
        while run <= 2 && !g_st().got_int {
            items.clear();
            for opt_idx in 0..K_OPT_COUNT {
                let opt = get_option(opt_idx);
                if message_filtered(opt.fullname.as_ptr().cast_mut()) {
                    continue;
                }

                let mut varp: *mut c_void = ptr::null_mut();
                if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) != 0 {
                    if !option_is_global_only(opt_idx) {
                        varp = get_varp_scope(opt, opt_flags);
                    }
                } else {
                    varp = get_varp(&mut *opt);
                }
                if !varp.is_null() && (all || optval_default(opt_idx, varp) == 0) {
                    let len = if (opt_flags & OPT_ONECOLUMN) != 0 {
                        g_st().columns_global
                    } else if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
                        1
                    } else {
                        optval_fmt(opt_idx, opt_flags);
                        (opt.fullname.to_bytes().len() as i32)
                            + vim_strsize(g_st().name_buff.as_ptr())
                            + 1
                    };
                    if (len <= INC - GAP && run == 1) || (len > INC - GAP && run == 2) {
                        items.push(opt);
                    }
                }
            }

            let item_count = items.len();
            let rows = if run == 1 {
                let mut cols = (g_st().columns_global + GAP - 3) / INC;
                if cols == 0 {
                    cols = 1;
                }
                (item_count as i32 + cols - 1) / cols
            } else {
                item_count as i32
            };

            let mut row = 0;
            while row < rows && !g_st().got_int {
                msg_putchar(b'\n' as i32);
                if g_st().got_int {
                    break;
                }
                let mut col: i32 = 0;
                let mut i = row as usize;
                while i < item_count {
                    msg_advance(col);
                    showoneopt(&mut *items[i], opt_flags);
                    col += INC;
                    i += rows as usize;
                }
                os_breakcheck();
                row += 1;
            }
            run += 1;
        }
    }
}

fn optval_default(opt_idx: OptIndex, varp: *mut c_void) -> i32 {
    {
        if is_option_hidden(opt_idx) {
            return 1;
        }

        let current_val = opt_from_varp(opt_idx, varp);
        let default_val = get_option(opt_idx).def_val;

        let is_default = option_equal(current_val, default_val);
        optval_free_read(opt_idx, current_val);
        i32::from(is_default)
    }
}

/// Send update to UIs with values of UI relevant options.
pub fn ui_refresh_options() {
    {
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            if (opt.flags & K_OPT_FLAG_UIOPTION) == 0 {
                continue;
            }
            let name = cstr_as_string(opt.fullname.as_ptr());
            let value = opt_from_varp(opt_idx, opt.var);
            ui_call_option_set(name, value);
        }
        if !opt_vars().p_mouse.is_null() {
            setmouse();
        }
    }
}

fn showoneopt(opt: &mut Vimoption, opt_flags: i32) {
    unsafe {
        let save_silent = g_st().silent_mode;

        g_st().silent_mode = false;
        g_st().info_message = true;

        let opt_idx = get_opt_idx(&raw mut *opt);
        let varp = get_varp_scope(&raw mut *opt, opt_flags);

        if option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
            let val_int = if opt.immutable {
                i32::from(opt.def_val.data.boolean())
            } else if varp.cast::<i32>() == &raw mut curbuf_mut().b_changed {
                i32::from(curbuf_is_changed_fn())
            } else {
                *(varp.cast::<i32>())
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
        msg_puts(opt.fullname.as_ptr());
        if !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean) {
            msg_putchar(b'=' as i32);
            optval_fmt(opt_idx, opt_flags);
            let namebuf_ptr = g_st().name_buff.as_ptr();
            if !crate::memory::cstr_is_empty(namebuf_ptr) {
                msg_outtrans(namebuf_ptr, 0, false);
            }
        }

        g_st().silent_mode = save_silent;
        g_st().info_message = false;
    }
}

/// Write modified options as ":set" commands to a file.
pub fn makeset(fd: *mut FILE, opt_flags: i32, local_only: i32) -> bool {
    unsafe {
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

                    let mut varp = get_varp_scope(opt, opt_flags);
                    if varp.is_null() {
                        continue;
                    }
                    if (opt_flags & OPT_GLOBAL) != 0 && optval_default(opt_idx, varp) != 0 {
                        continue;
                    }

                    if (opt_flags & OPT_SKIPRTP) != 0
                        && (opt.var == (&raw mut opt_vars().p_rtp).cast::<c_void>()
                            || opt.var == (&raw mut opt_vars().p_pp).cast::<c_void>())
                    {
                        continue;
                    }

                    let mut round: i32 = 2;
                    let mut varp_local: *mut c_void = ptr::null_mut();
                    if option_is_window_local(opt_idx) {
                        if (opt_flags & OPT_LOCAL) == 0 {
                            continue;
                        }
                        if (opt_flags & OPT_GLOBAL) == 0 && local_only == 0 {
                            let varp_fresh = get_varp_scope(opt, OPT_GLOBAL);
                            if optval_default(opt_idx, varp_fresh) == 0 {
                                round = 1;
                                varp_local = varp;
                                varp = varp_fresh;
                            }
                        }
                    }

                    while round <= 2 {
                        let cmd = if round == 1 || (opt_flags & OPT_GLOBAL) != 0 {
                            c"set".as_ptr().cast_mut()
                        } else {
                            c"setlocal".as_ptr().cast_mut()
                        };

                        let mut do_endif = false;
                        if opt_idx == K_OPT_SYNTAX || opt_idx == K_OPT_FILETYPE {
                            if fprintf!(
                                fd,
                                c"if &%s != '%s'",
                                opt.fullname.as_ptr(),
                                *(varp.cast::<*mut c_char>()),
                            ) < 0
                                || !put_eol(fd.cast())
                            {
                                return false;
                            }
                            do_endif = true;
                        }
                        if !put_set(fd, cmd, opt_idx, varp) {
                            return false;
                        }
                        if do_endif && !put_line(fd.cast(), c"endif") {
                            return false;
                        }

                        varp = varp_local;
                        round += 1;
                    }
                }
            }
        }
        true
    }
}

/// Generate set commands for the local fold options only.
pub fn makefoldset(fd: *mut FILE) -> bool {
    {
        let cmd = c"setlocal".as_ptr().cast_mut();
        if !put_set(
            fd,
            cmd,
            K_OPT_FOLDMETHOD,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fdm).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDEXPR,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fde).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDMARKER,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fmr).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDIGNORE,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fdi).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDLEVEL,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fdl).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDMINLINES,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fml).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDNESTMAX,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fdn).cast(),
        ) || !put_set(
            fd,
            cmd,
            K_OPT_FOLDENABLE,
            (&raw mut curwin_mut().w_onebuf_opt.wo_fen).cast(),
        ) {
            return false;
        }
        true
    }
}

fn put_set_value(
    fd: *mut FILE,
    cmd: impl crate::strings::IntoCCharPtr,
    opt_idx: OptIndex,
    varp: *mut c_void,
    value: Object,
) -> bool {
    let cmd = cmd.into_c_char_ptr();
    {
        let opt = get_option(opt_idx);
        let name = opt.fullname.as_ptr();
        let flags = opt.flags;

        if varp != opt.var && optval_is_unset_local(opt_idx, value) {
            return true;
        }

        match value.obj_type {
            ObjectType::ObjectTypeBoolean => {
                let value_bool = value.data.boolean();
                if fprintf!(
                    fd,
                    c"%s %s%s",
                    cmd,
                    if value_bool {
                        c"".as_ptr()
                    } else {
                        c"no".as_ptr()
                    },
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
                if wc_use_keyname(varp.cast::<OptInt>(), &mut wc) != 0 {
                    if fputs(get_special_key_name(wc as i32, 0), fd) < 0 {
                        return false;
                    }
                } else {
                    let num_s = format!("{value_num}\0");
                    if fputs(num_s.as_ptr().cast(), fd) < 0 {
                        return false;
                    }
                }
            }
            ObjectType::ObjectTypeString => {
                if fprintf!(fd, c"%s %s=", cmd, name) < 0 {
                    return false;
                }
                let value_str = value.data.string().data;
                if !value_str.is_null() {
                    if (flags & K_OPT_FLAG_EXPAND) != 0 {
                        let size = crate::memory::cstr_len(value_str) + 1;
                        let mut buf_vec = vec![0 as c_char; size];
                        let buf = buf_vec.as_mut_ptr();
                        home_replace(None, value_str, buf, size, false);

                        if size >= MAXPATHL
                            && (flags & K_OPT_FLAG_COMMA) != 0
                            && crate::strings::vim_haschr(value_str, b',' as i32)
                        {
                            let mut part_vec = vec![0 as c_char; size];
                            let part = part_vec.as_mut_ptr();
                            if !put_eol(fd.cast()) {
                                return false;
                            }
                            let mut p = buf;
                            while !crate::memory::cstr_is_empty(p) {
                                if fprintf!(fd, c"%s %s+=", cmd, name) < 0 {
                                    return false;
                                }
                                copy_option_part(&raw mut p, part, size, c",");
                                if !put_escstr(fd.cast(), part, 2) || !put_eol(fd.cast()) {
                                    return false;
                                }
                            }
                            return true;
                        }
                        if !put_escstr(fd.cast(), buf, 2) {
                            return false;
                        }
                    } else if !put_escstr(fd.cast(), value_str, 2) {
                        return false;
                    }
                }
            }
            _ => std::process::abort(),
        }

        if !put_eol(fd.cast()) {
            return false;
        }
        true
    }
}

fn put_set(
    fd: *mut FILE,
    cmd: impl crate::strings::IntoCCharPtr,
    opt_idx: OptIndex,
    varp: *mut c_void,
) -> bool {
    let cmd = cmd.into_c_char_ptr();
    let value = opt_from_varp(opt_idx, varp);
    let ret = if value.obj_type == ObjectType::ObjectTypeLuaRef {
        true
    } else {
        put_set_value(fd, cmd, opt_idx, varp, value)
    };
    optval_free_read(opt_idx, value);
    ret
}

pub fn get_varp_scope_from(
    p: *mut Vimoption,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: &mut WinInfoInfo,
) -> *mut c_void {
    unsafe {
        let opt_idx = get_opt_idx(p);

        if (opt_flags & OPT_GLOBAL) != 0 && !option_is_global_only(opt_idx) {
            if option_is_window_local(opt_idx) {
                return get_varp_from(&mut *p, buf, win)
                    .cast::<u8>()
                    .wrapping_add(std::mem::size_of::<Winopt>())
                    .cast::<c_void>();
            }
            return (*p).var;
        }

        if (opt_flags & OPT_LOCAL) != 0 && option_is_global_local(opt_idx) {
            let ptr: *mut c_void = match opt_idx {
                K_OPT_FORMATPRG => (&raw mut buf.b_p_fp).cast(),
                K_OPT_FSYNC => (&raw mut buf.b_p_fs).cast(),
                K_OPT_FINDFUNC => (&raw mut buf.b_p_ffu).cast(),
                K_OPT_ERRORFORMAT => (&raw mut buf.b_p_efm).cast(),
                K_OPT_GREPFORMAT => (&raw mut buf.b_p_gefm).cast(),
                K_OPT_GREPPRG => (&raw mut buf.b_p_gp).cast(),
                K_OPT_MAKEPRG => (&raw mut buf.b_p_mp).cast(),
                K_OPT_EQUALPRG => (&raw mut buf.b_p_ep).cast(),
                K_OPT_KEYWORDPRG => (&raw mut buf.b_p_kp).cast(),
                K_OPT_PATH => (&raw mut buf.b_p_path).cast(),
                K_OPT_AUTOCOMPLETE => (&raw mut buf.b_p_ac).cast(),
                K_OPT_AUTOREAD => (&raw mut buf.b_p_ar).cast(),
                K_OPT_TAGS => (&raw mut buf.b_p_tags).cast(),
                K_OPT_TAGCASE => (&raw mut buf.b_p_tc).cast(),
                K_OPT_SIDESCROLLOFF => (&raw mut win.w_onebuf_opt.wo_siso).cast(),
                K_OPT_SCROLLOFF => (&raw mut win.w_onebuf_opt.wo_so).cast(),
                K_OPT_SCROLLOFFPAD => (&raw mut win.w_onebuf_opt.wo_sop).cast(),
                K_OPT_DEFINE => (&raw mut buf.b_p_def).cast(),
                K_OPT_INCLUDE => (&raw mut buf.b_p_inc).cast(),
                K_OPT_COMPLETEOPT => (&raw mut buf.b_p_cot).cast(),
                K_OPT_DICTIONARY => (&raw mut buf.b_p_dict).cast(),
                K_OPT_DIFFANCHORS => (&raw mut buf.b_p_dia).cast(),
                K_OPT_THESAURUS => (&raw mut buf.b_p_tsr).cast(),
                K_OPT_THESAURUSFUNC => (&raw mut buf.b_p_tsrfu).cast(),
                K_OPT_TAGFUNC => (&raw mut buf.b_p_tfu).cast(),
                K_OPT_SHOWBREAK => (&raw mut win.w_onebuf_opt.wo_sbr).cast(),
                K_OPT_STATUSLINE => (&raw mut win.w_onebuf_opt.wo_stl).cast(),
                K_OPT_WINBAR => (&raw mut win.w_onebuf_opt.wo_wbr).cast(),
                K_OPT_UNDOLEVELS => (&raw mut buf.b_p_ul).cast(),
                K_OPT_LISPWORDS => (&raw mut buf.b_p_lw).cast(),
                K_OPT_BACKUPCOPY => (&raw mut buf.b_p_bkc).cast(),
                K_OPT_MAKEENCODING => (&raw mut buf.b_p_menc).cast(),
                K_OPT_FILLCHARS => (&raw mut win.w_onebuf_opt.wo_fcs).cast(),
                K_OPT_LISTCHARS => (&raw mut win.w_onebuf_opt.wo_lcs).cast(),
                K_OPT_VIRTUALEDIT => (&raw mut win.w_onebuf_opt.wo_ve).cast(),
                _ => std::process::abort(),
            };
            return ptr;
        }

        get_varp_from(&mut *p, buf, win)
    }
}

/// Get pointer to option variable, depending on local or global scope.
pub fn get_varp_scope(p: *mut Vimoption, opt_flags: i32) -> *mut c_void {
    get_varp_scope_from(p, opt_flags, curbuf_mut(), curwin_mut())
}

/// Get pointer to option variable at 'opt_idx', depending on local or global scope.
pub fn get_option_varp_scope_from(
    opt_idx: OptIndex,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: *mut WinInfoInfo,
) -> *mut c_void {
    get_varp_scope_from(get_option(opt_idx), opt_flags, buf, unsafe { &mut *win })
}

pub fn get_varp_from(p: &mut Vimoption, buf: &mut FileBuf, win: &mut WinInfoInfo) -> *mut c_void {
    {
        let opt_idx = get_opt_idx(&raw mut *p);

        if is_option_hidden(opt_idx) || option_is_global_only(opt_idx) {
            return p.var;
        }

        match opt_idx {
            K_OPT_EQUALPRG => {
                if !crate::memory::cstr_is_empty(buf.b_p_ep) {
                    (&raw mut buf.b_p_ep).cast()
                } else {
                    p.var
                }
            }
            K_OPT_KEYWORDPRG => {
                if !crate::memory::cstr_is_empty(buf.b_p_kp) {
                    (&raw mut buf.b_p_kp).cast()
                } else {
                    p.var
                }
            }
            K_OPT_PATH => {
                if !crate::memory::cstr_is_empty(buf.b_p_path) {
                    (&raw mut buf.b_p_path).cast()
                } else {
                    p.var
                }
            }
            K_OPT_AUTOCOMPLETE => {
                if buf.b_p_ac >= 0 {
                    (&raw mut buf.b_p_ac).cast()
                } else {
                    p.var
                }
            }
            K_OPT_AUTOREAD => {
                if buf.b_p_ar >= 0 {
                    (&raw mut buf.b_p_ar).cast()
                } else {
                    p.var
                }
            }
            K_OPT_TAGS => {
                if !crate::memory::cstr_is_empty(buf.b_p_tags) {
                    (&raw mut buf.b_p_tags).cast()
                } else {
                    p.var
                }
            }
            K_OPT_TAGCASE => {
                if !crate::memory::cstr_is_empty(buf.b_p_tc) {
                    (&raw mut buf.b_p_tc).cast()
                } else {
                    p.var
                }
            }
            K_OPT_SIDESCROLLOFF => {
                if win.w_onebuf_opt.wo_siso >= 0 {
                    (&raw mut win.w_onebuf_opt.wo_siso).cast()
                } else {
                    p.var
                }
            }
            K_OPT_SCROLLOFF => {
                if win.w_onebuf_opt.wo_so >= 0 {
                    (&raw mut win.w_onebuf_opt.wo_so).cast()
                } else {
                    p.var
                }
            }
            K_OPT_SCROLLOFFPAD => {
                if win.w_onebuf_opt.wo_sop >= 0 {
                    (&raw mut win.w_onebuf_opt.wo_sop).cast()
                } else {
                    p.var
                }
            }
            K_OPT_BACKUPCOPY => {
                if !crate::memory::cstr_is_empty(buf.b_p_bkc) {
                    (&raw mut buf.b_p_bkc).cast()
                } else {
                    p.var
                }
            }
            K_OPT_DEFINE => {
                if !crate::memory::cstr_is_empty(buf.b_p_def) {
                    (&raw mut buf.b_p_def).cast()
                } else {
                    p.var
                }
            }
            K_OPT_INCLUDE => {
                if !crate::memory::cstr_is_empty(buf.b_p_inc) {
                    (&raw mut buf.b_p_inc).cast()
                } else {
                    p.var
                }
            }
            K_OPT_COMPLETEOPT => {
                if !crate::memory::cstr_is_empty(buf.b_p_cot) {
                    (&raw mut buf.b_p_cot).cast()
                } else {
                    p.var
                }
            }
            K_OPT_DICTIONARY => {
                if !crate::memory::cstr_is_empty(buf.b_p_dict) {
                    (&raw mut buf.b_p_dict).cast()
                } else {
                    p.var
                }
            }
            K_OPT_DIFFANCHORS => {
                if !crate::memory::cstr_is_empty(buf.b_p_dia) {
                    (&raw mut buf.b_p_dia).cast()
                } else {
                    p.var
                }
            }
            K_OPT_THESAURUS => {
                if !crate::memory::cstr_is_empty(buf.b_p_tsr) {
                    (&raw mut buf.b_p_tsr).cast()
                } else {
                    p.var
                }
            }
            K_OPT_THESAURUSFUNC => {
                if buf.b_p_tsrfu.cb_type() != K_CALLBACK_NONE_CONST {
                    (&raw mut buf.b_p_tsrfu).cast()
                } else {
                    p.var
                }
            }
            K_OPT_FORMATPRG => {
                if !crate::memory::cstr_is_empty(buf.b_p_fp) {
                    (&raw mut buf.b_p_fp).cast()
                } else {
                    p.var
                }
            }
            K_OPT_FSYNC => {
                if buf.b_p_fs >= 0 {
                    (&raw mut buf.b_p_fs).cast()
                } else {
                    p.var
                }
            }
            K_OPT_FINDFUNC => {
                if buf.b_p_ffu.cb_type() != K_CALLBACK_NONE_CONST {
                    (&raw mut buf.b_p_ffu).cast()
                } else {
                    p.var
                }
            }
            K_OPT_ERRORFORMAT => {
                if !crate::memory::cstr_is_empty(buf.b_p_efm) {
                    (&raw mut buf.b_p_efm).cast()
                } else {
                    p.var
                }
            }
            K_OPT_GREPFORMAT => {
                if !crate::memory::cstr_is_empty(buf.b_p_gefm) {
                    (&raw mut buf.b_p_gefm).cast()
                } else {
                    p.var
                }
            }
            K_OPT_GREPPRG => {
                if !crate::memory::cstr_is_empty(buf.b_p_gp) {
                    (&raw mut buf.b_p_gp).cast()
                } else {
                    p.var
                }
            }
            K_OPT_MAKEPRG => {
                if !crate::memory::cstr_is_empty(buf.b_p_mp) {
                    (&raw mut buf.b_p_mp).cast()
                } else {
                    p.var
                }
            }
            K_OPT_SHOWBREAK => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_sbr) {
                    (&raw mut win.w_onebuf_opt.wo_sbr).cast()
                } else {
                    p.var
                }
            }
            K_OPT_STATUSLINE => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_stl) {
                    (&raw mut win.w_onebuf_opt.wo_stl).cast()
                } else {
                    p.var
                }
            }
            K_OPT_WINBAR => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_wbr) {
                    (&raw mut win.w_onebuf_opt.wo_wbr).cast()
                } else {
                    p.var
                }
            }
            K_OPT_UNDOLEVELS => {
                if buf.b_p_ul != NO_LOCAL_UNDOLEVEL {
                    (&raw mut buf.b_p_ul).cast()
                } else {
                    p.var
                }
            }
            K_OPT_LISPWORDS => {
                if !crate::memory::cstr_is_empty(buf.b_p_lw) {
                    (&raw mut buf.b_p_lw).cast()
                } else {
                    p.var
                }
            }
            K_OPT_MAKEENCODING => {
                if !crate::memory::cstr_is_empty(buf.b_p_menc) {
                    (&raw mut buf.b_p_menc).cast()
                } else {
                    p.var
                }
            }
            K_OPT_FILLCHARS => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_fcs) {
                    (&raw mut win.w_onebuf_opt.wo_fcs).cast()
                } else {
                    p.var
                }
            }
            K_OPT_LISTCHARS => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_lcs) {
                    (&raw mut win.w_onebuf_opt.wo_lcs).cast()
                } else {
                    p.var
                }
            }
            K_OPT_VIRTUALEDIT => {
                if !crate::memory::cstr_is_empty(win.w_onebuf_opt.wo_ve) {
                    (&raw mut win.w_onebuf_opt.wo_ve).cast()
                } else {
                    p.var
                }
            }

            K_OPT_ARABIC => (&raw mut win.w_onebuf_opt.wo_arab).cast(),
            K_OPT_LIST => (&raw mut win.w_onebuf_opt.wo_list).cast(),
            K_OPT_SPELL => (&raw mut win.w_onebuf_opt.wo_spell).cast(),
            K_OPT_CURSORCOLUMN => (&raw mut win.w_onebuf_opt.wo_cuc).cast(),
            K_OPT_CURSORLINE => (&raw mut win.w_onebuf_opt.wo_cul).cast(),
            K_OPT_CURSORLINEOPT => (&raw mut win.w_onebuf_opt.wo_culopt).cast(),
            K_OPT_COLORCOLUMN => (&raw mut win.w_onebuf_opt.wo_cc).cast(),
            K_OPT_DIFF => (&raw mut win.w_onebuf_opt.wo_diff).cast(),
            K_OPT_EVENTIGNOREWIN => (&raw mut win.w_onebuf_opt.wo_eiw).cast(),
            K_OPT_FOLDCOLUMN => (&raw mut win.w_onebuf_opt.wo_fdc).cast(),
            K_OPT_FOLDENABLE => (&raw mut win.w_onebuf_opt.wo_fen).cast(),
            K_OPT_FOLDIGNORE => (&raw mut win.w_onebuf_opt.wo_fdi).cast(),
            K_OPT_FOLDLEVEL => (&raw mut win.w_onebuf_opt.wo_fdl).cast(),
            K_OPT_FOLDMETHOD => (&raw mut win.w_onebuf_opt.wo_fdm).cast(),
            K_OPT_FOLDMINLINES => (&raw mut win.w_onebuf_opt.wo_fml).cast(),
            K_OPT_FOLDNESTMAX => (&raw mut win.w_onebuf_opt.wo_fdn).cast(),
            K_OPT_FOLDEXPR => (&raw mut win.w_onebuf_opt.wo_fde).cast(),
            K_OPT_FOLDTEXT => (&raw mut win.w_onebuf_opt.wo_fdt).cast(),
            K_OPT_FOLDMARKER => (&raw mut win.w_onebuf_opt.wo_fmr).cast(),
            K_OPT_NUMBER => (&raw mut win.w_onebuf_opt.wo_nu).cast(),
            K_OPT_RELATIVENUMBER => (&raw mut win.w_onebuf_opt.wo_rnu).cast(),
            K_OPT_NUMBERWIDTH => (&raw mut win.w_onebuf_opt.wo_nuw).cast(),
            K_OPT_WINFIXBUF => (&raw mut win.w_onebuf_opt.wo_wfb).cast(),
            K_OPT_WINFIXHEIGHT => (&raw mut win.w_onebuf_opt.wo_wfh).cast(),
            K_OPT_WINFIXWIDTH => (&raw mut win.w_onebuf_opt.wo_wfw).cast(),
            K_OPT_WINPINNED => (&raw mut win.w_onebuf_opt.wo_wp).cast(),
            K_OPT_PREVIEWWINDOW => (&raw mut win.w_onebuf_opt.wo_pvw).cast(),
            K_OPT_LHISTORY => (&raw mut win.w_onebuf_opt.wo_lhi).cast(),
            K_OPT_RIGHTLEFT => (&raw mut win.w_onebuf_opt.wo_rl).cast(),
            K_OPT_RIGHTLEFTCMD => (&raw mut win.w_onebuf_opt.wo_rlc).cast(),
            K_OPT_SCROLL => (&raw mut win.w_onebuf_opt.wo_scr).cast(),
            K_OPT_SMOOTHSCROLL => (&raw mut win.w_onebuf_opt.wo_sms).cast(),
            K_OPT_WRAP => (&raw mut win.w_onebuf_opt.wo_wrap).cast(),
            K_OPT_LINEBREAK => (&raw mut win.w_onebuf_opt.wo_lbr).cast(),
            K_OPT_BREAKINDENT => (&raw mut win.w_onebuf_opt.wo_bri).cast(),
            K_OPT_BREAKINDENTOPT => (&raw mut win.w_onebuf_opt.wo_briopt).cast(),
            K_OPT_SCROLLBIND => (&raw mut win.w_onebuf_opt.wo_scb).cast(),
            K_OPT_CURSORBIND => (&raw mut win.w_onebuf_opt.wo_crb).cast(),
            K_OPT_CONCEALCURSOR => (&raw mut win.w_onebuf_opt.wo_cocu).cast(),
            K_OPT_CONCEALLEVEL => (&raw mut win.w_onebuf_opt.wo_cole).cast(),

            K_OPT_AUTOINDENT => (&raw mut buf.b_p_ai).cast(),
            K_OPT_BINARY => (&raw mut buf.b_p_bin).cast(),
            K_OPT_BOMB => (&raw mut buf.b_p_bomb).cast(),
            K_OPT_BUFHIDDEN => (&raw mut buf.b_p_bh).cast(),
            K_OPT_BUFTYPE => (&raw mut buf.b_p_bt).cast(),
            K_OPT_BUFLISTED => (&raw mut buf.b_p_bl).cast(),
            K_OPT_BUSY => (&raw mut buf.b_p_busy).cast(),
            K_OPT_CHANNEL => (&raw mut buf.b_p_channel).cast(),
            K_OPT_COPYINDENT => (&raw mut buf.b_p_ci).cast(),
            K_OPT_CINDENT => (&raw mut buf.b_p_cin).cast(),
            K_OPT_CINKEYS => (&raw mut buf.b_p_cink).cast(),
            K_OPT_CINOPTIONS => (&raw mut buf.b_p_cino).cast(),
            K_OPT_CINSCOPEDECLS => (&raw mut buf.b_p_cinsd).cast(),
            K_OPT_CINWORDS => (&raw mut buf.b_p_cinw).cast(),
            K_OPT_COMMENTS => (&raw mut buf.b_p_com).cast(),
            K_OPT_COMMENTSTRING => (&raw mut buf.b_p_cms).cast(),
            K_OPT_COMPLETE => (&raw mut buf.b_p_cpt).cast(),
            #[cfg(windows)]
            K_OPT_COMPLETESLASH => (&raw mut buf.b_p_csl).cast(),
            K_OPT_COMPLETEFUNC => (&raw mut buf.b_p_cfu).cast(),
            K_OPT_OMNIFUNC => (&raw mut buf.b_p_ofu).cast(),
            K_OPT_ENDOFFILE => (&raw mut buf.b_p_eof).cast(),
            K_OPT_ENDOFLINE => (&raw mut buf.b_p_eol).cast(),
            K_OPT_FIXENDOFLINE => (&raw mut buf.b_p_fixeol).cast(),
            K_OPT_EXPANDTAB => (&raw mut buf.b_p_et).cast(),
            K_OPT_FILEENCODING => (&raw mut buf.b_p_fenc).cast(),
            K_OPT_FILEFORMAT => (&raw mut buf.b_p_ff).cast(),
            K_OPT_FILETYPE => (&raw mut buf.b_p_ft).cast(),
            K_OPT_FOLLOW => (&raw mut buf.b_p_follow).cast(),
            K_OPT_FORMATOPTIONS => (&raw mut buf.b_p_fo).cast(),
            K_OPT_FORMATLISTPAT => (&raw mut buf.b_p_flp).cast(),
            K_OPT_IMINSERT => (&raw mut buf.b_p_iminsert).cast(),
            K_OPT_IMSEARCH => (&raw mut buf.b_p_imsearch).cast(),
            K_OPT_INFERCASE => (&raw mut buf.b_p_inf).cast(),
            K_OPT_ISKEYWORD => (&raw mut buf.b_p_isk).cast(),
            K_OPT_INCLUDEEXPR => (&raw mut buf.b_p_inex).cast(),
            K_OPT_INDENTEXPR => (&raw mut buf.b_p_inde).cast(),
            K_OPT_INDENTKEYS => (&raw mut buf.b_p_indk).cast(),
            K_OPT_FORMATEXPR => (&raw mut buf.b_p_fex).cast(),
            K_OPT_LISP => (&raw mut buf.b_p_lisp).cast(),
            K_OPT_LISPOPTIONS => (&raw mut buf.b_p_lop).cast(),
            K_OPT_MODELINE => (&raw mut buf.b_p_ml).cast(),
            K_OPT_MATCHPAIRS => (&raw mut buf.b_p_mps).cast(),
            K_OPT_MODIFIABLE => (&raw mut buf.b_p_ma).cast(),
            K_OPT_MODIFIED => (&raw mut buf.b_changed).cast(),
            K_OPT_NRFORMATS => (&raw mut buf.b_p_nf).cast(),
            K_OPT_PRESERVEINDENT => (&raw mut buf.b_p_pi).cast(),
            K_OPT_QUOTEESCAPE => (&raw mut buf.b_p_qe).cast(),
            K_OPT_READONLY => (&raw mut buf.b_p_ro).cast(),
            K_OPT_SCROLLBACK => (&raw mut buf.b_p_scbk).cast(),
            K_OPT_SMARTINDENT => (&raw mut buf.b_p_si).cast(),
            K_OPT_SOFTTABSTOP => (&raw mut buf.b_p_sts).cast(),
            K_OPT_SUFFIXESADD => (&raw mut buf.b_p_sua).cast(),
            K_OPT_SWAPFILE => (&raw mut buf.b_p_swf).cast(),
            K_OPT_SYNMAXCOL => (&raw mut buf.b_p_smc).cast(),
            K_OPT_SYNTAX => (&raw mut buf.b_p_syn).cast(),
            K_OPT_SPELLCAPCHECK => {
                let ws = win.w_s_ptr();
                (&raw mut ws.b_p_spc).cast()
            }
            K_OPT_SPELLFILE => {
                let ws = win.w_s_ptr();
                (&raw mut ws.b_p_spf).cast()
            }
            K_OPT_SPELLLANG => {
                let ws = win.w_s_ptr();
                (&raw mut ws.b_p_spl).cast()
            }
            K_OPT_SPELLOPTIONS => {
                let ws = win.w_s_ptr();
                (&raw mut ws.b_p_spo).cast()
            }
            K_OPT_SHIFTWIDTH => (&raw mut buf.b_p_sw).cast(),
            K_OPT_TAGFUNC => (&raw mut buf.b_p_tfu).cast(),
            K_OPT_TABSTOP => (&raw mut buf.b_p_ts).cast(),
            K_OPT_TEXTWIDTH => (&raw mut buf.b_p_tw).cast(),
            K_OPT_UNDOFILE => (&raw mut buf.b_p_udf).cast(),
            K_OPT_WRAPMARGIN => (&raw mut buf.b_p_wm).cast(),
            K_OPT_VARSOFTTABSTOP => (&raw mut buf.b_p_vsts).cast(),
            K_OPT_VARTABSTOP => (&raw mut buf.b_p_vts).cast(),
            K_OPT_KEYMAP => (&raw mut buf.b_p_keymap).cast(),
            K_OPT_SIGNCOLUMN => (&raw mut win.w_onebuf_opt.wo_scl).cast(),
            K_OPT_WINHIGHLIGHT => (&raw mut win.w_onebuf_opt.wo_winhl).cast(),
            K_OPT_WINBLEND => (&raw mut win.w_onebuf_opt.wo_winbl).cast(),
            K_OPT_STATUSCOLUMN => (&raw mut win.w_onebuf_opt.wo_stc).cast(),
            _ => {
                iemsg(tr(c"E356: get_varp ERROR"));
                (&raw mut buf.b_p_wm).cast()
            }
        }
    }
}

/// Get the value of 'equalprg', either the buffer-local one or the global one.
pub fn get_equalprg() -> *mut c_char {
    {
        if crate::memory::cstr_is_empty(curbuf_mut().b_p_ep) {
            opt_vars().p_ep
        } else {
            curbuf_mut().b_p_ep
        }
    }
}

/// Get the value of 'findfunc', either the buffer-local one or the global one.
pub fn get_findfunc() -> &'static mut Callback {
    if curbuf_mut().b_p_ffu.cb_type() != K_CALLBACK_NONE_CONST {
        &mut curbuf_mut().b_p_ffu
    } else {
        &mut opt_vars().p_ffu
    }
}

/// Copy options from one window to another.
pub fn win_copy_options(wp_from: &mut WinInfoInfo, wp_to: &mut WinInfoInfo) {
    copy_winopt(&mut wp_from.w_onebuf_opt, &mut wp_to.w_onebuf_opt);
    copy_winopt(&mut wp_from.w_allbuf_opt, &mut wp_to.w_allbuf_opt);
    didset_window_options(&mut *wp_to, true);
}

fn copy_option_val(val: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let val = val.into_c_char_ptr();
    let empty_opt = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
    if ptr::eq(val, empty_opt) {
        empty_opt
    } else {
        alloc_string_option(val)
    }
}

/// Copy the options from one Winopt to another.
pub fn copy_winopt(from: &mut Winopt, to: &mut Winopt) {
    let empty_opt = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
    to.wo_arab = from.wo_arab;
    to.wo_list = from.wo_list;
    to.wo_lcs = copy_option_val(from.wo_lcs);
    to.wo_fcs = copy_option_val(from.wo_fcs);
    to.wo_nu = from.wo_nu;
    to.wo_rnu = from.wo_rnu;
    to.wo_ve = copy_option_val(from.wo_ve);
    to.wo_ve_flags = from.wo_ve_flags;
    to.wo_nuw = from.wo_nuw;
    to.wo_rl = from.wo_rl;
    to.wo_rlc = copy_option_val(from.wo_rlc);
    to.wo_sbr = copy_option_val(from.wo_sbr);
    to.wo_stl = copy_option_val(from.wo_stl);
    to.wo_wbr = copy_option_val(from.wo_wbr);
    to.wo_wrap = from.wo_wrap;
    to.wo_wrap_save = from.wo_wrap_save;
    to.wo_lbr = from.wo_lbr;
    to.wo_bri = from.wo_bri;
    to.wo_briopt = copy_option_val(from.wo_briopt);
    to.wo_scb = from.wo_scb;
    to.wo_scb_save = from.wo_scb_save;
    to.wo_sms = from.wo_sms;
    to.wo_crb = from.wo_crb;
    to.wo_crb_save = from.wo_crb_save;
    to.wo_siso = from.wo_siso;
    to.wo_so = from.wo_so;
    to.wo_sop = from.wo_sop;
    to.wo_spell = from.wo_spell;
    to.wo_cuc = from.wo_cuc;
    to.wo_cul = from.wo_cul;
    to.wo_culopt = copy_option_val(from.wo_culopt);
    to.wo_cc = copy_option_val(from.wo_cc);
    to.wo_diff = from.wo_diff;
    to.wo_diff_saved = from.wo_diff_saved;
    to.wo_eiw = copy_option_val(from.wo_eiw);
    to.wo_cocu = copy_option_val(from.wo_cocu);
    to.wo_cole = from.wo_cole;
    to.wo_fdc = copy_option_val(from.wo_fdc);
    to.wo_fdc_save = if from.wo_diff_saved != 0 {
        alloc_string_option(from.wo_fdc_save)
    } else {
        empty_opt
    };
    to.wo_fen = from.wo_fen;
    to.wo_fen_save = from.wo_fen_save;
    to.wo_fdi = copy_option_val(from.wo_fdi);
    to.wo_fml = from.wo_fml;
    to.wo_fdl = from.wo_fdl;
    to.wo_fdl_save = from.wo_fdl_save;
    to.wo_fdm = copy_option_val(from.wo_fdm);
    to.wo_fdm_save = if from.wo_diff_saved != 0 {
        alloc_string_option(from.wo_fdm_save)
    } else {
        empty_opt
    };
    to.wo_fdn = from.wo_fdn;
    callback_copy(&mut to.wo_fde, &from.wo_fde);
    callback_copy(&mut to.wo_fdt, &from.wo_fdt);
    to.wo_fmr = copy_option_val(from.wo_fmr);
    to.wo_scl = copy_option_val(from.wo_scl);
    to.wo_lhi = from.wo_lhi;
    to.wo_winhl = copy_option_val(from.wo_winhl);
    to.wo_winbl = from.wo_winbl;
    to.wo_stc = copy_option_val(from.wo_stc);

    to.wo_wrap_flags = from.wo_wrap_flags;
    to.wo_stl_flags = from.wo_stl_flags;
    to.wo_wbr_flags = from.wo_wbr_flags;
    to.wo_fde_flags = from.wo_fde_flags;
    to.wo_fdt_flags = from.wo_fdt_flags;

    to.wo_script_ctx = from.wo_script_ctx;
    check_winopt(to);
}

fn check_win_options(win: &mut WinInfoInfo) {
    check_winopt(&mut win.w_onebuf_opt);
    check_winopt(&mut win.w_allbuf_opt);
}

fn check_winopt(wop: &mut Winopt) {
    check_string_option(&raw mut wop.wo_fdc);
    check_string_option(&raw mut wop.wo_fdc_save);
    check_string_option(&raw mut wop.wo_fdi);
    check_string_option(&raw mut wop.wo_fdm);
    check_string_option(&raw mut wop.wo_fdm_save);
    check_string_option(&raw mut wop.wo_fmr);
    check_string_option(&raw mut wop.wo_eiw);
    check_string_option(&raw mut wop.wo_scl);
    check_string_option(&raw mut wop.wo_rlc);
    check_string_option(&raw mut wop.wo_sbr);
    check_string_option(&raw mut wop.wo_stl);
    check_string_option(&raw mut wop.wo_culopt);
    check_string_option(&raw mut wop.wo_cc);
    check_string_option(&raw mut wop.wo_cocu);
    check_string_option(&raw mut wop.wo_briopt);
    check_string_option(&raw mut wop.wo_winhl);
    check_string_option(&raw mut wop.wo_lcs);
    check_string_option(&raw mut wop.wo_fcs);
    check_string_option(&raw mut wop.wo_ve);
    check_string_option(&raw mut wop.wo_wbr);
    check_string_option(&raw mut wop.wo_stc);
}

/// Free the allocated memory inside a Winopt.
pub fn clear_winopt(wop: &mut Winopt) {
    clear_string_option(&raw mut wop.wo_fdc);
    clear_string_option(&raw mut wop.wo_fdc_save);
    clear_string_option(&raw mut wop.wo_fdi);
    clear_string_option(&raw mut wop.wo_fdm);
    clear_string_option(&raw mut wop.wo_fdm_save);
    callback_free(&mut wop.wo_fde);
    callback_free(&mut wop.wo_fdt);
    clear_string_option(&raw mut wop.wo_fmr);
    clear_string_option(&raw mut wop.wo_eiw);
    clear_string_option(&raw mut wop.wo_scl);
    clear_string_option(&raw mut wop.wo_rlc);
    clear_string_option(&raw mut wop.wo_sbr);
    clear_string_option(&raw mut wop.wo_stl);
    clear_string_option(&raw mut wop.wo_culopt);
    clear_string_option(&raw mut wop.wo_cc);
    clear_string_option(&raw mut wop.wo_cocu);
    clear_string_option(&raw mut wop.wo_briopt);
    clear_string_option(&raw mut wop.wo_winhl);
    clear_string_option(&raw mut wop.wo_lcs);
    clear_string_option(&raw mut wop.wo_fcs);
    clear_string_option(&raw mut wop.wo_ve);
    clear_string_option(&raw mut wop.wo_wbr);
    clear_string_option(&raw mut wop.wo_stc);
}

pub fn didset_window_options(wp: &mut WinInfoInfo, valid_cursor: bool) {
    {
        if wp.w_onebuf_opt.wo_wrap != 0 {
            wp.w_leftcol = 0;
        } else {
            wp.w_skipcol = 0;
        }
        check_colorcolumn(ptr::null_mut(), Some(&mut *wp));
        briopt_check(Some(&mut *wp));
        let mut culopt_flags: u32 = 0;
        if fill_culopt_flags(wp.w_onebuf_opt.wo_culopt, &mut culopt_flags) {
            wp.w_p_culopt_flags = culopt_flags as u8;
        }
        set_chars_option(
            wp,
            wp.w_onebuf_opt.wo_fcs,
            CharsOption::Fillchars,
            true,
            None,
        );
        set_chars_option(
            wp,
            wp.w_onebuf_opt.wo_lcs,
            CharsOption::Listchars,
            true,
            None,
        );
        parse_winhl_opt(ptr::null(), Some(&mut *wp));
        check_blending(wp);
        set_winbar_win(wp, false, valid_cursor);
        check_signcolumn(ptr::null_mut(), Some(&mut *wp));
        wp.w_grid_alloc.blending = wp.w_onebuf_opt.wo_winbl > 0;
    }
}

#[inline]
fn copy_opt_sctx(buf: &mut FileBuf, bv: usize) {
    {
        buf.b_p_script_ctx[bv] = get_option(BUF_OPT_IDX[bv]).script_ctx;
    }
}

#[inline]
fn copy_opt_insecure(bv: usize) -> u32 {
    get_option(BUF_OPT_IDX[bv]).flags & K_OPT_FLAG_INSECURE
}

/// Copy global option values to local options for one buffer.
pub fn buf_copy_options(buf: &mut FileBuf, flags: i32) {
    {
        let mut should_copy = true;
        let mut save_p_isk: *mut c_char = ptr::null_mut();
        let mut did_isk = false;
        let empty_opt = EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();

        if !opt_vars().p_cpo.is_null() {
            if (!crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_BUFOPTGLOB)
                || (flags & BCO_ENTER) == 0)
                && (buf.b_p_initialized
                    || ((flags & BCO_ENTER) == 0
                        && crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_BUFOPT)))
            {
                should_copy = false;
            }

            if should_copy || (flags & BCO_ALWAYS) != 0 {
                buf.b_p_script_ctx = [Sctx {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                }; 93];
                let dont_do_help = ((flags & BCO_NOHELP) != 0 && buf.b_help) || buf.b_p_initialized;
                if dont_do_help {
                    save_p_isk = buf.b_p_isk;
                    buf.b_p_isk = ptr::null_mut();
                }
                if !buf.b_p_initialized {
                    free_buf_options(buf, true);
                    buf.b_p_ro = 0;
                    buf.b_p_fenc = alloc_string_option(opt_vars().p_fenc);
                    buf.b_p_ff = match crate::memory::cstr_first_byte(opt_vars().p_ffs) {
                        b'm' => alloc_string_option(c"mac"),
                        b'd' => alloc_string_option(c"dos"),
                        b'u' => alloc_string_option(c"unix"),
                        _ => alloc_string_option(opt_vars().p_ff),
                    };
                    buf.b_p_bh = empty_opt;
                    buf.b_p_bt = empty_opt;
                } else {
                    free_buf_options(buf, false);
                }

                buf.b_p_ai = opt_vars().p_ai;
                copy_opt_sctx(buf, K_BUF_OPT_AUTOINDENT);
                buf.b_p_ai_nopaste = opt_st().p_ai_nopaste;
                buf.b_p_sw = opt_vars().p_sw;
                copy_opt_sctx(buf, K_BUF_OPT_SHIFTWIDTH);
                buf.b_p_scbk = opt_vars().p_scbk;
                copy_opt_sctx(buf, K_BUF_OPT_SCROLLBACK);
                buf.b_p_tw = opt_vars().p_tw;
                copy_opt_sctx(buf, K_BUF_OPT_TEXTWIDTH);
                buf.b_p_tw_nopaste = opt_st().p_tw_nopaste;
                buf.b_p_tw_nobin = opt_st().p_tw_nobin;
                buf.b_p_wm = opt_vars().p_wm;
                copy_opt_sctx(buf, K_BUF_OPT_WRAPMARGIN);
                buf.b_p_wm_nopaste = opt_st().p_wm_nopaste;
                buf.b_p_wm_nobin = opt_st().p_wm_nobin;
                buf.b_p_bin = opt_vars().p_bin;
                copy_opt_sctx(buf, K_BUF_OPT_BINARY);
                buf.b_p_bomb = opt_vars().p_bomb;
                copy_opt_sctx(buf, K_BUF_OPT_BOMB);
                buf.b_p_et = opt_vars().p_et;
                copy_opt_sctx(buf, K_BUF_OPT_EXPANDTAB);
                buf.b_p_fixeol = opt_vars().p_fixeol;
                copy_opt_sctx(buf, K_BUF_OPT_FIXENDOFLINE);
                buf.b_p_follow = opt_vars().p_follow;
                copy_opt_sctx(buf, K_BUF_OPT_FOLLOW);
                buf.b_p_et_nobin = opt_st().p_et_nobin;
                buf.b_p_et_nopaste = opt_st().p_et_nopaste;
                buf.b_p_ml = opt_vars().p_ml;
                copy_opt_sctx(buf, K_BUF_OPT_MODELINE);
                buf.b_p_ml_nobin = opt_st().p_ml_nobin;
                buf.b_p_inf = opt_vars().p_inf;
                copy_opt_sctx(buf, K_BUF_OPT_INFERCASE);
                if (g_st().cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0 {
                    buf.b_p_swf = 0;
                } else {
                    buf.b_p_swf = opt_vars().p_swf;
                    copy_opt_sctx(buf, K_BUF_OPT_SWAPFILE);
                }
                buf.b_p_cpt = alloc_string_option(opt_vars().p_cpt);
                copy_opt_sctx(buf, K_BUF_OPT_COMPLETE);
                buf.b_p_cpt_flags = copy_opt_insecure(K_BUF_OPT_COMPLETE);
                set_buflocal_cpt_callbacks(Some(&mut *buf));
                #[cfg(windows)]
                {
                    buf.b_p_csl = alloc_string_option(opt_vars().p_csl);
                    copy_opt_sctx(buf, K_BUF_OPT_COMPLETESLASH);
                }
                callback_copy(&mut buf.b_p_cfu, &opt_vars().p_cfu);
                copy_opt_sctx(buf, K_BUF_OPT_COMPLETEFUNC);
                callback_copy(&mut buf.b_p_ofu, &opt_vars().p_ofu);
                copy_opt_sctx(buf, K_BUF_OPT_OMNIFUNC);
                callback_copy(&mut buf.b_p_tfu, &opt_vars().p_tfu);
                copy_opt_sctx(buf, K_BUF_OPT_TAGFUNC);
                buf.b_p_sts = opt_vars().p_sts;
                copy_opt_sctx(buf, K_BUF_OPT_SOFTTABSTOP);
                buf.b_p_sts_nopaste = opt_st().p_sts_nopaste;
                buf.b_p_vsts = alloc_string_option(opt_vars().p_vsts);
                copy_opt_sctx(buf, K_BUF_OPT_VARSOFTTABSTOP);
                if !opt_vars().p_vsts.is_null() && opt_vars().p_vsts != empty_opt {
                    tabstop_set(opt_vars().p_vsts, &mut buf.b_p_vsts_array, None);
                } else {
                    buf.b_p_vsts_array.clear();
                }
                buf.b_p_vsts_nopaste = opt_st().p_vsts_nopaste.clone();
                buf.b_p_com = alloc_string_option(opt_vars().p_com);
                copy_opt_sctx(buf, K_BUF_OPT_COMMENTS);
                buf.b_p_cms = alloc_string_option(opt_vars().p_cms);
                copy_opt_sctx(buf, K_BUF_OPT_COMMENTSTRING);
                buf.b_p_fo = alloc_string_option(opt_vars().p_fo);
                copy_opt_sctx(buf, K_BUF_OPT_FORMATOPTIONS);
                buf.b_p_flp = alloc_string_option(opt_vars().p_flp);
                copy_opt_sctx(buf, K_BUF_OPT_FORMATLISTPAT);
                buf.b_p_nf = alloc_string_option(opt_vars().p_nf);
                copy_opt_sctx(buf, K_BUF_OPT_NRFORMATS);
                buf.b_p_mps = alloc_string_option(opt_vars().p_mps);
                copy_opt_sctx(buf, K_BUF_OPT_MATCHPAIRS);
                buf.b_p_si = opt_vars().p_si;
                copy_opt_sctx(buf, K_BUF_OPT_SMARTINDENT);
                buf.b_p_channel = 0;
                buf.b_p_ci = opt_vars().p_ci;
                copy_opt_sctx(buf, K_BUF_OPT_COPYINDENT);
                buf.b_p_cin = opt_vars().p_cin;
                copy_opt_sctx(buf, K_BUF_OPT_CINDENT);
                buf.b_p_cink = alloc_string_option(opt_vars().p_cink);
                copy_opt_sctx(buf, K_BUF_OPT_CINKEYS);
                buf.b_p_cino = alloc_string_option(opt_vars().p_cino);
                copy_opt_sctx(buf, K_BUF_OPT_CINOPTIONS);
                buf.b_p_cinsd = alloc_string_option(opt_vars().p_cinsd);
                copy_opt_sctx(buf, K_BUF_OPT_CINSCOPEDECLS);
                buf.b_p_lop = alloc_string_option(opt_vars().p_lop);
                copy_opt_sctx(buf, K_BUF_OPT_LISPOPTIONS);

                buf.b_p_ft = empty_opt;
                buf.b_p_pi = opt_vars().p_pi;
                copy_opt_sctx(buf, K_BUF_OPT_PRESERVEINDENT);
                buf.b_p_cinw = alloc_string_option(opt_vars().p_cinw);
                copy_opt_sctx(buf, K_BUF_OPT_CINWORDS);
                buf.b_p_lisp = opt_vars().p_lisp;
                copy_opt_sctx(buf, K_BUF_OPT_LISP);
                buf.b_p_syn = empty_opt;
                buf.b_p_smc = opt_vars().p_smc;
                copy_opt_sctx(buf, K_BUF_OPT_SYNMAXCOL);
                buf.b_s.b_syn_isk = None;
                buf.b_s.b_p_spc = alloc_string_option(opt_vars().p_spc);
                copy_opt_sctx(buf, K_BUF_OPT_SPELLCAPCHECK);
                compile_cap_prog(buf.b_s.b_p_spc, &mut buf.b_s.b_cap_prog);
                buf.b_s.b_p_spf = alloc_string_option(opt_vars().p_spf);
                copy_opt_sctx(buf, K_BUF_OPT_SPELLFILE);
                buf.b_s.b_p_spl = alloc_string_option(opt_vars().p_spl);
                copy_opt_sctx(buf, K_BUF_OPT_SPELLLANG);
                buf.b_s.b_p_spo = alloc_string_option(opt_vars().p_spo);
                copy_opt_sctx(buf, K_BUF_OPT_SPELLOPTIONS);
                buf.b_s.b_p_spo_flags = opt_vars().spo_flags;
                callback_copy(&mut buf.b_p_inde, &opt_vars().p_inde);
                copy_opt_sctx(buf, K_BUF_OPT_INDENTEXPR);
                buf.b_p_inde_flags = copy_opt_insecure(K_BUF_OPT_INDENTEXPR);
                buf.b_p_indk = alloc_string_option(opt_vars().p_indk);
                copy_opt_sctx(buf, K_BUF_OPT_INDENTKEYS);
                buf.b_p_fp = empty_opt;
                callback_copy(&mut buf.b_p_fex, &opt_vars().p_fex);
                copy_opt_sctx(buf, K_BUF_OPT_FORMATEXPR);
                buf.b_p_fex_flags = copy_opt_insecure(K_BUF_OPT_FORMATEXPR);
                buf.b_p_sua = alloc_string_option(opt_vars().p_sua);
                copy_opt_sctx(buf, K_BUF_OPT_SUFFIXESADD);
                buf.b_p_keymap = alloc_string_option(opt_vars().p_keymap);
                copy_opt_sctx(buf, K_BUF_OPT_KEYMAP);
                buf.b_kmap_state |= KEYMAP_INIT;
                buf.b_p_iminsert = opt_vars().p_iminsert;
                copy_opt_sctx(buf, K_BUF_OPT_IMINSERT);
                buf.b_p_imsearch = opt_vars().p_imsearch;
                copy_opt_sctx(buf, K_BUF_OPT_IMSEARCH);

                buf.b_p_ac = -1;
                buf.b_p_ar = -1;
                buf.b_p_fs = -1;
                buf.b_p_ul = NO_LOCAL_UNDOLEVEL;
                buf.b_p_bkc = empty_opt;
                buf.b_bkc_flags = 0;
                buf.b_p_gefm = empty_opt;
                buf.b_p_gp = empty_opt;
                buf.b_p_mp = empty_opt;
                buf.b_p_efm = empty_opt;
                buf.b_p_ep = empty_opt;
                buf.b_p_ffu = Callback::NONE;
                buf.b_p_kp = empty_opt;
                buf.b_p_path = empty_opt;
                buf.b_p_tags = empty_opt;
                buf.b_p_tc = empty_opt;
                buf.b_tc_flags = 0;
                buf.b_p_def = empty_opt;
                buf.b_p_inc = empty_opt;
                callback_copy(&mut buf.b_p_inex, &opt_vars().p_inex);
                copy_opt_sctx(buf, K_BUF_OPT_INCLUDEEXPR);
                buf.b_p_inex_flags = copy_opt_insecure(K_BUF_OPT_INCLUDEEXPR);
                buf.b_p_cot = empty_opt;
                buf.b_cot_flags = 0;
                buf.b_p_dict = empty_opt;
                buf.b_p_dia = empty_opt;
                buf.b_p_tsr = empty_opt;
                buf.b_p_tsrfu = Callback::NONE;
                buf.b_p_qe = alloc_string_option(opt_vars().p_qe);
                copy_opt_sctx(buf, K_BUF_OPT_QUOTEESCAPE);
                buf.b_p_udf = opt_vars().p_udf;
                copy_opt_sctx(buf, K_BUF_OPT_UNDOFILE);
                buf.b_p_lw = empty_opt;
                buf.b_p_menc = empty_opt;

                if dont_do_help {
                    buf.b_p_isk = save_p_isk;
                    if !opt_vars().p_vts.is_null()
                        && !crate::memory::cstr_is_empty(opt_vars().p_vts)
                        && buf.b_p_vts_array.is_empty()
                    {
                        tabstop_set(opt_vars().p_vts, &mut buf.b_p_vts_array, None);
                    } else {
                        buf.b_p_vts_array.clear();
                    }
                } else {
                    buf.b_p_isk = alloc_string_option(opt_vars().p_isk);
                    copy_opt_sctx(buf, K_BUF_OPT_ISKEYWORD);
                    did_isk = true;
                    buf.b_p_ts = opt_vars().p_ts;
                    copy_opt_sctx(buf, K_BUF_OPT_TABSTOP);
                    buf.b_p_vts = alloc_string_option(opt_vars().p_vts);
                    copy_opt_sctx(buf, K_BUF_OPT_VARTABSTOP);
                    if !opt_vars().p_vts.is_null()
                        && !crate::memory::cstr_is_empty(opt_vars().p_vts)
                        && buf.b_p_vts_array.is_empty()
                    {
                        tabstop_set(opt_vars().p_vts, &mut buf.b_p_vts_array, None);
                    } else {
                        buf.b_p_vts_array.clear();
                    }
                    buf.b_help = false;
                    if crate::memory::cstr_first_byte(buf.b_p_bt) == b'h' {
                        clear_string_option(&raw mut buf.b_p_bt);
                    }
                    buf.b_p_ma = opt_vars().p_ma;
                    copy_opt_sctx(buf, K_BUF_OPT_MODIFIABLE);
                }
            }

            if should_copy {
                buf.b_p_initialized = true;
            }
        }

        check_buf_options(buf);
        if did_isk {
            buf_init_isk_chartab(buf);
        }
    }
}

/// Reset the 'modifiable' option and its default value.
pub fn reset_modifiable() {
    {
        curbuf_mut().b_p_ma = 0;
        opt_vars().p_ma = 0;
        change_option_default(K_OPT_MODIFIABLE, boolean_obj(false));
    }
}

/// Set the global value for 'iminsert' to the local value.
pub fn set_iminsert_global(buf: &mut FileBuf) {
    opt_vars().p_iminsert = buf.b_p_iminsert;
}

/// Set the global value for 'imsearch' to the local value.
pub fn set_imsearch_global(buf: &mut FileBuf) {
    opt_vars().p_imsearch = buf.b_p_imsearch;
}

pub fn set_context_in_set_cmd(xp: &mut ExpandCtx, mut arg: *mut c_char, opt_flags: i32) {
    unsafe {
        opt_st().expand_option_flags = opt_flags;

        xp.xp_context = EXPAND_SETTINGS;
        if crate::memory::cstr_is_empty(arg) {
            xp.xp_pattern = arg;
            return;
        }
        let argend = arg.wrapping_add(crate::memory::cstr_len(arg));
        let mut p = argend.wrapping_offset(-1);
        if crate::memory::cstr_first_byte(p) == b' ' && *p.wrapping_offset(-1) != b'\\' as c_char {
            xp.xp_pattern = p.wrapping_add(1);
            return;
        }
        while p > arg {
            let mut s = p;
            if crate::memory::cstr_first_byte(p) == b' '
                || crate::memory::cstr_first_byte(p) == b','
            {
                while s > arg && *s.wrapping_offset(-1) == b'\\' as c_char {
                    s = s.wrapping_offset(-1);
                }
            }
            if crate::memory::cstr_first_byte(p) == b' '
                && (crate::memory::ptr_offset_from(p, s) & 1) == 0
            {
                p = p.wrapping_add(1);
                break;
            }
            p = p.wrapping_offset(-1);
        }
        if crate::memory::cstr_prefix_bytes(p, 2) == &b"no"[..2] {
            xp.xp_context = EXPAND_BOOL_SETTINGS;
            xp.xp_prefix = XP_PREFIX_NO;
            p = p.wrapping_add(2);
        } else if crate::memory::cstr_prefix_bytes(p, 3) == &b"inv"[..3] {
            xp.xp_context = EXPAND_BOOL_SETTINGS;
            xp.xp_prefix = XP_PREFIX_INV;
            p = p.wrapping_add(3);
        }
        xp.xp_pattern = p;
        arg = p;

        let mut nextchar: c_char;
        let mut flags: u32 = 0;
        let mut opt_idx: OptIndex = 0;
        let mut is_term_option = false;

        if crate::memory::cstr_first_byte(arg) == b'<' {
            while crate::memory::cstr_first_byte(p) != b'>' {
                if crate::memory::cstr_is_empty(p) {
                    return;
                }
                p = p.wrapping_add(1);
            }
            p = p.wrapping_add(1);
            let key = get_special_key_code(arg.wrapping_add(1));
            if key == 0 {
                xp.xp_context = EXPAND_NOTHING;
                return;
            }
            nextchar = *p;
            is_term_option = true;
            opt_st().expand_option_name[2] = key2termcap0(key) as c_char;
            opt_st().expand_option_name[3] = key2termcap1(key) as c_char;
        } else if crate::memory::cstr_first_byte(p) == b't'
            && crate::memory::cstr_byte_at(p, 1) == b'_'
        {
            p = p.wrapping_add(2);
            if !crate::memory::cstr_is_empty(p) {
                p = p.wrapping_add(1);
            }
            if crate::memory::cstr_is_empty(p) {
                return;
            }
            p = p.wrapping_add(1);
            nextchar = *p;
            is_term_option = true;
            opt_st().expand_option_name[2] = *p.wrapping_offset(-2);
            opt_st().expand_option_name[3] = *p.wrapping_offset(-1);
        } else {
            while ascii_isalnum(crate::memory::cstr_first_char(p))
                || crate::memory::cstr_first_byte(p) == b'_'
                || crate::memory::cstr_first_byte(p) == b'*'
            {
                p = p.wrapping_add(1);
            }
            if crate::memory::cstr_is_empty(p) {
                return;
            }
            nextchar = *p;
            opt_idx = find_option_len(arg, crate::memory::ptr_offset_from(p, arg) as usize);
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

        opt_st().expand_option_append = false;
        let mut expand_option_subtract = false;
        if (nextchar == b'-' as c_char || nextchar == b'+' as c_char || nextchar == b'^' as c_char)
            && crate::memory::cstr_byte_at(p, 1) == b'='
        {
            if nextchar == b'-' as c_char {
                expand_option_subtract = true;
            }
            if nextchar == b'+' as c_char || nextchar == b'^' as c_char {
                opt_st().expand_option_append = true;
            }
            p = p.wrapping_add(1);
            nextchar = b'=' as c_char;
        }
        if (nextchar != b'=' as c_char && nextchar != b':' as c_char)
            || xp.xp_context == EXPAND_BOOL_SETTINGS
        {
            xp.xp_context = EXPAND_UNSUCCESSFUL;
            return;
        }

        if is_term_option {
            opt_st().expand_option_idx = K_OPT_INVALID;
        } else {
            opt_st().expand_option_idx = opt_idx;
        }

        xp.xp_pattern = p.wrapping_add(1);
        opt_st().expand_option_start_col =
            crate::memory::ptr_offset_from(p.wrapping_add(1), xp.xp_line) as i32;

        let opt_var = get_option(opt_idx).var;
        if opt_var == (&raw mut opt_vars().p_syn).cast::<c_void>() {
            xp.xp_context = EXPAND_OWNSYNTAX;
            return;
        }
        if opt_var == (&raw mut opt_vars().p_ft).cast::<c_void>() {
            xp.xp_context = EXPAND_FILETYPE;
            return;
        }
        if opt_var == (&raw mut opt_vars().p_keymap).cast::<c_void>() {
            xp.xp_context = EXPAND_KEYMAP;
            return;
        }

        if expand_option_subtract {
            xp.xp_context = EXPAND_SETTING_SUBTRACT;
            return;
        } else if opt_st().expand_option_idx != K_OPT_INVALID
            && get_option(opt_st().expand_option_idx)
                .opt_expand_cb
                .is_some()
        {
            xp.xp_context = EXPAND_STRING_SETTING;
        } else if crate::memory::cstr_is_empty(xp.xp_pattern) {
            xp.xp_context = EXPAND_OLD_SETTING;
            return;
        } else {
            xp.xp_context = EXPAND_NOTHING;
        }

        if is_term_option || option_has_type(opt_idx, ObjectType::ObjectTypeInteger) {
            return;
        }

        if (flags & K_OPT_FLAG_EXPAND) != 0 {
            let v = get_option(opt_idx).var;
            if v == (&raw mut opt_vars().p_bdir).cast()
                || v == (&raw mut opt_vars().p_dir).cast()
                || v == (&raw mut opt_vars().p_path).cast()
                || v == (&raw mut opt_vars().p_pp).cast()
                || v == (&raw mut opt_vars().p_rtp).cast()
                || v == (&raw mut opt_vars().p_cdpath).cast()
                || v == (&raw mut opt_vars().p_vdir).cast()
            {
                xp.xp_context = EXPAND_DIRECTORIES;
                if v == (&raw mut opt_vars().p_path).cast()
                    || v == (&raw mut opt_vars().p_cdpath).cast()
                {
                    xp.xp_backslash = XP_BS_THREE;
                } else {
                    xp.xp_backslash = XP_BS_ONE;
                }
            } else {
                xp.xp_context = EXPAND_FILES;
                if v == (&raw mut opt_vars().p_tags).cast() {
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
            p = argend.wrapping_offset(-1);
            while p > xp.xp_pattern {
                if crate::memory::cstr_first_byte(p) == b' '
                    || crate::memory::cstr_first_byte(p) == b','
                    || (crate::memory::cstr_first_byte(p) == b':'
                        && (flags & K_OPT_FLAG_COLON) != 0)
                {
                    let mut s = p;
                    while s > xp.xp_pattern && *s.wrapping_offset(-1) == b'\\' as c_char {
                        s = s.wrapping_offset(-1);
                    }
                    #[cfg(windows)]
                    let comma_unesc = crate::memory::cstr_first_byte(p) == b','
                        && (flags & K_OPT_FLAG_COMMA) != 0
                        && crate::memory::ptr_offset_from(p, s) < 1;
                    #[cfg(not(windows))]
                    let comma_unesc = crate::memory::cstr_first_byte(p) == b','
                        && (flags & K_OPT_FLAG_COMMA) != 0
                        && p.offset_from(s) < 2;

                    if (crate::memory::cstr_first_byte(p) == b' '
                        && ((xp.xp_backslash & XP_BS_THREE) != 0
                            && crate::memory::ptr_offset_from(p, s) < 3))
                        || comma_unesc
                        || (crate::memory::cstr_first_byte(p) == b':'
                            && (flags & K_OPT_FLAG_COLON) != 0)
                    {
                        xp.xp_pattern = p.wrapping_add(1);
                        break;
                    }
                }
                p = p.wrapping_offset(-1);
            }
        }

        if (flags & K_OPT_FLAG_FLAG_LIST) != 0 {
            xp.xp_pattern = argend;
        }

        if get_option(opt_idx).var == (&raw mut opt_vars().p_sps).cast() {
            if crate::memory::cstr_prefix_bytes(xp.xp_pattern, 5) == &b"file:"[..5] {
                xp.xp_pattern = xp.xp_pattern.wrapping_add(5);
            } else if get_option(opt_st().expand_option_idx)
                .opt_expand_cb
                .is_some()
            {
                xp.xp_context = EXPAND_STRING_SETTING;
            }
        }
    }
}

fn match_str(
    str_arg: impl crate::strings::IntoCCharPtr,
    regmatch: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
    fuzzy: bool,
    fuzzystr: impl crate::strings::IntoCCharPtr,
    fuzmatch: &mut Vec<FuzmatchStr>,
) -> bool {
    let str_arg = str_arg.into_c_char_ptr();
    let fuzzystr = fuzzystr.into_c_char_ptr();
    {
        if !fuzzy {
            if vim_regexec(regmatch, str_arg, 0) {
                matches.push(crate::memory::cstr_to_cstring(str_arg));
                return true;
            }
        } else {
            let score = fuzzy_match_str(str_arg, fuzzystr);
            if score != FUZZY_SCORE_NONE {
                let idx = fuzmatch.len() as i32;
                fuzmatch.push(FuzmatchStr {
                    idx,
                    str_cstr: crate::memory::cstr_to_cstring(str_arg),
                    score,
                });
                return true;
            }
        }
        false
    }
}

pub fn expand_settings(
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    fuzzystr: impl crate::strings::IntoCCharPtr,
    matches: &mut Vec<std::ffi::CString>,
    can_fuzzy: bool,
) -> bool {
    let fuzzystr = fuzzystr.into_c_char_ptr();
    {
        let names: [*mut c_char; 1] = [c"all".as_ptr().cast_mut()];
        let ic = regmatch.rm_ic;

        matches.clear();
        let mut fuzmatch: Vec<FuzmatchStr> = Vec::new();
        let fuzzy = can_fuzzy && cmdline_fuzzy_complete(fuzzystr);

        regmatch.rm_ic = ic;
        if xp.xp_context != EXPAND_BOOL_SETTINGS {
            for &name in &names {
                match_str(name, regmatch, matches, fuzzy, fuzzystr, &mut fuzmatch);
            }
        }
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            let str_ptr = opt.fullname.as_ptr().cast_mut();
            if is_option_hidden(opt_idx) {
                continue;
            }
            if xp.xp_context == EXPAND_BOOL_SETTINGS
                && !option_has_type(opt_idx, ObjectType::ObjectTypeBoolean)
            {
                continue;
            }

            if !match_str(str_ptr, regmatch, matches, fuzzy, fuzzystr, &mut fuzmatch)
                && !fuzzy
                && opt
                    .shortname
                    .is_some_and(|s| vim_regexec(regmatch, s.as_ptr(), 0))
            {
                matches.push(opt.fullname.to_owned());
            }
        }

        if fuzzy {
            fuzzymatches_slice_to_strmatches(&mut fuzmatch, matches, false);
        }

        true
    }
}

/// Escape an option value that can be used on the command-line with :set.
pub fn escape_option_str_cmdline(var_arg: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let var_arg = var_arg.into_c_char_ptr();
    let buf_cstr = vim_strsave_escaped(var_arg, ESCAPE_CHARS);

    #[cfg(windows)]
    unsafe {
        let mut bytes = buf_cstr.into_bytes_with_nul();
        let buf = bytes.as_mut_ptr().cast::<c_char>();
        let mut var = buf;
        while !crate::memory::cstr_is_empty(var) {
            if crate::memory::cstr_first_byte(var) == b'\\'
                && crate::memory::cstr_byte_at(var, 1) == b'\\'
                && opt_st().expand_option_idx != K_OPT_INVALID
                && (get_option(opt_st().expand_option_idx).flags & K_OPT_FLAG_EXPAND) != 0
                && vim_isfilec(crate::memory::cstr_byte_at(var, 2) as i32)
                && (crate::memory::cstr_byte_at(var, 2) != b'\\'
                    || (var == buf && crate::memory::cstr_byte_at(var, 4) != b'\\'))
            {
                let src = var.wrapping_add(1);
                ptr::copy(src, var, crate::memory::cstr_len(src) + 1);
            }
            var = var.wrapping_add(utfc_ptr2len(var) as usize);
        }
        let len = crate::memory::cstr_len(buf);
        crate::memory::cstr_from_bytes(&bytes[..len])
    }
    #[cfg(not(windows))]
    {
        buf_cstr
    }
}

/// Expansion handler for :set= when we just want to fill in with the existing value.
pub fn expand_old_setting(matches: &mut Vec<std::ffi::CString>) -> bool {
    matches.clear();

    if opt_st().expand_option_idx == K_OPT_INVALID {
        opt_st().expand_option_idx = find_option((&raw const opt_st().expand_option_name).cast());
    }

    let var = if opt_st().expand_option_idx != K_OPT_INVALID {
        let o = opt_from_varp(
            opt_st().expand_option_idx,
            get_varp_scope(
                get_option(opt_st().expand_option_idx),
                opt_st().expand_option_flags,
            ),
        );
        let is_luafn = o.obj_type == ObjectType::ObjectTypeLuaRef;
        optval_free_read(opt_st().expand_option_idx, o);
        if is_luafn {
            return true;
        }
        optval_fmt(opt_st().expand_option_idx, opt_st().expand_option_flags);
        g_st().name_buff.as_mut_ptr()
    } else {
        c"".as_ptr().cast_mut()
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
    {
        if opt_st().expand_option_idx == K_OPT_INVALID {
            return false;
        }
        let opt = get_option(opt_st().expand_option_idx);
        let Some(expand_cb) = opt.opt_expand_cb else {
            return false;
        };

        let set_arg = xp
            .xp_line
            .wrapping_add(opt_st().expand_option_start_col as usize);
        let mut args = Optexpand {
            oe_varp: get_varp_scope(opt, opt_st().expand_option_flags).cast::<c_char>(),
            oe_idx: opt_st().expand_option_idx,
            oe_opt_value: ptr::null_mut(),
            oe_append: opt_st().expand_option_append,
            oe_include_orig_val: !opt_st().expand_option_append
                && crate::memory::cstr_is_empty(set_arg),
            oe_regmatch: regmatch,
            oe_xp: xp,
            oe_set_arg: set_arg,
        };

        optval_fmt(opt_st().expand_option_idx, opt_st().expand_option_flags);
        let buf = escape_option_str_cmdline(g_st().name_buff.as_mut_ptr());
        args.oe_opt_value = buf.as_ptr().cast_mut();

        expand_cb(&mut args, matches)
    }
}

/// Expansion handler for :set-=
pub fn expand_setting_subtract(
    xp: &mut ExpandCtx,
    regmatch: &mut Regmatch,
    matches: &mut Vec<std::ffi::CString>,
) -> bool {
    unsafe {
        if opt_st().expand_option_idx == K_OPT_INVALID
            || option_has_type(opt_st().expand_option_idx, ObjectType::ObjectTypeInteger)
            || is_callback_option(opt_st().expand_option_idx)
        {
            return expand_old_setting(matches);
        }

        let option_val = *(get_option_varp_scope_from(
            opt_st().expand_option_idx,
            opt_st().expand_option_flags,
            curbuf_mut(),
            curwin_ptr(),
        )
        .cast::<*mut c_char>());

        let option_flags = get_option(opt_st().expand_option_idx).flags;

        if (option_flags & K_OPT_FLAG_COMMA) != 0 {
            if crate::memory::cstr_is_empty(option_val) {
                return false;
            }

            let mut option_copy = crate::memory::cstr_to_cstring(option_val).into_bytes_with_nul();
            let mut next_val = option_copy.as_mut_ptr().cast::<c_char>();

            matches.clear();

            loop {
                let item = next_val;
                let mut comma = vim_strchr(next_val, b',' as i32);
                while !comma.is_null()
                    && comma != next_val
                    && *comma.wrapping_offset(-1) == b'\\' as c_char
                {
                    comma = vim_strchr(comma.wrapping_add(1), b',' as i32);
                }
                if !comma.is_null() {
                    *comma = NUL;
                    next_val = comma.wrapping_add(1);
                } else {
                    next_val = ptr::null_mut();
                }

                if !crate::memory::cstr_is_empty(item) && vim_regexec(regmatch, item, 0) {
                    matches.push(escape_option_str_cmdline(item));
                }

                if next_val.is_null() {
                    break;
                }
            }

            return true;
        } else if (option_flags & K_OPT_FLAG_FLAG_LIST) != 0 {
            if !crate::memory::cstr_is_empty(xp.xp_pattern) {
                return false;
            }

            let num_flags = crate::memory::cstr_len(option_val);
            if num_flags == 0 {
                return false;
            }

            matches.clear();
            matches.reserve_exact(num_flags + 1);
            matches.push(crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                option_val.cast::<u8>(),
                num_flags,
            )));

            if num_flags > 1 {
                let flag = option_val;
                for &flag_byte in crate::memory::cstr_as_bytes(flag) {
                    matches.push(crate::memory::cstr_from_bytes(&[flag_byte]));
                }
            }

            return true;
        }

        expand_old_setting(matches)
    }
}

fn optval_fmt(opt_idx: OptIndex, opt_flags: i32) {
    unsafe {
        let opt = get_option(opt_idx);
        let varp = get_varp_scope(opt, opt_flags);
        assert!(!varp.is_null());

        let namebuf_ptr = g_st().name_buff.as_mut_ptr();
        let mut wc: OptInt = 0;
        if wc_use_keyname(varp.cast::<OptInt>(), &mut wc) != 0 {
            xstrlcpy(namebuf_ptr, get_special_key_name(wc as i32, 0), MAXPATHL);
            return;
        }
        if wc != 0 {
            xstrlcpy(namebuf_ptr, transchar(wc as i32), MAXPATHL);
            return;
        }
        if (opt.flags & K_OPT_FLAG_EXPAND) != 0
            && !is_callback_option(opt_idx)
            && option_has_type(opt_idx, ObjectType::ObjectTypeString)
        {
            home_replace(
                None,
                *(varp.cast::<*mut c_char>()),
                namebuf_ptr,
                MAXPATHL,
                false,
            );
            return;
        }

        let o = opt_from_varp(opt_idx, varp);
        let s = optval_to_cstr(o, false);
        xstrlcpy(namebuf_ptr, s.as_ptr(), MAXPATHL);
        optval_free_read(opt_idx, o);
    }
}

fn wc_use_keyname(p: *const OptInt, wcp: &mut OptInt) -> i32 {
    unsafe {
        if p == &raw const opt_vars().p_wc || p == &raw const opt_vars().p_wcm {
            *wcp = *p;
            if is_special(*wcp as i32) || find_special_key_in_table(*wcp as i32) >= 0 {
                return 1;
            }
        }
        0
    }
}

pub fn shortmess(x: i32) -> bool {
    !opt_vars().p_shm.is_null()
        && (crate::strings::vim_haschr(opt_vars().p_shm, x)
            || (crate::strings::vim_haschr(opt_vars().p_shm, b'a' as i32)
                && crate::strings::vim_haschr(SHM_ALL_ABBREVIATIONS, x)))
}

pub fn vimrc_found(fname: impl crate::strings::IntoCCharPtr, envname: &std::ffi::CStr) {
    let fname = fname.into_c_char_ptr();
    if !fname.is_null()
        && vim_getenv(envname.as_ptr().cast_mut()).is_none()
        && let Some(p) = FullName_save(fname, false)
    {
        os_setenv(envname.as_ptr().cast_mut(), p.as_ptr(), 1);
    }
}

/// Check whether global option has been set.
pub fn option_was_set(opt_idx: OptIndex) -> bool {
    {
        assert!(opt_idx != K_OPT_INVALID);
        (get_option(opt_idx).flags & K_OPT_FLAG_WAS_SET) != 0
    }
}

/// Reset the flag indicating option "name" was set.
pub fn reset_option_was_set(opt_idx: OptIndex) {
    {
        assert!(opt_idx != K_OPT_INVALID);
        get_option(opt_idx).flags &= !K_OPT_FLAG_WAS_SET;
    }
}

/// Parse 'cursorlineopt', writing flags only when the value is valid.
pub fn fill_culopt_flags(val: impl crate::strings::IntoCCharPtr, flags: &mut u32) -> bool {
    let val = val.into_c_char_ptr();
    {
        let mut p = val;
        let mut culopt_flags_new: u8 = 0;

        while !crate::memory::cstr_is_empty(p) {
            if crate::memory::cstr_prefix_bytes(p, 4) == &b"line"[..4] {
                p = p.wrapping_add(4);
                culopt_flags_new |= K_OPT_CULOPT_FLAG_LINE;
            } else if crate::memory::cstr_prefix_bytes(p, 4) == &b"both"[..4] {
                p = p.wrapping_add(4);
                culopt_flags_new |= K_OPT_CULOPT_FLAG_LINE | K_OPT_CULOPT_FLAG_NUMBER;
            } else if crate::memory::cstr_prefix_bytes(p, 6) == &b"number"[..6] {
                p = p.wrapping_add(6);
                culopt_flags_new |= K_OPT_CULOPT_FLAG_NUMBER;
            } else if crate::memory::cstr_prefix_bytes(p, 10) == &b"screenline"[..10] {
                p = p.wrapping_add(10);
                culopt_flags_new |= K_OPT_CULOPT_FLAG_SCREENLINE;
            }

            if crate::memory::cstr_first_byte(p) != b',' && !crate::memory::cstr_is_empty(p) {
                return false;
            }
            if crate::memory::cstr_first_byte(p) == b',' {
                p = p.wrapping_add(1);
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
}

/// Set the callback function value for an option that accepts a function name, lambda, et al.
pub fn option_set_callback_func(
    optval: impl crate::strings::IntoCCharPtr,
    optcb: &mut Callback,
) -> bool {
    let optval = optval.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(optval) {
            callback_free(optcb);
            return true;
        }

        let mut tv: TypVal = if crate::memory::cstr_first_byte(optval) == b'{'
            || (crate::memory::cstr_prefix_bytes(optval, 9) == &b"function("[..9])
            || (crate::memory::cstr_prefix_bytes(optval, 8) == &b"funcref("[..8])
        {
            let Some(evaluated) = eval_expr(optval, None) else {
                return false;
            };
            evaluated
        } else {
            let mut allocated = TypVal::new();
            crate::eval::typval::tv_set_string(&mut allocated, optval);
            allocated
        };

        let mut cb = Callback::NONE;
        if !callback_from_typval(&mut cb, &tv) || cb.cb_type() == K_CALLBACK_NONE_CONST {
            tv_clear(&mut tv);
            return false;
        }

        callback_free(optcb);
        *optcb = cb;
        tv_clear(&mut tv);
        true
    }
}

fn didset_options_sctx(opt_flags: i32, opts: &[i32]) {
    for &opt_idx in opts {
        set_option_sctx(opt_idx, opt_flags, g_st().current_sctx);
    }
}

/// Check if backspacing over something is allowed.
pub fn can_bs(what: i32) -> bool {
    {
        if what == BS_START && bt_prompt(Some(crate::globals::curbuf_mut())) {
            return false;
        }

        if what == BS_EOL && !mc_ins_replay_can_join() {
            return false;
        }

        if crate::memory::cstr_first_byte(opt_vars().p_bs) == b'2' {
            return what != BS_NOSTOP;
        }

        crate::strings::vim_haschr(opt_vars().p_bs, what)
    }
}

/// Get the local or global value of 'backupcopy' flags.
pub fn get_bkc_flags(buf: &mut FileBuf) -> u32 {
    {
        if buf.b_bkc_flags != 0 {
            buf.b_bkc_flags
        } else {
            opt_vars().bkc_flags
        }
    }
}

/// Get the local or global value of 'formatlistpat'.
pub fn get_flp_value(buf: &mut FileBuf) -> *mut c_char {
    {
        if crate::memory::cstr_is_empty(buf.b_p_flp) {
            opt_vars().p_flp
        } else {
            buf.b_p_flp
        }
    }
}

/// Get the local or global value of 'virtualedit' flags.
pub fn get_ve_flags(wp: &mut WinInfoInfo) -> u32 {
    {
        let f = if wp.w_onebuf_opt.wo_ve_flags != 0 {
            wp.w_onebuf_opt.wo_ve_flags
        } else {
            opt_vars().ve_flags
        };
        f & !(K_OPT_VE_FLAG_NONE | K_OPT_VE_FLAG_NONE_U)
    }
}

/// Get the local or global value of 'showbreak'.
pub fn get_showbreak_value(win: &mut WinInfoInfo) -> *mut c_char {
    {
        let sbr = win.w_onebuf_opt.wo_sbr;
        if crate::memory::cstr_is_empty(sbr) {
            return opt_vars().p_sbr;
        }
        if crate::memory::cstr_as_bytes(sbr) == b"NONE" {
            return EMPTY_STRING_OPTION.as_ptr().cast::<c_char>();
        }
        sbr
    }
}

/// Return the current end-of-line type: EOL_DOS, EOL_UNIX or EOL_MAC.
pub fn get_fileformat(buf: &FileBuf) -> i32 {
    {
        let c = crate::memory::cstr_first_byte(buf.b_p_ff);
        if buf.b_p_bin != 0 || c == b'u' {
            return EOL_UNIX;
        }
        if c == b'm' {
            return EOL_MAC;
        }
        EOL_DOS
    }
}

/// Like get_fileformat(), but override 'fileformat' with "p" for "++opt=val" argument.
pub fn get_fileformat_force(buf: &FileBuf, eap: Option<&Exarg>) -> i32 {
    {
        let c = if let Some(eap) = eap
            && eap.force_ff != 0
        {
            eap.force_ff as u8
        } else {
            let is_bin = if let Some(eap) = eap
                && eap.force_bin != 0
            {
                eap.force_bin == FORCE_BIN
            } else {
                buf.b_p_bin != 0
            };
            if is_bin {
                return EOL_UNIX;
            }
            crate::memory::cstr_first_byte(buf.b_p_ff)
        };
        if c == b'u' {
            return EOL_UNIX;
        }
        if c == b'm' {
            return EOL_MAC;
        }
        EOL_DOS
    }
}

/// Return the default fileformat from 'fileformats'.
pub fn default_fileformat() -> i32 {
    {
        match crate::memory::cstr_first_byte(opt_vars().p_ffs) {
            b'm' => EOL_MAC,
            b'd' => EOL_DOS,
            _ => EOL_UNIX,
        }
    }
}

/// Set the current end-of-line type to EOL_UNIX, EOL_MAC, or EOL_DOS.
pub fn set_fileformat(eol_style: i32, opt_flags: i32) {
    let p: *const c_char = match eol_style {
        EOL_UNIX => c"unix".as_ptr(),
        EOL_MAC => c"mac".as_ptr(),
        EOL_DOS => c"dos".as_ptr(),
        _ => ptr::null(),
    };

    if !p.is_null() {
        set_option_direct(K_OPT_FILEFORMAT, cstr_as_obj(p), opt_flags, 0);
    }

    redraw_buf_status_later(g_st().curbuf);
    g_st().redraw_tabline = true;
    g_st().need_maketitle = true;
}

/// Skip to next part of an option argument: skip space and comma.
pub fn skip_to_option_part(p: impl crate::strings::IntoCCharPtr) -> *mut c_char {
    let mut p = p.into_c_char_ptr();
    {
        if crate::memory::cstr_first_byte(p) == b',' {
            p = p.wrapping_add(1);
        }
        while crate::memory::cstr_first_byte(p) == b' ' {
            p = p.wrapping_add(1);
        }
        p.cast_mut()
    }
}

/// Isolate one part of a string option separated by `sep_chars`.
pub fn copy_option_part(
    option: *mut *mut c_char,
    buf: *mut c_char,
    maxlen: usize,
    sep_chars: &std::ffi::CStr,
) -> usize {
    unsafe {
        let mut len: usize = 0;
        let mut p = *option;

        if crate::memory::cstr_first_byte(p) == b'.' {
            *buf.wrapping_add(len) = *p;
            len += 1;
            p = p.wrapping_add(1);
        }
        while !crate::memory::cstr_is_empty(p)
            && !crate::strings::vim_haschr(
                sep_chars.as_ptr(),
                crate::memory::cstr_first_byte(p) as i32,
            )
        {
            if crate::memory::cstr_first_byte(p) == b'\\'
                && crate::strings::vim_haschr(
                    sep_chars.as_ptr(),
                    crate::memory::cstr_byte_at(p, 1) as i32,
                )
            {
                p = p.wrapping_add(1);
            }
            if len < maxlen - 1 {
                *buf.wrapping_add(len) = *p;
                len += 1;
            }
            p = p.wrapping_add(1);
        }
        *buf.wrapping_add(len) = NUL;

        if !crate::memory::cstr_is_empty(p) && crate::memory::cstr_first_byte(p) != b',' {
            p = p.wrapping_add(1);
        }
        p = skip_to_option_part(p);

        *option = p;
        len
    }
}

/// Iterates comma-separated parts of option, splitting each part at its first ':' into key/value.
pub fn option_next_keyval(
    p: *mut *const c_char,
    key: *mut *const c_char,
    keylen: &mut usize,
    val: *mut *const c_char,
    vallen: &mut usize,
) -> bool {
    unsafe {
        let s = *p;
        if crate::memory::cstr_is_empty(s) {
            return false;
        }

        let comma = vim_strchr(s, b',' as i32);
        let end = if !comma.is_null() {
            comma as *const c_char
        } else {
            s.wrapping_add(crate::memory::cstr_len(s))
        };
        let colon = vim_strchr(s, b':' as i32) as *const c_char;

        *key = s;
        if !colon.is_null() && colon < end {
            *keylen = crate::memory::ptr_offset_from(colon, s) as usize;
            *val = colon.wrapping_add(1);
            *vallen = crate::memory::ptr_offset_from(end, colon.wrapping_add(1)) as usize;
        } else {
            *keylen = crate::memory::ptr_offset_from(end, s) as usize;
            *val = ptr::null();
            *vallen = 0;
        }

        *p = if !comma.is_null() {
            (comma as *const c_char).wrapping_add(1)
        } else {
            end
        };
        true
    }
}

/// Compare a (non-NUL-terminated) slice from option_next_keyval() against a name.
pub fn option_slice_eq(
    slice: impl crate::strings::IntoCCharPtr,
    slicelen: usize,
    name: &std::ffi::CStr,
) -> bool {
    let slice = slice.into_c_char_ptr();
    !slice.is_null()
        && name.to_bytes().len() == slicelen
        && (crate::memory::cstr_prefix_bytes(slice, slicelen)
            == crate::memory::cstr_prefix_bytes(name.as_ptr(), slicelen))
}

/// Return true when 'shell' has "csh" in the tail.
pub fn csh_like_shell() -> i32 {
    i32::from(c_str_contains(path_tail(opt_vars().p_sh), b"csh"))
}

/// Return true when 'shell' has "fish" in the tail.
pub fn fish_like_shell() -> bool {
    c_str_contains(path_tail(opt_vars().p_sh), b"fish")
}

/// Get window or buffer local options.
pub fn get_winbuf_options(bufopt: i32) -> &'static mut TvDict {
    {
        let d = tv_dict_alloc();

        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);

            if (bufopt != 0 && option_has_scope(opt_idx, K_OPT_SCOPE_BUF))
                || (bufopt == 0 && option_has_scope(opt_idx, K_OPT_SCOPE_WIN))
            {
                let varp = get_varp(&mut *opt);
                if !varp.is_null() {
                    let ov = opt_from_varp(opt_idx, varp);
                    let mut opt_tv = opt_to_tv(ov, true);
                    tv_dict_add_tv(
                        &mut *d,
                        opt.fullname.as_ptr(),
                        opt.fullname.to_bytes().len(),
                        &mut opt_tv,
                    );
                    tv_clear(&mut opt_tv);
                    optval_free_read(opt_idx, ov);
                }
            }
        }

        d
    }
}

/// Return the effective 'scrolloff' value for the current window.
pub fn get_scrolloff_value(wp: &mut WinInfoInfo) -> i64 {
    {
        if (g_st().state_global & MODE_TERMINAL) != 0 && !wp.buf_mut().terminal.is_null() {
            return 0;
        }
        if wp.w_onebuf_opt.wo_so < 0 {
            opt_vars().p_so
        } else {
            wp.w_onebuf_opt.wo_so
        }
    }
}

/// Return the effective 'scrolloffpad' value for the current window.
pub fn get_scrolloffpad_value(wp: &mut WinInfoInfo) -> i64 {
    {
        if wp.w_onebuf_opt.wo_sop == -1 {
            opt_vars().p_sop
        } else {
            wp.w_onebuf_opt.wo_sop
        }
    }
}

/// Return the effective 'sidescrolloff' value for the current window.
pub fn get_sidescrolloff_value(wp: &mut WinInfoInfo) -> i64 {
    {
        if wp.w_onebuf_opt.wo_siso < 0 {
            opt_vars().p_siso
        } else {
            wp.w_onebuf_opt.wo_siso
        }
    }
}

/// Metadata Dict for option `name`; errors if `name` is unknown.
pub fn get_option_info(
    name: NvimString,
    opt_flags: i32,
    buf: *mut FileBuf,
    win: &mut WinInfoInfo,
    arena: &mut Arena,
    err: &mut Error,
) -> Dict {
    unsafe {
        let opt_idx = find_option_len(name.data, name.size);
        if opt_idx == K_OPT_INVALID {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Invalid option (not found): '%s'",
                name.data,
            );
            return Dict::new();
        }

        option_info_dict(&mut *get_option(opt_idx), opt_flags, &mut *buf, win, arena)
    }
}

/// Metadata Dict for every option, keyed by full name.
pub fn get_all_options_info(arena: &mut Arena) -> Dict {
    {
        let mut retval = arena_dict(arena, K_OPT_COUNT as usize);
        for opt_idx in 0..K_OPT_COUNT {
            let opt = get_option(opt_idx);
            let opt_dict =
                option_info_dict(&mut *opt, OPT_GLOBAL, curbuf_mut(), curwin_mut(), arena);
            put_c(&mut retval, opt.fullname.as_ptr(), dict_obj(opt_dict));
        }
        retval
    }
}

fn option_info_dict(
    opt: &mut Vimoption,
    opt_flags: i32,
    buf: &mut FileBuf,
    win: &mut WinInfoInfo,
    arena: &mut Arena,
) -> Dict {
    {
        let opt_idx = get_opt_idx(&raw mut *opt);
        let mut dict = arena_dict(arena, 13);

        put_c(&mut dict, c"name", cstr_as_obj(opt.fullname.as_ptr()));
        put_c(
            &mut dict,
            c"shortname",
            cstr_as_obj(opt.shortname.map_or(ptr::null(), |s| s.as_ptr())),
        );

        let scope = if option_has_scope(opt_idx, K_OPT_SCOPE_BUF) {
            c"buf".as_ptr()
        } else if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
            c"win".as_ptr()
        } else if option_has_scope(opt_idx, K_OPT_SCOPE_TAB) {
            c"tab".as_ptr()
        } else {
            c"global".as_ptr()
        };

        put_c(&mut dict, c"scope", cstr_as_obj(scope));

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
                script_ctx = buf.b_p_script_ctx[opt.scope_idx[K_OPT_SCOPE_BUF as usize] as usize];
            }
            if option_has_scope(opt_idx, K_OPT_SCOPE_WIN) {
                script_ctx = win.w_onebuf_opt.wo_script_ctx
                    [opt.scope_idx[K_OPT_SCOPE_WIN as usize] as usize];
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
            cstr_as_obj(optval_type_name(
                get_option(get_opt_idx(&raw mut *opt)).opt_type,
            )),
        );
        put_c(&mut dict, c"default", opt.def_val);
        put_c(
            &mut dict,
            c"allows_duplicates",
            boolean_obj((opt.flags & K_OPT_FLAG_NO_DUP) == 0),
        );

        dict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_option_and_find_option_len_resolve_names_and_abbreviations() {
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(find_option(c"number"), K_OPT_NUMBER);
        assert_eq!(find_option(c"nu"), K_OPT_NUMBER);
        assert_eq!(find_option_len(c"tabstop=4", 7), K_OPT_TABSTOP);
        assert_eq!(find_option(c"nonexistent_option"), K_OPT_INVALID);

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
