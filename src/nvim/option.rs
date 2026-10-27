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
    EVENT_BUFADD, EVENT_BUFDELETE, EVENT_OPTIONSET, EVENT_SYNTAX, OPT_NOWIN, apply_autocmds,
    aucmd_defer_modified, do_filetype_autocmd, kOptModified,
};
use crate::buffer::{
    BCO_ALWAYS, KEYMAP_INIT, NO_LOCAL_UNDOLEVEL, OPT_MODELINE, bt_prompt, buf_T, buf_is_empty,
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
    EXPAND_BOOL_SETTINGS, EXPAND_DIRECTORIES, EXPAND_FILES, EXPAND_FILETYPE, EXPAND_KEYMAP,
    EXPAND_NOTHING, EXPAND_OLD_SETTING, EXPAND_OWNSYNTAX, EXPAND_SETTING_SUBTRACT, EXPAND_SETTINGS,
    EXPAND_STRING_SETTING, EXPAND_UNSUCCESSFUL, XP_BS_COMMA, XP_BS_ONE, XP_BS_THREE, XP_PREFIX_INV,
    XP_PREFIX_NO, cmdidx_T, cmdline_fuzzy_complete,
};
use crate::context::{CtxSwitch, ctx_restore, ctx_switch};
use crate::cursor_shape::{SHAPE_CURSOR, parse_shape_opt};
use crate::decoration_provider::get_decor_provider;
use crate::diff::{diff_buf_adjust, kOptDiff, kOptFoldmethod};
use crate::drawscreen::{
    UPD_CLEAR, UPD_NOT_VALID, UPD_SOME_VALID, check_screensize, comp_col, redraw_all_later,
    redraw_buf_later, redraw_buf_status_later, redraw_later, screen_resize, showmode,
    status_redraw_all, status_redraw_curbuf,
};
use crate::errors::*;
use crate::eval::encode::encode_tv2string;
use crate::eval::typval::{
    Callback, callback_copy, callback_free, callback_put, dict_T, tv_clear, tv_dict_add_tv,
    tv_dict_alloc, typval_T,
};
use crate::eval::userfunc::get_scriptlocal_funcname;
use crate::eval::vars::{
    FORCE_BIN, VV_OPTION_COMMAND, VV_OPTION_NEW, VV_OPTION_OLD, VV_OPTION_OLDGLOBAL,
    VV_OPTION_OLDLOCAL, VV_OPTION_TYPE, VV_WARNINGMSG, get_vim_var_str, kOptFlagFunc, opt_to_tv,
    reset_v_option_vars, scid_T, set_vim_var_string, set_vim_var_tv, tristate_from_int,
};
use crate::eval::{STR2NR_ALL, callback_from_typval, eval_expr, last_set_msg};
use crate::ex_docmd::{
    CMOD_NOSWAPFILE, UNDO_HASH_SIZE, exarg_T, free_findfunc_option, kOptFindfunc, set_no_hlsearch,
};
use crate::ex_getln::{
    B_IMODE_NONE, B_IMODE_USE_INSERT, check_opt_wim, did_set_cedit, gotocmdline,
};
use crate::ex_session::{OPT_SKIPRTP, put_eol, put_line};
#[cfg(unix)]
use crate::fileio::getuid;
use crate::fileio::{EOL_DOS, EOL_UNIX, FILE, fprintf, fputs, kOptFileencoding};
use crate::fold::{
    foldUpdateAll, foldmethodIsDiff, foldmethodIsIndent, foldmethodIsSyntax, newFoldLevel,
};
use crate::fuzzy::{
    FUZZY_SCORE_NONE, fuzmatch_str_T, fuzzy_match_str, fuzzymatches_slice_to_strmatches,
};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::highlight::{HL_GLOBAL, hl_invalidate_blends, ns_hl_def};
use crate::highlight_group::{highlight_changed, syn_check_group};
use crate::indent::{
    briopt_check, kBufOptIndentexpr, kOptBreakindentopt, kOptIndentexpr, kOptTabstop,
    kOptVartabstop, tabstop_set,
};
use crate::indent_c::parse_cino;
use crate::input_cmdatom::atom_follow_changed;
use crate::insexpand::{kOptComplete, set_buflocal_cpt_callbacks};
use crate::keycodes::{
    FSK_KEEP_X_KEY, FSK_KEYCODE, FSK_SIMPLIFY, K_KENTER, K_ZERO, find_special_key,
    find_special_key_in_table, get_special_key_code, get_special_key_name, termcap2key,
};
use crate::log::logmsg;

use crate::highlight::HlAttrs;
use crate::lua::executor::{api_free_luaref, api_new_luaref, nlua_funcref_str, nlua_set_sctx};
use crate::mapping::{langmap_init, put_escstr};
use crate::math::{FAIL, OK};
use crate::mbyte::{enc_locale, utfc_ptr2len};
use crate::mcursor::{kOptFollow, mc_ins_replay_can_join};
use crate::memfile::mf_close_file;
use crate::memline::{ml_open_file, ml_open_files};
use crate::memory::{Arena, strequal, strnequal, xmemcpyz, xstrlcpy};
use crate::message::{
    emsg, iemsg, message_filtered, msg, msg_advance, msg_ext_set_kind, msg_outtrans, msg_putchar,
    msg_puts, msg_puts_title, msg_source,
};
use crate::mouse::setmouse;
use crate::r#move::changed_window_setting;
use crate::normal::{do_check_scrollbind, get_vtopline, kOptFoldenable, kOptFoldlevel};
use crate::ops::free_operatorfunc_option;
use crate::option_vars::*;
use crate::optionstr::{
    CharBuf, CharsOption, OptIndex, OptScopeFlags, check_buf_options, check_illegal_path_names,
    alloc_string_option, check_signcolumn, check_string_option, clear_string_option,
    did_set_breakat, didset_string_options, free_string_option, opt_dict_schema,
    opt_strings_check, optexpand_T,
    optset_T, set_chars_option, vimoption_T,
};
use crate::os::env::{
    expand_T, expand_env_esc, home_replace, os_env_exists, os_getenv_cstr, os_setenv, vim_getenv,
};
use crate::os::input::os_breakcheck;
use crate::os::lang::{get_mess_lang, lang_init};
use crate::os::stdpaths::{stdpaths_user_conf_subpath, stdpaths_user_state_subpath};
use crate::path::{
    FullName_save, after_pathsep, invocation_path_tail, path_tail, vim_ispathlistsep,
};
use crate::popupmenu::{pum_drawn, pum_redraw};
use crate::quickfix::{ll_resize_stack, qf_resize_stack};
use crate::regexp::{MAXCOL, NUMBUFLEN, regmatch_T, vim_regexec};
use crate::runtime::{
    DIP_ALL, SID_NONE, runtimepath_default, sctx_T, source_runtime_vim_lua, sourcing_lnum,
};
use crate::spell::{compile_cap_prog, did_set_spell_option, init_spell_chartab, parse_spelllang};
use crate::spellsuggest::{spell_apply_sps, spell_check_sps};
use crate::strings::{
    gettext, strchr, strcmp, strcpy, strlen, strncmp, strstr, tr, vim_snprintf,
    vim_snprintf_safelen, vim_strchr, vim_strsave_escaped,
};
use crate::syntax::synblock_T;
use crate::tag::free_tagfunc_option;
use crate::terminal::on_scrollback_option_changed;
use crate::ui::ui_call_option_set;
use crate::undo::{bufIsChanged, curbufIsChanged, u_compute_hash, u_read_undo, u_sync};
use crate::window::{
    MIN_COLUMNS, STATUS_HEIGHT, check_colorcolumn, command_height, frame_new_height,
    global_stl_height, last_status, min_rows, min_rows_for_all_tabpages, set_winbar,
    set_winbar_all, set_winbar_win, tabline_height, tabpage_T, unuse_tabpage, use_tabpage, win_T,
    win_comp_pos, win_comp_scroll, win_default_scroll, win_equal, win_find_tabpage,
    win_new_screen_rows, win_setheight, win_setwidth, winopt_T,
};
use crate::winfloat::win_float_update_statusline;
use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

pub type OptInt = i64;
pub type OptScope = c_int;
pub type set_op_T = c_int;
pub type set_prefix_T = c_int;

pub(crate) const OPT_GLOBAL: c_int = 0x01;
pub(crate) const OPT_LOCAL: c_int = 0x02;
pub(crate) const PREFIX_NONE: set_prefix_T = 1;
pub(crate) const kOptScopeGlobal: OptScope = 0;
pub(crate) const kOptScopeWin: OptScope = 1;
pub(crate) const kOptScopeBuf: OptScope = 2;
pub(crate) const kOptScopeTab: OptScope = 3;
pub(crate) const kOptInvalid: OptIndex = -1;
pub(crate) const OP_NONE: set_op_T = 0;
pub(crate) const OP_ADDING: set_op_T = 1;
pub(crate) const OP_PREPENDING: set_op_T = 2;
pub(crate) const OP_REMOVING: set_op_T = 3;
pub(crate) const kOptBufhidden: OptIndex = 27;
pub(crate) const kOptBuftype: OptIndex = 29;
pub(crate) const kOptFiletype: OptIndex = 97;
pub(crate) const kOptModeline: OptIndex = 192;
pub(crate) const kOptSwapfile: OptIndex = 301;

use crate::api::ui::NO_SCREEN;
use crate::api::vim::ERR_BUFLEN;
use crate::api::win_config::kCtxNoDisplay;
use crate::buffer::{BCO_ENTER, BCO_NOHELP};
use crate::charset::NUL;
use crate::drawline::{
    kOptCuloptFlagLine, kOptCuloptFlagNumber, kOptCuloptFlagScreenline, kOptFlagInsecure,
};
use crate::eval::typval::{
    kCallbackExpr, kCallbackFuncref, kCallbackLua, kCallbackNone, kCallbackPartial,
};
use crate::ex_cmds::{OPT_WINONLY, kOptFoldcolumn};
use crate::ex_cmds2::BF_SYN_SET;
use crate::extmark::{TriState, kFalse, kNone, kTrue};
use crate::file_search::{kBufOptIncludeexpr, kOptIncludeexpr};
use crate::help::kOptIskeyword;
use crate::insert::{BS_EOL, BS_NOSTOP, BS_START};
use crate::os::fs::MAXPATHL;
use crate::state::MODE_TERMINAL;

const OPT_ONECOLUMN: c_int = 0x20;

const PREFIX_NO: set_prefix_T = 0;
const PREFIX_INV: set_prefix_T = 2;

const kOptScopeSize: OptScope = 4;

const kOptFlagExpand: u32 = 1 << 0;
const kOptFlagNoDefExp: u32 = 1 << 1;
const kOptFlagNoDefault: u32 = 1 << 2;
const kOptFlagWasSet: u32 = 1 << 3;
const kOptFlagNoMkrc: u32 = 1 << 4;
const kOptFlagUIOption: u32 = 1 << 5;
const kOptFlagRedrTabl: u32 = 1 << 6;
const kOptFlagRedrStat: u32 = 1 << 7;
const kOptFlagRedrWin: u32 = 1 << 8;
const kOptFlagRedrBuf: u32 = 1 << 9;
const kOptFlagRedrAll: u32 = kOptFlagRedrBuf | kOptFlagRedrWin;
pub const kOptFlagComma: u32 = 1 << 10;
pub(crate) const kOptFlagOneComma: u32 = (1 << 11) | kOptFlagComma;
const kOptFlagNoDup: u32 = 1 << 12;
pub const kOptFlagFlagList: u32 = 1 << 13;
const kOptFlagSecure: u32 = 1 << 14;
const kOptFlagGettext: u32 = 1 << 15;
const kOptFlagNoGlob: u32 = 1 << 16;
pub(crate) const kOptFlagNFname: u32 = 1 << 17;
const kOptFlagPriMkrc: u32 = 1 << 19;
const kOptFlagCurswant: u32 = 1 << 20;
pub(crate) const kOptFlagNDname: u32 = 1 << 21;
const kOptFlagHLOnly: u32 = 1 << 22;
const kOptFlagMLE: u32 = 1 << 23;
pub const kOptFlagColon: u32 = 1 << 25;
const kOptFlagExpr: u32 = 1 << 26;

pub(crate) const kOptArabic: OptIndex = 3;
const kOptAutocomplete: OptIndex = 6;
const kOptAutoindent: OptIndex = 9;
const kOptAutoread: OptIndex = 10;
pub(crate) const kOptBackupcopy: OptIndex = 16;
const kOptBackupdir: OptIndex = 17;
const kOptBackupskip: OptIndex = 19;
const kOptBinary: OptIndex = 21;
const kOptBomb: OptIndex = 22;
const kOptBreakindent: OptIndex = 24;
pub(crate) const kOptBuflisted: OptIndex = 28;
const kOptBusy: OptIndex = 30;
const kOptCdpath: OptIndex = 33;
const kOptChannel: OptIndex = 35;
const kOptChistory: OptIndex = 37;
const kOptCindent: OptIndex = 38;
const kOptCinkeys: OptIndex = 39;
const kOptCinoptions: OptIndex = 40;
const kOptCinscopedecls: OptIndex = 41;
const kOptCinwords: OptIndex = 42;
pub(crate) const kOptCmdheight: OptIndex = 44;
const kOptCmdwinheight: OptIndex = 45;
const kOptColorcolumn: OptIndex = 46;
const kOptColumns: OptIndex = 47;
pub(crate) const kOptComments: OptIndex = 48;
const kOptCommentstring: OptIndex = 49;
const kOptCompletefunc: OptIndex = 52;
pub(crate) const kOptCompleteopt: OptIndex = 54;
#[cfg(windows)]
const kOptCompleteslash: OptIndex = 55;
const kOptConcealcursor: OptIndex = 57;
const kOptConceallevel: OptIndex = 58;
const kOptCopyindent: OptIndex = 60;
const kOptCursorbind: OptIndex = 62;
const kOptCursorcolumn: OptIndex = 63;
const kOptCursorline: OptIndex = 64;
const kOptCursorlineopt: OptIndex = 65;
const kOptDefine: OptIndex = 67;
const kOptDictionary: OptIndex = 69;
const kOptDiffanchors: OptIndex = 71;
const kOptDirectory: OptIndex = 75;
const kOptEndoffile: OptIndex = 81;
const kOptEndofline: OptIndex = 82;
const kOptEqualprg: OptIndex = 84;
const kOptErrorformat: OptIndex = 87;
pub(crate) const kOptEventignorewin: OptIndex = 89;
const kOptExpandtab: OptIndex = 90;
pub(crate) const kOptFileformat: OptIndex = 94;
pub(crate) const kOptFileformats: OptIndex = 95;
const kOptFillchars: OptIndex = 98;
const kOptFixendofline: OptIndex = 100;
pub(crate) const kOptFoldexpr: OptIndex = 104;
const kOptFoldignore: OptIndex = 105;
const kOptFoldmarker: OptIndex = 108;
const kOptFoldminlines: OptIndex = 110;
const kOptFoldnestmax: OptIndex = 111;
pub(crate) const kOptFoldtext: OptIndex = 113;
pub(crate) const kOptFormatexpr: OptIndex = 115;
const kOptFormatlistpat: OptIndex = 116;
const kOptFormatoptions: OptIndex = 117;
const kOptFormatprg: OptIndex = 118;
const kOptFsync: OptIndex = 119;
const kOptGrepformat: OptIndex = 121;
const kOptGrepprg: OptIndex = 122;
const kOptHelpheight: OptIndex = 130;
const kOptHelplang: OptIndex = 131;
const kOptHistory: OptIndex = 134;
const kOptIcon: OptIndex = 138;
const kOptIminsert: OptIndex = 143;
const kOptImsearch: OptIndex = 144;
const kOptInclude: OptIndex = 146;
const kOptIndentkeys: OptIndex = 150;
const kOptInfercase: OptIndex = 151;
pub(crate) const kOptKeymap: OptIndex = 159;
const kOptKeywordprg: OptIndex = 161;
const kOptLhistory: OptIndex = 168;
const kOptLinebreak: OptIndex = 169;
const kOptLines: OptIndex = 170;
const kOptLisp: OptIndex = 172;
const kOptLispoptions: OptIndex = 173;
const kOptLispwords: OptIndex = 174;
const kOptList: OptIndex = 175;
pub(crate) const kOptListchars: OptIndex = 176;
const kOptMakeencoding: OptIndex = 180;
const kOptMakeprg: OptIndex = 181;
const kOptMatchpairs: OptIndex = 182;
const kOptMaxcombine: OptIndex = 184;
const kOptMaxsearchcount: OptIndex = 188;
const kOptModifiable: OptIndex = 195;
const kOptNrformats: OptIndex = 206;
const kOptNumber: OptIndex = 207;
const kOptNumberwidth: OptIndex = 208;
const kOptOmnifunc: OptIndex = 209;
const kOptPacklockfile: OptIndex = 212;
const kOptPackpath: OptIndex = 213;
const kOptPath: OptIndex = 219;
const kOptPreserveindent: OptIndex = 220;
const kOptPreviewwindow: OptIndex = 223;
const kOptPumblend: OptIndex = 225;
const kOptPyxversion: OptIndex = 230;
const kOptQuoteescape: OptIndex = 232;
const kOptReadonly: OptIndex = 233;
const kOptRegexpengine: OptIndex = 236;
const kOptRelativenumber: OptIndex = 237;
const kOptReport: OptIndex = 239;
const kOptRevins: OptIndex = 240;
pub(crate) const kOptRightleft: OptIndex = 241;
const kOptRightleftcmd: OptIndex = 242;
const kOptRuler: OptIndex = 243;
pub(crate) const kOptRuntimepath: OptIndex = 245;
const kOptScroll: OptIndex = 246;
const kOptScrollback: OptIndex = 247;
const kOptScrollbind: OptIndex = 248;
const kOptScrolljump: OptIndex = 249;
const kOptScrolloff: OptIndex = 250;
const kOptScrolloffpad: OptIndex = 251;
const kOptShell: OptIndex = 260;
const kOptShellcmdflag: OptIndex = 261;
const kOptShellpipe: OptIndex = 262;
const kOptShellredir: OptIndex = 264;
#[cfg(windows)]
const kOptShellslash: OptIndex = 265;
const kOptShellxquote: OptIndex = 268;
const kOptShiftwidth: OptIndex = 270;
const kOptShowbreak: OptIndex = 272;
const kOptShowmatch: OptIndex = 276;
const kOptSidescroll: OptIndex = 279;
const kOptSidescrolloff: OptIndex = 280;
const kOptSigncolumn: OptIndex = 281;
const kOptSmartindent: OptIndex = 283;
const kOptSmarttab: OptIndex = 284;
const kOptSmoothscroll: OptIndex = 285;
const kOptSofttabstop: OptIndex = 286;
pub(crate) const kOptSpell: OptIndex = 287;
const kOptSpellcapcheck: OptIndex = 288;
pub(crate) const kOptSpellfile: OptIndex = 289;
pub(crate) const kOptSpelllang: OptIndex = 290;
const kOptSpelloptions: OptIndex = 291;
pub(crate) const kOptStatuscolumn: OptIndex = 297;
pub(crate) const kOptStatusline: OptIndex = 298;
const kOptSuffixesadd: OptIndex = 300;
const kOptSynmaxcol: OptIndex = 303;
const kOptSyntax: OptIndex = 304;
pub(crate) const kOptTagcase: OptIndex = 310;
const kOptTagfunc: OptIndex = 311;
const kOptTags: OptIndex = 314;
const kOptTermbidi: OptIndex = 316;
const kOptTextwidth: OptIndex = 322;
const kOptThesaurus: OptIndex = 323;
const kOptThesaurusfunc: OptIndex = 324;
const kOptTimeoutlen: OptIndex = 327;
const kOptTitle: OptIndex = 328;
const kOptTitlelen: OptIndex = 329;
const kOptTtyfast: OptIndex = 334;
const kOptUndodir: OptIndex = 335;
const kOptUndofile: OptIndex = 336;
const kOptUndolevels: OptIndex = 337;
const kOptUpdatecount: OptIndex = 339;
const kOptUpdatetime: OptIndex = 340;
pub(crate) const kOptVarsofttabstop: OptIndex = 341;
const kOptViewdir: OptIndex = 345;
pub(crate) const kOptVirtualedit: OptIndex = 347;
const kOptWildchar: OptIndex = 351;
const kOptWildcharm: OptIndex = 352;
pub(crate) const kOptWinbar: OptIndex = 359;
const kOptWinblend: OptIndex = 360;
pub(crate) const kOptWindow: OptIndex = 362;
const kOptWinfixbuf: OptIndex = 363;
const kOptWinfixheight: OptIndex = 364;
const kOptWinfixwidth: OptIndex = 365;
const kOptWinheight: OptIndex = 366;
const kOptWinhighlight: OptIndex = 367;
const kOptWinminheight: OptIndex = 368;
const kOptWinminwidth: OptIndex = 369;
const kOptWinpinned: OptIndex = 370;
const kOptWinwidth: OptIndex = 371;
const kOptWrap: OptIndex = 372;
const kOptWrapmargin: OptIndex = 373;
const kOptWritedelay: OptIndex = 378;
const kOptCount: OptIndex = 379;

const kBufOptAutoindent: usize = 1;
const kBufOptBinary: usize = 4;
const kBufOptBomb: usize = 5;
const kBufOptCindent: usize = 11;
const kBufOptCinkeys: usize = 12;
const kBufOptCinoptions: usize = 13;
const kBufOptCinscopedecls: usize = 14;
const kBufOptCinwords: usize = 15;
const kBufOptComments: usize = 16;
const kBufOptCommentstring: usize = 17;
const kBufOptComplete: usize = 18;
const kBufOptCompletefunc: usize = 19;
#[cfg(windows)]
const kBufOptCompleteslash: usize = 21;
const kBufOptCopyindent: usize = 22;
const kBufOptExpandtab: usize = 30;
const kBufOptFixendofline: usize = 35;
const kBufOptFollow: usize = 36;
pub(crate) const kBufOptFormatexpr: usize = 37;
const kBufOptFormatlistpat: usize = 38;
const kBufOptFormatoptions: usize = 39;
const kBufOptIminsert: usize = 44;
const kBufOptImsearch: usize = 45;
const kBufOptIndentkeys: usize = 49;
const kBufOptInfercase: usize = 50;
const kBufOptIskeyword: usize = 51;
const kBufOptKeymap: usize = 52;
const kBufOptLisp: usize = 54;
const kBufOptLispoptions: usize = 55;
const kBufOptMatchpairs: usize = 59;
const kBufOptModeline: usize = 60;
const kBufOptModifiable: usize = 61;
const kBufOptNrformats: usize = 63;
const kBufOptOmnifunc: usize = 64;
const kBufOptPreserveindent: usize = 66;
const kBufOptQuoteescape: usize = 67;
const kBufOptScrollback: usize = 69;
const kBufOptShiftwidth: usize = 70;
const kBufOptSmartindent: usize = 71;
const kBufOptSofttabstop: usize = 72;
const kBufOptSpellcapcheck: usize = 73;
const kBufOptSpellfile: usize = 74;
const kBufOptSpelllang: usize = 75;
const kBufOptSpelloptions: usize = 76;
const kBufOptSuffixesadd: usize = 77;
const kBufOptSwapfile: usize = 78;
const kBufOptSynmaxcol: usize = 79;
const kBufOptTabstop: usize = 81;
const kBufOptTagfunc: usize = 83;
const kBufOptTextwidth: usize = 85;
const kBufOptUndofile: usize = 88;
const kBufOptVarsofttabstop: usize = 90;
const kBufOptVartabstop: usize = 91;
const kBufOptWrapmargin: usize = 92;

const kCpoBufopt: c_int = b's' as c_int;
const kCpoBufoptglob: c_int = b'S' as c_int;

const kOptVeFlagNone: c_uint = 0x10;
const kOptVeFlagNoneU: c_uint = 0x20;

pub(crate) const SB_MAX: OptInt = 1_000_000;
const MAX_NUMBERWIDTH: OptInt = 20;
const MAX_MCO: OptInt = 6;

const B_IMODE_LAST: OptInt = 1;

const Ctrl_C: OptInt = 3;

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

static mut P_TERM: Option<std::ffi::CString> = None;
static mut P_TTYTYPE: Option<std::ffi::CString> = None;

// Saved values for when 'bin' is set.
static mut P_ET_NOBIN: c_int = 0;
static mut P_ML_NOBIN: c_int = 0;
static mut P_TW_NOBIN: OptInt = 0;
static mut P_WM_NOBIN: OptInt = 0;

// Saved values for when 'paste' is set.
static mut P_AI_NOPASTE: c_int = 0;
static mut P_ET_NOPASTE: c_int = 0;
static mut P_STS_NOPASTE: OptInt = 0;
static mut P_TW_NOPASTE: OptInt = 0;
static mut P_WM_NOPASTE: OptInt = 0;
static mut P_VSTS_NOPASTE: Option<std::ffi::CString> = None;

// Saved values inside did_set_paste.
static mut OLD_P_PASTE: c_int = 0;
static mut SAVE_SM: c_int = 0;
static mut SAVE_STA: c_int = 0;
static mut SAVE_RU: c_int = 0;
static mut SAVE_RI: c_int = 0;

// Saved recursion depth inside do_syntax_autocmd.
static mut SYN_RECURSIVE: c_int = 0;

// Saved state for command-line expansion of :set.
static mut EXPAND_OPTION_IDX: OptIndex = kOptInvalid;
static mut EXPAND_OPTION_START_COL: c_int = 0;
static mut EXPAND_OPTION_NAME: [c_char; 5] = [b't' as c_char, b'_' as c_char, 0, 0, 0];
static mut EXPAND_OPTION_FLAGS: c_int = 0;
static mut EXPAND_OPTION_APPEND: bool = false;

static mut P_BIN_DEP_OPTS: [c_int; 5] = [
    kOptTextwidth,
    kOptWrapmargin,
    kOptModeline,
    kOptExpandtab,
    kOptInvalid,
];

static mut P_PASTE_DEP_OPTS: [c_int; 11] = [
    kOptAutoindent,
    kOptExpandtab,
    kOptRuler,
    kOptShowmatch,
    kOptSmarttab,
    kOptSofttabstop,
    kOptTextwidth,
    kOptWrapmargin,
    kOptRevins,
    kOptVarsofttabstop,
    kOptInvalid,
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

pub unsafe fn opt_error(
    errbuf: *const CharBuf,
    fmt: *const c_char,
    arg: *const c_char,
) -> *const c_char {
    if errbuf.is_null() {
        return c"".as_ptr();
    }
    unsafe {
        vim_snprintf!((*errbuf).data, (*errbuf).size, gettext(fmt), arg);
        (*errbuf).data.cast_const()
    }
}

pub unsafe fn opt_error_int(
    errbuf: *const CharBuf,
    fmt: *const c_char,
    arg: c_int,
) -> *const c_char {
    if errbuf.is_null() {
        return c"".as_ptr();
    }
    unsafe {
        vim_snprintf!((*errbuf).data, (*errbuf).size, gettext(fmt), arg);
        (*errbuf).data.cast_const()
    }
}

#[inline]
unsafe fn c_strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char {
    unsafe { strstr(haystack, needle) }
}

#[inline]
const fn unset_obj() -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeUnset,
        data: ObjectData::Integer(0),
    }
}

use crate::keycodes::{ctrl_chr, key2termcap0, key2termcap1};

#[inline]
unsafe fn optval_type_name(t: ObjectType) -> *const c_char {
    match t {
        ObjectType::kObjectTypeBoolean => c"boolean".as_ptr(),
        ObjectType::kObjectTypeInteger => c"number".as_ptr(),
        ObjectType::kObjectTypeString => c"string".as_ptr(),
        _ => std::process::abort(),
    }
}

#[inline]
unsafe fn join_cstrings(items: &[std::ffi::CString], sep: &[u8]) -> std::ffi::CString {
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
unsafe fn get_opt_idx(opt: *mut vimoption_T) -> OptIndex {
    unsafe { opt.offset_from(get_option(0)) as OptIndex }
}

#[inline]
unsafe fn get_varp(p: *mut vimoption_T) -> *mut c_void {
    unsafe { get_varp_from(p, curbuf, curwin) }
}

pub unsafe fn set_init_tablocal() {
    unsafe {
        // cmdheight calls itself OPT_GLOBAL but is really tablocal!
        p_ch = (*get_option(kOptCmdheight)).def_val.data.integer();
    }
}

/// Initialize the 'shell' option to a default value.
unsafe fn set_init_default_shell() {
    unsafe {
        if let Some(shell) = os_getenv_cstr(c"SHELL".as_ptr()) {
            let shell_ptr = shell.as_ptr();
            if !vim_strchr(shell_ptr, b' ' as c_int).is_null() {
                let len = shell.as_bytes().len() + 3;
                let mut cmd = vec![0 as c_char; len];
                vim_snprintf!(cmd.as_mut_ptr(), len, c"\"%s\"".as_ptr(), shell_ptr);
                set_string_default(kOptShell, cmd.as_ptr());
            } else {
                set_string_default(kOptShell, shell_ptr);
            }
        }
    }
}

/// Set the default for 'backupskip' to include environment variables for temp files.
unsafe fn set_init_default_backupskip() {
    unsafe {
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
        let opt_idx = kOptBackupskip;

        for &name in &names {
            #[cfg(unix)]
            let (p_owned, p, mut plen): (Option<std::ffi::CString>, *const c_char, usize) =
                if *name == NUL {
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
            let (p_owned, p, mut plen): (Option<std::ffi::CString>, *const c_char, usize) = {
                let env = vim_getenv(name);
                let ptr = env.as_ref().map_or(ptr::null(), |s| s.as_ptr());
                (env, ptr, 0)
            };
            let _ = &p_owned;

            if !p.is_null() && *p != NUL {
                let mut has_trailing_path_sep = false;

                if plen == 0 {
                    plen = strlen(p);
                    if after_pathsep(p, p.add(plen)) != 0 {
                        has_trailing_path_sep = true;
                    }
                }

                let itemsize = plen + usize::from(!has_trailing_path_sep) + 2;
                let mut item = vec![0 as c_char; itemsize];

                let itemlen = vim_snprintf!(
                    item.as_mut_ptr(),
                    itemsize,
                    c"%s%s*".as_ptr(),
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
                    (*get_option(opt_idx)).flags,
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
            set_string_default(kOptBackupskip, buf.as_ptr());
        }
    }
}

/// Initialize the 'cdpath' option to a default value.
unsafe fn set_init_default_cdpath() {
    unsafe {
        let Some(cdpath_cstr) = vim_getenv(c"CDPATH".as_ptr()) else {
            return;
        };
        let cdpath = cdpath_cstr.as_ptr();

        let mut buf_vec = vec![0 as c_char; 2 * cdpath_cstr.as_bytes().len() + 2];
        let buf = buf_vec.as_mut_ptr();
        *buf = b',' as c_char;
        let mut j: usize = 1;
        let mut i: usize = 0;
        while *cdpath.add(i) != NUL {
            let ch = *cdpath.add(i);
            if vim_ispathlistsep(ch as c_int) {
                *buf.add(j) = b',' as c_char;
                j += 1;
            } else {
                if ch == b' ' as c_char || ch == b',' as c_char {
                    *buf.add(j) = b'\\' as c_char;
                    j += 1;
                }
                *buf.add(j) = ch;
                j += 1;
            }
            i += 1;
        }
        *buf.add(j) = NUL;
        change_option_default(kOptCdpath, cstr_to_obj(buf));
    }
}

/// Expand environment variables and things like "~" for the defaults.
unsafe fn set_init_expand_env() {
    unsafe {
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            if ((*opt).flags & kOptFlagNoDefExp) != 0 {
                continue;
            }
            let mut p: *mut c_char = ptr::null_mut();
            if ((*opt).flags & kOptFlagGettext) != 0 && !(*opt).var.is_null() {
                p = tr(*((*opt).var.cast::<*mut c_char>()));
            } else if ((*opt).flags & kOptFlagExpand) != 0 {
                p = option_expand(opt_idx, (*opt).def_val.data.string().data);
            }
            if !p.is_null() {
                let ov = cstr_to_obj(p);
                set_option_varp(opt_idx, (*opt).var, ov);
                change_option_default(opt_idx, ov);
            }
        }
    }
}

/// Initialize the encoding used for "default" in 'fileencodings'.
unsafe fn set_init_fenc_default() {
    unsafe {
        fenc_default = Some(enc_locale().unwrap_or_else(|| c"utf-8".to_owned()));
    }
}

/// Initialize the options, first part.
pub unsafe fn set_init_1(clean_arg: bool) {
    unsafe {
        langmap_init();

        alloc_options_default();

        set_init_default_shell();
        set_init_default_backupskip();
        set_init_default_cdpath();

        let raw_backupdir = stdpaths_user_state_subpath(c"backup".as_ptr(), 2, true);
        let mut backupdir_bytes = Vec::with_capacity(raw_backupdir.as_bytes().len() + 2);
        backupdir_bytes.extend_from_slice(b".,");
        backupdir_bytes.extend_from_slice(raw_backupdir.as_bytes());
        let backupdir = crate::memory::cstr_from_bytes(&backupdir_bytes);
        set_string_default(kOptBackupdir, backupdir.as_ptr());
        set_string_default(
            kOptViewdir,
            stdpaths_user_state_subpath(c"view".as_ptr(), 2, false).as_ptr(),
        );
        set_string_default(
            kOptDirectory,
            stdpaths_user_state_subpath(c"swap".as_ptr(), 2, true).as_ptr(),
        );
        set_string_default(
            kOptUndodir,
            stdpaths_user_state_subpath(c"undo".as_ptr(), 2, true).as_ptr(),
        );
        set_string_default(
            kOptPacklockfile,
            stdpaths_user_conf_subpath(c"nvim-pack-lock.json".as_ptr()).as_ptr(),
        );
        if let Some(rtp) = runtimepath_default(clean_arg) {
            set_string_default(kOptRuntimepath, rtp.as_ptr());
            set_string_default(kOptPackpath, rtp.as_ptr());
        }

        set_options_default(0);

        (*curbuf).b_p_initialized = true;
        (*curbuf).b_p_ac = -1;
        (*curbuf).b_p_ar = -1;
        (*curbuf).b_p_fs = -1;
        (*curbuf).b_p_ul = NO_LOCAL_UNDOLEVEL;
        check_buf_options(curbuf);
        check_win_options(curwin);
        check_options();

        last_status(false);

        didset_options();

        init_spell_chartab();

        set_init_expand_env();

        if os_env_exists(c"NVIM_NOTTYFAST".as_ptr(), false) {
            set_option_value_give_err(kOptTtyfast, boolean_obj(false), 0);
        }

        save_file_ff(curbuf);

        if os_env_exists(c"MLTERM".as_ptr(), false) {
            set_option_value_give_err(kOptTermbidi, boolean_obj(true), 0);
        }

        didset_options2();

        lang_init();
        set_init_fenc_default();

        let _ = bind_textdomain_codeset(c"nvim".as_ptr(), p_enc);

        set_helplang_default(get_mess_lang());
    }
}

/// Get default value for option, based on the option's type and scope.
pub unsafe fn get_option_default(opt_idx: OptIndex, opt_flags: c_int) -> Object {
    unsafe {
        let opt = get_option(opt_idx);
        let is_global_local_option = option_is_global_local(opt_idx);

        #[cfg(unix)]
        if opt_idx == kOptModeline && getuid() == ROOT_UID {
            return boolean_obj(false);
        }

        if (opt_flags & OPT_LOCAL) != 0 && is_global_local_option {
            return unset_obj();
        }

        let mut def_val = (*opt).def_val;
        if option_has_type(opt_idx, ObjectType::kObjectTypeString)
            && ((*opt).flags & kOptFlagNoDefExp) == 0
        {
            let s = option_expand(opt_idx, def_val.data.string().data);
            if !s.is_null() {
                def_val = cstr_as_obj(s);
            }
        }
        if optval_is_owned(opt_idx) {
            copy_object(def_val, ptr::null_mut())
        } else {
            def_val
        }
    }
}

/// Allocate the default values for all options by copying them from the stack.
unsafe fn alloc_options_default() {
    unsafe {
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            (*opt).def_val = copy_object((*opt).def_val, ptr::null_mut());
        }
    }
}

/// Change the default value for an option.
unsafe fn change_option_default(opt_idx: OptIndex, value: Object) {
    unsafe {
        let opt = get_option(opt_idx);
        optval_free((*opt).def_val);
        (*opt).def_val = value;
    }
}

/// Set an option to its default value.
unsafe fn set_option_default(opt_idx: OptIndex, opt_flags: c_int) {
    unsafe {
        let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
        let def_val = get_option_default(opt_idx, opt_flags);
        set_option_direct(opt_idx, def_val, opt_flags, current_sctx.sc_sid);
        optval_free_read(opt_idx, def_val);

        if opt_idx == kOptScroll {
            win_comp_scroll(curwin);
        }

        let mut flagsp = insecure_flag(curwin, opt_idx, opt_flags);
        *flagsp &= !kOptFlagInsecure;
        if both {
            flagsp = insecure_flag(curwin, opt_idx, OPT_LOCAL);
            *flagsp &= !kOptFlagInsecure;
        }
    }
}

/// Set all options (except terminal options) to their default value.
unsafe fn set_options_default(opt_flags: c_int) {
    unsafe {
        for opt_idx in 0..kOptCount {
            if ((*get_option(opt_idx)).flags & kOptFlagNoDefault) == 0 {
                set_option_default(opt_idx, opt_flags);
            }
        }

        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                win_comp_scroll(wp);
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }

        parse_cino(curbuf);
    }
}

/// Set the Vi-default value of a string option.
unsafe fn set_string_default(opt_idx: OptIndex, val: *const c_char) {
    unsafe {
        assert!(opt_idx != kOptInvalid);
        change_option_default(opt_idx, cstr_to_obj(val));
    }
}

/// For an option value that contains comma separated items, find "newval" in "origval".
unsafe fn find_dup_item(
    origval: *const c_char,
    newval: *const c_char,
    newvallen: usize,
    flags: u32,
) -> *const c_char {
    unsafe {
        if origval.is_null() {
            return ptr::null();
        }

        let mut bs: c_int = 0;
        let mut s = origval;
        while *s != NUL {
            if ((flags & kOptFlagComma) == 0
                || s == origval
                || (*s.offset(-1) == b',' as c_char && (bs & 1) == 0))
                && strncmp(s, newval, newvallen) == 0
                && ((flags & kOptFlagComma) == 0
                    || *s.add(newvallen) == b',' as c_char
                    || *s.add(newvallen) == NUL)
            {
                return s;
            }
            if (s > origval.add(1)
                && *s.offset(-1) == b'\\' as c_char
                && *s.offset(-2) != b',' as c_char)
                || (s == origval.add(1) && *s.offset(-1) == b'\\' as c_char)
            {
                bs += 1;
            } else {
                bs = 0;
            }
            s = s.add(1);
        }
        ptr::null()
    }
}

/// Free all options.
pub unsafe fn free_all_options() {
    unsafe {
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            let hidden = is_option_hidden(opt_idx);

            if !hidden
                && !(*opt).var.is_null()
                && (option_is_global_only(opt_idx) || !option_is_window_local(opt_idx))
            {
                if is_callback_option(opt_idx) {
                    callback_free((*opt).var.cast::<Callback>());
                } else if option_has_type(opt_idx, ObjectType::kObjectTypeString) {
                    crate::optionstr::clear_string_option((*opt).var.cast::<*mut c_char>());
                }
            }
            optval_free((*opt).def_val);
        }
        free_operatorfunc_option();
        free_tagfunc_option();
        free_findfunc_option();
        fenc_default = None;
        P_TERM = None;
        P_TTYTYPE = None;
    }
}

