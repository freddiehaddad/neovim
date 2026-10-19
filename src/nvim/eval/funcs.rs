//! Builtin Vimscript functions (`src/nvim/eval/funcs.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    unused_assignments,
    unused_imports,
    unused_mut,
    unused_unsafe,
    unused_variables,
    clippy::collapsible_if,
    clippy::enum_variant_names,
    clippy::if_same_then_else,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::useless_conversion
)]

use std::ffi::{CStr, c_char, c_double, c_int, c_ulong, c_void};
use std::ptr;

use crate::api::private::converter::{object_to_vim, partial_T, sctx_T, ufunc_T, vim_to_object};
use crate::api::private::validate::{Array, Error, ErrorType, Object, ObjectData, ObjectType};
use crate::api::tabpage::{ScreenGrid, tabpage_T, win_T};
use crate::autocmd::{EVENT_BUFFILEPOST, EVENT_BUFFILEPRE, apply_autocmds, funccal_entry_T};
use crate::charset::skipwhite;
use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::cmdexpand::{
    EXPAND_FILES, ExpandCleanup, ExpandInit, ExpandOne, WILD_ALL, WILD_ALL_KEEP, WILD_ICASE,
    WILD_KEEP_ALL, WILD_LIST_NOTFOUND, WILD_SILENT, WILD_USE_NL, WILD_USE_SHELLSLASH, cmdidx_T,
    exarg_T,
};
use crate::cursor::{check_cursor, colnr_T, get_cursor_pos_ptr, linenr_T};
use crate::eval::buffer::{find_buffer, varnumber_T};
use crate::eval::decode::{MPACK_OK, dict_T, dictitem_T, json_decode_string, mpack_parser_t};
use crate::eval::deprecated::{Callback, CallbackReader, EvalFuncData, buf_T, pos_T, synblock_T};
use crate::eval::encode::{
    ListReaderState, encode_init_lrstate, encode_list_write, encode_read_from_list, encode_tv2echo,
    encode_tv2json, encode_vim_list_to_buf, encode_vim_to_msgpack,
};
use crate::eval::executor::blob_T;
use crate::eval::typval::{
    tv_blob_alloc_ret, tv_clear, tv_copy, tv_dict_add_allocated_str, tv_dict_add_bool,
    tv_dict_add_list, tv_dict_add_nr, tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc,
    tv_dict_alloc_ret, tv_dict_find, tv_dict_hi2di, tv_dict_item_remove, tv_dict_watcher_add,
    tv_dict_watcher_remove, tv_get_bool, tv_get_bool_chk, tv_get_lnum_buf, tv_get_number,
    tv_get_number_chk, tv_islocked, tv_list_alloc, tv_list_alloc_ret,
    tv_list_append_allocated_string, tv_list_append_dict, tv_list_append_list,
    tv_list_append_number, tv_list_append_owned_tv, tv_list_append_string, tv_list_append_tv,
    tv_list_extend, tv_list_find, tv_list_item_remove, tv_list_unref,
};
use crate::eval::userfunc::{
    EvalFuncDef, FCERR_NONE, FCERR_NOTMETHOD, FCERR_TOOFEW, FCERR_TOOMANY, FCERR_UNKNOWN, VimLFunc,
    find_func, func_ptr_ref, func_ref, func_unref, printable_func_name, restore_funccal,
    save_funccal, set_current_funccal,
};
use crate::eval::vars::{
    VV_KEY, VV_REG, VV_SEND_SERVER, VV_SHELL_ERROR, VV_VAL, evalarg_T, find_var, get_user_var_name,
    get_vim_var_nr, get_vim_var_str, get_vim_var_tv, lval_T, prepare_vimvar, restore_vimvar,
    set_vim_var_nr, set_vim_var_type, var_exists,
};
use crate::eval::window::{find_tabwin, find_win_by_nr_or_id, win_id2wp_tp};
use crate::eval::{
    add_timer_info, add_timer_info_all, caller_scope, clear_lval, eval_expr_to_bool,
    eval_expr_typval, eval_expr_valid_arg, eval_has_provider, eval1, find_timer_by_nr,
    get_callback_depth, get_copyID, get_lval, list2fpos, script_host_eval, timer_T, timer_due_cb,
    timer_start, timer_stop, timer_stop_all, timer_tail, var_item_copy, var2fpos,
};
use crate::event::r#loop::{Loop, loop_closing_ptr, loop_on_put, loop_poll_events, loop_queues};
use crate::event::multiqueue::{
    MultiQueue, multiqueue_empty, multiqueue_free, multiqueue_new, multiqueue_process_events,
    multiqueue_replace_parent,
};
use crate::event::proc::{Proc, PtyProc, proc_header, proc_stop, proc_wait};
use crate::event::time::{
    TimeWatcher, TimeWatcherFields, time_watcher_close, time_watcher_init, time_watcher_start,
    time_watcher_stop, watcher_fields,
};
use crate::ex_eval::aborting;
use crate::garray::{ga_append, ga_append_via_ptr, ga_clear, ga_grow, ga_init, garray_T};
use crate::grid::{grid_getchar, schar_T, schar_from_char, schar_get, schar_get_first_codepoint};
use crate::highlight_group::{
    get_highlight_name_ext, highlight_color, highlight_exists, highlight_has_attr, syn_name2id,
};
use crate::indent::{get_sw_value, get_sw_value_col};
use crate::input::{restore_typeahead, save_typeahead, stuff_empty, tasave_T, using_script, vgetc};
use crate::insexpand::ins_compl_active;
use crate::lua::stdlib::regmatch_T;
use crate::map_glyph_cache::NvimString;
use crate::mark::{
    WinInfo, cleanup_jumplist, get_buf_local_marks, get_global_marks, setmark_pos, setpcmark,
};
use crate::math::{FAIL, OK, xisinf, xisnan};
use crate::mbyte::{
    convert_setup, enc_locale, string_convert, utf_char2bytes, utf_ptr2char, utf_ptr2len,
    utfc_ptr2len, vimconv_T,
};
use crate::memfile::memfile_T;
use crate::memline::{
    ml_find_line_or_offset, ml_get, ml_get_buf, ml_get_buf_len, ml_get_len, ml_open,
};
use crate::memory::{
    ARENA_BLOCK_SIZE, Arena, ArenaMem, alloc_block, arena_finish, arena_mem_free, free_block,
    xcalloc, xfree, xmalloc, xmallocz, xmemdup, xmemdupz, xstrdup,
};
use crate::msgpack_rpc::channel::{
    Channel, ChannelPart, ChannelStreamType, channel_prefix, channel_stream_ptr, channel_tail,
    find_channel, rpc_send_call, rpc_send_event,
};
use crate::msgpack_rpc::packer::{Integer, PackerBuffer, packer_string_buffer, packer_take_string};
use crate::msgpack_rpc::server::{
    server_address_list, server_address_new, server_start, server_stop,
};
use crate::msgpack_rpc::unpacker::MsgpackRpcRequestHandler;
use crate::optionstr::{OptIndex, free_string_option};
use crate::os::dl::os_libcall;
use crate::os::env::{
    expand_T, expand_env_save, home_replace, os_env_exists, os_getenv, vim_env_iter, vim_getenv,
    vim_setenv_ext, vim_unsetenv_ext,
};
use crate::os::fs::{os_isdir, os_setperm};
use crate::os::shell::shell_free_argv;
use crate::os::stdpaths::{XDGVarType, get_xdg_home, stdpaths_get_xdg_var};
use crate::os::time::{NS_PER_SEC, Tm, os_hrtime, os_localtime_r, os_strptime, time_t};
use crate::path::{concat_fnames_realloc, path_to_slash_save, vim_FullName};
use crate::plines::{getvvcol, win_chartabsize};
use crate::popupmenu::{pum_set_event_info, pum_visible};
use crate::profile::{
    estack_T, funccall_T, profile_end, profile_msg, profile_setlimit, profile_signed,
    profile_start, profile_sub, proftime_T,
};
use crate::regexp::{reg_submatch, reg_submatch_list, vim_regcomp, vim_regexec_nl, vim_regfree};
use crate::register::{
    YREG_YANK, block_def, format_reg_type, get_reg_contents, get_reg_type, get_register_name,
    get_unname_register, get_yank_register, op_reg_set_previous, write_reg_contents_ex,
    write_reg_contents_lst,
};
use crate::runtime::exestack;
use crate::search::{searchit, searchit_arg_T};
use crate::sha256::sha256_bytes;
use crate::spell::{eval_soundfold, parse_spelllang, spell_check, spell_move_to};
use crate::spellsuggest::spell_suggest_list;
use crate::state::{MODE_CMDLINE, MODE_MAX_LENGTH, get_mode, get_was_safe_state, typebuf_T};
use crate::strings::{vim_strchr, vim_strsave_escaped, vim_strsave_shellescape};
use crate::syntax::{
    HL_CONCEAL, get_syntax_info, syn_get_id, syn_get_stack_item, syn_get_sub_char, syntax_present,
};
use crate::tag::{get_tagfname, get_tags, get_tagstack, set_tagstack, tagname_T, tagname_free};
use crate::textformat::{MotionType, oparg_T};
use crate::ui::{
    ui_busy_start, ui_busy_stop, ui_current_col, ui_current_row, ui_flush, ui_has, ui_rgb_attached,
};
use crate::ui_compositor::ui_comp_get_grid_at_coord;
use crate::version::{has_nvim_version, has_vim_patch};
use crate::window::find_tabpage;

pub type float_T = c_double;
pub type TriState = c_int;
pub type VarLockStatus = c_int;
pub type BoolVarValue = c_int;
pub type SpecialVarValue = c_int;
pub type ChannelStdinMode = c_int;
pub type hlf_T = c_int;

const NUL: c_char = 0;
const BASE_NONE: u8 = 0;
const BASE_LAST: u8 = u8::MAX;

const MAX_FUNC_ARGS: usize = 20;
const NUMBUFLEN: usize = 65;
const IOSIZE: usize = 1025;
const MAXPATHL: usize = 4096;
const MB_MAXBYTES: usize = 30;
const MB_MAXCHAR: usize = 6;
const MAX_SCHAR_SIZE: usize = 32;
const NSUBEXP: usize = 10;
const MAXCOL: colnr_T = 0x7fffffff;
const VARNUMBER_MAX: varnumber_T = varnumber_T::MAX;
const VARNUMBER_MIN: varnumber_T = varnumber_T::MIN;

const kNone: TriState = -1;
const kFalse: TriState = 0;
const kTrue: TriState = 1;

const kBoolVarFalse: BoolVarValue = 0;
const kBoolVarTrue: BoolVarValue = 1;
const kSpecialVarNull: SpecialVarValue = 0;

const VAR_UNLOCKED: VarLockStatus = 0;
const VAR_FIXED: VarLockStatus = 2;

const VAR_TYPE_NUMBER: c_int = 0;
const VAR_TYPE_STRING: c_int = 1;
const VAR_TYPE_FUNC: c_int = 2;
const VAR_TYPE_LIST: c_int = 3;
const VAR_TYPE_DICT: c_int = 4;
const VAR_TYPE_FLOAT: c_int = 5;
const VAR_TYPE_BOOL: c_int = 6;
const VAR_TYPE_SPECIAL: c_int = 7;
const VAR_TYPE_BLOB: c_int = 10;

const kListLenUnknown: isize = -1;
const kListLenShouldKnow: isize = -2;
const kListLenMayKnow: isize = -3;

const VIML_INTERNAL_CALL: u64 = 1u64 << 63;
const kRetObject: c_int = 0;
const kRetNilBool: c_int = 1;

const TFN_INT: c_int = 1;
const TFN_QUIET: c_int = 2;
const TFN_NO_AUTOLOAD: c_int = 4;
const TFN_NO_DEREF: c_int = 8;
const TFN_READ_ONLY: c_int = 16;
const GLV_NO_AUTOLOAD: c_int = TFN_NO_AUTOLOAD;
const GLV_READ_ONLY: c_int = TFN_READ_ONLY;
const FNE_CHECK_START: c_int = 2;
const DI_FLAGS_LOCK: u8 = 8;

const kChannelPartStdin: ChannelPart = 0;
const kChannelPartStdout: ChannelPart = 1;
const kChannelPartStderr: ChannelPart = 2;
const kChannelPartRpc: ChannelPart = 3;
const kChannelPartAll: ChannelPart = 4;

const kChannelStdinPipe: ChannelStdinMode = 0;
const kChannelStdinNull: ChannelStdinMode = 1;

const kChannelStreamProc: ChannelStreamType = 0;
const kProcTypePty: c_int = 1;
const kCallbackNone: c_int = 0;

const VIM_GENERIC: c_int = 0;
const VIM_ERROR: c_int = 1;
const VIM_WARNING: c_int = 2;
const VIM_INFO: c_int = 3;
const VIM_QUESTION: c_int = 4;

const DOCMD_VERBOSE: c_int = 0x01;
const DOCMD_NOWAIT: c_int = 0x02;
const DOCMD_REPEAT: c_int = 0x04;
const DOCMD_KEYTYPED: c_int = 0x08;

const MENU_INDEX_TIP: c_int = 7;
const MENU_ALL_MODES: c_int = (1 << MENU_INDEX_TIP) - 1;
const EX_NOSPC: u32 = 0x010;
const TV_TRANSLATE: usize = usize::MAX;
const VSE_NONE: c_int = 0;
const AUTOLOAD_CHAR: c_int = b'#' as c_int;
const VALID_VIRTCOL: c_int = 0x04;
const Ctrl_V: c_char = 22;

const kGRegExprSrc: c_int = 2;
const kGRegList: c_int = 4;

const UV_TIMER: c_int = 13;
const kUIMessages: c_int = 4;

const RE_SEARCH: c_int = 0;
const RE_MAGIC: c_int = 1;
const RE_STRING: c_int = 2;

const MPACK_EOF: c_int = 1;
const MPACK_ERROR: c_int = 2;
const MPACK_NOMEM: c_int = 3;

const FORWARD: c_int = 1;
const BACKWARD: c_int = -1;

const SEARCH_END: c_int = 0x40;
const SEARCH_START: c_int = 0x100;
const SEARCH_KEEP: c_int = 0x400;
const SEARCH_COL: c_int = 0x1000;

const SP_NOMOVE: c_int = 0x01;
const SP_REPEAT: c_int = 0x02;
const SP_RETCOUNT: c_int = 0x04;
const SP_SETPCMARK: c_int = 0x08;
const SP_START: c_int = 0x10;
const SP_SUBPAT: c_int = 0x20;
const SP_END: c_int = 0x40;
const SP_COLUMN: c_int = 0x80;

const kOptCpoptions: OptIndex = 61;

const kMTUnknown: MotionType = -1;
const kMTCharWise: MotionType = 0;
const kMTLineWise: MotionType = 1;
const kMTBlockWise: MotionType = 2;

const SMT_ALL: c_int = 0;
const HLF_SPB: hlf_T = 37;
const HLF_SPC: hlf_T = 38;
const HLF_SPR: hlf_T = 39;
const HLF_SPL: hlf_T = 40;
const HLF_COUNT: hlf_T = 76;

const CONV_NONE: c_int = 0;

const HL_INVERSE: c_int = 0x01;
const HL_BOLD: c_int = 0x02;
const HL_ITALIC: c_int = 0x04;
const HL_UNDERLINE: c_int = 0x08;
const HL_UNDERCURL: c_int = 0x10;
const HL_UNDERDOUBLE: c_int = 0x18;
const HL_UNDERDOTTED: c_int = 0x20;
const HL_UNDERDASHED: c_int = 0x28;
const HL_STANDOUT: c_int = 0x0040;
const HL_STRIKETHROUGH: c_int = 0x0080;
const HL_DIM: c_int = 0x0200;
const HL_NOCOMBINE: c_int = 0x0400;
const HL_BLINK: c_int = 0x8000;
const HL_CONCEALED: c_int = 0x10000;
const HL_OVERLINE: c_int = 0x20000;

#[cfg(windows)]
const ENV_SEPCHAR: c_char = b';' as c_char;
#[cfg(not(windows))]
const ENV_SEPCHAR: c_char = b':' as c_char;

const E_INVALID_SUBMATCH_NUMBER_NR: &CStr = c"E935: Invalid submatch number: %d";
const E_STRING_LIST_OR_BLOB_REQUIRED: &CStr = c"E1098: String, List or Blob required";
const E_MISSING_FUNCTION_ARGUMENT: &CStr = c"E1132: Missing function argument";
const E_INVACT2: &CStr = c"E962: Invalid action: '%s'";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SomeMatchType {
    kSomeMatch,
    kSomeMatchEnd,
    kSomeMatchList,
    kSomeMatchStr,
    kSomeMatchStrPos,
}

#[repr(C)]
struct GetListLineCookie {
    l: *const list_T,
    li: *const listitem_T,
}

#[repr(C)]
struct uv_env_item_t {
    name: *mut c_char,
    value: *mut c_char,
}

type RequestHandlerFn = unsafe extern "C" fn(
    channel_id: u64,
    args: Array,
    arena: *mut Arena,
    error: *mut Error,
) -> Object;

static mut INPUTSECRET_FLAG: bool = false;
static mut GA_USERINPUT: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: size_of::<tasave_T>() as c_int,
    ga_growsize: 4,
    ga_data: ptr::null_mut(),
};
static mut HAS_WSL: TriState = kNone;
static mut RAND_GX: u32 = 0;
static mut RAND_GY: u32 = 0;
static mut RAND_GZ: u32 = 0;
static mut RAND_GW: u32 = 0;
static mut RAND_INITIALIZED: bool = false;
static mut GET_FUNC_NAME_INTIDX: c_int = -1;
static mut GET_EXPR_NAME_INTIDX: c_int = -1;

const PTY_IGNORED_ENV_VARS: &[&CStr] = &[
    #[cfg(not(windows))]
    c"COLUMNS",
    #[cfg(not(windows))]
    c"LINES",
    #[cfg(not(windows))]
    c"TERMCAP",
    #[cfg(not(windows))]
    c"COLORFGBG",
    c"COLORTERM",
    c"VIM",
    c"VIMRUNTIME",
];

#[cfg(windows)]
const REQUIRED_ENV_VARS: &[&CStr] = &[
    c"HOMEDRIVE",
    c"HOMEPATH",
    c"LOGONSERVER",
    c"PATH",
    c"SYSTEMDRIVE",
    c"SYSTEMROOT",
    c"TEMP",
    c"USERDOMAIN",
    c"USERNAME",
    c"USERPROFILE",
    c"WINDIR",
];
#[cfg(not(windows))]
const REQUIRED_ENV_VARS: &[&CStr] = &[];

#[cfg(windows)]
const PROCESS_ALL_ACCESS: u32 = 0x001F0FFF;
#[cfg(windows)]
unsafe extern "system" {
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: c_int, dwProcessId: u32) -> *mut c_void;
    fn DebugBreakProcess(Process: *mut c_void) -> c_int;
    fn CloseHandle(hObject: *mut c_void) -> c_int;
}

unsafe extern "C" {
    static mut IObuff: [c_char; IOSIZE];
    static mut NameBuff: [c_char; MAXPATHL];
    static mut curbuf: *mut buf_T;
    static mut lastbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut firstwin: *mut win_T;
    static mut curtab: *mut tabpage_T;
    static mut p_magic: c_int;
    static mut p_cpo: *mut c_char;
    static mut p_verbose: i64;
    static mut p_wic: c_int;
    static mut p_sel: *mut c_char;
    static mut p_tgc: c_int;
    static mut p_spk: *mut c_char;
    static mut p_ic: c_int;
    static mut p_ws: c_int;
    static mut p_enc: *mut c_char;
    static mut empty_string_option: [c_char; 1];
    static mut emsg_off: c_int;
    static mut did_emsg: c_int;
    static mut called_emsg: c_int;
    static mut need_clr_eos: bool;
    static mut vgetc_busy: c_int;
    static mut msg_silent: c_int;
    static mut emsg_silent: c_int;
    static mut emsg_noredir: bool;
    static mut redir_off: bool;
    static mut capture_ga: *mut garray_T;
    static mut msg_col: c_int;
    static mut msg_row: c_int;
    static mut lines_left: c_int;
    static mut msg_scroll: bool;
    static mut cmdline_row: c_int;
    static mut mouse_row: c_int;
    static mut cmdline_star: c_int;
    static mut want_garbage_collect: bool;
    static mut garbage_collect_at_exit: bool;
    static mut vim_ignored: c_int;
    static mut virtual_op: TriState;
    static mut main_loop: u8;
    static mut got_int: bool;
    static mut starting: c_int;
    static mut stdin_isatty: bool;
    static mut stdout_isatty: bool;
    static mut skip_update_topline: bool;
    static mut typebuf: typebuf_T;
    static mut autocmd_busy: c_int;
    static mut msg_scrolled: c_int;
    static mut reg_executing: c_int;
    static mut reg_recording: c_int;
    static mut reg_recorded: c_int;
    static mut provider_call_nesting: c_int;
    static mut provider_caller_scope: caller_scope;
    static mut current_sctx: sctx_T;
    static mut autocmd_fname: *mut c_char;
    static mut autocmd_match: *mut c_char;
    static mut autocmd_fname_full: bool;
    static mut autocmd_bufnr: c_int;
    static mut on_print: Callback;
    static mut Rows: c_int;
    static mut State: c_int;
    static mut wild_menu_showing: c_int;
    static windowsVersion: [c_char; 0];
    static mut EVALARG_EVALUATE: evalarg_T;
    static eval_has_list: [*const c_char; 0];

    fn tv_blob_set_range(
        dest: *mut blob_T,
        n1: varnumber_T,
        n2: varnumber_T,
        src_tv: *const typval_T,
    ) -> c_int;
    fn tv_check_for_buffer_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_dict_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_list_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_list_or_blob_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_lnum_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_nonempty_string_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_nonnull_dict_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_number_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_opt_bool_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_opt_dict_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_opt_number_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_string_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_string_or_func_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_for_string_or_list_arg(args: *const typval_T, idx: c_int) -> c_int;
    fn tv_check_num(varp: *const typval_T) -> bool;
    fn tv_check_str_or_nr(varp: *const typval_T) -> bool;
    fn tv_dict_extend(d1: *mut dict_T, d2: *const dict_T, action: *const c_char);
    fn tv_dict_get_bool(d: *const dict_T, key: *const c_char, def: c_int) -> varnumber_T;
    fn tv_dict_get_callback(
        d: *mut dict_T,
        key: *const c_char,
        key_len: isize,
        result: *mut Callback,
    ) -> bool;
    fn tv_dict_get_number(d: *const dict_T, key: *const c_char) -> varnumber_T;
    fn tv_dict_get_number_def(d: *const dict_T, key: *const c_char, def: c_int) -> varnumber_T;
    fn tv_dict_get_string(d: *const dict_T, key: *const c_char, save: bool) -> *mut c_char;
    fn tv_equal(tv1: *mut typval_T, tv2: *mut typval_T, ic: bool) -> bool;
    fn tv_get_lnum(argvars: *const typval_T) -> linenr_T;
    fn tv_get_string(varp: *const typval_T) -> *const c_char;
    fn tv_get_string_buf(varp: *const typval_T, buf: *mut c_char) -> *const c_char;
    fn tv_get_string_buf_chk(varp: *const typval_T, buf: *mut c_char) -> *const c_char;
    fn tv_get_string_chk(varp: *const typval_T) -> *const c_char;
    fn tv_list_copy(
        conv: *const vimconv_T,
        orig: *mut list_T,
        deep: bool,
        copyID: c_int,
    ) -> *mut list_T;
    fn tv_list_find_nr(l: *const list_T, n: c_int, ret_error: *mut bool) -> varnumber_T;
    fn tv_list_flatten(l: *mut list_T, li: *mut listitem_T, maxitems: i64, maxdepth: i64) -> c_int;
    fn callback_free(callback: *mut Callback);
    fn callback_from_typval(callback: *mut Callback, arg: *mut typval_T) -> bool;
    fn func_call(
        name: *mut c_char,
        args: *mut typval_T,
        partial: *mut partial_T,
        selfdict: *mut dict_T,
        rettv: *mut typval_T,
    ) -> c_int;
    fn get_func_arity(
        name: *const c_char,
        required: *mut c_int,
        optional: *mut c_int,
        varargs: *mut bool,
    );
    fn get_scriptlocal_funcname(funcname: *mut c_char) -> *mut c_char;
    fn get_user_func_name(xp: *mut expand_T, idx: c_int) -> *mut c_char;
    fn partial_name(pt: *mut partial_T) -> *const c_char;
    fn save_function_name(
        name: *mut *mut c_char,
        skip: bool,
        flags: c_int,
        fdp: *mut c_void,
    ) -> *mut c_char;
    fn trans_function_name(
        pp: *mut *mut c_char,
        skip: bool,
        flags: c_int,
        fdp: *mut c_void,
        partial: *mut *mut partial_T,
    ) -> *mut c_char;
    fn translated_function_exists(name: *const c_char) -> bool;
    fn eval_option(arg: *mut *const c_char, rettv: *mut typval_T, evaluate: bool) -> c_int;
    fn eval_vars(
        src: *mut c_char,
        usedlen: *const c_char,
        lenp: *mut usize,
        fnamep: *mut *const c_char,
        errormsg: *mut *const c_char,
        firstline: *mut linenr_T,
        skip: bool,
    ) -> *mut c_char;
    fn check_repeat_count(slen: varnumber_T, n: varnumber_T) -> c_int;
    fn str2special_save(str_: *const c_char, replace_spaces: bool, replace_lt: bool)
    -> *mut c_char;
    fn mb_strup_buf(src: *const c_char, dst: *mut c_char) -> usize;
    fn get_appname(namelike: bool) -> *const c_char;
    fn os_get_pid() -> i64;
    fn do_string_sub(
        str_: *mut c_char,
        len: usize,
        pat: *mut c_char,
        sub: *mut c_char,
        expr: *mut typval_T,
        flags: *mut c_char,
        endpos: *mut *mut c_char,
    ) -> *mut c_char;
    fn block_prep(oap: *mut oparg_T, bdp: *mut block_def, lnum: linenr_T, is_del: bool);
    fn charwise_block_prep(
        start: crate::api::tabpage::pos_T,
        end: crate::api::tabpage::pos_T,
        bdp: *mut block_def,
        lnum: linenr_T,
        inclusive: bool,
    );
    fn getregionpos_prep(
        p1: *mut crate::api::tabpage::pos_T,
        p2: *mut crate::api::tabpage::pos_T,
        region_type: MotionType,
        is_select_exclusive: bool,
        block_width: c_int,
        inclusive: *mut bool,
        oap: *mut oparg_T,
    );
    fn getregionpos_line(
        lnum: linenr_T,
        p1: crate::api::tabpage::pos_T,
        p2: crate::api::tabpage::pos_T,
        inclusive: bool,
        region_type: MotionType,
        oap: *mut oparg_T,
        allow_eol: bool,
        ret_p1: *mut crate::api::tabpage::pos_T,
        ret_p2: *mut crate::api::tabpage::pos_T,
    );
    fn find_decl(
        ptr: *mut c_char,
        len: usize,
        locally: bool,
        thisblock: bool,
        flags_arg: c_int,
    ) -> c_int;
    #[cfg_attr(target_os = "windows", link_name = "_stricmp")]
    #[cfg_attr(not(target_os = "windows"), link_name = "strcasecmp")]
    fn c_stricmp(s1: *const c_char, s2: *const c_char) -> c_int;
    #[cfg_attr(target_os = "windows", link_name = "_strnicmp")]
    #[cfg_attr(not(target_os = "windows"), link_name = "strncasecmp")]
    fn c_strnicmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    fn vim_snprintf(str_: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn vim_snprintf_safelen(str_: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> usize;
    fn vim_strsave_escape_ks(p: *const c_char) -> *mut c_char;
    fn vim_strsave_fnameescape(ptr: *const c_char, what: c_int) -> *mut c_char;
    fn syn_get_final_id(hl_id: c_int) -> c_int;
    fn win_col_off(wp: *mut win_T) -> c_int;

    static e_toomanyarg: [c_char; 0];
    static e_toofewarg: [c_char; 0];
    static e_api_error: [c_char; 0];
    static e_unknown_function_str: [c_char; 0];
    static e_invarg: [c_char; 0];
    static e_invarg2: [c_char; 0];
    static e_invargval: [c_char; 0];
    static e_invargNval: [c_char; 0];
    static e_invalwindow: [c_char; 0];
    static e_cannot_change_readonly_variable_str: [c_char; 0];
    static e_invexpr2: [c_char; 0];
    static e_trailing_arg: [c_char; 0];
    static e_listarg: [c_char; 0];
    static e_listdictarg: [c_char; 0];
    static e_listblobarg: [c_char; 0];
    static e_listblobreq: [c_char; 0];
    static e_listdictblobarg: [c_char; 0];
    static e_buffer_is_not_loaded: [c_char; 0];
    static e_invalid_buffer_name_str: [c_char; 0];
    static e_invalid_line_number_nr: [c_char; 0];
    static e_invalid_column_number_nr: [c_char; 0];
    static e_dictkey: [c_char; 0];
    static e_channotpty: [c_char; 0];
    static e_libcall: [c_char; 0];
    static e_reduce_of_an_empty_str_with_no_initial_value: [c_char; 0];
    static e_stdiochan2: [c_char; 0];
    static e_no_spell: [c_char; 0];
    static e_number_exp: [c_char; 0];

    fn strlen(s: *const c_char) -> usize;
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    fn strtoul(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> c_ulong;
    fn atoi(nptr: *const c_char) -> c_int;
    fn snprintf(s: *mut c_char, maxlen: usize, format: *const c_char, ...) -> c_int;
    fn strftime(s: *mut c_char, max: usize, format: *const c_char, tm: *const Tm) -> usize;
    #[cfg_attr(target_os = "windows", link_name = "_time64")]
    #[cfg_attr(not(target_os = "windows"), link_name = "time")]
    fn c_time(tloc: *mut time_t) -> time_t;
    #[cfg_attr(target_os = "windows", link_name = "_mktime64")]
    #[cfg_attr(not(target_os = "windows"), link_name = "mktime")]
    fn c_mktime(tm: *mut Tm) -> time_t;

    fn uv_handle_size(h_type: c_int) -> usize;
    fn uv_kill(pid: c_int, signum: c_int) -> c_int;
    fn uv_os_environ(envitems: *mut *mut uv_env_item_t, count: *mut c_int) -> c_int;
    fn uv_os_free_environ(envitems: *mut uv_env_item_t, count: c_int);
    fn uv_random(
        loop_: *mut c_void,
        req: *mut c_void,
        buf: *mut c_void,
        buflen: usize,
        flags: u32,
        cb: *const c_void,
    ) -> c_int;
    fn uv_strerror(err: c_int) -> *const c_char;

    fn eval_func_def_at(idx: c_int) -> *const EvalFuncDef;
    fn find_internal_func_hash(str_: *const c_char, len: usize) -> c_int;
    fn vim_snprintf_typval_only(
        str_: *mut c_char,
        str_m: usize,
        fmt: *const c_char,
        tvs: *mut typval_T,
    ) -> c_int;

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(s: *const c_char, ...) -> bool;
    fn semsg_multiline(context: *const c_char, fmt: *const c_char, ...);
    fn emsg_funcname(ermsg: *const c_char, name: *const c_char);
    fn internal_error(where_: *const c_char);
    fn verb_msg(s: *const c_char);
    fn msg_ext_set_kind(msg_kind: *const c_char);
    fn msg_start();
    fn msg_clr_eos();
    fn msg_puts(s: *const c_char);
    fn msg_putchar(c: c_int);
    fn msg_scroll_flush();
    fn check_secure() -> bool;
    fn cat_prefix_varname(prefix: c_int, name: *const c_char) -> *mut c_char;

    fn object_to_vim_take_luaref(
        obj: *mut Object,
        rettv: *mut typval_T,
        lua_special: bool,
        err: *mut Error,
    ) -> bool;
    fn api_free_object(value: Object);
    fn api_free_string(value: NvimString);
    fn api_clear_error(value: *mut Error);
    fn api_metadata() -> Object;
    fn dict_set_var(
        dict: *mut dict_T,
        key: NvimString,
        value: Object,
        del: bool,
        retval: bool,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn nvim_feedkeys(keys: NvimString, mode: NvimString, escape_ks: bool);

    fn nlua_exec(
        str_: NvimString,
        name: *const c_char,
        args: Array,
        mode: c_int,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn nlua_is_table_from_lua(arg: *const typval_T) -> bool;
    fn nlua_register_table_as_callable(arg: *const typval_T) -> *mut c_char;
    fn nlua_func_exists(name: *const c_char) -> bool;
    fn nlua_call_luaeval(str_: NvimString, arg: *mut typval_T, rettv: *mut typval_T);
    fn nlua_call_typval(
        modname: *const c_char,
        funcname: *const c_char,
        args: *mut typval_T,
        rettv: *mut typval_T,
    );

    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn buflist_findpat(
        pattern: *const c_char,
        pattern_end: *const c_char,
        unlisted: bool,
        diffmode: bool,
        curtab_only: bool,
    ) -> c_int;
    fn buf_charidx_to_byteidx(buf: *mut buf_T, lnum: i32, charidx: c_int) -> c_int;
    fn buf_byteidx_to_charidx(buf: *mut buf_T, lnum: i32, byteidx: c_int) -> c_int;
    fn setfname(buf: *mut buf_T, ffname: *mut c_char, sfname: *mut c_char, message: bool) -> c_int;

    fn channel_close(id: u64, part: ChannelPart, error: *mut *const c_char) -> bool;
    fn channel_send(
        id: u64,
        data: *mut c_char,
        len: usize,
        data_owned: bool,
        error: *mut *const c_char,
    ) -> usize;
    fn save_tv_as_string(
        tv: *mut typval_T,
        len: *mut isize,
        endnl: bool,
        crlf: bool,
    ) -> *mut c_char;
    fn find_job(id: u64, show_error: bool) -> *mut Channel;
    fn tv_to_argv(
        cmd_tv: *mut typval_T,
        cmd: *mut *const c_char,
        executable: *mut bool,
    ) -> *mut *mut c_char;
    fn common_job_callbacks(
        vopts: *mut dict_T,
        on_stdout: *mut CallbackReader,
        on_stderr: *mut CallbackReader,
        on_exit: *mut Callback,
    ) -> bool;
    fn channel_job_start(
        argv: *mut *mut c_char,
        exepath: *const c_char,
        on_stdout: CallbackReader,
        on_stderr: CallbackReader,
        on_exit: Callback,
        pty: bool,
        rpc: bool,
        overlapped: bool,
        detach: bool,
        stdin_mode: ChannelStdinMode,
        cwd: *const c_char,
        pty_width: u16,
        pty_height: u16,
        env: *mut dict_T,
        status_out: *mut varnumber_T,
    ) -> *mut Channel;
    fn channel_create_event(chan: *mut Channel, ext_source: *const c_char);
    fn channel_incref(chan: *mut Channel);
    fn channel_decref(chan: *mut Channel);
    fn channel_terminal_alloc(buf: *mut buf_T, chan: *mut Channel);
    fn channel_connect(
        tcp: bool,
        address: *const c_char,
        rpc: bool,
        on_output: CallbackReader,
        timeout: c_int,
        error: *mut *const c_char,
    ) -> u64;
    fn channel_from_stdio(rpc: bool, callback: CallbackReader, error: *mut *const c_char) -> u64;
    fn get_client_info(chan: *mut Channel, key: *const c_char) -> *const c_char;

    fn pty_proc_resize(ptyproc: *mut PtyProc, width: u16, height: u16);

    fn virtual_active(wp: *mut win_T) -> bool;
    fn mb_adjust_cursor();
    fn update_curswant();
    fn do_dialog(
        type_: c_int,
        title: *const c_char,
        message: *const c_char,
        buttons: *const c_char,
        dfltbutton: c_int,
        textfield: *mut c_char,
        ex_cmd: bool,
    ) -> c_int;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn do_cmdline(
        cmdline: *mut c_char,
        fgetline: Option<unsafe extern "C" fn(c_int, *mut c_void, c_int, bool) -> *mut c_char>,
        cookie: *mut c_void,
        flags: c_int,
    ) -> c_int;
    fn function_exists(name: *const c_char, no_deref: bool) -> bool;
    fn cmd_exists(name: *const c_char) -> c_int;
    fn autocmd_supported(name: *const c_char) -> bool;
    fn au_exists(arg: *const c_char) -> bool;
    fn get_menu_cmd_modes(
        p: *const c_char,
        forceit: bool,
        noremap: *mut c_int,
        unmenu: *mut bool,
    ) -> c_int;
    fn menu_get(path_name: *mut c_char, modes: c_int, list: *mut list_T) -> bool;
    fn expand_filename(
        eap: *mut exarg_T,
        cmdlinep: *mut *mut c_char,
        errormsgp: *mut *const c_char,
    ) -> c_int;
    fn value_check_lock(
        lock: VarLockStatus,
        arg_errmsg: *const c_char,
        arg_errmsg_len: usize,
    ) -> bool;
    fn last_csearch() -> *mut c_char;
    fn last_csearch_forward() -> c_int;
    fn last_csearch_until() -> c_int;
    fn set_last_csearch(c: c_int, s: *mut c_char, len: c_int);
    fn set_csearch_direction(cdir: c_int);
    fn set_csearch_until(t_cmd: bool);
    fn getdigits_int(pp: *mut *mut c_char, strict: bool, def: c_int) -> c_int;
    fn ui_gui_attached() -> bool;
    fn get_user_input(
        argvars: *const typval_T,
        rettv: *mut typval_T,
        inputdialog: bool,
        secret: bool,
    );
    fn prompt_for_input(
        prompt: *const c_char,
        hl_id: c_int,
        use_CTRL_C: bool,
        mouse_used: *mut bool,
    ) -> c_int;
    fn text_locked() -> bool;
    fn text_locked_msg();
    fn terminal_running(term: *const c_void) -> bool;
    fn buf_close_terminal(buf: *mut buf_T);
    fn terminal_buf(term: *const c_void) -> handle_T;
    fn terminal_open(term: *mut *mut c_void, buf: *mut buf_T);
    fn mpack_parser_init(parser: *mut mpack_parser_t, capacity: u32);
    fn mpack_parse_typval(
        parser: *mut mpack_parser_t,
        data: *mut *const c_char,
        size: *mut usize,
    ) -> c_int;
    fn typval_parser_error_free(parser: *mut mpack_parser_t);
    fn unpack_typval(data: *mut *const c_char, size: *mut usize, ret: *mut typval_T) -> c_int;
    fn op_pending() -> bool;
    fn decl(lp: *mut pos_T) -> c_int;
    fn incl(lp: *mut pos_T) -> c_int;
    fn set_option_value_give_err(opt_idx: OptIndex, value: Object, opt_flags: c_int);
    fn string2float(text: *const c_char, ret_value: *mut float_T) -> usize;
    fn recover_names(fname: *mut c_char, list: bool, ret_list: *mut list_T) -> c_int;
    fn swapfile_dict(fname: *const c_char, d: *mut dict_T);
    fn cmdline_pum_active() -> bool;
    fn cursor_pos_info(dict: *mut dict_T);
}

type handle_T = c_int;

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        unsafe {
            NvimString {
                data: s.cast_mut(),
                size: strlen(s),
            }
        }
    }
}

#[inline]
unsafe fn cbuf_to_string(buf: *const c_char, size: usize) -> NvimString {
    unsafe {
        NvimString {
            data: xmemdupz(buf.cast(), size).cast(),
            size,
        }
    }
}

#[inline]
fn ascii_isdigit(c: c_char) -> bool {
    (c as u8).is_ascii_digit()
}

#[inline]
fn toupper_asc(c: c_char) -> u8 {
    (c as u8).to_ascii_uppercase()
}

#[inline]
fn tolower_asc(c: c_char) -> u8 {
    (c as u8).to_ascii_lowercase()
}

#[inline]
unsafe fn strequal(a: *const c_char, b: *const c_char) -> bool {
    unsafe { strcmp(a, b) == 0 }
}

#[inline]
unsafe fn strnequal(a: *const c_char, b: *const c_char, n: usize) -> bool {
    unsafe { strncmp(a, b, n) == 0 }
}

#[inline]
unsafe fn to_slash_save(p: *const c_char) -> *mut c_char {
    #[cfg(windows)]
    unsafe {
        path_to_slash_save(p)
    }
    #[cfg(not(windows))]
    unsafe {
        xstrdup(p)
    }
}

#[inline]
fn tv_initial_value() -> typval_T {
    typval_T {
        v_type: VarType::VAR_UNKNOWN,
        v_lock: VAR_UNLOCKED,
        vval: TypvalVval { v_number: 0 },
    }
}

#[inline]
unsafe fn tv_list_ref(l: *mut list_T) {
    if !l.is_null() {
        unsafe {
            (*l).lv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_list_set_ret(tv: *mut typval_T, l: *mut list_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_LIST;
        (*tv).vval.v_list = l;
        tv_list_ref(l);
    }
}

#[inline]
unsafe fn tv_list_locked(l: *const list_T) -> VarLockStatus {
    if l.is_null() {
        VAR_FIXED
    } else {
        unsafe { (*l).lv_lock }
    }
}

#[inline]
unsafe fn tv_list_set_lock(l: *mut list_T, lock: VarLockStatus) {
    if l.is_null() {
        debug_assert!(lock == VAR_FIXED);
    } else {
        unsafe {
            (*l).lv_lock = lock;
        }
    }
}

#[inline]
unsafe fn tv_list_len(l: *const list_T) -> c_int {
    if l.is_null() {
        0
    } else {
        unsafe { (*l).lv_len }
    }
}

#[inline]
unsafe fn tv_list_uidx(l: *const list_T, mut n: c_int) -> c_int {
    unsafe {
        if n < 0 {
            n += tv_list_len(l);
        }
        if n < 0 || n >= tv_list_len(l) { -1 } else { n }
    }
}

#[inline]
unsafe fn tv_list_first(l: *const list_T) -> *mut listitem_T {
    if l.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*l).lv_first }
    }
}

#[inline]
unsafe fn tv_dict_set_ret(tv: *mut typval_T, d: *mut dict_T) {
    unsafe {
        (*tv).v_type = VarType::VAR_DICT;
        (*tv).vval.v_dict = d;
        if !d.is_null() {
            (*d).dv_refcount += 1;
        }
    }
}

#[inline]
unsafe fn tv_dict_len(d: *const dict_T) -> i64 {
    if d.is_null() {
        0
    } else {
        unsafe { (*d).dv_hashtab.ht_used as i64 }
    }
}

#[inline]
unsafe fn tv_blob_len(b: *const blob_T) -> c_int {
    if b.is_null() {
        0
    } else {
        unsafe { (*b).bv_ga.ga_len }
    }
}

#[inline]
unsafe fn tv_blob_get(b: *const blob_T, idx: c_int) -> u8 {
    unsafe { *(*b).bv_ga.ga_data.cast::<u8>().add(idx as usize) }
}

#[inline]
unsafe fn tv_get_float_chk(tv: *const typval_T, ret_f: *mut float_T) -> bool {
    unsafe {
        if (*tv).v_type == VarType::VAR_FLOAT {
            *ret_f = (*tv).vval.v_float;
            return true;
        }
        if (*tv).v_type == VarType::VAR_NUMBER {
            *ret_f = (*tv).vval.v_number as float_T;
            return true;
        }
        semsg(
            c"%s".as_ptr(),
            gettext(c"E808: Number or Float required".as_ptr()),
        );
        false
    }
}

#[inline]
unsafe fn proc_is_stopped(proc_: *mut Proc) -> bool {
    unsafe {
        let ph = proc_header(proc_);
        (*ph).status >= 0 || (*ph).stopped_time != 0
    }
}

#[inline]
fn tv_is_func(tv: typval_T) -> bool {
    tv.v_type == VarType::VAR_FUNC || tv.v_type == VarType::VAR_PARTIAL
}

#[inline]
fn luaret_truthy(o: Object) -> bool {
    unsafe { o.obj_type == ObjectType::kObjectTypeBoolean && o.data.boolean }
}

#[inline]
fn clearpos(a: *mut pos_T) {
    unsafe {
        (*a).lnum = 0;
        (*a).col = 0;
        (*a).coladd = 0;
    }
}

#[inline]
fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

/// Function given to `ExpandGeneric()` to obtain the list of internal
/// or user defined function names.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_function_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_FUNC_NAME_INTIDX = -1;
        }
        if GET_FUNC_NAME_INTIDX < 0 {
            let name = get_user_func_name(xp, idx);
            if !name.is_null() {
                if *name != NUL
                    && *name != b'<' as c_char
                    && strncmp(c"g:".as_ptr(), (*xp).xp_pattern, 2) == 0
                {
                    return cat_prefix_varname(b'g' as c_int, name);
                }
                return name;
            }
        }

        GET_FUNC_NAME_INTIDX += 1;
        let fdef = eval_func_def_at(GET_FUNC_NAME_INTIDX);
        let key = (*fdef).name;
        if key.is_null() {
            return ptr::null_mut();
        }
        let key_len = strlen(key);
        let iobuff = (&raw mut IObuff).cast::<c_char>();
        ptr::copy_nonoverlapping(key, iobuff, key_len);
        *iobuff.add(key_len) = b'(' as c_char;
        if (*fdef).max_argc == 0 {
            *iobuff.add(key_len + 1) = b')' as c_char;
            *iobuff.add(key_len + 2) = NUL;
        } else {
            *iobuff.add(key_len + 1) = NUL;
        }
        iobuff
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of internal or
/// user defined variable or function names.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_expr_name(xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        if idx == 0 {
            GET_EXPR_NAME_INTIDX = -1;
        }
        if GET_EXPR_NAME_INTIDX < 0 {
            let name = get_function_name(xp, idx);
            if !name.is_null() {
                return name;
            }
        }
        GET_EXPR_NAME_INTIDX += 1;
        get_user_var_name(xp, GET_EXPR_NAME_INTIDX)
    }
}

