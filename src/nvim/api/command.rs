//! Command API functions (`src/nvim/api/command.c`).

use crate::api::deprecated::KeyDict_empty;
use crate::api::private::dispatch::{
    KeyDict_cmd_magic_get_field, KeyDict_cmd_mods_filter_get_field, KeyDict_cmd_mods_get_field,
};
use crate::api::private::helpers::{
    add_c, api_dict_to_keydict, api_set_error, api_typename, arena_array, arena_dict, arena_string,
    boolean_obj, cbuf_to_arena_str, cstr_as_string, cstrn_as_string, dict_obj, error_set,
    find_buffer_by_handle, has_key, integer_obj, put_c, static_cstr_as_string, string_obj,
    string_to_cstr, try_wrap, with_script_context,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType, api_err_exp, api_err_invalid,
    api_err_required,
};
use crate::api::tabpage::Buffer;
use crate::autocmd::{EVENT_CMDUNDEFINED, apply_autocmds, has_event};
use crate::charset::{NUL, ascii_isupper, ascii_iswhite, getdigits_long, skiptowhite, skipwhite};
use crate::ex_docmd::{
    ADDR_ARGUMENTS, ADDR_BUFFERS, ADDR_LINES, ADDR_LOADED_BUFFERS, ADDR_NONE, ADDR_OTHER,
    ADDR_QUICKFIX, ADDR_TABS, ADDR_WINDOWS, CMD_SIZE, CMD_USER, CMD_USER_BUF, CMOD_BROWSE,
    CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS,
    CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE, CMOD_SANDBOX, CMOD_SILENT,
    CMOD_UNSILENT, EX_ARGOPT, EX_ARGSPACE, EX_BANG, EX_CMDARG, EX_COUNT, EX_DFLALL, EX_EXTRA,
    EX_KEEPSCRIPT, EX_NEEDARG, EX_NOSPC, EX_PREVIEW, EX_RANGE, EX_REGSTR, EX_SBOXOK, EX_TRLBAR,
    EX_XFILE, EX_ZEROR, cmdmod_T, exarg_T, excmd_get_argt, execute_cmd, find_ex_command,
    get_command_name, getargcmd, getargopt, invalid_range, is_cmd_ni, is_map_cmd, parse_cmdline,
    replace_makeprg, set_cmd_count, undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{Boolean, LUA_NOREF, LuaRef};
use crate::lua::executor::{api_free_luaref, api_new_luaref};
use crate::mbyte::mb_islower;
use crate::memory::{Arena, arena_alloc, arena_memdupz, strequal_str, xfree, xmalloc, xmemdupz};
use crate::msgpack_rpc::packer::Integer;
use crate::regexp::{RE_MAGIC, vim_regcomp};
use crate::register::valid_yank_reg;
use crate::usercmd::{
    UC_BUFFER, commands_array, get_user_command_name, parse_addr_type_arg, parse_compl_arg,
    uc_add_command, uc_nargs_upper_bound, uc_split_args_iter, uc_validate_name, ucmds,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::cmdexpand::{EXPAND_NOTHING, EXPAND_USER_LUA, cmd_addr_T};
use crate::cursor::linenr_T;

const CMD_iput: c_int = 200;
const CMD_put: c_int = 350;

const KEYSET_OPTIDX_cmd__cmd: u64 = 1;
const KEYSET_OPTIDX_cmd__reg: u64 = 2;
const KEYSET_OPTIDX_cmd__bang: u64 = 3;
const KEYSET_OPTIDX_cmd__addr: u64 = 4;
const KEYSET_OPTIDX_cmd__mods: u64 = 5;
const KEYSET_OPTIDX_cmd__args: u64 = 6;
const KEYSET_OPTIDX_cmd__count: u64 = 7;
const KEYSET_OPTIDX_cmd__magic: u64 = 8;
const KEYSET_OPTIDX_cmd__nargs: u64 = 9;
const KEYSET_OPTIDX_cmd__range: u64 = 10;
const KEYSET_OPTIDX_cmd__nextcmd: u64 = 11;

const KEYSET_OPTIDX_cmd_magic__bar: u64 = 1;
const KEYSET_OPTIDX_cmd_magic__file: u64 = 2;

const KEYSET_OPTIDX_cmd_mods__tab: u64 = 1;
const KEYSET_OPTIDX_cmd_mods__split: u64 = 3;
const KEYSET_OPTIDX_cmd_mods__filter: u64 = 5;
const KEYSET_OPTIDX_cmd_mods__verbose: u64 = 10;

const KEYSET_OPTIDX_cmd_mods_filter__pattern: u64 = 2;

const KEYSET_OPTIDX_user_command__addr: u64 = 2;
const KEYSET_OPTIDX_user_command__desc: u64 = 4;
const KEYSET_OPTIDX_user_command__count: u64 = 5;
const KEYSET_OPTIDX_user_command__force: u64 = 6;
const KEYSET_OPTIDX_user_command__nargs: u64 = 7;
const KEYSET_OPTIDX_user_command__range: u64 = 8;
const KEYSET_OPTIDX_user_command__preview: u64 = 9;
const KEYSET_OPTIDX_user_command__complete: u64 = 10;

use crate::math::{FAIL, OK};
use crate::window::{WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT};

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd {
    pub is_set__cmd_: OptionalKeys,
    pub cmd: NvimString,
    pub range: Array,
    pub count: Integer,
    pub reg: NvimString,
    pub bang: Boolean,
    pub args: Array,
    pub magic: Dict,
    pub mods: Dict,
    pub nargs: Object,
    pub addr: NvimString,
    pub nextcmd: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd_magic {
    pub is_set__cmd_magic_: OptionalKeys,
    pub file: Boolean,
    pub bar: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd_mods {
    pub is_set__cmd_mods_: OptionalKeys,
    pub silent: Boolean,
    pub emsg_silent: Boolean,
    pub unsilent: Boolean,
    pub filter: Dict,
    pub sandbox: Boolean,
    pub noautocmd: Boolean,
    pub browse: Boolean,
    pub confirm: Boolean,
    pub hide: Boolean,
    pub horizontal: Boolean,
    pub keepalt: Boolean,
    pub keepjumps: Boolean,
    pub keepmarks: Boolean,
    pub keeppatterns: Boolean,
    pub lockmarks: Boolean,
    pub noswapfile: Boolean,
    pub tab: Integer,
    pub verbose: Integer,
    pub vertical: Boolean,
    pub split: NvimString,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd_mods_filter {
    pub is_set__cmd_mods_filter_: OptionalKeys,
    pub pattern: NvimString,
    pub force: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd_opts {
    pub output: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_user_command {
    pub is_set__user_command_: OptionalKeys,
    pub addr: Object,
    pub bang: Boolean,
    pub bar: Boolean,
    pub complete: Object,
    pub count: Object,
    pub desc: Object,
    pub force: Boolean,
    pub keepscript: Boolean,
    pub nargs: Object,
    pub preview: Object,
    pub range: Object,
    pub register_: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_get_commands {
    pub builtin: Boolean,
}

#[inline]
unsafe fn cstr_to_arena_str(arena: *mut Arena, str_ptr: *const c_char) -> NvimString {
    unsafe {
        if str_ptr.is_null() {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        } else {
            let len = CStr::from_ptr(str_ptr).to_bytes().len();
            cbuf_to_arena_str(arena, str_ptr, len)
        }
    }
}

pub trait CmdIdxVal {
    fn check_user_cmd(self) -> bool;
}

impl CmdIdxVal for c_int {
    #[inline]
    fn check_user_cmd(self) -> bool {
        self < 0
    }
}

impl CmdIdxVal for crate::cmdexpand::cmdidx_T {
    #[inline]
    fn check_user_cmd(self) -> bool {
        (self as c_int) < 0
    }
}

#[inline]
pub fn is_user_cmdidx<T: CmdIdxVal>(cmdidx: T) -> bool {
    cmdidx.check_user_cmd()
}

unsafe fn parse_map_cmd(arg_str: *const c_char, arena: *mut Arena) -> Array {
    unsafe {
        let mut args = arena_array(arena, 2);

        let lhs_start = arg_str as *mut c_char;
        let lhs_end = skiptowhite(lhs_start);
        let lhs_len = lhs_end.offset_from(lhs_start) as usize;

        add_c(&mut args, string_obj(cstrn_as_string(lhs_start, lhs_len)));

        let rhs_start = skipwhite(lhs_end);
        if *rhs_start != NUL {
            let rhs_len = CStr::from_ptr(rhs_start).to_bytes().len();
            add_c(&mut args, string_obj(cstrn_as_string(rhs_start, rhs_len)));
        }

        args
    }
}

/// Parse command line.
///
/// Doesn't check the validity of command arguments.
///
/// @param str       Command line string to parse. Cannot contain "\n".
/// @param opts      Optional parameters. Reserved for future use.
/// @param[out] err  Error details, if any.
/// @return Dict containing command information, with these keys:
///         - cmd: (string) Command name.
///         - range: (array) (optional) Command range ([<line1>] [<line2>]).
///                          Omitted if command doesn't accept a range.
///                          Otherwise, has no elements if no range was specified, one element if
///                          only a single range item was specified, or two elements if both range
///                          items were specified.
///         - count: (number) (optional) Command [<count>].
///                           Omitted if command cannot take a count.
///         - reg: (string) (optional) Command [<register>].
///                         Omitted if command cannot take a register.
///         - bang: (boolean) Whether command contains a [<bang>] (!) modifier.
///         - args: (array) Command arguments.
///         - addr: (string) Value of |:command-addr|. Uses short name or "line" for -addr=lines.
///         - nargs: (string) Value of |:command-nargs|.
///         - nextcmd: (string) Next command if there are multiple commands separated by a |:bar|.
///                             Empty if there isn't a next command.
///         - magic: (dict) Which characters have special meaning in the command arguments.
///             - file: (boolean) The command expands filenames. Which means characters such as "%",
///                               "#" and wildcards are expanded.
///             - bar: (boolean) The "|" character is treated as a command separator and the double
///                              quote character (") is treated as the start of a comment.
///         - mods: (dict) |:command-modifiers|.
///             - filter: (dict) |:filter|.
///                 - pattern: (string) Filter pattern. Empty string if there is no filter.
///                 - force: (boolean) Whether filter is inverted or not.
///             - silent: (boolean) |:silent|.
///             - emsg_silent: (boolean) |:silent!|.
///             - unsilent: (boolean) |:unsilent|.
///             - sandbox: (boolean) |:sandbox|.
///             - noautocmd: (boolean) |:noautocmd|.
///             - browse: (boolean) |:browse|.
///             - confirm: (boolean) |:confirm|.
///             - hide: (boolean) |:hide|.
///             - horizontal: (boolean) |:horizontal|.
///             - keepalt: (boolean) |:keepalt|.
///             - keepjumps: (boolean) |:keepjumps|.
///             - keepmarks: (boolean) |:keepmarks|.
///             - keeppatterns: (boolean) |:keeppatterns|.
///             - lockmarks: (boolean) |:lockmarks|.
///             - noswapfile: (boolean) |:noswapfile|.
///             - tab: (integer) |:tab|. -1 when omitted.
///             - verbose: (integer) |:verbose|. -1 when omitted.
///             - vertical: (boolean) |:vertical|.
///             - split: (string) Split modifier string, is an empty string when there's no split
///                               modifier. If there is a split modifier it can be one of:
///               - "aboveleft": |:aboveleft|.
///               - "belowright": |:belowright|.
///               - "topleft": |:topleft|.
///               - "botright": |:botright|.
// DLLEXPORT Dict(cmd) nvim_parse_cmd(String str, Dict(empty) *opts, Arena *arena, Error *err)
//   FUNC_API_SINCE(10) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_parse_cmd(
    str_val: NvimString,
    _opts: *mut KeyDict_empty,
    arena: *mut Arena,
    err: *mut Error,
) -> KeyDict_cmd {
    unsafe {
        let mut result = KeyDict_cmd::default();
        let mut ea: exarg_T = exarg_T::new();
        let mut cmod: cmdmod_T = cmdmod_T::new();
        let mut cmdline = arena_memdupz(arena, str_val.data, str_val.size);
        let mut errormsg: *const c_char = ptr::null();

        if !parse_cmdline(&mut cmdline, &mut ea, &mut cmod, &mut errormsg, false) {
            if !errormsg.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Parsing command-line: %s".as_ptr(),
                    errormsg,
                );
            } else {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Parsing command-line".as_ptr(),
                );
            }
            return result;
        }

        let mut args = Array::new();
        let length = CStr::from_ptr(ea.arg).to_bytes().len();

        if ea.cmdidx != CMD_SIZE
            && is_map_cmd(crate::cmdexpand::cmdidx_T::from_raw(ea.cmdidx))
            && *ea.arg != NUL
        {
            args = parse_map_cmd(ea.arg, arena);
        } else if (ea.argt & EX_NOSPC) != 0 {
            if *ea.arg != NUL {
                args = arena_array(arena, 1);
                add_c(&mut args, string_obj(cstrn_as_string(ea.arg, length)));
            }
        } else {
            let mut end: usize = 0;
            let mut len: usize = 0;
            let mut buf = arena_alloc(arena, length + 1, false).cast::<c_char>();
            let mut done = false;
            args = arena_array(arena, uc_nargs_upper_bound(ea.arg, length));

            while !done {
                done = uc_split_args_iter(ea.arg, length, &mut end, buf, &mut len);
                if len > 0 {
                    add_c(&mut args, string_obj(cstrn_as_string(buf, len)));
                    buf = buf.add(len + 1);
                }
            }
        }

        let cmd = if ea.cmdidx == CMD_USER {
            let u = &raw mut ucmds;
            (*u).as_mut_ptr().add(ea.useridx as usize)
        } else if ea.cmdidx == CMD_USER_BUF {
            (*curbuf).b_ucmds.as_mut_ptr().add(ea.useridx as usize)
        } else {
            ptr::null_mut()
        };

        let name = if ea.cmdidx == CMD_SIZE {
            c"".as_ptr()
        } else if !cmd.is_null() {
            (*cmd).uc_name
        } else {
            get_command_name(ptr::null_mut(), ea.cmdidx)
        };

        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__cmd;
        result.cmd = cstr_as_string(name);

        if (ea.argt & EX_RANGE) != 0 && ea.addr_count > 0 {
            let mut range = arena_array(arena, 2);
            if ea.addr_count > 1 {
                add_c(&mut range, integer_obj(i64::from(ea.line1)));
            }
            add_c(&mut range, integer_obj(i64::from(ea.line2)));
            result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__range;
            result.range = range;
        }

        if (ea.argt & EX_COUNT) != 0 {
            let count = if ea.addr_count > 0 {
                i64::from(ea.line2)
            } else if !cmd.is_null() {
                (*cmd).uc_def
            } else {
                0
            };
            if ea.addr_count > 0 || (!cmd.is_null() && (*cmd).uc_def != 0) || count != 0 {
                result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__count;
                result.count = count;
            }
        }

        if (ea.argt & EX_REGSTR) != 0 {
            let reg = [ea.regname as c_char, NUL];
            result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__reg;
            result.reg = cstr_to_arena_str(arena, reg.as_ptr());
        }

        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__bang;
        result.bang = ea.forceit != 0;
        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__args;
        result.args = args;

        let nargs_ch = if (ea.argt & EX_EXTRA) != 0 {
            if (ea.argt & EX_NOSPC) != 0 {
                if (ea.argt & EX_ARGSPACE) != 0 {
                    b'_'
                } else if (ea.argt & EX_NEEDARG) != 0 {
                    b'1'
                } else {
                    b'?'
                }
            } else if (ea.argt & EX_NEEDARG) != 0 {
                b'+'
            } else {
                b'*'
            }
        } else {
            b'0'
        };
        let nargs = [nargs_ch as c_char, NUL];
        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__nargs;
        result.nargs = string_obj(cstr_to_arena_str(arena, nargs.as_ptr()));

        let addr = match ea.addr_type {
            ADDR_LINES => c"line".as_ptr(),
            ADDR_ARGUMENTS => c"arg".as_ptr(),
            ADDR_BUFFERS => c"buf".as_ptr(),
            ADDR_LOADED_BUFFERS => c"load".as_ptr(),
            ADDR_WINDOWS => c"win".as_ptr(),
            ADDR_TABS => c"tab".as_ptr(),
            ADDR_QUICKFIX => c"qf".as_ptr(),
            ADDR_NONE => c"none".as_ptr(),
            _ => c"?".as_ptr(),
        };
        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__addr;
        result.addr = cstr_as_string(addr);
        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__nextcmd;
        result.nextcmd = cstr_as_string(ea.nextcmd);

        let mut mods = arena_dict(arena, 20);
        let mut filter = arena_dict(arena, 2);
        put_c(
            &mut filter,
            c"pattern",
            string_obj(cstr_to_arena_str(arena, cmod.cmod_filter_pat)),
        );
        put_c(&mut filter, c"force", boolean_obj(cmod.cmod_filter_force));
        put_c(&mut mods, c"filter", dict_obj(filter));

        put_c(
            &mut mods,
            c"silent",
            boolean_obj((cmod.cmod_flags & CMOD_SILENT) != 0),
        );
        put_c(
            &mut mods,
            c"emsg_silent",
            boolean_obj((cmod.cmod_flags & CMOD_ERRSILENT) != 0),
        );
        put_c(
            &mut mods,
            c"unsilent",
            boolean_obj((cmod.cmod_flags & CMOD_UNSILENT) != 0),
        );
        put_c(
            &mut mods,
            c"sandbox",
            boolean_obj((cmod.cmod_flags & CMOD_SANDBOX) != 0),
        );
        put_c(
            &mut mods,
            c"noautocmd",
            boolean_obj((cmod.cmod_flags & CMOD_NOAUTOCMD) != 0),
        );
        put_c(&mut mods, c"tab", integer_obj(i64::from(cmod.cmod_tab - 1)));
        put_c(
            &mut mods,
            c"verbose",
            integer_obj(i64::from(cmod.cmod_verbose - 1)),
        );
        put_c(
            &mut mods,
            c"browse",
            boolean_obj((cmod.cmod_flags & CMOD_BROWSE) != 0),
        );
        put_c(
            &mut mods,
            c"confirm",
            boolean_obj((cmod.cmod_flags & CMOD_CONFIRM) != 0),
        );
        put_c(
            &mut mods,
            c"hide",
            boolean_obj((cmod.cmod_flags & CMOD_HIDE) != 0),
        );
        put_c(
            &mut mods,
            c"keepalt",
            boolean_obj((cmod.cmod_flags & CMOD_KEEPALT) != 0),
        );
        put_c(
            &mut mods,
            c"keepjumps",
            boolean_obj((cmod.cmod_flags & CMOD_KEEPJUMPS) != 0),
        );
        put_c(
            &mut mods,
            c"keepmarks",
            boolean_obj((cmod.cmod_flags & CMOD_KEEPMARKS) != 0),
        );
        put_c(
            &mut mods,
            c"keeppatterns",
            boolean_obj((cmod.cmod_flags & CMOD_KEEPPATTERNS) != 0),
        );
        put_c(
            &mut mods,
            c"lockmarks",
            boolean_obj((cmod.cmod_flags & CMOD_LOCKMARKS) != 0),
        );
        put_c(
            &mut mods,
            c"noswapfile",
            boolean_obj((cmod.cmod_flags & CMOD_NOSWAPFILE) != 0),
        );
        put_c(
            &mut mods,
            c"vertical",
            boolean_obj((cmod.cmod_split & WSP_VERT) != 0),
        );
        put_c(
            &mut mods,
            c"horizontal",
            boolean_obj((cmod.cmod_split & WSP_HOR) != 0),
        );

        let split = if (cmod.cmod_split & WSP_BOT) != 0 {
            c"botright"
        } else if (cmod.cmod_split & WSP_TOP) != 0 {
            c"topleft"
        } else if (cmod.cmod_split & WSP_BELOW) != 0 {
            c"belowright"
        } else if (cmod.cmod_split & WSP_ABOVE) != 0 {
            c"aboveleft"
        } else {
            c""
        };
        put_c(
            &mut mods,
            c"split",
            string_obj(static_cstr_as_string(split)),
        );

        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__mods;
        result.mods = mods;

        let mut magic = arena_dict(arena, 2);
        put_c(&mut magic, c"file", boolean_obj(ea.magic.file));
        put_c(&mut magic, c"bar", boolean_obj(ea.magic.bar));
        result.is_set__cmd_ |= 1u64 << KEYSET_OPTIDX_cmd__magic;
        result.magic = magic;

        undo_cmdmod(&mut cmod);
        result
    }
}

/// Executes an Ex command `cmd`, specified as a Dict with the same structure as returned by
/// |nvim_parse_cmd()|.
///
/// Use `magic={…=false}` to disable special chars (see also |open-file|):
/// ```lua
/// vim.api.nvim_cmd({
///     cmd = 'edit',
///     args = { '%foo"|bar#baz"' },
///     magic = { file = false, bar = false }
///   },
///   {}
/// )
/// ```
///
/// - See |nvim_parse_cmd()| to parse a cmdline string (which can then be passed to `nvim_cmd`).
/// - See |nvim_command()| to execute a cmdline string.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @see |nvim_command()|
/// @see |nvim_exec2()|
/// @see |nvim_parse_cmd()|
///
/// @param cmd       Command to execute, a Dict with the same structure as the return value of
///                  |nvim_parse_cmd()| (except "addr", "nargs" and "nextcmd" are ignored).
///                  All keys except "cmd" are optional.
/// @param opts      Optional parameters.
///                  - output: (boolean, default false) Whether to return command output.
/// @param[out] err  Error details, if any.
/// @return Command output (non-error, non-shell |:!|) if `output` is true, else empty string.
// DLLEXPORT String nvim_cmd(uint64_t channel_id, Dict(cmd) *cmd, Dict(cmd_opts) *opts, Arena *arena,
//                           Error *err)
//   FUNC_API_SINCE(10);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_cmd(
    channel_id: u64,
    cmd: *mut KeyDict_cmd,
    opts: *mut KeyDict_cmd_opts,
    arena: *mut Arena,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let mut ea: exarg_T = exarg_T::new();
        let mut cmod: cmdmod_T = cmdmod_T::new();
        let mut cmdline: *mut c_char = ptr::null_mut();
        let mut errormsg: *const c_char = ptr::null();
        let mut args = Array::new();
        let mut retv = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };

        let mask = (*cmd).is_set__cmd_;

        'end: {
            if !has_key(mask, KEYSET_OPTIDX_cmd__cmd) {
                api_err_required(err, c"cmd".as_ptr());
                break 'end;
            }

            if *(*cmd).cmd.data == NUL
                && !((has_key(mask, KEYSET_OPTIDX_cmd__range) && (*cmd).range.size > 0)
                    || has_key(mask, KEYSET_OPTIDX_cmd__mods))
            {
                api_err_exp(
                    err,
                    c"cmd".as_ptr(),
                    c"non-empty String".as_ptr(),
                    ptr::null(),
                );
                break 'end;
            }

            let cmdname = arena_string(arena, (*cmd).cmd).data;
            ea.cmd = cmdname;

            let mut p = find_ex_command(&mut ea, ptr::null_mut());

            if !p.is_null()
                && ea.cmdidx == CMD_SIZE
                && ascii_isupper(*ea.cmd)
                && has_event(EVENT_CMDUNDEFINED)
            {
                p = arena_string(arena, (*cmd).cmd).data;
                let ret = apply_autocmds(EVENT_CMDUNDEFINED, p, p, true, ptr::null_mut());
                p = if ret && !aborting() {
                    find_ex_command(&mut ea, ptr::null_mut())
                } else {
                    ea.cmd
                };
            }

            let range_only = ea.cmdidx == CMD_SIZE
                && *(*cmd).cmd.data == NUL
                && has_key(mask, KEYSET_OPTIDX_cmd__range)
                && (*cmd).range.size > 0;

            if ea.cmdidx == CMD_SIZE
                && *(*cmd).cmd.data == NUL
                && (!has_key(mask, KEYSET_OPTIDX_cmd__range) || (*cmd).range.size == 0)
                && has_key(mask, KEYSET_OPTIDX_cmd__mods)
            {
                break 'end;
            }

            if !((!p.is_null() && ea.cmdidx != CMD_SIZE) || range_only) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Command not found: %s".as_ptr(),
                    cmdname,
                );
                break 'end;
            }

            if !(range_only || !is_cmd_ni(crate::cmdexpand::cmdidx_T::from_raw(ea.cmdidx))) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Command not implemented: %s".as_ptr(),
                    cmdname,
                );
                break 'end;
            }

            if !range_only {
                let fullname = if is_user_cmdidx(ea.cmdidx) {
                    get_user_command_name(ea.useridx, ea.cmdidx)
                } else {
                    get_command_name(ptr::null_mut(), ea.cmdidx)
                };
                let cmdname_bytes = CStr::from_ptr(cmdname).to_bytes();
                let fullname_bytes = CStr::from_ptr(fullname).to_bytes();
                if !fullname_bytes.starts_with(cmdname_bytes) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Invalid command: \"%s\"".as_ptr(),
                        cmdname,
                    );
                    break 'end;
                }
            }

            if range_only {
                ea.argt = EX_RANGE | EX_SBOXOK;
            } else if !is_user_cmdidx(ea.cmdidx) {
                ea.argt = excmd_get_argt(crate::cmdexpand::cmdidx_T::from_raw(ea.cmdidx));
            }

            let mut count_from_first_arg = false;
            if has_key(mask, KEYSET_OPTIDX_cmd__args) {
                if (*cmd).args.size == 1 && (ea.argt & EX_COUNT) != 0 && (ea.argt & EX_EXTRA) == 0 {
                    let first_arg = *(*cmd).args.items.add(0);
                    let mut is_numeric = false;
                    let mut count_value: i64 = 0;

                    if first_arg.obj_type == ObjectType::kObjectTypeInteger {
                        is_numeric = true;
                        count_value = first_arg.data.integer;
                    } else if first_arg.obj_type == ObjectType::kObjectTypeString {
                        let mut endptr: *const c_char = first_arg.data.string.data;
                        let val = getdigits_long((&raw mut endptr).cast(), false, 0);
                        if *endptr == 0 && first_arg.data.string.size > 0 {
                            is_numeric = true;
                            {
                                count_value = val as _;
                            }
                        }
                    }

                    if is_numeric && count_value >= 0 {
                        count_from_first_arg = true;
                        ea.addr_count = 1;
                        ea.line1 = count_value as linenr_T;
                        ea.line2 = count_value as linenr_T;
                        args = arena_array(arena, 0);
                    }
                }

                if !count_from_first_arg {
                    args = arena_array(arena, (*cmd).args.size);
                    for i in 0..(*cmd).args.size {
                        let elem = *(*cmd).args.items.add(i);
                        match elem.obj_type {
                            ObjectType::kObjectTypeBoolean => {
                                let data_str = arena_alloc(arena, 2, false).cast::<c_char>();
                                *data_str.add(0) =
                                    if elem.data.boolean { b'1' } else { b'0' } as c_char;
                                *data_str.add(1) = NUL;
                                add_c(&mut args, string_obj(cstr_as_string(data_str)));
                            }
                            ObjectType::kObjectTypeBuffer
                            | ObjectType::kObjectTypeWindow
                            | ObjectType::kObjectTypeTabpage
                            | ObjectType::kObjectTypeInteger => {
                                let s = elem.data.integer.to_string();
                                let data_str =
                                    arena_memdupz(arena, s.as_ptr().cast::<c_char>(), s.len());
                                add_c(&mut args, string_obj(cstr_as_string(data_str)));
                            }
                            ObjectType::kObjectTypeString => {
                                if string_iswhite(elem.data.string) {
                                    api_err_exp(
                                        err,
                                        c"command arg".as_ptr(),
                                        c"non-whitespace".as_ptr(),
                                        ptr::null(),
                                    );
                                    break 'end;
                                }
                                add_c(&mut args, elem);
                            }
                            _ => {
                                api_err_exp(
                                    err,
                                    c"command arg".as_ptr(),
                                    c"valid type".as_ptr(),
                                    api_typename(elem.obj_type),
                                );
                                break 'end;
                            }
                        }
                    }

                    let argc_valid = match ea.argt & (EX_EXTRA | EX_NOSPC | EX_NEEDARG) {
                        x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG) => args.size == 1,
                        x if x == (EX_EXTRA | EX_NOSPC) => args.size <= 1,
                        x if x == (EX_EXTRA | EX_NEEDARG) => args.size >= 1,
                        EX_EXTRA => true,
                        _ => args.size == 0,
                    };

                    if !argc_valid {
                        api_set_error!(
                            err,
                            ErrorType::kErrorTypeValidation,
                            c"%s".as_ptr(),
                            c"Wrong number of arguments".as_ptr(),
                        );
                        break 'end;
                    }
                }
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__range) {
                if (ea.argt & EX_RANGE) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Command cannot accept %s: %s".as_ptr(),
                        c"range".as_ptr(),
                        (*cmd).cmd.data,
                    );
                    break 'end;
                }
                if (*cmd).range.size > 2 {
                    api_err_exp(
                        err,
                        c"range".as_ptr(),
                        c"<=2 elements".as_ptr(),
                        ptr::null(),
                    );
                    break 'end;
                }

                let range = (*cmd).range;
                ea.addr_count = range.size as c_int;

                for i in 0..range.size {
                    let elem = *range.items.add(i);
                    if elem.obj_type != ObjectType::kObjectTypeInteger || elem.data.integer < 0 {
                        api_err_exp(
                            err,
                            c"range element".as_ptr(),
                            c"non-negative Integer".as_ptr(),
                            ptr::null(),
                        );
                        break 'end;
                    }
                }

                if range.size > 0 {
                    ea.line1 = (*range.items.add(0)).data.integer as linenr_T;
                    ea.line2 = (*range.items.add(range.size - 1)).data.integer as linenr_T;
                }
            }

            build_cmdline_str(&mut cmdline, &mut ea, args);
            if !parse_cmdline(&mut cmdline, &mut ea, &mut cmod, &mut errormsg, true) {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"%s".as_ptr(),
                    if !errormsg.is_null() {
                        errormsg
                    } else {
                        c"Parsing command-line".as_ptr()
                    },
                );
                break 'end;
            }

            if range_only {
                ea.argt = EX_RANGE | EX_SBOXOK;
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__range)
                && !invalid_range((&raw const ea).cast_mut()).is_null()
            {
                api_err_invalid(err, c"range".as_ptr(), c"".as_ptr(), 0, true);
                break 'end;
            }

            if ea.addr_count == 0 && (ea.argt & EX_DFLALL) == 0 && ea.addr_type == ADDR_OTHER {
                ea.line2 = 1;
            }

            ea.argc = args.size;
            if ea.argc > 0 {
                ea.args = xmalloc(ea.argc * std::mem::size_of::<*mut c_char>()).cast();
                ea.arglens = xmalloc(ea.argc * std::mem::size_of::<usize>()).cast();
                p = ea.arg;
                for i in 0..ea.argc {
                    *ea.args.add(i) = p;
                    let arglen = (*args.items.add(i)).data.string.size;
                    *ea.arglens.add(i) = arglen;
                    p = p.add(arglen + 1);
                }
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__count) {
                if count_from_first_arg {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Cannot specify both 'count' and numeric argument".as_ptr(),
                    );
                    break 'end;
                }
                if (ea.argt & EX_COUNT) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Command cannot accept %s: %s".as_ptr(),
                        c"count".as_ptr(),
                        (*cmd).cmd.data,
                    );
                    break 'end;
                }
                if (*cmd).count < 0 {
                    api_err_exp(
                        err,
                        c"count".as_ptr(),
                        c"non-negative Integer".as_ptr(),
                        ptr::null(),
                    );
                    break 'end;
                }
                set_cmd_count(&mut ea, (*cmd).count as linenr_T, true);
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__reg) {
                if (ea.argt & EX_REGSTR) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Command cannot accept %s: %s".as_ptr(),
                        c"register".as_ptr(),
                        (*cmd).cmd.data,
                    );
                    break 'end;
                }
                if (*cmd).reg.size != 1 {
                    api_err_exp(
                        err,
                        c"reg".as_ptr(),
                        c"single character".as_ptr(),
                        (*cmd).reg.data,
                    );
                    break 'end;
                }
                let regname = *(*cmd).reg.data;
                if regname == b'=' as c_char {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Cannot use register \"=".as_ptr(),
                    );
                    break 'end;
                }
                if !valid_yank_reg(
                    c_int::from(regname as u8),
                    !is_user_cmdidx(ea.cmdidx) && ea.cmdidx != CMD_put && ea.cmdidx != CMD_iput,
                ) {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Invalid register: \"%c".as_ptr(),
                        c_int::from(regname),
                    );
                    break 'end;
                }
                ea.regname = c_int::from(regname as u8);
            }

            ea.forceit = c_int::from((*cmd).bang);
            if ea.forceit != 0 && (ea.argt & EX_BANG) == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Command cannot accept %s: %s".as_ptr(),
                    c"bang".as_ptr(),
                    (*cmd).cmd.data,
                );
                break 'end;
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__magic) {
                let mut magic = KeyDict_cmd_magic::default();
                if !api_dict_to_keydict(
                    (&raw mut magic).cast(),
                    Some(KeyDict_cmd_magic_get_field),
                    (*cmd).magic,
                    err,
                ) {
                    break 'end;
                }

                ea.magic.file = if has_key(magic.is_set__cmd_magic_, KEYSET_OPTIDX_cmd_magic__file)
                {
                    magic.file
                } else {
                    (ea.argt & EX_XFILE) != 0
                };
                ea.magic.bar = if has_key(magic.is_set__cmd_magic_, KEYSET_OPTIDX_cmd_magic__bar) {
                    magic.bar
                } else {
                    (ea.argt & EX_TRLBAR) != 0
                };
                if ea.magic.file {
                    ea.argt |= EX_XFILE;
                } else {
                    ea.argt &= !EX_XFILE;
                }
            } else {
                ea.magic.file = (ea.argt & EX_XFILE) != 0;
                ea.magic.bar = (ea.argt & EX_TRLBAR) != 0;
            }

            if has_key(mask, KEYSET_OPTIDX_cmd__mods) {
                let mut mods = KeyDict_cmd_mods::default();
                if !api_dict_to_keydict(
                    (&raw mut mods).cast(),
                    Some(KeyDict_cmd_mods_get_field),
                    (*cmd).mods,
                    err,
                ) {
                    break 'end;
                }

                let mods_mask = mods.is_set__cmd_mods_;
                if has_key(mods_mask, KEYSET_OPTIDX_cmd_mods__filter) {
                    let mut filter = KeyDict_cmd_mods_filter::default();
                    if !api_dict_to_keydict(
                        (&raw mut filter).cast(),
                        Some(KeyDict_cmd_mods_filter_get_field),
                        mods.filter,
                        err,
                    ) {
                        break 'end;
                    }

                    if has_key(
                        filter.is_set__cmd_mods_filter_,
                        KEYSET_OPTIDX_cmd_mods_filter__pattern,
                    ) {
                        cmod.cmod_filter_force = filter.force;
                        if *filter.pattern.data != NUL || cmod.cmod_filter_force {
                            cmod.cmod_filter_pat = string_to_cstr(filter.pattern);
                            cmod.cmod_filter_regmatch.regprog =
                                vim_regcomp(cmod.cmod_filter_pat, RE_MAGIC);
                        }
                    }
                }

                if has_key(mods_mask, KEYSET_OPTIDX_cmd_mods__tab) && (mods.tab as c_int) >= 0 {
                    cmod.cmod_tab = (mods.tab as c_int) + 1;
                }

                if has_key(mods_mask, KEYSET_OPTIDX_cmd_mods__verbose)
                    && (mods.verbose as c_int) >= 0
                {
                    cmod.cmod_verbose = (mods.verbose as c_int) + 1;
                }

                if mods.vertical {
                    cmod.cmod_split |= WSP_VERT;
                }
                if mods.horizontal {
                    cmod.cmod_split |= WSP_HOR;
                }

                if has_key(mods_mask, KEYSET_OPTIDX_cmd_mods__split) {
                    if *mods.split.data == NUL {
                        // Empty string
                    } else if strequal_str(mods.split.data, "aboveleft")
                        || strequal_str(mods.split.data, "leftabove")
                    {
                        cmod.cmod_split |= WSP_ABOVE;
                    } else if strequal_str(mods.split.data, "belowright")
                        || strequal_str(mods.split.data, "rightbelow")
                    {
                        cmod.cmod_split |= WSP_BELOW;
                    } else if strequal_str(mods.split.data, "topleft") {
                        cmod.cmod_split |= WSP_TOP;
                    } else if strequal_str(mods.split.data, "botright") {
                        cmod.cmod_split |= WSP_BOT;
                    } else {
                        api_err_invalid(err, c"mods.split".as_ptr(), c"".as_ptr(), 0, true);
                        break 'end;
                    }
                }

                if mods.silent {
                    cmod.cmod_flags |= CMOD_SILENT;
                }
                if mods.emsg_silent {
                    cmod.cmod_flags |= CMOD_ERRSILENT;
                }
                if mods.unsilent {
                    cmod.cmod_flags |= CMOD_UNSILENT;
                }
                if mods.sandbox {
                    cmod.cmod_flags |= CMOD_SANDBOX;
                }
                if mods.noautocmd {
                    cmod.cmod_flags |= CMOD_NOAUTOCMD;
                }
                if mods.browse {
                    cmod.cmod_flags |= CMOD_BROWSE;
                }
                if mods.confirm {
                    cmod.cmod_flags |= CMOD_CONFIRM;
                }
                if mods.hide {
                    cmod.cmod_flags |= CMOD_HIDE;
                }
                if mods.keepalt {
                    cmod.cmod_flags |= CMOD_KEEPALT;
                }
                if mods.keepjumps {
                    cmod.cmod_flags |= CMOD_KEEPJUMPS;
                }
                if mods.keepmarks {
                    cmod.cmod_flags |= CMOD_KEEPMARKS;
                }
                if mods.keeppatterns {
                    cmod.cmod_flags |= CMOD_KEEPPATTERNS;
                }
                if mods.lockmarks {
                    cmod.cmod_flags |= CMOD_LOCKMARKS;
                }
                if mods.noswapfile {
                    cmod.cmod_flags |= CMOD_NOSWAPFILE;
                }

                if (cmod.cmod_flags & CMOD_ERRSILENT) != 0 {
                    cmod.cmod_flags |= CMOD_SILENT;
                }

                if (cmod.cmod_flags & CMOD_SANDBOX) != 0 && (ea.argt & EX_SBOXOK) == 0 {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"%s".as_ptr(),
                        c"Command cannot be run in sandbox".as_ptr(),
                    );
                    break 'end;
                }
            }

            p = replace_makeprg(&mut ea, ea.arg, &mut cmdline);
            if p != ea.arg {
                ea.arg = p;
                xfree(ea.args.cast());
                ea.args = ptr::null_mut();
                xfree(ea.arglens.cast());
                ea.arglens = ptr::null_mut();
                ea.argc = 0;
            }

            if (ea.argt & EX_ARGOPT) != 0 {
                while *ea.arg.add(0) == b'+' as c_char && *ea.arg.add(1) == b'+' as c_char {
                    let orig_arg = ea.arg;
                    let result = getargopt(&mut ea);
                    if result == FAIL
                        && !is_cmd_ni(crate::cmdexpand::cmdidx_T::from_raw(ea.cmdidx))
                    {
                        api_err_invalid(err, c"argument ".as_ptr(), orig_arg, 0, true);
                        break 'end;
                    }
                }
            }

            if (ea.argt & EX_CMDARG) != 0 && ea.usefilter == 0 {
                ea.do_ecmd_cmd = getargcmd(&mut ea.arg);
            }

            let mut capture_local: Vec<c_char> = Vec::new();
            let save_msg_silent = msg_silent;
            let save_redir_off = redir_off;
            let save_capture_ga = capture_ga;
            let save_msg_col = msg_col;

            if (*opts).output {
                capture_ga = &raw mut capture_local;
            }

            try_wrap(err, || {
                if (*opts).output {
                    msg_silent += 1;
                    redir_off = false;
                    msg_col = 0;
                }

                with_script_context(channel_id, || {
                    execute_cmd(&mut ea, &mut cmod, false);
                });

                if (*opts).output {
                    capture_ga = save_capture_ga;
                    msg_silent = save_msg_silent;
                    redir_off = save_redir_off;
                    msg_col = save_msg_col;
                }
            });

            if !error_set(err) && (*opts).output && capture_local.len() > 1 {
                retv = cbuf_to_arena_str(arena, capture_local.as_ptr(), capture_local.len());
                if *retv.data == b'\n' as c_char {
                    retv.data = retv.data.add(1);
                    retv.size -= 1;
                }
            }
        }

        xfree(cmdline.cast());
        xfree(ea.args.cast());
        xfree(ea.arglens.cast());
        retv
    }
}