/// Initialize the options, part two: After getting Rows and Columns.
pub unsafe fn set_init_2(_headless: bool) {
    unsafe {
        logmsg!(
            1,
            ptr::null(),
            c"set_init_2".as_ptr(),
            592,
            true,
            c"startup runtimepath/packpath value: %s".as_ptr(),
            p_rtp,
        );

        if ((*get_option(kOptScroll)).flags & kOptFlagWasSet) == 0 {
            set_option_default(kOptScroll, OPT_LOCAL);
        }
        comp_col();

        if !option_was_set(kOptWindow) {
            p_window = OptInt::from(Rows - 1);
        }
        change_option_default(kOptWindow, integer_obj(OptInt::from(Rows - 1)));
    }
}

unsafe fn change_option_and_default_if_unset(idx: OptIndex, val: Option<&'static CStr>) {
    unsafe {
        let Some(val) = val else {
            return;
        };
        if ((*get_option(idx)).flags & kOptFlagWasSet) != 0 {
            return;
        }
        let optval = cstr_as_obj(val.as_ptr());
        set_option_direct(idx, optval, 0, SID_NONE);
        change_option_default(idx, copy_object(optval, ptr::null_mut()));
    }
}

/// Initialize the options, part three: After reading the .vimrc
pub unsafe fn set_init_3() {
    unsafe {
        parse_shape_opt(SHAPE_CURSOR);

        let mut len: usize = 0;
        let mut name = [0 as c_char; MAXPATHL];
        let p = invocation_path_tail(p_sh, &raw mut len);
        xmemcpyz(name.as_mut_ptr().cast(), p.cast(), len);
        for (i, rule) in SHELL_RULES.iter().enumerate() {
            if c_strstr(name.as_ptr(), rule.pat.as_ptr()).is_null() {
                continue;
            }
            change_option_and_default_if_unset(kOptShellcmdflag, rule.shcf);
            change_option_and_default_if_unset(kOptShellpipe, rule.sp);
            change_option_and_default_if_unset(kOptShellredir, rule.srr);
            change_option_and_default_if_unset(kOptShellxquote, rule.sxq);
            #[cfg(windows)]
            if i > 0 && ((*get_option(kOptShellslash)).flags & kOptFlagWasSet) == 0 {
                set_option_direct(kOptShellslash, boolean_obj(true), 0, SID_NONE);
                change_option_default(kOptShellslash, boolean_obj(true));
            }
            let _ = i;
            break;
        }

        if buf_is_empty(curbuf) && ((*get_option(kOptFileformats)).flags & kOptFlagWasSet) != 0 {
            set_fileformat(default_fileformat(), OPT_LOCAL);
        }

        set_title_defaults();
    }
}

/// When 'helplang' is still at its default value, set it to "lang".
pub unsafe fn set_helplang_default(lang: *const c_char) {
    unsafe {
        if lang.is_null() {
            return;
        }

        let lang_len = strlen(lang);
        if lang_len < 2 {
            return;
        }
        if ((*get_option(kOptHelplang)).flags & kOptFlagWasSet) != 0 {
            return;
        }

        free_string_option(p_hlg);
        let mut hlg_bytes = [
            *lang as u8,
            *lang.add(1) as u8,
        ];
        if lang_len >= 5
            && hlg_bytes[0].eq_ignore_ascii_case(&b'z')
            && hlg_bytes[1].eq_ignore_ascii_case(&b'h')
            && *lang.add(2) == b'_' as c_char
        {
            hlg_bytes[0] = (*lang.add(3) as u8).to_ascii_lowercase();
            hlg_bytes[1] = (*lang.add(4) as u8).to_ascii_lowercase();
        } else if lang_len > 0 && *lang == b'C' as c_char {
            hlg_bytes[0] = b'e';
            hlg_bytes[1] = b'n';
        }
        p_hlg = crate::optionstr::alloc_string_option_bytes(&hlg_bytes);
    }
}

pub unsafe fn set_title_defaults() {
    unsafe {
        if ((*get_option(kOptTitle)).flags & kOptFlagWasSet) == 0 {
            change_option_default(kOptTitle, boolean_obj(false));
            p_title = 0;
        }
        if ((*get_option(kOptIcon)).flags & kOptFlagWasSet) == 0 {
            change_option_default(kOptIcon, boolean_obj(false));
            p_icon = 0;
        }
    }
}

pub unsafe fn ex_set(eap: *mut exarg_T) {
    unsafe {
        let mut flags: c_int = 0;

        if (*eap).cmdidx == cmdidx_T::CMD_setlocal as c_int {
            flags = OPT_LOCAL;
        } else if (*eap).cmdidx == cmdidx_T::CMD_setglobal as c_int {
            flags = OPT_GLOBAL;
        }
        if (*eap).forceit != 0 {
            flags |= OPT_ONECOLUMN;
        }
        do_set((*eap).arg, flags);
    }
}

/// Copy the new string value into allocated memory for the option.
unsafe fn stropt_copy_value(
    origval: *const c_char,
    argp: *mut *mut c_char,
    op: set_op_T,
    flags: u32,
) -> Vec<c_char> {
    unsafe {
        let mut arg = *argp;

        let mut newlen = strlen(arg) + 1;
        if op != OP_NONE {
            newlen += strlen(origval) + 1;
        }
        let mut newval_vec: Vec<c_char> = vec![0; newlen];
        let newval = newval_vec.as_mut_ptr();
        let mut s = newval;

        while *arg != NUL && !ascii_iswhite(*arg) {
            #[cfg(windows)]
            let remove_bs = *arg == b'\\' as c_char
                && *arg.add(1) != NUL
                && !((flags & kOptFlagExpand) != 0
                    && vim_isfilec(*arg.add(1) as u8 as c_int)
                    && !ascii_iswhite(*arg.add(1))
                    && (*arg.add(1) != b'\\' as c_char
                        || (s == newval && *arg.add(2) != b'\\' as c_char)));
            #[cfg(not(windows))]
            let remove_bs = {
                let _ = flags;
                *arg == b'\\' as c_char && *arg.add(1) != NUL
            };
            if remove_bs {
                arg = arg.add(1);
            }
            let i = utfc_ptr2len(arg);
            if i > 1 {
                ptr::copy(arg, s, i as usize);
                arg = arg.add(i as usize);
                s = s.add(i as usize);
            } else {
                *s = *arg;
                s = s.add(1);
                arg = arg.add(1);
            }
        }
        *s = NUL;

        *argp = arg;
        newval_vec
    }
}

/// Expand environment variables and ~ in string option value 'newval'.
unsafe fn stropt_expand_envvar(
    opt_idx: OptIndex,
    origval: *const c_char,
    newval: &mut Vec<c_char>,
    op: set_op_T,
) {
    unsafe {
        let s = option_expand(opt_idx, newval.as_mut_ptr());
        if s.is_null() {
            return;
        }

        let mut newlen = (strlen(s) as u32) + 1;
        if op != OP_NONE {
            newlen += (strlen(origval) as u32) + 1;
        }
        *newval = vec![0; newlen as usize];
        strcpy(newval.as_mut_ptr(), s);
    }
}

/// Concatenate the original and new values of a string option, adding a "," if needed.
unsafe fn stropt_concat_with_comma(
    origval: *const c_char,
    newval: *mut c_char,
    op: set_op_T,
    flags: u32,
) {
    unsafe {
        let mut len: usize;
        let comma = usize::from((flags & kOptFlagComma) != 0 && *origval != NUL && *newval != NUL);
        if op == OP_ADDING {
            len = strlen(origval);
            if comma != 0
                && len > 1
                && (flags & kOptFlagOneComma) == kOptFlagOneComma
                && *origval.add(len - 1) == b',' as c_char
                && *origval.add(len - 2) != b'\\' as c_char
            {
                len -= 1;
            }
            ptr::copy(newval, newval.add(len + comma), strlen(newval) + 1);
            ptr::copy(origval, newval, len);
        } else {
            len = strlen(newval);
            ptr::copy(origval, newval.add(len + comma), strlen(origval) + 1);
        }
        if comma != 0 {
            *newval.add(len) = b',' as c_char;
        }
    }
}

/// Remove a value from a string option.
unsafe fn stropt_remove_val(
    origval: *const c_char,
    newval: *mut c_char,
    flags: u32,
    mut strval: *const c_char,
    mut len: c_int,
) {
    unsafe {
        strcpy(newval, origval);
        if *strval != NUL {
            if (flags & kOptFlagComma) != 0 {
                if strval == origval {
                    if *strval.add(len as usize) == b',' as c_char {
                        len += 1;
                    }
                } else {
                    strval = strval.offset(-1);
                    len += 1;
                }
            }
            let src = strval.add(len as usize);
            let dst_off = strval.offset_from(origval) as usize;
            ptr::copy(src, newval.add(dst_off), strlen(src) + 1);
        }
    }
}

/// Find a comma-separated item in "src" that matches the key part of "key".
unsafe fn find_key_item(
    src: *mut c_char,
    key: *mut c_char,
    keylen: isize,
    itemlenp: *mut isize,
) -> *mut c_char {
    unsafe {
        let mut p = src;

        while *p != NUL {
            if (p == src || *p.offset(-1) == b',' as c_char)
                && strncmp(p, key, keylen as usize) == 0
            {
                let mut end = vim_strchr(p, b',' as c_int);
                if end.is_null() {
                    end = p.add(strlen(p));
                }
                *itemlenp = end.offset_from(p);
                return p;
            }
            p = p.add(1);
        }
        ptr::null_mut()
    }
}

/// Remove one item of length "itemlen" at position "item" from comma-separated string "str" in-place.
unsafe fn remove_comma_item(str_arg: *const c_char, item: *mut c_char, itemlen: isize) {
    unsafe {
        if *item.offset(itemlen) == b',' as c_char {
            let src = item.offset(itemlen + 1);
            ptr::copy(src, item, strlen(src) + 1);
        } else if (item as *const c_char) > str_arg && *item.offset(-1) == b',' as c_char {
            let src = item.offset(itemlen);
            ptr::copy(src, item.offset(-1), strlen(src) + 1);
        } else {
            *item = NUL;
        }
    }
}

/// Remove all items matching "key" (with ':') from comma-separated string "str" in-place.
unsafe fn remove_key_item(
    str_arg: *mut c_char,
    key: *mut c_char,
    keylen: isize,
    skip: *const c_char,
) {
    unsafe {
        let mut itemlen: isize = 0;
        loop {
            let mut found = find_key_item(str_arg, key, keylen, &raw mut itemlen);
            if found.is_null() {
                break;
            }
            if ptr::eq(found, skip) {
                let mut next = found.offset(itemlen);
                if *next == b',' as c_char {
                    next = next.add(1);
                }
                found = find_key_item(next, key, keylen, &raw mut itemlen);
                if found.is_null() {
                    break;
                }
            }

            remove_comma_item(str_arg, found, itemlen);
        }
    }
}

/// Append a comma-separated item to the end of "str" in-place.
unsafe fn append_item(str_arg: *mut c_char, item: *mut c_char, item_len: isize) {
    unsafe {
        let mut len = strlen(str_arg) as isize;
        if len > 0 {
            *str_arg.offset(len) = b',' as c_char;
            len += 1;
        }
        ptr::copy(item, str_arg.offset(len), item_len as usize);
        *str_arg.offset(len + item_len) = NUL;
    }
}

/// Prepend a comma-separated item to the beginning of "str" in-place.
unsafe fn prepend_item(str_arg: *mut c_char, item: *mut c_char, item_len: isize) {
    unsafe {
        let len = strlen(str_arg) as isize;
        let comma: isize = if len > 0 { 1 } else { 0 };

        ptr::copy(
            str_arg,
            str_arg.offset(item_len + comma),
            (len + 1) as usize,
        );
        ptr::copy(item, str_arg, item_len as usize);
        if comma != 0 {
            *str_arg.offset(item_len) = b',' as c_char;
        }
    }
}

/// For a P_COMMA option: process "key:value" items in "newval" individually.
unsafe fn stropt_handle_keymatch(
    origval: *const c_char,
    newval: *mut c_char,
    op: set_op_T,
    _flags: u32,
) -> bool {
    unsafe {
        if vim_strchr(newval, b':' as c_int).is_null()
            && vim_strchr(newval, b',' as c_int).is_null()
        {
            return false;
        }

        let newval_copy = crate::memory::cstr_to_cstring(newval);
        strcpy(newval, origval);

        let mut item_start = newval_copy.as_ptr().cast_mut();
        loop {
            let p = vim_strchr(item_start, b',' as c_int);
            let item_len = if p.is_null() {
                strlen(item_start) as isize
            } else {
                p.offset_from(item_start)
            };

            if item_len > 0 {
                let colon = vim_strchr(item_start, b':' as c_int);
                if !colon.is_null() && colon < item_start.offset(item_len) {
                    let keylen = colon.offset_from(item_start) + 1;

                    if op == OP_ADDING || op == OP_PREPENDING {
                        let mut old_itemlen: isize = 0;
                        let found = find_key_item(newval, item_start, keylen, &raw mut old_itemlen);
                        if !found.is_null() {
                            if old_itemlen == item_len
                                && strncmp(found, item_start, item_len as usize) == 0
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
                    let found = find_dup_item(newval, item_start, item_len as usize, kOptFlagComma);
                    if found.is_null() {
                        if op == OP_PREPENDING {
                            prepend_item(newval, item_start, item_len);
                        } else {
                            append_item(newval, item_start, item_len);
                        }
                    }
                } else if op == OP_REMOVING {
                    let found = find_dup_item(newval, item_start, item_len as usize, kOptFlagComma)
                        .cast_mut();
                    if !found.is_null() {
                        remove_comma_item(newval, found, item_len);
                    }
                }
            }

            if p.is_null() {
                break;
            }
            item_start = p.add(1);
        }

        true
    }
}

/// Remove flags that appear twice in the string option value 'newval'.
unsafe fn stropt_remove_dupflags(newval: *mut c_char, flags: u32) {
    unsafe {
        let mut s = newval;
        while *s != NUL {
            if (flags & kOptFlagOneComma) != 0 {
                if *s != b',' as c_char
                    && *s.add(1) == b',' as c_char
                    && !vim_strchr(s.add(2), *s as u8 as c_int).is_null()
                {
                    let src = s.add(2);
                    ptr::copy(src, s, strlen(src) + 1);
                    continue;
                }
            } else if ((flags & kOptFlagComma) == 0 || *s != b',' as c_char)
                && !vim_strchr(s.add(1), *s as u8 as c_int).is_null()
            {
                let src = s.add(1);
                ptr::copy(src, s, strlen(src) + 1);
                continue;
            }
            s = s.add(1);
        }
    }
}

/// Get the string value specified for a ":set" command.
unsafe fn stropt_get_newval(
    opt_idx: OptIndex,
    argp: *mut *mut c_char,
    varp: *mut c_void,
    origval: *const c_char,
    op_arg: *mut set_op_T,
) -> std::ffi::CString {
    unsafe {
        let mut arg = *argp;
        let mut op = *op_arg;
        let mut save_arg: *mut c_char = ptr::null_mut();
        let mut s: *const c_char = ptr::null();
        let flags = (*get_option(opt_idx)).flags;

        arg = arg.add(1);

        if varp == (&raw mut p_kp).cast::<c_void>() && (*arg == NUL || *arg == b' ' as c_char) {
            save_arg = arg;
            arg = c":help".as_ptr().cast_mut();
        }

        let mut newval_vec = stropt_copy_value(origval, &raw mut arg, op, flags);

        if op == OP_NONE || (flags & kOptFlagComma) != 0 {
            stropt_expand_envvar(opt_idx, origval, &mut newval_vec, op);
        }
        let newval = newval_vec.as_mut_ptr();

        if (flags & kOptFlagComma) != 0
            && (flags & kOptFlagColon) != 0
            && op != OP_NONE
            && stropt_handle_keymatch(origval, newval, op, flags)
        {
            // fully handled
        } else {
            let mut len: c_int = 0;
            if op == OP_REMOVING || (flags & kOptFlagNoDup) != 0 {
                len = strlen(newval) as c_int;
                s = find_dup_item(origval, newval, len as usize, flags);

                if (op == OP_ADDING || op == OP_PREPENDING) && !s.is_null() {
                    op = OP_NONE;
                    strcpy(newval, origval);
                }

                if s.is_null() {
                    s = origval.add(strlen(origval));
                }
            }

            if op == OP_ADDING || op == OP_PREPENDING {
                stropt_concat_with_comma(origval, newval, op, flags);
            } else if op == OP_REMOVING {
                stropt_remove_val(origval, newval, flags, s, len);
            }
        }

        if (flags & kOptFlagFlagList) != 0 {
            stropt_remove_dupflags(newval, flags);
        }

        if !save_arg.is_null() {
            arg = save_arg;
        }
        *argp = arg;
        *op_arg = op;

        CStr::from_ptr(newval).to_owned()
    }
}

unsafe fn get_op(arg: *const c_char) -> set_op_T {
    unsafe {
        let mut op = OP_NONE;
        if *arg != NUL && *arg.add(1) == b'=' as c_char {
            if *arg == b'+' as c_char {
                op = OP_ADDING;
            } else if *arg == b'^' as c_char {
                op = OP_PREPENDING;
            } else if *arg == b'-' as c_char {
                op = OP_REMOVING;
            }
        }
        op
    }
}

unsafe fn get_option_prefix(argp: *mut *mut c_char) -> set_prefix_T {
    unsafe {
        if strncmp(*argp, c"no".as_ptr(), 2) == 0 {
            *argp = (*argp).add(2);
            PREFIX_NO
        } else if strncmp(*argp, c"inv".as_ptr(), 3) == 0 {
            *argp = (*argp).add(3);
            PREFIX_INV
        } else {
            PREFIX_NONE
        }
    }
}

unsafe fn validate_opt_idx(
    win: *mut win_T,
    opt_idx: OptIndex,
    opt_flags: c_int,
    flags: u32,
    prefix: set_prefix_T,
    errmsg: *mut *const c_char,
) -> c_int {
    unsafe {
        if !option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) && prefix != PREFIX_NONE {
            *errmsg = e_invarg.as_ptr();
            return FAIL;
        }

        if (opt_flags & OPT_WINONLY) != 0 && !option_is_window_local(opt_idx) {
            return FAIL;
        }

        if (opt_flags & OPT_NOWIN) != 0 && option_is_window_local(opt_idx) {
            return FAIL;
        }

        if (opt_flags & OPT_MODELINE) != 0 {
            if (flags & kOptFlagSecure) != 0 {
                *errmsg = E_NOT_ALLOWED_IN_MODELINE;
                return FAIL;
            }
            if (flags & kOptFlagMLE) != 0 && p_mle == 0 {
                *errmsg = E_NOT_ALLOWED_IN_MODELINE_WHEN_MODELINEEXPR_IS_OFF;
                return FAIL;
            }
            if (*win).w_onebuf_opt.wo_diff != 0
                && (opt_idx == kOptFoldmethod || opt_idx == kOptWrap)
            {
                return FAIL;
            }
        }

        if sandbox != 0 && (flags & kOptFlagSecure) != 0 {
            *errmsg = e_sandbox.as_ptr();
            return FAIL;
        }

        OK
    }
}

/// Skip over the name of a TTY option or keycode option.
unsafe fn find_tty_option_end(arg: *const c_char) -> *const c_char {
    unsafe {
        if strequal(arg, c"term".as_ptr()) {
            return arg.add("term".len());
        } else if strequal(arg, c"ttytype".as_ptr()) {
            return arg.add("ttytype".len());
        }

        let mut p = arg;
        let mut delimit = false;

        if *arg == b'<' as c_char {
            delimit = true;
            p = p.add(1);
        }
        if *p == b't' as c_char
            && *p.add(1) == b'_' as c_char
            && *p.add(2) != NUL
            && *p.add(3) != NUL
        {
            p = p.add(4);
        } else if delimit {
            while *p != NUL && *p != b'>' as c_char {
                p = p.add(1);
            }
        }
        if delimit {
            if *p != b'>' as c_char {
                return ptr::null();
            }
            p = p.add(1);
        }

        if arg == p { ptr::null() } else { p }
    }
}

/// Skip over the name of an option.
pub unsafe fn find_option_end(arg: *const c_char, opt_idxp: *mut OptIndex) -> *const c_char {
    unsafe {
        let mut p = find_tty_option_end(arg);
        if !p.is_null() {
            *opt_idxp = kOptInvalid;
            return p;
        }
        p = arg;

        if !ascii_isalpha(*p) {
            *opt_idxp = kOptInvalid;
            return ptr::null();
        }
        while ascii_isalpha(*p) {
            p = p.add(1);
        }

        *opt_idxp = find_option_len(arg, p.offset_from(arg) as usize);
        p
    }
}

/// Get new option value from argp. Allocated Object must be freed by caller.
pub unsafe fn get_option_newval(
    opt_idx: OptIndex,
    opt_flags: c_int,
    prefix: set_prefix_T,
    argp: *mut *mut c_char,
    nextchar: c_int,
    mut op: set_op_T,
    varp: *mut c_void,
    oldval_override: *mut Object,
    errmsg: *mut *const c_char,
) -> Object {
    unsafe {
        assert!(!varp.is_null());

        let opt = get_option(opt_idx);
        let mut arg = *argp;
        let mut newval = nil_obj();

        if nextchar == b'&' as c_int {
            return optval_own(opt_idx, get_option_default(opt_idx, OPT_GLOBAL));
        } else if nextchar == b'<' as c_int {
            if option_is_global_local(opt_idx) && (opt_flags & OPT_LOCAL) == 0 {
                unset_option_local_value(opt_idx);
            }
            return get_option_value(opt_idx, OPT_GLOBAL);
        }

        let oldval = if !oldval_override.is_null() {
            *oldval_override
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
            ObjectType::kObjectTypeNil => std::process::abort(),
            ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeBoolean => {
                let newval_bool: TriState = if nextchar == b'!' as c_int {
                    if oldval.obj_type == ObjectType::kObjectTypeUnset {
                        kNone
                    } else if oldval.data.boolean() {
                        kFalse
                    } else {
                        kTrue
                    }
                } else if prefix == PREFIX_INV {
                    *(varp.cast::<c_int>()) ^ 1
                } else if prefix == PREFIX_NO {
                    0
                } else {
                    1
                };

                newval = opt_from_tristate(newval_bool);
            }
            ObjectType::kObjectTypeInteger => {
                let oldval_num = oldval.data.integer();
                let mut newval_num: OptInt = 0;

                arg = arg.add(1);
                if (varp.cast::<OptInt>() == &raw mut p_wc
                    || varp.cast::<OptInt>() == &raw mut p_wcm)
                    && (*arg == b'<' as c_char
                        || *arg == b'^' as c_char
                        || (*arg != NUL
                            && (*arg.add(1) == NUL || ascii_iswhite(*arg.add(1)))
                            && !ascii_isdigit(*arg)))
                {
                    newval_num = OptInt::from(string_to_key(arg));
                    if newval_num == 0 {
                        *errmsg = e_invarg.as_ptr();
                        if oldval_override.is_null() {
                            optval_free(oldval);
                        }
                        return newval;
                    }
                } else if *arg == b'-' as c_char || ascii_isdigit(*arg) {
                    let mut i: c_int = 0;
                    vim_str2nr(
                        arg,
                        ptr::null_mut(),
                        &raw mut i,
                        STR2NR_ALL,
                        &raw mut newval_num,
                        ptr::null_mut(),
                        0,
                        true,
                        ptr::null_mut(),
                    );
                    if i == 0
                        || (*arg.add(i as usize) != NUL && !ascii_iswhite(*arg.add(i as usize)))
                    {
                        *errmsg = E_NUMBER_REQUIRED_AFTER_EQUAL;
                        if oldval_override.is_null() {
                            optval_free(oldval);
                        }
                        return newval;
                    }
                } else {
                    *errmsg = E_NUMBER_REQUIRED_AFTER_EQUAL;
                    if oldval_override.is_null() {
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
            ObjectType::kObjectTypeString => {
                let oldval_str = oldval.data.string().data;
                let newval_str = stropt_get_newval(opt_idx, argp, varp, oldval_str, &raw mut op);
                newval = cstr_to_obj(newval_str.as_ptr());
            }
            ObjectType::kObjectTypeLuaRef => {
                let newval_str = stropt_get_newval(opt_idx, argp, varp, c"".as_ptr(), &raw mut op);
                newval = cstr_to_obj(newval_str.as_ptr());
            }
            _ => std::process::abort(),
        }

        if oldval_override.is_null() {
            optval_free(oldval);
        }
        newval
    }
}

unsafe fn do_one_set_option(
    opt_flags: c_int,
    argp: *mut *mut c_char,
    did_show: *mut bool,
    errbuf: *const CharBuf,
) -> *const c_char {
    unsafe {
        let prefix = get_option_prefix(argp);
        let arg = *argp;

        let mut opt_idx: OptIndex = kOptInvalid;
        let option_end = find_option_end(arg, &raw mut opt_idx);

        if opt_idx != kOptInvalid {
            assert!(option_end >= arg as *const c_char);
        } else if is_tty_option(arg) {
            return ptr::null();
        } else {
            return E_UNKNOWN_OPTION;
        }

        let afterchar = *option_end;
        let mut p = option_end.cast_mut();

        while ascii_iswhite(*p) {
            p = p.add(1);
        }

        let op = get_op(p);
        if op != OP_NONE {
            p = p.add(1);
        }

        let nextchar = *p as u8 as c_int;
        let opt = get_option(opt_idx);
        let flags = (*opt).flags;
        let varp = get_varp_scope(opt, opt_flags);

        let mut errmsg: *const c_char = ptr::null();
        if validate_opt_idx(curwin, opt_idx, opt_flags, flags, prefix, &raw mut errmsg) == FAIL {
            return errmsg;
        }

        if !vim_strchr(c"?=:!&<".as_ptr(), nextchar).is_null() {
            *argp = p;

            if nextchar == b'&' as c_int
                && *(*argp).add(1) == b'v' as c_char
                && *(*argp).add(2) == b'i' as c_char
            {
                if *(*argp).add(3) == b'm' as c_char {
                    *argp = (*argp).add(3);
                } else {
                    *argp = (*argp).add(2);
                }
            }
            if !vim_strchr(c"?!&<".as_ptr(), nextchar).is_null()
                && *(*argp).add(1) != NUL
                && !ascii_iswhite(*(*argp).add(1))
            {
                return e_trailing.as_ptr();
            }
        }

        if nextchar == b'?' as c_int
            || (prefix == PREFIX_NONE
                && vim_strchr(c"=:&<".as_ptr(), nextchar).is_null()
                && !option_has_type(opt_idx, ObjectType::kObjectTypeBoolean))
        {
            if *did_show {
                msg_putchar(b'\n' as c_int);
            } else {
                msg_ext_set_kind(c"list_cmd".as_ptr());
                gotocmdline(true);
                *did_show = true;
            }
            showoneopt(opt, opt_flags);

            if p_verbose > 0 {
                if varp == (*opt).var {
                    last_set_msg((*opt).script_ctx);
                } else if option_has_scope(opt_idx, kOptScopeWin) {
                    last_set_msg(
                        (*curwin).w_onebuf_opt.wo_script_ctx
                            [option_scope_idx(opt_idx, kOptScopeWin) as usize],
                    );
                } else if option_has_scope(opt_idx, kOptScopeBuf) {
                    last_set_msg(
                        (*curbuf).b_p_script_ctx[option_scope_idx(opt_idx, kOptScopeBuf) as usize],
                    );
                }
            }

            if nextchar != b'?' as c_int && nextchar != NUL as c_int && !ascii_iswhite(afterchar) {
                return e_trailing.as_ptr();
            }
            return ptr::null();
        }

        if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
            if !vim_strchr(c"=:".as_ptr(), nextchar).is_null() {
                return e_invarg.as_ptr();
            }

            if vim_strchr(c"!&<".as_ptr(), nextchar).is_null()
                && nextchar != NUL as c_int
                && !ascii_iswhite(afterchar)
            {
                return e_trailing.as_ptr();
            }
        } else if vim_strchr(c"=:&<".as_ptr(), nextchar).is_null() {
            return e_invarg.as_ptr();
        }

        let newval = get_option_newval(
            opt_idx,
            opt_flags,
            prefix,
            argp,
            nextchar,
            op,
            varp,
            ptr::null_mut(),
            &raw mut errmsg,
        );

        if newval.obj_type == ObjectType::kObjectTypeNil || !errmsg.is_null() {
            return errmsg;
        }

        set_option(opt_idx, newval, opt_flags, 0, false, op == OP_NONE, errbuf)
    }
}

/// Parse 'arg' for option settings.
pub unsafe fn do_set(mut arg: *mut c_char, opt_flags: c_int) -> c_int {
    unsafe {
        let mut did_show = false;

        if *arg == NUL {
            showoptions(false, opt_flags);
            did_show = true;
        } else {
            while *arg != NUL {
                if strncmp(arg, c"all".as_ptr(), 3) == 0
                    && !ascii_isalpha(*arg.add(3))
                    && (opt_flags & OPT_MODELINE) == 0
                {
                    arg = arg.add(3);
                    if *arg == b'&' as c_char {
                        arg = arg.add(1);
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
                    let errmsg = do_one_set_option(
                        opt_flags,
                        &raw mut arg,
                        &raw mut did_show,
                        &raw const errbuf,
                    );

                    for _ in 0..2 {
                        arg = skiptowhite_esc(arg);
                        arg = skipwhite(arg);
                        if *arg != b'=' as c_char {
                            break;
                        }
                    }

                    if !errmsg.is_null() {
                        let iobuf_ptr = (&raw mut IObuff).cast::<c_char>();
                        let i = (vim_snprintf!(iobuf_ptr, IOSIZE, c"%s".as_ptr(), tr(errmsg)) + 2)
                            as isize;
                        let arg_len = arg.offset_from(startarg);
                        if (i + arg_len) < IOSIZE as isize {
                            xstrlcpy(
                                iobuf_ptr.offset(i - 2),
                                c": ".as_ptr(),
                                (IOSIZE as isize - i + 2) as usize,
                            );
                            assert!(arg >= startarg);
                            ptr::copy(startarg, iobuf_ptr.offset(i), arg_len as usize);
                            *iobuf_ptr.offset(i + arg_len) = NUL;
                        }
                        trans_characters(iobuf_ptr, IOSIZE as c_int);

                        no_wait_return += 1;
                        emsg(iobuf_ptr);
                        no_wait_return -= 1;

                        return FAIL;
                    }
                }

                arg = skipwhite(arg);
            }
        }

        if silent_mode && did_show {
            silent_mode = false;
            info_message = true;
            msg_putchar(b'\n' as c_int);
            silent_mode = true;
            info_message = false;
        }

        OK
    }
}

unsafe fn find_key_len(arg_arg: *const c_char, len: usize, has_lt: bool) -> c_int {
    unsafe {
        let mut key: c_int = 0;
        let mut arg = arg_arg;

        if len >= 4 && *arg == b't' as c_char && *arg.add(1) == b'_' as c_char {
            if !has_lt || *arg.add(4) == b'>' as c_char {
                key = termcap2key(*arg.add(2) as u8 as c_int, *arg.add(3) as u8 as c_int);
            }
        } else if has_lt {
            arg = arg.offset(-1);
            let mut modifiers: c_int = 0;
            key = find_special_key(
                &raw mut arg,
                len + 1,
                &raw mut modifiers,
                FSK_KEYCODE | FSK_KEEP_X_KEY | FSK_SIMPLIFY,
                ptr::null_mut(),
            );
            if modifiers != 0 {
                key = 0;
            }
        }
        key
    }
}

/// Convert a key name or string into a key value.
pub unsafe fn string_to_key(arg: *mut c_char) -> c_int {
    unsafe {
        if *arg == b'<' as c_char && *arg.add(1) != NUL {
            return find_key_len(arg.add(1), strlen(arg), true);
        }
        if *arg == b'^' as c_char && *arg.add(1) != NUL {
            let mut key = ctrl_chr(c_int::from(*arg.add(1) as u8));
            if key == 0 {
                key = K_ZERO;
            }
            return key;
        }
        *arg as u8 as c_int
    }
}

pub unsafe fn did_set_title() {
    unsafe {
        if starting != NO_SCREEN {
            maketitle();
        }
    }
}

/// set_options_bin - called when 'bin' changes value.
pub unsafe fn set_options_bin(oldval: c_int, newval: c_int, opt_flags: c_int) {
    unsafe {
        if newval != 0 {
            if oldval == 0 {
                if (opt_flags & OPT_GLOBAL) == 0 {
                    (*curbuf).b_p_tw_nobin = (*curbuf).b_p_tw;
                    (*curbuf).b_p_wm_nobin = (*curbuf).b_p_wm;
                    (*curbuf).b_p_ml_nobin = (*curbuf).b_p_ml;
                    (*curbuf).b_p_et_nobin = (*curbuf).b_p_et;
                }
                if (opt_flags & OPT_LOCAL) == 0 {
                    P_TW_NOBIN = p_tw;
                    P_WM_NOBIN = p_wm;
                    P_ML_NOBIN = p_ml;
                    P_ET_NOBIN = p_et;
                }
            }

            if (opt_flags & OPT_GLOBAL) == 0 {
                (*curbuf).b_p_tw = 0;
                (*curbuf).b_p_wm = 0;
                (*curbuf).b_p_ml = 0;
                (*curbuf).b_p_et = 0;
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                p_tw = 0;
                p_wm = 0;
                p_ml = 0;
                p_et = 0;
                p_bin = 1;
            }
        } else if oldval != 0 {
            if (opt_flags & OPT_GLOBAL) == 0 {
                (*curbuf).b_p_tw = (*curbuf).b_p_tw_nobin;
                (*curbuf).b_p_wm = (*curbuf).b_p_wm_nobin;
                (*curbuf).b_p_ml = (*curbuf).b_p_ml_nobin;
                (*curbuf).b_p_et = (*curbuf).b_p_et_nobin;
            }
            if (opt_flags & OPT_LOCAL) == 0 {
                p_tw = P_TW_NOBIN;
                p_wm = P_WM_NOBIN;
                p_ml = P_ML_NOBIN;
                p_et = P_ET_NOBIN;
            }
        }

        didset_options_sctx(opt_flags, (&raw mut P_BIN_DEP_OPTS).cast::<c_int>());
    }
}

/// Expand environment variables for some string options.
unsafe fn option_expand(opt_idx: OptIndex, mut val: *const c_char) -> *mut c_char {
    unsafe {
        let opt = get_option(opt_idx);
        if ((*opt).flags & kOptFlagExpand) == 0 || is_option_hidden(opt_idx) {
            return ptr::null_mut();
        }

        if val.is_null() {
            val = *((*opt).var.cast::<*const c_char>());
        }

        if val.is_null() || strlen(val) > MAXPATHL {
            return ptr::null_mut();
        }

        let var = (*opt).var.cast::<*mut c_char>();
        let esc = var == &raw mut p_tags || var == &raw mut p_path;
        let namebuf_ptr = (&raw mut NameBuff).cast::<c_char>();
        expand_env_esc(
            val,
            namebuf_ptr,
            MAXPATHL as c_int,
            if esc {
                c" \t".as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
            false,
            if var == &raw mut p_sps {
                c"file:".as_ptr().cast_mut()
            } else {
                ptr::null_mut()
            },
        );
        if strcmp(namebuf_ptr, val) == 0 {
            return ptr::null_mut();
        }

        namebuf_ptr
    }
}

/// After setting various option values: recompute variables that depend on option values.
unsafe fn didset_options() {
    unsafe {
        buf_init_isk_chartab(curbuf);
        init_isf_chartab();
        init_isi_chartab();

        didset_string_options();

        let mut sps_flags: c_int = 0;
        let mut sps_limit: c_int = 0;
        if spell_check_sps(p_sps, &raw mut sps_flags, &raw mut sps_limit) == OK {
            spell_apply_sps(sps_flags, sps_limit);
        }
        let ws = (*curwin).w_s.cast::<synblock_T>();
        compile_cap_prog((*ws).b_p_spc, &mut (*ws).b_cap_prog);
        did_set_spell_option();
        did_set_cedit(ptr::null_mut());
        did_set_breakat(ptr::null_mut());
        didset_window_options(curwin, true);
    }
}

unsafe fn didset_options2() {
    unsafe {
        highlight_changed();

        set_chars_option(
            curwin,
            (*curwin).w_onebuf_opt.wo_fcs,
            CharsOption::kFillchars,
            true,
            ptr::null(),
        );
        set_chars_option(
            curwin,
            (*curwin).w_onebuf_opt.wo_lcs,
            CharsOption::kListchars,
            true,
            ptr::null(),
        );

        check_opt_wim(p_wim, (&raw mut wim_flags).cast::<u8>());
        tabstop_set(
            (*curbuf).b_p_vsts,
            &mut (*curbuf).b_p_vsts_array,
            ptr::null(),
        );
        tabstop_set((*curbuf).b_p_vts, &mut (*curbuf).b_p_vts_array, ptr::null());
    }
}

/// Repair UI state after `:set all&`.
unsafe fn didset_options_all() {
    unsafe {
        let errmsg = parse_shape_opt(SHAPE_CURSOR);
        assert!(errmsg.is_null());
        last_status(false);
        win_float_update_statusline();
        win_new_screen_rows();
    }
}

/// Check for string options that are NULL (normally only termcap options).
pub unsafe fn check_options() {
    unsafe {
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            if option_has_type(opt_idx, ObjectType::kObjectTypeString)
                && !is_callback_option(opt_idx)
                && !(*opt).immutable
                && !(*opt).var.is_null()
            {
                check_string_option(get_varp(opt).cast::<*mut c_char>());
            }
        }
    }
}

/// Check if option was set insecurely.
pub unsafe fn was_set_insecurely(wp: *mut win_T, opt_idx: OptIndex, opt_flags: c_int) -> c_int {
    unsafe {
        assert!(opt_idx != kOptInvalid);
        let flagp = insecure_flag(wp, opt_idx, opt_flags);
        c_int::from((*flagp & kOptFlagInsecure) != 0)
    }
}

/// Get a pointer to the flags used for the kOptFlagInsecure flag of option "opt_idx".
pub unsafe fn insecure_flag(wp: *mut win_T, opt_idx: OptIndex, opt_flags: c_int) -> *mut u32 {
    unsafe {
        if (opt_flags & OPT_LOCAL) != 0 {
            assert!(!wp.is_null());
            match opt_idx {
                kOptWrap => return &raw mut (*wp).w_onebuf_opt.wo_wrap_flags,
                kOptStatusline => return &raw mut (*wp).w_onebuf_opt.wo_stl_flags,
                kOptWinbar => return &raw mut (*wp).w_onebuf_opt.wo_wbr_flags,
                kOptFoldexpr => return &raw mut (*wp).w_onebuf_opt.wo_fde_flags,
                kOptFoldtext => return &raw mut (*wp).w_onebuf_opt.wo_fdt_flags,
                kOptIndentexpr => return &raw mut (*(*wp).w_buffer).b_p_inde_flags,
                kOptFormatexpr => return &raw mut (*(*wp).w_buffer).b_p_fex_flags,
                kOptIncludeexpr => return &raw mut (*(*wp).w_buffer).b_p_inex_flags,
                kOptComplete => return &raw mut (*(*wp).w_buffer).b_p_cpt_flags,
                _ => {}
            }
        } else {
            match opt_idx {
                kOptWrap => return &raw mut (*wp).w_allbuf_opt.wo_wrap_flags,
                kOptFoldexpr => return &raw mut (*wp).w_allbuf_opt.wo_fde_flags,
                kOptFoldtext => return &raw mut (*wp).w_allbuf_opt.wo_fdt_flags,
                _ => {}
            }
        }
        &raw mut (*get_option(opt_idx)).flags
    }
}

/// Redraw the window title and/or tab page text later.
pub unsafe fn redraw_titles() {
    unsafe {
        need_maketitle = true;
        redraw_tabline = true;
    }
}

/// Return true if "val" is a valid name.
pub unsafe fn valid_name(val: *const c_char, allowed: *const c_char) -> bool {
    unsafe {
        let mut s = val;
        while *s != NUL {
            if !ascii_isalnum(*s) && vim_strchr(allowed, *s as u8 as c_int).is_null() {
                return false;
            }
            s = s.add(1);
        }
        true
    }
}

pub unsafe fn check_blending(wp: *mut win_T) {
    unsafe {
        (*wp).w_grid_alloc.blending =
            (*wp).w_onebuf_opt.wo_winbl > 0 || ((*wp).w_floating && (*wp).w_config.shadow);
    }
}

/// Handle setting 'winhighlight' in window "wp".
pub unsafe fn parse_winhl_opt(winhl: *const c_char, wp: *mut win_T) -> bool {
    unsafe {
        let mut p: *const c_char = (&raw const empty_string_option).cast();
        if !winhl.is_null() {
            p = winhl;
        } else if !wp.is_null() {
            p = (*wp).w_onebuf_opt.wo_winhl;
        }

        if *p == NUL {
            if !wp.is_null() && (*wp).w_ns_hl_winhl > 0 && (*wp).w_ns_hl == (*wp).w_ns_hl_winhl {
                (*wp).w_ns_hl = 0;
                (*wp).w_hl_needs_update = 1;
            }
            return true;
        }

        let mut ns_hl: c_int = 0;
        if !wp.is_null() {
            if (*wp).w_ns_hl_winhl == 0 {
                (*wp).w_ns_hl_winhl = nvim_create_namespace(NvimString {
                    data: ptr::null_mut(),
                    size: 0,
                }) as c_int;
            } else {
                let dp = get_decor_provider((*wp).w_ns_hl_winhl, true);
                (*dp).hl_valid += 1;
            }
            ns_hl = (*wp).w_ns_hl_winhl;
            if (*wp).w_ns_hl <= 0 {
                (*wp).w_ns_hl = (*wp).w_ns_hl_winhl;
            }
        }

        let mut key: *const c_char = ptr::null();
        let mut val: *const c_char = ptr::null();
        let mut keylen: usize = 0;
        let mut vallen: usize = 0;
        while option_next_keyval(
            &raw mut p,
            &raw mut key,
            &raw mut keylen,
            &raw mut val,
            &raw mut vallen,
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

            if !wp.is_null() {
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
                ns_hl_def(ns_hl, hl_id_link, attrs, hl_id, ptr::null_mut());
            }
        }

        if !wp.is_null() {
            (*wp).w_hl_needs_update = 1;
        }
        true
    }
}

/// Get the script context of global option at index opt_idx.
pub unsafe fn get_option_sctx(opt_idx: OptIndex) -> *mut sctx_T {
    unsafe {
        assert!(opt_idx != kOptInvalid);
        &raw mut (*get_option(opt_idx)).script_ctx
    }
}

/// Set the script_ctx for an option.
pub unsafe fn set_option_sctx(opt_idx: OptIndex, opt_flags: c_int, mut script_ctx: sctx_T) {
    unsafe {
        let both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;

        if (opt_flags & OPT_MODELINE) == 0 {
            script_ctx.sc_lnum += sourcing_lnum();
        }
        nlua_set_sctx(&raw mut script_ctx);

        if both || (opt_flags & OPT_GLOBAL) != 0 || option_is_global_only(opt_idx) {
            (*get_option(opt_idx)).script_ctx = script_ctx;
        }
        if both || (opt_flags & OPT_LOCAL) != 0 {
            if option_has_scope(opt_idx, kOptScopeBuf) {
                (*curbuf).b_p_script_ctx[option_scope_idx(opt_idx, kOptScopeBuf) as usize] =
                    script_ctx;
            } else if option_has_scope(opt_idx, kOptScopeWin) {
                let win_idx = option_scope_idx(opt_idx, kOptScopeWin) as usize;
                (*curwin).w_onebuf_opt.wo_script_ctx[win_idx] = script_ctx;
                if both {
                    (*curwin).w_allbuf_opt.wo_script_ctx[win_idx] = script_ctx;
                }
            }
        }
    }
}

/// Execute OptionSet autocmd now (not deferred).
pub unsafe fn apply_optionset_autocmd_now(
    opt_idx: OptIndex,
    opt_flags: c_int,
    oldval: Object,
    oldval_g: Object,
    oldval_l: Object,
    newval: Object,
) {
    unsafe {
        if starting != 0 || *get_vim_var_str(VV_OPTION_TYPE) != NUL {
            return;
        }

        let mut buf_type = [0 as c_char; 7];
        let mut oldval_tv = opt_to_tv(oldval, false);
        let mut oldval_g_tv = opt_to_tv(oldval_g, false);
        let mut oldval_l_tv = opt_to_tv(oldval_l, false);
        let mut newval_tv = opt_to_tv(newval, false);

        set_vim_var_tv(VV_OPTION_OLD, &raw mut oldval_tv);
        set_vim_var_tv(VV_OPTION_NEW, &raw mut newval_tv);
        let typelen = vim_snprintf_safelen!(
            buf_type.as_mut_ptr(),
            buf_type.len(),
            c"%s".as_ptr(),
            if (opt_flags & OPT_LOCAL) != 0 {
                c"local".as_ptr()
            } else {
                c"global".as_ptr()
            },
        );
        set_vim_var_string(VV_OPTION_TYPE, buf_type.as_ptr(), typelen as isize);
        if (opt_flags & OPT_LOCAL) != 0 {
            set_vim_var_string(
                VV_OPTION_COMMAND,
                c"setlocal".as_ptr(),
                "setlocal".len() as isize,
            );
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &raw mut oldval_tv);
        }
        if (opt_flags & OPT_GLOBAL) != 0 {
            set_vim_var_string(
                VV_OPTION_COMMAND,
                c"setglobal".as_ptr(),
                "setglobal".len() as isize,
            );
            set_vim_var_tv(VV_OPTION_OLDGLOBAL, &raw mut oldval_tv);
        }
        if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0 {
            set_vim_var_string(VV_OPTION_COMMAND, c"set".as_ptr(), "set".len() as isize);
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &raw mut oldval_l_tv);
            set_vim_var_tv(VV_OPTION_OLDGLOBAL, &raw mut oldval_g_tv);
        }
        if (opt_flags & OPT_MODELINE) != 0 {
            set_vim_var_string(
                VV_OPTION_COMMAND,
                c"modeline".as_ptr(),
                "modeline".len() as isize,
            );
            set_vim_var_tv(VV_OPTION_OLDLOCAL, &raw mut oldval_tv);
        }
        apply_autocmds(
            EVENT_OPTIONSET,
            (*get_option(opt_idx)).fullname.as_ptr().cast_mut(),
            ptr::null_mut(),
            false,
            ptr::null_mut(),
        );
        reset_v_option_vars();

        tv_clear(&raw mut oldval_tv);
        tv_clear(&raw mut oldval_g_tv);
        tv_clear(&raw mut oldval_l_tv);
        tv_clear(&raw mut newval_tv);
    }
}

/// For 'modified', the event is deferred.
unsafe fn apply_optionset_autocmd(
    opt_idx: OptIndex,
    opt_flags: c_int,
    oldval: Object,
    oldval_g: Object,
    oldval_l: Object,
    newval: Object,
) {
    unsafe {
        if starting != 0 {
            return;
        }
        if opt_idx == kOptModified {
            aucmd_defer_modified(curbuf, newval.data.boolean());
            return;
        }
        apply_optionset_autocmd_now(opt_idx, opt_flags, oldval, oldval_g, oldval_l, newval);
    }
}

/// Process the updated 'arabic' option value.
pub unsafe fn did_set_arabic(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        let mut errmsg: *const c_char = ptr::null();

        if (*win).w_onebuf_opt.wo_arab != 0 {
            if p_tbidi == 0 {
                if (*win).w_onebuf_opt.wo_rl == 0 {
                    (*win).w_onebuf_opt.wo_rl = 1;
                    changed_window_setting(win);
                }

                if p_arshape == 0 {
                    p_arshape = 1;
                    redraw_all_later(UPD_NOT_VALID);
                }
            }

            if strcmp(p_enc, c"utf-8".as_ptr()) != 0 {
                msg_source(HLF_W);
                msg(tr(W_ARABIC), HLF_W);
                set_vim_var_string(VV_WARNINGMSG, tr(W_ARABIC), -1);
            }

            p_deco = 1;

            errmsg = set_option_value(
                kOptKeymap,
                string_obj(NvimString {
                    data: c"arabic".as_ptr().cast_mut(),
                    size: "arabic".len(),
                }),
                OPT_LOCAL,
                true,
                (*args).os_errbuf,
            );
        } else {
            if p_tbidi == 0 && (*win).w_onebuf_opt.wo_rl != 0 {
                (*win).w_onebuf_opt.wo_rl = 0;
                changed_window_setting(win);
            }

            (*(*win).w_buffer).b_p_iminsert = B_IMODE_NONE;
            (*(*win).w_buffer).b_p_imsearch = B_IMODE_USE_INSERT;
        }

        errmsg
    }
}

/// Process the updated 'autochdir' option value.
pub unsafe fn did_set_autochdir(_args: *mut optset_T) -> *const c_char {
    unsafe {
        do_autochdir();
        ptr::null()
    }
}

/// Process the updated 'binary' option value.
pub unsafe fn did_set_binary(args: *mut optset_T) -> *const c_char {
    unsafe {
        set_options_bin(
            c_int::from((*args).os_oldval.data.boolean()),
            (*(*args).os_buf).b_p_bin,
            (*args).os_flags,
        );
        redraw_titles();
        ptr::null()
    }
}

/// Process the updated 'buflisted' option value.
pub unsafe fn did_set_buflisted(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;

        if c_int::from((*args).os_oldval.data.boolean()) != (*buf).b_p_bl {
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
pub unsafe fn did_set_cmdheight(args: *mut optset_T) -> *const c_char {
    unsafe {
        let old_value = (*args).os_oldval.data.integer();

        if p_ch > OptInt::from(Rows - min_rows(curtab) + 1) {
            p_ch = OptInt::from(Rows - min_rows(curtab) + 1);
        }

        if (p_ch != old_value
            || OptInt::from(tabline_height() + global_stl_height() + (*topframe).fr_height)
                != OptInt::from(Rows) - p_ch)
            && full_screen
        {
            command_height();
        }

        ptr::null()
    }
}

/// Process the updated 'diff' option value.
pub unsafe fn did_set_diff(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        diff_buf_adjust(win);
        if foldmethodIsDiff(win) {
            foldUpdateAll(win);
        }
        ptr::null()
    }
}

/// Process the updated 'endoffile' or 'endofline' or 'fixendofline' or 'bomb' option value.
pub unsafe fn did_set_eof_eol_fixeol_bomb(_args: *mut optset_T) -> *const c_char {
    unsafe {
        redraw_titles();
        ptr::null()
    }
}

/// Process the updated 'equalalways' option value.
pub unsafe fn did_set_equalalways(args: *mut optset_T) -> *const c_char {
    unsafe {
        if p_ea != 0 && !(*args).os_oldval.data.boolean() {
            win_equal((*args).os_win, false, 0);
        }
        ptr::null()
    }
}

/// Process the new 'foldenable' option value.
pub unsafe fn did_set_foldenable(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if (*args).os_varp == (&raw mut (*win).w_onebuf_opt.wo_fen).cast::<c_void>()
            && c_int::from((*args).os_oldval.data.boolean()) != (*win).w_onebuf_opt.wo_fen
            && foldmethodIsDiff(win)
            && (*win).w_onebuf_opt.wo_scb != 0
        {
            let mut wp = firstwin;
            while !wp.is_null() {
                if wp != win && foldmethodIsDiff(wp) && (*wp).w_onebuf_opt.wo_scb != 0 {
                    (*wp).w_onebuf_opt.wo_fen = (*win).w_onebuf_opt.wo_fen;
                    changed_window_setting(wp);
                }
                wp = (*wp).w_next;
            }
        }
        ptr::null()
    }
}

/// Process the new 'foldlevel' option value.
pub unsafe fn did_set_foldlevel(_args: *mut optset_T) -> *const c_char {
    unsafe {
        newFoldLevel();
        ptr::null()
    }
}

/// Process the new 'foldminlines' option value.
pub unsafe fn did_set_foldminlines(args: *mut optset_T) -> *const c_char {
    unsafe {
        foldUpdateAll((*args).os_win);
        ptr::null()
    }
}

/// Process the new 'foldnestmax' option value.
pub unsafe fn did_set_foldnestmax(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if foldmethodIsSyntax(win) || foldmethodIsIndent(win) {
            foldUpdateAll(win);
        }
        ptr::null()
    }
}

