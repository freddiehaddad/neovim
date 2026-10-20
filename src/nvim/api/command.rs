//! Command API functions (`src/nvim/api/command.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

#[allow(unused_imports)]
use crate::api::private::helpers::add_c;
#[allow(unused_imports)]
use crate::api::private::helpers::api_set_error;
#[allow(unused_imports)]
use crate::api::private::helpers::api_typename;
#[allow(unused_imports)]
use crate::api::private::helpers::{
    api_dict_to_keydict, api_set_sctx, arena_array, arena_dict, arena_string,
    find_buffer_by_handle, try_enter, try_leave,
};
#[allow(unused_imports)]
use crate::api::private::helpers::{
    boolean_obj, cstr_as_string, dict_obj, integer_obj, string_obj,
};
#[allow(unused_imports)]
use crate::autocmd::{apply_autocmds, has_event};
#[allow(unused_imports)]
use crate::charset::ascii_iswhite;
#[allow(unused_imports)]
use crate::charset::getdigits_long;
#[allow(unused_imports)]
use crate::ex_docmd::get_command_name;
#[allow(unused_imports)]
use crate::ex_docmd::invalid_range;
#[allow(unused_imports)]
use crate::ex_docmd::{excmd_get_argt, is_cmd_ni, is_map_cmd};
#[allow(unused_imports)]
use crate::ex_docmd::{
    execute_cmd, find_ex_command, getargopt, parse_cmdline, replace_makeprg, set_cmd_count,
};
#[allow(unused_imports)]
use crate::ex_docmd::{getargcmd, undo_cmdmod};
#[allow(unused_imports)]
use crate::ex_eval::aborting;
use crate::globals::*;
#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, api_new_luaref};
#[allow(unused_imports)]
use crate::mbyte::mb_islower;
#[allow(unused_imports)]
use crate::regexp::vim_regcomp;
#[allow(unused_imports)]
use crate::register::valid_yank_reg;
#[allow(unused_imports)]
use crate::usercmd::get_user_command_name;
use crate::usercmd::ucmds;
#[allow(unused_imports)]
use crate::usercmd::{
    commands_array, free_ucmd, parse_addr_type_arg, parse_compl_arg, uc_add_command,
    uc_nargs_upper_bound, uc_split_args_iter, uc_validate_name,
};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::deprecated::{KeyDict_empty, StringBuilder};
use crate::api::private::converter::sctx_T;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectType, api_err_exp, api_err_invalid,
    api_err_required,
};
use crate::api::tabpage::Buffer;
use crate::garray::{ga_clear, ga_init, garray_T};
use crate::lua::converter::{Boolean, LuaRef};
use crate::map_glyph_cache::NvimString;
use crate::memory::{Arena, arena_alloc, arena_memdupz, xcalloc, xfree, xmemdupz};
use crate::msgpack_rpc::packer::Integer;

pub use crate::cursor::colnr_T;
pub use crate::cursor::linenr_T;
pub type cmdidx_T = c_int;
pub use crate::api::autocmd::event_T;
pub use crate::cmdexpand::cmd_addr_T;

const OK: c_int = 1;
const FAIL: c_int = 0;
const NUL: c_char = 0;
const LUA_NOREF: LuaRef = -2;
const RE_MAGIC: c_int = 1;
const UC_BUFFER: c_int = 1;

const CMD_iput: cmdidx_T = 200;
const CMD_put: cmdidx_T = 350;
const CMD_SIZE: cmdidx_T = 564;
const CMD_USER: cmdidx_T = -1;
const CMD_USER_BUF: cmdidx_T = -2;

const EVENT_CMDUNDEFINED: event_T = 30;

const EXPAND_NOTHING: c_int = 0;
const EXPAND_USER_LUA: c_int = 32;

const EX_RANGE: u32 = 0x001;
const EX_BANG: u32 = 0x002;
const EX_EXTRA: u32 = 0x004;
const EX_XFILE: u32 = 0x008;
const EX_NOSPC: u32 = 0x010;
const EX_DFLALL: u32 = 0x020;
const EX_NEEDARG: u32 = 0x080;
const EX_TRLBAR: u32 = 0x100;
const EX_REGSTR: u32 = 0x200;
const EX_COUNT: u32 = 0x400;
const EX_ZEROR: u32 = 0x1000;
const EX_CMDARG: u32 = 0x4000;
const EX_ARGOPT: u32 = 0x20000;
const EX_SBOXOK: u32 = 0x40000;
const EX_KEEPSCRIPT: u32 = 0x4000000;
const EX_PREVIEW: u32 = 0x8000000;
const EX_ARGSPACE: u32 = 0x40000000;