unsafe fn string_iswhite(str_val: NvimString) -> bool {
    unsafe {
        for i in 0..str_val.size {
            let c = *str_val.data.add(i);
            if !ascii_iswhite(c) {
                return false;
            } else if c == NUL {
                break;
            }
        }
        true
    }
}

unsafe fn build_cmdline_str(cmdlinep: *mut *mut c_char, eap: *mut exarg_T, args: Array) {
    unsafe {
        let mut cmdline: Vec<u8> = Vec::with_capacity(32);

        if ((*eap).argt & EX_RANGE) != 0 {
            if (*eap).addr_count == 1 {
                let s = (*eap).line2.to_string();
                cmdline.extend_from_slice(s.as_bytes());
            } else if (*eap).addr_count > 1 {
                let s = format!("{},{}", (*eap).line1, (*eap).line2);
                cmdline.extend_from_slice(s.as_bytes());
            }
        }

        cmdline.extend_from_slice(CStr::from_ptr((*eap).cmd).to_bytes());

        for i in 0..args.size {
            let s = (*args.items.add(i)).data.string;
            cmdline.push(b' ');
            if s.size > 0 {
                cmdline.extend_from_slice(std::slice::from_raw_parts(s.data.cast::<u8>(), s.size));
            }
        }

        *cmdlinep = xmemdupz(cmdline.as_ptr().cast(), cmdline.len()).cast();
    }
}