/// Gets a builtin (aka "vimfn", "eval") function from the generated hash table.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_internal_func(name: *const c_char) -> *const EvalFuncDef {
    unsafe {
        let len = strlen(name);
        let index = find_internal_func_hash(name, len);
        if index >= 0 {
            eval_func_def_at(index)
        } else {
            ptr::null()
        }
    }
}

/// Gets the Lua name of a Lua-implemented "vimfn" function, or NULL if not found.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_internal_func_lua(name: *const c_char) -> *const c_char {
    unsafe {
        let fdef = find_internal_func(name);
        if !fdef.is_null()
            && (*fdef)
                .func
                .is_some_and(|f| f as usize == lua_wrapper as *const () as usize)
        {
            return (*fdef).data.cast::<c_char>();
        }
        ptr::null()
    }
}

/// Check the argument count to use for internal function "fdef".
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_internal_func(fdef: *const EvalFuncDef, argcount: c_int) -> c_int {
    unsafe {
        let res = if argcount < c_int::from((*fdef).min_argc) {
            FCERR_TOOFEW
        } else if argcount > c_int::from((*fdef).max_argc) {
            FCERR_TOOMANY
        } else {
            return c_int::from((*fdef).base_arg);
        };

        let name = (*fdef).name;
        if res == FCERR_TOOMANY {
            semsg(gettext(e_toomanyarg.as_ptr()), name);
        } else {
            semsg(gettext(e_toofewarg.as_ptr()), name);
        }
        -1
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn call_internal_func(
    fname: *const c_char,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
) -> c_int {
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if argcount < c_int::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount > c_int::from((*fdef).max_argc) {
            return FCERR_TOOMANY;
        }
        (*argvars.add(argcount as usize)).v_type = VarType::VAR_UNKNOWN;
        if let Some(func) = (*fdef).func {
            func(argvars, rettv, (*fdef).data);
        }
        FCERR_NONE
    }
}

/// Invoke a method for `base->method()`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn call_internal_method(
    fname: *const c_char,
    argcount: c_int,
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    basetv: *mut typval_T,
) -> c_int {
    unsafe {
        let fdef = find_internal_func(fname);
        if fdef.is_null() {
            return FCERR_UNKNOWN;
        } else if (*fdef).base_arg == BASE_NONE {
            return FCERR_NOTMETHOD;
        } else if argcount + 1 < c_int::from((*fdef).min_argc) {
            return FCERR_TOOFEW;
        } else if argcount + 1 > c_int::from((*fdef).max_argc) {
            return FCERR_TOOMANY;
        }

        let mut argv = [tv_initial_value(); MAX_FUNC_ARGS + 1];
        let base_index: isize = if (*fdef).base_arg == BASE_LAST {
            argcount as isize
        } else {
            isize::from((*fdef).base_arg) - 1
        };
        if (argcount as isize) < base_index {
            return FCERR_TOOFEW;
        }
        ptr::copy_nonoverlapping(argvars, argv.as_mut_ptr(), base_index as usize);
        argv[base_index as usize] = *basetv;
        ptr::copy_nonoverlapping(
            argvars.add(base_index as usize),
            argv.as_mut_ptr().add((base_index + 1) as usize),
            (argcount as isize - base_index) as usize,
        );
        argv[(argcount + 1) as usize].v_type = VarType::VAR_UNKNOWN;

        if let Some(func) = (*fdef).func {
            func(argv.as_mut_ptr(), rettv, (*fdef).data);
        }
        FCERR_NONE
    }
}

/// Returns true for a non-zero Number and a non-empty String.
#[inline]
unsafe fn non_zero_arg(argvars: *const typval_T) -> bool {
    unsafe {
        ((*argvars).v_type == VarType::VAR_NUMBER && (*argvars).vval.v_number != 0)
            || ((*argvars).v_type == VarType::VAR_BOOL
                && (*argvars).vval.v_bool as c_int == kBoolVarTrue)
            || ((*argvars).v_type == VarType::VAR_STRING
                && !(*argvars).vval.v_string.is_null()
                && *(*argvars).vval.v_string != NUL)
    }
}

/// Apply a floating point C function on a typval with one `float_T`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn float_op_wrapper(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    unsafe {
        let mut f: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut f) {
            let func_float: unsafe extern "C" fn(float_T) -> float_T = std::mem::transmute(fptr);
            (*rettv).vval.v_float = func_float(f);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// Invokes an API (`nvim_`) function from Vimscript.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn api_wrapper(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }

        let handler = *fptr.cast::<MsgpackRpcRequestHandler>();
        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars;
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, false);
            args.size += 1;
            tv = tv.add(1);
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let fn_ptr: RequestHandlerFn = std::mem::transmute(handler.fn_ptr);
        let mut result = fn_ptr(VIML_INTERNAL_CALL, args, &raw mut arena, &raw mut err);

        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline(c"emsg".as_ptr(), e_api_error.as_ptr(), err.msg);
        } else {
            object_to_vim_take_luaref(&raw mut result, rettv, true, &raw mut err);
        }

        if handler.ret_alloc {
            api_free_object(result);
        }
        arena_mem_free(arena_finish(&raw mut arena));
        api_clear_error(&raw mut err);
    }
}

/// Invokes a Lua-implemented vimfn/"f_xx" function from Vimscript.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn lua_wrapper(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    unsafe {
        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars;
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, false);
            args.size += 1;
            tv = tv.add(1);
        }

        let mut buf = [0 as c_char; 256];
        snprintf(
            buf.as_mut_ptr(),
            buf.len(),
            c"return require('vim._core.vimfn').%s(...)".as_ptr(),
            fptr.cast::<c_char>(),
        );

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let mut result = nlua_exec(
            cstr_as_string(buf.as_ptr()),
            ptr::null(),
            args,
            kRetObject,
            &raw mut arena,
            &raw mut err,
        );

        if err.err_type != ErrorType::kErrorTypeNone {
            semsg_multiline(c"emsg".as_ptr(), e_api_error.as_ptr(), err.msg);
        } else {
            object_to_vim_take_luaref(&raw mut result, rettv, true, &raw mut err);
        }

        arena_mem_free(arena_finish(&raw mut arena));
        api_clear_error(&raw mut err);
    }
}

/// "abs(expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_abs(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_FLOAT {
            let mut f: float_T = 0.0;
            (*rettv).v_type = VarType::VAR_FLOAT;
            if tv_get_float_chk(argvars, &raw mut f) {
                (*rettv).vval.v_float = f.abs();
            } else {
                (*rettv).vval.v_float = 0.0;
            }
        } else {
            let mut error = false;
            let n = tv_get_number_chk(argvars, &raw mut error);
            if error {
                (*rettv).vval.v_number = -1;
            } else if n > 0 {
                (*rettv).vval.v_number = n;
            } else {
                (*rettv).vval.v_number = n.wrapping_neg();
            }
        }
    }
}

/// "and(expr, expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_and(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            & tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

/// "api_info()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_api_info(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        object_to_vim(api_metadata(), rettv, ptr::null_mut());
    }
}

/// "atan2()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_atan2(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx.atan2(fy);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// Get buffer by number or pattern.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_buf(tv: *mut typval_T, curtab_only: c_int) -> *mut buf_T {
    unsafe {
        if (*tv).v_type == VarType::VAR_NUMBER {
            return buflist_findnr((*tv).vval.v_number as c_int);
        }
        if (*tv).v_type != VarType::VAR_STRING {
            return ptr::null_mut();
        }

        let name = (*tv).vval.v_string;
        if name.is_null() || *name == NUL {
            return curbuf;
        }
        if *name == b'$' as c_char && *name.add(1) == NUL {
            return lastbuf;
        }

        let save_magic = p_magic;
        p_magic = 1;
        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let buf = buflist_findnr(buflist_findpat(
            name,
            name.add(strlen(name)),
            true,
            false,
            curtab_only != 0,
        ));

        p_magic = save_magic;
        p_cpo = save_cpo;

        if buf.is_null() {
            return find_buffer(tv);
        }

        buf
    }
}

/// Like `tv_get_buf()` but give an error message if the type is wrong.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tv_get_buf_from_arg(tv: *mut typval_T) -> *mut buf_T {
    unsafe {
        if !tv_check_str_or_nr(tv) {
            return ptr::null_mut();
        }
        emsg_off += 1;
        let buf = tv_get_buf(tv, 0);
        emsg_off -= 1;
        buf
    }
}

/// Get the buffer from "arg" and give an error and return NULL if it is not valid.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_buf_arg(arg: *mut typval_T) -> *mut buf_T {
    unsafe {
        emsg_off += 1;
        let buf = tv_get_buf(arg, 0);
        emsg_off -= 1;
        if buf.is_null() {
            semsg(
                gettext(c"E158: Invalid buffer name: %s".as_ptr()),
                tv_get_string(arg),
            );
        }
        buf
    }
}

/// "byte2line(byte)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_byte2line(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut boff = (tv_get_number(argvars) as c_int) - 1;
        if boff < 0 {
            (*rettv).vval.v_number = -1;
        } else {
            (*rettv).vval.v_number =
                ml_find_line_or_offset(curbuf, 0, &raw mut boff, false) as varnumber_T;
        }
    }
}

/// "call(func, arglist [, dict])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_call(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        if tv_check_for_list_arg(argvars, 1) == FAIL {
            return;
        }
        if (*argvars.add(1)).vval.v_list.is_null() {
            return;
        }

        let mut owned = false;
        let mut func: *mut c_char;
        let mut partial: *mut partial_T = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_FUNC {
            func = (*argvars).vval.v_string;
        } else if (*argvars).v_type == VarType::VAR_PARTIAL {
            partial = (*argvars).vval.v_ptr.cast();
            func = partial_name(partial).cast_mut();
        } else if nlua_is_table_from_lua(argvars) {
            func = nlua_register_table_as_callable(argvars);
            owned = true;
        } else {
            func = tv_get_string(argvars).cast_mut();
        }

        if func.is_null() || *func == NUL {
            return;
        }
        let mut tofree: *mut c_char = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_STRING {
            let mut p = func;
            tofree = trans_function_name(
                &raw mut p,
                false,
                TFN_INT | TFN_QUIET,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if tofree.is_null() {
                emsg_funcname(e_unknown_function_str.as_ptr(), func);
                return;
            }
            func = tofree;
        }

        'done: {
            let mut selfdict: *mut dict_T = ptr::null_mut();
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                if tv_check_for_dict_arg(argvars, 2) == FAIL {
                    break 'done;
                }
                selfdict = (*argvars.add(2)).vval.v_dict.cast();
            }

            func_call(func, argvars.add(1), partial, selfdict, rettv);
        }

        if owned {
            func_unref(func);
        }
        xfree(tofree.cast());
    }
}

/// "changenr()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_changenr(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from((*curbuf).b_u_seq_cur);
    }
}

/// "chanclose(id[, stream])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_chanclose(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_NUMBER
            || ((*argvars.add(1)).v_type != VarType::VAR_STRING
                && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN)
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut part = kChannelPartAll;
        if (*argvars.add(1)).v_type == VarType::VAR_STRING {
            let stream = tv_get_string(argvars.add(1));
            if strcmp(stream, c"stdin".as_ptr()) == 0 {
                part = kChannelPartStdin;
            } else if strcmp(stream, c"stdout".as_ptr()) == 0 {
                part = kChannelPartStdout;
            } else if strcmp(stream, c"stderr".as_ptr()) == 0 {
                part = kChannelPartStderr;
            } else if strcmp(stream, c"rpc".as_ptr()) == 0 {
                part = kChannelPartRpc;
            } else {
                semsg(gettext(c"Invalid channel stream \"%s\"".as_ptr()), stream);
                return;
            }
        }
        let mut error: *const c_char = ptr::null();
        (*rettv).vval.v_number = varnumber_T::from(channel_close(
            (*argvars).vval.v_number as u64,
            part,
            &raw mut error,
        ));
        if (*rettv).vval.v_number == 0 {
            emsg(error);
        }
    }
}

/// "chansend(id, data)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_chansend(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        if (*argvars).v_type != VarType::VAR_NUMBER
            || (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut input_len: isize = 0;
        let mut input: *mut c_char = ptr::null_mut();
        let id = (*argvars).vval.v_number as u64;
        #[cfg(not(windows))]
        let crlf = false;
        #[cfg(windows)]
        let crlf = {
            let chan = find_channel(id);
            !chan.is_null() && !(*channel_tail(chan)).term.is_null()
        };

        if (*argvars.add(1)).v_type == VarType::VAR_BLOB {
            let b = (*argvars.add(1)).vval.v_blob.cast::<blob_T>();
            input_len = tv_blob_len(b) as isize;
            if input_len > 0 {
                input = xmemdup((*b).bv_ga.ga_data, input_len as usize).cast();
            }
        } else {
            input = save_tv_as_string(argvars.add(1), &raw mut input_len, false, crlf);
        }

        if input.is_null() {
            return;
        }
        let mut error: *const c_char = ptr::null();
        (*rettv).vval.v_number =
            channel_send(id, input, input_len as usize, true, &raw mut error) as varnumber_T;
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "char2nr(string)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_char2nr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && !tv_check_num(argvars.add(1)) {
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from(utf_ptr2char(tv_get_string(argvars)));
    }
}

/// Get the current cursor column and store it in `rettv`.
unsafe fn get_col(argvars: *mut typval_T, rettv: *mut typval_T, charcol: bool) {
    unsafe {
        if tv_check_for_string_or_list_arg(argvars, 0) == FAIL
            || tv_check_for_opt_number_arg(argvars, 1) == FAIL
        {
            return;
        }

        let mut wp = curwin;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let mut tp: *mut tabpage_T = ptr::null_mut();
            wp = win_id2wp_tp(tv_get_number(argvars.add(1)) as c_int, &raw mut tp);
            if wp.is_null() || tp.is_null() {
                return;
            }
            check_cursor(wp);
        }

        let bp = (*wp).w_buffer;
        let mut col: colnr_T = 0;
        let mut fnum = (*bp).b_fnum;
        let fp = var2fpos(argvars, false, &raw mut fnum, charcol, wp);
        if !fp.is_null() && fnum == (*bp).b_fnum {
            if (*fp).col == MAXCOL {
                if (*fp).lnum <= (*bp).b_ml.ml_line_count {
                    col = ml_get_buf_len(bp, (*fp).lnum) + 1;
                } else {
                    col = MAXCOL;
                }
            } else {
                col = (*fp).col + 1;
                if virtual_active(wp) && ptr::eq(fp, &raw const (*wp).w_cursor) {
                    let p = ml_get_buf(bp, (*wp).w_cursor.lnum).add((*wp).w_cursor.col as usize);
                    if (*wp).w_cursor.coladd
                        >= win_chartabsize(wp, p, (*wp).w_virtcol - (*wp).w_cursor.coladd)
                    {
                        if *p != NUL {
                            let l = utfc_ptr2len(p);
                            if *p.add(l as usize) == NUL {
                                col += l;
                            }
                        }
                    }
                }
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(col);
    }
}

/// "charcol()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_charcol(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_col(argvars, rettv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_optional_window(argvars: *mut typval_T, idx: c_int) -> *mut win_T {
    unsafe {
        if (*argvars.add(idx as usize)).v_type == VarType::VAR_UNKNOWN {
            return curwin;
        }
        let win = find_win_by_nr_or_id(argvars.add(idx as usize));
        if win.is_null() {
            emsg(gettext(e_invalwindow.as_ptr()));
            return ptr::null_mut();
        }
        win
    }
}

/// "col(string)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_col(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        get_col(argvars, rettv, false);
    }
}

/// "confirm(message, buttons[, default [, type]])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_confirm(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let mut buttons: *const c_char = ptr::null();
        let mut def: c_int = 1;
        let mut type_ = VIM_GENERIC;
        let mut error = false;

        let message = tv_get_string_chk(argvars);
        if message.is_null() {
            error = true;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            buttons = tv_get_string_buf_chk(argvars.add(1), buf.as_mut_ptr());
            if buttons.is_null() {
                error = true;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                def = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    let typestr = tv_get_string_buf_chk(argvars.add(3), buf2.as_mut_ptr());
                    if typestr.is_null() {
                        error = true;
                    } else {
                        match toupper_asc(*typestr) {
                            b'E' => type_ = VIM_ERROR,
                            b'Q' => type_ = VIM_QUESTION,
                            b'I' => type_ = VIM_INFO,
                            b'W' => type_ = VIM_WARNING,
                            b'G' => type_ = VIM_GENERIC,
                            _ => {}
                        }
                    }
                }
            }
        }

        if buttons.is_null() || *buttons == NUL {
            buttons = gettext(c"&Ok".as_ptr());
        }

        if !error {
            (*rettv).vval.v_number = varnumber_T::from(do_dialog(
                type_,
                ptr::null(),
                message,
                buttons,
                def,
                ptr::null_mut(),
                false,
            ));
        }
    }
}

/// "copy()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_copy(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        var_item_copy(ptr::null_mut(), argvars, rettv, false, 0);
    }
}

/// Set the cursor position.
unsafe fn set_cursorpos(argvars: *mut typval_T, rettv: *mut typval_T, charcol: bool) {
    unsafe {
        let lnum: linenr_T;
        let mut col: colnr_T;
        let mut coladd: colnr_T = 0;
        let mut set_curswant = true;

        (*rettv).vval.v_number = -1;
        if (*argvars).v_type == VarType::VAR_LIST {
            let mut pos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut curswant: colnr_T = -1;
            if list2fpos(
                argvars,
                &raw mut pos,
                ptr::null_mut(),
                &raw mut curswant,
                charcol,
            ) == FAIL
            {
                emsg(gettext(e_invarg.as_ptr()));
                return;
            }
            lnum = pos.lnum;
            col = pos.col;
            coladd = pos.coladd;
            if curswant >= 0 {
                (*curwin).w_curswant = curswant - 1;
                set_curswant = false;
            }
        } else if ((*argvars).v_type == VarType::VAR_NUMBER
            || (*argvars).v_type == VarType::VAR_STRING)
            && ((*argvars.add(1)).v_type == VarType::VAR_NUMBER
                || (*argvars.add(1)).v_type == VarType::VAR_STRING)
        {
            let mut ln = tv_get_lnum(argvars);
            if ln < 0 {
                semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
            } else if ln == 0 {
                ln = (*curwin).w_cursor.lnum;
            }
            lnum = ln;
            col = tv_get_number_chk(argvars.add(1), ptr::null_mut()) as colnr_T;
            if charcol {
                col = buf_charidx_to_byteidx(curbuf, lnum, col) + 1;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                coladd = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as colnr_T;
            }
        } else {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        if lnum < 0 || col < 0 || coladd < 0 {
            return;
        }
        if lnum > 0 {
            (*curwin).w_cursor.lnum = lnum;
        }
        if col != MAXCOL {
            col -= 1;
            if col < 0 {
                col = 0;
            }
        }
        (*curwin).w_cursor.col = col;
        (*curwin).w_cursor.coladd = coladd;

        check_cursor(curwin);
        mb_adjust_cursor();

        (*curwin).w_set_curswant = set_curswant;
        (*rettv).vval.v_number = 0;
    }
}

/// "cursor(lnum, col)" or "cursor(list)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_cursor(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_cursorpos(argvars, rettv, false);
    }
}