const ADDR_LINES: cmd_addr_T = 0;
const ADDR_WINDOWS: cmd_addr_T = 1;
const ADDR_ARGUMENTS: cmd_addr_T = 2;
const ADDR_LOADED_BUFFERS: cmd_addr_T = 3;
const ADDR_BUFFERS: cmd_addr_T = 4;
const ADDR_TABS: cmd_addr_T = 5;
const ADDR_QUICKFIX: cmd_addr_T = 8;
const ADDR_OTHER: cmd_addr_T = 10;
const ADDR_NONE: cmd_addr_T = 11;

const CMOD_SANDBOX: c_int = 0x0001;
const CMOD_SILENT: c_int = 0x0002;
const CMOD_ERRSILENT: c_int = 0x0004;
const CMOD_UNSILENT: c_int = 0x0008;
const CMOD_NOAUTOCMD: c_int = 0x0010;
const CMOD_HIDE: c_int = 0x0020;
const CMOD_BROWSE: c_int = 0x0040;
const CMOD_CONFIRM: c_int = 0x0080;
const CMOD_KEEPALT: c_int = 0x0100;
const CMOD_KEEPMARKS: c_int = 0x0200;
const CMOD_KEEPJUMPS: c_int = 0x0400;
const CMOD_LOCKMARKS: c_int = 0x0800;
const CMOD_KEEPPATTERNS: c_int = 0x1000;
const CMOD_NOSWAPFILE: c_int = 0x2000;