/// Creates a global |user-commands| command.
///
/// For Lua usage see |lua-guide-commands-create|.
///
/// Example:
///
/// ```vim
/// :call nvim_create_user_command('SayHello', 'echo "Hello world!"', {'bang': v:true})
/// :SayHello
/// Hello world!
/// ```
///
/// @param  name    Command name. First char must be uppercase.
/// @param  cmd     Command or Lua function, executed when the command is invoked. Lua function
///                 receives a table with keys:
///                 - args: (string) Args passed to the command, if any. [<args>]
///                 - bang: (boolean) true if the command was executed with "!". [<bang>]
///                 - count: (number) Count, if any. [<count>]
///                 - fargs: (table) Args split by unescaped whitespace (when more than one arg is
///                   allowed), if any. [<f-args>]
///                 - line1: (number) Start of the command range. [<line1>]
///                 - line2: (number) End of the command range. [<line2>]
///                 - mods: (string) Command modifiers (unstructured string), if any. [<mods>]
///                 - name: (string) Command name.
///                 - nargs: (string) Number of arguments allowed by the command. |:command-nargs|
///                 - range: (number) Number of items in the command range: 0, 1, or 2. [<range>]
///                 - reg: (string) Register name, if any. [<reg>]
///                 - smods: (table) Command modifiers (structured), same as "mods" in |nvim_parse_cmd()|.
/// @param  opts    Optional flags:
///                 - `addr` |:command-addr|
///                 - `complete` |:command-complete| command or function |:command-completion-customlist|.
///                 - `count` |:command-count|
///                 - `desc` (string) Command description.
///                 - `force` (boolean, default true) Override the existing definition, if any.
///                 - `nargs` Number of arguments allowed by the command. |:command-nargs|
///                 - `preview` (function) Preview handler for 'inccommand'. |:command-preview|
///                 - `range` |:command-range|
///                 - boolean |command-attributes| such as |:command-bang| or |:command-bar| (but
///                   not |:command-buffer|, use |nvim_buf_create_user_command()| instead).
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_create_user_command(uint64_t channel_id,
//                                         String name,
//                                         Union(String, LuaRefOf((DictAs(create_user_command__command_args) args))) cmd,
//                                         Dict(user_command) *opts,
//                                         Error *err)
//   FUNC_API_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_create_user_command(
    channel_id: u64,
    name: NvimString,
    cmd: Object,
    opts: *mut KeyDict_user_command,
    err: *mut Error,
) {
    unsafe {
        create_user_command(channel_id, name, cmd, opts, 0, err);
    }
}