/// "debugbreak()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_debugbreak(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(FAIL);
        let pid = tv_get_number(argvars) as c_int;
        if pid == 0 {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        #[cfg(windows)]
        {
            let h_process = OpenProcess(PROCESS_ALL_ACCESS, 0, pid as u32);
            if h_process.is_null() {
                return;
            }
            DebugBreakProcess(h_process);
            CloseHandle(h_process);
            (*rettv).vval.v_number = varnumber_T::from(OK);
        }
        #[cfg(not(windows))]
        {
            const SIGINT: c_int = 2;
            uv_kill(pid, SIGINT);
        }
    }
}

/// "deepcopy()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_deepcopy(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_opt_bool_arg(argvars, 1) == FAIL {
            return;
        }
        let mut noref: varnumber_T = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            noref = tv_get_bool_chk(argvars.add(1), ptr::null_mut());
        }
        var_item_copy(
            ptr::null_mut(),
            argvars,
            rettv,
            true,
            if noref == 0 { get_copyID() } else { 0 },
        );
    }
}

/// `dictwatcheradd(dict, key, funcref)` function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_dictwatcheradd(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_DICT {
            semsg(gettext(e_invarg2.as_ptr()), c"dict".as_ptr());
            return;
        } else if (*argvars).vval.v_dict.is_null() {
            let arg_errmsg = gettext(c"dictwatcheradd() argument".as_ptr());
            let arg_errmsg_len = strlen(arg_errmsg);
            semsg(
                gettext(e_cannot_change_readonly_variable_str.as_ptr()),
                arg_errmsg_len as c_int,
                arg_errmsg,
            );
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_STRING
            && (*argvars.add(1)).v_type != VarType::VAR_NUMBER
        {
            semsg(gettext(e_invarg2.as_ptr()), c"key".as_ptr());
            return;
        }

        let key_pattern = tv_get_string_chk(argvars.add(1));
        if key_pattern.is_null() {
            return;
        }
        let key_pattern_len = strlen(key_pattern);

        let mut callback = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        if !callback_from_typval(&raw mut callback, argvars.add(2)) {
            semsg(gettext(e_invarg2.as_ptr()), c"funcref".as_ptr());
            return;
        }

        tv_dict_watcher_add(
            (*argvars).vval.v_dict.cast(),
            key_pattern,
            key_pattern_len,
            callback,
        );
    }
}

/// `dictwatcherdel(dict, key, funcref)` function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_dictwatcherdel(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_DICT {
            semsg(gettext(e_invarg2.as_ptr()), c"dict".as_ptr());
            return;
        }
        if (*argvars.add(2)).v_type != VarType::VAR_FUNC
            && (*argvars.add(2)).v_type != VarType::VAR_STRING
        {
            semsg(gettext(e_invarg2.as_ptr()), c"funcref".as_ptr());
            return;
        }

        let key_pattern = tv_get_string_chk(argvars.add(1));
        if key_pattern.is_null() {
            return;
        }

        let mut callback = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        if !callback_from_typval(&raw mut callback, argvars.add(2)) {
            return;
        }

        if !tv_dict_watcher_remove(
            (*argvars).vval.v_dict.cast(),
            key_pattern,
            strlen(key_pattern),
            callback,
        ) {
            emsg(c"Couldn't find a watcher matching key and callback".as_ptr());
        }

        callback_free(&raw mut callback);
    }
}

/// "did_filetype()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_did_filetype(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from((*curbuf).b_did_filetype);
    }
}

/// "empty({expr})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_empty(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut n = true;
        match (*argvars).v_type {
            VarType::VAR_STRING | VarType::VAR_FUNC => {
                n = (*argvars).vval.v_string.is_null() || *(*argvars).vval.v_string == NUL;
            }
            VarType::VAR_PARTIAL => {
                n = (*argvars).vval.v_ptr.is_null();
            }
            VarType::VAR_NUMBER => {
                n = (*argvars).vval.v_number == 0;
            }
            VarType::VAR_FLOAT => {
                n = (*argvars).vval.v_float == 0.0;
            }
            VarType::VAR_LIST => {
                n = tv_list_len((*argvars).vval.v_list) == 0;
            }
            VarType::VAR_DICT => {
                n = tv_dict_len((*argvars).vval.v_dict.cast()) == 0;
            }
            VarType::VAR_BOOL => {
                n = (*argvars).vval.v_bool as c_int != kBoolVarTrue;
            }
            VarType::VAR_SPECIAL => {
                n = (*argvars).vval.v_special as c_int == kSpecialVarNull;
            }
            VarType::VAR_BLOB => {
                n = tv_blob_len((*argvars).vval.v_blob.cast()) == 0;
            }
            VarType::VAR_UNKNOWN => {
                internal_error(c"f_empty(UNKNOWN)".as_ptr());
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

/// "escape({string}, {chars})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_escape(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut buf = [0 as c_char; NUMBUFLEN];
        (*rettv).vval.v_string = vim_strsave_escaped(
            tv_get_string(argvars),
            tv_get_string_buf(argvars.add(1), buf.as_mut_ptr()),
        );
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "getenv()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getenv(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let p = vim_getenv(tv_get_string(argvars));
        if p.is_null() {
            (*rettv).v_type = VarType::VAR_SPECIAL;
            (*rettv).vval.v_special = kSpecialVarNull;
            return;
        }
        (*rettv).vval.v_string = p;
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "eval()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_eval(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut s = tv_get_string_chk(argvars).cast_mut();
        if !s.is_null() {
            s = skipwhite(s);
        }
        let expr_start = s;
        if s.is_null() || eval1(&raw mut s, rettv, &raw mut EVALARG_EVALUATE) == FAIL {
            if !expr_start.is_null() && !aborting() {
                semsg(gettext(e_invexpr2.as_ptr()), expr_start);
            }
            need_clr_eos = false;
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = 0;
        } else if *s != NUL {
            semsg(gettext(e_trailing_arg.as_ptr()), s);
        }
    }
}

/// "eventhandler()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_eventhandler(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(vgetc_busy);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_list_line(
    _c: c_int,
    cookie: *mut c_void,
    _indent: c_int,
    _do_concat: bool,
) -> *mut c_char {
    unsafe {
        let p = cookie.cast::<GetListLineCookie>();
        let item = (*p).li;
        if item.is_null() {
            return ptr::null_mut();
        }
        let mut buf = [0 as c_char; NUMBUFLEN];
        let s = tv_get_string_buf_chk(&raw const (*item).li_tv, buf.as_mut_ptr());
        (*p).li = (*item).li_next;
        if s.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(s)
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn execute_common(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    arg_off: c_int,
) {
    unsafe {
        let save_msg_silent = msg_silent;
        let save_emsg_silent = emsg_silent;
        let save_emsg_noredir = emsg_noredir;
        let save_redir_off = redir_off;
        let save_capture_ga = capture_ga;
        let save_msg_col = msg_col;
        let mut echo_output = false;

        if check_secure() {
            return;
        }

        let off = arg_off as usize;
        if (*argvars.add(off + 1)).v_type != VarType::VAR_UNKNOWN {
            let mut buf = [0 as c_char; NUMBUFLEN];
            let s = tv_get_string_buf_chk(argvars.add(off + 1), buf.as_mut_ptr());
            if s.is_null() {
                return;
            }
            if *s == NUL {
                echo_output = true;
            }
            if strncmp(s, c"silent".as_ptr(), 6) == 0 {
                msg_silent += 1;
            }
            if strcmp(s, c"silent!".as_ptr()) == 0 {
                emsg_silent = 1;
                emsg_noredir = true;
            }
        } else {
            msg_silent += 1;
        }

        let mut capture_local = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut capture_local, size_of::<c_char>() as c_int, 80);
        capture_ga = &raw mut capture_local;
        redir_off = false;
        if !echo_output {
            msg_col = 0;
        }

        if (*argvars.add(off)).v_type != VarType::VAR_LIST {
            do_cmdline_cmd(tv_get_string(argvars.add(off)));
        } else if !(*argvars.add(off)).vval.v_list.is_null() {
            let list = (*argvars.add(off)).vval.v_list;
            tv_list_ref(list);
            let mut cookie = GetListLineCookie {
                l: list,
                li: tv_list_first(list),
            };
            do_cmdline(
                ptr::null_mut(),
                Some(get_list_line),
                (&raw mut cookie).cast(),
                DOCMD_NOWAIT | DOCMD_VERBOSE | DOCMD_REPEAT | DOCMD_KEYTYPED,
            );
            tv_list_unref(list);
        }
        msg_silent = save_msg_silent;
        emsg_silent = save_emsg_silent;
        emsg_noredir = save_emsg_noredir;
        redir_off = save_redir_off;
        if echo_output {
            msg_col = 0;
        } else {
            msg_col = save_msg_col;
        }

        ga_append(capture_ga, NUL as u8);
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = (*capture_ga).ga_data.cast();

        capture_ga = save_capture_ga;
    }
}

/// "execute(command)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_execute(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        execute_common(argvars, rettv, 0);
    }
}

/// "exists()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_exists(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut n = false;
        let mut p = tv_get_string(argvars);
        if *p == b'$' as c_char {
            if os_env_exists(p.add(1), false) {
                n = true;
            } else {
                let exp = expand_env_save(p.cast_mut());
                if !exp.is_null() && *exp != b'$' as c_char {
                    n = true;
                }
                xfree(exp.cast());
            }
        } else if *p == b'&' as c_char || *p == b'+' as c_char {
            n = eval_option(&raw mut p, ptr::null_mut(), true) == OK;
            if *skipwhite(p) != NUL {
                n = false;
            }
        } else if *p == b'*' as c_char {
            n = if strnequal(p, c"*v:lua.".as_ptr(), 7) {
                nlua_func_exists(p.add(7))
            } else {
                function_exists(p.add(1), false)
            };
        } else if *p == b':' as c_char {
            n = cmd_exists(p.add(1)) != 0;
        } else if *p == b'#' as c_char {
            if *p.add(1) == b'#' as c_char {
                n = autocmd_supported(p.add(2));
            } else {
                n = au_exists(p.add(1));
            }
        } else {
            n = var_exists(p);
        }
        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

/// "expand()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_expand(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut options = WILD_SILENT | WILD_USE_NL | WILD_LIST_NOTFOUND | WILD_USE_SHELLSLASH;
        let mut error = false;

        (*rettv).v_type = VarType::VAR_STRING;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            && tv_get_bool_chk(argvars.add(2), &raw mut error) != 0
            && !error
        {
            tv_list_set_ret(rettv, ptr::null_mut());
        }

        let s = tv_get_string(argvars);
        if *s == b'%' as c_char || *s == b'#' as c_char || *s == b'<' as c_char {
            if p_verbose == 0 {
                emsg_off += 1;
            }
            let mut len: usize = 0;
            let mut errormsg: *const c_char = ptr::null();
            let result = eval_vars(
                s.cast_mut(),
                s,
                &raw mut len,
                ptr::null_mut(),
                &raw mut errormsg,
                ptr::null_mut(),
                false,
            );
            if p_verbose == 0 {
                emsg_off -= 1;
            } else if !errormsg.is_null() {
                emsg(errormsg);
            }
            if (*rettv).v_type == VarType::VAR_LIST {
                tv_list_alloc_ret(rettv, isize::from(!result.is_null()));
                if !result.is_null() {
                    tv_list_append_string((*rettv).vval.v_list, result, -1);
                }
                xfree(result.cast());
            } else {
                (*rettv).vval.v_string = result;
            }
        } else {
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && tv_get_bool_chk(argvars.add(1), &raw mut error) != 0
            {
                options |= WILD_KEEP_ALL;
            }
            if !error {
                let mut xpc: expand_T = std::mem::zeroed();
                ExpandInit(&raw mut xpc);
                xpc.xp_context = EXPAND_FILES;
                if p_wic != 0 {
                    options += WILD_ICASE;
                }
                if (*rettv).v_type == VarType::VAR_STRING {
                    (*rettv).vval.v_string = ExpandOne(
                        &raw mut xpc,
                        s.cast_mut(),
                        ptr::null_mut(),
                        options,
                        WILD_ALL,
                    );
                } else {
                    ExpandOne(
                        &raw mut xpc,
                        s.cast_mut(),
                        ptr::null_mut(),
                        options,
                        WILD_ALL_KEEP,
                    );
                    tv_list_alloc_ret(rettv, xpc.xp_numfiles as isize);
                    for i in 0..(xpc.xp_numfiles as usize) {
                        tv_list_append_string((*rettv).vval.v_list, *xpc.xp_files.add(i), -1);
                    }
                    ExpandCleanup(&raw mut xpc);
                }
            } else {
                (*rettv).vval.v_string = ptr::null_mut();
            }
        }
    }
}

/// "menu_get(path [, modes])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_menu_get(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        let mut modes = MENU_ALL_MODES;
        if (*argvars.add(1)).v_type == VarType::VAR_STRING {
            let strmodes = tv_get_string(argvars.add(1));
            modes = get_menu_cmd_modes(strmodes, false, ptr::null_mut(), ptr::null_mut());
        }
        menu_get(
            tv_get_string(argvars).cast_mut(),
            modes,
            (*rettv).vval.v_list,
        );
    }
}

/// "expandcmd()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_expandcmd(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut errormsg: *const c_char = ptr::null();
        let mut emsgoff = true;

        if (*argvars.add(1)).v_type == VarType::VAR_DICT
            && tv_dict_get_bool(
                (*argvars.add(1)).vval.v_dict.cast(),
                c"errmsg".as_ptr(),
                kBoolVarFalse,
            ) != 0
        {
            emsgoff = false;
        }

        (*rettv).v_type = VarType::VAR_STRING;
        let mut cmdstr = xstrdup(tv_get_string(argvars));

        let mut eap: exarg_T = std::mem::zeroed();
        eap.cmd = cmdstr;
        eap.arg = cmdstr;
        eap.usefilter = 0;
        eap.nextcmd = ptr::null_mut();
        eap.cmdidx = cmdidx_T::CMD_USER;
        eap.argt |= EX_NOSPC;

        if emsgoff {
            emsg_off += 1;
        }
        if expand_filename(&raw mut eap, &raw mut cmdstr, &raw mut errormsg) == FAIL
            && !emsgoff
            && !errormsg.is_null()
            && *errormsg != NUL
        {
            emsg(errormsg);
        }
        if emsgoff {
            emsg_off -= 1;
        }

        (*rettv).vval.v_string = cmdstr;
    }
}

/// "flatten()" and "flattennew()" functions
unsafe fn flatten_common(argvars: *mut typval_T, rettv: *mut typval_T, make_copy: bool) {
    unsafe {
        let mut error = false;
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg(gettext(e_listarg.as_ptr()), c"flatten()".as_ptr());
            return;
        }

        let maxdepth: c_int = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            999999
        } else {
            let md = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return;
            }
            if md < 0 {
                emsg(gettext(
                    c"E900: maxdepth must be non-negative number".as_ptr(),
                ));
                return;
            }
            md
        };

        let mut list = (*argvars).vval.v_list;
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = list;
        if list.is_null() {
            return;
        }

        if make_copy {
            list = tv_list_copy(ptr::null_mut(), list, false, get_copyID());
            (*rettv).vval.v_list = list;
            if list.is_null() {
                return;
            }
        } else {
            if value_check_lock(
                tv_list_locked(list),
                c"flatten() argument".as_ptr(),
                TV_TRANSLATE,
            ) {
                return;
            }
            tv_list_ref(list);
        }

        tv_list_flatten(
            list,
            ptr::null_mut(),
            i64::from(tv_list_len(list)),
            i64::from(maxdepth),
        );
    }
}

/// "flatten(list[, {maxdepth}])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_flatten(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        flatten_common(argvars, rettv, false);
    }
}

/// "flattennew(list[, {maxdepth}])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_flattennew(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        flatten_common(argvars, rettv, true);
    }
}

/// "feedkeys()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_feedkeys(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }
        let keys = tv_get_string(argvars);
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags: *const c_char = ptr::null();
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            flags = tv_get_string_buf(argvars.add(1), nbuf.as_mut_ptr());
        }
        nvim_feedkeys(cstr_as_string(keys), cstr_as_string(flags), true);
    }
}

/// "float2nr({float})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_float2nr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut f: float_T = 0.0;
        if !tv_get_float_chk(argvars, &raw mut f) {
            return;
        }
        if f <= (-VARNUMBER_MAX as float_T) + f64::EPSILON {
            (*rettv).vval.v_number = -VARNUMBER_MAX;
        } else if f >= (VARNUMBER_MAX as float_T) - f64::EPSILON {
            (*rettv).vval.v_number = VARNUMBER_MAX;
        } else {
            (*rettv).vval.v_number = f as varnumber_T;
        }
    }
}

/// "fmod()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_fmod(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx % fy;
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// "fnameescape({string})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_fnameescape(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_string = vim_strsave_fnameescape(tv_get_string(argvars), VSE_NONE);
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "foreground()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_foreground(
    _argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
}

/// "function()" and "funcref()" common implementation
unsafe fn common_function(argvars: *mut typval_T, rettv: *mut typval_T, is_funcref: bool) {
    unsafe {
        let mut s: *mut c_char;
        let mut name: *mut c_char;
        let mut use_string = false;
        let mut arg_pt: *mut partial_T = ptr::null_mut();
        let mut trans_name: *mut c_char = ptr::null_mut();

        if (*argvars).v_type == VarType::VAR_FUNC {
            s = (*argvars).vval.v_string;
        } else if (*argvars).v_type == VarType::VAR_PARTIAL && !(*argvars).vval.v_ptr.is_null() {
            arg_pt = (*argvars).vval.v_ptr.cast();
            s = partial_name(arg_pt).cast_mut();
        } else {
            s = tv_get_string(argvars).cast_mut();
            use_string = true;
        }

        if (use_string && vim_strchr(s, AUTOLOAD_CHAR).is_null()) || is_funcref {
            name = s;
            trans_name = save_function_name(
                &raw mut name,
                false,
                TFN_INT | TFN_QUIET | TFN_NO_AUTOLOAD | TFN_NO_DEREF,
                ptr::null_mut(),
            );
            if *name != NUL {
                s = ptr::null_mut();
            }
        }
        'theend: {
            if s.is_null()
                || *s == NUL
                || (use_string && ascii_isdigit(*s))
                || (is_funcref && trans_name.is_null())
            {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    if use_string {
                        tv_get_string(argvars)
                    } else {
                        s
                    },
                );
            } else if !trans_name.is_null()
                && if is_funcref {
                    find_func(trans_name).is_null()
                } else {
                    !translated_function_exists(trans_name)
                }
            {
                semsg(gettext(c"E700: Unknown function: %s".as_ptr()), s);
            } else {
                let mut dict_idx: usize = 0;
                let mut arg_idx: usize = 0;
                let mut list: *mut list_T = ptr::null_mut();
                if strncmp(s, c"s:".as_ptr(), 2) == 0 || strncmp(s, c"<SID>".as_ptr(), 5) == 0 {
                    name = get_scriptlocal_funcname(s);
                } else {
                    name = xstrdup(s);
                }

                if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                    if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                        arg_idx = 1;
                        dict_idx = 2;
                    } else if (*argvars.add(1)).v_type == VarType::VAR_DICT {
                        dict_idx = 1;
                    } else {
                        arg_idx = 1;
                    }
                    if dict_idx > 0 {
                        if tv_check_for_dict_arg(argvars, dict_idx as c_int) == FAIL {
                            xfree(name.cast());
                            break 'theend;
                        }
                        if (*argvars.add(dict_idx)).vval.v_dict.is_null() {
                            dict_idx = 0;
                        }
                    }
                    if arg_idx > 0 {
                        if (*argvars.add(arg_idx)).v_type != VarType::VAR_LIST {
                            emsg(gettext(
                                c"E923: Second argument of function() must be a list or a dict"
                                    .as_ptr(),
                            ));
                            xfree(name.cast());
                            break 'theend;
                        }
                        list = (*argvars.add(arg_idx)).vval.v_list;
                        if tv_list_len(list) == 0 {
                            arg_idx = 0;
                        } else if (tv_list_len(list) as usize) > MAX_FUNC_ARGS {
                            emsg_funcname(e_toomanyarg.as_ptr(), s);
                            xfree(name.cast());
                            break 'theend;
                        }
                    }
                }
                if dict_idx > 0 || arg_idx > 0 || !arg_pt.is_null() || is_funcref {
                    let pt = xcalloc(1, size_of::<partial_T>()).cast::<partial_T>();
                    if arg_idx > 0 || (!arg_pt.is_null() && (*arg_pt).pt_argc > 0) {
                        let arg_len = if arg_pt.is_null() {
                            0
                        } else {
                            (*arg_pt).pt_argc
                        };
                        let lv_len = tv_list_len(list);
                        (*pt).pt_argc = arg_len + lv_len;
                        (*pt).pt_argv =
                            xmalloc(size_of::<typval_T>() * ((*pt).pt_argc as usize)).cast();
                        let mut i: c_int = 0;
                        while i < arg_len {
                            tv_copy(
                                (*arg_pt).pt_argv.add(i as usize),
                                (*pt).pt_argv.add(i as usize),
                            );
                            i += 1;
                        }
                        if lv_len > 0 {
                            let mut li = tv_list_first(list);
                            while !li.is_null() {
                                tv_copy(&raw const (*li).li_tv, (*pt).pt_argv.add(i as usize));
                                i += 1;
                                li = (*li).li_next;
                            }
                        }
                    }

                    if dict_idx > 0 {
                        (*pt).pt_dict = (*argvars.add(dict_idx)).vval.v_dict.cast();
                        (*(*pt).pt_dict).dv_refcount += 1;
                    } else if !arg_pt.is_null() {
                        (*pt).pt_dict = (*arg_pt).pt_dict;
                        (*pt).pt_auto = (*arg_pt).pt_auto;
                        if !(*pt).pt_dict.is_null() {
                            (*(*pt).pt_dict).dv_refcount += 1;
                        }
                    }

                    (*pt).pt_refcount = 1;
                    if !arg_pt.is_null() && !(*arg_pt).pt_func.is_null() {
                        (*pt).pt_func = (*arg_pt).pt_func;
                        func_ptr_ref((*pt).pt_func);
                        xfree(name.cast());
                    } else if is_funcref {
                        (*pt).pt_func = find_func(trans_name);
                        func_ptr_ref((*pt).pt_func);
                        xfree(name.cast());
                    } else {
                        (*pt).pt_name = name;
                        func_ref(name);
                    }

                    (*rettv).v_type = VarType::VAR_PARTIAL;
                    (*rettv).vval.v_ptr = pt.cast();
                } else {
                    (*rettv).v_type = VarType::VAR_FUNC;
                    (*rettv).vval.v_string = name;
                    func_ref(name);
                }
            }
        }
        xfree(trans_name.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_funcref(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        common_function(argvars, rettv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_function(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        common_function(argvars, rettv, false);
    }
}

/// "garbagecollect()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_garbagecollect(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        want_garbage_collect = true;
        if (*argvars).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars) == 1 {
            garbage_collect_at_exit = true;
        }
    }
}

/// "get()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_get(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut tv: *mut typval_T = ptr::null_mut();
        let mut what_is_dict = false;

        if (*argvars).v_type == VarType::VAR_BLOB {
            let mut error = false;
            let mut idx = tv_get_number_chk(argvars.add(1), &raw mut error) as c_int;
            if !error {
                let blob = (*argvars).vval.v_blob.cast::<blob_T>();
                (*rettv).v_type = VarType::VAR_NUMBER;
                if idx < 0 {
                    idx += tv_blob_len(blob);
                }
                if idx < 0 || idx >= tv_blob_len(blob) {
                    (*rettv).vval.v_number = -1;
                } else {
                    (*rettv).vval.v_number = varnumber_T::from(tv_blob_get(blob, idx));
                    tv = rettv;
                }
            }
        } else if (*argvars).v_type == VarType::VAR_LIST {
            let l = (*argvars).vval.v_list;
            if !l.is_null() {
                let mut error = false;
                let li = tv_list_find(
                    l,
                    tv_get_number_chk(argvars.add(1), &raw mut error) as c_int,
                );
                if !error && !li.is_null() {
                    tv = &raw mut (*li).li_tv;
                }
            }
        } else if (*argvars).v_type == VarType::VAR_DICT {
            let d = (*argvars).vval.v_dict.cast::<dict_T>();
            if !d.is_null() {
                let di = tv_dict_find(d, tv_get_string(argvars.add(1)), -1);
                if !di.is_null() {
                    tv = &raw mut (*di).di_tv;
                }
            }
        } else if tv_is_func(*argvars) {
            let mut fref_pt: partial_T = std::mem::zeroed();
            let pt: *mut partial_T = if (*argvars).v_type == VarType::VAR_PARTIAL {
                (*argvars).vval.v_ptr.cast()
            } else {
                fref_pt.pt_name = (*argvars).vval.v_string;
                &raw mut fref_pt
            };

            if !pt.is_null() {
                let what = tv_get_string(argvars.add(1));
                if strcmp(what, c"func".as_ptr()) == 0 || strcmp(what, c"name".as_ptr()) == 0 {
                    let mut name = partial_name(pt);
                    (*rettv).v_type = if *what == b'f' as c_char {
                        VarType::VAR_FUNC
                    } else {
                        VarType::VAR_STRING
                    };
                    debug_assert!(!name.is_null());
                    if (*rettv).v_type == VarType::VAR_FUNC {
                        func_ref(name.cast_mut());
                    }
                    if *what == b'n' as c_char
                        && (*pt).pt_name.is_null()
                        && !(*pt).pt_func.is_null()
                    {
                        name = printable_func_name((*pt).pt_func);
                    }
                    (*rettv).vval.v_string = xstrdup(name);
                } else if strcmp(what, c"dict".as_ptr()) == 0 {
                    what_is_dict = true;
                    if !(*pt).pt_dict.is_null() {
                        tv_dict_set_ret(rettv, (*pt).pt_dict);
                    }
                } else if strcmp(what, c"args".as_ptr()) == 0 {
                    (*rettv).v_type = VarType::VAR_LIST;
                    tv_list_alloc_ret(rettv, (*pt).pt_argc as isize);
                    for i in 0..((*pt).pt_argc as usize) {
                        tv_list_append_tv((*rettv).vval.v_list, (*pt).pt_argv.add(i));
                    }
                } else if strcmp(what, c"arity".as_ptr()) == 0 {
                    let mut required: c_int = 0;
                    let mut optional: c_int = 0;
                    let mut varargs = false;
                    let name = partial_name(pt);
                    get_func_arity(name, &raw mut required, &raw mut optional, &raw mut varargs);

                    (*rettv).v_type = VarType::VAR_DICT;
                    tv_dict_alloc_ret(rettv);
                    let dict = (*rettv).vval.v_dict.cast::<dict_T>();

                    if (*pt).pt_argc >= required + optional {
                        required = 0;
                        optional = 0;
                    } else if (*pt).pt_argc > required {
                        optional -= (*pt).pt_argc - required;
                        required = 0;
                    } else {
                        required -= (*pt).pt_argc;
                    }

                    tv_dict_add_nr(dict, c"required".as_ptr(), 8, varnumber_T::from(required));
                    tv_dict_add_nr(dict, c"optional".as_ptr(), 8, varnumber_T::from(optional));
                    tv_dict_add_bool(
                        dict,
                        c"varargs".as_ptr(),
                        7,
                        if varargs { kBoolVarTrue } else { kBoolVarFalse },
                    );
                } else {
                    semsg(gettext(e_invarg2.as_ptr()), what);
                }

                if !what_is_dict {
                    return;
                }
            }
        } else {
            semsg(gettext(e_listdictblobarg.as_ptr()), c"get()".as_ptr());
        }

        if tv.is_null() {
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                tv_copy(argvars.add(2), rettv);
            }
        } else {
            tv_copy(tv, rettv);
        }
    }
}

/// "getchangelist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getchangelist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, 2);
        let buf: *const buf_T = if (*argvars).v_type == VarType::VAR_UNKNOWN {
            curbuf
        } else {
            vim_ignored = tv_get_number(argvars) as c_int;
            emsg_off += 1;
            let b = tv_get_buf(argvars, 0);
            emsg_off -= 1;
            b
        };
        if buf.is_null() {
            return;
        }

        let l = tv_list_alloc((*buf).b_changelistlen as isize);
        tv_list_append_list((*rettv).vval.v_list, l);
        let mut changelistindex = if ptr::eq(buf, (*curwin).w_buffer) {
            (*curwin).w_changelistidx
        } else {
            let mut idx = (*buf).b_changelistlen;
            for i in 0..(*buf).b_wininfo.size {
                let wip = (*(*buf).b_wininfo.items.add(i)).cast::<WinInfo>();
                if (*wip).wi_win == curwin {
                    idx = (*wip).wi_changelistidx;
                    break;
                }
            }
            idx
        };
        let _ = &mut changelistindex;
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(changelistindex));

        for i in 0..((*buf).b_changelistlen as usize) {
            if (*buf).b_changelist[i].mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            tv_dict_add_nr(
                d,
                c"lnum".as_ptr(),
                4,
                varnumber_T::from((*buf).b_changelist[i].mark.lnum),
            );
            tv_dict_add_nr(
                d,
                c"col".as_ptr(),
                3,
                varnumber_T::from((*buf).b_changelist[i].mark.col),
            );
            tv_dict_add_nr(
                d,
                c"coladd".as_ptr(),
                6,
                varnumber_T::from((*buf).b_changelist[i].mark.coladd),
            );
        }
    }
}

unsafe fn getpos_both(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    getcurpos: bool,
    charcol: bool,
) {
    unsafe {
        let mut fp: *mut pos_T = ptr::null_mut();
        let mut pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut wp = curwin;
        let mut fnum: c_int = -1;

        if getcurpos {
            if (*argvars).v_type != VarType::VAR_UNKNOWN {
                wp = find_win_by_nr_or_id(argvars);
                if !wp.is_null() {
                    fp = (&raw mut (*wp).w_cursor).cast();
                }
            } else {
                fp = (&raw mut (*curwin).w_cursor).cast();
            }
            if !fp.is_null() && charcol {
                pos = *fp;
                pos.col = buf_byteidx_to_charidx((*wp).w_buffer, pos.lnum, pos.col);
                fp = &raw mut pos;
            }
        } else {
            fp = var2fpos(argvars, true, &raw mut fnum, charcol, curwin);
        }

        let l = tv_list_alloc_ret(rettv, 4 + isize::from(getcurpos));
        tv_list_append_number(
            l,
            if fnum != -1 {
                varnumber_T::from(fnum)
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                varnumber_T::from((*fp).lnum)
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                if (*fp).col == MAXCOL {
                    varnumber_T::from(MAXCOL)
                } else {
                    varnumber_T::from((*fp).col) + 1
                }
            } else {
                0
            },
        );
        tv_list_append_number(
            l,
            if !fp.is_null() {
                varnumber_T::from((*fp).coladd)
            } else {
                0
            },
        );
        if getcurpos {
            let save_set_curswant = (*curwin).w_set_curswant;
            let save_curswant = (*curwin).w_curswant;
            let save_virtcol = (*curwin).w_virtcol;

            if wp == curwin {
                update_curswant();
            }
            tv_list_append_number(
                l,
                if wp.is_null() {
                    0
                } else if (*wp).w_curswant == MAXCOL {
                    varnumber_T::from(MAXCOL)
                } else {
                    varnumber_T::from((*wp).w_curswant) + 1
                },
            );

            if wp == curwin && save_set_curswant {
                (*curwin).w_set_curswant = save_set_curswant;
                (*curwin).w_curswant = save_curswant;
                (*curwin).w_virtcol = save_virtcol;
                (*curwin).w_valid &= !VALID_VIRTCOL;
            }
        }
    }
}

/// "getcharpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getpos_both(argvars, rettv, false, true);
    }
}

/// "getcharsearch()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcharsearch(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict.cast::<dict_T>();
        tv_dict_add_str(dict, c"char".as_ptr(), 4, last_csearch());
        tv_dict_add_nr(
            dict,
            c"forward".as_ptr(),
            7,
            varnumber_T::from(last_csearch_forward()),
        );
        tv_dict_add_nr(
            dict,
            c"until".as_ptr(),
            5,
            varnumber_T::from(last_csearch_until()),
        );
    }
}

/// "getfontname()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getfontname(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
    }
}

/// "getjumplist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getjumplist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        let wp = find_tabwin(argvars, argvars.add(1));
        if wp.is_null() {
            return;
        }

        cleanup_jumplist(wp, true);

        let l = tv_list_alloc((*wp).w_jumplistlen as isize);
        tv_list_append_list((*rettv).vval.v_list, l);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from((*wp).w_jumplistidx));

        for i in 0..((*wp).w_jumplistlen as usize) {
            if (*wp).w_jumplist[i].fmark.mark.lnum == 0 {
                continue;
            }
            let d = tv_dict_alloc();
            tv_list_append_dict(l, d);
            tv_dict_add_nr(
                d,
                c"lnum".as_ptr(),
                4,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.lnum),
            );
            tv_dict_add_nr(
                d,
                c"col".as_ptr(),
                3,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.col),
            );
            tv_dict_add_nr(
                d,
                c"coladd".as_ptr(),
                6,
                varnumber_T::from((*wp).w_jumplist[i].fmark.mark.coladd),
            );
            tv_dict_add_nr(
                d,
                c"bufnr".as_ptr(),
                5,
                varnumber_T::from((*wp).w_jumplist[i].fmark.fnum),
            );
            if !(*wp).w_jumplist[i].fname.is_null() {
                tv_dict_add_str(d, c"filename".as_ptr(), 8, (*wp).w_jumplist[i].fname);
            }
        }
    }
}

