//! Command API functions (`src/nvim/api/command.c`).
#![forbid(unsafe_code)]

use crate::api::deprecated::KeyDict_empty;
use crate::api::private::dispatch::{
    key_dict_cmd_magic_get_field, key_dict_cmd_mods_filter_get_field, key_dict_cmd_mods_get_field,
};
use crate::api::private::helpers::{
    add_c, api_dict_to_keydict, api_set_error, api_typename, boolean_obj, cstr_as_string, dict_obj,
    error_set, find_buffer_by_handle, has_key, integer_obj, put_c, static_cstr_as_string,
    string_obj, try_wrap, with_script_context,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, NvimString, Object, ObjectType, api_err_exp, api_err_invalid,
    api_err_required,
};
use crate::api::tabpage::Buffer;
use crate::autocmd::{EVENT_CMDUNDEFINED, apply_autocmds, has_event};
use crate::charset::{ascii_isupper, getdigits_long};
use crate::cmdexpand::Cmdidx;
use crate::ex_docmd::{
    ADDR_ARGUMENTS, ADDR_BUFFERS, ADDR_LINES, ADDR_LOADED_BUFFERS, ADDR_NONE, ADDR_OTHER,
    ADDR_QUICKFIX, ADDR_TABS, ADDR_WINDOWS, CMOD_BROWSE, CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE,
    CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS,
    CMOD_NOAUTOCMD, CMOD_NOSWAPFILE, CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, Cmdmod, EX_ARGOPT,
    EX_ARGSPACE, EX_BANG, EX_CMDARG, EX_COUNT, EX_DFLALL, EX_EXTRA, EX_KEEPSCRIPT, EX_NEEDARG,
    EX_NOSPC, EX_PREVIEW, EX_RANGE, EX_REGSTR, EX_SBOXOK, EX_TRLBAR, EX_XFILE, EX_ZEROR, Exarg,
    excmd_get_argt, execute_cmd, find_ex_command, get_command_name, getargcmd, getargopt,
    invalid_range, is_cmd_ni, is_map_cmd, parse_cmdline, replace_makeprg, set_cmd_count,
    undo_cmdmod,
};
use crate::ex_eval::aborting;
use crate::globals::*;
use crate::highlight::OptionalKeys;
use crate::lua::converter::{Boolean, LUA_NOREF, LuaRef};
use crate::lua::executor::{api_free_luaref, api_new_luaref};
use crate::mbyte::mb_islower;
use crate::memory::bytes_to_cstring;
use crate::msgpack_rpc::packer::Integer;
use crate::regexp::{RE_MAGIC, vim_regcomp};
use crate::register::valid_yank_reg;
use crate::usercmd::{
    UC_BUFFER, commands_array, get_user_command_name, parse_addr_type_arg, parse_compl_arg,
    uc_add_command, uc_nargs_upper_bound, uc_split_args_iter, uc_validate_name, with_ucmds,
    with_ucmds_mut,
};
use std::ffi::CStr;

use crate::cmdexpand::{CmdAddr, EXPAND_NOTHING, EXPAND_USER_LUA};
use crate::cursor::Linenr;

const KEYSET_OPTIDX_CMD_CMD: u64 = 1;
const KEYSET_OPTIDX_CMD_REG: u64 = 2;
const KEYSET_OPTIDX_CMD_BANG: u64 = 3;
const KEYSET_OPTIDX_CMD_ADDR: u64 = 4;
const KEYSET_OPTIDX_CMD_MODS: u64 = 5;
const KEYSET_OPTIDX_CMD_ARGS: u64 = 6;
const KEYSET_OPTIDX_CMD_COUNT: u64 = 7;
const KEYSET_OPTIDX_CMD_MAGIC: u64 = 8;
const KEYSET_OPTIDX_CMD_NARGS: u64 = 9;
const KEYSET_OPTIDX_CMD_RANGE: u64 = 10;
const KEYSET_OPTIDX_CMD_NEXTCMD: u64 = 11;

const KEYSET_OPTIDX_CMD_MAGIC_BAR: u64 = 1;
const KEYSET_OPTIDX_CMD_MAGIC_FILE: u64 = 2;

const KEYSET_OPTIDX_CMD_MODS_TAB: u64 = 1;
const KEYSET_OPTIDX_CMD_MODS_SPLIT: u64 = 3;
const KEYSET_OPTIDX_CMD_MODS_FILTER: u64 = 5;
const KEYSET_OPTIDX_CMD_MODS_VERBOSE: u64 = 10;

const KEYSET_OPTIDX_CMD_MODS_FILTER_PATTERN: u64 = 2;

const KEYSET_OPTIDX_USER_COMMAND_ADDR: u64 = 2;
const KEYSET_OPTIDX_USER_COMMAND_DESC: u64 = 4;
const KEYSET_OPTIDX_USER_COMMAND_COUNT: u64 = 5;
const KEYSET_OPTIDX_USER_COMMAND_FORCE: u64 = 6;
const KEYSET_OPTIDX_USER_COMMAND_NARGS: u64 = 7;
const KEYSET_OPTIDX_USER_COMMAND_RANGE: u64 = 8;
const KEYSET_OPTIDX_USER_COMMAND_PREVIEW: u64 = 9;
const KEYSET_OPTIDX_USER_COMMAND_COMPLETE: u64 = 10;