/// Multicursor: handle updates to 'follow' option.
pub unsafe fn did_set_follow(args: *mut optset_T) -> *const c_char {
    unsafe {
        if (*args).os_buf == curbuf {
            atom_follow_changed();
        }
        ptr::null()
    }
}

/// Process the new 'helpheight' option value.
pub unsafe fn did_set_helpheight(_args: *mut optset_T) -> *const c_char {
    unsafe {
        if !(*firstwin).w_next.is_null()
            && (*curbuf).b_help
            && OptInt::from((*curwin).w_height) < p_hh
        {
            win_setheight(p_hh as c_int);
        }
        ptr::null()
    }
}

/// Process the updated 'hlsearch' option value.
pub unsafe fn did_set_hlsearch(_args: *mut optset_T) -> *const c_char {
    unsafe {
        set_no_hlsearch(false);
        ptr::null()
    }
}

/// Process the updated 'ignorecase' option value.
pub unsafe fn did_set_ignorecase(_args: *mut optset_T) -> *const c_char {
    unsafe {
        if p_hls != 0 {
            redraw_all_later(UPD_SOME_VALID);
        }
        ptr::null()
    }
}

/// Process the new 'iminsert' option value.
pub unsafe fn did_set_iminsert(_args: *mut optset_T) -> *const c_char {
    unsafe {
        showmode();
        status_redraw_curbuf();
        ptr::null()
    }
}

/// Process the updated 'langnoremap' option value.
pub unsafe fn did_set_langnoremap(_args: *mut optset_T) -> *const c_char {
    unsafe {
        p_lrm = c_int::from(p_lnr == 0);
        ptr::null()
    }
}

/// Process the updated 'langremap' option value.
pub unsafe fn did_set_langremap(_args: *mut optset_T) -> *const c_char {
    unsafe {
        p_lnr = c_int::from(p_lrm == 0);
        ptr::null()
    }
}

/// Process the new 'laststatus' option value.
pub unsafe fn did_set_laststatus(args: *mut optset_T) -> *const c_char {
    unsafe {
        let old_value = (*args).os_oldval.data.integer();
        let value = (*args).os_newval.data.integer();

        if value == 3 && old_value != 3 {
            frame_new_height(
                topframe,
                (*topframe).fr_height - STATUS_HEIGHT,
                false,
                false,
                false,
            );
            win_comp_pos();
            clear_cmdline = true;
        }
        if old_value == 3 && value != 3 {
            frame_new_height(
                topframe,
                (*topframe).fr_height + STATUS_HEIGHT,
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
pub unsafe fn did_set_lines_or_columns(args: *mut optset_T) -> *const c_char {
    unsafe {
        if p_lines != OptInt::from(Rows) || p_columns != OptInt::from(Columns) {
            if updating_screen {
                let oldval = integer_obj((*args).os_oldval.data.integer());
                set_option_varp((*args).os_idx, (*args).os_varp, oldval);
            } else if full_screen {
                screen_resize(p_columns as c_int, p_lines as c_int);
            } else {
                Rows = p_lines as c_int;
                Columns = p_columns as c_int;
                check_screensize();
                let new_row = (OptInt::from(Rows) - p_ch.max(1)) as c_int;
                if cmdline_row > new_row && OptInt::from(Rows) > p_ch {
                    cmdline_row = new_row;
                }
            }
            if p_window >= OptInt::from(Rows) || !option_was_set(kOptWindow) {
                p_window = OptInt::from(Rows - 1);
            }
        }

        if p_sj >= OptInt::from(Rows) && full_screen {
            p_sj = OptInt::from(Rows / 2);
        }

        ptr::null()
    }
}

/// Process the updated 'lisp' option value.
pub unsafe fn did_set_lisp(args: *mut optset_T) -> *const c_char {
    unsafe {
        buf_init_isk_chartab((*args).os_buf);
        ptr::null()
    }
}

/// Process the updated 'modifiable' option value.
pub unsafe fn did_set_modifiable(_args: *mut optset_T) -> *const c_char {
    unsafe {
        redraw_titles();
        ptr::null()
    }
}

/// Process the updated 'modified' option value.
pub unsafe fn did_set_modified(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;
        if !(*args).os_newval.data.boolean() {
            save_file_ff(buf);
        }
        redraw_titles();
        (*buf).b_modified_was_set = (*args).os_newval.data.boolean();
        ptr::null()
    }
}

/// Process the updated 'number' or 'relativenumber' option value.
pub unsafe fn did_set_number_relativenumber(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if *(*win).w_onebuf_opt.wo_stc != NUL {
            (*win).w_nrwidth_line_count = 0;
        }
        check_signcolumn(ptr::null_mut(), win);
        ptr::null()
    }
}

/// Process the new 'numberwidth' option value.
pub unsafe fn did_set_numberwidth(args: *mut optset_T) -> *const c_char {
    unsafe {
        (*(*args).os_win).w_nrwidth_line_count = 0;
        ptr::null()
    }
}

/// Process the updated 'paste' option value.
pub unsafe fn did_set_paste(_args: *mut optset_T) -> *const c_char {
    unsafe {
        let empty_opt = (&raw mut empty_string_option).cast::<c_char>();
        if p_paste != 0 {
            if OLD_P_PASTE == 0 {
                let mut buf = firstbuf;
                while !buf.is_null() {
                    (*buf).b_p_tw_nopaste = (*buf).b_p_tw;
                    (*buf).b_p_wm_nopaste = (*buf).b_p_wm;
                    (*buf).b_p_sts_nopaste = (*buf).b_p_sts;
                    (*buf).b_p_ai_nopaste = (*buf).b_p_ai;
                    (*buf).b_p_et_nopaste = (*buf).b_p_et;
                    (*buf).b_p_vsts_nopaste =
                        if !(*buf).b_p_vsts.is_null() && (*buf).b_p_vsts != empty_opt {
                            Some(crate::memory::cstr_to_cstring((*buf).b_p_vsts))
                        } else {
                            None
                        };
                    buf = (*buf).b_next;
                }

                SAVE_SM = p_sm;
                SAVE_STA = p_sta;
                SAVE_RU = p_ru;
                SAVE_RI = p_ri;
                P_AI_NOPASTE = p_ai;
                P_ET_NOPASTE = p_et;
                P_STS_NOPASTE = p_sts;
                P_TW_NOPASTE = p_tw;
                P_WM_NOPASTE = p_wm;
                P_VSTS_NOPASTE = if !p_vsts.is_null() && p_vsts != empty_opt {
                    Some(crate::memory::cstr_to_cstring(p_vsts))
                } else {
                    None
                };
            }

            let mut buf = firstbuf;
            while !buf.is_null() {
                (*buf).b_p_tw = 0;
                (*buf).b_p_wm = 0;
                (*buf).b_p_sts = 0;
                (*buf).b_p_ai = 0;
                (*buf).b_p_et = 0;
                if !(*buf).b_p_vsts.is_null() {
                    free_string_option((*buf).b_p_vsts);
                }
                (*buf).b_p_vsts = empty_opt;
                (*buf).b_p_vsts_array.clear();
                buf = (*buf).b_next;
            }

            p_sm = 0;
            p_sta = 0;
            if p_ru != 0 {
                status_redraw_all();
            }
            p_ru = 0;
            p_ri = 0;
            p_tw = 0;
            p_wm = 0;
            p_sts = 0;
            p_ai = 0;
            p_et = 0;
            if !p_vsts.is_null() {
                free_string_option(p_vsts);
            }
            p_vsts = empty_opt;
        } else if OLD_P_PASTE != 0 {
            let mut buf = firstbuf;
            while !buf.is_null() {
                (*buf).b_p_tw = (*buf).b_p_tw_nopaste;
                (*buf).b_p_wm = (*buf).b_p_wm_nopaste;
                (*buf).b_p_sts = (*buf).b_p_sts_nopaste;
                (*buf).b_p_ai = (*buf).b_p_ai_nopaste;
                (*buf).b_p_et = (*buf).b_p_et_nopaste;
                if !(*buf).b_p_vsts.is_null() {
                    free_string_option((*buf).b_p_vsts);
                }
                (*buf).b_p_vsts = (*buf)
                    .b_p_vsts_nopaste
                    .as_ref()
                    .map_or(empty_opt, |s| alloc_string_option(s.as_ptr()));
                if !(*buf).b_p_vsts.is_null() && (*buf).b_p_vsts != empty_opt {
                    tabstop_set((*buf).b_p_vsts, &mut (*buf).b_p_vsts_array, ptr::null());
                } else {
                    (*buf).b_p_vsts_array.clear();
                }
                buf = (*buf).b_next;
            }

            p_sm = SAVE_SM;
            p_sta = SAVE_STA;
            if p_ru != SAVE_RU {
                status_redraw_all();
            }
            p_ru = SAVE_RU;
            p_ri = SAVE_RI;
            p_ai = P_AI_NOPASTE;
            p_et = P_ET_NOPASTE;
            p_sts = P_STS_NOPASTE;
            p_tw = P_TW_NOPASTE;
            p_wm = P_WM_NOPASTE;
            if !p_vsts.is_null() {
                free_string_option(p_vsts);
            }
            let vsts_np_ptr = &raw const P_VSTS_NOPASTE;
            p_vsts = (*vsts_np_ptr)
                .as_ref()
                .map_or(empty_opt, |s| alloc_string_option(s.as_ptr()));
        }

        OLD_P_PASTE = p_paste;

        didset_options_sctx(
            OPT_LOCAL | OPT_GLOBAL,
            (&raw mut P_PASTE_DEP_OPTS).cast::<c_int>(),
        );

        ptr::null()
    }
}

/// Validate the 'previewwindow' option.
pub unsafe fn validate_previewwindow(args: *mut optset_T) -> *const c_char {
    unsafe {
        if ((*args).os_flags & OPT_GLOBAL) != 0 || !(*args).os_newval.data.boolean() {
            return ptr::null();
        }
        let win = (*args).os_win;
        let tab = if win == curwin {
            curtab
        } else {
            win_find_tabpage(win)
        };
        let mut wp = if tab == curtab {
            firstwin
        } else {
            (*tab).tp_firstwin
        };
        while !wp.is_null() {
            if wp != win && (*wp).w_onebuf_opt.wo_pvw != 0 {
                return E_PREVIEW_WINDOW_ALREADY_EXISTS;
            }
            wp = (*wp).w_next;
        }
        ptr::null()
    }
}

/// Process the new 'pumblend' option value.
pub unsafe fn did_set_pumblend(_args: *mut optset_T) -> *const c_char {
    unsafe {
        hl_invalidate_blends();
        if pum_drawn() {
            pum_redraw();
        }
        ptr::null()
    }
}

/// Process the updated 'readonly' option value.
pub unsafe fn did_set_readonly(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;

        if (*buf).b_p_ro == 0 && ((*args).os_flags & OPT_LOCAL) == 0 {
            readonlymode = false;
        }

        if (*buf).b_p_ro != 0 {
            (*buf).b_did_warn = false;
        }

        redraw_titles();
        ptr::null()
    }
}

/// Process the new 'scrollback' option value.
pub unsafe fn did_set_scrollback(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;
        let old_value = (*args).os_oldval.data.integer();
        let value = (*args).os_newval.data.integer();

        if !(*buf).terminal.is_null() && value < old_value {
            on_scrollback_option_changed((*buf).terminal.cast());
        }
        ptr::null()
    }
}

/// Process the updated 'scrollbind' option value.
pub unsafe fn did_set_scrollbind(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;

        if (*win).w_onebuf_opt.wo_scb == 0 {
            return ptr::null();
        }
        do_check_scrollbind(false);
        (*win).w_scbind_pos = get_vtopline(win);
        ptr::null()
    }
}

/// Process the new 'shiftwidth' or the 'tabstop' option value.
pub unsafe fn did_set_shiftwidth_tabstop(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;
        let win = (*args).os_win;
        let pp = (*args).os_varp.cast::<OptInt>();

        if foldmethodIsIndent(win) {
            foldUpdateAll(win);
        }
        if pp == &raw mut (*buf).b_p_sw || (*buf).b_p_sw == 0 {
            parse_cino(buf);
        }
        ptr::null()
    }
}

/// Process the new 'showtabline' option value.
pub unsafe fn did_set_showtabline(_args: *mut optset_T) -> *const c_char {
    unsafe {
        win_new_screen_rows();
        ptr::null()
    }
}

/// Process the updated 'smoothscroll' option value.
pub unsafe fn did_set_smoothscroll(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if (*win).w_onebuf_opt.wo_sms == 0 {
            (*win).w_skipcol = 0;
        }
        ptr::null()
    }
}

/// Process the updated 'spell' option value.
pub unsafe fn did_set_spell(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if (*win).w_onebuf_opt.wo_spell != 0 {
            return parse_spelllang(win);
        }
        ptr::null()
    }
}

/// Process the updated 'swapfile' option value.
pub unsafe fn did_set_swapfile(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;
        if (*buf).b_p_swf != 0 && p_uc != 0 {
            ml_open_file(buf);
        } else {
            mf_close_file(buf, true);
        }
        ptr::null()
    }
}

/// Process the new 'textwidth' option value.
pub unsafe fn did_set_textwidth(_args: *mut optset_T) -> *const c_char {
    unsafe {
        let mut tp = first_tabpage;
        while !tp.is_null() {
            let mut wp = if tp == curtab {
                firstwin
            } else {
                (*tp).tp_firstwin
            };
            while !wp.is_null() {
                check_colorcolumn(ptr::null_mut(), wp);
                wp = (*wp).w_next;
            }
            tp = (*tp).tp_next;
        }
        ptr::null()
    }
}

/// Process the updated 'title' or the 'icon' option value.
pub unsafe fn did_set_title_icon(_args: *mut optset_T) -> *const c_char {
    unsafe {
        did_set_title();
        ptr::null()
    }
}

/// Process the new 'titlelen' option value.
pub unsafe fn did_set_titlelen(args: *mut optset_T) -> *const c_char {
    unsafe {
        let old_value = (*args).os_oldval.data.integer();
        if starting != NO_SCREEN && old_value != p_titlelen {
            need_maketitle = true;
        }
        ptr::null()
    }
}

/// Process the updated 'undofile' option value.
pub unsafe fn did_set_undofile(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;

        if (*buf).b_p_udf == 0 && p_udf == 0 {
            return ptr::null();
        }

        let mut hash = [0u8; UNDO_HASH_SIZE];

        let mut bp = firstbuf;
        while !bp.is_null() {
            if (buf == bp || ((*args).os_flags & OPT_GLOBAL) != 0 || (*args).os_flags == 0)
                && !bufIsChanged(bp)
                && (*bp).b_ml.ml_mfp.is_some()
            {
                u_compute_hash(bp, hash.as_mut_ptr());
                u_read_undo(ptr::null_mut(), hash.as_ptr(), (*bp).b_fname);
            }
            bp = (*bp).b_next;
        }

        ptr::null()
    }
}

/// Process the new global 'undolevels' option value.
pub unsafe fn did_set_global_undolevels(value: OptInt, old_value: OptInt) -> *const c_char {
    unsafe {
        p_ul = old_value;
        u_sync(true);
        p_ul = value;
        ptr::null()
    }
}

/// Process the new buffer local 'undolevels' option value.
pub unsafe fn did_set_buflocal_undolevels(
    buf: *mut buf_T,
    value: OptInt,
    old_value: OptInt,
) -> *const c_char {
    unsafe {
        (*buf).b_p_ul = old_value;
        u_sync(true);
        (*buf).b_p_ul = value;
        ptr::null()
    }
}

/// Process the new 'undolevels' option value.
pub unsafe fn did_set_undolevels(args: *mut optset_T) -> *const c_char {
    unsafe {
        let buf = (*args).os_buf;
        let pp = (*args).os_varp.cast::<OptInt>();

        if pp == &raw mut p_ul {
            did_set_global_undolevels(
                (*args).os_newval.data.integer(),
                (*args).os_oldval.data.integer(),
            );
        } else if pp == &raw mut (*buf).b_p_ul {
            did_set_buflocal_undolevels(
                buf,
                (*args).os_newval.data.integer(),
                (*args).os_oldval.data.integer(),
            );
        }

        ptr::null()
    }
}

/// Process the new 'updatecount' option value.
pub unsafe fn did_set_updatecount(args: *mut optset_T) -> *const c_char {
    unsafe {
        let old_value = (*args).os_oldval.data.integer();
        if p_uc != 0 && old_value == 0 {
            ml_open_files();
        }
        ptr::null()
    }
}

/// Validate the 'wildchar' or 'wildcharm' option.
pub unsafe fn validate_wildchar(args: *mut optset_T) -> *const c_char {
    unsafe {
        let c = (*args).os_newval.data.integer();
        if c == Ctrl_C
            || c == OptInt::from(b'\n')
            || c == OptInt::from(b'\r')
            || c == OptInt::from(K_KENTER)
        {
            return e_invarg.as_ptr();
        }
        ptr::null()
    }
}

/// Process the new 'winblend' option value.
pub unsafe fn did_set_winblend(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        let old_value = (*args).os_oldval.data.integer();
        let value = (*args).os_newval.data.integer();

        if value != old_value {
            (*win).w_onebuf_opt.wo_winbl = (*win).w_onebuf_opt.wo_winbl.clamp(0, 100);
            (*win).w_hl_needs_update = 1;
            check_blending(win);
        }
        ptr::null()
    }
}

/// Process the new 'window' option value.
pub unsafe fn did_set_window(_args: *mut optset_T) -> *const c_char {
    unsafe {
        if p_window < 1 || p_window >= OptInt::from(Rows) {
            p_window = OptInt::from(Rows - 1);
        }
        ptr::null()
    }
}

/// Process the new 'winheight' value.
pub unsafe fn did_set_winheight(_args: *mut optset_T) -> *const c_char {
    unsafe {
        if !(*firstwin).w_next.is_null() && OptInt::from((*curwin).w_height) < p_wh {
            win_setheight(p_wh as c_int);
        }
        ptr::null()
    }
}

/// Process the new 'winwidth' option value.
pub unsafe fn did_set_winwidth(_args: *mut optset_T) -> *const c_char {
    unsafe {
        if !(*firstwin).w_next.is_null() && OptInt::from((*curwin).w_width) < p_wiw {
            win_setwidth(p_wiw as c_int);
        }
        ptr::null()
    }
}

/// Process the updated 'wrap' option value.
pub unsafe fn did_set_wrap(args: *mut optset_T) -> *const c_char {
    unsafe {
        let win = (*args).os_win;
        if (*win).w_onebuf_opt.wo_wrap != 0 {
            (*win).w_leftcol = 0;
        } else {
            (*win).w_skipcol = 0;
        }
        ptr::null()
    }
}

/// Process the new 'chistory' or 'lhistory' option value.
pub unsafe fn did_set_xhistory(args: *mut optset_T) -> *const c_char {
    unsafe {
        let is_p_chi = (*args).os_varp.cast::<OptInt>() == &raw mut p_chi;
        let arg = if is_p_chi {
            &raw mut p_chi
        } else {
            (*args).os_varp.cast::<OptInt>()
        };

        if is_p_chi {
            qf_resize_stack(*arg as c_int);
        } else {
            ll_resize_stack((*args).os_win, *arg as c_int);
        }

        ptr::null()
    }
}

unsafe fn do_syntax_autocmd(buf: *mut buf_T, value_changed: bool) {
    unsafe {
        SYN_RECURSIVE += 1;
        (*buf).b_flags |= BF_SYN_SET;
        apply_autocmds(
            EVENT_SYNTAX,
            (*buf).b_p_syn,
            (*buf).b_fname,
            value_changed || SYN_RECURSIVE == 1,
            buf,
        );
        SYN_RECURSIVE -= 1;
    }
}

unsafe fn do_spelllang_source(win: *mut win_T) {
    unsafe {
        let mut fname = [0 as c_char; 200];
        let ws = (*win).w_s.cast::<synblock_T>();
        let mut q = (*ws).b_p_spl;

        if strncmp(q, c"cjk,".as_ptr(), 4) == 0 {
            q = q.add(4);
        }

        let mut p = q;
        while *p != NUL {
            if !ascii_isalnum(*p) && *p != b'-' as c_char {
                break;
            }
            p = p.add(1);
        }
        if p > q {
            vim_snprintf!(
                fname.as_mut_ptr(),
                fname.len(),
                c"spell/%.*s.*".as_ptr(),
                p.offset_from(q) as c_int,
                q,
            );
            source_runtime_vim_lua(fname.as_mut_ptr(), DIP_ALL);
        }
    }
}

/// Check the bounds of numeric options.
unsafe fn check_num_option_bounds(
    opt_idx: OptIndex,
    newval: *mut OptInt,
    win: *mut win_T,
    errbuf: *const CharBuf,
) -> *const c_char {
    unsafe {
        let mut errmsg: *const c_char = ptr::null();

        match opt_idx {
            kOptLines => {
                let min_r = min_rows_for_all_tabpages();
                if *newval < OptInt::from(min_r) && full_screen {
                    errmsg = opt_error_int(errbuf, c"E593: Need at least %d lines".as_ptr(), min_r);
                    *newval = OptInt::from(min_r);
                }
                *newval = (*newval).min(OptInt::from(c_int::MAX));
            }
            kOptColumns => {
                if *newval < OptInt::from(MIN_COLUMNS) && full_screen {
                    errmsg = opt_error_int(
                        errbuf,
                        c"E594: Need at least %d columns".as_ptr(),
                        MIN_COLUMNS,
                    );
                    *newval = OptInt::from(MIN_COLUMNS);
                }
                *newval = (*newval).min(OptInt::from(c_int::MAX));
            }
            kOptPumblend => {
                *newval = (*newval).clamp(0, 100);
            }
            kOptScrolljump => {
                if (*newval < -100 || *newval >= OptInt::from(Rows)) && full_screen {
                    errmsg = e_scroll.as_ptr();
                    *newval = 1;
                }
            }
            kOptScroll
                if (*newval <= 0
                    || (*newval > OptInt::from((*win).w_view_height)
                        && (*win).w_view_height > 0))
                    && full_screen =>
            {
                if *newval != 0 {
                    errmsg = e_scroll.as_ptr();
                }
                *newval = OptInt::from(win_default_scroll(win));
            }
            _ => {}
        }

        errmsg
    }
}

/// Validate and bound check option value.
unsafe fn validate_num_option(
    opt_idx: OptIndex,
    newval: *mut OptInt,
    win: *mut win_T,
    errbuf: *const CharBuf,
) -> *const c_char {
    unsafe {
        let value = *newval;

        if value < OptInt::from(c_int::MIN) || value > OptInt::from(c_int::MAX) {
            return e_invarg.as_ptr();
        }

        const MAX_SEARCH_COUNT: OptInt = 9999;

        match opt_idx {
            kOptHelpheight | kOptTitlelen | kOptUpdatecount | kOptReport | kOptUpdatetime
            | kOptSidescroll | kOptFoldlevel | kOptShiftwidth | kOptTextwidth | kOptWritedelay
            | kOptTimeoutlen => {
                if value < 0 {
                    return e_positive.as_ptr();
                }
            }
            kOptWinheight => {
                if value < 1 {
                    return e_positive.as_ptr();
                } else if p_wmh > value {
                    return e_winheight.as_ptr();
                }
            }
            kOptWinminheight => {
                if value < 0 {
                    return e_positive.as_ptr();
                } else if value > p_wh {
                    return e_winheight.as_ptr();
                }
            }
            kOptWinwidth => {
                if value < 1 {
                    return e_positive.as_ptr();
                } else if p_wmw > value {
                    return e_winwidth.as_ptr();
                }
            }
            kOptWinminwidth => {
                if value < 0 {
                    return e_positive.as_ptr();
                } else if value > p_wiw {
                    return e_winwidth.as_ptr();
                }
            }
            kOptMaxcombine => {
                *newval = MAX_MCO;
            }
            kOptCmdheight => {
                if value < 0 {
                    return e_positive.as_ptr();
                }
            }
            kOptHistory => {
                if value < 0 {
                    return e_positive.as_ptr();
                } else if value > 10000 {
                    return e_invarg.as_ptr();
                }
            }
            kOptPyxversion => {
                if value == 0 {
                    *newval = 3;
                } else if value != 3 {
                    return e_invarg.as_ptr();
                }
            }
            kOptRegexpengine => {
                if !(0..=2).contains(&value) {
                    return e_invarg.as_ptr();
                }
            }
            kOptScrolloff => {
                if value < 0 && full_screen {
                    return e_positive.as_ptr();
                }
            }
            kOptScrolloffpad => {
                if value < 0 {
                    return e_invarg.as_ptr();
                }
            }
            kOptSidescrolloff => {
                if value < 0 && full_screen {
                    return e_positive.as_ptr();
                }
            }
            kOptCmdwinheight => {
                if value < 1 {
                    return e_positive.as_ptr();
                }
            }
            kOptConceallevel => {
                if value < 0 {
                    return e_positive.as_ptr();
                } else if value > 3 {
                    return e_invarg.as_ptr();
                }
            }
            kOptNumberwidth => {
                if value < 1 {
                    return e_positive.as_ptr();
                } else if value > MAX_NUMBERWIDTH {
                    return e_invarg.as_ptr();
                }
            }
            kOptIminsert => {
                if !(0..=B_IMODE_LAST).contains(&value) {
                    return e_invarg.as_ptr();
                }
            }
            kOptImsearch => {
                if !(-1..=B_IMODE_LAST).contains(&value) {
                    return e_invarg.as_ptr();
                }
            }
            kOptChannel => {
                return e_invarg.as_ptr();
            }
            kOptScrollback => {
                if !(-1..=SB_MAX).contains(&value) {
                    return e_invarg.as_ptr();
                }
            }
            kOptTabstop => {
                if value < 1 {
                    return e_positive.as_ptr();
                } else if value > OptInt::from(crate::indent::TABSTOP_MAX) {
                    return e_invarg.as_ptr();
                }
            }
            kOptChistory | kOptLhistory => {
                if value < 1 {
                    return E_CANNOT_HAVE_NEGATIVE_OR_ZERO_NUMBER_OF_QUICKFIX;
                } else if value > 100 {
                    return E_CANNOT_HAVE_MORE_THAN_HUNDRED_QUICKFIX;
                }
            }
            kOptMaxsearchcount => {
                if value <= 0 {
                    return e_positive.as_ptr();
                } else if value > MAX_SEARCH_COUNT {
                    return e_invarg.as_ptr();
                }
            }
            _ => {}
        }

        check_num_option_bounds(opt_idx, newval, win, errbuf)
    }
}