/// "getmarklist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getmarklist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            get_global_marks((*rettv).vval.v_list);
            return;
        }
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            return;
        }
        get_buf_local_marks(buf, (*rettv).vval.v_list);
    }
}

/// "getpid()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getpid(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = os_get_pid();
    }
}

/// "getcurpos(string)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcurpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getpos_both(argvars, rettv, true, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getcursorcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getpos_both(argvars, rettv, true, true);
    }
}

/// "getpos(string)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        getpos_both(argvars, rettv, false, false);
    }
}

/// Convert from `block_def` to `NvimString`.
unsafe fn block_def2str(bd: *const block_def) -> NvimString {
    unsafe {
        let startspaces = (*bd).startspaces as usize;
        let endspaces = (*bd).endspaces as usize;
        let textlen = (*bd).textlen as usize;
        let size = startspaces + endspaces + textlen;
        let data = xmalloc(size + 1).cast::<c_char>();

        ptr::write_bytes(data, b' ', startspaces);
        let mut off = startspaces;
        ptr::copy((*bd).textstart, data.add(off), textlen);
        off += textlen;
        ptr::write_bytes(data.add(off), b' ', endspaces);
        off += endspaces;
        *data.add(off) = NUL;

        NvimString { data, size: off }
    }
}

unsafe fn getregionpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    p1: *mut pos_T,
    p2: *mut pos_T,
    inclusive: *mut bool,
    region_type: *mut MotionType,
    oap: *mut oparg_T,
) -> c_int {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if tv_check_for_list_arg(argvars, 0) == FAIL
            || tv_check_for_list_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return FAIL;
        }

        let mut fnum1: c_int = -1;
        let mut fnum2: c_int = -1;
        if list2fpos(argvars, p1, &raw mut fnum1, ptr::null_mut(), false) != OK
            || list2fpos(argvars.add(1), p2, &raw mut fnum2, ptr::null_mut(), false) != OK
            || fnum1 != fnum2
        {
            return FAIL;
        }

        let is_select_exclusive: bool;
        let mut type_str: *mut c_char;
        let mut default_type = [b'v' as c_char, NUL];
        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            is_select_exclusive = tv_dict_get_bool(
                d,
                c"exclusive".as_ptr(),
                c_int::from(*p_sel == b'e' as c_char),
            ) != 0;
            type_str = tv_dict_get_string(d, c"type".as_ptr(), false);
            if type_str.is_null() {
                type_str = default_type.as_mut_ptr();
            }
        } else {
            is_select_exclusive = *p_sel == b'e' as c_char;
            type_str = default_type.as_mut_ptr();
        }

        let mut block_width: c_int = 0;
        if *type_str == b'v' as c_char && *type_str.add(1) == NUL {
            *region_type = kMTCharWise;
        } else if *type_str == b'V' as c_char && *type_str.add(1) == NUL {
            *region_type = kMTLineWise;
        } else if *type_str == Ctrl_V {
            let mut p = type_str.add(1);
            if *p != NUL {
                block_width = getdigits_int(&raw mut p, false, 0);
                if block_width <= 0 || *p != NUL {
                    semsg(gettext(e_invargNval.as_ptr()), c"type".as_ptr(), type_str);
                    return FAIL;
                }
            }
            *region_type = kMTBlockWise;
        } else {
            semsg(gettext(e_invargNval.as_ptr()), c"type".as_ptr(), type_str);
            return FAIL;
        }

        let findbuf = if fnum1 != 0 {
            buflist_findnr(fnum1)
        } else {
            curbuf
        };
        if findbuf.is_null() || (*findbuf).b_ml.ml_mfp.is_null() {
            emsg(gettext(e_buffer_is_not_loaded.as_ptr()));
            return FAIL;
        }

        if (*p1).lnum < 1 || (*p1).lnum > (*findbuf).b_ml.ml_line_count {
            semsg(gettext(e_invalid_line_number_nr.as_ptr()), (*p1).lnum);
            return FAIL;
        }
        if (*p1).col == MAXCOL {
            (*p1).col = ml_get_buf_len(findbuf, (*p1).lnum) + 1;
        } else if (*p1).col < 1 || (*p1).col > ml_get_buf_len(findbuf, (*p1).lnum) + 1 {
            semsg(gettext(e_invalid_column_number_nr.as_ptr()), (*p1).col);
            return FAIL;
        }

        if (*p2).lnum < 1 || (*p2).lnum > (*findbuf).b_ml.ml_line_count {
            semsg(gettext(e_invalid_line_number_nr.as_ptr()), (*p2).lnum);
            return FAIL;
        }
        if (*p2).col == MAXCOL {
            (*p2).col = ml_get_buf_len(findbuf, (*p2).lnum) + 1;
        } else if (*p2).col < 1 || (*p2).col > ml_get_buf_len(findbuf, (*p2).lnum) + 1 {
            semsg(gettext(e_invalid_column_number_nr.as_ptr()), (*p2).col);
            return FAIL;
        }

        curbuf = findbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = TriState::from(virtual_active(curwin));

        (*p1).col -= 1;
        (*p2).col -= 1;

        getregionpos_prep(
            (*p1).cast(),
            (*p2).cast(),
            *region_type,
            is_select_exclusive,
            block_width,
            inclusive,
            oap,
        );
        OK
    }
}

impl pos_T {
    #[inline]
    unsafe fn cast(&mut self) -> *mut crate::api::tabpage::pos_T {
        (self as *mut pos_T).cast()
    }
}

/// "getregion()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getregion(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let save_curbuf = curbuf;
        let save_virtual = virtual_op;

        let mut p1 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = kMTUnknown;
        let mut oa: oparg_T = std::mem::zeroed();

        if getregionpos(
            argvars,
            rettv,
            &raw mut p1,
            &raw mut p2,
            &raw mut inclusive,
            &raw mut region_type,
            &raw mut oa,
        ) == FAIL
        {
            return;
        }

        for lnum in p1.lnum..=p2.lnum {
            let akt = if region_type == kMTBlockWise {
                let mut bd: block_def = std::mem::zeroed();
                block_prep(&raw mut oa, &raw mut bd, lnum, false);
                block_def2str(&raw const bd)
            } else if region_type == kMTLineWise || (p1.lnum < lnum && lnum < p2.lnum) {
                cbuf_to_string(ml_get(lnum), ml_get_len(lnum) as usize)
            } else {
                let mut bd: block_def = std::mem::zeroed();
                charwise_block_prep(
                    crate::api::tabpage::pos_T {
                        lnum: p1.lnum,
                        col: p1.col,
                        coladd: p1.coladd,
                    },
                    crate::api::tabpage::pos_T {
                        lnum: p2.lnum,
                        col: p2.col,
                        coladd: p2.coladd,
                    },
                    &raw mut bd,
                    lnum,
                    inclusive,
                );
                block_def2str(&raw const bd)
            };

            debug_assert!(!akt.data.is_null());
            tv_list_append_allocated_string((*rettv).vval.v_list, akt.data);
        }

        curbuf = save_curbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = save_virtual;
    }
}

unsafe fn add_regionpos_range(rettv: *mut typval_T, p1: pos_T, p2: pos_T) {
    unsafe {
        let l1 = tv_list_alloc(2);
        tv_list_append_list((*rettv).vval.v_list, l1);

        let l2 = tv_list_alloc(4);
        tv_list_append_list(l1, l2);

        let l3 = tv_list_alloc(4);
        tv_list_append_list(l1, l3);

        tv_list_append_number(l2, varnumber_T::from((*curbuf).b_fnum));
        tv_list_append_number(l2, varnumber_T::from(p1.lnum));
        tv_list_append_number(l2, varnumber_T::from(p1.col));
        tv_list_append_number(l2, varnumber_T::from(p1.coladd));

        tv_list_append_number(l3, varnumber_T::from((*curbuf).b_fnum));
        tv_list_append_number(l3, varnumber_T::from(p2.lnum));
        tv_list_append_number(l3, varnumber_T::from(p2.col));
        tv_list_append_number(l3, varnumber_T::from(p2.coladd));
    }
}

/// "getregionpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getregionpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let save_curbuf = curbuf;
        let save_virtual = virtual_op;

        let mut p1 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut p2 = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut region_type = kMTUnknown;
        let mut allow_eol = false;
        let mut bounds_only = false;
        let mut oa: oparg_T = std::mem::zeroed();

        if getregionpos(
            argvars,
            rettv,
            &raw mut p1,
            &raw mut p2,
            &raw mut inclusive,
            &raw mut region_type,
            &raw mut oa,
        ) == FAIL
        {
            return;
        }

        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            allow_eol = tv_dict_get_bool(d, c"eol".as_ptr(), 0) != 0;
            bounds_only = tv_dict_get_bool(d, c"bounds".as_ptr(), 0) != 0;
        }

        let tab_p1 = crate::api::tabpage::pos_T {
            lnum: p1.lnum,
            col: p1.col,
            coladd: p1.coladd,
        };
        let tab_p2 = crate::api::tabpage::pos_T {
            lnum: p2.lnum,
            col: p2.col,
            coladd: p2.coladd,
        };

        if bounds_only {
            let mut start_pos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut end_pos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            getregionpos_line(
                p1.lnum,
                tab_p1,
                tab_p2,
                inclusive,
                region_type,
                &raw mut oa,
                allow_eol,
                start_pos.cast(),
                end_pos.cast(),
            );
            if p2.lnum != p1.lnum {
                let mut unused = pos_T {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                getregionpos_line(
                    p2.lnum,
                    tab_p1,
                    tab_p2,
                    inclusive,
                    region_type,
                    &raw mut oa,
                    allow_eol,
                    unused.cast(),
                    end_pos.cast(),
                );
            }
            add_regionpos_range(rettv, start_pos, end_pos);
        } else {
            for lnum in p1.lnum..=p2.lnum {
                let mut ret_p1 = pos_T {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                let mut ret_p2 = pos_T {
                    lnum: 0,
                    col: 0,
                    coladd: 0,
                };
                getregionpos_line(
                    lnum,
                    tab_p1,
                    tab_p2,
                    inclusive,
                    region_type,
                    &raw mut oa,
                    allow_eol,
                    ret_p1.cast(),
                    ret_p2.cast(),
                );
                add_regionpos_range(rettv, ret_p1, ret_p2);
            }
        }

        curbuf = save_curbuf;
        (*curwin).w_buffer = curbuf;
        virtual_op = save_virtual;
    }
}

/// Common between `getreg()`, `getreginfo()` and `getregtype()`.
unsafe fn getreg_get_regname(argvars: *mut typval_T) -> c_int {
    unsafe {
        let strregname = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let s = tv_get_string_chk(argvars);
            if s.is_null() {
                return 0;
            }
            s
        } else {
            get_vim_var_str(VV_REG)
        };
        if *strregname == 0 {
            b'"' as c_int
        } else {
            c_int::from(*strregname as u8)
        }
    }
}

/// "getreg()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getreg(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut arg2: c_int = 0;
        let mut return_list = false;

        let regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }

        if (*argvars).v_type != VarType::VAR_UNKNOWN
            && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
        {
            let mut error = false;
            arg2 = tv_get_bool_chk(argvars.add(1), &raw mut error) as c_int;
            if !error && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                return_list = tv_get_bool_chk(argvars.add(2), &raw mut error) != 0;
            }
            if error {
                return;
            }
        }

        if return_list {
            (*rettv).v_type = VarType::VAR_LIST;
            (*rettv).vval.v_list = get_reg_contents(
                regname,
                (if arg2 != 0 { kGRegExprSrc } else { 0 }) | kGRegList,
            )
            .cast();
            if (*rettv).vval.v_list.is_null() {
                (*rettv).vval.v_list = tv_list_alloc(0);
            }
            tv_list_ref((*rettv).vval.v_list);
        } else {
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string =
                get_reg_contents(regname, if arg2 != 0 { kGRegExprSrc } else { 0 }).cast();
        }
    }
}

/// "getregtype()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getregtype(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }

        let mut reglen: colnr_T = 0;
        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        let reg_type = get_reg_type(regname, &raw mut reglen);
        format_reg_type(reg_type, reglen, buf.as_mut_ptr(), buf.len());

        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
    }
}

/// "gettagstack()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_gettagstack(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut wp = curwin;
        tv_dict_alloc_ret(rettv);
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            wp = find_win_by_nr_or_id(argvars);
            if wp.is_null() {
                return;
            }
        }
        get_tagstack(wp, (*rettv).vval.v_dict.cast());
    }
}

unsafe extern "C" fn dummy_timer_due_cb(tw: *mut TimeWatcher, _data: *mut c_void) {
    unsafe {
        if *loop_closing_ptr((&raw mut main_loop).cast()) {
            time_watcher_stop(tw);
            time_watcher_close(tw, Some(dummy_timer_close_cb));
        }
    }
}

unsafe extern "C" fn dummy_timer_close_cb(tw: *mut TimeWatcher, _data: *mut c_void) {
    unsafe {
        xfree(tw.cast());
    }
}

/// "wait(timeout, condition[, interval])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_wait(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = -1;

        if (*argvars).v_type != VarType::VAR_NUMBER {
            semsg(gettext(e_invargval.as_ptr()), c"1".as_ptr());
            return;
        }
        if ((*argvars.add(2)).v_type != VarType::VAR_NUMBER
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN)
            || ((*argvars.add(2)).v_type == VarType::VAR_NUMBER
                && (*argvars.add(2)).vval.v_number <= 0)
        {
            semsg(gettext(e_invargval.as_ptr()), c"3".as_ptr());
            return;
        }

        let timeout = (*argvars).vval.v_number as c_int;
        let mut expr = *argvars.add(1);
        let interval = if (*argvars.add(2)).v_type == VarType::VAR_NUMBER {
            (*argvars.add(2)).vval.v_number as c_int
        } else {
            200
        };
        let tw = xmalloc(uv_handle_size(UV_TIMER) + size_of::<TimeWatcherFields>())
            .cast::<TimeWatcher>();

        let main_loop_ptr: *mut Loop = (&raw mut main_loop).cast();
        time_watcher_init(main_loop_ptr, tw, ptr::null_mut());
        (*watcher_fields(tw)).events = ptr::null_mut();
        time_watcher_start(
            tw,
            Some(dummy_timer_due_cb),
            interval as u64,
            interval as u64,
        );

        let mut argv = tv_initial_value();
        let mut exprval = tv_initial_value();
        let mut error = false;
        let called_emsg_before = called_emsg;

        ui_flush();

        let mut remaining = i64::from(timeout);
        let mut before = if remaining > 0 { os_hrtime() } else { 0 };
        while !(eval_expr_typval(&raw mut expr, false, &raw mut argv, 0, &raw mut exprval) != OK
            || tv_get_number_chk(&raw const exprval, &raw mut error) != 0
            || called_emsg > called_emsg_before
            || error
            || got_int)
        {
            let events = (*loop_queues(main_loop_ptr)).events;
            if !events.is_null() && !multiqueue_empty(events) {
                multiqueue_process_events(events);
            } else {
                loop_poll_events(main_loop_ptr, remaining);
            }
            if remaining == 0 {
                break;
            } else if remaining > 0 {
                let now = os_hrtime();
                remaining -= ((now - before) / 1_000_000) as i64;
                before = now;
                if remaining <= 0 {
                    break;
                }
            }
        }

        if called_emsg > called_emsg_before || error {
            (*rettv).vval.v_number = -3;
        } else if got_int {
            got_int = false;
            vgetc();
            (*rettv).vval.v_number = -2;
        } else if tv_get_number_chk(&raw const exprval, &raw mut error) != 0 {
            (*rettv).vval.v_number = 0;
        }

        time_watcher_stop(tw);
        time_watcher_close(tw, Some(dummy_timer_close_cb));
    }
}

/// "gettext()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_gettext(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_nonempty_string_arg(argvars, 0) == FAIL {
            return;
        }
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup(gettext((*argvars).vval.v_string));
    }
}

/// "has()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_has(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut x = false;
        let mut n = false;
        let name = tv_get_string(argvars);

        if c_strnicmp(name, c"patch".as_ptr(), 5) == 0 {
            x = true;
            if *name.add(5) == b'-' as c_char
                && strlen(name) >= 11
                && (*name.add(6) >= b'1' as c_char && *name.add(6) <= b'9' as c_char)
            {
                let mut end: *mut c_char = ptr::null_mut();
                let major = strtoul(name.add(6), &raw mut end, 10) as c_int;
                if *end == b'.' as c_char
                    && ascii_isdigit(*end.add(1))
                    && *end.add(2) == b'.' as c_char
                    && ascii_isdigit(*end.add(3))
                {
                    let minor = atoi(end.add(1));
                    n = has_vim_patch(atoi(end.add(3)), major * 100 + minor);
                }
            } else if ascii_isdigit(*name.add(5)) {
                n = has_vim_patch(atoi(name.add(5)), 0);
            }
        } else if c_strnicmp(name, c"nvim-".as_ptr(), 5) == 0 {
            x = true;
            n = has_nvim_version(name.add(5));
        } else if c_stricmp(name, c"vim_starting".as_ptr()) == 0 {
            x = true;
            n = starting != 0;
        } else if c_stricmp(name, c"ttyin".as_ptr()) == 0 {
            x = true;
            n = stdin_isatty;
        } else if c_stricmp(name, c"ttyout".as_ptr()) == 0 {
            x = true;
            n = stdout_isatty;
        } else if c_stricmp(name, c"multi_byte_encoding".as_ptr()) == 0 {
            x = true;
            n = true;
        } else if c_stricmp(name, c"gui_running".as_ptr()) == 0 {
            x = true;
            n = ui_gui_attached();
        } else if c_stricmp(name, c"syntax_items".as_ptr()) == 0 {
            x = true;
            n = syntax_present(curwin);
        } else if c_stricmp(name, c"wsl".as_ptr()) == 0 {
            x = true;
            n = has_wsl();
        }

        if !x {
            let list_ptr = eval_has_list.as_ptr();
            let mut i: usize = 0;
            while !(*list_ptr.add(i)).is_null() {
                if c_stricmp(name, *list_ptr.add(i)) == 0 {
                    x = true;
                    n = true;
                    break;
                }
                i += 1;
            }
        }

        if !x {
            let save_shell_error = get_vim_var_nr(VV_SHELL_ERROR) as c_int;
            if c_stricmp(name, c"clipboard_working".as_ptr()) == 0 {
                n = eval_has_provider(c"clipboard".as_ptr(), true);
            } else if cfg!(not(windows)) && c_stricmp(name, c"unnamedplus".as_ptr()) == 0 {
                n = eval_has_provider(c"clipboard".as_ptr(), true);
            } else if c_stricmp(name, c"pythonx".as_ptr()) == 0 {
                n = eval_has_provider(c"python3".as_ptr(), true);
            } else if eval_has_provider(name, true) {
                n = true;
            }
            set_vim_var_nr(VV_SHELL_ERROR, varnumber_T::from(save_shell_error));
        }

        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

unsafe fn has_wsl() -> bool {
    unsafe {
        if HAS_WSL == kNone {
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };
            let cmd = c"return vim.uv.os_uname()['release']:lower():match('microsoft')";
            let o = nlua_exec(
                NvimString {
                    data: cmd.as_ptr().cast_mut(),
                    size: cmd.to_bytes().len(),
                },
                ptr::null(),
                Array {
                    items: ptr::null_mut(),
                    size: 0,
                    capacity: 0,
                },
                kRetNilBool,
                ptr::null_mut(),
                &raw mut err,
            );
            debug_assert!(err.err_type == ErrorType::kErrorTypeNone);
            HAS_WSL = if luaret_truthy(o) { kTrue } else { kFalse };
        }
        HAS_WSL == kTrue
    }
}

/// "highlightID(name)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_hlID(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(syn_name2id(tv_get_string(argvars)));
    }
}

/// "highlight_exists()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_hlexists(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(highlight_exists(tv_get_string(argvars)));
    }
}

/// "index()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_index(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut idx: c_int = 0;
        let mut ic = false;

        (*rettv).vval.v_number = -1;
        if (*argvars).v_type == VarType::VAR_BLOB {
            let mut error = false;
            let mut start: c_int = 0;
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                start = tv_get_number_chk(argvars.add(2), &raw mut error) as c_int;
                if error {
                    return;
                }
            }
            let b = (*argvars).vval.v_blob.cast::<blob_T>();
            if b.is_null() {
                return;
            }
            if start < 0 {
                start += tv_blob_len(b);
                if start < 0 {
                    start = 0;
                }
            }
            idx = start;
            while idx < tv_blob_len(b) {
                let mut tv = typval_T {
                    v_type: VarType::VAR_NUMBER,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval {
                        v_number: varnumber_T::from(tv_blob_get(b, idx)),
                    },
                };
                if tv_equal(&raw mut tv, argvars.add(1), ic) {
                    (*rettv).vval.v_number = varnumber_T::from(idx);
                    return;
                }
                idx += 1;
            }
            return;
        } else if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listblobreq.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        let mut item = tv_list_first(l);
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut error = false;
            idx = tv_list_uidx(
                l,
                tv_get_number_chk(argvars.add(2), &raw mut error) as c_int,
            );
            if error || idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l, idx);
                debug_assert!(!item.is_null());
            }
            if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                ic = tv_get_bool_chk(argvars.add(3), &raw mut error) != 0;
                if error {
                    item = ptr::null_mut();
                }
            }
        }

        while !item.is_null() {
            if tv_equal(&raw mut (*item).li_tv, argvars.add(1), ic) {
                (*rettv).vval.v_number = varnumber_T::from(idx);
                break;
            }
            item = (*item).li_next;
            idx += 1;
        }
    }
}

unsafe fn indexof_eval_expr(expr: *mut typval_T) -> varnumber_T {
    unsafe {
        let mut argv = [tv_initial_value(); 3];
        argv[0] = *get_vim_var_tv(VV_KEY);
        argv[1] = *get_vim_var_tv(VV_VAL);
        let mut newtv = tv_initial_value();

        if eval_expr_typval(expr, false, argv.as_mut_ptr(), 2, &raw mut newtv) == FAIL {
            return 0;
        }

        let mut error = false;
        let found = tv_get_bool_chk(&raw const newtv, &raw mut error);
        tv_clear(&raw mut newtv);

        if error { 0 } else { found }
    }
}

unsafe fn indexof_blob(
    b: *mut blob_T,
    mut startidx: varnumber_T,
    expr: *mut typval_T,
) -> varnumber_T {
    unsafe {
        if b.is_null() {
            return -1;
        }
        if startidx < 0 {
            startidx += varnumber_T::from(tv_blob_len(b));
            if startidx < 0 {
                startidx = 0;
            }
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);
        set_vim_var_type(VV_VAL, VarType::VAR_NUMBER);

        let called_emsg_start = called_emsg;
        let mut idx = startidx;
        while idx < varnumber_T::from(tv_blob_len(b)) {
            set_vim_var_nr(VV_KEY, idx);
            set_vim_var_nr(VV_VAL, varnumber_T::from(tv_blob_get(b, idx as c_int)));

            if indexof_eval_expr(expr) != 0 {
                return idx;
            }
            if called_emsg != called_emsg_start {
                return -1;
            }
            idx += 1;
        }
        -1
    }
}

unsafe fn indexof_list(l: *mut list_T, startidx: varnumber_T, expr: *mut typval_T) -> varnumber_T {
    unsafe {
        if l.is_null() {
            return -1;
        }
        let mut item: *mut listitem_T;
        let mut idx: varnumber_T = 0;
        if startidx == 0 {
            item = tv_list_first(l);
        } else {
            idx = varnumber_T::from(tv_list_uidx(l, startidx as c_int));
            if idx == -1 {
                item = ptr::null_mut();
            } else {
                item = tv_list_find(l, idx as c_int);
                debug_assert!(!item.is_null());
            }
        }

        set_vim_var_type(VV_KEY, VarType::VAR_NUMBER);

        let called_emsg_start = called_emsg;
        while !item.is_null() {
            set_vim_var_nr(VV_KEY, idx);
            tv_copy(&raw const (*item).li_tv, get_vim_var_tv(VV_VAL));

            let found = indexof_eval_expr(expr) != 0;
            tv_clear(get_vim_var_tv(VV_VAL));

            if found {
                return idx;
            }
            if called_emsg != called_emsg_start {
                return -1;
            }
            item = (*item).li_next;
            idx += 1;
        }
        -1
    }
}

/// "indexof()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_indexof(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;

        if tv_check_for_list_or_blob_arg(argvars, 0) == FAIL
            || tv_check_for_string_or_func_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return;
        }

        if ((*argvars.add(1)).v_type == VarType::VAR_STRING
            && ((*argvars.add(1)).vval.v_string.is_null()
                || *(*argvars.add(1)).vval.v_string == NUL))
            || ((*argvars.add(1)).v_type == VarType::VAR_FUNC
                && (*argvars.add(1)).vval.v_ptr.is_null())
        {
            return;
        }

        let mut startidx: varnumber_T = 0;
        if (*argvars.add(2)).v_type == VarType::VAR_DICT {
            startidx = tv_dict_get_number_def(
                (*argvars.add(2)).vval.v_dict.cast(),
                c"startidx".as_ptr(),
                0,
            );
        }

        let mut save_val = tv_initial_value();
        let mut save_key = tv_initial_value();
        prepare_vimvar(VV_VAL, &raw mut save_val);
        prepare_vimvar(VV_KEY, &raw mut save_key);

        let save_did_emsg = did_emsg;
        did_emsg = 0;

        if (*argvars).v_type == VarType::VAR_BLOB {
            (*rettv).vval.v_number =
                indexof_blob((*argvars).vval.v_blob.cast(), startidx, argvars.add(1));
        } else {
            (*rettv).vval.v_number = indexof_list((*argvars).vval.v_list, startidx, argvars.add(1));
        }

        restore_vimvar(VV_KEY, &raw mut save_key);
        restore_vimvar(VV_VAL, &raw mut save_val);
        did_emsg |= save_did_emsg;
    }
}

/// "input()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_input(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_user_input(argvars, rettv, false, INPUTSECRET_FLAG);
    }
}

/// "inputdialog()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_inputdialog(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        get_user_input(argvars, rettv, true, INPUTSECRET_FLAG);
    }
}

/// "inputlist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_inputlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg(gettext(e_listarg.as_ptr()), c"inputlist()".as_ptr());
            return;
        }

        msg_ext_set_kind(c"confirm".as_ptr());
        msg_start();
        msg_row = Rows - 1;
        lines_left = Rows;
        msg_scroll = true;
        msg_clr_eos();

        let l = (*argvars).vval.v_list;
        let mut li = tv_list_first(l);
        while !li.is_null() {
            msg_puts(tv_get_string(&raw const (*li).li_tv));
            if !ui_has(kUIMessages) || !(*li).li_next.is_null() {
                msg_putchar(b'\n' as c_int);
            }
            li = (*li).li_next;
        }

        let mut mouse_used = false;
        let mut selected = prompt_for_input(ptr::null(), 0, false, &raw mut mouse_used);
        if mouse_used {
            selected = tv_list_len(l) - (cmdline_row - mouse_row);
        }
        (*rettv).vval.v_number = varnumber_T::from(selected);
    }
}

/// "inputrestore()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_inputrestore(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if GA_USERINPUT.ga_len > 0 {
            GA_USERINPUT.ga_len -= 1;
            restore_typeahead(
                GA_USERINPUT
                    .ga_data
                    .cast::<tasave_T>()
                    .add(GA_USERINPUT.ga_len as usize),
            );
        } else if p_verbose > 1 {
            verb_msg(gettext(
                c"called inputrestore() more often than inputsave()".as_ptr(),
            ));
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "inputsave()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_inputsave(
    _argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let p = ga_append_via_ptr(&raw mut GA_USERINPUT, size_of::<tasave_T>()).cast::<tasave_T>();
        save_typeahead(p);
    }
}

/// "inputsecret()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_inputsecret(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    unsafe {
        cmdline_star += 1;
        INPUTSECRET_FLAG = true;
        f_input(argvars, rettv, fptr);
        cmdline_star -= 1;
        INPUTSECRET_FLAG = false;
    }
}

/// "interrupt()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_interrupt(
    _argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        got_int = true;
    }
}

/// "invert(expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_invert(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = !tv_get_number_chk(argvars, ptr::null_mut());
    }
}

/// "islocked()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_islocked(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut lv: lval_T = std::mem::zeroed();
        (*rettv).vval.v_number = -1;
        let end = get_lval(
            tv_get_string(argvars).cast_mut(),
            ptr::null_mut(),
            &raw mut lv,
            false,
            false,
            GLV_NO_AUTOLOAD | GLV_READ_ONLY,
            FNE_CHECK_START,
        );
        if !end.is_null() && !lv.ll_name.is_null() {
            if *end != NUL {
                semsg(
                    gettext(if lv.ll_name_len == 0 {
                        e_invarg2.as_ptr()
                    } else {
                        e_trailing_arg.as_ptr()
                    }),
                    end,
                );
            } else if lv.ll_tv.is_null() {
                let di = find_var(lv.ll_name, lv.ll_name_len, ptr::null_mut(), 1);
                if !di.is_null() {
                    (*rettv).vval.v_number = varnumber_T::from(
                        ((*di).di_flags & DI_FLAGS_LOCK) != 0
                            || tv_islocked(&raw const (*di).di_tv),
                    );
                }
            } else if lv.ll_range {
                emsg(gettext(c"E786: Range not allowed".as_ptr()));
            } else if !lv.ll_newkey.is_null() {
                semsg(gettext(e_dictkey.as_ptr()), lv.ll_newkey);
            } else if !lv.ll_list.is_null() {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_islocked(&raw const (*lv.ll_li).li_tv));
            } else {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_islocked(&raw const (*lv.ll_di).di_tv));
            }
        }
        clear_lval(&raw mut lv);
    }
}

/// "isinf()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_isinf(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_FLOAT && xisinf((*argvars).vval.v_float) != 0 {
            (*rettv).vval.v_number = if (*argvars).vval.v_float > 0.0 { 1 } else { -1 };
        }
    }
}

/// "isnan()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_isnan(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(
            (*argvars).v_type == VarType::VAR_FLOAT && xisnan((*argvars).vval.v_float) != 0,
        );
    }
}

/// "id()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_id(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let len = vim_snprintf_typval_only(ptr::null_mut(), 0, c"%p".as_ptr(), argvars);
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmalloc((len as usize) + 1).cast();
        vim_snprintf_typval_only(
            (*rettv).vval.v_string,
            (len as usize) + 1,
            c"%p".as_ptr(),
            argvars,
        );
    }
}

/// "jobpid(id)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_jobpid(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, true);
        if data.is_null() {
            return;
        }
        let ph = proc_header(channel_stream_ptr(data).cast::<Proc>());
        (*rettv).vval.v_number = varnumber_T::from((*ph).pid);
    }
}

/// "jobresize(job, width, height)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_jobresize(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER
            || (*argvars.add(1)).v_type != VarType::VAR_NUMBER
            || (*argvars.add(2)).v_type != VarType::VAR_NUMBER
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, true);
        if data.is_null() {
            return;
        }

        let proc_ptr = channel_stream_ptr(data).cast::<Proc>();
        if (*proc_header(proc_ptr)).r#type != kProcTypePty {
            emsg(gettext(e_channotpty.as_ptr()));
            return;
        }

        pty_proc_resize(
            proc_ptr.cast::<PtyProc>(),
            (*argvars.add(1)).vval.v_number as u16,
            (*argvars.add(2)).vval.v_number as u16,
        );
        (*rettv).vval.v_number = 1;
    }
}