const WSP_VERT: c_int = 0x02;
const WSP_HOR: c_int = 0x04;
const WSP_TOP: c_int = 0x08;
const WSP_BOT: c_int = 0x10;
const WSP_BELOW: c_int = 0x40;
const WSP_ABOVE: c_int = 0x80;

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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExMagic {
    pub file: bool,
    pub bar: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct exarg_T {
    pub arg: *mut c_char,
    pub args: *mut *mut c_char,
    pub arglens: *mut usize,
    pub argc: usize,
    pub nextcmd: *mut c_char,
    pub cmd: *mut c_char,
    pub cmdlinep: *mut *mut c_char,
    pub cmdline_tofree: *mut c_char,
    pub cmdidx: cmdidx_T,
    pub argt: u32,
    pub skip: c_int,
    pub forceit: c_int,
    pub addr_count: c_int,
    pub line1: linenr_T,
    pub line2: linenr_T,
    pub addr_type: cmd_addr_T,
    pub flags: c_int,
    pub do_ecmd_cmd: *mut c_char,
    pub do_ecmd_lnum: linenr_T,
    pub append: c_int,
    pub usefilter: c_int,
    pub amount: c_int,
    pub regname: c_int,
    pub force_bin: c_int,
    pub read_edit: c_int,
    pub mkdir_p: c_int,
    pub force_ff: c_int,
    pub force_enc: c_int,
    pub bad_char: c_int,
    pub useridx: c_int,
    pub errmsg: *mut c_char,
    pub ea_getline: crate::eval::vars::LineGetter,
    pub cookie: *mut c_void,
    pub cstack: *mut c_void,
    pub magic: ExMagic,
}

pub use crate::regexp::regmatch_T;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct cmdmod_T {
    pub cmod_flags: c_int,
    pub cmod_split: c_int,
    pub cmod_tab: c_int,
    pub cmod_filter_pat: *mut c_char,
    pub cmod_filter_regmatch: regmatch_T,
    pub cmod_filter_force: bool,
    pub cmod_verbose: c_int,
    pub cmod_save_ei: *mut c_char,
    pub cmod_did_sandbox: c_int,
    pub cmod_verbose_save: i64,
    pub cmod_save_msg_silent: c_int,
    pub cmod_save_msg_scroll: c_int,
    pub cmod_did_esilent: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ucmd_T {
    pub uc_name: *mut c_char,
    pub uc_argt: u32,
    pub uc_rep: *mut c_char,
    pub uc_def: i64,
    pub uc_compl: c_int,
    pub uc_addr_type: cmd_addr_T,
    pub uc_script_ctx: sctx_T,
    pub uc_compl_arg: *mut c_char,
    pub uc_compl_luaref: LuaRef,
    pub uc_preview_luaref: LuaRef,
    pub uc_luaref: LuaRef,
    pub uc_desc: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct KeyDict_cmd_magic {
    pub is_set__cmd_magic_: OptionalKeys,
    pub file: Boolean,
    pub bar: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct KeyDict_cmd_mods_filter {
    pub is_set__cmd_mods_filter_: OptionalKeys,
    pub pattern: NvimString,
    pub force: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyDict_cmd_opts {
    pub output: Boolean,
}

#[repr(C)]
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct KeyDict_get_commands {
    pub builtin: Boolean,
}

use crate::api::private::dispatch::{
    KeyDict_cmd_magic_get_field, KeyDict_cmd_mods_filter_get_field, KeyDict_cmd_mods_get_field,
};

#[inline]
unsafe fn cstrn_as_string(s: *const c_char, maxsize: usize) -> NvimString {
    if s.is_null() {
        return NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
    }
    let slice = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), maxsize) };
    let len = slice.iter().position(|&b| b == 0).unwrap_or(maxsize);
    NvimString {
        data: s as *mut c_char,
        size: len,
    }
}

#[inline]
const fn cstr_literal(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
unsafe fn cbuf_to_arena_str(arena: *mut Arena, buf: *const c_char, len: usize) -> NvimString {
    unsafe {
        NvimString {
            data: arena_memdupz(arena, buf, len),
            size: len,
        }
    }
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

#[inline]
unsafe fn string_to_cstr(str_val: NvimString) -> *mut c_char {
    unsafe { xmemdupz(str_val.data.cast(), str_val.size).cast() }
}

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
const fn is_user_cmdidx(cmdidx: cmdidx_T) -> bool {
    cmdidx < 0
}

#[inline]
const fn ascii_isupper(c: c_char) -> bool {
    let b = c as u8;
    b.is_ascii_uppercase()
}

#[inline]
unsafe fn skipwhite(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        while ascii_iswhite(*p) {
            p = p.add(1);
        }
        p
    }
}

#[inline]
unsafe fn skiptowhite(mut p: *mut c_char) -> *mut c_char {
    unsafe {
        while *p != NUL && !ascii_iswhite(*p) {
            p = p.add(1);
        }
        p
    }
}

#[inline]
unsafe fn strequal(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_literal(key),
            value,
        };
        dict.size += 1;
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

unsafe fn with_script_context<F: FnOnce()>(channel_id: u64, f: F) {
    unsafe {
        let save_current_sctx = api_set_sctx(channel_id);
        f();
        current_sctx = save_current_sctx;
    }
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_parse_cmd(
    str_val: NvimString,
    _opts: *mut KeyDict_empty,
    arena: *mut Arena,
    err: *mut Error,
) -> KeyDict_cmd {
    unsafe {
        let mut result: KeyDict_cmd = std::mem::zeroed();
        let mut ea: exarg_T = std::mem::zeroed();
        let mut cmod: cmdmod_T = std::mem::zeroed();
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
            && is_map_cmd(std::mem::transmute::<c_int, crate::cmdexpand::cmdidx_T>(
                ea.cmdidx,
            ))
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
            ucmds.ga_data.cast::<ucmd_T>().add(ea.useridx as usize)
        } else if ea.cmdidx == CMD_USER_BUF {
            (*curbuf)
                .b_ucmds
                .ga_data
                .cast::<ucmd_T>()
                .add(ea.useridx as usize)
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
        put_c(&mut mods, c"split", string_obj(cstr_literal(split)));

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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_cmd(
    channel_id: u64,
    cmd: *mut KeyDict_cmd,
    opts: *mut KeyDict_cmd_opts,
    arena: *mut Arena,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let mut ea: exarg_T = std::mem::zeroed();
        let mut cmod: cmdmod_T = std::mem::zeroed();
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

            if !(range_only
                || !is_cmd_ni(std::mem::transmute::<c_int, crate::cmdexpand::cmdidx_T>(
                    ea.cmdidx,
                )))
            {
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
                ea.argt = excmd_get_argt(std::mem::transmute::<c_int, crate::cmdexpand::cmdidx_T>(
                    ea.cmdidx,
                ));
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
                            count_value = i64::from(val);
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
                ea.args = xcalloc(ea.argc, std::mem::size_of::<*mut c_char>()).cast();
                ea.arglens = xcalloc(ea.argc, std::mem::size_of::<usize>()).cast();
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
                let mut magic: KeyDict_cmd_magic = std::mem::zeroed();
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
                let mut mods: KeyDict_cmd_mods = std::mem::zeroed();
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
                    let mut filter: KeyDict_cmd_mods_filter = std::mem::zeroed();
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
                    } else if strequal(mods.split.data, "aboveleft")
                        || strequal(mods.split.data, "leftabove")
                    {
                        cmod.cmod_split |= WSP_ABOVE;
                    } else if strequal(mods.split.data, "belowright")
                        || strequal(mods.split.data, "rightbelow")
                    {
                        cmod.cmod_split |= WSP_BELOW;
                    } else if strequal(mods.split.data, "topleft") {
                        cmod.cmod_split |= WSP_TOP;
                    } else if strequal(mods.split.data, "botright") {
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
                        && !is_cmd_ni(std::mem::transmute::<c_int, crate::cmdexpand::cmdidx_T>(
                            ea.cmdidx,
                        ))
                    {
                        api_err_invalid(err, c"argument ".as_ptr(), orig_arg, 0, true);
                        break 'end;
                    }
                }
            }

            if (ea.argt & EX_CMDARG) != 0 && ea.usefilter == 0 {
                ea.do_ecmd_cmd = getargcmd(&mut ea.arg);
            }

            let mut capture_local: garray_T = std::mem::zeroed();
            let save_msg_silent = msg_silent;
            let save_redir_off = redir_off;
            let save_capture_ga = capture_ga;
            let save_msg_col = msg_col;

            if (*opts).output {
                ga_init(&mut capture_local, 1, 80);
                capture_ga = &mut capture_local;
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

            if !error_set(err) && (*opts).output && capture_local.ga_len > 1 {
                retv = cbuf_to_arena_str(
                    arena,
                    capture_local.ga_data.cast::<c_char>(),
                    capture_local.ga_len as usize,
                );
                if *retv.data == b'\n' as c_char {
                    retv.data = retv.data.add(1);
                    retv.size -= 1;
                }
            }

            if (*opts).output {
                ga_clear(&mut capture_local);
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
        let mut cmdline = StringBuilder::new();
        cmdline.ensure_space(32);

        if ((*eap).argt & EX_RANGE) != 0 {
            if (*eap).addr_count == 1 {
                let s = (*eap).line2.to_string();
                cmdline.concat_len(s.as_ptr().cast::<c_char>(), s.len());
            } else if (*eap).addr_count > 1 {
                let s = format!("{},{}", (*eap).line1, (*eap).line2);
                cmdline.concat_len(s.as_ptr().cast::<c_char>(), s.len());
            }
        }

        let cmd_bytes = CStr::from_ptr((*eap).cmd).to_bytes();
        cmdline.concat_len(cmd_bytes.as_ptr().cast::<c_char>(), cmd_bytes.len());

        for i in 0..args.size {
            let s = (*args.items.add(i)).data.string;
            cmdline.push(b' ' as c_char);
            cmdline.concat_len(s.data, s.size);
        }

        cmdline.push(NUL);
        *cmdlinep = cmdline.items;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_del_user_command(name: NvimString, err: *mut Error) {
    unsafe {
        nvim_buf_del_user_command(-1, name, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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

#[cfg_attr(not(test), unsafe(no_mangle))]
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
        for i in 0..(*gap).ga_len {
            let cmd = (*gap).ga_data.cast::<ucmd_T>().add(i as usize);
            if CStr::from_ptr((*cmd).uc_name).to_bytes() == name_bytes {
                free_ucmd(cmd);
                (*gap).ga_len -= 1;
                if i < (*gap).ga_len {
                    ptr::copy(cmd.add(1), cmd, ((*gap).ga_len - i) as usize);
                }
                return;
            }
        }

        api_set_error!(
            err,
            ErrorType::kErrorTypeException,
            c"Invalid command (not found): %s".as_ptr(),
            name.data,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn create_user_command(
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_commands(
    opts: *mut KeyDict_get_commands,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe { nvim_buf_get_commands(-1, opts, arena, err) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
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