/// Delete a user-defined command.
///
/// @param  name    Name of the command to delete.
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_del_user_command(String name, Error *err)
//   FUNC_API_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_del_user_command(name: NvimString, err: *mut Error) {
    unsafe {
        nvim_buf_del_user_command(-1, name, err);
    }
}

/// Creates a buffer-local command |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer.
/// @param[out] err Error details, if any.
/// @see nvim_create_user_command
// DLLEXPORT void nvim_buf_create_user_command(uint64_t channel_id, Buffer buf, String name,
//                                             Object cmd, Dict(user_command) *opts, Error *err)
//   FUNC_API_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_create_user_command(
    channel_id: u64,
    buf: Buffer,
    name: NvimString,
    cmd: Object,
    opts: *mut KeyDict_user_command,
    err: *mut Error,
) {
    unsafe {
        let target_buf = find_buffer_by_handle(buf, err);
        if error_set(err) {
            return;
        }

        let save_curbuf = curbuf;
        curbuf = target_buf;
        create_user_command(channel_id, name, cmd, opts, UC_BUFFER, err);
        curbuf = save_curbuf;
    }
}

/// Delete a buffer-local user-defined command.
///
/// Only commands created with |:command-buffer| or
/// |nvim_buf_create_user_command()| can be deleted with this function.
///
/// @param  buf  Buffer id, or 0 for current buffer.
/// @param  name    Name of the command to delete.
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_buf_del_user_command(Buffer buf, String name, Error *err)
//   FUNC_API_SINCE(9);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_del_user_command(buf: Buffer, name: NvimString, err: *mut Error) {
    unsafe {
        let gap = if buf == -1 {
            &raw mut ucmds
        } else {
            let b = find_buffer_by_handle(buf, err);
            if error_set(err) {
                return;
            }
            &raw mut (*b).b_ucmds
        };

        let name_bytes = CStr::from_ptr(name.data).to_bytes();
        if let Some(i) = (*gap)
            .iter()
            .position(|cmd| CStr::from_ptr(cmd.uc_name).to_bytes() == name_bytes)
        {
            let _ = (*gap).remove(i);
            return;
        }

        api_set_error!(
            err,
            ErrorType::kErrorTypeException,
            c"Invalid command (not found): %s".as_ptr(),
            name.data,
        );
    }
}