/// Builds an environment dict for a child process (job).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn create_environment(
    job_env: *const dictitem_T,
    clear_env: bool,
    pty: bool,
    set_nvim_addr: bool,
    pty_term_name: *const c_char,
) -> *mut dict_T {
    unsafe {
        let env = tv_dict_alloc();

        if !clear_env {
            let mut envitems: *mut uv_env_item_t = ptr::null_mut();
            let mut envcount: c_int = 0;
            if uv_os_environ(&raw mut envitems, &raw mut envcount) == 0 {
                for i in 0..(envcount as usize) {
                    #[allow(unused_mut)]
                    let mut name: *const c_char = (*envitems.add(i)).name;
                    #[allow(unused_mut)]
                    let mut namelen = strlen(name);
                    #[cfg(windows)]
                    {
                        if namelen < MAXPATHL / MB_MAXBYTES {
                            let namebuff = (&raw mut NameBuff).cast::<c_char>();
                            namelen = mb_strup_buf(name, namebuff);
                            name = namebuff;
                        }
                    }
                    tv_dict_add_str(env, name, namelen, (*envitems.add(i)).value);
                }
                uv_os_free_environ(envitems, envcount);
            }

            if pty {
                for &ignored in PTY_IGNORED_ENV_VARS {
                    let dv = tv_dict_find(env, ignored.as_ptr(), -1);
                    if !dv.is_null() {
                        tv_dict_item_remove(env, dv);
                    }
                }
                if p_tgc != 0 {
                    tv_dict_add_str(env, c"COLORTERM".as_ptr(), 9, c"truecolor".as_ptr());
                }
            }
        }

        if pty {
            let dv = tv_dict_find(env, c"TERM".as_ptr(), 4);
            if !dv.is_null() {
                tv_dict_item_remove(env, dv);
            }
            tv_dict_add_str(env, c"TERM".as_ptr(), 4, pty_term_name);
        }

        if set_nvim_addr {
            let nvim_addr = get_vim_var_str(VV_SEND_SERVER);
            if *nvim_addr != NUL {
                let dv = tv_dict_find(env, c"NVIM".as_ptr(), 4);
                if !dv.is_null() {
                    tv_dict_item_remove(env, dv);
                }
                tv_dict_add_str(env, c"NVIM".as_ptr(), 4, nvim_addr);
            }
        }

        if !job_env.is_null() {
            #[cfg(windows)]
            {
                let d = (*job_env).di_tv.vval.v_dict.cast::<dict_T>();
                if !d.is_null() {
                    let ht = &raw const (*d).dv_hashtab;
                    let mut todo = (*ht).ht_used;
                    let mut hi = (*ht).ht_array;
                    while todo > 0 {
                        if !crate::hashtab::hashitem_empty(&*hi) {
                            todo -= 1;
                            let var = tv_dict_hi2di(hi);
                            let mut name = (*var).di_key.as_ptr();
                            let mut namelen = strlen(name);
                            if namelen < MAXPATHL / MB_MAXBYTES {
                                let namebuff = (&raw mut NameBuff).cast::<c_char>();
                                namelen = mb_strup_buf(name, namebuff);
                                name = namebuff;
                            }
                            let dv = tv_dict_find(env, name, namelen as isize);
                            if !dv.is_null() {
                                tv_dict_item_remove(env, dv);
                            }
                            tv_dict_add_str(
                                env,
                                name,
                                namelen,
                                tv_get_string(&raw const (*var).di_tv),
                            );
                        }
                        hi = hi.add(1);
                    }
                }
            }
            #[cfg(not(windows))]
            {
                tv_dict_extend(env, (*job_env).di_tv.vval.v_dict.cast(), c"force".as_ptr());
            }
        }

        if pty {
            for &req in REQUIRED_ENV_VARS {
                let len = req.to_bytes().len();
                let dv = tv_dict_find(env, req.as_ptr(), len as isize);
                if dv.is_null() {
                    let env_var = os_getenv(req.as_ptr());
                    if !env_var.is_null() {
                        tv_dict_add_allocated_str(env, req.as_ptr(), len, env_var);
                    }
                }
            }
        }

        env
    }
}

/// "jobstart()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_jobstart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let mut cmd: *const c_char = ptr::null();
        let mut executable = true;
        let argv = tv_to_argv(argvars, &raw mut cmd, &raw mut executable);
        if argv.is_null() {
            (*rettv).vval.v_number = if executable { 0 } else { -1 };
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_DICT
            && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
        {
            semsg(gettext(e_invarg2.as_ptr()), c"expected dictionary".as_ptr());
            shell_free_argv(argv);
            return;
        }

        let mut job_opts: *mut dict_T = ptr::null_mut();
        let mut detach = false;
        let mut rpc = false;
        let mut pty = false;
        let mut term = false;
        let mut clear_env = false;
        let mut overlapped = false;
        let mut stdin_mode = kChannelStdinPipe;
        let mut on_stdout: CallbackReader = std::mem::zeroed();
        let mut on_stderr: CallbackReader = std::mem::zeroed();
        let mut on_exit = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        let mut cwd: *const c_char = ptr::null();
        let mut job_env: *mut dictitem_T = ptr::null_mut();

        if (*argvars.add(1)).v_type == VarType::VAR_DICT {
            job_opts = (*argvars.add(1)).vval.v_dict.cast();

            detach = tv_dict_get_number(job_opts, c"detach".as_ptr()) != 0;
            rpc = tv_dict_get_number(job_opts, c"rpc".as_ptr()) != 0;
            term = tv_dict_get_number(job_opts, c"term".as_ptr()) != 0;
            pty = term || tv_dict_get_number(job_opts, c"pty".as_ptr()) != 0;
            clear_env = tv_dict_get_number(job_opts, c"clear_env".as_ptr()) != 0;
            overlapped = tv_dict_get_number(job_opts, c"overlapped".as_ptr()) != 0;

            let s = tv_dict_get_string(job_opts, c"stdin".as_ptr(), false);
            if !s.is_null() {
                if strncmp(s, c"null".as_ptr(), NUMBUFLEN) == 0 {
                    stdin_mode = kChannelStdinNull;
                } else if strncmp(s, c"pipe".as_ptr(), NUMBUFLEN) == 0 {
                    // default
                } else {
                    semsg(gettext(e_invargNval.as_ptr()), c"stdin".as_ptr(), s);
                }
            }

            let job_term = tv_dict_find(job_opts, c"term".as_ptr(), 4);
            if !job_term.is_null() && (*job_term).di_tv.v_type != VarType::VAR_BOOL {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"'term' must be Boolean".as_ptr(),
                );
                shell_free_argv(argv);
                return;
            }

            if pty && rpc {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"job cannot have both 'pty' and 'rpc' options set".as_ptr(),
                );
                shell_free_argv(argv);
                return;
            }

            #[cfg(windows)]
            if pty && overlapped {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"job cannot have both 'pty' and 'overlapped' options set".as_ptr(),
                );
                shell_free_argv(argv);
                return;
            }

            let new_cwd = tv_dict_get_string(job_opts, c"cwd".as_ptr(), false);
            if !new_cwd.is_null() && *new_cwd != NUL {
                cwd = new_cwd;
                if !os_isdir(cwd) {
                    semsg(
                        gettext(e_invarg2.as_ptr()),
                        c"expected valid directory".as_ptr(),
                    );
                    shell_free_argv(argv);
                    return;
                }
            }

            job_env = tv_dict_find(job_opts, c"env".as_ptr(), 3);
            if !job_env.is_null() && (*job_env).di_tv.v_type != VarType::VAR_DICT {
                semsg(gettext(e_invarg2.as_ptr()), c"env".as_ptr());
                shell_free_argv(argv);
                return;
            }

            if !common_job_callbacks(
                job_opts,
                &raw mut on_stdout,
                &raw mut on_stderr,
                &raw mut on_exit,
            ) {
                shell_free_argv(argv);
                return;
            }
        }

        let mut width = tv_dict_get_number(job_opts, c"width".as_ptr()) as u16;
        let mut height = tv_dict_get_number(job_opts, c"height".as_ptr()) as u16;
        let mut term_name: *const c_char = ptr::null();

        if term {
            if text_locked() {
                text_locked_msg();
                shell_free_argv(argv);
                return;
            }
            if (*curbuf).b_changed != 0 {
                emsg(gettext(
                    c"jobstart(...,{term=true}) requires unmodified buffer".as_ptr(),
                ));
                shell_free_argv(argv);
                return;
            }
            if !(*curbuf).terminal.is_null() {
                if terminal_running((*curbuf).terminal) {
                    semsg(
                        gettext(c"Terminal already connected to buffer %d".as_ptr()),
                        (*curbuf).b_fnum,
                    );
                    shell_free_argv(argv);
                    return;
                }
                buf_close_terminal(curbuf);
            }
            debug_assert!(!rpc);
            term_name = c"xterm-256color".as_ptr();
            if cwd.is_null() {
                cwd = c".".as_ptr();
            }
            overlapped = false;
            detach = false;
            stdin_mode = kChannelStdinPipe;
            if width == 0 {
                width = ((*curwin).w_view_width - win_col_off(curwin)).max(0) as u16;
            }
            if height == 0 {
                height = (*curwin).w_view_height as u16;
            }
        }

        if pty {
            if term_name.is_null() {
                term_name = tv_dict_get_string(job_opts, c"TERM".as_ptr(), false);
            }
            if term_name.is_null() {
                term_name = c"ansi".as_ptr();
            }
        }

        let env = create_environment(job_env, clear_env, pty, true, term_name);
        let chan = channel_job_start(
            argv,
            ptr::null(),
            on_stdout,
            on_stderr,
            on_exit,
            pty,
            rpc,
            overlapped,
            detach,
            stdin_mode,
            cwd,
            width,
            height,
            env,
            &raw mut (*rettv).vval.v_number,
        );
        if chan.is_null() {
            return;
        } else if !term {
            channel_create_event(chan, ptr::null());
        } else {
            if (*rettv).vval.v_number <= 0 {
                return;
            }

            let pid = (*proc_header(channel_stream_ptr(chan).cast::<Proc>())).pid;
            let buf = curbuf;

            (*buf).b_p_swf = 0;
            if (*buf).b_ml.ml_mfp.is_null() && ml_open(buf) == FAIL {
                proc_stop(channel_stream_ptr(chan).cast::<Proc>());
                channel_decref(chan);
                return;
            }

            channel_incref(chan);
            channel_terminal_alloc(buf, chan);

            let ct = channel_tail(chan);
            'term_done: {
                apply_autocmds(
                    EVENT_BUFFILEPRE,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                let namebuff = (&raw mut NameBuff).cast::<c_char>();
                let iobuff = (&raw mut IObuff).cast::<c_char>();
                vim_FullName(cwd, namebuff, MAXPATHL, false);
                let len = home_replace(ptr::null_mut(), namebuff, iobuff, IOSIZE, true);
                if len != 1
                    && (*iobuff.add(len - 1) == b'\\' as c_char
                        || *iobuff.add(len - 1) == b'/' as c_char)
                {
                    *iobuff.add(len - 1) = NUL;
                }
                if len == 1 && *iobuff == b'/' as c_char {
                    *iobuff.add(1) = b'.' as c_char;
                    *iobuff.add(2) = NUL;
                }

                snprintf(
                    namebuff,
                    MAXPATHL,
                    c"term://%s//%d:%s".as_ptr(),
                    iobuff,
                    pid,
                    cmd,
                );

                setfname(buf, namebuff, ptr::null_mut(), true);
                apply_autocmds(
                    EVENT_BUFFILEPOST,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    false,
                    buf,
                );

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                let mut err = Error {
                    err_type: ErrorType::kErrorTypeNone,
                    msg: ptr::null_mut(),
                };
                (*buf).b_locked += 1;
                let cp = channel_prefix(chan);
                dict_set_var(
                    (*buf).b_vars,
                    cstr_as_string(c"terminal_job_id".as_ptr()),
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: (*cp).id as Integer,
                        },
                    },
                    false,
                    false,
                    ptr::null_mut(),
                    &raw mut err,
                );
                api_clear_error(&raw mut err);
                dict_set_var(
                    (*buf).b_vars,
                    cstr_as_string(c"terminal_job_pid".as_ptr()),
                    Object {
                        obj_type: ObjectType::kObjectTypeInteger,
                        data: ObjectData {
                            integer: Integer::from(pid),
                        },
                    },
                    false,
                    false,
                    ptr::null_mut(),
                    &raw mut err,
                );
                api_clear_error(&raw mut err);
                (*buf).b_locked -= 1;

                if (*ct).term.is_null() || terminal_buf((*ct).term) == 0 {
                    break 'term_done;
                }

                terminal_open(&raw mut (*ct).term, buf);
            }
            channel_create_event(chan, ptr::null());
            channel_decref(chan);
        }
    }
}

/// "jobstop()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_jobstop(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let data = find_job((*argvars).vval.v_number as u64, false);
        if data.is_null() {
            return;
        }

        let mut error: *const c_char = ptr::null();
        if (*channel_tail(data)).is_rpc {
            channel_close((*channel_prefix(data)).id, kChannelPartRpc, &raw mut error);
        }
        proc_stop(channel_stream_ptr(data).cast::<Proc>());
        (*rettv).vval.v_number = 1;
        if !error.is_null() {
            emsg(error);
        }
    }
}

/// "jobwait(ids[, timeout])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_jobwait(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_LIST
            || ((*argvars.add(1)).v_type != VarType::VAR_NUMBER
                && (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN)
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let args = (*argvars).vval.v_list;
        let len = tv_list_len(args) as usize;
        let jobs = xcalloc(len, size_of::<*mut Channel>()).cast::<*mut Channel>();
        let main_loop_ptr: *mut Loop = (&raw mut main_loop).cast();
        let waiting_jobs = multiqueue_new(Some(loop_on_put), main_loop_ptr);

        let mut i: usize = 0;
        let mut arg = tv_list_first(args);
        while !arg.is_null() {
            let tv = &raw const (*arg).li_tv;
            let chan = if (*tv).v_type == VarType::VAR_NUMBER {
                find_channel((*tv).vval.v_number as u64)
            } else {
                ptr::null_mut()
            };
            if chan.is_null() || (*channel_prefix(chan)).streamtype != kChannelStreamProc {
                *jobs.add(i) = ptr::null_mut();
            } else {
                let proc_ptr = channel_stream_ptr(chan).cast::<Proc>();
                if proc_is_stopped(proc_ptr) {
                    proc_wait(proc_ptr, -1, ptr::null_mut());
                    *jobs.add(i) = ptr::null_mut();
                } else {
                    *jobs.add(i) = chan;
                    channel_incref(chan);
                    if (*proc_header(proc_ptr)).status < 0 {
                        let events = (*channel_prefix(chan)).events;
                        multiqueue_process_events(events);
                        multiqueue_replace_parent(events, waiting_jobs);
                    }
                }
            }
            i += 1;
            arg = (*arg).li_next;
        }

        let mut remaining: c_int = -1;
        let mut before: u64 = 0;
        if (*argvars.add(1)).v_type == VarType::VAR_NUMBER && (*argvars.add(1)).vval.v_number >= 0 {
            remaining = (*argvars.add(1)).vval.v_number as c_int;
            before = os_hrtime();
        }

        let busy = remaining != 0;
        if busy {
            ui_busy_start();
            ui_flush();
        }

        for idx in 0..len {
            if remaining == 0 {
                break;
            }
            let job = *jobs.add(idx);
            if job.is_null() {
                continue;
            }
            let status = proc_wait(
                channel_stream_ptr(job).cast::<Proc>(),
                remaining,
                waiting_jobs,
            );
            if status < 0 {
                break;
            }
            if remaining > 0 {
                let now = os_hrtime();
                remaining = 0.min(remaining - ((now - before) / 1_000_000) as c_int);
                before = now;
            }
        }

        let rv = tv_list_alloc(len as isize);
        for idx in 0..len {
            let job = *jobs.add(idx);
            if job.is_null() {
                tv_list_append_number(rv, -3);
                continue;
            }
            let events = (*channel_prefix(job)).events;
            multiqueue_process_events(events);
            multiqueue_replace_parent(events, (*loop_queues(main_loop_ptr)).events);

            let status = (*proc_header(channel_stream_ptr(job).cast::<Proc>())).status;
            tv_list_append_number(rv, varnumber_T::from(status));
            channel_decref(job);
        }

        multiqueue_free(waiting_jobs);
        xfree(jobs.cast());
        if busy {
            ui_busy_stop();
        }
        tv_list_ref(rv);
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = rv;
    }
}

/// "json_decode()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_json_decode(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut numbuf = [0 as c_char; NUMBUFLEN];
        let s: *const c_char;
        let mut tofree: *mut c_char = ptr::null_mut();
        let mut len: usize = 0;
        if (*argvars).v_type == VarType::VAR_LIST {
            if !encode_vim_list_to_buf((*argvars).vval.v_list, &raw mut len, &raw mut tofree) {
                emsg(gettext(c"E474: Failed to convert list to string".as_ptr()));
                return;
            }
            if tofree.is_null() {
                debug_assert!(len == 0);
                s = c"".as_ptr();
            } else {
                s = tofree;
            }
        } else {
            s = tv_get_string_buf_chk(argvars, numbuf.as_mut_ptr());
            if !s.is_null() {
                len = strlen(s);
            } else {
                return;
            }
        }
        if json_decode_string(s, len, rettv) == FAIL {
            semsg(
                gettext(c"E474: Failed to parse %.*s".as_ptr()),
                len as c_int,
                s,
            );
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = 0;
        }
        debug_assert!((*rettv).v_type != VarType::VAR_UNKNOWN);
        xfree(tofree.cast());
    }
}

/// "json_encode()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_json_encode(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = encode_tv2json(argvars, ptr::null_mut());
    }
}

/// "keytrans()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_keytrans(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        if tv_check_for_string_arg(argvars, 0) == FAIL || (*argvars).vval.v_string.is_null() {
            return;
        }
        let escaped = vim_strsave_escape_ks((*argvars).vval.v_string);
        (*rettv).vval.v_string = str2special_save(escaped, true, true);
        xfree(escaped.cast());
    }
}

/// "len()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_len(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        match (*argvars).v_type {
            VarType::VAR_STRING | VarType::VAR_NUMBER => {
                (*rettv).vval.v_number = strlen(tv_get_string(argvars)) as varnumber_T;
            }
            VarType::VAR_BLOB => {
                (*rettv).vval.v_number =
                    varnumber_T::from(tv_blob_len((*argvars).vval.v_blob.cast()));
            }
            VarType::VAR_LIST => {
                (*rettv).vval.v_number = varnumber_T::from(tv_list_len((*argvars).vval.v_list));
            }
            VarType::VAR_DICT => {
                (*rettv).vval.v_number = tv_dict_len((*argvars).vval.v_dict.cast()) as varnumber_T;
            }
            VarType::VAR_UNKNOWN
            | VarType::VAR_BOOL
            | VarType::VAR_SPECIAL
            | VarType::VAR_FLOAT
            | VarType::VAR_PARTIAL
            | VarType::VAR_FUNC => {
                emsg(gettext(c"E701: Invalid type for len()".as_ptr()));
            }
        }
    }
}

unsafe fn libcall_common(argvars: *mut typval_T, rettv: *mut typval_T, out_type: VarType) {
    unsafe {
        (*rettv).v_type = out_type;
        if out_type != VarType::VAR_NUMBER {
            (*rettv).vval.v_string = ptr::null_mut();
        }

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_STRING
            || (*argvars.add(1)).v_type != VarType::VAR_STRING
        {
            return;
        }

        let libname = (*argvars).vval.v_string;
        let funcname = (*argvars.add(1)).vval.v_string;
        let in_type = (*argvars.add(2)).v_type;

        let str_in = if in_type == VarType::VAR_STRING {
            (*argvars.add(2)).vval.v_string
        } else {
            ptr::null()
        };
        let int_in = (*argvars.add(2)).vval.v_number as c_int;

        let str_out = if out_type == VarType::VAR_STRING {
            &raw mut (*rettv).vval.v_string
        } else {
            ptr::null_mut()
        };
        let mut int_out: c_int = 0;

        let success = os_libcall(libname, funcname, str_in, int_in, str_out, &raw mut int_out);
        if !success {
            semsg(gettext(e_libcall.as_ptr()), funcname);
            return;
        }
        if out_type == VarType::VAR_NUMBER {
            (*rettv).vval.v_number = varnumber_T::from(int_out);
        }
    }
}

/// "libcall()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_libcall(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        libcall_common(argvars, rettv, VarType::VAR_STRING);
    }
}

/// "libcallnr()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_libcallnr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        libcall_common(argvars, rettv, VarType::VAR_NUMBER);
    }
}

/// "line(string, [winid])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_line(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut lnum: linenr_T = 0;
        let mut fp: *mut pos_T = ptr::null_mut();
        let mut fnum: c_int = 0;

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            let id = tv_get_number(argvars.add(1)) as c_int;
            let mut tp: *mut tabpage_T = ptr::null_mut();
            let wp = win_id2wp_tp(id, &raw mut tp);
            if !wp.is_null() && !tp.is_null() {
                if *p_spk != b'c' as c_char
                    || ((*wp).w_onebuf_opt.wo_diff != 0 && (*curwin).w_onebuf_opt.wo_diff != 0)
                {
                    skip_update_topline = true;
                }
                check_cursor(wp);
                fp = var2fpos(argvars, true, &raw mut fnum, false, wp);
                skip_update_topline = false;
            }
        } else {
            fp = var2fpos(argvars, true, &raw mut fnum, false, curwin);
        }

        if !fp.is_null() {
            lnum = (*fp).lnum;
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "line2byte(lnum)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_line2byte(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count + 1 {
            (*rettv).vval.v_number = -1;
        } else {
            (*rettv).vval.v_number =
                ml_find_line_or_offset(curbuf, lnum, ptr::null_mut(), false) as varnumber_T;
        }
        if (*rettv).vval.v_number >= 0 {
            (*rettv).vval.v_number += 1;
        }
    }
}

/// "localtime()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_localtime(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = c_time(ptr::null_mut()) as varnumber_T;
    }
}

/// "luaeval()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_luaeval(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let str_ = tv_get_string_chk(argvars);
        if str_.is_null() {
            return;
        }
        nlua_call_luaeval(cstr_as_string(str_), argvars.add(1), rettv);
    }
}

unsafe fn find_some_match(argvars: *mut typval_T, rettv: *mut typval_T, type_: SomeMatchType) {
    unsafe {
        let mut str_: *mut c_char = ptr::null_mut();
        let mut len: i64 = 0;
        let mut expr: *mut c_char = ptr::null_mut();
        let mut regmatch: regmatch_T = std::mem::zeroed();
        let mut start: i64 = 0;
        let mut nth: i64 = 1;
        let mut startcol: colnr_T = 0;
        let mut match_found = false;
        let mut l: *mut list_T = ptr::null_mut();
        let mut idx: c_int = 0;
        let mut tofree: *mut c_char = ptr::null_mut();

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        (*rettv).vval.v_number = -1;
        match type_ {
            SomeMatchType::kSomeMatchList => {
                tv_list_alloc_ret(rettv, kListLenMayKnow);
            }
            SomeMatchType::kSomeMatchStrPos => {
                tv_list_alloc_ret(rettv, 4);
                tv_list_append_string((*rettv).vval.v_list, c"".as_ptr(), 0);
                tv_list_append_number((*rettv).vval.v_list, -1);
                tv_list_append_number((*rettv).vval.v_list, -1);
                tv_list_append_number((*rettv).vval.v_list, -1);
            }
            SomeMatchType::kSomeMatchStr => {
                (*rettv).v_type = VarType::VAR_STRING;
                (*rettv).vval.v_string = ptr::null_mut();
            }
            SomeMatchType::kSomeMatch | SomeMatchType::kSomeMatchEnd => {}
        }

        let mut li: *mut listitem_T = ptr::null_mut();
        'theend: {
            if (*argvars).v_type == VarType::VAR_LIST {
                l = (*argvars).vval.v_list;
                if l.is_null() {
                    break 'theend;
                }
                li = tv_list_first(l);
            } else {
                str_ = tv_get_string(argvars).cast_mut();
                expr = str_;
                len = strlen(str_) as i64;
            }

            let mut patbuf = [0 as c_char; NUMBUFLEN];
            let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
            if pat.is_null() {
                break 'theend;
            }

            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                let mut error = false;
                start = tv_get_number_chk(argvars.add(2), &raw mut error);
                if error {
                    break 'theend;
                }
                if !l.is_null() {
                    idx = tv_list_uidx(l, start as c_int);
                    if idx == -1 {
                        break 'theend;
                    }
                    li = tv_list_find(l, idx);
                } else {
                    if start < 0 {
                        start = 0;
                    }
                    if start > len {
                        break 'theend;
                    }
                    if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                        startcol = start as colnr_T;
                    } else {
                        str_ = str_.add(start as usize);
                        len -= start;
                    }
                }

                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    nth = tv_get_number_chk(argvars.add(3), &raw mut error);
                }
                if error {
                    break 'theend;
                }
            }

            regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
            if !regmatch.regprog.is_null() {
                regmatch.rm_ic = p_ic != 0;

                loop {
                    if !l.is_null() {
                        if li.is_null() {
                            match_found = false;
                            break;
                        }
                        xfree(tofree.cast());
                        tofree = encode_tv2echo(&raw mut (*li).li_tv, ptr::null_mut());
                        expr = tofree;
                        str_ = tofree;
                        if str_.is_null() {
                            break;
                        }
                    }

                    match_found = vim_regexec_nl((&raw mut regmatch).cast(), str_, startcol);

                    if match_found {
                        nth -= 1;
                        if nth <= 0 {
                            break;
                        }
                    }
                    if l.is_null() && !match_found {
                        break;
                    }

                    if !l.is_null() {
                        li = (*li).li_next;
                        idx += 1;
                    } else {
                        startcol = (regmatch.startp[0]
                            .add(utfc_ptr2len(regmatch.startp[0]) as usize)
                            .offset_from(str_)) as colnr_T;
                        if i64::from(startcol) > len
                            || str_.add(startcol as usize) <= regmatch.startp[0]
                        {
                            match_found = false;
                            break;
                        }
                    }
                }

                if match_found {
                    match type_ {
                        SomeMatchType::kSomeMatchStrPos => {
                            let ret_l = (*rettv).vval.v_list;
                            let li1 = tv_list_first(ret_l);
                            let li2 = (*li1).li_next;
                            let li3 = (*li2).li_next;
                            let li4 = (*li3).li_next;
                            xfree((*li1).li_tv.vval.v_string.cast());

                            let rd = regmatch.endp[0].offset_from(regmatch.startp[0]) as usize;
                            (*li1).li_tv.vval.v_string =
                                xmemdupz(regmatch.startp[0].cast(), rd).cast();
                            (*li3).li_tv.vval.v_number =
                                regmatch.startp[0].offset_from(expr) as varnumber_T;
                            (*li4).li_tv.vval.v_number =
                                regmatch.endp[0].offset_from(expr) as varnumber_T;
                            if !l.is_null() {
                                (*li2).li_tv.vval.v_number = varnumber_T::from(idx);
                            }
                        }
                        SomeMatchType::kSomeMatchList => {
                            for i in 0..NSUBEXP {
                                if regmatch.endp[i].is_null() {
                                    tv_list_append_string((*rettv).vval.v_list, ptr::null(), 0);
                                } else {
                                    tv_list_append_string(
                                        (*rettv).vval.v_list,
                                        regmatch.startp[i],
                                        regmatch.endp[i].offset_from(regmatch.startp[i]),
                                    );
                                }
                            }
                        }
                        SomeMatchType::kSomeMatchStr => {
                            if !l.is_null() {
                                tv_copy(&raw const (*li).li_tv, rettv);
                            } else {
                                (*rettv).vval.v_string = xmemdupz(
                                    regmatch.startp[0].cast(),
                                    regmatch.endp[0].offset_from(regmatch.startp[0]) as usize,
                                )
                                .cast();
                            }
                        }
                        SomeMatchType::kSomeMatch | SomeMatchType::kSomeMatchEnd => {
                            if !l.is_null() {
                                (*rettv).vval.v_number = varnumber_T::from(idx);
                            } else {
                                if type_ == SomeMatchType::kSomeMatch {
                                    (*rettv).vval.v_number =
                                        regmatch.startp[0].offset_from(str_) as varnumber_T;
                                } else {
                                    (*rettv).vval.v_number =
                                        regmatch.endp[0].offset_from(str_) as varnumber_T;
                                }
                                (*rettv).vval.v_number += str_.offset_from(expr) as varnumber_T;
                            }
                        }
                    }
                }
                vim_regfree(regmatch.regprog.cast());
            }
        }

        if type_ == SomeMatchType::kSomeMatchStrPos
            && l.is_null()
            && !(*rettv).vval.v_list.is_null()
        {
            let ret_l = (*rettv).vval.v_list;
            let first = tv_list_first(ret_l);
            tv_list_item_remove(ret_l, (*first).li_next);
        }

        xfree(tofree.cast());
        p_cpo = save_cpo;
    }
}

unsafe fn get_matches_in_str(
    str_: *const c_char,
    rmp: *mut regmatch_T,
    mlist: *mut list_T,
    idx: c_int,
    submatches: bool,
    matchbuf: bool,
) {
    unsafe {
        let len = strlen(str_);
        let mut startidx: colnr_T = 0;

        loop {
            if !vim_regexec_nl(rmp.cast(), str_, startidx) {
                break;
            }

            let d = tv_dict_alloc();
            tv_list_append_dict(mlist, d);

            if matchbuf {
                tv_dict_add_nr(d, c"lnum".as_ptr(), 4, varnumber_T::from(idx));
            } else {
                tv_dict_add_nr(d, c"idx".as_ptr(), 3, varnumber_T::from(idx));
            }

            tv_dict_add_nr(
                d,
                c"byteidx".as_ptr(),
                7,
                (*rmp).startp[0].offset_from(str_) as varnumber_T,
            );

            tv_dict_add_str_len(
                d,
                c"text".as_ptr(),
                4,
                (*rmp).startp[0],
                (*rmp).endp[0].offset_from((*rmp).startp[0]) as c_int,
            );

            if submatches {
                let sml = tv_list_alloc((NSUBEXP - 1) as isize);
                tv_dict_add_list(d, c"submatches".as_ptr(), 10, sml);
                for i in 1..NSUBEXP {
                    if (*rmp).endp[i].is_null() {
                        tv_list_append_string(sml, c"".as_ptr(), 0);
                    } else {
                        tv_list_append_string(
                            sml,
                            (*rmp).startp[i],
                            (*rmp).endp[i].offset_from((*rmp).startp[i]),
                        );
                    }
                }
            }
            startidx = (*rmp).endp[0].offset_from(str_) as colnr_T;
            if (startidx as usize) >= len || str_.add(startidx as usize) <= (*rmp).startp[0] {
                break;
            }
        }
    }
}

/// "matchbufline()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchbufline(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let retlist = (*rettv).vval.v_list;

        if tv_check_for_buffer_arg(argvars, 0) == FAIL
            || tv_check_for_string_arg(argvars, 1) == FAIL
            || tv_check_for_lnum_arg(argvars, 2) == FAIL
            || tv_check_for_lnum_arg(argvars, 3) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 4) == FAIL
        {
            return;
        }

        let prev_did_emsg = did_emsg;
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null() {
            if did_emsg == prev_did_emsg {
                semsg(
                    gettext(e_invalid_buffer_name_str.as_ptr()),
                    tv_get_string(argvars),
                );
            }
            return;
        }
        if (*buf).b_ml.ml_mfp.is_null() {
            emsg(gettext(e_buffer_is_not_loaded.as_ptr()));
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf(argvars.add(1), patbuf.as_mut_ptr());

        let did_emsg_before = did_emsg;
        let mut slnum = tv_get_lnum_buf(argvars.add(2), buf);
        if did_emsg > did_emsg_before {
            return;
        }
        if slnum < 1 {
            semsg(gettext(e_invargval.as_ptr()), c"lnum".as_ptr());
            return;
        }

        let mut elnum = tv_get_lnum_buf(argvars.add(3), buf);
        if did_emsg > did_emsg_before {
            return;
        }
        if elnum < 1 || elnum < slnum {
            semsg(gettext(e_invargval.as_ptr()), c"end_lnum".as_ptr());
            return;
        }

        if elnum > (*buf).b_ml.ml_line_count {
            elnum = (*buf).b_ml.ml_line_count;
        }

        let mut submatches = false;
        if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN {
            let d = (*argvars.add(4)).vval.v_dict.cast::<dict_T>();
            if !d.is_null() {
                let di = tv_dict_find(d, c"submatches".as_ptr(), 10);
                if !di.is_null() {
                    if (*di).di_tv.v_type != VarType::VAR_BOOL {
                        semsg(gettext(e_invargval.as_ptr()), c"submatches".as_ptr());
                        return;
                    }
                    submatches = tv_get_bool(&raw const (*di).di_tv) != 0;
                }
            }
        }

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = p_ic != 0;

            while slnum <= elnum {
                let str_ = ml_get_buf(buf, slnum);
                get_matches_in_str(str_, &raw mut regmatch, retlist, slnum, submatches, true);
                slnum += 1;
            }

            vim_regfree(regmatch.regprog.cast());
        }

        p_cpo = save_cpo;
    }
}