use crate::window::{WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT};

#[repr(C)]
#[derive(Clone, Default)]
pub struct KeyDict_cmd {
    pub is_set_cmd: OptionalKeys,
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
    pub is_set_cmd_magic: OptionalKeys,
    pub file: Boolean,
    pub bar: Boolean,
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct KeyDict_cmd_mods {
    pub is_set_cmd_mods: OptionalKeys,
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
#[derive(Clone, Default)]
pub struct KeyDict_cmd_mods_filter {
    pub is_set_cmd_mods_filter: OptionalKeys,
    pub pattern: NvimString,
    pub force: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyDict_cmd_opts {
    pub output: Boolean,
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct KeyDict_user_command {
    pub is_set_user_command: OptionalKeys,
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
fn cstr_to_arena_str(str_ptr: Option<&CStr>) -> NvimString {
    match str_ptr {
        None => NvimString::new(),
        Some(s) => NvimString::from_bytes(s.to_bytes()),
    }
}

pub trait CmdIdxVal {
    fn check_user_cmd(self) -> bool;
}

impl CmdIdxVal for i32 {
    #[inline]
    fn check_user_cmd(self) -> bool {
        self < 0
    }
}

impl CmdIdxVal for crate::cmdexpand::Cmdidx {
    #[inline]
    fn check_user_cmd(self) -> bool {
        (self as i32) < 0
    }
}

#[inline]
pub fn is_user_cmdidx<T: CmdIdxVal>(cmdidx: T) -> bool {
    cmdidx.check_user_cmd()
}

fn parse_map_cmd(arg_str: &CStr) -> Array {
    let mut args = Array::with_capacity(2);
    let bytes = arg_str.to_bytes();
    let lhs_len = crate::charset::skiptowhite_len(bytes);

    add_c(
        &mut args,
        string_obj(NvimString::from_bytes(&bytes[..lhs_len])),
    );

    let rhs = crate::charset::skipwhite_slice(&bytes[lhs_len..]);
    if !rhs.is_empty() {
        add_c(&mut args, string_obj(NvimString::from_bytes(rhs)));
    }

    args
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
pub fn nvim_parse_cmd(
    str_val: NvimString,
    _opts: &mut KeyDict_empty,
    err: &mut Error,
) -> KeyDict_cmd {
    let mut result = KeyDict_cmd::default();
    let mut ea: Exarg = Exarg::new();
    let mut cmod: Cmdmod = Cmdmod::new();
    let mut errormsg = None;
    ea.cmdline_buf = str_val
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .collect();

    if !parse_cmdline(&mut ea, &mut cmod, &mut errormsg, false) {
        if let Some(msg) = errormsg {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"Parsing command-line: %s",
                msg,
            );
        } else {
            api_set_error!(err, ErrorType::ErrorTypeException, c"Parsing command-line",);
        }
        return result;
    }

    let mut args = Array::new();
    let length = ea.arg.len();

    if ea.cmdidx != Cmdidx::CmdSize && is_map_cmd(ea.cmdidx) && !ea.arg.is_empty() {
        args = parse_map_cmd(ea.arg.as_cstr());
    } else if (ea.argt & EX_NOSPC) != 0 {
        if !ea.arg.is_empty() {
            args = Array::with_capacity(1);
            add_c(
                &mut args,
                string_obj(NvimString::from_bytes(ea.arg.slice(length))),
            );
        }
    } else {
        let mut end: usize = 0;
        let mut len: usize = 0;
        let mut buf_vec = vec![0u8; length + 1];
        let mut buf_off = 0usize;
        let mut done = false;
        let arg_slice = ea.arg.slice(length);
        args = Array::with_capacity(uc_nargs_upper_bound(arg_slice));

        while !done {
            let buf_slice = &mut buf_vec[buf_off..];
            done = uc_split_args_iter(arg_slice, &mut end, buf_slice, &mut len);
            if len > 0 {
                add_c(
                    &mut args,
                    string_obj(NvimString::from_bytes(&buf_slice[..len])),
                );
                buf_off = (buf_off + len + 1).min(buf_vec.len());
            }
        }
    }

    let cmd_info = if ea.cmdidx == Cmdidx::CmdUser {
        with_ucmds(|u| {
            let c = &u[ea.useridx as usize];
            Some((c.uc_name.clone(), c.uc_def))
        })
    } else if ea.cmdidx == Cmdidx::CmdUserBuf {
        let cb = curbuf();
        let ucmds = cb.b_ucmds.borrow();
        let c = &ucmds[ea.useridx as usize];
        Some((c.uc_name.clone(), c.uc_def))
    } else {
        None
    };

    let mut xp = crate::os::env::ExpandCtx::new();
    let name = if ea.cmdidx == Cmdidx::CmdSize {
        Some(c"")
    } else if let Some((ref uc_name, _)) = cmd_info {
        Some(uc_name.as_c_str())
    } else {
        get_command_name(&mut xp, ea.cmdidx as i32)
    };

    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_CMD;
    result.cmd = match name {
        Some(n) => NvimString::from_bytes(n.to_bytes()),
        None => NvimString::default(),
    };

    if (ea.argt & EX_RANGE) != 0 && ea.addr_count > 0 {
        let mut range = Array::with_capacity(2);
        if ea.addr_count > 1 {
            add_c(&mut range, integer_obj(i64::from(ea.line1)));
        }
        add_c(&mut range, integer_obj(i64::from(ea.line2)));
        result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_RANGE;
        result.range = range;
    }

    if (ea.argt & EX_COUNT) != 0 {
        let count = if ea.addr_count > 0 {
            i64::from(ea.line2)
        } else if let Some((_, uc_def)) = cmd_info {
            uc_def
        } else {
            0
        };
        if ea.addr_count > 0 || cmd_info.is_some_and(|(_, uc_def)| uc_def != 0) || count != 0 {
            result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_COUNT;
            result.count = count;
        }
    }

    if (ea.argt & EX_REGSTR) != 0 {
        let reg = if ea.regname != 0 {
            &[ea.regname as u8][..]
        } else {
            b""
        };
        result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_REG;
        result.reg = NvimString::from_bytes(reg);
    }

    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_BANG;
    result.bang = ea.forceit;
    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_ARGS;
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
    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_NARGS;
    result.nargs = string_obj(NvimString::from_bytes(&[nargs_ch]));

    let addr = match ea.addr_type {
        ADDR_LINES => c"line",
        ADDR_ARGUMENTS => c"arg",
        ADDR_BUFFERS => c"buf",
        ADDR_LOADED_BUFFERS => c"load",
        ADDR_WINDOWS => c"win",
        ADDR_TABS => c"tab",
        ADDR_QUICKFIX => c"qf",
        ADDR_NONE => c"none",
        _ => c"?",
    };
    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_ADDR;
    result.addr = cstr_as_string(Some(addr));
    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_NEXTCMD;
    result.nextcmd = if ea.nextcmd.is_null() {
        cstr_as_string(None)
    } else {
        cstr_to_arena_str(ea.nextcmd.as_opt_cstr())
    };

    let mut mods = Dict::with_capacity(20);
    let mut filter = Dict::with_capacity(2);
    put_c(
        &mut filter,
        c"pattern",
        string_obj(cstr_to_arena_str(cmod.cmod_filter_pat.as_deref())),
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

    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_MODS;
    result.mods = mods;

    let mut magic = Dict::with_capacity(2);
    put_c(&mut magic, c"file", boolean_obj(ea.magic.file));
    put_c(&mut magic, c"bar", boolean_obj(ea.magic.bar));
    result.is_set_cmd |= 1u64 << KEYSET_OPTIDX_CMD_MAGIC;
    result.magic = magic;

    undo_cmdmod(&mut cmod);
    result
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
pub fn nvim_cmd(
    channel_id: u64,
    cmd: &mut KeyDict_cmd,
    opts: &mut KeyDict_cmd_opts,
    err: &mut Error,
) -> NvimString {
    let mut ea: Exarg = Exarg::new();
    let mut cmod: Cmdmod = Cmdmod::new();
    let mut cmdline_buf: Vec<u8> = Vec::new();
    let mut errormsg = None;
    let mut args = Array::new();
    let mut retv = NvimString::new();

    let mask = cmd.is_set_cmd;

    'end: {
        if !has_key(mask, KEYSET_OPTIDX_CMD_CMD) {
            api_err_required(err, c"cmd");
            break 'end;
        }

        if cmd.cmd.cstr_bytes().is_empty()
            && !((has_key(mask, KEYSET_OPTIDX_CMD_RANGE) && cmd.range.size > 0)
                || has_key(mask, KEYSET_OPTIDX_CMD_MODS))
        {
            api_err_exp(err, c"cmd", c"non-empty String", None);
            break 'end;
        }

        let cmdname = crate::api::private::helpers::ensure_nul_string(cmd.cmd.clone());
        ea.cmd = crate::strings::OwnedSlice::from_cstr(cmdname.as_cstr());

        let mut p_opt = find_ex_command(&mut ea, None);

        if !p_opt.is_null()
            && ea.cmdidx == Cmdidx::CmdSize
            && ascii_isupper(ea.cmd.first_byte())
            && has_event(EVENT_CMDUNDEFINED)
        {
            let p_raw = crate::api::private::helpers::ensure_nul_string(cmd.cmd.clone());
            let p_ev = p_raw.as_opt_cstr();
            let ret = apply_autocmds(EVENT_CMDUNDEFINED, p_ev, p_ev, true, None);
            p_opt = if ret && !aborting() {
                find_ex_command(&mut ea, None)
            } else {
                ea.cmd.clone()
            };
        }

        let range_only = ea.cmdidx == Cmdidx::CmdSize
            && cmd.cmd.cstr_bytes().is_empty()
            && has_key(mask, KEYSET_OPTIDX_CMD_RANGE)
            && cmd.range.size > 0;

        if ea.cmdidx == Cmdidx::CmdSize
            && cmd.cmd.cstr_bytes().is_empty()
            && (!has_key(mask, KEYSET_OPTIDX_CMD_RANGE) || cmd.range.size == 0)
            && has_key(mask, KEYSET_OPTIDX_CMD_MODS)
        {
            break 'end;
        }

        if !((!p_opt.is_null() && ea.cmdidx != Cmdidx::CmdSize) || range_only) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Command not found: %s",
                cmdname,
            );
            break 'end;
        }

        if !(range_only || !is_cmd_ni(ea.cmdidx)) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Command not implemented: %s",
                cmdname,
            );
            break 'end;
        }

        if !range_only {
            let mut xp = crate::os::env::ExpandCtx::new();
            let user_fullname = is_user_cmdidx(ea.cmdidx)
                .then(|| get_user_command_name(ea.useridx, ea.cmdidx))
                .flatten();
            let fullname = if is_user_cmdidx(ea.cmdidx) {
                user_fullname.as_deref()
            } else {
                get_command_name(&mut xp, ea.cmdidx as i32)
            };
            let cmdname_bytes = cmdname.as_bytes();
            let fullname_bytes = fullname.map_or(&b""[..], CStr::to_bytes);
            if !fullname_bytes.starts_with(cmdname_bytes) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Invalid command: \"%s\"",
                    cmdname,
                );
                break 'end;
            }
        }

        if range_only {
            ea.argt = EX_RANGE | EX_SBOXOK;
        } else if !is_user_cmdidx(ea.cmdidx) {
            ea.argt = excmd_get_argt(ea.cmdidx);
        }

        let mut count_from_first_arg = false;
        if has_key(mask, KEYSET_OPTIDX_CMD_ARGS) {
            if cmd.args.size == 1 && (ea.argt & EX_COUNT) != 0 && (ea.argt & EX_EXTRA) == 0 {
                let first_arg = &cmd.args[0];
                let mut is_numeric = false;
                let mut count_value: i64 = 0;

                if first_arg.obj_type == ObjectType::ObjectTypeInteger {
                    is_numeric = true;
                    count_value = first_arg.data.integer();
                } else if first_arg.obj_type == ObjectType::ObjectTypeString {
                    let s = first_arg.data.string();
                    let str_bytes = s.as_bytes();
                    let mut rem = str_bytes;
                    let val = getdigits_long(&mut rem, false, 0);
                    if rem.is_empty() && !str_bytes.is_empty() {
                        is_numeric = true;
                        count_value = val as _;
                    }
                }

                if is_numeric && count_value >= 0 {
                    count_from_first_arg = true;
                    ea.addr_count = 1;
                    ea.line1 = count_value as Linenr;
                    ea.line2 = count_value as Linenr;
                    args = Array::with_capacity(0);
                }
            }

            if !count_from_first_arg {
                args = Array::with_capacity(cmd.args.size);
                for elem in cmd.args.as_slice() {
                    match elem.obj_type {
                        ObjectType::ObjectTypeBoolean => {
                            let b = if elem.data.boolean() { b"1" } else { b"0" };
                            add_c(&mut args, string_obj(NvimString::from_bytes(b)));
                        }
                        ObjectType::ObjectTypeBuffer
                        | ObjectType::ObjectTypeWindow
                        | ObjectType::ObjectTypeTabpage
                        | ObjectType::ObjectTypeInteger => {
                            let s = elem.data.integer().to_string();
                            add_c(&mut args, string_obj(NvimString::from_bytes(s.as_bytes())));
                        }
                        ObjectType::ObjectTypeString => {
                            if string_iswhite(elem.data.string()) {
                                api_err_exp(err, c"command arg", c"non-whitespace", None);
                                break 'end;
                            }
                            add_c(&mut args, elem.clone());
                        }
                        _ => {
                            api_err_exp(
                                err,
                                c"command arg",
                                c"valid type",
                                Some(api_typename(elem.obj_type)),
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
                        ErrorType::ErrorTypeValidation,
                        c"%s",
                        c"Wrong number of arguments",
                    );
                    break 'end;
                }
            }
        }

        if has_key(mask, KEYSET_OPTIDX_CMD_RANGE) {
            if (ea.argt & EX_RANGE) == 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Command cannot accept %s: %s",
                    c"range",
                    cmd.cmd,
                );
                break 'end;
            }
            if cmd.range.size > 2 {
                api_err_exp(err, c"range", c"<=2 elements", None);
                break 'end;
            }

            let range = &cmd.range;
            ea.addr_count = range.size as i32;

            for elem in range.as_slice() {
                if elem.obj_type != ObjectType::ObjectTypeInteger || elem.data.integer() < 0 {
                    api_err_exp(err, c"range element", c"non-negative Integer", None);
                    break 'end;
                }
            }

            if range.size > 0 {
                ea.line1 = range[0].data.integer() as Linenr;
                ea.line2 = range[range.size - 1].data.integer() as Linenr;
            }
        }

        build_cmdline_str(&mut cmdline_buf, &mut ea, args.clone());
        ea.cmdline_buf = std::mem::take(&mut cmdline_buf);
        if !parse_cmdline(&mut ea, &mut cmod, &mut errormsg, true) {
            api_set_error!(
                err,
                ErrorType::ErrorTypeException,
                c"%s",
                errormsg.as_deref().unwrap_or(c"Parsing command-line"),
            );
            break 'end;
        }

        if range_only {
            ea.argt = EX_RANGE | EX_SBOXOK;
        }

        if has_key(mask, KEYSET_OPTIDX_CMD_RANGE) && invalid_range(&mut ea).is_some() {
            api_err_invalid(err, c"range", Some(c""), 0, true);
            break 'end;
        }

        if ea.addr_count == 0 && (ea.argt & EX_DFLALL) == 0 && ea.addr_type == ADDR_OTHER {
            ea.line2 = 1;
        }

        if args.size > 0 {
            ea.args.reserve_exact(args.size);
            ea.arglens.reserve_exact(args.size);
            let mut arg_cur = ea.arg.clone();
            for i in 0..args.size {
                ea.args.push(arg_cur.clone());
                let arglen = args[i].data.string().size;
                ea.arglens.push(arglen);
                arg_cur = arg_cur.wrapping_add(arglen + 1);
            }
        }

        if has_key(mask, KEYSET_OPTIDX_CMD_COUNT) {
            if count_from_first_arg {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Cannot specify both 'count' and numeric argument",
                );
                break 'end;
            }
            if (ea.argt & EX_COUNT) == 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Command cannot accept %s: %s",
                    c"count",
                    cmd.cmd,
                );
                break 'end;
            }
            if cmd.count < 0 {
                api_err_exp(err, c"count", c"non-negative Integer", None);
                break 'end;
            }
            set_cmd_count(&mut ea, cmd.count as Linenr, true);
        }

        if has_key(mask, KEYSET_OPTIDX_CMD_REG) {
            if (ea.argt & EX_REGSTR) == 0 {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Command cannot accept %s: %s",
                    c"register",
                    cmd.cmd,
                );
                break 'end;
            }
            if cmd.reg.size != 1 {
                api_err_exp(err, c"reg", c"single character", cmd.reg.as_opt_cstr());
                break 'end;
            }
            let regname = cmd.reg.first_byte();
            if regname == b'=' {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Cannot use register \"=",
                );
                break 'end;
            }
            if !valid_yank_reg(
                i32::from(regname),
                !is_user_cmdidx(ea.cmdidx)
                    && ea.cmdidx != Cmdidx::CmdPut
                    && ea.cmdidx != Cmdidx::CmdIput,
            ) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeValidation,
                    c"Invalid register: \"%c",
                    i32::from(regname),
                );
                break 'end;
            }
            ea.regname = i32::from(regname);
        }

        ea.forceit = cmd.bang;
        if ea.forceit && (ea.argt & EX_BANG) == 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"Command cannot accept %s: %s",
                c"bang",
                cmd.cmd,
            );
            break 'end;
        }

        if has_key(mask, KEYSET_OPTIDX_CMD_MAGIC) {
            let mut magic = KeyDict_cmd_magic::default();
            if !api_dict_to_keydict(
                &mut magic,
                Some(key_dict_cmd_magic_get_field),
                cmd.magic.clone(),
                err,
            ) {
                break 'end;
            }

            ea.magic.file = if has_key(magic.is_set_cmd_magic, KEYSET_OPTIDX_CMD_MAGIC_FILE) {
                magic.file
            } else {
                (ea.argt & EX_XFILE) != 0
            };
            ea.magic.bar = if has_key(magic.is_set_cmd_magic, KEYSET_OPTIDX_CMD_MAGIC_BAR) {
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

        if has_key(mask, KEYSET_OPTIDX_CMD_MODS) {
            let mut mods = KeyDict_cmd_mods::default();
            if !api_dict_to_keydict(
                &mut mods,
                Some(key_dict_cmd_mods_get_field),
                cmd.mods.clone(),
                err,
            ) {
                break 'end;
            }

            let mods_mask = mods.is_set_cmd_mods;
            if has_key(mods_mask, KEYSET_OPTIDX_CMD_MODS_FILTER) {
                let mut filter = KeyDict_cmd_mods_filter::default();
                if !api_dict_to_keydict(
                    &mut filter,
                    Some(key_dict_cmd_mods_filter_get_field),
                    mods.filter,
                    err,
                ) {
                    break 'end;
                }

                if has_key(
                    filter.is_set_cmd_mods_filter,
                    KEYSET_OPTIDX_CMD_MODS_FILTER_PATTERN,
                ) {
                    cmod.cmod_filter_force = filter.force;
                    if !filter.pattern.cstr_bytes().is_empty() || cmod.cmod_filter_force {
                        let pat = bytes_to_cstring(filter.pattern.as_bytes());
                        cmod.cmod_filter_regmatch
                            .set_prog(vim_regcomp(&pat, RE_MAGIC));
                        cmod.cmod_filter_pat = Some(pat);
                    }
                }
            }

            if has_key(mods_mask, KEYSET_OPTIDX_CMD_MODS_TAB) && (mods.tab as i32) >= 0 {
                cmod.cmod_tab = (mods.tab as i32) + 1;
            }

            if has_key(mods_mask, KEYSET_OPTIDX_CMD_MODS_VERBOSE) && (mods.verbose as i32) >= 0 {
                cmod.cmod_verbose = (mods.verbose as i32) + 1;
            }

            if mods.vertical {
                cmod.cmod_split |= WSP_VERT;
            }
            if mods.horizontal {
                cmod.cmod_split |= WSP_HOR;
            }

            if has_key(mods_mask, KEYSET_OPTIDX_CMD_MODS_SPLIT) {
                match mods.split.cstr_bytes() {
                    b"" => {}
                    b"aboveleft" | b"leftabove" => cmod.cmod_split |= WSP_ABOVE,
                    b"belowright" | b"rightbelow" => cmod.cmod_split |= WSP_BELOW,
                    b"topleft" => cmod.cmod_split |= WSP_TOP,
                    b"botright" => cmod.cmod_split |= WSP_BOT,
                    _ => {
                        api_err_invalid(err, c"mods.split", Some(c""), 0, true);
                        break 'end;
                    }
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
                    ErrorType::ErrorTypeValidation,
                    c"%s",
                    c"Command cannot be run in sandbox",
                );
                break 'end;
            }
        }

        let ea_arg = ea.arg.clone();
        let new_arg = replace_makeprg(&mut ea, ea_arg);
        if new_arg.addr() != ea.arg.addr() {
            ea.arg = new_arg;
            ea.args.clear();
            ea.arglens.clear();
        }

        if (ea.argt & EX_ARGOPT) != 0 {
            while ea.arg.as_bytes().starts_with(b"++") {
                let result = getargopt(&mut ea);
                if !result && !is_cmd_ni(ea.cmdidx) {
                    api_err_invalid(err, c"argument ", ea.arg.as_opt_cstr(), 0, true);
                    break 'end;
                }
            }
        }

        if (ea.argt & EX_CMDARG) != 0 && !ea.usefilter {
            ea.do_ecmd_cmd = getargcmd(&mut ea);
        }

        let mut capture_local: Vec<u8> = Vec::new();
        let save_msg_silent = g_st().msg_silent.get();
        let save_redir_off = g_st().redir_off.get();
        let mut save_capture_ga = g_st().capture_ga.take();
        let save_msg_col = g_st().msg_col.get();

        if opts.output {
            *g_st().capture_ga.borrow_mut() = Some(Vec::new());
        } else {
            *g_st().capture_ga.borrow_mut() = save_capture_ga.take();
        }

        try_wrap(err, || {
            if opts.output {
                g_st().msg_silent.set(g_st().msg_silent.get() + (1));
                g_st().redir_off.set(false);
                g_st().msg_col.set(0);
            }

            with_script_context(channel_id, || {
                execute_cmd(&mut ea, &mut cmod, false);
            });

            if opts.output {
                capture_local = g_st()
                    .capture_ga
                    .replace(save_capture_ga.take())
                    .unwrap_or_default();
                g_st().msg_silent.set(save_msg_silent);
                g_st().redir_off.set(save_redir_off);
                g_st().msg_col.set(save_msg_col);
            }
        });

        if !error_set(err) && opts.output && capture_local.len() > 1 {
            let slice = capture_local.strip_prefix(b"\n").unwrap_or(&capture_local);
            retv = NvimString::from_bytes(slice);
        }
    }

    drop(cmdline_buf);
    retv
}