// DLLEXPORT void create_user_command(uint64_t channel_id, String name, Union(String, LuaRef) cmd,
//                                    Dict(user_command) *opts, int flags, Error *err);
pub unsafe fn create_user_command(
    channel_id: u64,
    name: NvimString,
    cmd: Object,
    opts: *mut KeyDict_user_command,
    flags: c_int,
    err: *mut Error,
) {
    unsafe {
        let mut argt: u32 = 0;
        let mut def: i64 = -1;
        let mut addr_type_arg: cmd_addr_T = ADDR_NONE;
        let mut context: c_int = EXPAND_NOTHING;
        let mut compl_arg: *mut c_char = ptr::null_mut();
        let rep: *const c_char;
        let mut luaref: LuaRef = LUA_NOREF;
        let mut compl_luaref: LuaRef = LUA_NOREF;
        let mut preview_luaref: LuaRef = LUA_NOREF;

        'err: {
            if uc_validate_name(name.data).is_null() {
                api_err_invalid(err, c"command name".as_ptr(), name.data, 0, true);
                break 'err;
            }
            if mb_islower(c_int::from(*name.data.cast::<u8>())) {
                api_err_invalid(
                    err,
                    c"command name (must start with uppercase)".as_ptr(),
                    name.data,
                    0,
                    true,
                );
                break 'err;
            }
            let mask = (*opts).is_set__user_command_;
            if has_key(mask, KEYSET_OPTIDX_user_command__range)
                && has_key(mask, KEYSET_OPTIDX_user_command__count)
            {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"Cannot use both 'range' and 'count'".as_ptr(),
                );
                break 'err;
            }

            if (*opts).nargs.obj_type == ObjectType::kObjectTypeInteger {
                match (*opts).nargs.data.integer {
                    0 => {}
                    1 => {
                        argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG;
                    }
                    n => {
                        api_err_invalid(err, c"nargs".as_ptr(), ptr::null(), n, false);
                        break 'err;
                    }
                }
            } else if (*opts).nargs.obj_type == ObjectType::kObjectTypeString {
                if (*opts).nargs.data.string.size > 1 {
                    api_err_invalid(
                        err,
                        c"nargs".as_ptr(),
                        (*opts).nargs.data.string.data,
                        0,
                        true,
                    );
                    break 'err;
                }
                match *(*opts).nargs.data.string.data as u8 {
                    b'*' => argt |= EX_EXTRA,
                    b'?' => argt |= EX_EXTRA | EX_NOSPC,
                    b'+' => argt |= EX_EXTRA | EX_NEEDARG,
                    b'_' => argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE,
                    _ => {
                        api_err_invalid(
                            err,
                            c"nargs".as_ptr(),
                            (*opts).nargs.data.string.data,
                            0,
                            true,
                        );
                        break 'err;
                    }
                }
            } else if has_key(mask, KEYSET_OPTIDX_user_command__nargs) {
                api_err_invalid(err, c"nargs".as_ptr(), c"".as_ptr(), 0, true);
                break 'err;
            }

            if has_key(mask, KEYSET_OPTIDX_user_command__complete) && argt == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"%s".as_ptr(),
                    c"'complete' used without 'nargs'".as_ptr(),
                );
                break 'err;
            }

            if (*opts).range.obj_type == ObjectType::kObjectTypeBoolean {
                if (*opts).range.data.boolean {
                    argt |= EX_RANGE;
                    addr_type_arg = ADDR_LINES;
                }
            } else if (*opts).range.obj_type == ObjectType::kObjectTypeString {
                if !(*(*opts).range.data.string.data == b'%' as c_char
                    && (*opts).range.data.string.size == 1)
                {
                    api_err_invalid(err, c"range".as_ptr(), c"".as_ptr(), 0, true);
                    break 'err;
                }
                argt |= EX_RANGE | EX_DFLALL;
                addr_type_arg = ADDR_LINES;
            } else if (*opts).range.obj_type == ObjectType::kObjectTypeInteger {
                argt |= EX_RANGE | EX_ZEROR;
                def = (*opts).range.data.integer;
                addr_type_arg = ADDR_LINES;
            } else if has_key(mask, KEYSET_OPTIDX_user_command__range) {
                api_err_invalid(err, c"range".as_ptr(), c"".as_ptr(), 0, true);
                break 'err;
            }

            if (*opts).count.obj_type == ObjectType::kObjectTypeBoolean {
                if (*opts).count.data.boolean {
                    argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
                    addr_type_arg = ADDR_OTHER;
                    def = 0;
                }
            } else if (*opts).count.obj_type == ObjectType::kObjectTypeInteger {
                argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
                addr_type_arg = ADDR_OTHER;
                def = (*opts).count.data.integer;
            } else if has_key(mask, KEYSET_OPTIDX_user_command__count) {
                api_err_invalid(err, c"count".as_ptr(), c"".as_ptr(), 0, true);
                break 'err;
            }

            if has_key(mask, KEYSET_OPTIDX_user_command__addr) {
                if (*opts).addr.obj_type != ObjectType::kObjectTypeString {
                    api_err_exp(
                        err,
                        c"addr".as_ptr(),
                        api_typename(ObjectType::kObjectTypeString),
                        api_typename((*opts).addr.obj_type),
                    );
                    break 'err;
                }
                if parse_addr_type_arg(
                    (*opts).addr.data.string.data,
                    (*opts).addr.data.string.size as c_int,
                    &mut addr_type_arg,
                ) != OK
                {
                    api_err_invalid(
                        err,
                        c"addr".as_ptr(),
                        (*opts).addr.data.string.data,
                        0,
                        true,
                    );
                    break 'err;
                }
                argt |= EX_RANGE;
                if addr_type_arg != ADDR_LINES {
                    argt |= EX_ZEROR;
                }
            }

            if (*opts).bang {
                argt |= EX_BANG;
            }
            if (*opts).bar {
                argt |= EX_TRLBAR;
            }
            if (*opts).register_ {
                argt |= EX_REGSTR;
            }
            if (*opts).keepscript {
                argt |= EX_KEEPSCRIPT;
            }

            let force = if has_key(mask, KEYSET_OPTIDX_user_command__force) {
                (*opts).force
            } else {
                true
            };
            if error_set(err) {
                break 'err;
            }

            if (*opts).complete.obj_type == ObjectType::kObjectTypeLuaRef {
                context = EXPAND_USER_LUA;
                compl_luaref = (*opts).complete.data.luaref;
                (*opts).complete.data.luaref = LUA_NOREF;
            } else if (*opts).complete.obj_type == ObjectType::kObjectTypeString {
                if parse_compl_arg(
                    (*opts).complete.data.string.data,
                    (*opts).complete.data.string.size as c_int,
                    &mut context,
                    &mut argt,
                    &mut compl_arg,
                ) != OK
                {
                    api_err_invalid(
                        err,
                        c"complete".as_ptr(),
                        (*opts).complete.data.string.data,
                        0,
                        true,
                    );
                    break 'err;
                }
            } else if has_key(mask, KEYSET_OPTIDX_user_command__complete) {
                api_err_exp(
                    err,
                    c"complete".as_ptr(),
                    c"Function or String".as_ptr(),
                    ptr::null(),
                );
                break 'err;
            }

            if has_key(mask, KEYSET_OPTIDX_user_command__preview) {
                if (*opts).preview.obj_type != ObjectType::kObjectTypeLuaRef {
                    api_err_exp(
                        err,
                        c"preview".as_ptr(),
                        api_typename(ObjectType::kObjectTypeLuaRef),
                        api_typename((*opts).preview.obj_type),
                    );
                    break 'err;
                }
                argt |= EX_PREVIEW;
                preview_luaref = (*opts).preview.data.luaref;
                (*opts).preview.data.luaref = LUA_NOREF;
            }

            let mut desc: *const c_char = ptr::null();
            if has_key(mask, KEYSET_OPTIDX_user_command__desc) {
                if (*opts).desc.obj_type != ObjectType::kObjectTypeString {
                    api_err_exp(
                        err,
                        c"desc".as_ptr(),
                        api_typename(ObjectType::kObjectTypeString),
                        api_typename((*opts).desc.obj_type),
                    );
                    break 'err;
                }
                desc = (*opts).desc.data.string.data;
            }

            match cmd.obj_type {
                ObjectType::kObjectTypeLuaRef => {
                    luaref = api_new_luaref(cmd.data.luaref);
                    rep = c"".as_ptr();
                }
                ObjectType::kObjectTypeString => {
                    rep = cmd.data.string.data;
                }
                _ => {
                    api_err_exp(
                        err,
                        c"command".as_ptr(),
                        c"Function or String".as_ptr(),
                        ptr::null(),
                    );
                    break 'err;
                }
            }

            with_script_context(channel_id, || {
                if uc_add_command(
                    name.data,
                    name.size,
                    rep,
                    argt,
                    def,
                    flags,
                    context,
                    compl_arg,
                    compl_luaref,
                    preview_luaref,
                    addr_type_arg,
                    luaref,
                    desc,
                    force,
                ) != OK
                {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeException,
                        c"Failed to create user command".as_ptr(),
                    );
                }
            });

            return;
        }

        if luaref != LUA_NOREF {
            api_free_luaref(luaref);
        }
        if compl_luaref != LUA_NOREF {
            api_free_luaref(compl_luaref);
        }
        if preview_luaref != LUA_NOREF {
            api_free_luaref(preview_luaref);
        }
        xfree(compl_arg.cast());
    }
}