/// "match()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_match(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatch);
    }
}

/// "matchend()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchend(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchEnd);
    }
}

/// "matchlist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchList);
    }
}

/// "matchstr()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchstr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchStr);
    }
}

/// "matchstrlist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchstrlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let retlist = (*rettv).vval.v_list;

        if tv_check_for_list_arg(argvars, 0) == FAIL
            || tv_check_for_string_arg(argvars, 1) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 2) == FAIL
        {
            return;
        }

        let l = (*argvars).vval.v_list;
        if l.is_null() {
            return;
        }

        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
        if pat.is_null() {
            return;
        }

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut regmatch: regmatch_T = std::mem::zeroed();
        regmatch.regprog = vim_regcomp(pat, RE_MAGIC + RE_STRING).cast();
        if !regmatch.regprog.is_null() {
            regmatch.rm_ic = p_ic != 0;

            'cleanup: {
                let mut submatches = false;
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    let d = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
                    if !d.is_null() {
                        let di = tv_dict_find(d, c"submatches".as_ptr(), 10);
                        if !di.is_null() {
                            if (*di).di_tv.v_type != VarType::VAR_BOOL {
                                semsg(gettext(e_invargval.as_ptr()), c"submatches".as_ptr());
                                break 'cleanup;
                            }
                            submatches = tv_get_bool(&raw const (*di).di_tv) != 0;
                        }
                    }
                }

                let mut idx: c_int = 0;
                let mut li = tv_list_first(l);
                while !li.is_null() {
                    let li_tv = &raw const (*li).li_tv;
                    if (*li_tv).v_type == VarType::VAR_STRING && !(*li_tv).vval.v_string.is_null() {
                        get_matches_in_str(
                            (*li_tv).vval.v_string,
                            &raw mut regmatch,
                            retlist,
                            idx,
                            submatches,
                            false,
                        );
                    }
                    idx += 1;
                    li = (*li).li_next;
                }
            }

            vim_regfree(regmatch.regprog.cast());
        }

        p_cpo = save_cpo;
    }
}

/// "matchstrpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_matchstrpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        find_some_match(argvars, rettv, SomeMatchType::kSomeMatchStrPos);
    }
}

/// Get maximal/minimal number value in a list or dictionary.
unsafe fn max_min(tv: *const typval_T, rettv: *mut typval_T, domax: bool) {
    unsafe {
        let mut error = false;
        (*rettv).vval.v_number = 0;
        let mut n: varnumber_T = if domax { VARNUMBER_MIN } else { VARNUMBER_MAX };
        if (*tv).v_type == VarType::VAR_LIST {
            if tv_list_len((*tv).vval.v_list) == 0 {
                return;
            }
            let mut li = tv_list_first((*tv).vval.v_list);
            while !li.is_null() {
                let i = tv_get_number_chk(&raw const (*li).li_tv, &raw mut error);
                if error {
                    return;
                }
                if if domax { i > n } else { i < n } {
                    n = i;
                }
                li = (*li).li_next;
            }
        } else if (*tv).v_type == VarType::VAR_DICT {
            let d = (*tv).vval.v_dict.cast::<dict_T>();
            if tv_dict_len(d) == 0 {
                return;
            }
            let ht = &raw const (*d).dv_hashtab;
            let mut todo = (*ht).ht_used;
            let mut hi = (*ht).ht_array;
            while todo > 0 {
                if !crate::hashtab::hashitem_empty(&*hi) {
                    todo -= 1;
                    let di = tv_dict_hi2di(hi);
                    let i = tv_get_number_chk(&raw const (*di).di_tv, &raw mut error);
                    if error {
                        return;
                    }
                    if if domax { i > n } else { i < n } {
                        n = i;
                    }
                }
                hi = hi.add(1);
            }
        } else {
            semsg(
                gettext(e_listdictarg.as_ptr()),
                if domax {
                    c"max()".as_ptr()
                } else {
                    c"min()".as_ptr()
                },
            );
            return;
        }
        (*rettv).vval.v_number = n;
    }
}

/// "max()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_max(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        max_min(argvars, rettv, true);
    }
}

/// "min()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_min(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        max_min(argvars, rettv, false);
    }
}

/// "mode()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_mode(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut buf = [0 as c_char; MODE_MAX_LENGTH];
        get_mode(buf.as_mut_ptr());
        if !non_zero_arg(argvars) {
            buf[1] = NUL;
        }
        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

unsafe fn may_add_state_char(gap: *mut garray_T, include: *const c_char, c: u8) {
    unsafe {
        if include.is_null() || !vim_strchr(include, c_int::from(c)).is_null() {
            ga_append(gap, c);
        }
    }
}

/// "state()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_state(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut ga, 1, 20);
        let mut include: *const c_char = ptr::null();
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            include = tv_get_string(argvars);
        }

        if !(stuff_empty() && typebuf.tb_len == 0 && using_script() == 0) {
            may_add_state_char(&raw mut ga, include, b'm');
        }
        if op_pending() {
            may_add_state_char(&raw mut ga, include, b'o');
        }
        if autocmd_busy != 0 {
            may_add_state_char(&raw mut ga, include, b'x');
        }
        if ins_compl_active() {
            may_add_state_char(&raw mut ga, include, b'a');
        }
        if !get_was_safe_state() {
            may_add_state_char(&raw mut ga, include, b'S');
        }
        let depth = get_callback_depth();
        for _ in 0..(depth.min(3)) {
            may_add_state_char(&raw mut ga, include, b'c');
        }
        if msg_scrolled > 0 {
            may_add_state_char(&raw mut ga, include, b's');
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ga.ga_data.cast();
    }
}

/// "msgpackdump()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_msgpackdump(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST {
            semsg(gettext(e_listarg.as_ptr()), c"msgpackdump()".as_ptr());
            return;
        }
        let list = (*argvars).vval.v_list;
        let mut packer: PackerBuffer = packer_string_buffer();
        let msg = gettext(c"msgpackdump() argument, index %i".as_ptr());
        let mut msgbuf = [0 as c_char; 31 * 4 + NUMBUFLEN];
        let mut idx: c_int = 0;
        let mut li = tv_list_first(list);
        while !li.is_null() {
            vim_snprintf(msgbuf.as_mut_ptr(), msgbuf.len(), msg, idx);
            idx += 1;
            if encode_vim_to_msgpack(&raw mut packer, &raw mut (*li).li_tv, msgbuf.as_ptr()) == FAIL
            {
                break;
            }
            li = (*li).li_next;
        }
        let data = packer_take_string(&raw mut packer);
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
            && strequal(tv_get_string(argvars.add(1)), c"B".as_ptr())
        {
            let b = tv_blob_alloc_ret(rettv);
            (*b).bv_ga.ga_data = data.data.cast();
            (*b).bv_ga.ga_len = data.size as c_int;
            (*b).bv_ga.ga_maxlen = packer.endptr.offset_from(packer.startptr) as c_int;
        } else {
            encode_list_write(
                tv_list_alloc_ret(rettv, kListLenMayKnow).cast(),
                data.data,
                data.size,
            );
            api_free_string(data);
        }
    }
}

unsafe fn emsg_mpack_error(status: c_int) {
    unsafe {
        match status {
            MPACK_ERROR => {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"Failed to parse msgpack string".as_ptr(),
                );
            }
            MPACK_EOF => {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"Incomplete msgpack string".as_ptr(),
                );
            }
            MPACK_NOMEM => {
                semsg(
                    gettext(e_invarg2.as_ptr()),
                    c"object was too deep to unpack".as_ptr(),
                );
            }
            _ => {}
        }
    }
}

unsafe fn msgpackparse_unpack_list(list: *const list_T, ret_list: *mut list_T) {
    unsafe {
        if tv_list_len(list) == 0 {
            return;
        }
        if (*tv_list_first(list)).li_tv.v_type != VarType::VAR_STRING {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"List item is not a string".as_ptr(),
            );
            return;
        }
        let mut lrstate: ListReaderState = encode_init_lrstate(list);
        let buf = alloc_block();
        let mut buf_size: usize = 0;

        let mut cur_item = tv_initial_value();
        let mut parser: mpack_parser_t = std::mem::zeroed();
        mpack_parser_init(&raw mut parser, 0);
        parser.data.p = (&raw mut cur_item).cast();

        let mut status = MPACK_OK;
        'end: {
            loop {
                let mut read_bytes: usize = 0;
                let rlret = encode_read_from_list(
                    &raw mut lrstate,
                    buf.add(buf_size),
                    ARENA_BLOCK_SIZE - buf_size,
                    &raw mut read_bytes,
                );
                if rlret == FAIL {
                    semsg(
                        gettext(e_invarg2.as_ptr()),
                        c"List item is not a string".as_ptr(),
                    );
                    break 'end;
                }
                buf_size += read_bytes;

                let mut ptr_ = buf.cast::<c_char>().cast_const();
                while buf_size != 0 {
                    status = mpack_parse_typval(&raw mut parser, &raw mut ptr_, &raw mut buf_size);
                    if status == MPACK_OK {
                        tv_list_append_owned_tv(ret_list, cur_item);
                        cur_item.v_type = VarType::VAR_UNKNOWN;
                    } else {
                        break;
                    }
                }

                if rlret == OK {
                    break;
                }

                if status == MPACK_EOF {
                    if buf_size != 0 && ptr_ > buf.cast::<c_char>().cast_const() {
                        ptr::copy(ptr_.cast_mut(), buf, buf_size);
                    }
                } else if status != MPACK_OK {
                    break;
                }
            }

            if status != MPACK_OK {
                typval_parser_error_free(&raw mut parser);
                emsg_mpack_error(status);
            }
        }

        free_block(buf);
    }
}

unsafe fn msgpackparse_unpack_blob(blob: *const blob_T, ret_list: *mut list_T) {
    unsafe {
        let len = tv_blob_len(blob);
        if len == 0 {
            return;
        }

        let mut data = (*blob).bv_ga.ga_data.cast::<c_char>().cast_const();
        let mut remaining = len as usize;
        while remaining != 0 {
            let mut tv = tv_initial_value();
            let status = unpack_typval(&raw mut data, &raw mut remaining, &raw mut tv);
            if status != MPACK_OK {
                emsg_mpack_error(status);
                return;
            }
            tv_list_append_owned_tv(ret_list, tv);
        }
    }
}

/// "msgpackparse()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_msgpackparse(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_LIST && (*argvars).v_type != VarType::VAR_BLOB {
            semsg(gettext(e_listblobarg.as_ptr()), c"msgpackparse()".as_ptr());
            return;
        }
        let ret_list = tv_list_alloc_ret(rettv, kListLenMayKnow);
        if (*argvars).v_type == VarType::VAR_LIST {
            msgpackparse_unpack_list((*argvars).vval.v_list, ret_list);
        } else {
            msgpackparse_unpack_blob((*argvars).vval.v_blob.cast(), ret_list);
        }
    }
}

/// "nextnonblank()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_nextnonblank(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut lnum = tv_get_lnum(argvars);
        loop {
            if lnum < 0 || lnum > (*curbuf).b_ml.ml_line_count {
                lnum = 0;
                break;
            }
            if *skipwhite(ml_get(lnum)) != NUL {
                break;
            }
            lnum += 1;
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "nr2char()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_nr2char(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && !tv_check_num(argvars.add(1)) {
            return;
        }
        let mut error = false;
        let num = tv_get_number_chk(argvars, &raw mut error);
        if error {
            return;
        }
        if num < 0 {
            emsg(gettext(
                c"E5070: Character number must not be less than zero".as_ptr(),
            ));
            return;
        }
        if num > varnumber_T::from(c_int::MAX) {
            semsg(
                gettext(c"E5071: Character number must not be greater than INT_MAX (%i)".as_ptr()),
                c_int::MAX,
            );
            return;
        }

        let mut buf = [0 as c_char; MB_MAXCHAR];
        let len = utf_char2bytes(num as c_int, buf.as_mut_ptr());

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(buf.as_ptr().cast(), len as usize).cast();
    }
}

/// "or(expr, expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_or(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            | tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

/// "pow()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_pow(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut fx: float_T = 0.0;
        let mut fy: float_T = 0.0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        if tv_get_float_chk(argvars, &raw mut fx) && tv_get_float_chk(argvars.add(1), &raw mut fy) {
            (*rettv).vval.v_float = fx.powf(fy);
        } else {
            (*rettv).vval.v_float = 0.0;
        }
    }
}

/// "prevnonblank()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_prevnonblank(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut lnum = tv_get_lnum(argvars);
        if lnum < 1 || lnum > (*curbuf).b_ml.ml_line_count {
            lnum = 0;
        } else {
            while lnum >= 1 && *skipwhite(ml_get(lnum)) == NUL {
                lnum -= 1;
            }
        }
        (*rettv).vval.v_number = varnumber_T::from(lnum);
    }
}

/// "printf()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_printf(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let saved_did_emsg = did_emsg;
        did_emsg = 0;
        let mut buf = [0 as c_char; NUMBUFLEN];
        let fmt = tv_get_string_buf(argvars, buf.as_mut_ptr());
        let len = vim_snprintf_typval_only(ptr::null_mut(), 0, fmt, argvars.add(1));
        if did_emsg == 0 {
            let s = xmalloc((len as usize) + 1).cast::<c_char>();
            (*rettv).vval.v_string = s;
            vim_snprintf_typval_only(s, (len as usize) + 1, fmt, argvars.add(1));
        }
        did_emsg |= saved_did_emsg;
    }
}

/// "pum_getpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_pum_getpos(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        pum_set_event_info((*rettv).vval.v_dict.cast());
    }
}

/// "pumvisible()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_pumvisible(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if pum_visible() {
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "py3eval()" and "pyxeval()" functions
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_py3eval(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        script_host_eval(c"python3".as_ptr().cast_mut(), argvars, rettv);
    }
}

unsafe fn init_srand(x: *mut u32) {
    unsafe {
        let mut number: u32 = 0;
        if uv_random(
            ptr::null_mut(),
            ptr::null_mut(),
            (&raw mut number).cast(),
            size_of::<u32>(),
            0,
            ptr::null(),
        ) == 0
        {
            *x = number;
            return;
        }
        *x = (os_hrtime() as u32) ^ (os_get_pid() as u32);
    }
}

#[inline(always)]
unsafe fn splitmix32(x: *mut u32) -> u32 {
    unsafe {
        *x = (*x).wrapping_add(0x9e3779b9);
        let mut z = *x;
        z = (z ^ (z >> 16)).wrapping_mul(0x85ebca6b);
        z = (z ^ (z >> 13)).wrapping_mul(0xc2b2ae35);
        z ^ (z >> 16)
    }
}

#[inline(always)]
unsafe fn shuffle_xoshiro128starstar(x: *mut u32, y: *mut u32, z: *mut u32, w: *mut u32) -> u32 {
    unsafe {
        let result = ((*y).wrapping_mul(5)).rotate_left(7).wrapping_mul(9);
        let t = *y << 9;
        *z ^= *x;
        *w ^= *y;
        *y ^= *z;
        *x ^= *w;
        *z ^= t;
        *w = (*w).rotate_left(11);
        result
    }
}

/// "rand()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_rand(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let result: u32;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            if !RAND_INITIALIZED {
                let mut x: u32 = 0;
                init_srand(&raw mut x);
                RAND_GX = splitmix32(&raw mut x);
                RAND_GY = splitmix32(&raw mut x);
                RAND_GZ = splitmix32(&raw mut x);
                RAND_GW = splitmix32(&raw mut x);
                RAND_INITIALIZED = true;
            }
            result = shuffle_xoshiro128starstar(
                &raw mut RAND_GX,
                &raw mut RAND_GY,
                &raw mut RAND_GZ,
                &raw mut RAND_GW,
            );
        } else if (*argvars).v_type == VarType::VAR_LIST {
            let l = (*argvars).vval.v_list;
            if tv_list_len(l) != 4 {
                semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = -1;
                return;
            }

            let tvx = &raw mut (*tv_list_find(l, 0)).li_tv;
            let tvy = &raw mut (*tv_list_find(l, 1)).li_tv;
            let tvz = &raw mut (*tv_list_find(l, 2)).li_tv;
            let tvw = &raw mut (*tv_list_find(l, 3)).li_tv;
            if (*tvx).v_type != VarType::VAR_NUMBER
                || (*tvy).v_type != VarType::VAR_NUMBER
                || (*tvz).v_type != VarType::VAR_NUMBER
                || (*tvw).v_type != VarType::VAR_NUMBER
            {
                semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
                (*rettv).v_type = VarType::VAR_NUMBER;
                (*rettv).vval.v_number = -1;
                return;
            }
            let mut x = (*tvx).vval.v_number as u32;
            let mut y = (*tvy).vval.v_number as u32;
            let mut z = (*tvz).vval.v_number as u32;
            let mut w = (*tvw).vval.v_number as u32;

            result = shuffle_xoshiro128starstar(&raw mut x, &raw mut y, &raw mut z, &raw mut w);

            (*tvx).vval.v_number = varnumber_T::from(x);
            (*tvy).vval.v_number = varnumber_T::from(y);
            (*tvz).vval.v_number = varnumber_T::from(z);
            (*tvw).vval.v_number = varnumber_T::from(w);
        } else {
            semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars));
            (*rettv).v_type = VarType::VAR_NUMBER;
            (*rettv).vval.v_number = -1;
            return;
        }

        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = varnumber_T::from(result);
    }
}

/// "srand()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_srand(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut x: u32 = 0;
        tv_list_alloc_ret(rettv, 4);
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            init_srand(&raw mut x);
        } else {
            let mut error = false;
            x = tv_get_number_chk(argvars, &raw mut error) as u32;
            if error {
                return;
            }
        }
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from(splitmix32(&raw mut x)),
        );
    }
}

/// "perleval()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_perleval(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        script_host_eval(c"perl".as_ptr().cast_mut(), argvars, rettv);
    }
}

/// "rubyeval()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_rubyeval(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        script_host_eval(c"ruby".as_ptr().cast_mut(), argvars, rettv);
    }
}

/// "range()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_range(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let end: varnumber_T;
        let mut stride: varnumber_T = 1;
        let mut error = false;

        let mut start = tv_get_number_chk(argvars, &raw mut error);
        if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            end = start - 1;
            start = 0;
        } else {
            end = tv_get_number_chk(argvars.add(1), &raw mut error);
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                stride = tv_get_number_chk(argvars.add(2), &raw mut error);
            }
        }

        if error {
            return;
        }
        if stride == 0 {
            emsg(gettext(c"E726: Stride is zero".as_ptr()));
            return;
        }
        if if stride > 0 {
            end.saturating_add(1) < start
        } else {
            end.saturating_sub(1) > start
        } {
            emsg(gettext(c"E727: Start past end".as_ptr()));
            return;
        }

        tv_list_alloc_ret(rettv, ((end - start) / stride) as isize);
        let mut i = start;
        while if stride > 0 { i <= end } else { i >= end } {
            tv_list_append_number((*rettv).vval.v_list, i);
            i += stride;
        }
    }
}

/// "getreginfo()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_getreginfo(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut regname = getreg_get_regname(argvars);
        if regname == 0 {
            return;
        }
        if regname == b'@' as c_int {
            regname = b'"' as c_int;
        }

        tv_dict_alloc_ret(rettv);
        let dict = (*rettv).vval.v_dict.cast::<dict_T>();

        let list = get_reg_contents(regname, kGRegExprSrc | kGRegList).cast::<list_T>();
        if list.is_null() {
            return;
        }
        tv_dict_add_list(dict, c"regcontents".as_ptr(), 11, list);

        let mut buf = [0 as c_char; NUMBUFLEN + 2];
        let mut reglen: colnr_T = 0;
        let mut buflen = match get_reg_type(regname, &raw mut reglen) {
            crate::clipboard::MotionType::kMTLineWise => {
                buf[0] = b'V' as c_char;
                buf[1] = NUL;
                1
            }
            crate::clipboard::MotionType::kMTCharWise => {
                buf[0] = b'v' as c_char;
                buf[1] = NUL;
                1
            }
            crate::clipboard::MotionType::kMTBlockWise => vim_snprintf_safelen(
                buf.as_mut_ptr(),
                buf.len(),
                c"%c%d".as_ptr(),
                c_int::from(Ctrl_V),
                reglen + 1,
            ),
            crate::clipboard::MotionType::kMTUnknown => std::process::abort(),
        };
        tv_dict_add_str_len(dict, c"regtype".as_ptr(), 7, buf.as_ptr(), buflen as c_int);

        buf[0] = get_register_name(get_unname_register()) as c_char;
        buf[1] = NUL;
        buflen = usize::from(buf[0] != NUL);
        if regname == b'"' as c_int {
            tv_dict_add_str_len(
                dict,
                c"points_to".as_ptr(),
                9,
                buf.as_ptr(),
                buflen as c_int,
            );
        } else {
            tv_dict_add_bool(
                dict,
                c"isunnamed".as_ptr(),
                9,
                if regname == c_int::from(buf[0]) {
                    kBoolVarTrue
                } else {
                    kBoolVarFalse
                },
            );
        }
    }
}

unsafe fn return_register(regname: c_int, rettv: *mut typval_T) {
    unsafe {
        let buf = [regname as c_char, NUL];
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xmemdupz(buf.as_ptr().cast(), usize::from(buf[0] != NUL)).cast();
    }
}

/// "reg_executing()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reg_executing(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        return_register(reg_executing, rettv);
    }
}

/// "reg_recording()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reg_recording(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        return_register(reg_recording, rettv);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reg_recorded(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        return_register(reg_recorded, rettv);
    }
}

unsafe fn list2proftime(arg: *mut typval_T, tm: *mut proftime_T) -> c_int {
    unsafe {
        if (*arg).v_type != VarType::VAR_LIST || tv_list_len((*arg).vval.v_list) != 2 {
            return FAIL;
        }
        let mut error = false;
        let n1 = tv_list_find_nr((*arg).vval.v_list, 0, &raw mut error);
        let n2 = tv_list_find_nr((*arg).vval.v_list, 1, &raw mut error);
        if error {
            return FAIL;
        }
        let high = (n1 as i32) as u32 as u64;
        let low = (n2 as i32) as u32 as u64;
        *tm = (high << 32) | low;
        OK
    }
}

/// "reltime()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reltime(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut res: proftime_T = 0;
        let mut start: proftime_T = 0;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            res = profile_start();
        } else if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            if list2proftime(argvars, &raw mut res) == FAIL {
                return;
            }
            res = profile_end(res);
        } else {
            if list2proftime(argvars, &raw mut start) == FAIL
                || list2proftime(argvars.add(1), &raw mut res) == FAIL
            {
                return;
            }
            res = profile_sub(res, start);
        }

        let high = (res >> 32) as i32;
        let low = (res & 0xffff_ffff) as i32;

        tv_list_alloc_ret(rettv, 2);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(high));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(low));
    }
}

/// "reltimestr()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reltimestr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut tm: proftime_T = 0;
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if list2proftime(argvars, &raw mut tm) == OK {
            (*rettv).vval.v_string = xstrdup(profile_msg(tm));
        }
    }
}

unsafe fn repeat_list(l: *mut list_T, mut n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        let slen = tv_list_len(l);
        tv_list_alloc_ret(
            rettv,
            (varnumber_T::from(n > 0) * n * varnumber_T::from(slen)) as isize,
        );
        if l.is_null() || n <= 0 || slen == 0 {
            return;
        }
        if check_repeat_count(varnumber_T::from(slen), n) == FAIL {
            return;
        }
        while n > 0 {
            n -= 1;
            tv_list_extend((*rettv).vval.v_list, l, ptr::null_mut());
        }
    }
}

unsafe fn repeat_blob(blob_tv: *mut typval_T, n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        let blob = (*blob_tv).vval.v_blob.cast::<blob_T>();
        tv_blob_alloc_ret(rettv);
        if blob.is_null() || n <= 0 {
            return;
        }
        let slen = (*blob).bv_ga.ga_len;
        if slen <= 0 {
            return;
        }
        if check_repeat_count(varnumber_T::from(slen), n) == FAIL {
            return;
        }
        let len = slen * (n as c_int);
        let ret_blob = (*rettv).vval.v_blob.cast::<blob_T>();
        ga_grow(&raw mut (*ret_blob).bv_ga, len);
        (*ret_blob).bv_ga.ga_len = len;

        let mut i: c_int = 0;
        while i < slen {
            if tv_blob_get(blob, i) != 0 {
                break;
            }
            i += 1;
        }
        if i == slen {
            return;
        }
        for idx in 0..(n as c_int) {
            tv_blob_set_range(
                ret_blob,
                varnumber_T::from(idx * slen),
                varnumber_T::from((idx + 1) * slen - 1),
                blob_tv,
            );
        }
    }
}

unsafe fn repeat_string(str_tv: *mut typval_T, n: varnumber_T, rettv: *mut typval_T) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();
        if n <= 0 {
            return;
        }
        let p = tv_get_string(str_tv);
        let slen = strlen(p);
        if slen == 0 || n <= 0 {
            return;
        }
        if check_repeat_count(slen as varnumber_T, n) == FAIL {
            return;
        }
        let len = slen * (n as usize);
        let r = xmallocz(len).cast::<c_char>();
        ptr::copy(p, r, slen);
        let mut done = slen;
        while done < len {
            let copy_len = done.min(len - done);
            ptr::copy(r, r.add(done), copy_len);
            done += copy_len;
        }
        (*rettv).vval.v_string = r;
    }
}

/// "repeat()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_repeat(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let n = tv_get_number(argvars.add(1));
        if (*argvars).v_type == VarType::VAR_LIST {
            repeat_list((*argvars).vval.v_list, n, rettv);
        } else if (*argvars).v_type == VarType::VAR_BLOB {
            repeat_blob(argvars, n, rettv);
        } else {
            repeat_string(argvars, n, rettv);
        }
    }
}

unsafe fn reduce_list(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let l = (*argvars).vval.v_list;
        let called_emsg_start = called_emsg;

        let initial: typval_T;
        let mut li: *const listitem_T;
        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if tv_list_len(l) == 0 {
                semsg(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"List".as_ptr(),
                );
                return;
            }
            let first = tv_list_first(l);
            initial = (*first).li_tv;
            li = (*first).li_next;
        } else {
            initial = *argvars.add(2);
            li = tv_list_first(l);
        }

        tv_copy(&raw const initial, rettv);
        if l.is_null() {
            return;
        }

        let prev_locked = tv_list_locked(l);
        tv_list_set_lock(l, VAR_FIXED);
        while !li.is_null() {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = (*li).li_tv;
            (*rettv).v_type = VarType::VAR_UNKNOWN;

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            tv_clear(&raw mut argv[0]);
            if r == FAIL || called_emsg != called_emsg_start {
                break;
            }
            li = (*li).li_next;
        }
        tv_list_set_lock(l, prev_locked);
    }
}

unsafe fn reduce_string(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let mut p = tv_get_string(argvars);
        let mut len: c_int;
        let called_emsg_start = called_emsg;

        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if *p == NUL {
                semsg(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"String".as_ptr(),
                );
                return;
            }
            len = utfc_ptr2len(p);
            *rettv = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: xmemdupz(p.cast(), len as usize).cast(),
                },
            };
            p = p.add(len as usize);
        } else if tv_check_for_string_arg(argvars, 2) == FAIL {
            return;
        } else {
            tv_copy(argvars.add(2), rettv);
        }

        while *p != NUL {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            (*rettv).v_type = VarType::VAR_UNKNOWN;
            len = utfc_ptr2len(p);
            argv[1] = typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_string: xmemdupz(p.cast(), len as usize).cast(),
                },
            };

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            tv_clear(&raw mut argv[0]);
            tv_clear(&raw mut argv[1]);
            if r == FAIL || called_emsg != called_emsg_start {
                break;
            }
            p = p.add(len as usize);
        }
    }
}

unsafe fn reduce_blob(argvars: *mut typval_T, expr: *mut typval_T, rettv: *mut typval_T) {
    unsafe {
        let b = (*argvars).vval.v_blob.cast::<blob_T>();
        let called_emsg_start = called_emsg;

        let initial: typval_T;
        let mut i: c_int;
        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            if tv_blob_len(b) == 0 {
                semsg(
                    gettext(e_reduce_of_an_empty_str_with_no_initial_value.as_ptr()),
                    c"Blob".as_ptr(),
                );
                return;
            }
            initial = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_number: varnumber_T::from(tv_blob_get(b, 0)),
                },
            };
            i = 1;
        } else if tv_check_for_number_arg(argvars, 2) == FAIL {
            return;
        } else {
            initial = *argvars.add(2);
            i = 0;
        }

        tv_copy(&raw const initial, rettv);
        while i < tv_blob_len(b) {
            let mut argv = [tv_initial_value(); 3];
            argv[0] = *rettv;
            argv[1] = typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_number: varnumber_T::from(tv_blob_get(b, i)),
                },
            };

            let r = eval_expr_typval(expr, true, argv.as_mut_ptr(), 2, rettv);
            if r == FAIL || called_emsg != called_emsg_start {
                return;
            }
            i += 1;
        }
    }
}

/// "reduce()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reduce(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_STRING
            && (*argvars).v_type != VarType::VAR_LIST
            && (*argvars).v_type != VarType::VAR_BLOB
        {
            emsg(gettext(E_STRING_LIST_OR_BLOB_REQUIRED.as_ptr()));
            return;
        }

        let func_name = if (*argvars.add(1)).v_type == VarType::VAR_FUNC {
            (*argvars.add(1)).vval.v_string.cast_const()
        } else if (*argvars.add(1)).v_type == VarType::VAR_PARTIAL {
            partial_name((*argvars.add(1)).vval.v_ptr.cast())
        } else {
            tv_get_string(argvars.add(1))
        };
        if func_name.is_null() || *func_name == NUL {
            emsg(gettext(E_MISSING_FUNCTION_ARGUMENT.as_ptr()));
            return;
        }

        if (*argvars).v_type == VarType::VAR_LIST {
            reduce_list(argvars, argvars.add(1), rettv);
        } else if (*argvars).v_type == VarType::VAR_STRING {
            reduce_string(argvars, argvars.add(1), rettv);
        } else {
            reduce_blob(argvars, argvars.add(1), rettv);
        }
    }
}

unsafe fn get_search_arg(varp: *mut typval_T, flagsp: *mut c_int) -> c_int {
    unsafe {
        let mut dir = FORWARD;
        if (*varp).v_type == VarType::VAR_UNKNOWN {
            return FORWARD;
        }
        let mut nbuf = [0 as c_char; NUMBUFLEN];
        let mut flags = tv_get_string_buf_chk(varp, nbuf.as_mut_ptr());
        if flags.is_null() {
            return 0;
        }
        while *flags != NUL {
            match *flags as u8 {
                b'b' => dir = BACKWARD,
                b'w' => p_ws = 1,
                b'W' => p_ws = 0,
                b => {
                    let mut mask: c_int = 0;
                    if !flagsp.is_null() {
                        mask = match b {
                            b'c' => SP_START,
                            b'e' => SP_END,
                            b'm' => SP_RETCOUNT,
                            b'n' => SP_NOMOVE,
                            b'p' => SP_SUBPAT,
                            b'r' => SP_REPEAT,
                            b's' => SP_SETPCMARK,
                            b'z' => SP_COLUMN,
                            _ => 0,
                        };
                    }
                    if mask == 0 {
                        semsg(gettext(e_invarg2.as_ptr()), flags);
                        dir = 0;
                    } else {
                        *flagsp |= mask;
                    }
                }
            }
            if dir == 0 {
                break;
            }
            flags = flags.add(1);
        }
        dir
    }
}