/// Called after an option changed: check if something needs to be redrawn.
pub unsafe fn check_redraw_for(buf: *mut buf_T, win: *mut win_T, flags: u32) {
    unsafe {
        let all = (flags & kOptFlagRedrAll) == kOptFlagRedrAll;

        if (flags & kOptFlagRedrStat) != 0 || all {
            status_redraw_all();
        }

        if (flags & kOptFlagRedrTabl) != 0 || all {
            redraw_tabline = true;
        }

        if (flags & kOptFlagRedrBuf) != 0 || (flags & kOptFlagRedrWin) != 0 || all {
            if (flags & kOptFlagHLOnly) != 0 {
                redraw_later(win, UPD_NOT_VALID);
            } else {
                changed_window_setting(win);
            }
        }
        if (flags & kOptFlagRedrBuf) != 0 {
            redraw_buf_later(buf, UPD_NOT_VALID);
        }
        if all {
            redraw_all_later(UPD_NOT_VALID);
        }
    }
}

pub unsafe fn check_redraw(flags: u32) {
    unsafe {
        check_redraw_for(curbuf, curwin, flags);
    }
}

pub unsafe fn is_tty_option(name: *const c_char) -> bool {
    unsafe { !find_tty_option_end(name).is_null() }
}

/// Get value of TTY option.
pub unsafe fn get_tty_option(name: *const c_char) -> Object {
    unsafe {
        if strequal(name, c"t_Co".as_ptr()) {
            if t_colors <= 1 {
                cstr_to_obj(c"".as_ptr())
            } else {
                let mut value = [0 as c_char; NUMBUFLEN];
                vim_snprintf!(value.as_mut_ptr(), NUMBUFLEN, c"%d".as_ptr(), t_colors);
                cstr_to_obj(value.as_ptr())
            }
        } else if strequal(name, c"term".as_ptr()) {
            let term_ptr = &raw const P_TERM;
            cstr_to_obj(
                (*term_ptr)
                    .as_ref()
                    .map_or(c"nvim".as_ptr(), |s| s.as_ptr()),
            )
        } else if strequal(name, c"ttytype".as_ptr()) {
            let tty_ptr = &raw const P_TTYTYPE;
            cstr_to_obj(
                (*tty_ptr)
                    .as_ref()
                    .map_or(c"nvim".as_ptr(), |s| s.as_ptr()),
            )
        } else if is_tty_option(name) {
            cstr_to_obj(c"".as_ptr())
        } else {
            nil_obj()
        }
    }
}

pub unsafe fn set_tty_option(name: *const c_char, value: Option<std::ffi::CString>) -> bool {
    unsafe {
        if strequal(name, c"term".as_ptr()) {
            P_TERM = value;
            return true;
        }

        if strequal(name, c"ttytype".as_ptr()) {
            P_TTYTYPE = value;
            return true;
        }

        false
    }
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
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData::String(NvimString {
                    data: S.as_ptr().cast_mut(),
                    size: S.to_bytes().len(),
                }),
        }
    }};
}

macro_rules! opt_cb {
    ($cb:path) => {{ { Some($cb) } }};
}