/// Gets a map of global (non-buffer-local) Ex commands.
///
/// Currently only |user-commands| are supported, not builtin Ex commands.
///
/// @see |nvim_get_all_options_info()|
///
/// @param  opts  Optional parameters. Currently only supports
///               {"builtin":false}
/// @param[out]  err   Error details, if any.
///
/// @returns Map of maps describing commands.
// DLLEXPORT DictOf(DictAs(command_info)) nvim_get_commands(Dict(get_commands) *opts, Arena *arena,
//                                                          Error *err)
//   FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_get_commands(
    opts: *mut KeyDict_get_commands,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe { nvim_buf_get_commands(-1, opts, arena, err) }
}

/// Gets a map of buffer-local |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer
/// @param  opts  Optional parameters. Currently not used.
/// @param[out]  err   Error details, if any.
///
/// @returns Map of maps describing commands.
// DLLEXPORT DictAs(command_info) nvim_buf_get_commands(Buffer buf, Dict(get_commands) *opts,
//                                                      Arena *arena, Error *err)
//   FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_buf_get_commands(
    buf: Buffer,
    opts: *mut KeyDict_get_commands,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let global = buf == -1;
        if error_set(err) {
            return Dict::new();
        }

        if global {
            if (*opts).builtin {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"builtin=true not implemented".as_ptr(),
                );
                return Dict::new();
            }
            return commands_array(ptr::null_mut(), arena);
        }

        let b = find_buffer_by_handle(buf, err);
        if (*opts).builtin || b.is_null() {
            return Dict::new();
        }
        commands_array(b, arena)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_user_cmdidx_predicates() {
        assert!(is_user_cmdidx(CMD_USER));
        assert!(is_user_cmdidx(CMD_USER_BUF));
        assert!(!is_user_cmdidx(CMD_SIZE));
        assert!(ascii_isupper(b'A' as c_char));
        assert!(!ascii_isupper(b'a' as c_char));
        assert!(ascii_iswhite(b' ' as c_char));
        assert!(ascii_iswhite(b'\t' as c_char));
        assert!(!ascii_iswhite(b'x' as c_char));
    }
}