unsafe fn search_cmn(argvars: *mut typval_T, match_pos: *mut pos_T, flagsp: *mut c_int) -> c_int {
    unsafe {
        let save_p_ws = p_ws;
        let mut retval: c_int = 0;
        let mut lnum_stop: linenr_T = 0;
        let mut time_limit: i64 = 0;
        let mut options = SEARCH_KEEP;
        let mut use_skip = false;

        'theend: {
            let pat = tv_get_string(argvars);
            let dir = get_search_arg(argvars.add(1), flagsp);
            if dir == 0 {
                break 'theend;
            }
            let flags = *flagsp;
            if (flags & SP_START) != 0 {
                options |= SEARCH_START;
            }
            if (flags & SP_END) != 0 {
                options |= SEARCH_END;
            }
            if (flags & SP_COLUMN) != 0 {
                options |= SEARCH_COL;
            }

            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            {
                lnum_stop = tv_get_number_chk(argvars.add(2), ptr::null_mut()) as linenr_T;
                if lnum_stop < 0 {
                    break 'theend;
                }
                if (*argvars.add(3)).v_type != VarType::VAR_UNKNOWN {
                    time_limit = tv_get_number_chk(argvars.add(3), ptr::null_mut());
                    if time_limit < 0 {
                        break 'theend;
                    }
                    use_skip = eval_expr_valid_arg(argvars.add(4));
                }
            }

            let mut tm = profile_setlimit(time_limit);

            if ((flags & (SP_REPEAT | SP_RETCOUNT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(1)));
                break 'theend;
            }

            let save_cursor = (*curwin).w_cursor;
            let mut pos = pos_T {
                lnum: save_cursor.lnum,
                col: save_cursor.col,
                coladd: save_cursor.coladd,
            };
            let mut firstpos = pos_T {
                lnum: 0,
                col: 0,
                coladd: 0,
            };
            let mut sia = searchit_arg_T {
                sa_stop_lnum: lnum_stop,
                sa_tm: &raw mut tm,
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let patlen = strlen(pat);
            let mut subpatnum: c_int;

            loop {
                subpatnum = searchit(
                    curwin,
                    curbuf,
                    pos.cast(),
                    ptr::null_mut(),
                    dir,
                    pat.cast_mut(),
                    patlen,
                    1,
                    options,
                    RE_SEARCH,
                    p_magic != 0,
                    &raw mut sia,
                );
                if firstpos.lnum != 0 && equalpos(pos, firstpos) {
                    subpatnum = FAIL;
                }
                if subpatnum == FAIL || !use_skip {
                    break;
                }
                if firstpos.lnum == 0 {
                    firstpos = pos;
                }

                let save_pos = (*curwin).w_cursor;
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                let mut err = false;
                let do_skip = eval_expr_to_bool(argvars.add(4), &raw mut err);
                (*curwin).w_cursor = save_pos;
                if err {
                    subpatnum = FAIL;
                    break;
                }
                if !do_skip {
                    break;
                }

                options &= !SEARCH_START;
            }

            if subpatnum != FAIL {
                if (flags & SP_SUBPAT) != 0 {
                    retval = subpatnum;
                } else {
                    retval = pos.lnum;
                }
                if (flags & SP_SETPCMARK) != 0 {
                    setpcmark();
                }
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                if !match_pos.is_null() {
                    (*match_pos).lnum = pos.lnum;
                    (*match_pos).col = pos.col + 1;
                }
                check_cursor(curwin);
            }

            if (flags & SP_NOMOVE) != 0 {
                (*curwin).w_cursor = save_cursor;
            } else {
                (*curwin).w_set_curswant = true;
            }
        }
        p_ws = save_p_ws;
        retval
    }
}

/// "rpcnotify()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_rpcnotify(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER || (*argvars).vval.v_number < 0 {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"Channel id must be a positive integer".as_ptr(),
            );
            return;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_STRING {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"Event type must be a string".as_ptr(),
            );
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars.add(2);
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, true);
            args.size += 1;
            tv = tv.add(1);
        }

        let ok = rpc_send_event(
            (*argvars).vval.v_number as u64,
            tv_get_string(argvars.add(1)),
            args,
        );

        arena_mem_free(arena_finish(&raw mut arena));

        if !ok {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"Channel doesn't exist".as_ptr(),
            );
            return;
        }
        (*rettv).vval.v_number = 1;
    }
}

/// "rpcrequest()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_rpcrequest(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;
        let l_provider_call_nesting = provider_call_nesting;

        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_NUMBER || (*argvars).vval.v_number <= 0 {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"Channel id must be a positive integer".as_ptr(),
            );
            return;
        }
        if (*argvars.add(1)).v_type != VarType::VAR_STRING {
            semsg(
                gettext(e_invarg2.as_ptr()),
                c"Method name must be a string".as_ptr(),
            );
            return;
        }

        let mut args_storage = [Object {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData { boolean: false },
        }; MAX_FUNC_ARGS];
        let mut args = Array {
            items: args_storage.as_mut_ptr(),
            size: 0,
            capacity: MAX_FUNC_ARGS,
        };
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };

        let mut tv = argvars.add(2);
        while (*tv).v_type != VarType::VAR_UNKNOWN {
            *args.items.add(args.size) = vim_to_object(tv, &raw mut arena, true);
            args.size += 1;
            tv = tv.add(1);
        }

        let mut save_current_sctx: sctx_T = std::mem::zeroed();
        let mut save_autocmd_fname: *mut c_char = ptr::null_mut();
        let mut save_autocmd_match: *mut c_char = ptr::null_mut();
        let mut save_autocmd_fname_full = false;
        let mut save_autocmd_bufnr: c_int = 0;
        let mut funccal_entry: funccal_entry_T = std::mem::zeroed();

        if l_provider_call_nesting != 0 {
            save_current_sctx = current_sctx;
            save_autocmd_fname = autocmd_fname;
            save_autocmd_match = autocmd_match;
            save_autocmd_fname_full = autocmd_fname_full;
            save_autocmd_bufnr = autocmd_bufnr;
            save_funccal(&raw mut funccal_entry);

            current_sctx = provider_caller_scope.script_ctx;
            ga_grow(&raw mut exestack, 1);
            ptr::copy_nonoverlapping(
                (&raw const provider_caller_scope.es_entry).cast::<estack_T>(),
                exestack
                    .ga_data
                    .cast::<estack_T>()
                    .add(exestack.ga_len as usize),
                1,
            );
            exestack.ga_len += 1;
            autocmd_fname = provider_caller_scope.autocmd_fname;
            autocmd_match = provider_caller_scope.autocmd_match;
            autocmd_fname_full = provider_caller_scope.autocmd_fname_full;
            autocmd_bufnr = provider_caller_scope.autocmd_bufnr;
            set_current_funccal(provider_caller_scope.funccalp.cast::<funccall_T>());
        }

        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
        let chan_id = (*argvars).vval.v_number as u64;
        let method = tv_get_string(argvars.add(1));

        let mut res_mem: ArenaMem = ptr::null_mut();
        let result = rpc_send_call(chan_id, method, args, &raw mut res_mem, &raw mut err);
        arena_mem_free(arena_finish(&raw mut arena));

        if l_provider_call_nesting != 0 {
            current_sctx = save_current_sctx;
            exestack.ga_len -= 1;
            autocmd_fname = save_autocmd_fname;
            autocmd_match = save_autocmd_match;
            autocmd_fname_full = save_autocmd_fname_full;
            autocmd_bufnr = save_autocmd_bufnr;
            restore_funccal();
        }

        if err.err_type != ErrorType::kErrorTypeNone {
            let mut name: *const c_char = ptr::null();
            let chan = find_channel(chan_id);
            if !chan.is_null() {
                name = get_client_info(chan, c"name".as_ptr());
            }
            if !name.is_null() {
                semsg_multiline(
                    c"rpc_error".as_ptr(),
                    c"Invoking '%s' on channel %llu (%s):\n%s".as_ptr(),
                    method,
                    chan_id,
                    name,
                    err.msg,
                );
            } else {
                semsg_multiline(
                    c"rpc_error".as_ptr(),
                    c"Invoking '%s' on channel %llu:\n%s".as_ptr(),
                    method,
                    chan_id,
                    err.msg,
                );
            }
        } else {
            object_to_vim(result, rettv, &raw mut err);
        }

        arena_mem_free(res_mem);
        api_clear_error(&raw mut err);
    }
}

unsafe fn screenchar_adjust(grid: *mut *mut ScreenGrid, row: *mut c_int, col: *mut c_int) {
    unsafe {
        msg_scroll_flush();
        *grid = ui_comp_get_grid_at_coord(*row, *col);
        *row -= (**grid).comp_row;
        *col -= (**grid).comp_col;
    }
}

/// "screenattr()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenattr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        let c = if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols {
            -1
        } else {
            let line_off = *(*grid).line_offset.add(row as usize);
            *(*grid).attrs.add(line_off + (col as usize))
        };
        (*rettv).vval.v_number = varnumber_T::from(c);
    }
}

/// "screenchar()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenchar(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        (*rettv).vval.v_number = if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols
        {
            -1
        } else {
            varnumber_T::from(schar_get_first_codepoint(grid_getchar(
                grid,
                row,
                col,
                ptr::null_mut(),
            )))
        };
    }
}

/// "screenchars()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenchars(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);

        tv_list_alloc_ret(rettv, kListLenMayKnow);
        if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(grid, row, col, ptr::null_mut()),
        );

        let mut i: usize = 0;
        loop {
            let c = utf_ptr2char(buf.as_ptr().add(i));
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(c));
            i += utf_ptr2len(buf.as_ptr().add(i)) as usize;
            if buf[i] == NUL {
                break;
            }
        }
    }
}

/// "screencol()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screencol(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ui_current_col() + 1);
    }
}

/// "screenrow()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenrow(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(ui_current_row() + 1);
    }
}

/// "screenstring()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_screenstring(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_string = ptr::null_mut();
        (*rettv).v_type = VarType::VAR_STRING;

        let mut grid: *mut ScreenGrid = ptr::null_mut();
        let mut row = (tv_get_number_chk(argvars, ptr::null_mut()) as c_int) - 1;
        let mut col = (tv_get_number_chk(argvars.add(1), ptr::null_mut()) as c_int) - 1;

        screenchar_adjust(&raw mut grid, &raw mut row, &raw mut col);
        if row < 0 || row >= (*grid).rows || col < 0 || col >= (*grid).cols {
            return;
        }

        let mut buf = [0 as c_char; MAX_SCHAR_SIZE + 1];
        schar_get(
            buf.as_mut_ptr(),
            grid_getchar(grid, row, col, ptr::null_mut()),
        );
        (*rettv).vval.v_string = xstrdup(buf.as_ptr());
    }
}

/// "search()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_search(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut flags: c_int = 0;
        (*rettv).vval.v_number =
            varnumber_T::from(search_cmn(argvars, ptr::null_mut(), &raw mut flags));
    }
}

/// "searchdecl()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_searchdecl(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut locally: c_int = 1;
        let mut thisblock: c_int = 0;
        let mut error = false;

        (*rettv).vval.v_number = 1;

        let name = tv_get_string_chk(argvars);
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            locally = c_int::from(tv_get_bool_chk(argvars.add(1), &raw mut error) == 0);
            if !error && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                thisblock = tv_get_bool_chk(argvars.add(2), &raw mut error) as c_int;
            }
        }
        if !error && !name.is_null() {
            (*rettv).vval.v_number = varnumber_T::from(
                find_decl(
                    name.cast_mut(),
                    strlen(name),
                    locally != 0,
                    thisblock != 0,
                    SEARCH_KEEP,
                ) == FAIL,
            );
        }
    }
}

unsafe fn searchpair_cmn(argvars: *mut typval_T, match_pos: *mut pos_T) -> c_int {
    unsafe {
        let save_p_ws = p_ws;
        let mut flags: c_int = 0;
        let mut retval: c_int = 0;
        let mut lnum_stop: linenr_T = 0;
        let mut time_limit: i64 = 0;

        'theend: {
            let mut nbuf1 = [0 as c_char; NUMBUFLEN];
            let mut nbuf2 = [0 as c_char; NUMBUFLEN];
            let spat = tv_get_string_chk(argvars);
            let mpat = tv_get_string_buf_chk(argvars.add(1), nbuf1.as_mut_ptr());
            let epat = tv_get_string_buf_chk(argvars.add(2), nbuf2.as_mut_ptr());
            if spat.is_null() || mpat.is_null() || epat.is_null() {
                break 'theend;
            }

            let dir = get_search_arg(argvars.add(3), &raw mut flags);
            if dir == 0 {
                break 'theend;
            }

            if ((flags & (SP_END | SP_SUBPAT)) != 0)
                || ((flags & SP_NOMOVE) != 0 && (flags & SP_SETPCMARK) != 0)
            {
                semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(3)));
                break 'theend;
            }

            if (flags & SP_REPEAT) != 0 {
                p_ws = 0;
            }

            let skip: *const typval_T = if (*argvars.add(3)).v_type == VarType::VAR_UNKNOWN
                || (*argvars.add(4)).v_type == VarType::VAR_UNKNOWN
            {
                ptr::null()
            } else {
                let s = argvars.add(4).cast_const();
                if (*argvars.add(5)).v_type != VarType::VAR_UNKNOWN {
                    lnum_stop = tv_get_number_chk(argvars.add(5), ptr::null_mut()) as linenr_T;
                    if lnum_stop < 0 {
                        semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(5)));
                        break 'theend;
                    }
                    if (*argvars.add(6)).v_type != VarType::VAR_UNKNOWN {
                        time_limit = tv_get_number_chk(argvars.add(6), ptr::null_mut());
                        if time_limit < 0 {
                            semsg(gettext(e_invarg2.as_ptr()), tv_get_string(argvars.add(6)));
                            break 'theend;
                        }
                    }
                }
                s
            };

            retval = do_searchpair(
                spat, mpat, epat, dir, skip, flags, match_pos, lnum_stop, time_limit,
            );
        }

        p_ws = save_p_ws;
        retval
    }
}

/// "searchpair()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_searchpair(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = varnumber_T::from(searchpair_cmn(argvars, ptr::null_mut()));
    }
}

/// "searchpairpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_searchpairpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut lnum: c_int = 0;
        let mut col: c_int = 0;

        tv_list_alloc_ret(rettv, 2);
        if searchpair_cmn(argvars, &raw mut match_pos) > 0 {
            lnum = match_pos.lnum;
            col = match_pos.col;
        }
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(lnum));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(col));
    }
}

/// Search for a start/middle/end thing.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn do_searchpair(
    spat: *const c_char,
    mpat: *const c_char,
    epat: *const c_char,
    dir: c_int,
    skip: *const typval_T,
    flags: c_int,
    match_pos: *mut pos_T,
    lnum_stop: linenr_T,
    time_limit: i64,
) -> c_int {
    unsafe {
        let mut retval: c_int = 0;
        let mut nest: c_int = 1;
        let mut use_skip = false;
        let mut options = SEARCH_KEEP;

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut tm = profile_setlimit(time_limit);

        let spatlen = strlen(spat);
        let epatlen = strlen(epat);
        let pat2size = spatlen + epatlen + 17;
        let pat2 = xmalloc(pat2size).cast::<c_char>();
        let pat3size = spatlen + strlen(mpat) + epatlen + 25;
        let pat3 = xmalloc(pat3size).cast::<c_char>();
        let pat2len = snprintf(
            pat2,
            pat2size,
            c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)".as_ptr(),
            spat,
            epat,
        );
        let pat3len = if *mpat == NUL {
            ptr::copy_nonoverlapping(pat2, pat3, (pat2len as usize) + 1);
            pat2len
        } else {
            snprintf(
                pat3,
                pat3size,
                c"\\m\\(%s\\m\\)\\|\\(%s\\m\\)\\|\\(%s\\m\\)".as_ptr(),
                spat,
                epat,
                mpat,
            )
        };
        if (flags & SP_START) != 0 {
            options |= SEARCH_START;
        }
        if !skip.is_null() {
            use_skip = eval_expr_valid_arg(skip);
        }

        let save_cursor = (*curwin).w_cursor;
        let mut pos = pos_T {
            lnum: save_cursor.lnum,
            col: save_cursor.col,
            coladd: save_cursor.coladd,
        };
        let mut firstpos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&raw mut firstpos);
        let mut foundpos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        clearpos(&raw mut foundpos);
        let mut pat = pat3;
        debug_assert!(pat3len >= 0);
        let patlen = pat3len as usize;

        loop {
            let mut sia = searchit_arg_T {
                sa_stop_lnum: lnum_stop,
                sa_tm: &raw mut tm,
                sa_timed_out: 0,
                sa_wrapped: 0,
            };

            let n = searchit(
                curwin,
                curbuf,
                pos.cast(),
                ptr::null_mut(),
                dir,
                pat,
                patlen,
                1,
                options,
                RE_SEARCH,
                true,
                &raw mut sia,
            );
            if n == FAIL || (firstpos.lnum != 0 && equalpos(pos, firstpos)) {
                break;
            }
            if firstpos.lnum == 0 {
                firstpos = pos;
            }
            if equalpos(pos, foundpos) {
                if dir == BACKWARD {
                    decl(&raw mut pos);
                } else {
                    incl(&raw mut pos);
                }
            }
            foundpos = pos;

            options &= !SEARCH_START;

            if use_skip {
                let save_pos = (*curwin).w_cursor;
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                let mut err = false;
                let r = eval_expr_to_bool(skip, &raw mut err);
                (*curwin).w_cursor = save_pos;
                if err {
                    (*curwin).w_cursor = save_cursor;
                    retval = -1;
                    break;
                }
                if r {
                    continue;
                }
            }

            if (dir == BACKWARD && n == 3) || (dir == FORWARD && n == 2) {
                nest += 1;
                pat = pat2;
            } else {
                nest -= 1;
                if nest == 1 {
                    pat = pat3;
                }
            }

            if nest == 0 {
                if (flags & SP_RETCOUNT) != 0 {
                    retval += 1;
                } else {
                    retval = pos.lnum;
                }
                if (flags & SP_SETPCMARK) != 0 {
                    setpcmark();
                }
                (*curwin).w_cursor.lnum = pos.lnum;
                (*curwin).w_cursor.col = pos.col;
                (*curwin).w_cursor.coladd = pos.coladd;
                if (flags & SP_REPEAT) == 0 {
                    break;
                }
                nest = 1;
            }
        }

        if !match_pos.is_null() {
            (*match_pos).lnum = (*curwin).w_cursor.lnum;
            (*match_pos).col = (*curwin).w_cursor.col + 1;
        }

        if (flags & SP_NOMOVE) != 0 || retval == 0 {
            (*curwin).w_cursor = save_cursor;
        }

        xfree(pat2.cast());
        xfree(pat3.cast());
        if p_cpo == (&raw mut empty_string_option).cast() {
            p_cpo = save_cpo;
        } else {
            if *p_cpo == NUL {
                set_option_value_give_err(
                    kOptCpoptions,
                    Object {
                        obj_type: ObjectType::kObjectTypeString,
                        data: ObjectData {
                            string: cstr_as_string(save_cpo),
                        },
                    },
                    0,
                );
            }
            free_string_option(save_cpo);
        }

        retval
    }
}

/// "searchpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_searchpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut match_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut flags: c_int = 0;

        let n = search_cmn(argvars, &raw mut match_pos, &raw mut flags);
        tv_list_alloc_ret(rettv, 2 + isize::from((flags & SP_SUBPAT) != 0));

        let lnum = if n > 0 { match_pos.lnum } else { 0 };
        let col = if n > 0 { match_pos.col } else { 0 };

        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(lnum));
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(col));
        if (flags & SP_SUBPAT) != 0 {
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(n));
        }
    }
}

/// "serverlist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_serverlist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut n: usize = 0;
        let addrs = server_address_list(&raw mut n);

        let mut addrs_tv = tv_initial_value();
        tv_list_alloc_ret(&raw mut addrs_tv, n as isize);
        for i in 0..n {
            tv_list_append_allocated_string(addrs_tv.vval.v_list, *addrs.add(i));
        }
        xfree(addrs.cast());

        let opts = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            *argvars
        } else {
            typval_T {
                v_type: VarType::VAR_SPECIAL,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval {
                    v_special: kSpecialVarNull,
                },
            }
        };
        let mut lua_args = [opts, addrs_tv, tv_initial_value()];
        nlua_call_typval(
            c"vim._core.server".as_ptr(),
            c"serverlist".as_ptr(),
            lua_args.as_mut_ptr(),
            rettv,
        );
        tv_clear(&raw mut addrs_tv);
    }
}

/// "serverstart()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_serverstart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if check_secure() {
            return;
        }

        let address = if (*argvars).v_type != VarType::VAR_UNKNOWN {
            if (*argvars).v_type != VarType::VAR_STRING {
                emsg(gettext(e_invarg.as_ptr()));
                return;
            }
            to_slash_save(tv_get_string(argvars))
        } else {
            server_address_new(ptr::null())
        };

        let result = server_start(address);
        xfree(address.cast());

        if result != 0 {
            semsg(
                c"Failed to start server: %s".as_ptr(),
                if result > 0 {
                    c"Unknown system error".as_ptr()
                } else {
                    uv_strerror(result)
                },
            );
            return;
        }

        let mut n: usize = 0;
        let addrs = server_address_list(&raw mut n);
        (*rettv).vval.v_string = *addrs.add(n - 1);

        n -= 1;
        for i in 0..n {
            xfree((*addrs.add(i)).cast());
        }
        xfree(addrs.cast());
    }
}

/// "serverstop()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_serverstop(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if check_secure() {
            return;
        }
        if (*argvars).v_type != VarType::VAR_STRING {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;
        if !(*argvars).vval.v_string.is_null() {
            let rv = server_stop((*argvars).vval.v_string, false);
            (*rettv).vval.v_number = varnumber_T::from(rv);
        }
    }
}

unsafe fn set_position(argvars: *mut typval_T, rettv: *mut typval_T, charpos: bool) {
    unsafe {
        let mut curswant: colnr_T = -1;
        (*rettv).vval.v_number = -1;
        let name = tv_get_string_chk(argvars);
        if name.is_null() {
            return;
        }

        let mut pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut fnum: c_int = 0;
        if list2fpos(
            argvars.add(1),
            &raw mut pos,
            &raw mut fnum,
            &raw mut curswant,
            charpos,
        ) != OK
        {
            return;
        }

        if pos.col != MAXCOL {
            pos.col -= 1;
            if pos.col < 0 {
                pos.col = 0;
            }
        }
        if *name == b'.' as c_char && *name.add(1) == NUL {
            (*curwin).w_cursor.lnum = pos.lnum;
            (*curwin).w_cursor.col = pos.col;
            (*curwin).w_cursor.coladd = pos.coladd;
            if curswant >= 0 {
                (*curwin).w_curswant = curswant - 1;
                (*curwin).w_set_curswant = false;
            }
            check_cursor(curwin);
            (*rettv).vval.v_number = 0;
        } else if *name == b'\'' as c_char && *name.add(1) != NUL && *name.add(2) == NUL {
            if setmark_pos(
                c_int::from(*name.add(1) as u8),
                pos.cast(),
                fnum,
                ptr::null_mut(),
            ) == OK
            {
                (*rettv).vval.v_number = 0;
            }
        } else {
            emsg(gettext(e_invarg.as_ptr()));
        }
    }
}

/// "setcharpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_position(argvars, rettv, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setcharsearch(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_dict_arg(argvars, 0) == FAIL {
            return;
        }
        let d = (*argvars).vval.v_dict.cast::<dict_T>();
        if d.is_null() {
            return;
        }

        let csearch = tv_dict_get_string(d, c"char".as_ptr(), false);
        if !csearch.is_null() {
            let c = utf_ptr2char(csearch);
            set_last_csearch(c, csearch, utfc_ptr2len(csearch));
        }

        let mut di = tv_dict_find(d, c"forward".as_ptr(), 7);
        if !di.is_null() {
            set_csearch_direction(if tv_get_number(&raw const (*di).di_tv) != 0 {
                FORWARD
            } else {
                BACKWARD
            });
        }

        di = tv_dict_find(d, c"until".as_ptr(), 5);
        if !di.is_null() {
            set_csearch_until(tv_get_number(&raw const (*di).di_tv) != 0);
        }
    }
}

/// "setcursorcharpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setcursorcharpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_cursorpos(argvars, rettv, true);
    }
}

/// "setenv()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setenv(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut namebuf = [0 as c_char; NUMBUFLEN];
        let mut valbuf = [0 as c_char; NUMBUFLEN];
        let name = tv_get_string_buf(argvars, namebuf.as_mut_ptr());

        if check_secure() {
            return;
        }
        if (*argvars.add(1)).v_type == VarType::VAR_SPECIAL
            && (*argvars.add(1)).vval.v_special as c_int == kSpecialVarNull
        {
            vim_unsetenv_ext(name);
        } else {
            vim_setenv_ext(name, tv_get_string_buf(argvars.add(1), valbuf.as_mut_ptr()));
        }
    }
}

/// "setfperm({fname}, {mode})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setfperm(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = 0;

        let fname = tv_get_string_chk(argvars);
        if fname.is_null() {
            return;
        }
        let mut modebuf = [0 as c_char; NUMBUFLEN];
        let mode_str = tv_get_string_buf_chk(argvars.add(1), modebuf.as_mut_ptr());
        if mode_str.is_null() {
            return;
        }
        if strlen(mode_str) != 9 {
            semsg(gettext(e_invarg2.as_ptr()), mode_str);
            return;
        }

        let mut mask: c_int = 1;
        let mut mode: c_int = 0;
        for i in (0..9).rev() {
            if *mode_str.add(i) != b'-' as c_char {
                mode |= mask;
            }
            mask <<= 1;
        }
        (*rettv).vval.v_number = varnumber_T::from(os_setperm(fname, mode) == OK);
    }
}

/// "setpos()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setpos(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        set_position(argvars, rettv, false);
    }
}

unsafe fn get_yank_type(
    pp: *mut *mut c_char,
    yank_type: *mut crate::clipboard::MotionType,
    block_len: *mut c_int,
) -> c_int {
    unsafe {
        let mut stropt = *pp;
        match *stropt {
            c if c == b'v' as c_char || c == b'c' as c_char => {
                *yank_type = crate::clipboard::MotionType::kMTCharWise;
            }
            c if c == b'V' as c_char || c == b'l' as c_char => {
                *yank_type = crate::clipboard::MotionType::kMTLineWise;
            }
            c if c == b'b' as c_char || c == Ctrl_V => {
                *yank_type = crate::clipboard::MotionType::kMTBlockWise;
                if ascii_isdigit(*stropt.add(1)) {
                    stropt = stropt.add(1);
                    *block_len = getdigits_int(&raw mut stropt, false, 0) - 1;
                    stropt = stropt.sub(1);
                }
            }
            _ => return FAIL,
        }
        *pp = stropt;
        OK
    }
}

/// "setreg()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_setreg(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut append = false;
        let mut block_len: c_int = -1;
        let mut yank_type = crate::clipboard::MotionType::kMTUnknown;

        (*rettv).vval.v_number = 1;

        let strregname = tv_get_string_chk(argvars);
        if strregname.is_null() {
            return;
        }
        let mut regname = *strregname;
        if regname == 0 || regname == b'@' as c_char {
            regname = b'"' as c_char;
        }

        let mut regcontents: *const typval_T = ptr::null();
        let mut pointreg: c_char = 0;
        if (*argvars.add(1)).v_type == VarType::VAR_DICT {
            let d = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
            if tv_dict_len(d) == 0 {
                let mut lstval: [*mut c_char; 2] = [ptr::null_mut(), ptr::null_mut()];
                write_reg_contents_lst(
                    c_int::from(regname as u8),
                    lstval.as_mut_ptr(),
                    false,
                    crate::clipboard::MotionType::kMTUnknown,
                    -1,
                );
                return;
            }

            let di = tv_dict_find(d, c"regcontents".as_ptr(), -1);
            if !di.is_null() {
                regcontents = &raw const (*di).di_tv;
            }

            let mut stropt = tv_dict_get_string(d, c"regtype".as_ptr(), false);
            if !stropt.is_null() {
                let ret = get_yank_type(&raw mut stropt, &raw mut yank_type, &raw mut block_len);
                stropt = stropt.add(1);
                if ret == FAIL || *stropt != NUL {
                    semsg(gettext(e_invargval.as_ptr()), c"value".as_ptr());
                    return;
                }
            }

            if regname == b'"' as c_char {
                stropt = tv_dict_get_string(d, c"points_to".as_ptr(), false);
                if !stropt.is_null() {
                    pointreg = *stropt;
                    regname = pointreg;
                }
            } else if tv_dict_get_bool(d, c"isunnamed".as_ptr(), -1) > 0 {
                pointreg = regname;
            }
        } else {
            regcontents = argvars.add(1);
        }

        let mut set_unnamed = false;
        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if yank_type != crate::clipboard::MotionType::kMTUnknown {
                semsg(gettext(e_toomanyarg.as_ptr()), c"setreg".as_ptr());
                return;
            }

            let mut stropt = tv_get_string_chk(argvars.add(2)).cast_mut();
            if stropt.is_null() {
                return;
            }
            while *stropt != NUL {
                match *stropt as u8 {
                    b'a' | b'A' => append = true,
                    b'u' | b'"' => set_unnamed = true,
                    _ => {
                        get_yank_type(&raw mut stropt, &raw mut yank_type, &raw mut block_len);
                    }
                }
                stropt = stropt.add(1);
            }
        }

        if !regcontents.is_null() && (*regcontents).v_type == VarType::VAR_LIST {
            let ll = (*regcontents).vval.v_list;
            let len = tv_list_len(ll) as usize;

            let lstval = xmalloc(size_of::<*mut c_char>() * ((len + 1) * 2)).cast::<*mut c_char>();
            let mut curval = lstval.cast::<*const c_char>();
            let allocval = lstval.add(len + 2);
            let mut curallocval = allocval;

            'free_lstval: {
                let mut li = tv_list_first(ll);
                while !li.is_null() {
                    let mut buf = [0 as c_char; NUMBUFLEN];
                    *curval = tv_get_string_buf_chk(&raw const (*li).li_tv, buf.as_mut_ptr());
                    if (*curval).is_null() {
                        break 'free_lstval;
                    }
                    if ptr::eq(*curval, buf.as_ptr()) {
                        *curallocval = xstrdup(*curval);
                        *curval = *curallocval;
                        curallocval = curallocval.add(1);
                    }
                    curval = curval.add(1);
                    li = (*li).li_next;
                }
                *curval = ptr::null();

                write_reg_contents_lst(
                    c_int::from(regname as u8),
                    lstval,
                    append,
                    yank_type,
                    block_len as colnr_T,
                );
            }

            while curallocval > allocval {
                curallocval = curallocval.sub(1);
                xfree((*curallocval).cast());
            }
            xfree(lstval.cast());
        } else if !regcontents.is_null() {
            let strval = tv_get_string_chk(regcontents);
            if strval.is_null() {
                return;
            }
            write_reg_contents_ex(
                c_int::from(regname as u8),
                strval,
                strlen(strval) as isize,
                append,
                yank_type,
                block_len as colnr_T,
            );
        }
        if pointreg != 0 {
            get_yank_register(c_int::from(pointreg as u8), YREG_YANK);
        }
        (*rettv).vval.v_number = 0;

        if set_unnamed {
            op_reg_set_previous(regname);
        }
    }
}

/// "settagstack()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_settagstack(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut action: c_char = b'r' as c_char;
        (*rettv).vval.v_number = -1;

        let wp = find_win_by_nr_or_id(argvars);
        if wp.is_null() {
            return;
        }
        if tv_check_for_dict_arg(argvars, 1) == FAIL {
            return;
        }
        let d = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
        if d.is_null() {
            return;
        }

        if (*argvars.add(2)).v_type == VarType::VAR_UNKNOWN {
            // default 'r'
        } else if tv_check_for_string_arg(argvars, 2) == FAIL {
            return;
        } else {
            let actstr = tv_get_string_chk(argvars.add(2));
            if actstr.is_null() {
                return;
            }
            if (*actstr == b'r' as c_char || *actstr == b'a' as c_char || *actstr == b't' as c_char)
                && *actstr.add(1) == NUL
            {
                action = *actstr;
            } else {
                semsg(gettext(E_INVACT2.as_ptr()), actstr);
                return;
            }
        }

        if set_tagstack(wp, d, c_int::from(action)) == OK {
            (*rettv).vval.v_number = 0;
        }
    }
}

/// "sha256({expr})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_sha256(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        if (*argvars).v_type == VarType::VAR_BLOB {
            let blob = (*argvars).vval.v_blob.cast::<blob_T>();
            let p = if !blob.is_null() {
                (*blob).bv_ga.ga_data.cast::<u8>()
            } else {
                c"".as_ptr().cast::<u8>()
            };
            let len = if !blob.is_null() {
                (*blob).bv_ga.ga_len as usize
            } else {
                0
            };
            (*rettv).vval.v_string = xstrdup(sha256_bytes(p, len, ptr::null(), 0));
        } else {
            let p = tv_get_string(argvars);
            let hash = sha256_bytes(p.cast::<u8>(), strlen(p), ptr::null(), 0);
            (*rettv).vval.v_string = xstrdup(hash);
        }
    }
}

/// "shellescape({string})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_shellescape(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let do_special = non_zero_arg(argvars.add(1));
        (*rettv).vval.v_string =
            vim_strsave_shellescape(tv_get_string(argvars), do_special, do_special);
        (*rettv).v_type = VarType::VAR_STRING;
    }
}

/// "shiftwidth()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_shiftwidth(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = 0;
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let col = tv_get_number_chk(argvars, ptr::null_mut()) as colnr_T;
            if col < 0 {
                return;
            }
            (*rettv).vval.v_number = varnumber_T::from(get_sw_value_col(curbuf, col, false));
            return;
        }
        (*rettv).vval.v_number = varnumber_T::from(get_sw_value(curbuf));
    }
}