static mut OPTIONS: [vimoption_T; 379] = [
    vimoption_T { fullname: c"aleph", shortname: Some(c"al"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[0].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [0, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(224) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"allowrevins", shortname: Some(c"ari"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ari).cast(), flags_var: ptr::null_mut(), scope_idx: [1, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ambiwidth", shortname: Some(c"ambw"), flags: kOptFlagRedrAll|kOptFlagUIOption, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ambw).cast(), flags_var: ptr::null_mut(), scope_idx: [2, -1, -1, -1], immutable: false, values: &opt_ambw_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_ambiwidth), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"single"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"arabic", shortname: Some(c"arab"), flags: kOptFlagCurswant, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 0, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_arabic), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"arabicshape", shortname: Some(c"arshape"), flags: kOptFlagRedrAll|kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_arshape).cast(), flags_var: ptr::null_mut(), scope_idx: [3, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autochdir", shortname: Some(c"acd"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_acd).cast(), flags_var: ptr::null_mut(), scope_idx: [4, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_autochdir), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autocomplete", shortname: Some(c"ac"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_ac).cast(), flags_var: ptr::null_mut(), scope_idx: [5, -1, 0, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autocompletedelay", shortname: Some(c"acl"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_acl).cast(), flags_var: ptr::null_mut(), scope_idx: [6, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autocompletetimeout", shortname: Some(c"act"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_act).cast(), flags_var: ptr::null_mut(), scope_idx: [7, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autoindent", shortname: Some(c"ai"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ai).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autoread", shortname: Some(c"ar"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_ar).cast(), flags_var: ptr::null_mut(), scope_idx: [8, -1, 2, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autowrite", shortname: Some(c"aw"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_aw).cast(), flags_var: ptr::null_mut(), scope_idx: [9, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"autowriteall", shortname: Some(c"awa"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_awa).cast(), flags_var: ptr::null_mut(), scope_idx: [10, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"background", shortname: Some(c"bg"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bg).cast(), flags_var: ptr::null_mut(), scope_idx: [11, -1, -1, -1], immutable: false, values: &opt_bg_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_background), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dark"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"backspace", shortname: Some(c"bs"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bs).cast(), flags_var: ptr::null_mut(), scope_idx: [12, -1, -1, -1], immutable: false, values: &opt_bs_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backspace), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"indent,eol,start"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"backup", shortname: Some(c"bk"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bk).cast(), flags_var: ptr::null_mut(), scope_idx: [13, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(unix)]
    vimoption_T { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_bkc).cast(), flags_var: &raw mut bkc_flags, scope_idx: [14, -1, 3, -1], immutable: false, values: &opt_bkc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(unix))]
    vimoption_T { fullname: c"backupcopy", shortname: Some(c"bkc"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_bkc).cast(), flags_var: &raw mut bkc_flags, scope_idx: [14, -1, 3, -1], immutable: false, values: &opt_bkc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupcopy), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_backupcopy), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"backupdir", shortname: Some(c"bdir"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDefExp|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bdir).cast(), flags_var: ptr::null_mut(), scope_idx: [15, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"backupext", shortname: Some(c"bex"), flags: kOptFlagNFname, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bex).cast(), flags_var: ptr::null_mut(), scope_idx: [16, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"~"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"backupskip", shortname: Some(c"bsk"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bsk).cast(), flags_var: ptr::null_mut(), scope_idx: [17, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"belloff", shortname: Some(c"bo"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_bo).cast(), flags_var: &raw mut bo_flags, scope_idx: [18, -1, -1, -1], immutable: false, values: &opt_bo_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"all"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"binary", shortname: Some(c"bin"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_bin).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 4, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_binary), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"bomb", shortname: None, flags: kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_bomb).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 5, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"breakat", shortname: Some(c"brk"), flags: kOptFlagFlagList|kOptFlagRedrAll, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_breakat).cast(), flags_var: ptr::null_mut(), scope_idx: [19, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakat), opt_expand_cb: None, def_val: cstr_def_val!(c" \t!@*-+;:,./?"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"breakindent", shortname: Some(c"bri"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"breakindentopt", shortname: Some(c"briopt"), flags: kOptFlagOneComma|kOptFlagRedrBuf|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 2, -1, -1], immutable: false, values: &opt_briopt_values, schema: &opt_briopt_schema, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_breakindentopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"browsedir", shortname: Some(c"bsdir"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[26].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [20, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"bufhidden", shortname: Some(c"bh"), flags: kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_bh).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 6, -1], immutable: false, values: &opt_bh_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"buflisted", shortname: Some(c"bl"), flags: kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_bl).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 7, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_buflisted), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"buftype", shortname: Some(c"bt"), flags: kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_bt).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 8, -1], immutable: false, values: &opt_bt_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_buftype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_buftype), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"busy", shortname: None, flags: kOptFlagRedrStat|kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_busy).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 9, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"casemap", shortname: Some(c"cmp"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cmp).cast(), flags_var: &raw mut cmp_flags, scope_idx: [21, -1, -1, -1], immutable: false, values: &opt_cmp_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"internal,keepascii"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"cdhome", shortname: Some(c"cdh"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cdh).cast(), flags_var: ptr::null_mut(), scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"cdhome", shortname: Some(c"cdh"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cdh).cast(), flags_var: ptr::null_mut(), scope_idx: [22, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cdpath", shortname: Some(c"cd"), flags: kOptFlagComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cdpath).cast(), flags_var: ptr::null_mut(), scope_idx: [23, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c",,"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cedit", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cedit).cast(), flags_var: ptr::null_mut(), scope_idx: [24, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::ex_getln::validate_cedit), opt_did_set_cb: opt_cb!(crate::ex_getln::did_set_cedit), opt_expand_cb: None, def_val: cstr_def_val!(c"\x06"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"channel", shortname: None, flags: kOptFlagNoMkrc|kOptFlagNoDefault, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_channel).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 10, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"charconvert", shortname: Some(c"ccv"), flags: kOptFlagExpr|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ccv).cast(), flags_var: ptr::null_mut(), scope_idx: [25, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"chistory", shortname: Some(c"chi"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_chi).cast(), flags_var: ptr::null_mut(), scope_idx: [26, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cindent", shortname: Some(c"cin"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cin).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 11, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cinkeys", shortname: Some(c"cink"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cink).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 12, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cinoptions", shortname: Some(c"cino"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cino).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 13, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cinoptions), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cinscopedecls", shortname: Some(c"cinsd"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cinsd).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 14, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"public,protected,private"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cinwords", shortname: Some(c"cinw"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cinw).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 15, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"if,else,while,do,for,switch"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"clipboard", shortname: Some(c"cb"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cb).cast(), flags_var: &raw mut cb_flags, scope_idx: [27, -1, -1, -1], immutable: false, values: &opt_cb_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cmdheight", shortname: Some(c"ch"), flags: kOptFlagRedrAll, opt_type: ObjectType::kObjectTypeInteger, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeTab)) as u8, var: (&raw mut p_ch).cast(), flags_var: ptr::null_mut(), scope_idx: [28, -1, -1, 0], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_cmdheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cmdwinheight", shortname: Some(c"cwh"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cwh).cast(), flags_var: ptr::null_mut(), scope_idx: [29, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(7) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"colorcolumn", shortname: Some(c"cc"), flags: kOptFlagOneComma|kOptFlagRedrWin|kOptFlagHLOnly|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 3, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_colorcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_colorcolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"columns", shortname: Some(c"co"), flags: kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_columns).cast(), flags_var: ptr::null_mut(), scope_idx: [30, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(80) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"comments", shortname: Some(c"com"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_com).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 16, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_comments), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"s1:/*,mb:*,ex:*/,://,b:#,:%,:XCOMM,n:>,fb:-,fb:•"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"commentstring", shortname: Some(c"cms"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cms).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 17, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_commentstring), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"compatible", shortname: Some(c"cp"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[50].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [31, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"complete", shortname: Some(c"cpt"), flags: kOptFlagOneComma|kOptFlagNoDup|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cpt).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 18, -1], immutable: false, values: &opt_cpt_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_complete), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_complete), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c".,w,b,u,t"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"completefunc", shortname: Some(c"cfu"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_cfu).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 19, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"completeitemalign", shortname: Some(c"cia"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cia).cast(), flags_var: ptr::null_mut(), scope_idx: [32, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_completeitemalign), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeitemalign), opt_expand_cb: None, def_val: cstr_def_val!(c"abbr,kind,menu"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"completeopt", shortname: Some(c"cot"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_cot).cast(), flags_var: &raw mut cot_flags, scope_idx: [33, -1, 20, -1], immutable: false, values: &opt_cot_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu,popup"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_csl).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 21, -1], immutable: false, values: &opt_csl_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_completeslash), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"completeslash", shortname: Some(c"csl"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: unsafe { (&raw mut OPTIONS[55].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [-1, -1, 21, -1], immutable: true, values: &opt_csl_values, schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"completetimeout", shortname: Some(c"cto"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cto).cast(), flags_var: ptr::null_mut(), scope_idx: [34, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"concealcursor", shortname: Some(c"cocu"), flags: kOptFlagFlagList|kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 4, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_concealcursor), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_concealcursor), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"conceallevel", shortname: Some(c"cole"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 5, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"confirm", shortname: Some(c"cf"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_confirm).cast(), flags_var: ptr::null_mut(), scope_idx: [35, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"copyindent", shortname: Some(c"ci"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ci).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 22, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cpoptions", shortname: Some(c"cpo"), flags: kOptFlagFlagList|kOptFlagRedrAll, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_cpo).cast(), flags_var: ptr::null_mut(), scope_idx: [36, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cpoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_cpoptions), def_val: cstr_def_val!(CPO_VIM_CSTR), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cursorbind", shortname: Some(c"crb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 6, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cursorcolumn", shortname: Some(c"cuc"), flags: kOptFlagRedrWin|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 7, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cursorline", shortname: Some(c"cul"), flags: kOptFlagRedrWin|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 8, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"cursorlineopt", shortname: Some(c"culopt"), flags: kOptFlagOneComma|kOptFlagRedrWin|kOptFlagHLOnly|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 9, -1, -1], immutable: false, values: &opt_culopt_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_cursorlineopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_cursorlineopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"debug", shortname: None, flags: kOptFlagComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_debug).cast(), flags_var: ptr::null_mut(), scope_idx: [37, -1, -1, -1], immutable: false, values: &opt_debug_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"define", shortname: Some(c"def"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_def).cast(), flags_var: ptr::null_mut(), scope_idx: [38, -1, 23, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"delcombine", shortname: Some(c"deco"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_deco).cast(), flags_var: ptr::null_mut(), scope_idx: [39, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"dictionary", shortname: Some(c"dict"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagNDname, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_dict).cast(), flags_var: ptr::null_mut(), scope_idx: [40, -1, 24, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"diff", shortname: None, flags: kOptFlagRedrWin|kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 10, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_diff), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"diffanchors", shortname: Some(c"dia"), flags: kOptFlagOneComma, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_dia).cast(), flags_var: ptr::null_mut(), scope_idx: [41, -1, 25, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffanchors), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"diffexpr", shortname: Some(c"dex"), flags: kOptFlagCurswant|kOptFlagExpr|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_dex).cast(), flags_var: ptr::null_mut(), scope_idx: [42, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"diffopt", shortname: Some(c"dip"), flags: kOptFlagOneComma|kOptFlagColon|kOptFlagRedrWin|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_dip).cast(), flags_var: ptr::null_mut(), scope_idx: [43, -1, -1, -1], immutable: false, values: &opt_dip_values, schema: &opt_dip_schema, opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_diffopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_diffopt), def_val: cstr_def_val!(c"internal,filler,closeoff,indent-heuristic,inline:char,linematch:40"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"digraph", shortname: Some(c"dg"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_dg).cast(), flags_var: ptr::null_mut(), scope_idx: [44, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"directory", shortname: Some(c"dir"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDefExp|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_dir).cast(), flags_var: ptr::null_mut(), scope_idx: [45, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"display", shortname: Some(c"dy"), flags: kOptFlagOneComma|kOptFlagRedrAll|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_dy).cast(), flags_var: &raw mut dy_flags, scope_idx: [46, -1, -1, -1], immutable: false, values: &opt_dy_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_display), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"lastline"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"eadirection", shortname: Some(c"ead"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ead).cast(), flags_var: ptr::null_mut(), scope_idx: [47, -1, -1, -1], immutable: false, values: &opt_ead_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"both"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"edcompatible", shortname: Some(c"ed"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[78].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [48, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"emoji", shortname: Some(c"emo"), flags: kOptFlagRedrAll|kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_emoji).cast(), flags_var: ptr::null_mut(), scope_idx: [49, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_emoji), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"encoding", shortname: Some(c"enc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_enc).cast(), flags_var: ptr::null_mut(), scope_idx: [50, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: None, def_val: cstr_def_val!(c"utf-8"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"endoffile", shortname: Some(c"eof"), flags: kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_eof).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 26, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"endofline", shortname: Some(c"eol"), flags: kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_eol).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 27, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"equalalways", shortname: Some(c"ea"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ea).cast(), flags_var: ptr::null_mut(), scope_idx: [51, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_equalalways), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"equalprg", shortname: Some(c"ep"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_ep).cast(), flags_var: ptr::null_mut(), scope_idx: [52, -1, 28, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"errorbells", shortname: Some(c"eb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_eb).cast(), flags_var: ptr::null_mut(), scope_idx: [53, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"errorfile", shortname: Some(c"ef"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ef).cast(), flags_var: ptr::null_mut(), scope_idx: [54, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"errors.err"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"errorformat", shortname: Some(c"efm"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_efm).cast(), flags_var: ptr::null_mut(), scope_idx: [55, -1, 29, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_EFM_CSTR), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"eventignore", shortname: Some(c"ei"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ei).cast(), flags_var: ptr::null_mut(), scope_idx: [56, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"eventignorewin", shortname: Some(c"eiw"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 11, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_eventignore), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_eventignore), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"expandtab", shortname: Some(c"et"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_et).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 30, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"exrc", shortname: Some(c"ex"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_exrc).cast(), flags_var: ptr::null_mut(), scope_idx: [57, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"fileencoding", shortname: Some(c"fenc"), flags: kOptFlagRedrStat|kOptFlagRedrBuf|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_fenc).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 31, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"fileencodings", shortname: Some(c"fencs"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fencs).cast(), flags_var: ptr::null_mut(), scope_idx: [58, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c"ucs-bom,utf-8,default,latin1"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"fileformat", shortname: Some(c"ff"), flags: kOptFlagCurswant|kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ff).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 32, -1], immutable: false, values: &opt_ff_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"fileformat", shortname: Some(c"ff"), flags: kOptFlagCurswant|kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ff).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 32, -1], immutable: false, values: &opt_ff_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_fileformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_fileformat), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"fileformats", shortname: Some(c"ffs"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ffs).cast(), flags_var: ptr::null_mut(), scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"dos,unix"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"fileformats", shortname: Some(c"ffs"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ffs).cast(), flags_var: ptr::null_mut(), scope_idx: [59, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"unix,dos"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(any(windows, target_os = "macos"))]
    vimoption_T { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fic).cast(), flags_var: ptr::null_mut(), scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(any(windows, target_os = "macos")))]
    vimoption_T { fullname: c"fileignorecase", shortname: Some(c"fic"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fic).cast(), flags_var: ptr::null_mut(), scope_idx: [60, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"filetype", shortname: Some(c"ft"), flags: kOptFlagNoGlob|kOptFlagNFname, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ft).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 33, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"fillchars", shortname: Some(c"fcs"), flags: kOptFlagOneComma|kOptFlagColon|kOptFlagRedrWin|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_fcs).cast(), flags_var: ptr::null_mut(), scope_idx: [61, 12, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"findfunc", shortname: Some(c"ffu"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_ffu).cast(), flags_var: ptr::null_mut(), scope_idx: [62, -1, 34, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"fixendofline", shortname: Some(c"fixeol"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_fixeol).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 35, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_eof_eol_fixeol_bomb), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldclose", shortname: Some(c"fcl"), flags: kOptFlagOneComma|kOptFlagRedrWin|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fcl).cast(), flags_var: ptr::null_mut(), scope_idx: [63, -1, -1, -1], immutable: false, values: &opt_fcl_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldcolumn", shortname: Some(c"fdc"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 13, -1, -1], immutable: false, values: &opt_fdc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"0"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldenable", shortname: Some(c"fen"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 14, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldenable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldexpr", shortname: Some(c"fde"), flags: kOptFlagRedrWin|kOptFlagExpr|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 15, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldexpr), opt_expand_cb: None, def_val: cstr_def_val!(c"0"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldignore", shortname: Some(c"fdi"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 16, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldignore), opt_expand_cb: None, def_val: cstr_def_val!(c"#"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldlevel", shortname: Some(c"fdl"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 17, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldlevel), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldlevelstart", shortname: Some(c"fdls"), flags: kOptFlagCurswant, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fdls).cast(), flags_var: ptr::null_mut(), scope_idx: [64, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldmarker", shortname: Some(c"fmr"), flags: kOptFlagOneComma|kOptFlagRedrWin|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 18, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_foldmarker), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmarker), opt_expand_cb: None, def_val: cstr_def_val!(c"{{{,}}}"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldmethod", shortname: Some(c"fdm"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 19, -1, -1], immutable: false, values: &opt_fdm_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_foldmethod), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"manual"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldminlines", shortname: Some(c"fml"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 20, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldminlines), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldnestmax", shortname: Some(c"fdn"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 21, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_foldnestmax), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldopen", shortname: Some(c"fdo"), flags: kOptFlagOneComma|kOptFlagCurswant|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_fdo).cast(), flags_var: &raw mut fdo_flags, scope_idx: [65, -1, -1, -1], immutable: false, values: &opt_fdo_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"block,hor,mark,percent,quickfix,search,tag,undo"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"foldtext", shortname: Some(c"fdt"), flags: kOptFlagRedrWin|kOptFlagExpr|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 22, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"foldtext()"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"follow", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_follow).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 36, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_follow), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"formatexpr", shortname: Some(c"fex"), flags: kOptFlagExpr|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_fex).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 37, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"formatlistpat", shortname: Some(c"flp"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_flp).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 38, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"^\\s*\\d\\+[\\]:.)}\\t ]\\s*"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"formatoptions", shortname: Some(c"fo"), flags: kOptFlagFlagList, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_fo).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 39, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_formatoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_formatoptions), def_val: cstr_def_val!(c"tcqj"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"formatprg", shortname: Some(c"fp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_fp).cast(), flags_var: ptr::null_mut(), scope_idx: [66, -1, 40, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"fsync", shortname: Some(c"fs"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_fs).cast(), flags_var: ptr::null_mut(), scope_idx: [67, -1, 41, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"gdefault", shortname: Some(c"gd"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_gd).cast(), flags_var: ptr::null_mut(), scope_idx: [68, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"grepformat", shortname: Some(c"gfm"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_gefm).cast(), flags_var: ptr::null_mut(), scope_idx: [69, -1, 42, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"%f:%l:%m,%f:%l%m,%f  %l%m"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"grepprg", shortname: Some(c"gp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_gp).cast(), flags_var: ptr::null_mut(), scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"findstr /n $* nul"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"grepprg", shortname: Some(c"gp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_gp).cast(), flags_var: ptr::null_mut(), scope_idx: [70, -1, 43, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"grep -HIn $* /dev/null"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guicursor", shortname: Some(c"gcr"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_guicursor).cast(), flags_var: ptr::null_mut(), scope_idx: [71, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_guicursor), opt_expand_cb: None, def_val: cstr_def_val!(c"n-v-c-sm:block,i-ci-ve:ver25,r-cr-o:hor20,t:block-blinkon500-blinkoff500-TermCursor"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guifont", shortname: Some(c"gfn"), flags: kOptFlagOneComma|kOptFlagUIOption|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_guifont).cast(), flags_var: ptr::null_mut(), scope_idx: [72, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(DFLT_GFN_CSTR), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guifontwide", shortname: Some(c"gfw"), flags: kOptFlagOneComma|kOptFlagUIOption|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_guifontwide).cast(), flags_var: ptr::null_mut(), scope_idx: [73, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guioptions", shortname: Some(c"go"), flags: kOptFlagFlagList, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[126].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [74, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guitablabel", shortname: Some(c"gtl"), flags: kOptFlagRedrWin|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[127].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [75, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"guitabtooltip", shortname: Some(c"gtt"), flags: kOptFlagRedrWin|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[128].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [76, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"helpfile", shortname: Some(c"hf"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hf).cast(), flags_var: ptr::null_mut(), scope_idx: [77, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_helpfile), opt_expand_cb: None, def_val: cstr_def_val!(c"$VIMRUNTIME/doc/help.txt"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"helpheight", shortname: Some(c"hh"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hh).cast(), flags_var: ptr::null_mut(), scope_idx: [78, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_helpheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"helplang", shortname: Some(c"hlg"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hlg).cast(), flags_var: ptr::null_mut(), scope_idx: [79, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_helplang), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"hidden", shortname: Some(c"hid"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hid).cast(), flags_var: ptr::null_mut(), scope_idx: [80, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"highlight", shortname: Some(c"hl"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hl).cast(), flags_var: ptr::null_mut(), scope_idx: [81, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_highlight), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(HIGHLIGHT_INIT_CSTR), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"history", shortname: Some(c"hi"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hi).cast(), flags_var: ptr::null_mut(), scope_idx: [82, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"hkmap", shortname: Some(c"hk"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[135].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [83, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"hkmapp", shortname: Some(c"hkp"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[136].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [84, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"hlsearch", shortname: Some(c"hls"), flags: kOptFlagRedrAll|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_hls).cast(), flags_var: ptr::null_mut(), scope_idx: [85, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_hlsearch), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"icon", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_icon).cast(), flags_var: ptr::null_mut(), scope_idx: [86, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"iconstring", shortname: None, flags: kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_iconstring).cast(), flags_var: ptr::null_mut(), scope_idx: [87, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iconstring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ignorecase", shortname: Some(c"ic"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ic).cast(), flags_var: ptr::null_mut(), scope_idx: [88, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_ignorecase), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"imcmdline", shortname: Some(c"imc"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[141].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [89, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"imdisable", shortname: Some(c"imd"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[142].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [90, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"iminsert", shortname: Some(c"imi"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_iminsert).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 44, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_iminsert), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"imsearch", shortname: Some(c"ims"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_imsearch).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 45, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"inccommand", shortname: Some(c"icm"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_icm).cast(), flags_var: ptr::null_mut(), scope_idx: [91, -1, -1, -1], immutable: false, values: &opt_icm_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_inccommand), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"nosplit"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"include", shortname: Some(c"inc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_inc).cast(), flags_var: ptr::null_mut(), scope_idx: [92, -1, 46, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"includeexpr", shortname: Some(c"inex"), flags: kOptFlagExpr|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_inex).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 47, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"incsearch", shortname: Some(c"is"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_is).cast(), flags_var: ptr::null_mut(), scope_idx: [93, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"indentexpr", shortname: Some(c"inde"), flags: kOptFlagExpr|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_inde).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 48, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"indentkeys", shortname: Some(c"indk"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_indk).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 49, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"0{,0},0),0],:,0#,!^F,o,O,e"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"infercase", shortname: Some(c"inf"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_inf).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 50, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"insertmode", shortname: Some(c"im"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[152].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [94, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"isfname", shortname: Some(c"isf"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_isf).cast(), flags_var: ptr::null_mut(), scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,\\,.,-,_,+,,,#,$,%,{,},[,],@-@,!,~,="), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"isfname", shortname: Some(c"isf"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_isf).cast(), flags_var: ptr::null_mut(), scope_idx: [95, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,/,.,-,_,+,,,#,$,%,~,="), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"isident", shortname: Some(c"isi"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_isi).cast(), flags_var: ptr::null_mut(), scope_idx: [96, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_isopt), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"iskeyword", shortname: Some(c"isk"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_isk).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 51, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_isopt), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_iskeyword), opt_expand_cb: None, def_val: cstr_def_val!(c"@,48-57,_,192-255"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"isprint", shortname: Some(c"isp"), flags: kOptFlagComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[156].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [97, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"@,161-255"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"joinspaces", shortname: Some(c"js"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_js).cast(), flags_var: ptr::null_mut(), scope_idx: [98, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"jumpoptions", shortname: Some(c"jop"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_jop).cast(), flags_var: &raw mut jop_flags, scope_idx: [99, -1, -1, -1], immutable: false, values: &opt_jop_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"clean"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"keymap", shortname: Some(c"kmp"), flags: kOptFlagRedrStat|kOptFlagRedrBuf|kOptFlagNFname|kOptFlagPriMkrc, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_keymap).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 52, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"keymodel", shortname: Some(c"km"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_km).cast(), flags_var: ptr::null_mut(), scope_idx: [100, -1, -1, -1], immutable: false, values: &opt_km_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_keymodel), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"keywordprg", shortname: Some(c"kp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_kp).cast(), flags_var: ptr::null_mut(), scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":help!"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"keywordprg", shortname: Some(c"kp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_kp).cast(), flags_var: ptr::null_mut(), scope_idx: [101, -1, 53, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c":Man"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"langmap", shortname: Some(c"lmap"), flags: kOptFlagOneComma|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_langmap).cast(), flags_var: ptr::null_mut(), scope_idx: [102, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::mapping::did_set_langmap), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"langmenu", shortname: Some(c"lm"), flags: kOptFlagNFname, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lm).cast(), flags_var: ptr::null_mut(), scope_idx: [103, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"langnoremap", shortname: Some(c"lnr"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lnr).cast(), flags_var: ptr::null_mut(), scope_idx: [104, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langnoremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"langremap", shortname: Some(c"lrm"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lrm).cast(), flags_var: ptr::null_mut(), scope_idx: [105, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_langremap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"laststatus", shortname: Some(c"ls"), flags: kOptFlagRedrAll, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ls).cast(), flags_var: ptr::null_mut(), scope_idx: [106, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_laststatus), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lazyredraw", shortname: Some(c"lz"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lz).cast(), flags_var: ptr::null_mut(), scope_idx: [107, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lhistory", shortname: Some(c"lhi"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 23, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_xhistory), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(10) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"linebreak", shortname: Some(c"lbr"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 24, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lines", shortname: None, flags: kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lines).cast(), flags_var: ptr::null_mut(), scope_idx: [108, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lines_or_columns), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(24) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"linespace", shortname: Some(c"lsp"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_linespace).cast(), flags_var: ptr::null_mut(), scope_idx: [109, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lisp", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_lisp).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 54, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_lisp), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lispoptions", shortname: Some(c"lop"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_lop).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 55, -1], immutable: false, values: &opt_lop_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_lispoptions), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"lispwords", shortname: Some(c"lw"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_lispwords).cast(), flags_var: ptr::null_mut(), scope_idx: [110, -1, 56, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(LISPWORD_VALUE_CSTR), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"list", shortname: None, flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 25, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"listchars", shortname: Some(c"lcs"), flags: kOptFlagOneComma|kOptFlagColon|kOptFlagRedrWin|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_lcs).cast(), flags_var: ptr::null_mut(), scope_idx: [111, 26, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_chars_option), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_chars_option), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_chars_option), def_val: cstr_def_val!(c"tab:> ,trail:-,nbsp:+"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"loadplugins", shortname: Some(c"lpl"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_lpl).cast(), flags_var: ptr::null_mut(), scope_idx: [112, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"magic", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_magic).cast(), flags_var: ptr::null_mut(), scope_idx: [113, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"makeef", shortname: Some(c"mef"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mef).cast(), flags_var: ptr::null_mut(), scope_idx: [114, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"makeencoding", shortname: Some(c"menc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_menc).cast(), flags_var: ptr::null_mut(), scope_idx: [115, -1, 57, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_encoding), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_encoding), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_encoding), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"makeprg", shortname: Some(c"mp"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_mp).cast(), flags_var: ptr::null_mut(), scope_idx: [116, -1, 58, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"make"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"matchpairs", shortname: Some(c"mps"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_mps).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 59, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_matchpairs), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"(:),{:},[:]"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"matchtime", shortname: Some(c"mat"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mat).cast(), flags_var: ptr::null_mut(), scope_idx: [117, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"maxcombine", shortname: Some(c"mco"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mco).cast(), flags_var: ptr::null_mut(), scope_idx: [118, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(6) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"maxfuncdepth", shortname: Some(c"mfd"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mfd).cast(), flags_var: ptr::null_mut(), scope_idx: [119, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(100) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"maxmapdepth", shortname: Some(c"mmd"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mmd).cast(), flags_var: ptr::null_mut(), scope_idx: [120, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"maxmempattern", shortname: Some(c"mmp"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mmp).cast(), flags_var: ptr::null_mut(), scope_idx: [121, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"maxsearchcount", shortname: Some(c"msc"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_msc).cast(), flags_var: ptr::null_mut(), scope_idx: [122, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(999) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"menuitems", shortname: Some(c"mis"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mis).cast(), flags_var: ptr::null_mut(), scope_idx: [123, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(25) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"messagesopt", shortname: Some(c"mopt"), flags: kOptFlagOneComma|kOptFlagColon|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mopt).cast(), flags_var: ptr::null_mut(), scope_idx: [124, -1, -1, -1], immutable: false, values: &opt_mopt_values, schema: &opt_mopt_schema, opt_validate_cb: opt_cb!(crate::message::validate_messagesopt), opt_did_set_cb: opt_cb!(crate::message::did_set_messagesopt), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"hit-enter,history:500,progress:c"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mkspellmem", shortname: Some(c"msm"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_msm).cast(), flags_var: ptr::null_mut(), scope_idx: [125, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mkspellmem), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"460000,2000,500"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"modeline", shortname: Some(c"ml"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ml).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 60, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"modelineexpr", shortname: Some(c"mle"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mle).cast(), flags_var: ptr::null_mut(), scope_idx: [126, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"modelines", shortname: Some(c"mls"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mls).cast(), flags_var: ptr::null_mut(), scope_idx: [127, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(5) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"modifiable", shortname: Some(c"ma"), flags: kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ma).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 61, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modifiable), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"modified", shortname: Some(c"mod"), flags: kOptFlagRedrStat|kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_mod).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 62, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_modified), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"more", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_more).cast(), flags_var: ptr::null_mut(), scope_idx: [128, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mouse", shortname: None, flags: kOptFlagFlagList, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mouse).cast(), flags_var: ptr::null_mut(), scope_idx: [129, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_mouse), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_mouse), def_val: cstr_def_val!(c"nvi"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousefocus", shortname: Some(c"mousef"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mousef).cast(), flags_var: ptr::null_mut(), scope_idx: [130, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousehide", shortname: Some(c"mh"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mh).cast(), flags_var: ptr::null_mut(), scope_idx: [131, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousemodel", shortname: Some(c"mousem"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mousem).cast(), flags_var: ptr::null_mut(), scope_idx: [132, -1, -1, -1], immutable: false, values: &opt_mousem_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"popup_setpos"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousemoveevent", shortname: Some(c"mousemev"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mousemev).cast(), flags_var: ptr::null_mut(), scope_idx: [133, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousescroll", shortname: None, flags: kOptFlagComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mousescroll).cast(), flags_var: ptr::null_mut(), scope_idx: [134, -1, -1, -1], immutable: false, values: &opt_mousescroll_values, schema: &opt_mousescroll_schema, opt_validate_cb: opt_cb!(crate::optionstr::validate_mousescroll), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_mousescroll), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver:3,hor:6"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mouseshape", shortname: Some(c"mouses"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[204].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [135, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"mousetime", shortname: Some(c"mouset"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_mouset).cast(), flags_var: ptr::null_mut(), scope_idx: [136, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(500) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"nrformats", shortname: Some(c"nf"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_nf).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 63, -1], immutable: false, values: &opt_nf_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"bin,hex"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"number", shortname: Some(c"nu"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 27, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"numberwidth", shortname: Some(c"nuw"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 28, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_numberwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(4) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"omnifunc", shortname: Some(c"ofu"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ofu).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 64, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"opendevice", shortname: Some(c"odev"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[210].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [137, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"operatorfunc", shortname: Some(c"opfunc"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_opfunc).cast(), flags_var: ptr::null_mut(), scope_idx: [138, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"packlockfile", shortname: Some(c"plf"), flags: kOptFlagExpand|kOptFlagNoDefExp|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_plf).cast(), flags_var: ptr::null_mut(), scope_idx: [139, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"$XDG_CONFIG_HOME/nvim/nvim-pack-lock.json"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"packpath", shortname: Some(c"pp"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pp).cast(), flags_var: ptr::null_mut(), scope_idx: [140, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"paragraphs", shortname: Some(c"para"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_para).cast(), flags_var: ptr::null_mut(), scope_idx: [141, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"IPLPPPQPP TPHPLIPpLpItpplpipbp"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"paste", shortname: None, flags: kOptFlagPriMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_paste).cast(), flags_var: ptr::null_mut(), scope_idx: [142, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_paste), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pastetoggle", shortname: Some(c"pt"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[216].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [143, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"patchexpr", shortname: Some(c"pex"), flags: kOptFlagExpr|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pex).cast(), flags_var: ptr::null_mut(), scope_idx: [144, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"patchmode", shortname: Some(c"pm"), flags: kOptFlagNFname, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pm).cast(), flags_var: ptr::null_mut(), scope_idx: [145, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_backupext_or_patchmode), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"path", shortname: Some(c"pa"), flags: kOptFlagComma|kOptFlagExpand|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_path).cast(), flags_var: ptr::null_mut(), scope_idx: [146, -1, 65, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".,,"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"preserveindent", shortname: Some(c"pi"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_pi).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 66, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"previewheight", shortname: Some(c"pvh"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pvh).cast(), flags_var: ptr::null_mut(), scope_idx: [147, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(12) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"previewpopup", shortname: Some(c"pvp"), flags: kOptFlagComma|kOptFlagColon, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pvp).cast(), flags_var: ptr::null_mut(), scope_idx: [148, -1, -1, -1], immutable: false, values: &opt_pvp_values, schema: &opt_pvp_schema, opt_validate_cb: opt_cb!(crate::optionstr::validate_previewpopup), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_popupoption), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"previewwindow", shortname: Some(c"pvw"), flags: kOptFlagRedrStat|kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 29, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_previewwindow), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"prompt", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[224].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [149, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pumblend", shortname: Some(c"pb"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pb).cast(), flags_var: ptr::null_mut(), scope_idx: [150, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_pumblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pumborder", shortname: None, flags: kOptFlagOneComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pumborder).cast(), flags_var: ptr::null_mut(), scope_idx: [151, -1, -1, -1], immutable: false, values: &opt_pumborder_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pumheight", shortname: Some(c"ph"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ph).cast(), flags_var: ptr::null_mut(), scope_idx: [152, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pummaxwidth", shortname: Some(c"pmw"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pmw).cast(), flags_var: ptr::null_mut(), scope_idx: [153, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pumwidth", shortname: Some(c"pw"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pw).cast(), flags_var: ptr::null_mut(), scope_idx: [154, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(15) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"pyxversion", shortname: Some(c"pyx"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_pyx).cast(), flags_var: ptr::null_mut(), scope_idx: [155, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(3) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"quickfixtextfunc", shortname: Some(c"qftf"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_qftf).cast(), flags_var: ptr::null_mut(), scope_idx: [156, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"quoteescape", shortname: Some(c"qe"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_qe).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 67, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"\\"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"readonly", shortname: Some(c"ro"), flags: kOptFlagRedrStat|kOptFlagNoGlob, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ro).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 68, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_readonly), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"redrawdebug", shortname: Some(c"rdb"), flags: kOptFlagOneComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_rdb).cast(), flags_var: &raw mut rdb_flags, scope_idx: [157, -1, -1, -1], immutable: false, values: &opt_rdb_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"redrawtime", shortname: Some(c"rdt"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_rdt).cast(), flags_var: ptr::null_mut(), scope_idx: [158, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(2000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"regexpengine", shortname: Some(c"re"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_re).cast(), flags_var: ptr::null_mut(), scope_idx: [159, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"relativenumber", shortname: Some(c"rnu"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 30, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_number_relativenumber), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"remap", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[238].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [160, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"report", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_report).cast(), flags_var: ptr::null_mut(), scope_idx: [161, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(2) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"revins", shortname: Some(c"ri"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ri).cast(), flags_var: ptr::null_mut(), scope_idx: [162, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"rightleft", shortname: Some(c"rl"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 31, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"rightleftcmd", shortname: Some(c"rlc"), flags: kOptFlagComma|kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 32, -1, -1], immutable: false, values: &opt_rlc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"search"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ruler", shortname: Some(c"ru"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ru).cast(), flags_var: ptr::null_mut(), scope_idx: [163, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"rulerformat", shortname: Some(c"ruf"), flags: kOptFlagRedrStat|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ruf).cast(), flags_var: ptr::null_mut(), scope_idx: [164, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_rulerformat), opt_expand_cb: None, def_val: cstr_def_val!(c"%18(%l,%c%V%= %P%)%<"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"runtimepath", shortname: Some(c"rtp"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDefExp|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_rtp).cast(), flags_var: ptr::null_mut(), scope_idx: [165, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::runtime::did_set_runtimepackpath), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scroll", shortname: Some(c"scr"), flags: kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 33, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrollback", shortname: Some(c"scbk"), flags: kOptFlagRedrBuf, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_scbk).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 69, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollback), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(-1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrollbind", shortname: Some(c"scb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 34, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_scrollbind), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrolljump", shortname: Some(c"sj"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sj).cast(), flags_var: ptr::null_mut(), scope_idx: [166, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrolloff", shortname: Some(c"so"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_so).cast(), flags_var: ptr::null_mut(), scope_idx: [167, 35, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrolloffpad", shortname: Some(c"sop"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_sop).cast(), flags_var: ptr::null_mut(), scope_idx: [168, 36, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"scrollopt", shortname: Some(c"sbo"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sbo).cast(), flags_var: ptr::null_mut(), scope_idx: [169, -1, -1, -1], immutable: false, values: &opt_sbo_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"ver,jump"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"sections", shortname: Some(c"sect"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sections).cast(), flags_var: ptr::null_mut(), scope_idx: [170, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"SHNHH HUnhsh"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"secure", shortname: None, flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_secure).cast(), flags_var: ptr::null_mut(), scope_idx: [171, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"selection", shortname: Some(c"sel"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sel).cast(), flags_var: ptr::null_mut(), scope_idx: [172, -1, -1, -1], immutable: false, values: &opt_sel_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_selection), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"inclusive"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"selectmode", shortname: Some(c"slm"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_slm).cast(), flags_var: ptr::null_mut(), scope_idx: [173, -1, -1, -1], immutable: false, values: &opt_slm_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"sessionoptions", shortname: Some(c"ssop"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ssop).cast(), flags_var: &raw mut ssop_flags, scope_idx: [174, -1, -1, -1], immutable: false, values: &opt_ssop_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_sessionoptions), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"blank,buffers,curdir,folds,help,tabpages,winsize,terminal"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shada", shortname: Some(c"sd"), flags: kOptFlagOneComma|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_shada).cast(), flags_var: ptr::null_mut(), scope_idx: [175, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shada), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"!,'100,<50,s10,h,r/tmp/,r/private/"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shadafile", shortname: Some(c"sdf"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_shadafile).cast(), flags_var: ptr::null_mut(), scope_idx: [176, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"shell", shortname: Some(c"sh"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sh).cast(), flags_var: ptr::null_mut(), scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"cmd.exe"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"shell", shortname: Some(c"sh"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sh).cast(), flags_var: ptr::null_mut(), scope_idx: [177, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"sh"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shellcmdflag", shortname: Some(c"shcf"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_shcf).cast(), flags_var: ptr::null_mut(), scope_idx: [178, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"-c"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"shellpipe", shortname: Some(c"sp"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sp).cast(), flags_var: ptr::null_mut(), scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"2>&1| tee"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"shellpipe", shortname: Some(c"sp"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sp).cast(), flags_var: ptr::null_mut(), scope_idx: [179, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"| tee"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shellquote", shortname: Some(c"shq"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_shq).cast(), flags_var: ptr::null_mut(), scope_idx: [180, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"shellredir", shortname: Some(c"srr"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_srr).cast(), flags_var: ptr::null_mut(), scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">%s 2>&1"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"shellredir", shortname: Some(c"srr"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_srr).cast(), flags_var: ptr::null_mut(), scope_idx: [181, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shellpipe_redir), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c">"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(windows)]
    vimoption_T { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ssl).cast(), flags_var: ptr::null_mut(), scope_idx: [182, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    #[cfg(not(windows))]
    vimoption_T { fullname: c"shellslash", shortname: Some(c"ssl"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[265].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [182, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shelltemp", shortname: Some(c"stmp"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_stmp).cast(), flags_var: ptr::null_mut(), scope_idx: [183, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shellxescape", shortname: Some(c"sxe"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sxe).cast(), flags_var: ptr::null_mut(), scope_idx: [184, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shellxquote", shortname: Some(c"sxq"), flags: kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sxq).cast(), flags_var: ptr::null_mut(), scope_idx: [185, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shiftround", shortname: Some(c"sr"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sr).cast(), flags_var: ptr::null_mut(), scope_idx: [186, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shiftwidth", shortname: Some(c"sw"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_sw).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 70, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"shortmess", shortname: Some(c"shm"), flags: kOptFlagFlagList, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_shm).cast(), flags_var: ptr::null_mut(), scope_idx: [187, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_shortmess), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_shortmess), def_val: cstr_def_val!(c"ltToOCF"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showbreak", shortname: Some(c"sbr"), flags: kOptFlagRedrAll, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_sbr).cast(), flags_var: ptr::null_mut(), scope_idx: [188, 37, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_showbreak), opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showcmd", shortname: Some(c"sc"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sc).cast(), flags_var: ptr::null_mut(), scope_idx: [189, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showcmdloc", shortname: Some(c"sloc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sloc).cast(), flags_var: ptr::null_mut(), scope_idx: [190, -1, -1, -1], immutable: false, values: &opt_sloc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_showcmdloc), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"last"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showfulltag", shortname: Some(c"sft"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sft).cast(), flags_var: ptr::null_mut(), scope_idx: [191, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showmatch", shortname: Some(c"sm"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sm).cast(), flags_var: ptr::null_mut(), scope_idx: [192, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showmode", shortname: Some(c"smd"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_smd).cast(), flags_var: ptr::null_mut(), scope_idx: [193, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"showtabline", shortname: Some(c"stal"), flags: kOptFlagRedrAll|kOptFlagUIOption, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_stal).cast(), flags_var: ptr::null_mut(), scope_idx: [194, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_showtabline), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"sidescroll", shortname: Some(c"ss"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ss).cast(), flags_var: ptr::null_mut(), scope_idx: [195, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"sidescrolloff", shortname: Some(c"siso"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_siso).cast(), flags_var: ptr::null_mut(), scope_idx: [196, 38, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"signcolumn", shortname: Some(c"scl"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 39, -1, -1], immutable: false, values: &opt_scl_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_signcolumn), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_signcolumn), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"auto"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"smartcase", shortname: Some(c"scs"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_scs).cast(), flags_var: ptr::null_mut(), scope_idx: [197, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"smartindent", shortname: Some(c"si"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_si).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 71, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"smarttab", shortname: Some(c"sta"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sta).cast(), flags_var: ptr::null_mut(), scope_idx: [198, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"smoothscroll", shortname: Some(c"sms"), flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 40, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_smoothscroll), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"softtabstop", shortname: Some(c"sts"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_sts).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 72, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spell", shortname: None, flags: kOptFlagRedrWin|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 41, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_spell), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spellcapcheck", shortname: Some(c"spc"), flags: kOptFlagRedrBuf|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_spc).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 73, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellcapcheck), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellcapcheck), opt_expand_cb: None, def_val: cstr_def_val!(c"[.?!]\\_[\\])'\"\\t ]\\+"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spellfile", shortname: Some(c"spf"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_spf).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 74, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellfile), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellfile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spelllang", shortname: Some(c"spl"), flags: kOptFlagOneComma|kOptFlagRedrBuf|kOptFlagHLOnly|kOptFlagExpand|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_spl).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 75, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spelllang), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelllang), opt_expand_cb: None, def_val: cstr_def_val!(c"en"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spelloptions", shortname: Some(c"spo"), flags: kOptFlagOneComma|kOptFlagRedrBuf|kOptFlagHLOnly|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_spo).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 76, -1], immutable: false, values: &opt_spo_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spelloptions), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"spellsuggest", shortname: Some(c"sps"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sps).cast(), flags_var: ptr::null_mut(), scope_idx: [199, -1, -1, -1], immutable: false, values: &opt_sps_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_spellsuggest), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_spellsuggest), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"best"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"splitbelow", shortname: Some(c"sb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sb).cast(), flags_var: ptr::null_mut(), scope_idx: [200, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"splitkeep", shortname: Some(c"spk"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_spk).cast(), flags_var: ptr::null_mut(), scope_idx: [201, -1, -1, -1], immutable: false, values: &opt_spk_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_splitkeep), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"cursor"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"splitright", shortname: Some(c"spr"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_spr).cast(), flags_var: ptr::null_mut(), scope_idx: [202, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"startofline", shortname: Some(c"sol"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_sol).cast(), flags_var: ptr::null_mut(), scope_idx: [203, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"statuscolumn", shortname: Some(c"stc"), flags: kOptFlagRedrWin|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 42, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statuscolumn), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"statusline", shortname: Some(c"stl"), flags: kOptFlagRedrStat|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_stl).cast(), flags_var: ptr::null_mut(), scope_idx: [204, 43, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_statusline), opt_expand_cb: None, def_val: cstr_def_val!(c"%<%f%( %h%w%m%r%{ v:lua.require('vim._core.util').term_exitcode() }%)%= %(%-10S %)%{ &busy > 0 ? '◐ ' : '' }%(%{ luaeval('(package.loaded[''vim.ui''] and vim.api.nvim_get_current_win() == tonumber(vim.g.actual_curwin or -1) and vim.ui.progress_status()) or '''' ')} %)%{% luaeval('(package.loaded[''vim.diagnostic''] and next(vim.diagnostic.count(0)) and vim.diagnostic.status() .. '' '') or '''' ') %}%(%k %)%{% &ruler ? &rulerformat : '' %}"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"suffixes", shortname: Some(c"su"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_su).cast(), flags_var: ptr::null_mut(), scope_idx: [205, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c".bak,~,.o,.h,.info,.swp,.obj"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"suffixesadd", shortname: Some(c"sua"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_sua).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 77, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"swapfile", shortname: Some(c"swf"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_swf).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 78, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_swapfile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"switchbuf", shortname: Some(c"swb"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_swb).cast(), flags_var: &raw mut swb_flags, scope_idx: [206, -1, -1, -1], immutable: false, values: &opt_swb_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"uselast"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"synmaxcol", shortname: Some(c"smc"), flags: kOptFlagRedrBuf, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_smc).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 79, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(3000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"syntax", shortname: Some(c"syn"), flags: kOptFlagNoGlob|kOptFlagNFname, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_syn).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 80, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_filetype), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_filetype_or_syntax), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tabclose", shortname: Some(c"tcl"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tcl).cast(), flags_var: &raw mut tcl_flags, scope_idx: [207, -1, -1, -1], immutable: false, values: &opt_tcl_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tabline", shortname: Some(c"tal"), flags: kOptFlagRedrTabl|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tal).cast(), flags_var: ptr::null_mut(), scope_idx: [208, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tabline), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tabpagemax", shortname: Some(c"tpm"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tpm).cast(), flags_var: ptr::null_mut(), scope_idx: [209, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tabstop", shortname: Some(c"ts"), flags: kOptFlagRedrBuf, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_ts).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 81, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_shiftwidth_tabstop), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(8) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tagbsearch", shortname: Some(c"tbs"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tbs).cast(), flags_var: ptr::null_mut(), scope_idx: [210, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tagcase", shortname: Some(c"tc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_tc).cast(), flags_var: &raw mut tc_flags, scope_idx: [211, -1, 82, -1], immutable: false, values: &opt_tc_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_tagcase), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"followic"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tagfunc", shortname: Some(c"tfu"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_tfu).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 83, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"taglength", shortname: Some(c"tl"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tl).cast(), flags_var: ptr::null_mut(), scope_idx: [212, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tagrelative", shortname: Some(c"tr"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tr).cast(), flags_var: ptr::null_mut(), scope_idx: [213, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tags", shortname: Some(c"tag"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_tags).cast(), flags_var: ptr::null_mut(), scope_idx: [214, -1, 84, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c"./tags;,tags"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tagstack", shortname: Some(c"tgst"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tgst).cast(), flags_var: ptr::null_mut(), scope_idx: [215, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"termbidi", shortname: Some(c"tbidi"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tbidi).cast(), flags_var: ptr::null_mut(), scope_idx: [216, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"termencoding", shortname: Some(c"tenc"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[317].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [217, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"termguicolors", shortname: Some(c"tgc"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tgc).cast(), flags_var: ptr::null_mut(), scope_idx: [218, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"termpastefilter", shortname: Some(c"tpf"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tpf).cast(), flags_var: &raw mut tpf_flags, scope_idx: [219, -1, -1, -1], immutable: false, values: &opt_tpf_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"BS,ESC,DEL"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"termsync", shortname: None, flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_termsync).cast(), flags_var: ptr::null_mut(), scope_idx: [220, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"terse", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: unsafe { (&raw mut OPTIONS[321].def_val).cast() }, flags_var: ptr::null_mut(), scope_idx: [221, -1, -1, -1], immutable: true, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"textwidth", shortname: Some(c"tw"), flags: kOptFlagRedrBuf|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_tw).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 85, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_textwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"thesaurus", shortname: Some(c"tsr"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDup|kOptFlagNDname, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_tsr).cast(), flags_var: ptr::null_mut(), scope_idx: [222, -1, 86, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"thesaurusfunc", shortname: Some(c"tsrfu"), flags: kOptFlagFunc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_tsrfu).cast(), flags_var: ptr::null_mut(), scope_idx: [223, -1, 87, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"tildeop", shortname: Some(c"top"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_to).cast(), flags_var: ptr::null_mut(), scope_idx: [224, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"timeout", shortname: Some(c"to"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_timeout).cast(), flags_var: ptr::null_mut(), scope_idx: [225, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"timeoutlen", shortname: Some(c"tm"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tm).cast(), flags_var: ptr::null_mut(), scope_idx: [226, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"title", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_title).cast(), flags_var: ptr::null_mut(), scope_idx: [227, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_title_icon), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"titlelen", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_titlelen).cast(), flags_var: ptr::null_mut(), scope_idx: [228, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_titlelen), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(85) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"titleold", shortname: None, flags: kOptFlagNoMkrc|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_titleold).cast(), flags_var: ptr::null_mut(), scope_idx: [229, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"titlestring", shortname: None, flags: kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_titlestring).cast(), flags_var: ptr::null_mut(), scope_idx: [230, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_titlestring), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ttimeout", shortname: None, flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ttimeout).cast(), flags_var: ptr::null_mut(), scope_idx: [231, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ttimeoutlen", shortname: Some(c"ttm"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ttm).cast(), flags_var: ptr::null_mut(), scope_idx: [232, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(50) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"ttyfast", shortname: Some(c"tf"), flags: kOptFlagNoMkrc, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_tf).cast(), flags_var: ptr::null_mut(), scope_idx: [233, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"undodir", shortname: Some(c"udir"), flags: kOptFlagOneComma|kOptFlagExpand|kOptFlagNoDefExp|kOptFlagNoDup|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_udir).cast(), flags_var: ptr::null_mut(), scope_idx: [234, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"undofile", shortname: Some(c"udf"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_udf).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 88, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undofile), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"undolevels", shortname: Some(c"ul"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeBuf)) as u8, var: (&raw mut p_ul).cast(), flags_var: ptr::null_mut(), scope_idx: [235, -1, 89, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_undolevels), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"undoreload", shortname: Some(c"ur"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ur).cast(), flags_var: ptr::null_mut(), scope_idx: [236, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(10000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"updatecount", shortname: Some(c"uc"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_uc).cast(), flags_var: ptr::null_mut(), scope_idx: [237, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_updatecount), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(200) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"updatetime", shortname: Some(c"ut"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ut).cast(), flags_var: ptr::null_mut(), scope_idx: [238, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(4000) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"varsofttabstop", shortname: Some(c"vsts"), flags: kOptFlagComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_vsts).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 90, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"vartabstop", shortname: Some(c"vts"), flags: kOptFlagComma|kOptFlagRedrBuf, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_vts).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 91, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_vartabstop), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_vartabstop), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"verbose", shortname: Some(c"vbs"), flags: kOptFlagUIOption, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_verbose).cast(), flags_var: ptr::null_mut(), scope_idx: [239, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"verbosefile", shortname: Some(c"vfile"), flags: kOptFlagExpand|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_vfile).cast(), flags_var: ptr::null_mut(), scope_idx: [240, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_verbosefile), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"viewdir", shortname: Some(c"vdir"), flags: kOptFlagExpand|kOptFlagNoDefExp|kOptFlagSecure, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_vdir).cast(), flags_var: ptr::null_mut(), scope_idx: [241, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"viewoptions", shortname: Some(c"vop"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_vop).cast(), flags_var: &raw mut vop_flags, scope_idx: [242, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"folds,cursor,curdir"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"virtualedit", shortname: Some(c"ve"), flags: kOptFlagOneComma|kOptFlagCurswant|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_ve).cast(), flags_var: &raw mut ve_flags, scope_idx: [243, 44, -1, -1], immutable: false, values: &opt_ve_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_virtualedit), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"visualbell", shortname: Some(c"vb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_vb).cast(), flags_var: ptr::null_mut(), scope_idx: [244, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"warn", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_warn).cast(), flags_var: ptr::null_mut(), scope_idx: [245, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"whichwrap", shortname: Some(c"ww"), flags: kOptFlagComma|kOptFlagFlagList, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ww).cast(), flags_var: ptr::null_mut(), scope_idx: [246, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_whichwrap), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_whichwrap), def_val: cstr_def_val!(c"b,s"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildchar", shortname: Some(c"wc"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wc).cast(), flags_var: ptr::null_mut(), scope_idx: [247, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(9) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildcharm", shortname: Some(c"wcm"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wcm).cast(), flags_var: ptr::null_mut(), scope_idx: [248, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(validate_wildchar), opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildignore", shortname: Some(c"wig"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wig).cast(), flags_var: ptr::null_mut(), scope_idx: [249, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildignorecase", shortname: Some(c"wic"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wic).cast(), flags_var: ptr::null_mut(), scope_idx: [250, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildmenu", shortname: Some(c"wmnu"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wmnu).cast(), flags_var: ptr::null_mut(), scope_idx: [251, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildmode", shortname: Some(c"wim"), flags: kOptFlagOneComma|kOptFlagColon, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wim).cast(), flags_var: ptr::null_mut(), scope_idx: [252, -1, -1, -1], immutable: false, values: &opt_wim_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_wildmode), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_wildmode), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"full"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wildoptions", shortname: Some(c"wop"), flags: kOptFlagOneComma|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wop).cast(), flags_var: &raw mut wop_flags, scope_idx: [253, -1, -1, -1], immutable: false, values: &opt_wop_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_str_generic), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"pum,tagfile"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winaltkeys", shortname: Some(c"wak"), flags: 0, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wak).cast(), flags_var: ptr::null_mut(), scope_idx: [254, -1, -1, -1], immutable: false, values: &opt_wak_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_str_generic), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c"menu"), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winbar", shortname: Some(c"wbr"), flags: kOptFlagRedrStat|kOptFlagMLE, opt_type: ObjectType::kObjectTypeString, scope_flags: ((1 << kOptScopeGlobal) | (1 << kOptScopeWin)) as u8, var: (&raw mut p_wbr).cast(), flags_var: ptr::null_mut(), scope_idx: [255, 45, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_statustabline_rulerformat), opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winbar), opt_expand_cb: None, def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winblend", shortname: Some(c"winbl"), flags: kOptFlagRedrWin|kOptFlagHLOnly, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 46, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winblend), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winborder", shortname: None, flags: kOptFlagOneComma, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_winborder).cast(), flags_var: ptr::null_mut(), scope_idx: [256, -1, -1, -1], immutable: false, values: &opt_winborder_values, schema: &[], opt_validate_cb: opt_cb!(crate::optionstr::validate_border), opt_did_set_cb: None, opt_expand_cb: opt_cb!(crate::optionstr::expand_set_str_generic), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"window", shortname: Some(c"wi"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_window).cast(), flags_var: ptr::null_mut(), scope_idx: [257, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_window), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winfixbuf", shortname: Some(c"wfb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 47, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winfixheight", shortname: Some(c"wfh"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 48, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winfixwidth", shortname: Some(c"wfw"), flags: kOptFlagRedrStat, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 49, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winheight", shortname: Some(c"wh"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wh).cast(), flags_var: ptr::null_mut(), scope_idx: [258, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winhighlight", shortname: Some(c"winhl"), flags: kOptFlagOneComma|kOptFlagColon|kOptFlagRedrWin|kOptFlagHLOnly|kOptFlagNoDup, opt_type: ObjectType::kObjectTypeString, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 50, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::optionstr::did_set_winhighlight), opt_expand_cb: opt_cb!(crate::optionstr::expand_set_winhighlight), def_val: cstr_def_val!(c""), script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winminheight", shortname: Some(c"wmh"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wmh).cast(), flags_var: ptr::null_mut(), scope_idx: [259, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminheight), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winminwidth", shortname: Some(c"wmw"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wmw).cast(), flags_var: ptr::null_mut(), scope_idx: [260, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(crate::window::did_set_winminwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(1) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winpinned", shortname: Some(c"wp"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 51, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"winwidth", shortname: Some(c"wiw"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wiw).cast(), flags_var: ptr::null_mut(), scope_idx: [261, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_winwidth), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(20) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wrap", shortname: None, flags: kOptFlagRedrWin, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeWin) as u8, var: ptr::null_mut(), flags_var: ptr::null_mut(), scope_idx: [-1, 52, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: opt_cb!(did_set_wrap), opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wrapmargin", shortname: Some(c"wm"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeBuf) as u8, var: (&raw mut p_wm).cast(), flags_var: ptr::null_mut(), scope_idx: [-1, -1, 92, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"wrapscan", shortname: Some(c"ws"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_ws).cast(), flags_var: ptr::null_mut(), scope_idx: [262, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"write", shortname: None, flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_write).cast(), flags_var: ptr::null_mut(), scope_idx: [263, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"writeany", shortname: Some(c"wa"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wa).cast(), flags_var: ptr::null_mut(), scope_idx: [264, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(false) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"writebackup", shortname: Some(c"wb"), flags: 0, opt_type: ObjectType::kObjectTypeBoolean, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wb).cast(), flags_var: ptr::null_mut(), scope_idx: [265, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeBoolean, data: ObjectData::Boolean(true) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
    vimoption_T { fullname: c"writedelay", shortname: Some(c"wd"), flags: 0, opt_type: ObjectType::kObjectTypeInteger, scope_flags: (1 << kOptScopeGlobal) as u8, var: (&raw mut p_wd).cast(), flags_var: ptr::null_mut(), scope_idx: [266, -1, -1, -1], immutable: false, values: &[], schema: &[], opt_validate_cb: None, opt_did_set_cb: None, opt_expand_cb: None, def_val: Object { obj_type: ObjectType::kObjectTypeInteger, data: ObjectData::Integer(0) }, script_ctx: sctx_T { sc_sid: 0, sc_seq: 0, sc_lnum: 0, sc_chan: 0 } },
];

/// Return information for option at `opt_idx` (`get_option`).
pub unsafe fn get_option(opt_idx: OptIndex) -> *mut vimoption_T {
    assert!(opt_idx != kOptInvalid);
    unsafe { (&raw mut OPTIONS[opt_idx as usize]) }
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
pub unsafe fn find_option_len(name: *const c_char, len: usize) -> OptIndex {
    if name.is_null() {
        return kOptInvalid;
    }
    let slice = unsafe { std::slice::from_raw_parts(name.cast::<u8>(), len) };
    match OPTION_NAME_MAP.binary_search_by_key(&slice, |&(k, _)| k) {
        Ok(i) => OPTION_NAME_MAP[i].1,
        Err(_) => kOptInvalid,
    }
}

/// Find index for an option.
pub unsafe fn find_option(name: *const c_char) -> OptIndex {
    unsafe { find_option_len(name, strlen(name)) }
}

/// True if `opt_idx` is a dict option (`schema.dict` in options.lua, e.g. 'diffopt').
pub unsafe fn is_dict_option(opt_idx: OptIndex) -> bool {
    opt_dict_schema(opt_idx).is_some()
}

/// Release an option value read from opt_from_varp() (and similar).
pub unsafe fn optval_free_read(opt_idx: OptIndex, value: Object) {
    unsafe {
        if optval_is_owned(opt_idx) {
            optval_free(value);
        }
    }
}

/// Free an allocated option value.
pub unsafe fn optval_free(o: Object) {
    unsafe {
        match o.obj_type {
            ObjectType::kObjectTypeString => {
                if o.data.string().data != (&raw mut empty_string_option).cast::<c_char>() {
                    api_free_string(o.data.string());
                }
            }
            ObjectType::kObjectTypeLuaRef => {
                api_free_luaref(o.data.luaref());
            }
            ObjectType::kObjectTypeUnset
            | ObjectType::kObjectTypeNil
            | ObjectType::kObjectTypeBoolean
            | ObjectType::kObjectTypeInteger => {}
            _ => std::process::abort(),
        }
    }
}

/// Check if two option values are equal.
pub unsafe fn option_equal(o1: Object, o2: Object) -> bool {
    unsafe {
        if o1.obj_type != o2.obj_type {
            return false;
        }

        match o1.obj_type {
            ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => true,
            ObjectType::kObjectTypeBoolean => o1.data.boolean() == o2.data.boolean(),
            ObjectType::kObjectTypeInteger => o1.data.integer() == o2.data.integer(),
            ObjectType::kObjectTypeString => {
                o1.data.string().size == o2.data.string().size
                    && (o1.data.string().data == o2.data.string().data
                        || strnequal(
                            o1.data.string().data,
                            o2.data.string().data,
                            o1.data.string().size,
                        ))
            }
            ObjectType::kObjectTypeLuaRef => false,
            _ => std::process::abort(),
        }
    }
}

/// Convert a tri-state boolean option to Object.
pub unsafe fn opt_from_tristate(t: TriState) -> Object {
    if t == kNone {
        unset_obj()
    } else {
        boolean_obj(t == kTrue)
    }
}

unsafe fn optval_type(o: Object) -> ObjectType {
    match o.obj_type {
        ObjectType::kObjectTypeNil => ObjectType::kObjectTypeNil,
        ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeBoolean => {
            ObjectType::kObjectTypeBoolean
        }
        ObjectType::kObjectTypeInteger => ObjectType::kObjectTypeInteger,
        ObjectType::kObjectTypeString => ObjectType::kObjectTypeString,
        ObjectType::kObjectTypeLuaRef => ObjectType::kObjectTypeLuaRef,
        _ => std::process::abort(),
    }
}

unsafe fn is_callback_option(opt_idx: OptIndex) -> bool {
    unsafe { ((*get_option(opt_idx)).flags & (kOptFlagFunc | kOptFlagExpr)) != 0 }
}

unsafe fn optval_is_owned(opt_idx: OptIndex) -> bool {
    unsafe { is_callback_option(opt_idx) }
}

pub unsafe fn optval_own(opt_idx: OptIndex, value: Object) -> Object {
    unsafe {
        if optval_is_owned(opt_idx) {
            value
        } else {
            copy_object(value, ptr::null_mut())
        }
    }
}

unsafe fn optval_snapshot(value: Object) -> Object {
    unsafe {
        if value.obj_type == ObjectType::kObjectTypeLuaRef {
            let s = nlua_funcref_str(value.data.luaref(), true);
            return cstr_to_obj(s.as_ptr());
        }
        copy_object(value, ptr::null_mut())
    }
}

unsafe fn optval_snapshot_varp(opt_idx: OptIndex, varp: *mut c_void) -> Object {
    unsafe {
        let value = opt_from_varp(opt_idx, varp);
        let snapshot = optval_snapshot(value);
        optval_free_read(opt_idx, value);
        snapshot
    }
}

unsafe fn opt_from_callback(cb: *mut Callback) -> Object {
    unsafe {
        match (*cb).cb_type() {
            kCallbackLua => {
                let luaref = (*cb).data.as_luaref();
                return luaref_obj(api_new_luaref(luaref));
            }
            kCallbackExpr | kCallbackFuncref => {
                return cstr_to_obj((*cb).data.as_funcref());
            }
            kCallbackPartial => {
                let mut tv = typval_T::new();
                callback_put(cb, &raw mut tv);
                let str_ = encode_tv2string(&raw mut tv, ptr::null_mut());
                tv_clear(&raw mut tv);
                return cstr_to_obj(str_.as_ptr());
            }
            kCallbackNone => {}
            _ => {}
        }
        cstr_to_obj(c"".as_ptr())
    }
}

unsafe fn opt_to_callback(value: Object, is_expr: bool) -> Callback {
    unsafe {
        let mut cb = Callback::NONE;
        match value.obj_type {
            ObjectType::kObjectTypeLuaRef => {
                cb.data = crate::eval::typval::CallbackData::lua(api_new_luaref(value.data.luaref()));
            }
            ObjectType::kObjectTypeString => {
                if !is_expr {
                    option_set_callback_func(value.data.string().data, &raw mut cb);
                } else if value.data.string().size > 0 {
                    let funcref =
                        if let Some(name) = get_scriptlocal_funcname(value.data.string().data) {
                            name
                        } else {
                            CStr::from_ptr(value.data.string().data).to_owned()
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
pub unsafe fn opt_from_varp(opt_idx: OptIndex, varp: *mut c_void) -> Object {
    unsafe {
        let opt = get_option(opt_idx);
        if (*opt).immutable {
            return if (*opt).opt_type == ObjectType::kObjectTypeString {
                string_obj(cstr_as_string((*opt).def_val.data.string().data))
            } else {
                (*opt).def_val
            };
        }

        if varp.cast::<c_int>() == &raw mut (*curbuf).b_changed {
            return boolean_obj(curbufIsChanged());
        }

        if is_callback_option(opt_idx) {
            return opt_from_callback(varp.cast::<Callback>());
        }

        match (*opt).opt_type {
            ObjectType::kObjectTypeBoolean => {
                opt_from_tristate(tristate_from_int(OptInt::from(*(varp.cast::<c_int>()))))
            }
            ObjectType::kObjectTypeInteger => integer_obj(*(varp.cast::<OptInt>())),
            ObjectType::kObjectTypeString => {
                string_obj(cstr_as_string(*(varp.cast::<*mut c_char>())))
            }
            _ => std::process::abort(),
        }
    }
}

/// Stores an option value at its "varp", freeing the previously-stored value.
unsafe fn set_option_varp(opt_idx: OptIndex, varp: *mut c_void, value: Object) {
    unsafe {
        let opt = get_option(opt_idx);
        if (*opt).immutable {
            return;
        }

        if is_callback_option(opt_idx) {
            let cb = varp.cast::<Callback>();
            callback_free(cb);
            *cb = opt_to_callback(value, ((*opt).flags & kOptFlagExpr) != 0);
            return;
        }

        match value.obj_type {
            ObjectType::kObjectTypeUnset => {
                assert!(option_is_global_local(opt_idx));
                if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
                    *(varp.cast::<c_int>()) = kNone;
                } else if option_has_type(opt_idx, ObjectType::kObjectTypeString) {
                    let pp = varp.cast::<*mut c_char>();
                    free_string_option(*pp);
                    *pp = (&raw mut empty_string_option).cast::<c_char>();
                } else if opt_idx == kOptUndolevels {
                    *(varp.cast::<OptInt>()) = NO_LOCAL_UNDOLEVEL;
                } else {
                    assert!(
                        opt_idx == kOptScrolloff
                            || opt_idx == kOptScrolloffpad
                            || opt_idx == kOptSidescrolloff
                    );
                    *(varp.cast::<OptInt>()) = -1;
                }
            }
            ObjectType::kObjectTypeBoolean => {
                assert!(option_has_type(opt_idx, ObjectType::kObjectTypeBoolean));
                *(varp.cast::<c_int>()) = c_int::from(value.data.boolean());
            }
            ObjectType::kObjectTypeInteger => {
                assert!(option_has_type(opt_idx, ObjectType::kObjectTypeInteger));
                *(varp.cast::<OptInt>()) = value.data.integer();
            }
            ObjectType::kObjectTypeString => {
                assert!(option_has_type(opt_idx, ObjectType::kObjectTypeString));
                let pp = varp.cast::<*mut c_char>();
                free_string_option(*pp);
                *pp = copy_option_val(value.data.string().data);
            }
            _ => std::process::abort(),
        }
    }
}

/// Gets the string form of an option value.
unsafe fn optval_to_cstr(o: Object, quote: bool) -> std::ffi::CString {
    unsafe {
        match o.obj_type {
            ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {
                std::ffi::CString::default()
            }
            ObjectType::kObjectTypeBoolean => {
                if o.data.boolean() {
                    c"true".to_owned()
                } else {
                    c"false".to_owned()
                }
            }
            ObjectType::kObjectTypeInteger => {
                let s = format!("{}", o.data.integer());
                crate::memory::cstr_from_bytes(s.as_bytes())
            }
            ObjectType::kObjectTypeString => {
                let slice = if o.data.string().data.is_null() || o.data.string().size == 0 {
                    &[][..]
                } else {
                    std::slice::from_raw_parts(o.data.string().data.cast::<u8>(), o.data.string().size)
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
            ObjectType::kObjectTypeLuaRef => nlua_funcref_str(o.data.luaref(), false),
            _ => std::process::abort(),
        }
    }
}

/// Converts a structured option (non-scalar Object) to the internal scalar (:set-style) form.
pub unsafe fn optval_from_obj(
    opt_idx: OptIndex,
    o: Object,
    op: set_op_T,
    error: *mut bool,
) -> Object {
    unsafe {
        if o.obj_type == ObjectType::kObjectTypeNil {
            return nil_obj();
        }

        let flags = (*get_option(opt_idx)).flags;
        let is_list = (flags & (kOptFlagComma | kOptFlagFlagList)) != 0;
        let is_map = (flags & kOptFlagColon) != 0 || is_dict_option(opt_idx);
        let is_flaglist = (flags & kOptFlagFlagList) != 0;
        let is_comma = (flags & kOptFlagComma) != 0;
        let allow_dup = (flags & kOptFlagNoDup) == 0;

        let type_ok = match o.obj_type {
            ObjectType::kObjectTypeBoolean => {
                option_has_type(opt_idx, ObjectType::kObjectTypeBoolean)
            }
            ObjectType::kObjectTypeInteger => {
                option_has_type(opt_idx, ObjectType::kObjectTypeInteger)
            }
            ObjectType::kObjectTypeString => {
                option_has_type(opt_idx, ObjectType::kObjectTypeString)
                    || opt_idx == kOptWildchar
                    || opt_idx == kOptWildcharm
            }
            ObjectType::kObjectTypeArray => is_list,
            ObjectType::kObjectTypeDict => is_map || is_flaglist,
            ObjectType::kObjectTypeLuaRef => is_callback_option(opt_idx),
            _ => false,
        };
        if !type_ok {
            *error = true;
            return nil_obj();
        }

        match o.obj_type {
            ObjectType::kObjectTypeBoolean | ObjectType::kObjectTypeInteger => return o,
            ObjectType::kObjectTypeLuaRef => return luaref_obj(api_new_luaref(o.data.luaref())),
            _ => {}
        }

        let mut str_cstr: std::ffi::CString = if o.obj_type == ObjectType::kObjectTypeString {
            crate::memory::cstr_to_cstring(o.data.string().data)
        } else if o.obj_type == ObjectType::kObjectTypeArray {
            let mut items: Vec<std::ffi::CString> = Vec::with_capacity(o.data.array().size);
            for ai in 0..o.data.array().size {
                let item = *o.data.array().items.add(ai);
                if item.obj_type != ObjectType::kObjectTypeString {
                    *error = true;
                    return nil_obj();
                }
                let s_cstr = CStr::from_ptr(item.data.string().data);
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
                    if !(v.obj_type == ObjectType::kObjectTypeNil
                        || (v.obj_type == ObjectType::kObjectTypeBoolean && !v.data.boolean()))
                    {
                        items.push(CStr::from_ptr(k.data).to_owned());
                    }
                } else if v.obj_type == ObjectType::kObjectTypeString {
                    let k_bytes = CStr::from_ptr(k.data).to_bytes();
                    let v_bytes = CStr::from_ptr(v.data.string().data).to_bytes();
                    let mut kv = Vec::with_capacity(k_bytes.len() + 1 + v_bytes.len());
                    kv.extend_from_slice(k_bytes);
                    kv.push(b':');
                    kv.extend_from_slice(v_bytes);
                    items.push(crate::memory::cstr_from_bytes(&kv));
                } else if v.obj_type == ObjectType::kObjectTypeInteger {
                    let k_bytes = CStr::from_ptr(k.data).to_bytes();
                    let num_s = format!(":{}", v.data.integer());
                    let mut kv = Vec::with_capacity(k_bytes.len() + num_s.len());
                    kv.extend_from_slice(k_bytes);
                    kv.extend_from_slice(num_s.as_bytes());
                    items.push(crate::memory::cstr_from_bytes(&kv));
                } else if v.obj_type == ObjectType::kObjectTypeBoolean
                    || v.obj_type == ObjectType::kObjectTypeNil
                {
                    if v.obj_type == ObjectType::kObjectTypeBoolean && v.data.boolean() {
                        items.push(CStr::from_ptr(k.data).to_owned());
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

        if op == OP_REMOVING && is_map && strchr(str_cstr.as_ptr(), b':' as c_int).is_null() {
            let mut bytes = str_cstr.into_bytes();
            bytes.push(b':');
            str_cstr = std::ffi::CString::new(bytes).unwrap();
        }

        cstr_to_obj(str_cstr.as_ptr())
    }
}

/// Check if option is hidden.
pub unsafe fn is_option_hidden(opt_idx: OptIndex) -> bool {
    unsafe { opt_idx != kOptInvalid && (*get_option(opt_idx)).immutable }
}

/// Check if option supports a specific type.
pub unsafe fn option_has_type(opt_idx: OptIndex, opt_type: ObjectType) -> bool {
    unsafe { opt_idx != kOptInvalid && (*get_option(opt_idx)).opt_type == opt_type }
}

/// Check if option supports a specific scope.
pub unsafe fn option_has_scope(opt_idx: OptIndex, scope: OptScope) -> bool {
    unsafe {
        assert!((kOptScopeGlobal..kOptScopeSize).contains(&scope));
        ((*get_option(opt_idx)).scope_flags & (1u8 << scope)) != 0
    }
}

#[inline]
unsafe fn option_is_global_local(opt_idx: OptIndex) -> bool {
    unsafe {
        if opt_idx == kOptInvalid {
            return false;
        }
        let bw: OptScopeFlags = (1 << kOptScopeBuf) | (1 << kOptScopeWin);
        ((*get_option(opt_idx)).scope_flags & bw) != 0 && option_has_scope(opt_idx, kOptScopeGlobal)
    }
}

#[inline]
unsafe fn option_is_global_only(opt_idx: OptIndex) -> bool {
    unsafe {
        if opt_idx == kOptInvalid {
            return false;
        }
        let bw: OptScopeFlags = (1 << kOptScopeBuf) | (1 << kOptScopeWin);
        ((*get_option(opt_idx)).scope_flags & bw) == 0 && option_has_scope(opt_idx, kOptScopeGlobal)
    }
}

#[inline]
unsafe fn option_is_window_local(opt_idx: OptIndex) -> bool {
    unsafe {
        if opt_idx == kOptInvalid {
            return false;
        }
        let exclude: OptScopeFlags = (1 << kOptScopeGlobal) | (1 << kOptScopeBuf);
        ((*get_option(opt_idx)).scope_flags & exclude) == 0
            && option_has_scope(opt_idx, kOptScopeWin)
    }
}

/// Get option index for scope.
pub unsafe fn option_scope_idx(opt_idx: OptIndex, scope: OptScope) -> isize {
    unsafe { (*get_option(opt_idx)).scope_idx[scope as usize] }
}

/// Gets the value for an option.
pub unsafe fn get_option_value(opt_idx: OptIndex, opt_flags: c_int) -> Object {
    unsafe {
        if opt_idx == kOptInvalid {
            return nil_obj();
        }
        let opt = get_option(opt_idx);
        optval_own(
            opt_idx,
            opt_from_varp(opt_idx, get_varp_scope(opt, opt_flags)),
        )
    }
}

unsafe fn optval_is_unset_local(opt_idx: OptIndex, value: Object) -> bool {
    unsafe {
        if !option_is_global_local(opt_idx) {
            return false;
        }
        if value.obj_type == ObjectType::kObjectTypeUnset {
            return true;
        }
        if option_has_type(opt_idx, ObjectType::kObjectTypeString) {
            return value.obj_type == ObjectType::kObjectTypeString && value.data.string().size == 0;
        }
        if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean)
            || value.obj_type != ObjectType::kObjectTypeInteger
        {
            return false;
        }
        match opt_idx {
            kOptScrolloff | kOptScrolloffpad | kOptSidescrolloff => value.data.integer() == -1,
            kOptUndolevels => value.data.integer() == NO_LOCAL_UNDOLEVEL,
            _ => std::process::abort(),
        }
    }
}

unsafe fn is_option_local_value_unset(opt_idx: OptIndex) -> bool {
    unsafe {
        if !option_is_global_local(opt_idx) {
            return false;
        }
        let opt = get_option(opt_idx);
        let local_value = opt_from_varp(opt_idx, get_varp_scope(opt, OPT_LOCAL));
        let is_unset = optval_is_unset_local(opt_idx, local_value);
        optval_free_read(opt_idx, local_value);
        is_unset
    }
}

/// Handle side-effects of setting an option.
unsafe fn did_set_option(
    args: *mut optset_T,
    set_sid: scid_T,
    direct: bool,
    value_replaced: bool,
    value_untrusted: bool,
) -> *const c_char {
    unsafe {
        let opt_idx = (*args).os_idx;
        let varp = (*args).os_varp;
        let opt_flags = (*args).os_flags;
        let opt = get_option(opt_idx);
        let mut errmsg: *const c_char = ptr::null();

        if direct {
            // Don't do any extra processing if setting directly.
        } else if (secure != 0 || sandbox != 0) && ((*opt).flags & kOptFlagSecure) != 0 {
            errmsg = e_secure.as_ptr();
        } else if (*args).os_newval.obj_type == ObjectType::kObjectTypeString
            && !is_callback_option(opt_idx)
            && check_illegal_path_names(*(varp.cast::<*mut c_char>()), (*opt).flags)
        {
            errmsg = e_invarg.as_ptr();
        } else if let Some(cb) = (*opt).opt_did_set_cb {
            errmsg = cb(args);
        }

        if !errmsg.is_null() {
            set_option_varp(opt_idx, varp, (*args).os_oldval);
            optval_free((*args).os_oldval);
            return errmsg;
        }

        let new_value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));

        if set_sid != SID_NONE {
            let script_ctx = if set_sid == 0 {
                current_sctx
            } else {
                sctx_T {
                    sc_sid: set_sid,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                }
            };
            set_option_sctx(opt_idx, opt_flags, script_ctx);
        }

        optval_free((*args).os_oldval);

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

        let ws = (*curwin).w_s.cast::<synblock_T>();
        if varp == (&raw mut (*curbuf).b_p_syn).cast::<c_void>() {
            do_syntax_autocmd(curbuf, (*args).os_value_changed);
        } else if varp == (&raw mut (*curbuf).b_p_ft).cast::<c_void>() {
            if (opt_flags & OPT_MODELINE) == 0 || (*args).os_value_changed {
                do_filetype_autocmd(curbuf, (*args).os_value_changed);
            }
        } else if varp == (&raw mut (*ws).b_p_spl).cast::<c_void>() {
            do_spelllang_source(curwin);
        }

        comp_col();

        if varp == (&raw mut p_mouse).cast::<c_void>() {
            setmouse();
        } else if (varp == (&raw mut p_flp).cast::<c_void>()
            || varp == (&raw mut (*curbuf).b_p_flp).cast::<c_void>())
            && (*curwin).w_briopt_list != 0
        {
            redraw_all_later(UPD_NOT_VALID);
        } else if varp == (&raw mut p_wbr).cast::<c_void>()
            || varp == (&raw mut (*curwin).w_onebuf_opt.wo_wbr).cast::<c_void>()
        {
            if varp == (&raw mut p_wbr).cast::<c_void>() {
                set_winbar_all(true);
            } else {
                set_winbar(true);
            }
        }

        if (*curwin).w_curswant != MAXCOL
            && ((*opt).flags & (kOptFlagCurswant | kOptFlagRedrAll)) != 0
            && ((*opt).flags & kOptFlagHLOnly) == 0
        {
            (*curwin).w_set_curswant = true;
        }

        check_redraw((*opt).flags);

        if errmsg.is_null() {
            (*opt).flags |= kOptFlagWasSet;

            let flagsp = insecure_flag(curwin, opt_idx, opt_flags);
            let flagsp_local = if scope_both {
                insecure_flag(curwin, opt_idx, OPT_LOCAL)
            } else {
                ptr::null_mut()
            };
            if !(*args).os_value_checked && value_untrusted {
                *flagsp |= kOptFlagInsecure;
                if !flagsp_local.is_null() {
                    *flagsp_local |= kOptFlagInsecure;
                }
            } else if value_replaced {
                *flagsp &= !kOptFlagInsecure;
                if !flagsp_local.is_null() {
                    *flagsp_local &= !kOptFlagInsecure;
                }
            }
        }

        errmsg
    }
}

/// Validates an option value and prepares any data needed to apply it, without side effects.
pub unsafe fn validate_and_prepare_option_value(args: *mut optset_T) -> *const c_char {
    unsafe {
        let opt_idx = (*args).os_idx;
        let newval = &raw mut (*args).os_newval;
        let opt_flags = (*args).os_flags;
        let buf = (*args).os_buf;
        let win = (*args).os_win;
        let errbuf = (*args).os_errbuf;
        let mut errmsg: *const c_char = ptr::null();
        let opt = get_option(opt_idx);

        if (opt_flags & OPT_LOCAL) != 0 && optval_is_unset_local(opt_idx, *newval) {
            return ptr::null();
        }

        if (*newval).obj_type == ObjectType::kObjectTypeNil {
            if opt_flags == OPT_GLOBAL {
                errmsg = tr(c"Cannot unset global option value".as_ptr());
            } else {
                let varp = get_varp_scope_from(opt, OPT_GLOBAL, buf, win);
                *newval = if option_is_global_local(opt_idx) {
                    unset_obj()
                } else {
                    optval_own(opt_idx, opt_from_varp(opt_idx, varp))
                };
            }
        } else if (*newval).obj_type == ObjectType::kObjectTypeLuaRef {
            assert!(is_callback_option(opt_idx));
        } else if !option_has_type(opt_idx, optval_type(*newval)) {
            let rep = optval_to_cstr(*newval, true);
            let type_str = optval_type_name((*opt).opt_type);
            errmsg = if errbuf.is_null() {
                c"".as_ptr()
            } else {
                vim_snprintf!(
                    (*errbuf).data,
                    (*errbuf).size,
                    gettext(c"Invalid value for option '%s': expected %s, got %s %s".as_ptr()),
                    (*opt).fullname.as_ptr(),
                    type_str,
                    optval_type_name(optval_type(*newval)),
                    rep.as_ptr(),
                );
                (*errbuf).data.cast_const()
            };
        } else if (*newval).obj_type == ObjectType::kObjectTypeInteger {
            errmsg = validate_num_option(opt_idx, (*newval).data.integer_mut(), win, errbuf);
        }
        if !errmsg.is_null() {
            return errmsg;
        }

        if (*opt).immutable {
            let oldval = opt_from_varp(opt_idx, get_varp_scope_from(opt, opt_flags, buf, win));
            let unchanged = option_equal(oldval, *newval);
            optval_free_read(opt_idx, oldval);
            if !unchanged {
                return e_unsupportedoption.as_ptr();
            }
        }
        let schema = (*opt).schema;
        if (*newval).obj_type == ObjectType::kObjectTypeString && !schema.is_empty() {
            errmsg = opt_strings_check((*newval).data.string().data, schema, errbuf);
            if !errmsg.is_null() {
                return errmsg;
            }
        }
        if (*newval).obj_type != ObjectType::kObjectTypeUnset
            && let Some(validate_cb) = (*opt).opt_validate_cb
        {
            let scope_both = (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) == 0;
            (*args).os_varp = if scope_both && option_is_global_local(opt_idx) {
                (*opt).var
            } else {
                get_varp_scope_from(opt, opt_flags, buf, win)
            };
            (*args).os_oldval = opt_from_varp(opt_idx, (*args).os_varp);
            errmsg = validate_cb(args);
            optval_free_read(opt_idx, (*args).os_oldval);
            (*args).os_oldval = nil_obj();
            return errmsg;
        }
        ptr::null()
    }
}

/// Set the value of an option using an Object.
unsafe fn set_option(
    opt_idx: OptIndex,
    mut value: Object,
    opt_flags: c_int,
    set_sid: scid_T,
    direct: bool,
    value_replaced: bool,
    errbuf: *const CharBuf,
) -> *const c_char {
    unsafe {
        assert!(opt_idx != kOptInvalid);

        let mut errmsg: *const c_char = ptr::null();
        let mut args = optset_T {
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
            os_win: curwin,
            os_buf: curbuf,
        };

        let opt = get_option(opt_idx);
        if !direct {
            if (secure != 0 || sandbox != 0) && ((*opt).flags & kOptFlagSecure) != 0 {
                optval_free(value);
                return e_secure.as_ptr();
            }

            errmsg = validate_and_prepare_option_value(&raw mut args);
            value = args.os_newval;

            if errmsg.is_null()
                && value.obj_type == ObjectType::kObjectTypeString
                && ((*opt).flags & kOptFlagFunc) != 0
                && value.data.string().size > 0
            {
                let mut cb = Callback::NONE;
                if option_set_callback_func(value.data.string().data, &raw mut cb) == FAIL {
                    errmsg = e_invarg.as_ptr();
                }
                callback_free(&raw mut cb);
            }
        } else if value.obj_type == ObjectType::kObjectTypeString && is_dict_option(opt_idx) {
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
            let flags = (*opt).flags;
            if value.obj_type == ObjectType::kObjectTypeString
                && (flags & kOptFlagExpand) != 0
                && opt_idx != kOptEqualprg
                && opt_idx != kOptFormatprg
                && opt_idx != kOptGrepprg
                && opt_idx != kOptKeywordprg
                && opt_idx != kOptMakeprg
                && opt_idx != kOptShell
            {
                let allow_comma = (flags & kOptFlagComma) != 0;
                let allow_space =
                    opt_idx == kOptCdpath || opt_idx == kOptPath || opt_idx == kOptTags;
                let mut p = value.data.string().data;
                while *p != NUL {
                    if *p != b'\\' as c_char
                        || (*p.add(1) == b',' as c_char && allow_comma)
                        || (*p.add(1) == b' ' as c_char && allow_space)
                    {
                        p = p.add(1);
                        continue;
                    }
                    *p = b'/' as c_char;
                    p = p.add(1);
                }
            }
        }

        let scope_local = (opt_flags & OPT_LOCAL) != 0;
        let scope_global = (opt_flags & OPT_GLOBAL) != 0;
        let scope_both = !scope_local && !scope_global;
        let is_opt_local_unset = is_option_local_value_unset(opt_idx);

        let varp = if scope_both && option_is_global_local(opt_idx) {
            (*opt).var
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
                copy_object(saved_old_global_value, ptr::null_mut())
            } else {
                optval_snapshot_varp(opt_idx, varp_local)
            };
            saved_used_value = if scope_local && is_opt_local_unset {
                optval_snapshot_varp(opt_idx, get_varp(opt))
            } else {
                optval_snapshot(old_value)
            };
        }

        let secure_saved = secure;

        let value_untrusted = secure != 0
            || sandbox != 0
            || (opt_flags & OPT_MODELINE) != 0
            || (!value_replaced && was_set_insecurely(curwin, opt_idx, opt_flags) != 0);
        if secure == 0 && value_untrusted {
            secure = 1;
        }

        set_option_varp(opt_idx, varp, value);
        if value.obj_type == ObjectType::kObjectTypeUnset {
            value = optval_own(opt_idx, opt_from_varp(opt_idx, varp));
        }
        if !direct {
            saved_new_value = optval_snapshot(value);
        }
        args.os_varp = varp;
        args.os_oldval = old_value;
        args.os_newval = value;
        args.os_buf = curbuf;
        args.os_win = curwin;
        errmsg = did_set_option(
            &raw mut args,
            set_sid,
            direct,
            value_replaced,
            value_untrusted,
        );
        args.os_prep = crate::optionstr::OptSetPrep::None;

        secure = secure_saved;

        if errmsg.is_null() && !direct {
            if starting == 0 {
                apply_optionset_autocmd(
                    opt_idx,
                    opt_flags,
                    saved_used_value,
                    saved_old_global_value,
                    saved_old_local_value,
                    saved_new_value,
                );
            }
            if ((*opt).flags & kOptFlagUIOption) != 0 {
                ui_call_option_set(cstr_as_string((*opt).fullname.as_ptr()), saved_new_value);
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
pub unsafe fn set_option_direct(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: c_int,
    set_sid: scid_T,
) {
    unsafe {
        if is_option_hidden(opt_idx) {
            return;
        }

        let errmsg = set_option(
            opt_idx,
            copy_object(value, ptr::null_mut()),
            opt_flags,
            set_sid,
            true,
            true,
            ptr::null(),
        );
        assert!(errmsg.is_null());
    }
}

/// Set option value directly for buffer / window, without processing any side effects.
pub unsafe fn set_option_direct_for(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: c_int,
    set_sid: scid_T,
    target: OptTarget,
) {
    unsafe {
        let save_curbuf = curbuf;
        let save_curwin = curwin;

        match target {
            OptTarget::Global => {}
            OptTarget::Win(win) => {
                curwin = win;
                curbuf = (*curwin).w_buffer;
            }
            OptTarget::Buf(buf) => {
                curbuf = buf;
            }
            OptTarget::Tab(_) => std::process::abort(),
        }

        set_option_direct(opt_idx, value, opt_flags, set_sid);

        curwin = save_curwin;
        curbuf = save_curbuf;
    }
}

/// Sets the value of an (non-tty) option.
pub unsafe fn set_option_value(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: c_int,
    value_replaced: bool,
    errbuf: *const CharBuf,
) -> *const c_char {
    unsafe {
        assert!(opt_idx != kOptInvalid);

        let flags = (*get_option(opt_idx)).flags;
        if sandbox > 0 && (flags & kOptFlagSecure) != 0 {
            return tr(e_sandbox.as_ptr());
        }

        set_option(
            opt_idx,
            copy_object(value, ptr::null_mut()),
            opt_flags,
            0,
            false,
            value_replaced,
            errbuf,
        )
    }
}

#[inline]
unsafe fn unset_option_local_value(opt_idx: OptIndex) -> *const c_char {
    unsafe {
        assert!(option_is_global_local(opt_idx));
        set_option_value(opt_idx, unset_obj(), OPT_LOCAL, true, ptr::null())
    }
}

/// Call set_option_value() and when an error is returned, report it.
pub unsafe fn set_option_value_give_err(opt_idx: OptIndex, value: Object, opt_flags: c_int) {
    unsafe {
        let mut errbuf_storage = [0 as c_char; IOSIZE];
        let errbuf = CharBuf {
            data: errbuf_storage.as_mut_ptr(),
            size: IOSIZE,
        };
        let errmsg = set_option_value(opt_idx, value, opt_flags, true, &raw const errbuf);

        if !errmsg.is_null() {
            emsg(tr(errmsg));
        }
    }
}

#[derive(Clone, Copy)]
pub enum OptTarget {
    Global,
    Win(*mut win_T),
    Buf(*mut buf_T),
    Tab(*mut tabpage_T),
}

enum OptSavedCtx {
    None,
    Switch(CtxSwitch),
    Tab(*mut tabpage_T),
}

/// Switch current context to get/set option value for the target window/buffer/tabpage.
unsafe fn switch_option_context(
    ctx: &mut OptSavedCtx,
    scope: OptScope,
    from: OptTarget,
    err: *mut Error,
) -> bool {
    unsafe {
        match scope {
            kOptScopeGlobal => false,
            kOptScopeWin => {
                let OptTarget::Win(win) = from else {
                    return false;
                };
                if win == curwin {
                    return false;
                }
                let mut cs = CtxSwitch::new();
                if !ctx_switch(
                    &raw mut cs,
                    win,
                    win_find_tabpage(win),
                    ptr::null_mut(),
                    kCtxNoDisplay,
                ) {
                    ctx_restore(&raw mut cs);
                    if !error_set(err) {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeException,
                            c"Problem while switching windows".as_ptr(),
                        );
                    }
                    return false;
                }
                *ctx = OptSavedCtx::Switch(cs);
                true
            }
            kOptScopeBuf => {
                let OptTarget::Buf(buf) = from else {
                    return false;
                };
                if buf == curbuf {
                    return false;
                }
                let mut cs = CtxSwitch::new();
                ctx_switch(&raw mut cs, ptr::null_mut(), ptr::null_mut(), buf, 0);
                *ctx = OptSavedCtx::Switch(cs);
                true
            }
            kOptScopeTab => {
                let OptTarget::Tab(tab) = from else {
                    return false;
                };
                if tab == curtab {
                    return false;
                }
                *ctx = OptSavedCtx::Tab(curtab);
                unuse_tabpage(curtab);
                use_tabpage(tab);
                true
            }
            _ => unreachable!(),
        }
    }
}

unsafe fn restore_option_context(ctx: &mut OptSavedCtx, scope: OptScope) {
    unsafe {
        match (scope, ctx) {
            (kOptScopeGlobal, _) => {}
            (kOptScopeWin | kOptScopeBuf, OptSavedCtx::Switch(cs)) => {
                ctx_restore(cs);
            }
            (kOptScopeTab, OptSavedCtx::Tab(saved_curtab)) => {
                unuse_tabpage(curtab);
                use_tabpage(*saved_curtab);
            }
            _ => {}
        }
    }
}

/// Get option value for buffer / window.
pub unsafe fn get_option_value_for(
    opt_idx: OptIndex,
    opt_flags: c_int,
    scope: OptScope,
    from: OptTarget,
    err: *mut Error,
) -> Object {
    unsafe {
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
}

/// Set option value for buffer / window.
pub unsafe fn set_option_value_for(
    opt_idx: OptIndex,
    value: Object,
    opt_flags: c_int,
    scope: OptScope,
    from: OptTarget,
    value_replaced: bool,
    err: *mut Error,
) {
    unsafe {
        if let (kOptScopeTab, OptTarget::Tab(tab)) = (scope, from)
            && tab != curtab
        {
            assert!(opt_idx == kOptCmdheight);
            if value.obj_type != ObjectType::kObjectTypeInteger {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"'cmdheight' requires a Number".as_ptr(),
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
        let errmsg = set_option_value(opt_idx, value, opt_flags, value_replaced, &raw const errbuf);
        if !errmsg.is_null() {
            api_set_error!(err, ErrorType::kErrorTypeException, c"%s".as_ptr(), errmsg);
        }

        if switched {
            restore_option_context(&mut ctx, scope);
        }
    }
}

unsafe fn showoptions(all: bool, opt_flags: c_int) {
    unsafe {
        const INC: c_int = 20;
        const GAP: c_int = 3;

        let mut items: Vec<*mut vimoption_T> = Vec::with_capacity(kOptCount as usize);

        msg_ext_set_kind(c"list_cmd".as_ptr());
        if (opt_flags & OPT_GLOBAL) != 0 {
            msg_puts_title(tr(c"\n--- Global option values ---".as_ptr()));
        } else if (opt_flags & OPT_LOCAL) != 0 {
            msg_puts_title(tr(c"\n--- Local option values ---".as_ptr()));
        } else {
            msg_puts_title(tr(c"\n--- Options ---".as_ptr()));
        }

        let mut run = 1;
        while run <= 2 && !got_int {
            items.clear();
            for opt_idx in 0..kOptCount {
                let opt = get_option(opt_idx);
                if message_filtered((*opt).fullname.as_ptr().cast_mut()) {
                    continue;
                }

                let mut varp: *mut c_void = ptr::null_mut();
                if (opt_flags & (OPT_LOCAL | OPT_GLOBAL)) != 0 {
                    if !option_is_global_only(opt_idx) {
                        varp = get_varp_scope(opt, opt_flags);
                    }
                } else {
                    varp = get_varp(opt);
                }
                if !varp.is_null() && (all || optval_default(opt_idx, varp) == 0) {
                    let len = if (opt_flags & OPT_ONECOLUMN) != 0 {
                        Columns
                    } else if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
                        1
                    } else {
                        optval_fmt(opt_idx, opt_flags);
                        ((*opt).fullname.to_bytes().len() as c_int)
                            + vim_strsize((&raw const NameBuff).cast())
                            + 1
                    };
                    if (len <= INC - GAP && run == 1) || (len > INC - GAP && run == 2) {
                        items.push(opt);
                    }
                }
            }

            let item_count = items.len();
            let rows = if run == 1 {
                let mut cols = (Columns + GAP - 3) / INC;
                if cols == 0 {
                    cols = 1;
                }
                (item_count as c_int + cols - 1) / cols
            } else {
                item_count as c_int
            };

            let mut row = 0;
            while row < rows && !got_int {
                msg_putchar(b'\n' as c_int);
                if got_int {
                    break;
                }
                let mut col: c_int = 0;
                let mut i = row as usize;
                while i < item_count {
                    msg_advance(col);
                    showoneopt(items[i], opt_flags);
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

unsafe fn optval_default(opt_idx: OptIndex, varp: *mut c_void) -> c_int {
    unsafe {
        if is_option_hidden(opt_idx) {
            return 1;
        }

        let current_val = opt_from_varp(opt_idx, varp);
        let default_val = (*get_option(opt_idx)).def_val;

        let is_default = option_equal(current_val, default_val);
        optval_free_read(opt_idx, current_val);
        c_int::from(is_default)
    }
}

/// Send update to UIs with values of UI relevant options.
pub unsafe fn ui_refresh_options() {
    unsafe {
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            if ((*opt).flags & kOptFlagUIOption) == 0 {
                continue;
            }
            let name = cstr_as_string((*opt).fullname.as_ptr());
            let value = opt_from_varp(opt_idx, (*opt).var);
            ui_call_option_set(name, value);
        }
        if !p_mouse.is_null() {
            setmouse();
        }
    }
}

unsafe fn showoneopt(opt: *mut vimoption_T, opt_flags: c_int) {
    unsafe {
        let save_silent = silent_mode;

        silent_mode = false;
        info_message = true;

        let opt_idx = get_opt_idx(opt);
        let varp = get_varp_scope(opt, opt_flags);

        if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
            let val_int = if (*opt).immutable {
                c_int::from((*opt).def_val.data.boolean())
            } else if varp.cast::<c_int>() == &raw mut (*curbuf).b_changed {
                c_int::from(curbufIsChanged())
            } else {
                *(varp.cast::<c_int>())
            };
            if val_int == 0 {
                msg_puts(c"no".as_ptr());
            } else if val_int < 0 {
                msg_puts(c"--".as_ptr());
            } else {
                msg_puts(c"  ".as_ptr());
            }
        } else {
            msg_puts(c"  ".as_ptr());
        }
        msg_puts((*opt).fullname.as_ptr());
        if !option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
            msg_putchar(b'=' as c_int);
            optval_fmt(opt_idx, opt_flags);
            let namebuf_ptr = (&raw const NameBuff).cast::<c_char>();
            if *namebuf_ptr != NUL {
                msg_outtrans(namebuf_ptr, 0, false);
            }
        }

        silent_mode = save_silent;
        info_message = false;
    }
}

/// Write modified options as ":set" commands to a file.
pub unsafe fn makeset(fd: *mut FILE, opt_flags: c_int, local_only: c_int) -> c_int {
    unsafe {
        for pri in (0..=1).rev() {
            for opt_idx in 0..kOptCount {
                let opt = get_option(opt_idx);

                if ((*opt).flags & kOptFlagNoMkrc) == 0
                    && ((pri == 1) == (((*opt).flags & kOptFlagPriMkrc) != 0))
                {
                    if option_is_global_only(opt_idx) && (opt_flags & OPT_GLOBAL) == 0 {
                        continue;
                    }

                    if (opt_flags & OPT_GLOBAL) != 0 && ((*opt).flags & kOptFlagNoGlob) != 0 {
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
                        && ((*opt).var == (&raw mut p_rtp).cast::<c_void>()
                            || (*opt).var == (&raw mut p_pp).cast::<c_void>())
                    {
                        continue;
                    }

                    let mut round: c_int = 2;
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
                        if opt_idx == kOptSyntax || opt_idx == kOptFiletype {
                            if fprintf!(
                                fd,
                                c"if &%s != '%s'".as_ptr(),
                                (*opt).fullname.as_ptr(),
                                *(varp.cast::<*mut c_char>()),
                            ) < 0
                                || put_eol(fd.cast()) < 0
                            {
                                return FAIL;
                            }
                            do_endif = true;
                        }
                        if put_set(fd, cmd, opt_idx, varp) == FAIL {
                            return FAIL;
                        }
                        if do_endif && put_line(fd.cast(), c"endif".as_ptr().cast_mut()) == FAIL {
                            return FAIL;
                        }

                        varp = varp_local;
                        round += 1;
                    }
                }
            }
        }
        OK
    }
}

/// Generate set commands for the local fold options only.
pub unsafe fn makefoldset(fd: *mut FILE) -> c_int {
    unsafe {
        let cmd = c"setlocal".as_ptr().cast_mut();
        if put_set(
            fd,
            cmd,
            kOptFoldmethod,
            (&raw mut (*curwin).w_onebuf_opt.wo_fdm).cast(),
        ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldexpr,
                (&raw mut (*curwin).w_onebuf_opt.wo_fde).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldmarker,
                (&raw mut (*curwin).w_onebuf_opt.wo_fmr).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldignore,
                (&raw mut (*curwin).w_onebuf_opt.wo_fdi).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldlevel,
                (&raw mut (*curwin).w_onebuf_opt.wo_fdl).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldminlines,
                (&raw mut (*curwin).w_onebuf_opt.wo_fml).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldnestmax,
                (&raw mut (*curwin).w_onebuf_opt.wo_fdn).cast(),
            ) == FAIL
            || put_set(
                fd,
                cmd,
                kOptFoldenable,
                (&raw mut (*curwin).w_onebuf_opt.wo_fen).cast(),
            ) == FAIL
        {
            return FAIL;
        }
        OK
    }
}

unsafe fn put_set_value(
    fd: *mut FILE,
    cmd: *mut c_char,
    opt_idx: OptIndex,
    varp: *mut c_void,
    value: Object,
) -> c_int {
    unsafe {
        let opt = get_option(opt_idx);
        let name = (*opt).fullname.as_ptr();
        let flags = (*opt).flags;

        if varp != (*opt).var && optval_is_unset_local(opt_idx, value) {
            return OK;
        }

        match value.obj_type {
            ObjectType::kObjectTypeBoolean => {
                let value_bool = value.data.boolean();
                if fprintf!(
                    fd,
                    c"%s %s%s".as_ptr(),
                    cmd,
                    if value_bool {
                        c"".as_ptr()
                    } else {
                        c"no".as_ptr()
                    },
                    name,
                ) < 0
                {
                    return FAIL;
                }
            }
            ObjectType::kObjectTypeInteger => {
                if fprintf!(fd, c"%s %s=".as_ptr(), cmd, name) < 0 {
                    return FAIL;
                }
                let value_num = value.data.integer();
                let mut wc: OptInt = 0;
                if wc_use_keyname(varp.cast::<OptInt>(), &raw mut wc) != 0 {
                    if fputs(get_special_key_name(wc as c_int, 0), fd) < 0 {
                        return FAIL;
                    }
                } else {
                    let num_s = format!("{value_num}\0");
                    if fputs(num_s.as_ptr().cast(), fd) < 0 {
                        return FAIL;
                    }
                }
            }
            ObjectType::kObjectTypeString => {
                if fprintf!(fd, c"%s %s=".as_ptr(), cmd, name) < 0 {
                    return FAIL;
                }
                let value_str = value.data.string().data;
                if !value_str.is_null() {
                    if (flags & kOptFlagExpand) != 0 {
                        let size = strlen(value_str) + 1;
                        let mut buf_vec = vec![0 as c_char; size];
                        let buf = buf_vec.as_mut_ptr();
                        home_replace(ptr::null_mut(), value_str, buf, size, false);

                        if size >= MAXPATHL
                            && (flags & kOptFlagComma) != 0
                            && !vim_strchr(value_str, b',' as c_int).is_null()
                        {
                            let mut part_vec = vec![0 as c_char; size];
                            let part = part_vec.as_mut_ptr();
                            if put_eol(fd.cast()) == FAIL {
                                return FAIL;
                            }
                            let mut p = buf;
                            while *p != NUL {
                                if fprintf!(fd, c"%s %s+=".as_ptr(), cmd, name) < 0 {
                                    return FAIL;
                                }
                                copy_option_part(&raw mut p, part, size, c",".as_ptr().cast_mut());
                                if put_escstr(fd.cast(), part, 2) == FAIL
                                    || put_eol(fd.cast()) == FAIL
                                {
                                    return FAIL;
                                }
                            }
                            return OK;
                        }
                        if put_escstr(fd.cast(), buf, 2) == FAIL {
                            return FAIL;
                        }
                    } else if put_escstr(fd.cast(), value_str, 2) == FAIL {
                        return FAIL;
                    }
                }
            }
            _ => std::process::abort(),
        }

        if put_eol(fd.cast()) < 0 {
            return FAIL;
        }
        OK
    }
}

unsafe fn put_set(fd: *mut FILE, cmd: *mut c_char, opt_idx: OptIndex, varp: *mut c_void) -> c_int {
    unsafe {
        let value = opt_from_varp(opt_idx, varp);
        let ret = if value.obj_type == ObjectType::kObjectTypeLuaRef {
            OK
        } else {
            put_set_value(fd, cmd, opt_idx, varp, value)
        };
        optval_free_read(opt_idx, value);
        ret
    }
}

pub unsafe fn get_varp_scope_from(
    p: *mut vimoption_T,
    opt_flags: c_int,
    buf: *mut buf_T,
    win: *mut win_T,
) -> *mut c_void {
    unsafe {
        let opt_idx = get_opt_idx(p);

        if (opt_flags & OPT_GLOBAL) != 0 && !option_is_global_only(opt_idx) {
            if option_is_window_local(opt_idx) {
                return get_varp_from(p, buf, win)
                    .cast::<u8>()
                    .add(std::mem::size_of::<winopt_T>())
                    .cast::<c_void>();
            }
            return (*p).var;
        }

        if (opt_flags & OPT_LOCAL) != 0 && option_is_global_local(opt_idx) {
            let ptr: *mut c_void = match opt_idx {
                kOptFormatprg => (&raw mut (*buf).b_p_fp).cast(),
                kOptFsync => (&raw mut (*buf).b_p_fs).cast(),
                kOptFindfunc => (&raw mut (*buf).b_p_ffu).cast(),
                kOptErrorformat => (&raw mut (*buf).b_p_efm).cast(),
                kOptGrepformat => (&raw mut (*buf).b_p_gefm).cast(),
                kOptGrepprg => (&raw mut (*buf).b_p_gp).cast(),
                kOptMakeprg => (&raw mut (*buf).b_p_mp).cast(),
                kOptEqualprg => (&raw mut (*buf).b_p_ep).cast(),
                kOptKeywordprg => (&raw mut (*buf).b_p_kp).cast(),
                kOptPath => (&raw mut (*buf).b_p_path).cast(),
                kOptAutocomplete => (&raw mut (*buf).b_p_ac).cast(),
                kOptAutoread => (&raw mut (*buf).b_p_ar).cast(),
                kOptTags => (&raw mut (*buf).b_p_tags).cast(),
                kOptTagcase => (&raw mut (*buf).b_p_tc).cast(),
                kOptSidescrolloff => (&raw mut (*win).w_onebuf_opt.wo_siso).cast(),
                kOptScrolloff => (&raw mut (*win).w_onebuf_opt.wo_so).cast(),
                kOptScrolloffpad => (&raw mut (*win).w_onebuf_opt.wo_sop).cast(),
                kOptDefine => (&raw mut (*buf).b_p_def).cast(),
                kOptInclude => (&raw mut (*buf).b_p_inc).cast(),
                kOptCompleteopt => (&raw mut (*buf).b_p_cot).cast(),
                kOptDictionary => (&raw mut (*buf).b_p_dict).cast(),
                kOptDiffanchors => (&raw mut (*buf).b_p_dia).cast(),
                kOptThesaurus => (&raw mut (*buf).b_p_tsr).cast(),
                kOptThesaurusfunc => (&raw mut (*buf).b_p_tsrfu).cast(),
                kOptTagfunc => (&raw mut (*buf).b_p_tfu).cast(),
                kOptShowbreak => (&raw mut (*win).w_onebuf_opt.wo_sbr).cast(),
                kOptStatusline => (&raw mut (*win).w_onebuf_opt.wo_stl).cast(),
                kOptWinbar => (&raw mut (*win).w_onebuf_opt.wo_wbr).cast(),
                kOptUndolevels => (&raw mut (*buf).b_p_ul).cast(),
                kOptLispwords => (&raw mut (*buf).b_p_lw).cast(),
                kOptBackupcopy => (&raw mut (*buf).b_p_bkc).cast(),
                kOptMakeencoding => (&raw mut (*buf).b_p_menc).cast(),
                kOptFillchars => (&raw mut (*win).w_onebuf_opt.wo_fcs).cast(),
                kOptListchars => (&raw mut (*win).w_onebuf_opt.wo_lcs).cast(),
                kOptVirtualedit => (&raw mut (*win).w_onebuf_opt.wo_ve).cast(),
                _ => std::process::abort(),
            };
            return ptr;
        }

        get_varp_from(p, buf, win)
    }
}

/// Get pointer to option variable, depending on local or global scope.
pub unsafe fn get_varp_scope(p: *mut vimoption_T, opt_flags: c_int) -> *mut c_void {
    unsafe { get_varp_scope_from(p, opt_flags, curbuf, curwin) }
}

/// Get pointer to option variable at 'opt_idx', depending on local or global scope.
pub unsafe fn get_option_varp_scope_from(
    opt_idx: OptIndex,
    opt_flags: c_int,
    buf: *mut buf_T,
    win: *mut win_T,
) -> *mut c_void {
    unsafe { get_varp_scope_from(get_option(opt_idx), opt_flags, buf, win) }
}

pub unsafe fn get_varp_from(p: *mut vimoption_T, buf: *mut buf_T, win: *mut win_T) -> *mut c_void {
    unsafe {
        let opt_idx = get_opt_idx(p);

        if is_option_hidden(opt_idx) || option_is_global_only(opt_idx) {
            return (*p).var;
        }

        match opt_idx {
            kOptEqualprg => {
                if *(*buf).b_p_ep != NUL {
                    (&raw mut (*buf).b_p_ep).cast()
                } else {
                    (*p).var
                }
            }
            kOptKeywordprg => {
                if *(*buf).b_p_kp != NUL {
                    (&raw mut (*buf).b_p_kp).cast()
                } else {
                    (*p).var
                }
            }
            kOptPath => {
                if *(*buf).b_p_path != NUL {
                    (&raw mut (*buf).b_p_path).cast()
                } else {
                    (*p).var
                }
            }
            kOptAutocomplete => {
                if (*buf).b_p_ac >= 0 {
                    (&raw mut (*buf).b_p_ac).cast()
                } else {
                    (*p).var
                }
            }
            kOptAutoread => {
                if (*buf).b_p_ar >= 0 {
                    (&raw mut (*buf).b_p_ar).cast()
                } else {
                    (*p).var
                }
            }
            kOptTags => {
                if *(*buf).b_p_tags != NUL {
                    (&raw mut (*buf).b_p_tags).cast()
                } else {
                    (*p).var
                }
            }
            kOptTagcase => {
                if *(*buf).b_p_tc != NUL {
                    (&raw mut (*buf).b_p_tc).cast()
                } else {
                    (*p).var
                }
            }
            kOptSidescrolloff => {
                if (*win).w_onebuf_opt.wo_siso >= 0 {
                    (&raw mut (*win).w_onebuf_opt.wo_siso).cast()
                } else {
                    (*p).var
                }
            }
            kOptScrolloff => {
                if (*win).w_onebuf_opt.wo_so >= 0 {
                    (&raw mut (*win).w_onebuf_opt.wo_so).cast()
                } else {
                    (*p).var
                }
            }
            kOptScrolloffpad => {
                if (*win).w_onebuf_opt.wo_sop >= 0 {
                    (&raw mut (*win).w_onebuf_opt.wo_sop).cast()
                } else {
                    (*p).var
                }
            }
            kOptBackupcopy => {
                if *(*buf).b_p_bkc != NUL {
                    (&raw mut (*buf).b_p_bkc).cast()
                } else {
                    (*p).var
                }
            }
            kOptDefine => {
                if *(*buf).b_p_def != NUL {
                    (&raw mut (*buf).b_p_def).cast()
                } else {
                    (*p).var
                }
            }
            kOptInclude => {
                if *(*buf).b_p_inc != NUL {
                    (&raw mut (*buf).b_p_inc).cast()
                } else {
                    (*p).var
                }
            }
            kOptCompleteopt => {
                if *(*buf).b_p_cot != NUL {
                    (&raw mut (*buf).b_p_cot).cast()
                } else {
                    (*p).var
                }
            }
            kOptDictionary => {
                if *(*buf).b_p_dict != NUL {
                    (&raw mut (*buf).b_p_dict).cast()
                } else {
                    (*p).var
                }
            }
            kOptDiffanchors => {
                if *(*buf).b_p_dia != NUL {
                    (&raw mut (*buf).b_p_dia).cast()
                } else {
                    (*p).var
                }
            }
            kOptThesaurus => {
                if *(*buf).b_p_tsr != NUL {
                    (&raw mut (*buf).b_p_tsr).cast()
                } else {
                    (*p).var
                }
            }
            kOptThesaurusfunc => {
                if (*buf).b_p_tsrfu.cb_type() != kCallbackNone {
                    (&raw mut (*buf).b_p_tsrfu).cast()
                } else {
                    (*p).var
                }
            }
            kOptFormatprg => {
                if *(*buf).b_p_fp != NUL {
                    (&raw mut (*buf).b_p_fp).cast()
                } else {
                    (*p).var
                }
            }
            kOptFsync => {
                if (*buf).b_p_fs >= 0 {
                    (&raw mut (*buf).b_p_fs).cast()
                } else {
                    (*p).var
                }
            }
            kOptFindfunc => {
                if (*buf).b_p_ffu.cb_type() != kCallbackNone {
                    (&raw mut (*buf).b_p_ffu).cast()
                } else {
                    (*p).var
                }
            }
            kOptErrorformat => {
                if *(*buf).b_p_efm != NUL {
                    (&raw mut (*buf).b_p_efm).cast()
                } else {
                    (*p).var
                }
            }
            kOptGrepformat => {
                if *(*buf).b_p_gefm != NUL {
                    (&raw mut (*buf).b_p_gefm).cast()
                } else {
                    (*p).var
                }
            }
            kOptGrepprg => {
                if *(*buf).b_p_gp != NUL {
                    (&raw mut (*buf).b_p_gp).cast()
                } else {
                    (*p).var
                }
            }
            kOptMakeprg => {
                if *(*buf).b_p_mp != NUL {
                    (&raw mut (*buf).b_p_mp).cast()
                } else {
                    (*p).var
                }
            }
            kOptShowbreak => {
                if *(*win).w_onebuf_opt.wo_sbr != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_sbr).cast()
                } else {
                    (*p).var
                }
            }
            kOptStatusline => {
                if *(*win).w_onebuf_opt.wo_stl != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_stl).cast()
                } else {
                    (*p).var
                }
            }
            kOptWinbar => {
                if *(*win).w_onebuf_opt.wo_wbr != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_wbr).cast()
                } else {
                    (*p).var
                }
            }
            kOptUndolevels => {
                if (*buf).b_p_ul != NO_LOCAL_UNDOLEVEL {
                    (&raw mut (*buf).b_p_ul).cast()
                } else {
                    (*p).var
                }
            }
            kOptLispwords => {
                if *(*buf).b_p_lw != NUL {
                    (&raw mut (*buf).b_p_lw).cast()
                } else {
                    (*p).var
                }
            }
            kOptMakeencoding => {
                if *(*buf).b_p_menc != NUL {
                    (&raw mut (*buf).b_p_menc).cast()
                } else {
                    (*p).var
                }
            }
            kOptFillchars => {
                if *(*win).w_onebuf_opt.wo_fcs != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_fcs).cast()
                } else {
                    (*p).var
                }
            }
            kOptListchars => {
                if *(*win).w_onebuf_opt.wo_lcs != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_lcs).cast()
                } else {
                    (*p).var
                }
            }
            kOptVirtualedit => {
                if *(*win).w_onebuf_opt.wo_ve != NUL {
                    (&raw mut (*win).w_onebuf_opt.wo_ve).cast()
                } else {
                    (*p).var
                }
            }

            kOptArabic => (&raw mut (*win).w_onebuf_opt.wo_arab).cast(),
            kOptList => (&raw mut (*win).w_onebuf_opt.wo_list).cast(),
            kOptSpell => (&raw mut (*win).w_onebuf_opt.wo_spell).cast(),
            kOptCursorcolumn => (&raw mut (*win).w_onebuf_opt.wo_cuc).cast(),
            kOptCursorline => (&raw mut (*win).w_onebuf_opt.wo_cul).cast(),
            kOptCursorlineopt => (&raw mut (*win).w_onebuf_opt.wo_culopt).cast(),
            kOptColorcolumn => (&raw mut (*win).w_onebuf_opt.wo_cc).cast(),
            kOptDiff => (&raw mut (*win).w_onebuf_opt.wo_diff).cast(),
            kOptEventignorewin => (&raw mut (*win).w_onebuf_opt.wo_eiw).cast(),
            kOptFoldcolumn => (&raw mut (*win).w_onebuf_opt.wo_fdc).cast(),
            kOptFoldenable => (&raw mut (*win).w_onebuf_opt.wo_fen).cast(),
            kOptFoldignore => (&raw mut (*win).w_onebuf_opt.wo_fdi).cast(),
            kOptFoldlevel => (&raw mut (*win).w_onebuf_opt.wo_fdl).cast(),
            kOptFoldmethod => (&raw mut (*win).w_onebuf_opt.wo_fdm).cast(),
            kOptFoldminlines => (&raw mut (*win).w_onebuf_opt.wo_fml).cast(),
            kOptFoldnestmax => (&raw mut (*win).w_onebuf_opt.wo_fdn).cast(),
            kOptFoldexpr => (&raw mut (*win).w_onebuf_opt.wo_fde).cast(),
            kOptFoldtext => (&raw mut (*win).w_onebuf_opt.wo_fdt).cast(),
            kOptFoldmarker => (&raw mut (*win).w_onebuf_opt.wo_fmr).cast(),
            kOptNumber => (&raw mut (*win).w_onebuf_opt.wo_nu).cast(),
            kOptRelativenumber => (&raw mut (*win).w_onebuf_opt.wo_rnu).cast(),
            kOptNumberwidth => (&raw mut (*win).w_onebuf_opt.wo_nuw).cast(),
            kOptWinfixbuf => (&raw mut (*win).w_onebuf_opt.wo_wfb).cast(),
            kOptWinfixheight => (&raw mut (*win).w_onebuf_opt.wo_wfh).cast(),
            kOptWinfixwidth => (&raw mut (*win).w_onebuf_opt.wo_wfw).cast(),
            kOptWinpinned => (&raw mut (*win).w_onebuf_opt.wo_wp).cast(),
            kOptPreviewwindow => (&raw mut (*win).w_onebuf_opt.wo_pvw).cast(),
            kOptLhistory => (&raw mut (*win).w_onebuf_opt.wo_lhi).cast(),
            kOptRightleft => (&raw mut (*win).w_onebuf_opt.wo_rl).cast(),
            kOptRightleftcmd => (&raw mut (*win).w_onebuf_opt.wo_rlc).cast(),
            kOptScroll => (&raw mut (*win).w_onebuf_opt.wo_scr).cast(),
            kOptSmoothscroll => (&raw mut (*win).w_onebuf_opt.wo_sms).cast(),
            kOptWrap => (&raw mut (*win).w_onebuf_opt.wo_wrap).cast(),
            kOptLinebreak => (&raw mut (*win).w_onebuf_opt.wo_lbr).cast(),
            kOptBreakindent => (&raw mut (*win).w_onebuf_opt.wo_bri).cast(),
            kOptBreakindentopt => (&raw mut (*win).w_onebuf_opt.wo_briopt).cast(),
            kOptScrollbind => (&raw mut (*win).w_onebuf_opt.wo_scb).cast(),
            kOptCursorbind => (&raw mut (*win).w_onebuf_opt.wo_crb).cast(),
            kOptConcealcursor => (&raw mut (*win).w_onebuf_opt.wo_cocu).cast(),
            kOptConceallevel => (&raw mut (*win).w_onebuf_opt.wo_cole).cast(),

            kOptAutoindent => (&raw mut (*buf).b_p_ai).cast(),
            kOptBinary => (&raw mut (*buf).b_p_bin).cast(),
            kOptBomb => (&raw mut (*buf).b_p_bomb).cast(),
            kOptBufhidden => (&raw mut (*buf).b_p_bh).cast(),
            kOptBuftype => (&raw mut (*buf).b_p_bt).cast(),
            kOptBuflisted => (&raw mut (*buf).b_p_bl).cast(),
            kOptBusy => (&raw mut (*buf).b_p_busy).cast(),
            kOptChannel => (&raw mut (*buf).b_p_channel).cast(),
            kOptCopyindent => (&raw mut (*buf).b_p_ci).cast(),
            kOptCindent => (&raw mut (*buf).b_p_cin).cast(),
            kOptCinkeys => (&raw mut (*buf).b_p_cink).cast(),
            kOptCinoptions => (&raw mut (*buf).b_p_cino).cast(),
            kOptCinscopedecls => (&raw mut (*buf).b_p_cinsd).cast(),
            kOptCinwords => (&raw mut (*buf).b_p_cinw).cast(),
            kOptComments => (&raw mut (*buf).b_p_com).cast(),
            kOptCommentstring => (&raw mut (*buf).b_p_cms).cast(),
            kOptComplete => (&raw mut (*buf).b_p_cpt).cast(),
            #[cfg(windows)]
            kOptCompleteslash => (&raw mut (*buf).b_p_csl).cast(),
            kOptCompletefunc => (&raw mut (*buf).b_p_cfu).cast(),
            kOptOmnifunc => (&raw mut (*buf).b_p_ofu).cast(),
            kOptEndoffile => (&raw mut (*buf).b_p_eof).cast(),
            kOptEndofline => (&raw mut (*buf).b_p_eol).cast(),
            kOptFixendofline => (&raw mut (*buf).b_p_fixeol).cast(),
            kOptExpandtab => (&raw mut (*buf).b_p_et).cast(),
            kOptFileencoding => (&raw mut (*buf).b_p_fenc).cast(),
            kOptFileformat => (&raw mut (*buf).b_p_ff).cast(),
            kOptFiletype => (&raw mut (*buf).b_p_ft).cast(),
            kOptFollow => (&raw mut (*buf).b_p_follow).cast(),
            kOptFormatoptions => (&raw mut (*buf).b_p_fo).cast(),
            kOptFormatlistpat => (&raw mut (*buf).b_p_flp).cast(),
            kOptIminsert => (&raw mut (*buf).b_p_iminsert).cast(),
            kOptImsearch => (&raw mut (*buf).b_p_imsearch).cast(),
            kOptInfercase => (&raw mut (*buf).b_p_inf).cast(),
            kOptIskeyword => (&raw mut (*buf).b_p_isk).cast(),
            kOptIncludeexpr => (&raw mut (*buf).b_p_inex).cast(),
            kOptIndentexpr => (&raw mut (*buf).b_p_inde).cast(),
            kOptIndentkeys => (&raw mut (*buf).b_p_indk).cast(),
            kOptFormatexpr => (&raw mut (*buf).b_p_fex).cast(),
            kOptLisp => (&raw mut (*buf).b_p_lisp).cast(),
            kOptLispoptions => (&raw mut (*buf).b_p_lop).cast(),
            kOptModeline => (&raw mut (*buf).b_p_ml).cast(),
            kOptMatchpairs => (&raw mut (*buf).b_p_mps).cast(),
            kOptModifiable => (&raw mut (*buf).b_p_ma).cast(),
            kOptModified => (&raw mut (*buf).b_changed).cast(),
            kOptNrformats => (&raw mut (*buf).b_p_nf).cast(),
            kOptPreserveindent => (&raw mut (*buf).b_p_pi).cast(),
            kOptQuoteescape => (&raw mut (*buf).b_p_qe).cast(),
            kOptReadonly => (&raw mut (*buf).b_p_ro).cast(),
            kOptScrollback => (&raw mut (*buf).b_p_scbk).cast(),
            kOptSmartindent => (&raw mut (*buf).b_p_si).cast(),
            kOptSofttabstop => (&raw mut (*buf).b_p_sts).cast(),
            kOptSuffixesadd => (&raw mut (*buf).b_p_sua).cast(),
            kOptSwapfile => (&raw mut (*buf).b_p_swf).cast(),
            kOptSynmaxcol => (&raw mut (*buf).b_p_smc).cast(),
            kOptSyntax => (&raw mut (*buf).b_p_syn).cast(),
            kOptSpellcapcheck => {
                let ws = (*win).w_s.cast::<synblock_T>();
                (&raw mut (*ws).b_p_spc).cast()
            }
            kOptSpellfile => {
                let ws = (*win).w_s.cast::<synblock_T>();
                (&raw mut (*ws).b_p_spf).cast()
            }
            kOptSpelllang => {
                let ws = (*win).w_s.cast::<synblock_T>();
                (&raw mut (*ws).b_p_spl).cast()
            }
            kOptSpelloptions => {
                let ws = (*win).w_s.cast::<synblock_T>();
                (&raw mut (*ws).b_p_spo).cast()
            }
            kOptShiftwidth => (&raw mut (*buf).b_p_sw).cast(),
            kOptTagfunc => (&raw mut (*buf).b_p_tfu).cast(),
            kOptTabstop => (&raw mut (*buf).b_p_ts).cast(),
            kOptTextwidth => (&raw mut (*buf).b_p_tw).cast(),
            kOptUndofile => (&raw mut (*buf).b_p_udf).cast(),
            kOptWrapmargin => (&raw mut (*buf).b_p_wm).cast(),
            kOptVarsofttabstop => (&raw mut (*buf).b_p_vsts).cast(),
            kOptVartabstop => (&raw mut (*buf).b_p_vts).cast(),
            kOptKeymap => (&raw mut (*buf).b_p_keymap).cast(),
            kOptSigncolumn => (&raw mut (*win).w_onebuf_opt.wo_scl).cast(),
            kOptWinhighlight => (&raw mut (*win).w_onebuf_opt.wo_winhl).cast(),
            kOptWinblend => (&raw mut (*win).w_onebuf_opt.wo_winbl).cast(),
            kOptStatuscolumn => (&raw mut (*win).w_onebuf_opt.wo_stc).cast(),
            _ => {
                iemsg(tr(c"E356: get_varp ERROR".as_ptr()));
                (&raw mut (*buf).b_p_wm).cast()
            }
        }
    }
}

/// Get the value of 'equalprg', either the buffer-local one or the global one.
pub unsafe fn get_equalprg() -> *mut c_char {
    unsafe {
        if *(*curbuf).b_p_ep == NUL {
            p_ep
        } else {
            (*curbuf).b_p_ep
        }
    }
}

/// Get the value of 'findfunc', either the buffer-local one or the global one.
pub unsafe fn get_findfunc() -> *mut Callback {
    unsafe {
        if (*curbuf).b_p_ffu.cb_type() != kCallbackNone {
            &raw mut (*curbuf).b_p_ffu
        } else {
            &raw mut p_ffu
        }
    }
}

/// Copy options from one window to another.
pub unsafe fn win_copy_options(wp_from: *mut win_T, wp_to: *mut win_T) {
    unsafe {
        copy_winopt(
            &raw mut (*wp_from).w_onebuf_opt,
            &raw mut (*wp_to).w_onebuf_opt,
        );
        copy_winopt(
            &raw mut (*wp_from).w_allbuf_opt,
            &raw mut (*wp_to).w_allbuf_opt,
        );
        didset_window_options(wp_to, true);
    }
}

unsafe fn copy_option_val(val: *const c_char) -> *mut c_char {
    unsafe {
        let empty_opt = (&raw mut empty_string_option).cast::<c_char>();
        if ptr::eq(val, empty_opt) {
            empty_opt
        } else {
            alloc_string_option(val)
        }
    }
}

/// Copy the options from one winopt_T to another.
pub unsafe fn copy_winopt(from: *mut winopt_T, to: *mut winopt_T) {
    unsafe {
        let empty_opt = (&raw mut empty_string_option).cast::<c_char>();
        (*to).wo_arab = (*from).wo_arab;
        (*to).wo_list = (*from).wo_list;
        (*to).wo_lcs = copy_option_val((*from).wo_lcs);
        (*to).wo_fcs = copy_option_val((*from).wo_fcs);
        (*to).wo_nu = (*from).wo_nu;
        (*to).wo_rnu = (*from).wo_rnu;
        (*to).wo_ve = copy_option_val((*from).wo_ve);
        (*to).wo_ve_flags = (*from).wo_ve_flags;
        (*to).wo_nuw = (*from).wo_nuw;
        (*to).wo_rl = (*from).wo_rl;
        (*to).wo_rlc = copy_option_val((*from).wo_rlc);
        (*to).wo_sbr = copy_option_val((*from).wo_sbr);
        (*to).wo_stl = copy_option_val((*from).wo_stl);
        (*to).wo_wbr = copy_option_val((*from).wo_wbr);
        (*to).wo_wrap = (*from).wo_wrap;
        (*to).wo_wrap_save = (*from).wo_wrap_save;
        (*to).wo_lbr = (*from).wo_lbr;
        (*to).wo_bri = (*from).wo_bri;
        (*to).wo_briopt = copy_option_val((*from).wo_briopt);
        (*to).wo_scb = (*from).wo_scb;
        (*to).wo_scb_save = (*from).wo_scb_save;
        (*to).wo_sms = (*from).wo_sms;
        (*to).wo_crb = (*from).wo_crb;
        (*to).wo_crb_save = (*from).wo_crb_save;
        (*to).wo_siso = (*from).wo_siso;
        (*to).wo_so = (*from).wo_so;
        (*to).wo_sop = (*from).wo_sop;
        (*to).wo_spell = (*from).wo_spell;
        (*to).wo_cuc = (*from).wo_cuc;
        (*to).wo_cul = (*from).wo_cul;
        (*to).wo_culopt = copy_option_val((*from).wo_culopt);
        (*to).wo_cc = copy_option_val((*from).wo_cc);
        (*to).wo_diff = (*from).wo_diff;
        (*to).wo_diff_saved = (*from).wo_diff_saved;
        (*to).wo_eiw = copy_option_val((*from).wo_eiw);
        (*to).wo_cocu = copy_option_val((*from).wo_cocu);
        (*to).wo_cole = (*from).wo_cole;
        (*to).wo_fdc = copy_option_val((*from).wo_fdc);
        (*to).wo_fdc_save = if (*from).wo_diff_saved != 0 {
            alloc_string_option((*from).wo_fdc_save)
        } else {
            empty_opt
        };
        (*to).wo_fen = (*from).wo_fen;
        (*to).wo_fen_save = (*from).wo_fen_save;
        (*to).wo_fdi = copy_option_val((*from).wo_fdi);
        (*to).wo_fml = (*from).wo_fml;
        (*to).wo_fdl = (*from).wo_fdl;
        (*to).wo_fdl_save = (*from).wo_fdl_save;
        (*to).wo_fdm = copy_option_val((*from).wo_fdm);
        (*to).wo_fdm_save = if (*from).wo_diff_saved != 0 {
            alloc_string_option((*from).wo_fdm_save)
        } else {
            empty_opt
        };
        (*to).wo_fdn = (*from).wo_fdn;
        callback_copy(
            &raw mut (*to).wo_fde,
            (&raw const (*from).wo_fde).cast_mut(),
        );
        callback_copy(
            &raw mut (*to).wo_fdt,
            (&raw const (*from).wo_fdt).cast_mut(),
        );
        (*to).wo_fmr = copy_option_val((*from).wo_fmr);
        (*to).wo_scl = copy_option_val((*from).wo_scl);
        (*to).wo_lhi = (*from).wo_lhi;
        (*to).wo_winhl = copy_option_val((*from).wo_winhl);
        (*to).wo_winbl = (*from).wo_winbl;
        (*to).wo_stc = copy_option_val((*from).wo_stc);

        (*to).wo_wrap_flags = (*from).wo_wrap_flags;
        (*to).wo_stl_flags = (*from).wo_stl_flags;
        (*to).wo_wbr_flags = (*from).wo_wbr_flags;
        (*to).wo_fde_flags = (*from).wo_fde_flags;
        (*to).wo_fdt_flags = (*from).wo_fdt_flags;

        (*to).wo_script_ctx = (*from).wo_script_ctx;
        check_winopt(to);
    }
}

unsafe fn check_win_options(win: *mut win_T) {
    unsafe {
        check_winopt(&raw mut (*win).w_onebuf_opt);
        check_winopt(&raw mut (*win).w_allbuf_opt);
    }
}

unsafe fn check_winopt(wop: *mut winopt_T) {
    unsafe {
        check_string_option(&raw mut (*wop).wo_fdc);
        check_string_option(&raw mut (*wop).wo_fdc_save);
        check_string_option(&raw mut (*wop).wo_fdi);
        check_string_option(&raw mut (*wop).wo_fdm);
        check_string_option(&raw mut (*wop).wo_fdm_save);
        check_string_option(&raw mut (*wop).wo_fmr);
        check_string_option(&raw mut (*wop).wo_eiw);
        check_string_option(&raw mut (*wop).wo_scl);
        check_string_option(&raw mut (*wop).wo_rlc);
        check_string_option(&raw mut (*wop).wo_sbr);
        check_string_option(&raw mut (*wop).wo_stl);
        check_string_option(&raw mut (*wop).wo_culopt);
        check_string_option(&raw mut (*wop).wo_cc);
        check_string_option(&raw mut (*wop).wo_cocu);
        check_string_option(&raw mut (*wop).wo_briopt);
        check_string_option(&raw mut (*wop).wo_winhl);
        check_string_option(&raw mut (*wop).wo_lcs);
        check_string_option(&raw mut (*wop).wo_fcs);
        check_string_option(&raw mut (*wop).wo_ve);
        check_string_option(&raw mut (*wop).wo_wbr);
        check_string_option(&raw mut (*wop).wo_stc);
    }
}

/// Free the allocated memory inside a winopt_T.
pub unsafe fn clear_winopt(wop: *mut winopt_T) {
    unsafe {
        clear_string_option(&raw mut (*wop).wo_fdc);
        clear_string_option(&raw mut (*wop).wo_fdc_save);
        clear_string_option(&raw mut (*wop).wo_fdi);
        clear_string_option(&raw mut (*wop).wo_fdm);
        clear_string_option(&raw mut (*wop).wo_fdm_save);
        callback_free(&raw mut (*wop).wo_fde);
        callback_free(&raw mut (*wop).wo_fdt);
        clear_string_option(&raw mut (*wop).wo_fmr);
        clear_string_option(&raw mut (*wop).wo_eiw);
        clear_string_option(&raw mut (*wop).wo_scl);
        clear_string_option(&raw mut (*wop).wo_rlc);
        clear_string_option(&raw mut (*wop).wo_sbr);
        clear_string_option(&raw mut (*wop).wo_stl);
        clear_string_option(&raw mut (*wop).wo_culopt);
        clear_string_option(&raw mut (*wop).wo_cc);
        clear_string_option(&raw mut (*wop).wo_cocu);
        clear_string_option(&raw mut (*wop).wo_briopt);
        clear_string_option(&raw mut (*wop).wo_winhl);
        clear_string_option(&raw mut (*wop).wo_lcs);
        clear_string_option(&raw mut (*wop).wo_fcs);
        clear_string_option(&raw mut (*wop).wo_ve);
        clear_string_option(&raw mut (*wop).wo_wbr);
        clear_string_option(&raw mut (*wop).wo_stc);
    }
}

pub unsafe fn didset_window_options(wp: *mut win_T, valid_cursor: bool) {
    unsafe {
        if (*wp).w_onebuf_opt.wo_wrap != 0 {
            (*wp).w_leftcol = 0;
        } else {
            (*wp).w_skipcol = 0;
        }
        check_colorcolumn(ptr::null_mut(), wp);
        briopt_check(wp);
        let mut culopt_flags: c_uint = 0;
        if fill_culopt_flags((*wp).w_onebuf_opt.wo_culopt, &raw mut culopt_flags) == OK {
            (*wp).w_p_culopt_flags = culopt_flags as u8;
        }
        set_chars_option(
            wp,
            (*wp).w_onebuf_opt.wo_fcs,
            CharsOption::kFillchars,
            true,
            ptr::null(),
        );
        set_chars_option(
            wp,
            (*wp).w_onebuf_opt.wo_lcs,
            CharsOption::kListchars,
            true,
            ptr::null(),
        );
        parse_winhl_opt(ptr::null(), wp);
        check_blending(wp);
        set_winbar_win(wp, false, valid_cursor);
        check_signcolumn(ptr::null_mut(), wp);
        (*wp).w_grid_alloc.blending = (*wp).w_onebuf_opt.wo_winbl > 0;
    }
}

#[inline]
unsafe fn copy_opt_sctx(buf: *mut buf_T, bv: usize) {
    unsafe {
        (*buf).b_p_script_ctx[bv] = (*get_option(buf_opt_idx[bv])).script_ctx;
    }
}

#[inline]
unsafe fn copy_opt_insecure(bv: usize) -> u32 {
    unsafe { (*get_option(buf_opt_idx[bv])).flags & kOptFlagInsecure }
}

/// Copy global option values to local options for one buffer.
pub unsafe fn buf_copy_options(buf: *mut buf_T, flags: c_int) {
    unsafe {
        let mut should_copy = true;
        let mut save_p_isk: *mut c_char = ptr::null_mut();
        let mut did_isk = false;
        let empty_opt = (&raw mut empty_string_option).cast::<c_char>();

        if !p_cpo.is_null() {
            if (vim_strchr(p_cpo, kCpoBufoptglob).is_null() || (flags & BCO_ENTER) == 0)
                && ((*buf).b_p_initialized
                    || ((flags & BCO_ENTER) == 0 && !vim_strchr(p_cpo, kCpoBufopt).is_null()))
            {
                should_copy = false;
            }

            if should_copy || (flags & BCO_ALWAYS) != 0 {
                (*buf).b_p_script_ctx = [sctx_T {
                    sc_sid: 0,
                    sc_seq: 0,
                    sc_lnum: 0,
                    sc_chan: 0,
                }; 93];
                let dont_do_help =
                    ((flags & BCO_NOHELP) != 0 && (*buf).b_help) || (*buf).b_p_initialized;
                if dont_do_help {
                    save_p_isk = (*buf).b_p_isk;
                    (*buf).b_p_isk = ptr::null_mut();
                }
                if !(*buf).b_p_initialized {
                    free_buf_options(buf, true);
                    (*buf).b_p_ro = 0;
                    (*buf).b_p_fenc = alloc_string_option(p_fenc);
                    (*buf).b_p_ff = match *p_ffs as u8 {
                        b'm' => alloc_string_option(c"mac".as_ptr()),
                        b'd' => alloc_string_option(c"dos".as_ptr()),
                        b'u' => alloc_string_option(c"unix".as_ptr()),
                        _ => alloc_string_option(p_ff),
                    };
                    (*buf).b_p_bh = empty_opt;
                    (*buf).b_p_bt = empty_opt;
                } else {
                    free_buf_options(buf, false);
                }

                (*buf).b_p_ai = p_ai;
                copy_opt_sctx(buf, kBufOptAutoindent);
                (*buf).b_p_ai_nopaste = P_AI_NOPASTE;
                (*buf).b_p_sw = p_sw;
                copy_opt_sctx(buf, kBufOptShiftwidth);
                (*buf).b_p_scbk = p_scbk;
                copy_opt_sctx(buf, kBufOptScrollback);
                (*buf).b_p_tw = p_tw;
                copy_opt_sctx(buf, kBufOptTextwidth);
                (*buf).b_p_tw_nopaste = P_TW_NOPASTE;
                (*buf).b_p_tw_nobin = P_TW_NOBIN;
                (*buf).b_p_wm = p_wm;
                copy_opt_sctx(buf, kBufOptWrapmargin);
                (*buf).b_p_wm_nopaste = P_WM_NOPASTE;
                (*buf).b_p_wm_nobin = P_WM_NOBIN;
                (*buf).b_p_bin = p_bin;
                copy_opt_sctx(buf, kBufOptBinary);
                (*buf).b_p_bomb = p_bomb;
                copy_opt_sctx(buf, kBufOptBomb);
                (*buf).b_p_et = p_et;
                copy_opt_sctx(buf, kBufOptExpandtab);
                (*buf).b_p_fixeol = p_fixeol;
                copy_opt_sctx(buf, kBufOptFixendofline);
                (*buf).b_p_follow = p_follow;
                copy_opt_sctx(buf, kBufOptFollow);
                (*buf).b_p_et_nobin = P_ET_NOBIN;
                (*buf).b_p_et_nopaste = P_ET_NOPASTE;
                (*buf).b_p_ml = p_ml;
                copy_opt_sctx(buf, kBufOptModeline);
                (*buf).b_p_ml_nobin = P_ML_NOBIN;
                (*buf).b_p_inf = p_inf;
                copy_opt_sctx(buf, kBufOptInfercase);
                if (cmdmod.cmod_flags & CMOD_NOSWAPFILE) != 0 {
                    (*buf).b_p_swf = 0;
                } else {
                    (*buf).b_p_swf = p_swf;
                    copy_opt_sctx(buf, kBufOptSwapfile);
                }
                (*buf).b_p_cpt = alloc_string_option(p_cpt);
                copy_opt_sctx(buf, kBufOptComplete);
                (*buf).b_p_cpt_flags = copy_opt_insecure(kBufOptComplete);
                set_buflocal_cpt_callbacks(buf);
                #[cfg(windows)]
                {
                    (*buf).b_p_csl = alloc_string_option(p_csl);
                    copy_opt_sctx(buf, kBufOptCompleteslash);
                }
                callback_copy(&raw mut (*buf).b_p_cfu, &raw mut p_cfu);
                copy_opt_sctx(buf, kBufOptCompletefunc);
                callback_copy(&raw mut (*buf).b_p_ofu, &raw mut p_ofu);
                copy_opt_sctx(buf, kBufOptOmnifunc);
                callback_copy(&raw mut (*buf).b_p_tfu, &raw mut p_tfu);
                copy_opt_sctx(buf, kBufOptTagfunc);
                (*buf).b_p_sts = p_sts;
                copy_opt_sctx(buf, kBufOptSofttabstop);
                (*buf).b_p_sts_nopaste = P_STS_NOPASTE;
                (*buf).b_p_vsts = alloc_string_option(p_vsts);
                copy_opt_sctx(buf, kBufOptVarsofttabstop);
                if !p_vsts.is_null() && p_vsts != empty_opt {
                    tabstop_set(p_vsts, &mut (*buf).b_p_vsts_array, ptr::null());
                } else {
                    (*buf).b_p_vsts_array.clear();
                }
                let vsts_np_ptr = &raw const P_VSTS_NOPASTE;
                (*buf).b_p_vsts_nopaste = (*vsts_np_ptr).clone();
                (*buf).b_p_com = alloc_string_option(p_com);
                copy_opt_sctx(buf, kBufOptComments);
                (*buf).b_p_cms = alloc_string_option(p_cms);
                copy_opt_sctx(buf, kBufOptCommentstring);
                (*buf).b_p_fo = alloc_string_option(p_fo);
                copy_opt_sctx(buf, kBufOptFormatoptions);
                (*buf).b_p_flp = alloc_string_option(p_flp);
                copy_opt_sctx(buf, kBufOptFormatlistpat);
                (*buf).b_p_nf = alloc_string_option(p_nf);
                copy_opt_sctx(buf, kBufOptNrformats);
                (*buf).b_p_mps = alloc_string_option(p_mps);
                copy_opt_sctx(buf, kBufOptMatchpairs);
                (*buf).b_p_si = p_si;
                copy_opt_sctx(buf, kBufOptSmartindent);
                (*buf).b_p_channel = 0;
                (*buf).b_p_ci = p_ci;
                copy_opt_sctx(buf, kBufOptCopyindent);
                (*buf).b_p_cin = p_cin;
                copy_opt_sctx(buf, kBufOptCindent);
                (*buf).b_p_cink = alloc_string_option(p_cink);
                copy_opt_sctx(buf, kBufOptCinkeys);
                (*buf).b_p_cino = alloc_string_option(p_cino);
                copy_opt_sctx(buf, kBufOptCinoptions);
                (*buf).b_p_cinsd = alloc_string_option(p_cinsd);
                copy_opt_sctx(buf, kBufOptCinscopedecls);
                (*buf).b_p_lop = alloc_string_option(p_lop);
                copy_opt_sctx(buf, kBufOptLispoptions);

                (*buf).b_p_ft = empty_opt;
                (*buf).b_p_pi = p_pi;
                copy_opt_sctx(buf, kBufOptPreserveindent);
                (*buf).b_p_cinw = alloc_string_option(p_cinw);
                copy_opt_sctx(buf, kBufOptCinwords);
                (*buf).b_p_lisp = p_lisp;
                copy_opt_sctx(buf, kBufOptLisp);
                (*buf).b_p_syn = empty_opt;
                (*buf).b_p_smc = p_smc;
                copy_opt_sctx(buf, kBufOptSynmaxcol);
                (*buf).b_s.b_syn_isk = None;
                (*buf).b_s.b_p_spc = alloc_string_option(p_spc);
                copy_opt_sctx(buf, kBufOptSpellcapcheck);
                compile_cap_prog((*buf).b_s.b_p_spc, &mut (*buf).b_s.b_cap_prog);
                (*buf).b_s.b_p_spf = alloc_string_option(p_spf);
                copy_opt_sctx(buf, kBufOptSpellfile);
                (*buf).b_s.b_p_spl = alloc_string_option(p_spl);
                copy_opt_sctx(buf, kBufOptSpelllang);
                (*buf).b_s.b_p_spo = alloc_string_option(p_spo);
                copy_opt_sctx(buf, kBufOptSpelloptions);
                (*buf).b_s.b_p_spo_flags = spo_flags;
                callback_copy(&raw mut (*buf).b_p_inde, &raw mut p_inde);
                copy_opt_sctx(buf, kBufOptIndentexpr);
                (*buf).b_p_inde_flags = copy_opt_insecure(kBufOptIndentexpr);
                (*buf).b_p_indk = alloc_string_option(p_indk);
                copy_opt_sctx(buf, kBufOptIndentkeys);
                (*buf).b_p_fp = empty_opt;
                callback_copy(&raw mut (*buf).b_p_fex, &raw mut p_fex);
                copy_opt_sctx(buf, kBufOptFormatexpr);
                (*buf).b_p_fex_flags = copy_opt_insecure(kBufOptFormatexpr);
                (*buf).b_p_sua = alloc_string_option(p_sua);
                copy_opt_sctx(buf, kBufOptSuffixesadd);
                (*buf).b_p_keymap = alloc_string_option(p_keymap);
                copy_opt_sctx(buf, kBufOptKeymap);
                (*buf).b_kmap_state |= KEYMAP_INIT;
                (*buf).b_p_iminsert = p_iminsert;
                copy_opt_sctx(buf, kBufOptIminsert);
                (*buf).b_p_imsearch = p_imsearch;
                copy_opt_sctx(buf, kBufOptImsearch);

                (*buf).b_p_ac = -1;
                (*buf).b_p_ar = -1;
                (*buf).b_p_fs = -1;
                (*buf).b_p_ul = NO_LOCAL_UNDOLEVEL;
                (*buf).b_p_bkc = empty_opt;
                (*buf).b_bkc_flags = 0;
                (*buf).b_p_gefm = empty_opt;
                (*buf).b_p_gp = empty_opt;
                (*buf).b_p_mp = empty_opt;
                (*buf).b_p_efm = empty_opt;
                (*buf).b_p_ep = empty_opt;
                (*buf).b_p_ffu = Callback::NONE;
                (*buf).b_p_kp = empty_opt;
                (*buf).b_p_path = empty_opt;
                (*buf).b_p_tags = empty_opt;
                (*buf).b_p_tc = empty_opt;
                (*buf).b_tc_flags = 0;
                (*buf).b_p_def = empty_opt;
                (*buf).b_p_inc = empty_opt;
                callback_copy(&raw mut (*buf).b_p_inex, &raw mut p_inex);
                copy_opt_sctx(buf, kBufOptIncludeexpr);
                (*buf).b_p_inex_flags = copy_opt_insecure(kBufOptIncludeexpr);
                (*buf).b_p_cot = empty_opt;
                (*buf).b_cot_flags = 0;
                (*buf).b_p_dict = empty_opt;
                (*buf).b_p_dia = empty_opt;
                (*buf).b_p_tsr = empty_opt;
                (*buf).b_p_tsrfu = Callback::NONE;
                (*buf).b_p_qe = alloc_string_option(p_qe);
                copy_opt_sctx(buf, kBufOptQuoteescape);
                (*buf).b_p_udf = p_udf;
                copy_opt_sctx(buf, kBufOptUndofile);
                (*buf).b_p_lw = empty_opt;
                (*buf).b_p_menc = empty_opt;

                if dont_do_help {
                    (*buf).b_p_isk = save_p_isk;
                    if !p_vts.is_null() && *p_vts != NUL && (*buf).b_p_vts_array.is_empty() {
                        tabstop_set(p_vts, &mut (*buf).b_p_vts_array, ptr::null());
                    } else {
                        (*buf).b_p_vts_array.clear();
                    }
                } else {
                    (*buf).b_p_isk = alloc_string_option(p_isk);
                    copy_opt_sctx(buf, kBufOptIskeyword);
                    did_isk = true;
                    (*buf).b_p_ts = p_ts;
                    copy_opt_sctx(buf, kBufOptTabstop);
                    (*buf).b_p_vts = alloc_string_option(p_vts);
                    copy_opt_sctx(buf, kBufOptVartabstop);
                    if !p_vts.is_null() && *p_vts != NUL && (*buf).b_p_vts_array.is_empty() {
                        tabstop_set(p_vts, &mut (*buf).b_p_vts_array, ptr::null());
                    } else {
                        (*buf).b_p_vts_array.clear();
                    }
                    (*buf).b_help = false;
                    if *(*buf).b_p_bt == b'h' as c_char {
                        clear_string_option(&raw mut (*buf).b_p_bt);
                    }
                    (*buf).b_p_ma = p_ma;
                    copy_opt_sctx(buf, kBufOptModifiable);
                }
            }

            if should_copy {
                (*buf).b_p_initialized = true;
            }
        }

        check_buf_options(buf);
        if did_isk {
            buf_init_isk_chartab(buf);
        }
    }
}

/// Reset the 'modifiable' option and its default value.
pub unsafe fn reset_modifiable() {
    unsafe {
        (*curbuf).b_p_ma = 0;
        p_ma = 0;
        change_option_default(kOptModifiable, boolean_obj(false));
    }
}

/// Set the global value for 'iminsert' to the local value.
pub unsafe fn set_iminsert_global(buf: *mut buf_T) {
    unsafe {
        p_iminsert = (*buf).b_p_iminsert;
    }
}

/// Set the global value for 'imsearch' to the local value.
pub unsafe fn set_imsearch_global(buf: *mut buf_T) {
    unsafe {
        p_imsearch = (*buf).b_p_imsearch;
    }
}

pub unsafe fn set_context_in_set_cmd(xp: *mut expand_T, mut arg: *mut c_char, opt_flags: c_int) {
    unsafe {
        EXPAND_OPTION_FLAGS = opt_flags;

        (*xp).xp_context = EXPAND_SETTINGS;
        if *arg == NUL {
            (*xp).xp_pattern = arg;
            return;
        }
        let argend = arg.add(strlen(arg));
        let mut p = argend.offset(-1);
        if *p == b' ' as c_char && *p.offset(-1) != b'\\' as c_char {
            (*xp).xp_pattern = p.add(1);
            return;
        }
        while p > arg {
            let mut s = p;
            if *p == b' ' as c_char || *p == b',' as c_char {
                while s > arg && *s.offset(-1) == b'\\' as c_char {
                    s = s.offset(-1);
                }
            }
            if *p == b' ' as c_char && (p.offset_from(s) & 1) == 0 {
                p = p.add(1);
                break;
            }
            p = p.offset(-1);
        }
        if strncmp(p, c"no".as_ptr(), 2) == 0 {
            (*xp).xp_context = EXPAND_BOOL_SETTINGS;
            (*xp).xp_prefix = XP_PREFIX_NO;
            p = p.add(2);
        } else if strncmp(p, c"inv".as_ptr(), 3) == 0 {
            (*xp).xp_context = EXPAND_BOOL_SETTINGS;
            (*xp).xp_prefix = XP_PREFIX_INV;
            p = p.add(3);
        }
        (*xp).xp_pattern = p;
        arg = p;

        let mut nextchar: c_char;
        let mut flags: u32 = 0;
        let mut opt_idx: OptIndex = 0;
        let mut is_term_option = false;

        if *arg == b'<' as c_char {
            while *p != b'>' as c_char {
                if *p == NUL {
                    return;
                }
                p = p.add(1);
            }
            p = p.add(1);
            let key = get_special_key_code(arg.add(1));
            if key == 0 {
                (*xp).xp_context = EXPAND_NOTHING;
                return;
            }
            nextchar = *p;
            is_term_option = true;
            EXPAND_OPTION_NAME[2] = key2termcap0(key) as c_char;
            EXPAND_OPTION_NAME[3] = key2termcap1(key) as c_char;
        } else if *p == b't' as c_char && *p.add(1) == b'_' as c_char {
            p = p.add(2);
            if *p != NUL {
                p = p.add(1);
            }
            if *p == NUL {
                return;
            }
            p = p.add(1);
            nextchar = *p;
            is_term_option = true;
            EXPAND_OPTION_NAME[2] = *p.offset(-2);
            EXPAND_OPTION_NAME[3] = *p.offset(-1);
        } else {
            while ascii_isalnum(*p) || *p == b'_' as c_char || *p == b'*' as c_char {
                p = p.add(1);
            }
            if *p == NUL {
                return;
            }
            nextchar = *p;
            opt_idx = find_option_len(arg, p.offset_from(arg) as usize);
            if opt_idx == kOptInvalid || is_option_hidden(opt_idx) {
                (*xp).xp_context = EXPAND_NOTHING;
                return;
            }
            flags = (*get_option(opt_idx)).flags;
            if option_has_type(opt_idx, ObjectType::kObjectTypeBoolean) {
                (*xp).xp_context = EXPAND_NOTHING;
                return;
            }
        }

        EXPAND_OPTION_APPEND = false;
        let mut expand_option_subtract = false;
        if (nextchar == b'-' as c_char || nextchar == b'+' as c_char || nextchar == b'^' as c_char)
            && *p.add(1) == b'=' as c_char
        {
            if nextchar == b'-' as c_char {
                expand_option_subtract = true;
            }
            if nextchar == b'+' as c_char || nextchar == b'^' as c_char {
                EXPAND_OPTION_APPEND = true;
            }
            p = p.add(1);
            nextchar = b'=' as c_char;
        }
        if (nextchar != b'=' as c_char && nextchar != b':' as c_char)
            || (*xp).xp_context == EXPAND_BOOL_SETTINGS
        {
            (*xp).xp_context = EXPAND_UNSUCCESSFUL;
            return;
        }

        if is_term_option {
            EXPAND_OPTION_IDX = kOptInvalid;
        } else {
            EXPAND_OPTION_IDX = opt_idx;
        }

        (*xp).xp_pattern = p.add(1);
        EXPAND_OPTION_START_COL = p.add(1).offset_from((*xp).xp_line) as c_int;

        let opt_var = (*get_option(opt_idx)).var;
        if opt_var == (&raw mut p_syn).cast::<c_void>() {
            (*xp).xp_context = EXPAND_OWNSYNTAX;
            return;
        }
        if opt_var == (&raw mut p_ft).cast::<c_void>() {
            (*xp).xp_context = EXPAND_FILETYPE;
            return;
        }
        if opt_var == (&raw mut p_keymap).cast::<c_void>() {
            (*xp).xp_context = EXPAND_KEYMAP;
            return;
        }

        if expand_option_subtract {
            (*xp).xp_context = EXPAND_SETTING_SUBTRACT;
            return;
        } else if EXPAND_OPTION_IDX != kOptInvalid
            && (*get_option(EXPAND_OPTION_IDX)).opt_expand_cb.is_some()
        {
            (*xp).xp_context = EXPAND_STRING_SETTING;
        } else if *(*xp).xp_pattern == NUL {
            (*xp).xp_context = EXPAND_OLD_SETTING;
            return;
        } else {
            (*xp).xp_context = EXPAND_NOTHING;
        }

        if is_term_option || option_has_type(opt_idx, ObjectType::kObjectTypeInteger) {
            return;
        }

        if (flags & kOptFlagExpand) != 0 {
            let v = (*get_option(opt_idx)).var;
            if v == (&raw mut p_bdir).cast()
                || v == (&raw mut p_dir).cast()
                || v == (&raw mut p_path).cast()
                || v == (&raw mut p_pp).cast()
                || v == (&raw mut p_rtp).cast()
                || v == (&raw mut p_cdpath).cast()
                || v == (&raw mut p_vdir).cast()
            {
                (*xp).xp_context = EXPAND_DIRECTORIES;
                if v == (&raw mut p_path).cast() || v == (&raw mut p_cdpath).cast() {
                    (*xp).xp_backslash = XP_BS_THREE;
                } else {
                    (*xp).xp_backslash = XP_BS_ONE;
                }
            } else {
                (*xp).xp_context = EXPAND_FILES;
                if v == (&raw mut p_tags).cast() {
                    (*xp).xp_backslash = XP_BS_THREE;
                } else {
                    (*xp).xp_backslash = XP_BS_ONE;
                }
            }
            if (flags & kOptFlagComma) != 0 {
                (*xp).xp_backslash |= XP_BS_COMMA;
            }
        }

        if (flags & kOptFlagExpand) != 0
            || (flags & kOptFlagComma) != 0
            || (flags & kOptFlagColon) != 0
        {
            p = argend.offset(-1);
            while p > (*xp).xp_pattern {
                if *p == b' ' as c_char
                    || *p == b',' as c_char
                    || (*p == b':' as c_char && (flags & kOptFlagColon) != 0)
                {
                    let mut s = p;
                    while s > (*xp).xp_pattern && *s.offset(-1) == b'\\' as c_char {
                        s = s.offset(-1);
                    }
                    #[cfg(windows)]
                    let comma_unesc = *p == b',' as c_char
                        && (flags & kOptFlagComma) != 0
                        && p.offset_from(s) < 1;
                    #[cfg(not(windows))]
                    let comma_unesc = *p == b',' as c_char
                        && (flags & kOptFlagComma) != 0
                        && p.offset_from(s) < 2;

                    if (*p == b' ' as c_char
                        && (((*xp).xp_backslash & XP_BS_THREE) != 0 && p.offset_from(s) < 3))
                        || comma_unesc
                        || (*p == b':' as c_char && (flags & kOptFlagColon) != 0)
                    {
                        (*xp).xp_pattern = p.add(1);
                        break;
                    }
                }
                p = p.offset(-1);
            }
        }

        if (flags & kOptFlagFlagList) != 0 {
            (*xp).xp_pattern = argend;
        }

        if (*get_option(opt_idx)).var == (&raw mut p_sps).cast() {
            if strncmp((*xp).xp_pattern, c"file:".as_ptr(), 5) == 0 {
                (*xp).xp_pattern = (*xp).xp_pattern.add(5);
            } else if (*get_option(EXPAND_OPTION_IDX)).opt_expand_cb.is_some() {
                (*xp).xp_context = EXPAND_STRING_SETTING;
            }
        }
    }
}

unsafe fn match_str(
    str_arg: *mut c_char,
    regmatch: *mut regmatch_T,
    matches: &mut Vec<std::ffi::CString>,
    fuzzy: bool,
    fuzzystr: *const c_char,
    fuzmatch: &mut Vec<fuzmatch_str_T>,
) -> bool {
    unsafe {
        if !fuzzy {
            if vim_regexec(regmatch.cast(), str_arg, 0) {
                matches.push(CStr::from_ptr(str_arg).to_owned());
                return true;
            }
        } else {
            let score = fuzzy_match_str(str_arg, fuzzystr);
            if score != FUZZY_SCORE_NONE {
                let idx = fuzmatch.len() as c_int;
                fuzmatch.push(fuzmatch_str_T {
                    idx,
                    str_cstr: CStr::from_ptr(str_arg).to_owned(),
                    score,
                });
                return true;
            }
        }
        false
    }
}

pub unsafe fn ExpandSettings(
    xp: *mut expand_T,
    regmatch: *mut regmatch_T,
    fuzzystr: *mut c_char,
    matches: &mut Vec<std::ffi::CString>,
    can_fuzzy: bool,
) -> c_int {
    unsafe {
        let names: [*mut c_char; 1] = [c"all".as_ptr().cast_mut()];
        let ic = (*regmatch).rm_ic;

        matches.clear();
        let mut fuzmatch: Vec<fuzmatch_str_T> = Vec::new();
        let fuzzy = can_fuzzy && cmdline_fuzzy_complete(fuzzystr);

        (*regmatch).rm_ic = ic;
        if (*xp).xp_context != EXPAND_BOOL_SETTINGS {
            for &name in &names {
                match_str(name, regmatch, matches, fuzzy, fuzzystr, &mut fuzmatch);
            }
        }
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            let str_ptr = (*opt).fullname.as_ptr().cast_mut();
            if is_option_hidden(opt_idx) {
                continue;
            }
            if (*xp).xp_context == EXPAND_BOOL_SETTINGS
                && !option_has_type(opt_idx, ObjectType::kObjectTypeBoolean)
            {
                continue;
            }

            if !match_str(str_ptr, regmatch, matches, fuzzy, fuzzystr, &mut fuzmatch)
                && !fuzzy
                && (*opt)
                    .shortname
                    .is_some_and(|s| vim_regexec(regmatch.cast(), s.as_ptr(), 0))
            {
                matches.push((*opt).fullname.to_owned());
            }
        }

        if fuzzy {
            fuzzymatches_slice_to_strmatches(&mut fuzmatch, matches, false);
        }

        OK
    }
}

/// Escape an option value that can be used on the command-line with :set.
pub unsafe fn escape_option_str_cmdline(var_arg: *mut c_char) -> std::ffi::CString {
    unsafe {
        let buf_cstr = vim_strsave_escaped(var_arg, escape_chars);

        #[cfg(windows)]
        {
            let mut bytes = buf_cstr.into_bytes_with_nul();
            let buf = bytes.as_mut_ptr().cast::<c_char>();
            let mut var = buf;
            while *var != NUL {
                if *var == b'\\' as c_char
                    && *var.add(1) == b'\\' as c_char
                    && EXPAND_OPTION_IDX != kOptInvalid
                    && ((*get_option(EXPAND_OPTION_IDX)).flags & kOptFlagExpand) != 0
                    && vim_isfilec(*var.add(2) as u8 as c_int)
                    && (*var.add(2) != b'\\' as c_char
                        || (var == buf && *var.add(4) != b'\\' as c_char))
                {
                    let src = var.add(1);
                    ptr::copy(src, var, strlen(src) + 1);
                }
                var = var.add(utfc_ptr2len(var) as usize);
            }
            let len = CStr::from_ptr(buf).to_bytes().len();
            crate::memory::cstr_from_bytes(&bytes[..len])
        }
        #[cfg(not(windows))]
        {
            buf_cstr
        }
    }
}

/// Expansion handler for :set= when we just want to fill in with the existing value.
pub unsafe fn ExpandOldSetting(matches: &mut Vec<std::ffi::CString>) -> c_int {
    unsafe {
        matches.clear();

        if EXPAND_OPTION_IDX == kOptInvalid {
            EXPAND_OPTION_IDX = find_option((&raw const EXPAND_OPTION_NAME).cast());
        }

        let var = if EXPAND_OPTION_IDX != kOptInvalid {
            let o = opt_from_varp(
                EXPAND_OPTION_IDX,
                get_varp_scope(get_option(EXPAND_OPTION_IDX), EXPAND_OPTION_FLAGS),
            );
            let is_luafn = o.obj_type == ObjectType::kObjectTypeLuaRef;
            optval_free_read(EXPAND_OPTION_IDX, o);
            if is_luafn {
                return OK;
            }
            optval_fmt(EXPAND_OPTION_IDX, EXPAND_OPTION_FLAGS);
            (&raw mut NameBuff).cast::<c_char>()
        } else {
            c"".as_ptr().cast_mut()
        };

        matches.push(escape_option_str_cmdline(var));
        OK
    }
}

/// Expansion handler for :set=/:set+= when the option has a custom expansion handler.
pub unsafe fn ExpandStringSetting(
    xp: *mut expand_T,
    regmatch: *mut regmatch_T,
    matches: &mut Vec<std::ffi::CString>,
) -> c_int {
    unsafe {
        if EXPAND_OPTION_IDX == kOptInvalid {
            return FAIL;
        }
        let opt = get_option(EXPAND_OPTION_IDX);
        let Some(expand_cb) = (*opt).opt_expand_cb else {
            return FAIL;
        };

        let set_arg = (*xp).xp_line.add(EXPAND_OPTION_START_COL as usize);
        let mut args = optexpand_T {
            oe_varp: get_varp_scope(opt, EXPAND_OPTION_FLAGS).cast::<c_char>(),
            oe_idx: EXPAND_OPTION_IDX,
            oe_opt_value: ptr::null_mut(),
            oe_append: EXPAND_OPTION_APPEND,
            oe_include_orig_val: !EXPAND_OPTION_APPEND && *set_arg == NUL,
            oe_regmatch: regmatch,
            oe_xp: xp,
            oe_set_arg: set_arg,
        };

        optval_fmt(EXPAND_OPTION_IDX, EXPAND_OPTION_FLAGS);
        let buf = escape_option_str_cmdline((&raw mut NameBuff).cast::<c_char>());
        args.oe_opt_value = buf.as_ptr().cast_mut();

        expand_cb(&raw mut args, matches)
    }
}

/// Expansion handler for :set-=
pub unsafe fn ExpandSettingSubtract(
    xp: *mut expand_T,
    regmatch: *mut regmatch_T,
    matches: &mut Vec<std::ffi::CString>,
) -> c_int {
    unsafe {
        if EXPAND_OPTION_IDX == kOptInvalid
            || option_has_type(EXPAND_OPTION_IDX, ObjectType::kObjectTypeInteger)
            || is_callback_option(EXPAND_OPTION_IDX)
        {
            return ExpandOldSetting(matches);
        }

        let option_val =
            *(get_option_varp_scope_from(EXPAND_OPTION_IDX, EXPAND_OPTION_FLAGS, curbuf, curwin)
                .cast::<*mut c_char>());

        let option_flags = (*get_option(EXPAND_OPTION_IDX)).flags;

        if (option_flags & kOptFlagComma) != 0 {
            if *option_val == NUL {
                return FAIL;
            }

            let mut option_copy =
                crate::memory::cstr_to_cstring(option_val).into_bytes_with_nul();
            let mut next_val = option_copy.as_mut_ptr().cast::<c_char>();

            matches.clear();

            loop {
                let item = next_val;
                let mut comma = vim_strchr(next_val, b',' as c_int);
                while !comma.is_null() && comma != next_val && *comma.offset(-1) == b'\\' as c_char
                {
                    comma = vim_strchr(comma.add(1), b',' as c_int);
                }
                if !comma.is_null() {
                    *comma = NUL;
                    next_val = comma.add(1);
                } else {
                    next_val = ptr::null_mut();
                }

                if *item != NUL && vim_regexec(regmatch.cast(), item, 0) {
                    matches.push(escape_option_str_cmdline(item));
                }

                if next_val.is_null() {
                    break;
                }
            }

            return OK;
        } else if (option_flags & kOptFlagFlagList) != 0 {
            if *(*xp).xp_pattern != NUL {
                return FAIL;
            }

            let num_flags = strlen(option_val);
            if num_flags == 0 {
                return FAIL;
            }

            matches.clear();
            matches.reserve_exact(num_flags + 1);
            matches.push(crate::memory::cstr_from_bytes(std::slice::from_raw_parts(
                option_val.cast::<u8>(),
                num_flags,
            )));

            if num_flags > 1 {
                let mut flag = option_val;
                while *flag != NUL {
                    matches.push(crate::memory::cstr_from_bytes(&[*flag as u8]));
                    flag = flag.add(1);
                }
            }

            return OK;
        }

        ExpandOldSetting(matches)
    }
}

unsafe fn optval_fmt(opt_idx: OptIndex, opt_flags: c_int) {
    unsafe {
        let opt = get_option(opt_idx);
        let varp = get_varp_scope(opt, opt_flags);
        assert!(!varp.is_null());

        let namebuf_ptr = (&raw mut NameBuff).cast::<c_char>();
        let mut wc: OptInt = 0;
        if wc_use_keyname(varp.cast::<OptInt>(), &raw mut wc) != 0 {
            xstrlcpy(namebuf_ptr, get_special_key_name(wc as c_int, 0), MAXPATHL);
            return;
        }
        if wc != 0 {
            xstrlcpy(namebuf_ptr, transchar(wc as c_int), MAXPATHL);
            return;
        }
        if ((*opt).flags & kOptFlagExpand) != 0
            && !is_callback_option(opt_idx)
            && option_has_type(opt_idx, ObjectType::kObjectTypeString)
        {
            home_replace(
                ptr::null_mut(),
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

unsafe fn wc_use_keyname(p: *const OptInt, wcp: *mut OptInt) -> c_int {
    unsafe {
        if p == &raw const p_wc || p == &raw const p_wcm {
            *wcp = *p;
            if is_special(*wcp as c_int) || find_special_key_in_table(*wcp as c_int) >= 0 {
                return 1;
            }
        }
        0
    }
}

pub unsafe fn shortmess(x: c_int) -> bool {
    unsafe {
        !p_shm.is_null()
            && (!vim_strchr(p_shm, x).is_null()
                || (!vim_strchr(p_shm, b'a' as c_int).is_null()
                    && !vim_strchr(SHM_ALL_ABBREVIATIONS, x).is_null()))
    }
}

pub unsafe fn vimrc_found(fname: *mut c_char, envname: *mut c_char) {
    unsafe {
        if !fname.is_null()
            && !envname.is_null()
            && vim_getenv(envname).is_none()
            && let Some(p) = FullName_save(fname, false)
        {
            os_setenv(envname, p.as_ptr(), 1);
        }
    }
}

/// Check whether global option has been set.
pub unsafe fn option_was_set(opt_idx: OptIndex) -> bool {
    unsafe {
        assert!(opt_idx != kOptInvalid);
        ((*get_option(opt_idx)).flags & kOptFlagWasSet) != 0
    }
}

/// Reset the flag indicating option "name" was set.
pub unsafe fn reset_option_was_set(opt_idx: OptIndex) {
    unsafe {
        assert!(opt_idx != kOptInvalid);
        (*get_option(opt_idx)).flags &= !kOptFlagWasSet;
    }
}

/// Parse 'cursorlineopt', writing flags only when the value is valid.
pub unsafe fn fill_culopt_flags(val: *mut c_char, flags: *mut c_uint) -> c_int {
    unsafe {
        let mut p = val;
        let mut culopt_flags_new: u8 = 0;

        while *p != NUL {
            if strncmp(p, c"line".as_ptr(), 4) == 0 {
                p = p.add(4);
                culopt_flags_new |= kOptCuloptFlagLine;
            } else if strncmp(p, c"both".as_ptr(), 4) == 0 {
                p = p.add(4);
                culopt_flags_new |= kOptCuloptFlagLine | kOptCuloptFlagNumber;
            } else if strncmp(p, c"number".as_ptr(), 6) == 0 {
                p = p.add(6);
                culopt_flags_new |= kOptCuloptFlagNumber;
            } else if strncmp(p, c"screenline".as_ptr(), 10) == 0 {
                p = p.add(10);
                culopt_flags_new |= kOptCuloptFlagScreenline;
            }

            if *p != b',' as c_char && *p != NUL {
                return FAIL;
            }
            if *p == b',' as c_char {
                p = p.add(1);
            }
        }

        if (culopt_flags_new & kOptCuloptFlagLine) != 0
            && (culopt_flags_new & kOptCuloptFlagScreenline) != 0
        {
            return FAIL;
        }
        *flags = c_uint::from(culopt_flags_new);

        OK
    }
}

/// Set the callback function value for an option that accepts a function name, lambda, et al.
pub unsafe fn option_set_callback_func(optval: *mut c_char, optcb: *mut Callback) -> c_int {
    unsafe {
        if optval.is_null() || *optval == NUL {
            callback_free(optcb);
            return OK;
        }

        let mut tv: typval_T = if *optval == b'{' as c_char
            || strncmp(optval, c"function(".as_ptr(), 9) == 0
            || strncmp(optval, c"funcref(".as_ptr(), 8) == 0
        {
            let Some(evaluated) = eval_expr(optval, ptr::null_mut()) else {
                return FAIL;
            };
            evaluated
        } else {
            let mut allocated = typval_T::new();
            crate::eval::typval::tv_set_string(&raw mut allocated, optval);
            allocated
        };

        let mut cb = Callback::NONE;
        if !callback_from_typval(&raw mut cb, &raw mut tv) || cb.cb_type() == kCallbackNone {
            tv_clear(&raw mut tv);
            return FAIL;
        }

        callback_free(optcb);
        *optcb = cb;
        tv_clear(&raw mut tv);
        OK
    }
}

unsafe fn didset_options_sctx(opt_flags: c_int, buf: *mut c_int) {
    unsafe {
        let mut i: usize = 0;
        loop {
            let opt_idx = *buf.add(i);
            if opt_idx == kOptInvalid {
                break;
            }
            set_option_sctx(opt_idx, opt_flags, current_sctx);
            i += 1;
        }
    }
}

/// Check if backspacing over something is allowed.
pub unsafe fn can_bs(what: c_int) -> bool {
    unsafe {
        if what == BS_START && bt_prompt(curbuf) {
            return false;
        }

        if what == BS_EOL && !mc_ins_replay_can_join() {
            return false;
        }

        if *p_bs == b'2' as c_char {
            return what != BS_NOSTOP;
        }

        !vim_strchr(p_bs, what).is_null()
    }
}

/// Get the local or global value of 'backupcopy' flags.
pub unsafe fn get_bkc_flags(buf: *mut buf_T) -> c_uint {
    unsafe {
        if (*buf).b_bkc_flags != 0 {
            (*buf).b_bkc_flags
        } else {
            bkc_flags
        }
    }
}

/// Get the local or global value of 'formatlistpat'.
pub unsafe fn get_flp_value(buf: *mut buf_T) -> *mut c_char {
    unsafe {
        if (*buf).b_p_flp.is_null() || *(*buf).b_p_flp == NUL {
            p_flp
        } else {
            (*buf).b_p_flp
        }
    }
}

/// Get the local or global value of 'virtualedit' flags.
pub unsafe fn get_ve_flags(wp: *mut win_T) -> c_uint {
    unsafe {
        let f = if (*wp).w_onebuf_opt.wo_ve_flags != 0 {
            (*wp).w_onebuf_opt.wo_ve_flags
        } else {
            ve_flags
        };
        f & !(kOptVeFlagNone | kOptVeFlagNoneU)
    }
}

/// Get the local or global value of 'showbreak'.
pub unsafe fn get_showbreak_value(win: *mut win_T) -> *mut c_char {
    unsafe {
        let sbr = (*win).w_onebuf_opt.wo_sbr;
        if sbr.is_null() || *sbr == NUL {
            return p_sbr;
        }
        if strcmp(sbr, c"NONE".as_ptr()) == 0 {
            return (&raw mut empty_string_option).cast::<c_char>();
        }
        sbr
    }
}

/// Return the current end-of-line type: EOL_DOS, EOL_UNIX or EOL_MAC.
pub unsafe fn get_fileformat(buf: *const buf_T) -> c_int {
    unsafe {
        let c = *(*buf).b_p_ff as u8;
        if (*buf).b_p_bin != 0 || c == b'u' {
            return EOL_UNIX;
        }
        if c == b'm' {
            return EOL_MAC;
        }
        EOL_DOS
    }
}

/// Like get_fileformat(), but override 'fileformat' with "p" for "++opt=val" argument.
pub unsafe fn get_fileformat_force(buf: *const buf_T, eap: *const exarg_T) -> c_int {
    unsafe {
        let c = if !eap.is_null() && (*eap).force_ff != 0 {
            (*eap).force_ff as u8
        } else {
            let is_bin = if !eap.is_null() && (*eap).force_bin != 0 {
                (*eap).force_bin == FORCE_BIN
            } else {
                (*buf).b_p_bin != 0
            };
            if is_bin {
                return EOL_UNIX;
            }
            *(*buf).b_p_ff as u8
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
pub unsafe fn default_fileformat() -> c_int {
    unsafe {
        match *p_ffs as u8 {
            b'm' => EOL_MAC,
            b'd' => EOL_DOS,
            _ => EOL_UNIX,
        }
    }
}

/// Set the current end-of-line type to EOL_UNIX, EOL_MAC, or EOL_DOS.
pub unsafe fn set_fileformat(eol_style: c_int, opt_flags: c_int) {
    unsafe {
        let p: *const c_char = match eol_style {
            EOL_UNIX => c"unix".as_ptr(),
            EOL_MAC => c"mac".as_ptr(),
            EOL_DOS => c"dos".as_ptr(),
            _ => ptr::null(),
        };

        if !p.is_null() {
            set_option_direct(kOptFileformat, cstr_as_obj(p), opt_flags, 0);
        }

        redraw_buf_status_later(curbuf);
        redraw_tabline = true;
        need_maketitle = true;
    }
}

/// Skip to next part of an option argument: skip space and comma.
pub unsafe fn skip_to_option_part(mut p: *const c_char) -> *mut c_char {
    unsafe {
        if *p == b',' as c_char {
            p = p.add(1);
        }
        while *p == b' ' as c_char {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

/// Isolate one part of a string option separated by `sep_chars`.
pub unsafe fn copy_option_part(
    option: *mut *mut c_char,
    buf: *mut c_char,
    maxlen: usize,
    sep_chars: *const c_char,
) -> usize {
    unsafe {
        let mut len: usize = 0;
        let mut p = *option;

        if *p == b'.' as c_char {
            *buf.add(len) = *p;
            len += 1;
            p = p.add(1);
        }
        while *p != NUL && vim_strchr(sep_chars, *p as u8 as c_int).is_null() {
            if *p == b'\\' as c_char && !vim_strchr(sep_chars, *p.add(1) as u8 as c_int).is_null() {
                p = p.add(1);
            }
            if len < maxlen - 1 {
                *buf.add(len) = *p;
                len += 1;
            }
            p = p.add(1);
        }
        *buf.add(len) = NUL;

        if *p != NUL && *p != b',' as c_char {
            p = p.add(1);
        }
        p = skip_to_option_part(p);

        *option = p;
        len
    }
}

/// Iterates comma-separated parts of option, splitting each part at its first ':' into key/value.
pub unsafe fn option_next_keyval(
    p: *mut *const c_char,
    key: *mut *const c_char,
    keylen: *mut usize,
    val: *mut *const c_char,
    vallen: *mut usize,
) -> bool {
    unsafe {
        let s = *p;
        if *s == NUL {
            return false;
        }

        let comma = vim_strchr(s, b',' as c_int);
        let end = if !comma.is_null() {
            comma as *const c_char
        } else {
            s.add(strlen(s))
        };
        let colon = vim_strchr(s, b':' as c_int) as *const c_char;

        *key = s;
        if !colon.is_null() && colon < end {
            *keylen = colon.offset_from(s) as usize;
            *val = colon.add(1);
            *vallen = end.offset_from(colon.add(1)) as usize;
        } else {
            *keylen = end.offset_from(s) as usize;
            *val = ptr::null();
            *vallen = 0;
        }

        *p = if !comma.is_null() {
            (comma as *const c_char).add(1)
        } else {
            end
        };
        true
    }
}

/// Compare a (non-NUL-terminated) slice from option_next_keyval() against a name.
pub unsafe fn option_slice_eq(slice: *const c_char, slicelen: usize, name: *const c_char) -> bool {
    unsafe { !slice.is_null() && strlen(name) == slicelen && strncmp(slice, name, slicelen) == 0 }
}

/// Return true when 'shell' has "csh" in the tail.
pub unsafe fn csh_like_shell() -> c_int {
    unsafe { c_int::from(!c_strstr(path_tail(p_sh), c"csh".as_ptr()).is_null()) }
}

/// Return true when 'shell' has "fish" in the tail.
pub unsafe fn fish_like_shell() -> bool {
    unsafe { !c_strstr(path_tail(p_sh), c"fish".as_ptr()).is_null() }
}

/// Get window or buffer local options.
pub unsafe fn get_winbuf_options(bufopt: c_int) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();

        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);

            if (bufopt != 0 && option_has_scope(opt_idx, kOptScopeBuf))
                || (bufopt == 0 && option_has_scope(opt_idx, kOptScopeWin))
            {
                let varp = get_varp(opt);
                if !varp.is_null() {
                    let ov = opt_from_varp(opt_idx, varp);
                    let mut opt_tv = opt_to_tv(ov, true);
                    tv_dict_add_tv(
                        d,
                        (*opt).fullname.as_ptr(),
                        (*opt).fullname.to_bytes().len(),
                        &raw mut opt_tv,
                    );
                    tv_clear(&raw mut opt_tv);
                    optval_free_read(opt_idx, ov);
                }
            }
        }

        d
    }
}

/// Return the effective 'scrolloff' value for the current window.
pub unsafe fn get_scrolloff_value(wp: *mut win_T) -> i64 {
    unsafe {
        if (State & MODE_TERMINAL) != 0 && !(*(*wp).w_buffer).terminal.is_null() {
            return 0;
        }
        if (*wp).w_onebuf_opt.wo_so < 0 {
            p_so
        } else {
            (*wp).w_onebuf_opt.wo_so
        }
    }
}

/// Return the effective 'scrolloffpad' value for the current window.
pub unsafe fn get_scrolloffpad_value(wp: *mut win_T) -> i64 {
    unsafe {
        if (*wp).w_onebuf_opt.wo_sop == -1 {
            p_sop
        } else {
            (*wp).w_onebuf_opt.wo_sop
        }
    }
}

/// Return the effective 'sidescrolloff' value for the current window.
pub unsafe fn get_sidescrolloff_value(wp: *mut win_T) -> i64 {
    unsafe {
        if (*wp).w_onebuf_opt.wo_siso < 0 {
            p_siso
        } else {
            (*wp).w_onebuf_opt.wo_siso
        }
    }
}

/// Metadata Dict for option `name`; errors if `name` is unknown.
pub unsafe fn get_option_info(
    name: NvimString,
    opt_flags: c_int,
    buf: *mut buf_T,
    win: *mut win_T,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let opt_idx = find_option_len(name.data, name.size);
        if opt_idx == kOptInvalid {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Invalid option (not found): '%s'".as_ptr(),
                name.data,
            );
            return Dict::new();
        }

        option_info_dict(get_option(opt_idx), opt_flags, buf, win, arena)
    }
}

/// Metadata Dict for every option, keyed by full name.
pub unsafe fn get_all_options_info(arena: *mut Arena) -> Dict {
    unsafe {
        let mut retval = arena_dict(arena, kOptCount as usize);
        for opt_idx in 0..kOptCount {
            let opt = get_option(opt_idx);
            let opt_dict = option_info_dict(opt, OPT_GLOBAL, curbuf, curwin, arena);
            put_c(&mut retval, (*opt).fullname.as_ptr(), dict_obj(opt_dict));
        }
        retval
    }
}

unsafe fn option_info_dict(
    opt: *mut vimoption_T,
    opt_flags: c_int,
    buf: *mut buf_T,
    win: *mut win_T,
    arena: *mut Arena,
) -> Dict {
    unsafe {
        let opt_idx = get_opt_idx(opt);
        let mut dict = arena_dict(arena, 13);

        put_c(
            &mut dict,
            c"name".as_ptr(),
            cstr_as_obj((*opt).fullname.as_ptr()),
        );
        put_c(
            &mut dict,
            c"shortname".as_ptr(),
            cstr_as_obj((*opt).shortname.map_or(ptr::null(), |s| s.as_ptr())),
        );

        let scope = if option_has_scope(opt_idx, kOptScopeBuf) {
            c"buf".as_ptr()
        } else if option_has_scope(opt_idx, kOptScopeWin) {
            c"win".as_ptr()
        } else if option_has_scope(opt_idx, kOptScopeTab) {
            c"tab".as_ptr()
        } else {
            c"global".as_ptr()
        };

        put_c(&mut dict, c"scope".as_ptr(), cstr_as_obj(scope));

        put_c(
            &mut dict,
            c"global_local".as_ptr(),
            boolean_obj(option_is_global_local(opt_idx)),
        );
        put_c(
            &mut dict,
            c"commalist".as_ptr(),
            boolean_obj(((*opt).flags & kOptFlagComma) != 0),
        );
        put_c(
            &mut dict,
            c"flaglist".as_ptr(),
            boolean_obj(((*opt).flags & kOptFlagFlagList) != 0),
        );

        put_c(
            &mut dict,
            c"was_set".as_ptr(),
            boolean_obj(((*opt).flags & kOptFlagWasSet) != 0),
        );

        let mut script_ctx = sctx_T {
            sc_sid: 0,
            sc_seq: 0,
            sc_lnum: 0,
            sc_chan: 0,
        };
        if opt_flags == OPT_GLOBAL {
            script_ctx = (*opt).script_ctx;
        } else {
            if option_has_scope(opt_idx, kOptScopeBuf) {
                script_ctx =
                    (*buf).b_p_script_ctx[(*opt).scope_idx[kOptScopeBuf as usize] as usize];
            }
            if option_has_scope(opt_idx, kOptScopeWin) {
                script_ctx = (*win).w_onebuf_opt.wo_script_ctx
                    [(*opt).scope_idx[kOptScopeWin as usize] as usize];
            }
            if opt_flags != OPT_LOCAL && script_ctx.sc_sid == 0 {
                script_ctx = (*opt).script_ctx;
            }
        }

        put_c(
            &mut dict,
            c"last_set_sid".as_ptr(),
            integer_obj(i64::from(script_ctx.sc_sid)),
        );
        put_c(
            &mut dict,
            c"last_set_linenr".as_ptr(),
            integer_obj(i64::from(script_ctx.sc_lnum)),
        );
        put_c(
            &mut dict,
            c"last_set_chan".as_ptr(),
            integer_obj(script_ctx.sc_chan as i64),
        );

        put_c(
            &mut dict,
            c"type".as_ptr(),
            cstr_as_obj(optval_type_name((*get_option(get_opt_idx(opt))).opt_type)),
        );
        put_c(&mut dict, c"default".as_ptr(), (*opt).def_val);
        put_c(
            &mut dict,
            c"allows_duplicates".as_ptr(),
            boolean_obj(((*opt).flags & kOptFlagNoDup) == 0),
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
        unsafe {
            assert_eq!(find_option(c"number".as_ptr()), kOptNumber);
            assert_eq!(find_option(c"nu".as_ptr()), kOptNumber);
            assert_eq!(find_option_len(c"tabstop=4".as_ptr(), 7), kOptTabstop);
            assert_eq!(find_option(c"nonexistent_option".as_ptr()), kOptInvalid);

            assert!(option_has_type(kOptNumber, ObjectType::kObjectTypeBoolean));
            assert!(option_has_type(kOptTabstop, ObjectType::kObjectTypeInteger));
            assert!(option_has_scope(kOptNumber, kOptScopeWin));
            assert!(option_has_scope(kOptTabstop, kOptScopeBuf));
            assert!(!is_option_hidden(kOptNumber));
            assert!(is_option_hidden(0));
        }
    }
}