fn string_iswhite(str_val: NvimString) -> bool {
    str_val
        .cstr_bytes()
        .iter()
        .all(|&b| b == b' ' || b == b'\t')
}

fn build_cmdline_str(cmdline_buf: &mut Vec<u8>, eap: &mut Exarg, args: Array) {
    cmdline_buf.clear();
    cmdline_buf.reserve(32);

    if (eap.argt & EX_RANGE) != 0 {
        if eap.addr_count == 1 {
            let s = eap.line2.to_string();
            cmdline_buf.extend_from_slice(s.as_bytes());
        } else if eap.addr_count > 1 {
            let s = format!("{},{}", eap.line1, eap.line2);
            cmdline_buf.extend_from_slice(s.as_bytes());
        }
    }

    cmdline_buf.extend_from_slice(eap.cmd.as_bytes());

    for i in 0..args.size {
        let s = args[i].data.string();
        cmdline_buf.push(b' ');
        cmdline_buf.extend_from_slice(s.as_bytes());
    }
    cmdline_buf.push(0);
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
pub fn nvim_create_user_command(
    channel_id: u64,
    name: NvimString,
    cmd: Object,
    opts: &mut KeyDict_user_command,
    err: &mut Error,
) {
    create_user_command(channel_id, name, cmd, &mut *opts, 0, err);
}

/// Delete a user-defined command.
///
/// @param  name    Name of the command to delete.
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_del_user_command(String name, Error *err)
//   FUNC_API_SINCE(9);
pub fn nvim_del_user_command(name: NvimString, err: &mut Error) {
    nvim_buf_del_user_command(-1, name, err);
}

/// Creates a buffer-local command |user-commands|.
///
/// @param  buf  Buffer id, or 0 for current buffer.
/// @param[out] err Error details, if any.
/// @see nvim_create_user_command
// DLLEXPORT void nvim_buf_create_user_command(uint64_t channel_id, Buffer buf, String name,
//                                             Object cmd, Dict(user_command) *opts, Error *err)
//   FUNC_API_SINCE(9);
pub fn nvim_buf_create_user_command(
    channel_id: u64,
    buf: Buffer,
    name: NvimString,
    cmd: Object,
    opts: &mut KeyDict_user_command,
    err: &mut Error,
) {
    let Some(target_buf) = find_buffer_by_handle(buf, err) else {
        return;
    };

    let save_curbuf = g_st().curbuf.get();
    g_st().curbuf.set(target_buf);
    create_user_command(channel_id, name, cmd, &mut *opts, UC_BUFFER, err);
    g_st().curbuf.set(save_curbuf);
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
pub fn nvim_buf_del_user_command(buf: Buffer, name: NvimString, err: &mut Error) {
    let name_bytes = name.cstr_bytes();
    let removed = if buf == -1 {
        with_ucmds_mut(|gap| {
            if let Some(i) = gap
                .iter()
                .position(|cmd| cmd.uc_name.as_bytes() == name_bytes)
            {
                let _ = gap.remove(i);
                true
            } else {
                false
            }
        })
    } else {
        let Some(b) = crate::api::private::helpers::find_buffer_by_handle(buf, err) else {
            return;
        };
        let pos = b
            .b_ucmds
            .borrow()
            .iter()
            .position(|cmd| cmd.uc_name.as_bytes() == name_bytes);
        if let Some(i) = pos {
            let _ = b.b_ucmds.borrow_mut().remove(i);
            true
        } else {
            false
        }
    };
    if removed {
        return;
    }

    api_set_error!(
        err,
        ErrorType::ErrorTypeException,
        c"Invalid command (not found): %s",
        name,
    );
}

// DLLEXPORT void create_user_command(uint64_t channel_id, String name, Union(String, LuaRef) cmd,
//                                    Dict(user_command) *opts, int flags, Error *err);
pub fn create_user_command(
    channel_id: u64,
    name: NvimString,
    cmd: Object,
    opts: &mut KeyDict_user_command,
    flags: i32,
    err: &mut Error,
) {
    let mut argt: u32 = 0;
    let mut def: i64 = -1;
    let mut addr_type_arg: CmdAddr = ADDR_NONE;
    let mut context: i32 = EXPAND_NOTHING;
    let mut compl_arg: Option<std::ffi::CString> = None;
    let rep: &CStr;
    let mut luaref: LuaRef = LUA_NOREF;
    let mut compl_luaref: LuaRef = LUA_NOREF;
    let mut preview_luaref: LuaRef = LUA_NOREF;

    'err: {
        if uc_validate_name(name.as_bytes()).is_none() {
            api_err_invalid(err, c"command name", name.as_opt_cstr(), 0, true);
            break 'err;
        }
        if mb_islower(i32::from(name.first_byte())) {
            api_err_invalid(
                err,
                c"command name (must start with uppercase)",
                name.as_opt_cstr(),
                0,
                true,
            );
            break 'err;
        }
        let mask = opts.is_set_user_command;
        if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_RANGE)
            && has_key(mask, KEYSET_OPTIDX_USER_COMMAND_COUNT)
        {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"Cannot use both 'range' and 'count'",
            );
            break 'err;
        }

        if opts.nargs.obj_type == ObjectType::ObjectTypeInteger {
            match opts.nargs.data.integer() {
                0 => {}
                1 => {
                    argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG;
                }
                n => {
                    api_err_invalid(err, c"nargs", None, n, false);
                    break 'err;
                }
            }
        } else if opts.nargs.obj_type == ObjectType::ObjectTypeString {
            if opts.nargs.data.string().size > 1 {
                api_err_invalid(
                    err,
                    c"nargs",
                    opts.nargs.data.string().as_opt_cstr(),
                    0,
                    true,
                );
                break 'err;
            }
            match opts.nargs.data.string().first_byte() {
                b'*' => argt |= EX_EXTRA,
                b'?' => argt |= EX_EXTRA | EX_NOSPC,
                b'+' => argt |= EX_EXTRA | EX_NEEDARG,
                b'_' => argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE,
                _ => {
                    api_err_invalid(
                        err,
                        c"nargs",
                        opts.nargs.data.string().as_opt_cstr(),
                        0,
                        true,
                    );
                    break 'err;
                }
            }
        } else if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_NARGS) {
            api_err_invalid(err, c"nargs", Some(c""), 0, true);
            break 'err;
        }

        if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_COMPLETE) && argt == 0 {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"%s",
                c"'complete' used without 'nargs'",
            );
            break 'err;
        }

        if opts.range.obj_type == ObjectType::ObjectTypeBoolean {
            if opts.range.data.boolean() {
                argt |= EX_RANGE;
                addr_type_arg = ADDR_LINES;
            }
        } else if opts.range.obj_type == ObjectType::ObjectTypeString {
            if !(opts.range.data.string().first_byte() == b'%'
                && opts.range.data.string().size == 1)
            {
                api_err_invalid(err, c"range", Some(c""), 0, true);
                break 'err;
            }
            argt |= EX_RANGE | EX_DFLALL;
            addr_type_arg = ADDR_LINES;
        } else if opts.range.obj_type == ObjectType::ObjectTypeInteger {
            argt |= EX_RANGE | EX_ZEROR;
            def = opts.range.data.integer();
            addr_type_arg = ADDR_LINES;
        } else if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_RANGE) {
            api_err_invalid(err, c"range", Some(c""), 0, true);
            break 'err;
        }

        if opts.count.obj_type == ObjectType::ObjectTypeBoolean {
            if opts.count.data.boolean() {
                argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
                addr_type_arg = ADDR_OTHER;
                def = 0;
            }
        } else if opts.count.obj_type == ObjectType::ObjectTypeInteger {
            argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
            addr_type_arg = ADDR_OTHER;
            def = opts.count.data.integer();
        } else if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_COUNT) {
            api_err_invalid(err, c"count", Some(c""), 0, true);
            break 'err;
        }

        if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_ADDR) {
            if opts.addr.obj_type != ObjectType::ObjectTypeString {
                api_err_exp(
                    err,
                    c"addr",
                    api_typename(ObjectType::ObjectTypeString),
                    Some(api_typename(opts.addr.obj_type)),
                );
                break 'err;
            }
            if !parse_addr_type_arg(opts.addr.data.string().as_bytes(), &mut addr_type_arg) {
                api_err_invalid(err, c"addr", opts.addr.data.string().as_opt_cstr(), 0, true);
                break 'err;
            }
            argt |= EX_RANGE;
            if addr_type_arg != ADDR_LINES {
                argt |= EX_ZEROR;
            }
        }

        if opts.bang {
            argt |= EX_BANG;
        }
        if opts.bar {
            argt |= EX_TRLBAR;
        }
        if opts.register_ {
            argt |= EX_REGSTR;
        }
        if opts.keepscript {
            argt |= EX_KEEPSCRIPT;
        }

        let force = if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_FORCE) {
            opts.force
        } else {
            true
        };
        if error_set(err) {
            break 'err;
        }

        if opts.complete.obj_type == ObjectType::ObjectTypeLuaRef {
            context = EXPAND_USER_LUA;
            compl_luaref = opts.complete.data.luaref();
            opts.complete.data = crate::api::private::validate::ObjectData::LuaRef(LUA_NOREF);
        } else if opts.complete.obj_type == ObjectType::ObjectTypeString {
            if !parse_compl_arg(
                opts.complete.data.string().as_bytes(),
                &mut context,
                &mut argt,
                &mut compl_arg,
            ) {
                api_err_invalid(
                    err,
                    c"complete",
                    opts.complete.data.string().as_opt_cstr(),
                    0,
                    true,
                );
                break 'err;
            }
        } else if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_COMPLETE) {
            api_err_exp(err, c"complete", c"Function or String", None);
            break 'err;
        }

        if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_PREVIEW) {
            if opts.preview.obj_type != ObjectType::ObjectTypeLuaRef {
                api_err_exp(
                    err,
                    c"preview",
                    api_typename(ObjectType::ObjectTypeLuaRef),
                    Some(api_typename(opts.preview.obj_type)),
                );
                break 'err;
            }
            argt |= EX_PREVIEW;
            preview_luaref = opts.preview.data.luaref();
            opts.preview.data = crate::api::private::validate::ObjectData::LuaRef(LUA_NOREF);
        }

        let mut desc: Option<&CStr> = None;
        if has_key(mask, KEYSET_OPTIDX_USER_COMMAND_DESC) {
            if opts.desc.obj_type != ObjectType::ObjectTypeString {
                api_err_exp(
                    err,
                    c"desc",
                    api_typename(ObjectType::ObjectTypeString),
                    Some(api_typename(opts.desc.obj_type)),
                );
                break 'err;
            }
            desc = opts.desc.data.as_opt_cstr();
        }

        match cmd.obj_type {
            ObjectType::ObjectTypeLuaRef => {
                luaref = api_new_luaref(cmd.data.luaref());
                rep = c"";
            }
            ObjectType::ObjectTypeString => {
                rep = cmd.data.as_cstr();
            }
            _ => {
                api_err_exp(err, c"command", c"Function or String", None);
                break 'err;
            }
        }

        with_script_context(channel_id, || {
            if !uc_add_command(
                name.as_bytes(),
                rep.to_bytes(),
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
            ) {
                api_set_error!(
                    err,
                    ErrorType::ErrorTypeException,
                    c"Failed to create user command",
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
pub fn nvim_get_commands(opts: &mut KeyDict_get_commands, err: &mut Error) -> Dict {
    nvim_buf_get_commands(-1, opts, err)
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
pub fn nvim_buf_get_commands(
    buf: Buffer,
    opts: &mut KeyDict_get_commands,
    err: &mut Error,
) -> Dict {
    let global = buf == -1;
    if error_set(err) {
        return Dict::new();
    }

    if global {
        if opts.builtin {
            api_set_error!(
                err,
                ErrorType::ErrorTypeValidation,
                c"builtin=true not implemented",
            );
            return Dict::new();
        }
        return commands_array(None);
    }

    let Some(b) = crate::api::private::helpers::find_buffer_by_handle(buf, err) else {
        return Dict::new();
    };
    if opts.builtin {
        return Dict::new();
    }
    commands_array(Some(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_user_cmdidx_predicates() {
        assert!(is_user_cmdidx(Cmdidx::CmdUser));
        assert!(is_user_cmdidx(Cmdidx::CmdUserBuf));
        assert!(!is_user_cmdidx(Cmdidx::CmdSize));
        assert!(ascii_isupper(i32::from(b'A')));
        assert!(!ascii_isupper(i32::from(b'a')));
        assert!(string_iswhite(NvimString::from_bytes(b" \t")));
        assert!(!string_iswhite(NvimString::from_bytes(b" x")));
    }
}