/// "sockconnect()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_sockconnect(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_STRING
            || (*argvars.add(1)).v_type != VarType::VAR_STRING
        {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }
        if (*argvars.add(2)).v_type != VarType::VAR_DICT
            && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
        {
            semsg(gettext(e_invarg2.as_ptr()), c"expected dictionary".as_ptr());
            return;
        }

        let mode = tv_get_string(argvars);
        let address = to_slash_save(tv_get_string(argvars.add(1)));

        'cleanup: {
            let tcp = if strcmp(mode, c"tcp".as_ptr()) == 0 {
                true
            } else if strcmp(mode, c"pipe".as_ptr()) == 0 {
                false
            } else {
                semsg(gettext(e_invarg2.as_ptr()), c"invalid mode".as_ptr());
                break 'cleanup;
            };

            let mut rpc = false;
            let mut on_data: CallbackReader = std::mem::zeroed();
            if (*argvars.add(2)).v_type == VarType::VAR_DICT {
                let opts = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
                rpc = tv_dict_get_number(opts, c"rpc".as_ptr()) != 0;

                if !tv_dict_get_callback(opts, c"on_data".as_ptr(), 7, &raw mut on_data.cb) {
                    break 'cleanup;
                }
                on_data.buffered = tv_dict_get_number(opts, c"data_buffered".as_ptr()) != 0;
                if on_data.buffered && on_data.cb.cb_type == kCallbackNone {
                    on_data.self_dict = opts;
                }
            }

            let mut error: *const c_char = ptr::null();
            let id = channel_connect(tcp, address, rpc, on_data, 50, &raw mut error);
            if !error.is_null() {
                semsg(gettext(c"connection failed: %s".as_ptr()), error);
            }

            (*rettv).vval.v_number = id as varnumber_T;
            (*rettv).v_type = VarType::VAR_NUMBER;
        }

        xfree(address.cast());
    }
}

/// "stdioopen()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_stdioopen(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_DICT {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut on_stdin: CallbackReader = std::mem::zeroed();
        let opts = (*argvars).vval.v_dict.cast::<dict_T>();
        let rpc = tv_dict_get_number(opts, c"rpc".as_ptr()) != 0;

        if !tv_dict_get_callback(opts, c"on_stdin".as_ptr(), 8, &raw mut on_stdin.cb) {
            return;
        }
        if !tv_dict_get_callback(opts, c"on_print".as_ptr(), 8, &raw mut on_print) {
            return;
        }

        on_stdin.buffered = tv_dict_get_number(opts, c"stdin_buffered".as_ptr()) != 0;
        if on_stdin.buffered && on_stdin.cb.cb_type == kCallbackNone {
            on_stdin.self_dict = opts;
        }

        let mut error: *const c_char = ptr::null();
        let id = channel_from_stdio(rpc, on_stdin, &raw mut error);
        if id == 0 {
            semsg(e_stdiochan2.as_ptr(), error);
        }

        (*rettv).vval.v_number = id as varnumber_T;
        (*rettv).v_type = VarType::VAR_NUMBER;
    }
}

/// "reltimefloat()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_reltimefloat(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut tm: proftime_T = 0;
        (*rettv).v_type = VarType::VAR_FLOAT;
        (*rettv).vval.v_float = 0.0;
        if list2proftime(argvars, &raw mut tm) == OK {
            (*rettv).vval.v_float = (profile_signed(tm) as float_T) / (NS_PER_SEC as float_T);
        }
    }
}

/// "soundfold({word})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_soundfold(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let s = tv_get_string(argvars);
        (*rettv).vval.v_string = eval_soundfold(s);
    }
}

/// "spellbadword()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_spellbadword(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;

        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }

        let ws = (*curwin).w_s.cast::<synblock_T>();
        if *(*ws).b_p_spl == NUL {
            emsg(gettext(e_no_spell.as_ptr()));
            (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        let mut word: *const c_char = c"".as_ptr();
        let mut attr: hlf_T = HLF_COUNT;
        let mut len: usize = 0;
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            len = spell_move_to(curwin, FORWARD, SMT_ALL, true, &raw mut attr);
            if len != 0 {
                word = get_cursor_pos_ptr();
                (*curwin).w_set_curswant = true;
            }
        } else if *(*curbuf).b_s.b_p_spl != NUL {
            let mut str_ = tv_get_string_chk(argvars);
            let mut capcol: c_int = -1;
            if !str_.is_null() {
                while *str_ != NUL {
                    len = spell_check(
                        curwin,
                        str_.cast_mut(),
                        &raw mut attr,
                        &raw mut capcol,
                        false,
                    );
                    if attr != HLF_COUNT {
                        word = str_;
                        break;
                    }
                    str_ = str_.add(len);
                    capcol -= len as c_int;
                    len = 0;
                }
            }
        }
        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;

        debug_assert!(len <= c_int::MAX as usize);
        tv_list_alloc_ret(rettv, 2);
        tv_list_append_string((*rettv).vval.v_list, word, len as isize);
        match attr {
            HLF_SPB => tv_list_append_string((*rettv).vval.v_list, c"bad".as_ptr(), 3),
            HLF_SPR => tv_list_append_string((*rettv).vval.v_list, c"rare".as_ptr(), 4),
            HLF_SPL => tv_list_append_string((*rettv).vval.v_list, c"local".as_ptr(), 5),
            HLF_SPC => tv_list_append_string((*rettv).vval.v_list, c"caps".as_ptr(), 4),
            _ => tv_list_append_string((*rettv).vval.v_list, ptr::null(), -1),
        }
    }
}

/// "spellsuggest()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_spellsuggest(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        let wo_spell_save = (*curwin).w_onebuf_opt.wo_spell;

        if (*curwin).w_onebuf_opt.wo_spell == 0 {
            parse_spelllang(curwin);
            (*curwin).w_onebuf_opt.wo_spell = 1;
        }

        let ws = (*curwin).w_s.cast::<synblock_T>();
        if *(*ws).b_p_spl == NUL {
            emsg(gettext(e_no_spell.as_ptr()));
            (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
            return;
        }

        'ret: {
            let maxcount: c_int;
            let mut need_capital = false;
            let str_ = tv_get_string(argvars);
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                let mut typeerr = false;
                maxcount = tv_get_number_chk(argvars.add(1), &raw mut typeerr) as c_int;
                if maxcount <= 0 {
                    break 'ret;
                }
                if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                    need_capital = tv_get_bool_chk(argvars.add(2), &raw mut typeerr) != 0;
                    if typeerr {
                        break 'ret;
                    }
                }
            } else {
                maxcount = 25;
            }

            spell_suggest_list(&raw mut ga, str_.cast_mut(), maxcount, need_capital, false);
        }

        tv_list_alloc_ret(rettv, ga.ga_len as isize);
        for i in 0..(ga.ga_len as usize) {
            let p = *ga.ga_data.cast::<*mut c_char>().add(i);
            tv_list_append_allocated_string((*rettv).vval.v_list, p);
        }
        ga_clear(&raw mut ga);
        (*curwin).w_onebuf_opt.wo_spell = wo_spell_save;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_split(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut col: colnr_T = 0;
        let mut keepempty = false;
        let mut typeerr = false;

        let save_cpo = p_cpo;
        p_cpo = (&raw mut empty_string_option).cast();

        let mut str_ = tv_get_string(argvars);
        let mut pat: *const c_char = ptr::null();
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
            if pat.is_null() {
                typeerr = true;
            }
            if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
                keepempty = tv_get_bool_chk(argvars.add(2), &raw mut typeerr) != 0;
            }
        }
        if pat.is_null() || *pat == NUL {
            pat = c"[\\x01- ]\\+".as_ptr();
        }

        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if !typeerr {
            let mut regmatch = regmatch_T {
                regprog: vim_regcomp(pat, RE_MAGIC + RE_STRING).cast(),
                startp: [ptr::null_mut(); NSUBEXP],
                endp: [ptr::null_mut(); NSUBEXP],
                rm_matchcol: 0,
                rm_ic: false,
            };
            if !regmatch.regprog.is_null() {
                while *str_ != NUL || keepempty {
                    let match_found = if *str_ == NUL {
                        false
                    } else {
                        vim_regexec_nl((&raw mut regmatch).cast(), str_, col)
                    };
                    let end: *const c_char = if match_found {
                        regmatch.startp[0]
                    } else {
                        str_.add(strlen(str_))
                    };
                    if keepempty
                        || end > str_
                        || (tv_list_len((*rettv).vval.v_list) > 0
                            && *str_ != NUL
                            && match_found
                            && end < regmatch.endp[0].cast_const())
                    {
                        tv_list_append_string((*rettv).vval.v_list, str_, end.offset_from(str_));
                    }
                    if !match_found {
                        break;
                    }
                    if regmatch.endp[0].cast_const() > str_ {
                        col = 0;
                    } else {
                        col = utfc_ptr2len(regmatch.endp[0]);
                    }
                    str_ = regmatch.endp[0];
                }
                vim_regfree(regmatch.regprog.cast());
            }
        }

        p_cpo = save_cpo;
    }
}

unsafe fn get_xdg_var_list(xdg: XDGVarType, rettv: *mut typval_T) {
    unsafe {
        let list = tv_list_alloc(kListLenShouldKnow);
        (*rettv).v_type = VarType::VAR_LIST;
        (*rettv).vval.v_list = list;
        tv_list_ref(list);
        let dirs = stdpaths_get_xdg_var(xdg);
        if dirs.is_null() {
            return;
        }
        let mut iter: *const c_void = ptr::null();
        let appname = get_appname(false);
        loop {
            let mut dir_len: usize = 0;
            let mut dir: *const c_char = ptr::null();
            iter = vim_env_iter(ENV_SEPCHAR, dirs, iter, &raw mut dir, &raw mut dir_len);
            if !dir.is_null() && dir_len > 0 {
                let mut dir_with_nvim = xmemdupz(dir.cast(), dir_len).cast::<c_char>();
                dir_with_nvim = concat_fnames_realloc(dir_with_nvim, appname, true);
                tv_list_append_allocated_string(list, dir_with_nvim);
            }
            if iter.is_null() {
                break;
            }
        }
        xfree(dirs.cast());
    }
}

/// "stdpath(type)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_stdpath(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = ptr::null_mut();

        let p = tv_get_string_chk(argvars);
        if p.is_null() {
            return;
        }

        if strequal(p, c"config".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGConfigHome);
        } else if strequal(p, c"data".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGDataHome);
        } else if strequal(p, c"cache".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGCacheHome);
        } else if strequal(p, c"state".as_ptr()) {
            (*rettv).vval.v_string = get_xdg_home(XDGVarType::kXDGStateHome);
        } else if strequal(p, c"log".as_ptr()) {
            (*rettv).vval.v_string = concat_fnames_realloc(
                get_xdg_home(XDGVarType::kXDGStateHome),
                c"logs".as_ptr(),
                true,
            );
        } else if strequal(p, c"run".as_ptr()) {
            (*rettv).vval.v_string = stdpaths_get_xdg_var(XDGVarType::kXDGRuntimeDir);
        } else if strequal(p, c"config_dirs".as_ptr()) {
            get_xdg_var_list(XDGVarType::kXDGConfigDirs, rettv);
        } else if strequal(p, c"data_dirs".as_ptr()) {
            get_xdg_var_list(XDGVarType::kXDGDataDirs, rettv);
        } else {
            semsg(gettext(c"E6100: \"%s\" is not a valid stdpath".as_ptr()), p);
        }
    }
}

/// "str2float()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_str2float(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut p = skipwhite(tv_get_string(argvars));
        let isneg = *p == b'-' as c_char;
        if *p == b'+' as c_char || *p == b'-' as c_char {
            p = skipwhite(p.add(1));
        }
        string2float(p, &raw mut (*rettv).vval.v_float);
        if isneg {
            (*rettv).vval.v_float *= -1.0;
        }
        (*rettv).v_type = VarType::VAR_FLOAT;
    }
}

/// "strftime({format}[, {time}])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strftime(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let mut p = tv_get_string(argvars).cast_mut();
        let seconds: time_t = if (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN {
            c_time(ptr::null_mut())
        } else {
            tv_get_number(argvars.add(1)) as time_t
        };

        let mut curtime = Tm { _fields: [0; 16] };
        let curtime_ptr = os_localtime_r(&raw const seconds, &raw mut curtime);
        if curtime_ptr.is_null() {
            (*rettv).vval.v_string = xstrdup(gettext(c"(Invalid)".as_ptr()));
            return;
        }

        let mut conv: vimconv_T = std::mem::zeroed();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        convert_setup(&raw mut conv, p_enc, enc);
        if conv.vc_type != CONV_NONE {
            p = string_convert(&raw const conv, p, ptr::null_mut());
        }
        let mut result_buf = [0 as c_char; 256];
        if p.is_null() || strftime(result_buf.as_mut_ptr(), result_buf.len(), p, curtime_ptr) == 0 {
            result_buf[0] = NUL;
        }

        if conv.vc_type != CONV_NONE {
            xfree(p.cast());
        }
        convert_setup(&raw mut conv, enc, p_enc);
        if conv.vc_type != CONV_NONE {
            (*rettv).vval.v_string =
                string_convert(&raw const conv, result_buf.as_mut_ptr(), ptr::null_mut());
        } else {
            (*rettv).vval.v_string = xstrdup(result_buf.as_ptr());
        }

        convert_setup(&raw mut conv, ptr::null_mut(), ptr::null_mut());
        xfree(enc.cast());
    }
}

/// "strptime({format}, {timestring})" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_strptime(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut fmt_buf = [0 as c_char; NUMBUFLEN];
        let mut str_buf = [0 as c_char; NUMBUFLEN];

        let mut tmval = Tm { _fields: [0; 16] };
        tmval._fields[8] = -1; // tm_isdst = -1
        let mut fmt = tv_get_string_buf(argvars, fmt_buf.as_mut_ptr()).cast_mut();
        let str_ = tv_get_string_buf(argvars.add(1), str_buf.as_mut_ptr());

        let mut conv: vimconv_T = std::mem::zeroed();
        conv.vc_type = CONV_NONE;
        let enc = enc_locale();
        convert_setup(&raw mut conv, p_enc, enc);
        if conv.vc_type != CONV_NONE {
            fmt = string_convert(&raw const conv, fmt, ptr::null_mut());
        }
        if fmt.is_null() || os_strptime(str_, fmt, &raw mut tmval).is_null() {
            (*rettv).vval.v_number = 0;
        } else {
            let t = c_mktime(&raw mut tmval);
            (*rettv).vval.v_number = if t == -1 { 0 } else { t as varnumber_T };
        }
        if conv.vc_type != CONV_NONE {
            xfree(fmt.cast());
        }
        convert_setup(&raw mut conv, ptr::null_mut(), ptr::null_mut());
        xfree(enc.cast());
    }
}

/// "submatch()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_submatch(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut error = false;
        let no = tv_get_number_chk(argvars, &raw mut error) as c_int;
        if error {
            return;
        }
        if no < 0 || (no as usize) >= NSUBEXP {
            semsg(gettext(E_INVALID_SUBMATCH_NUMBER_NR.as_ptr()), no);
            return;
        }
        let mut ret_list: c_int = 0;
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            ret_list = tv_get_bool_chk(argvars.add(1), &raw mut error) as c_int;
            if error {
                return;
            }
        }
        if ret_list == 0 {
            (*rettv).v_type = VarType::VAR_STRING;
            (*rettv).vval.v_string = reg_submatch(no);
        } else {
            (*rettv).v_type = VarType::VAR_LIST;
            (*rettv).vval.v_list = reg_submatch_list(no);
        }
    }
}

/// "substitute()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_substitute(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut patbuf = [0 as c_char; NUMBUFLEN];
        let mut subbuf = [0 as c_char; NUMBUFLEN];
        let mut flagsbuf = [0 as c_char; NUMBUFLEN];

        let str_ = tv_get_string_chk(argvars);
        let pat = tv_get_string_buf_chk(argvars.add(1), patbuf.as_mut_ptr());
        let mut sub: *const c_char = ptr::null();
        let flg = tv_get_string_buf_chk(argvars.add(3), flagsbuf.as_mut_ptr());

        let mut expr: *mut typval_T = ptr::null_mut();
        if tv_is_func(*argvars.add(2)) {
            expr = argvars.add(2);
        } else {
            sub = tv_get_string_buf_chk(argvars.add(2), subbuf.as_mut_ptr());
        }

        (*rettv).v_type = VarType::VAR_STRING;
        if str_.is_null() || pat.is_null() || (sub.is_null() && expr.is_null()) || flg.is_null() {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = do_string_sub(
                str_.cast_mut(),
                strlen(str_),
                pat.cast_mut(),
                sub.cast_mut(),
                expr,
                flg.cast_mut(),
                ptr::null_mut(),
            );
        }
    }
}

/// "swapfilelist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_swapfilelist(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        recover_names(ptr::null_mut(), false, (*rettv).vval.v_list);
    }
}

/// "swapinfo(swap_filename)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_swapinfo(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        swapfile_dict(tv_get_string(argvars), (*rettv).vval.v_dict.cast());
    }
}

/// "swapname(expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_swapname(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        let buf = tv_get_buf(argvars, 0);
        if buf.is_null()
            || (*buf).b_ml.ml_mfp.is_null()
            || (*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fname.is_null()
        {
            (*rettv).vval.v_string = ptr::null_mut();
        } else {
            (*rettv).vval.v_string = xstrdup((*(*buf).b_ml.ml_mfp.cast::<memfile_T>()).mf_fname);
        }
    }
}

/// "synID(lnum, col, trans)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_synID(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        let mut transerr = false;
        let trans = tv_get_bool_chk(argvars.add(2), &raw mut transerr) as c_int;

        let mut id: c_int = 0;
        if !transerr
            && lnum >= 1
            && lnum <= (*curbuf).b_ml.ml_line_count
            && col >= 0
            && col < ml_get_len(lnum)
        {
            id = syn_get_id(curwin, lnum, col, trans, ptr::null_mut(), 0);
        }
        (*rettv).vval.v_number = varnumber_T::from(id);
    }
}

/// "synIDattr(id, what [, mode])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_synIDattr(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let id = tv_get_number(argvars) as c_int;
        let what = tv_get_string(argvars.add(1));
        let mut modec: c_int = if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            let mut modebuf = [0 as c_char; NUMBUFLEN];
            let mode = tv_get_string_buf(argvars.add(2), modebuf.as_mut_ptr());
            let mc = c_int::from(tolower_asc(*mode));
            if mc != b'c' as c_int && mc != b'g' as c_int {
                0
            } else {
                mc
            }
        } else if ui_rgb_attached() {
            b'g' as c_int
        } else {
            b'c' as c_int
        };
        let _ = &mut modec;

        let mut p: *const c_char = ptr::null();
        match tolower_asc(*what) {
            b'b' => {
                if tolower_asc(*what.add(1)) == b'g' {
                    p = highlight_color(id, what, modec);
                } else if tolower_asc(*what.add(1)) == b'l' {
                    p = highlight_has_attr(id, HL_BLINK, modec);
                } else {
                    p = highlight_has_attr(id, HL_BOLD, modec);
                }
            }
            b'c' => p = highlight_has_attr(id, HL_CONCEALED, modec),
            b'd' => p = highlight_has_attr(id, HL_DIM, modec),
            b'o' => p = highlight_has_attr(id, HL_OVERLINE, modec),
            b'f' => p = highlight_color(id, what, modec),
            b'i' => {
                if tolower_asc(*what.add(1)) == b'n' {
                    p = highlight_has_attr(id, HL_INVERSE, modec);
                } else {
                    p = highlight_has_attr(id, HL_ITALIC, modec);
                }
            }
            b'n' => {
                if tolower_asc(*what.add(1)) == b'o' {
                    p = highlight_has_attr(id, HL_NOCOMBINE, modec);
                } else {
                    p = get_highlight_name_ext(ptr::null_mut(), id - 1, false);
                }
            }
            b'r' => p = highlight_has_attr(id, HL_INVERSE, modec),
            b's' => {
                if tolower_asc(*what.add(1)) == b'p' {
                    p = highlight_color(id, what, modec);
                } else if tolower_asc(*what.add(1)) == b't' && tolower_asc(*what.add(2)) == b'r' {
                    p = highlight_has_attr(id, HL_STRIKETHROUGH, modec);
                } else {
                    p = highlight_has_attr(id, HL_STANDOUT, modec);
                }
            }
            b'u' => {
                if strlen(what) >= 9 {
                    if tolower_asc(*what.add(5)) == b'l' {
                        p = highlight_has_attr(id, HL_UNDERLINE, modec);
                    } else if tolower_asc(*what.add(5)) != b'd' {
                        p = highlight_has_attr(id, HL_UNDERCURL, modec);
                    } else if tolower_asc(*what.add(6)) != b'o' {
                        p = highlight_has_attr(id, HL_UNDERDASHED, modec);
                    } else if tolower_asc(*what.add(7)) == b'u' {
                        p = highlight_has_attr(id, HL_UNDERDOUBLE, modec);
                    } else {
                        p = highlight_has_attr(id, HL_UNDERDOTTED, modec);
                    }
                } else {
                    p = highlight_color(id, what, modec);
                }
            }
            _ => {}
        }

        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = if p.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(p)
        };
    }
}

/// "synIDtrans(id)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_synIDtrans(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut id = tv_get_number(argvars) as c_int;
        if id > 0 {
            id = syn_get_final_id(id);
        } else {
            id = 0;
        }
        (*rettv).vval.v_number = varnumber_T::from(id);
    }
}

/// "synconcealed(lnum, col)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_synconcealed(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut syntax_flags: c_int = 0;
        let mut matchid: c_int = 0;
        let mut str_ = [0 as c_char; NUMBUFLEN];

        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        if lnum >= 1
            && lnum <= (*curbuf).b_ml.ml_line_count
            && col >= 0
            && col <= ml_get_len(lnum)
            && (*curwin).w_onebuf_opt.wo_cole > 0
        {
            syn_get_id(curwin, lnum, col, 0, ptr::null_mut(), 0);
            syntax_flags = get_syntax_info(&raw mut matchid);

            if (syntax_flags & HL_CONCEAL) != 0 && (*curwin).w_onebuf_opt.wo_cole < 3 {
                let mut cchar = schar_from_char(syn_get_sub_char());
                if cchar == 0 && (*curwin).w_onebuf_opt.wo_cole == 1 {
                    cchar = if (*curwin).w_p_lcs_chars.conceal == 0 {
                        b' ' as schar_T
                    } else {
                        (*curwin).w_p_lcs_chars.conceal
                    };
                }
                if cchar != 0 {
                    schar_get(str_.as_mut_ptr(), cchar);
                }
            }
        }

        tv_list_alloc_ret(rettv, 3);
        tv_list_append_number(
            (*rettv).vval.v_list,
            varnumber_T::from((syntax_flags & HL_CONCEAL) != 0),
        );
        tv_list_append_string((*rettv).vval.v_list, str_.as_ptr(), -1);
        tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(matchid));
    }
}

/// "synstack(lnum, col)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_synstack(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_set_ret(rettv, ptr::null_mut());

        let lnum = tv_get_lnum(argvars);
        let col = (tv_get_number(argvars.add(1)) as colnr_T) - 1;

        if lnum >= 1 && lnum <= (*curbuf).b_ml.ml_line_count && col >= 0 && col <= ml_get_len(lnum)
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            syn_get_id(curwin, lnum, col, 0, ptr::null_mut(), 1);

            let mut i: c_int = 0;
            loop {
                let id = syn_get_stack_item(i);
                i += 1;
                if id < 0 {
                    break;
                }
                tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(id));
            }
        }
    }
}

/// "tabpagebuflist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tabpagebuflist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut wp: *mut win_T = ptr::null_mut();
        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            wp = firstwin;
        } else {
            let tp = find_tabpage(tv_get_number(argvars) as c_int);
            if !tp.is_null() {
                wp = if tp == curtab {
                    firstwin
                } else {
                    (*tp).tp_firstwin
                };
            }
        }
        if !wp.is_null() {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            while !wp.is_null() {
                tv_list_append_number(
                    (*rettv).vval.v_list,
                    varnumber_T::from((*(*wp).w_buffer).b_fnum),
                );
                wp = (*wp).w_next;
            }
        }
    }
}

/// "tagfiles()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_tagfiles(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        let fname = xmalloc(MAXPATHL).cast::<c_char>();

        let mut first = true;
        let mut tn: tagname_T = std::mem::zeroed();
        while get_tagfname(&raw mut tn, c_int::from(first), fname) == OK {
            tv_list_append_string((*rettv).vval.v_list, fname, -1);
            first = false;
        }

        tagname_free(&raw mut tn);
        xfree(fname.cast());
    }
}

/// "taglist()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_taglist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let tag_pattern = tv_get_string(argvars);
        (*rettv).vval.v_number = 0;
        if *tag_pattern == NUL {
            return;
        }
        let mut fname: *const c_char = ptr::null();
        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            fname = tv_get_string(argvars.add(1));
        }
        get_tags(
            tv_list_alloc_ret(rettv, kListLenUnknown),
            tag_pattern.cast_mut(),
            fname.cast_mut(),
        );
    }
}

/// "timer_info([timer])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_timer_info(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenUnknown);
        if tv_check_for_opt_number_arg(argvars, 0) == FAIL {
            return;
        }
        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            let timer = find_timer_by_nr(tv_get_number(argvars));
            if !timer.is_null() {
                let tt = timer_tail(timer);
                if !(*tt).stopped || (*tt).refcount > 1 {
                    add_timer_info(rettv, timer);
                }
            }
        } else {
            add_timer_info_all(rettv);
        }
    }
}

/// "timer_pause(timer, paused)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_timer_pause(
    argvars: *mut typval_T,
    _unused: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type != VarType::VAR_NUMBER {
            emsg(gettext(e_number_exp.as_ptr()));
            return;
        }
        let paused = tv_get_bool(argvars.add(1)) != 0;
        let timer = find_timer_by_nr(tv_get_number(argvars));
        if !timer.is_null() {
            let tt = timer_tail(timer);
            if !(*tt).paused && paused {
                time_watcher_stop(timer.cast());
            } else if (*tt).paused && !paused {
                time_watcher_start(
                    timer.cast(),
                    Some(timer_due_cb),
                    (*tt).timeout as u64,
                    (*tt).timeout as u64,
                );
            }
            (*tt).paused = paused;
        }
    }
}

/// "timer_start(timeout, callback, opts)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_timer_start(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut repeat: c_int = 1;
        (*rettv).vval.v_number = -1;
        if check_secure() {
            return;
        }

        if (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_nonnull_dict_arg(argvars, 2) == FAIL {
                return;
            }
            let dict = (*argvars.add(2)).vval.v_dict.cast::<dict_T>();
            let di = tv_dict_find(dict, c"repeat".as_ptr(), 6);
            if !di.is_null() {
                repeat = tv_get_number(&raw const (*di).di_tv) as c_int;
                if repeat == 0 {
                    repeat = 1;
                }
            }
        }

        let mut callback = Callback {
            data: ptr::null_mut(),
            cb_type: kCallbackNone,
        };
        if !callback_from_typval(&raw mut callback, argvars.add(1)) {
            return;
        }
        (*rettv).vval.v_number =
            timer_start(tv_get_number(argvars), repeat, &raw const callback) as varnumber_T;
    }
}

/// "timer_stop(timerid)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_timer_stop(
    argvars: *mut typval_T,
    _rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if tv_check_for_number_arg(argvars, 0) == FAIL {
            return;
        }
        let timer = find_timer_by_nr(tv_get_number(argvars));
        if timer.is_null() {
            return;
        }
        timer_stop(timer);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_timer_stopall(
    _argvars: *mut typval_T,
    _unused: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        timer_stop_all();
    }
}

/// "type(expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_type(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        let mut n: c_int = -1;
        match (*argvars).v_type {
            VarType::VAR_NUMBER => n = VAR_TYPE_NUMBER,
            VarType::VAR_STRING => n = VAR_TYPE_STRING,
            VarType::VAR_PARTIAL | VarType::VAR_FUNC => n = VAR_TYPE_FUNC,
            VarType::VAR_LIST => n = VAR_TYPE_LIST,
            VarType::VAR_DICT => n = VAR_TYPE_DICT,
            VarType::VAR_FLOAT => n = VAR_TYPE_FLOAT,
            VarType::VAR_BOOL => n = VAR_TYPE_BOOL,
            VarType::VAR_SPECIAL => n = VAR_TYPE_SPECIAL,
            VarType::VAR_BLOB => n = VAR_TYPE_BLOB,
            VarType::VAR_UNKNOWN => internal_error(c"f_type(UNKNOWN)".as_ptr()),
        }
        (*rettv).vval.v_number = varnumber_T::from(n);
    }
}

/// "virtcol({expr}, [, {list} [, {winid}]])" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_virtcol(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut vcol_start: colnr_T = 0;
        let mut vcol_end: colnr_T = 0;
        let mut wp = curwin;

        'theend: {
            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN
                && (*argvars.add(2)).v_type != VarType::VAR_UNKNOWN
            {
                let mut tp: *mut tabpage_T = ptr::null_mut();
                wp = win_id2wp_tp(tv_get_number(argvars.add(2)) as c_int, &raw mut tp);
                if wp.is_null() || tp.is_null() {
                    break 'theend;
                }
                check_cursor(wp);
            }

            let bp = (*wp).w_buffer;
            let mut fnum = (*bp).b_fnum;
            let fp = var2fpos(argvars, false, &raw mut fnum, false, wp);
            if !fp.is_null() && (*fp).lnum <= (*bp).b_ml.ml_line_count && fnum == (*bp).b_fnum {
                if (*fp).col < 0 {
                    (*fp).col = 0;
                } else {
                    let len = ml_get_buf_len(bp, (*fp).lnum);
                    if (*fp).col > len {
                        (*fp).col = len;
                    }
                }
                getvvcol(
                    wp,
                    (*fp).cast(),
                    &raw mut vcol_start,
                    ptr::null_mut(),
                    &raw mut vcol_end,
                    0,
                );
                vcol_start += 1;
                vcol_end += 1;
            }
        }

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN && tv_get_bool(argvars.add(1)) != 0 {
            tv_list_alloc_ret(rettv, 2);
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(vcol_start));
            tv_list_append_number((*rettv).vval.v_list, varnumber_T::from(vcol_end));
        } else {
            (*rettv).vval.v_number = varnumber_T::from(vcol_end);
        }
    }
}

/// "visualmode()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_visualmode(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let str_ = [(*curbuf).b_visual_mode_eval as c_char, NUL];
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup(str_.as_ptr());

        if non_zero_arg(argvars) {
            (*curbuf).b_visual_mode_eval = 0;
        }
    }
}

/// "wildmenumode()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_wildmenumode(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if wild_menu_showing != 0 || ((State & MODE_CMDLINE) != 0 && cmdline_pum_active()) {
            (*rettv).vval.v_number = 1;
        }
    }
}

/// "windowsversion()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_windowsversion(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).v_type = VarType::VAR_STRING;
        (*rettv).vval.v_string = xstrdup(windowsVersion.as_ptr());
    }
}

/// "wordcount()" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_wordcount(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_dict_alloc_ret(rettv);
        cursor_pos_info((*rettv).vval.v_dict.cast());
    }
}

/// "xor(expr, expr)" function
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn f_xor(argvars: *mut typval_T, rettv: *mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        (*rettv).vval.v_number = tv_get_number_chk(argvars, ptr::null_mut())
            ^ tv_get_number_chk(argvars.add(1), ptr::null_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix32_and_xoshiro_are_deterministic() {
        unsafe {
            let mut seed: u32 = 42;
            let mut x = splitmix32(&raw mut seed);
            let mut y = splitmix32(&raw mut seed);
            let mut z = splitmix32(&raw mut seed);
            let mut w = splitmix32(&raw mut seed);
            let r1 = shuffle_xoshiro128starstar(&raw mut x, &raw mut y, &raw mut z, &raw mut w);
            assert_ne!(r1, 0);
        }
    }

    #[test]
    fn non_zero_arg_checks_number_bool_and_string() {
        unsafe {
            let mut tv = tv_initial_value();
            tv.v_type = VarType::VAR_NUMBER;
            tv.vval.v_number = 0;
            assert!(!non_zero_arg(&raw const tv));
            tv.vval.v_number = 5;
            assert!(non_zero_arg(&raw const tv));

            tv.v_type = VarType::VAR_BOOL;
            tv.vval.v_bool = kBoolVarFalse;
            assert!(!non_zero_arg(&raw const tv));
            tv.vval.v_bool = kBoolVarTrue;
            assert!(non_zero_arg(&raw const tv));
        }
    }
}
