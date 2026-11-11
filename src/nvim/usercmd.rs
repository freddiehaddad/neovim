//! User defined command support (`src/nvim/usercmd.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::{
    arena_bytes, arena_dict, boolean_obj, cstr_as_obj, dict_obj, integer_obj, luaref_obj, nil_obj,
    put_c, string_obj,
};
use crate::api::private::validate::{Dict, KeyValuePair, NvimString, Object};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, HLF_8};
use crate::buffer::FileBuf;
use crate::charset::{ascii_isupper, ascii_iswhite, getdigits_int};
use crate::cmdexpand::Cmdidx;
use crate::cmdexpand::{
    CmdAddr, EXPAND_ARGLIST, EXPAND_AUGROUP, EXPAND_BREAKPOINT, EXPAND_BUFFERS, EXPAND_CHECKHEALTH,
    EXPAND_COLORS, EXPAND_COMMANDS, EXPAND_COMPILER, EXPAND_DIFF_BUFFERS, EXPAND_DIRECTORIES,
    EXPAND_DIRS_IN_CDPATH, EXPAND_ENV_VARS, EXPAND_EVENTS, EXPAND_EXPRESSION, EXPAND_FILES,
    EXPAND_FILES_IN_PATH, EXPAND_FILETYPE, EXPAND_FILETYPECMD, EXPAND_FUNCTIONS, EXPAND_HELP,
    EXPAND_HIGHLIGHT, EXPAND_HISTORY, EXPAND_KEYMAP, EXPAND_LOCALES, EXPAND_LUA, EXPAND_MAPCLEAR,
    EXPAND_MAPPINGS, EXPAND_MENUS, EXPAND_MESSAGES, EXPAND_NOTHING, EXPAND_OWNSYNTAX,
    EXPAND_PACKADD, EXPAND_RETAB, EXPAND_RUNTIME, EXPAND_SCRIPTNAMES, EXPAND_SETTINGS,
    EXPAND_SHELLCMD, EXPAND_SHELLCMDLINE, EXPAND_SIGN, EXPAND_SYNTIME, EXPAND_TAGS,
    EXPAND_TAGS_LISTFILES, EXPAND_UNSUCCESSFUL, EXPAND_USER, EXPAND_USER_ADDR_TYPE,
    EXPAND_USER_CMD_FLAGS, EXPAND_USER_COMMANDS, EXPAND_USER_COMPLETE, EXPAND_USER_DEFINED,
    EXPAND_USER_LIST, EXPAND_USER_LUA, EXPAND_USER_NARGS, EXPAND_USER_VARS, HLF_D,
};
use crate::eval::funcs::DOCMD_KEYTYPED;
use crate::eval::last_set_msg;
use crate::eval::typval::nlua_clear_ref;
use crate::ex_docmd::{
    ADDR_ARGUMENTS, ADDR_BUFFERS, ADDR_LINES, ADDR_LOADED_BUFFERS, ADDR_NONE, ADDR_OTHER,
    ADDR_QUICKFIX, ADDR_TABS, ADDR_WINDOWS, CMOD_BROWSE, CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE,
    CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS, CMOD_KEEPPATTERNS, CMOD_LOCKMARKS,
    CMOD_NOAUTOCMD, CMOD_NOSWAPFILE, CMOD_SANDBOX, CMOD_SILENT, CMOD_UNSILENT, Cmdmod, EX_ARGSPACE,
    EX_BANG, EX_BUFNAME, EX_COUNT, EX_DFLALL, EX_EXTRA, EX_KEEPSCRIPT, EX_NEEDARG, EX_NOSPC,
    EX_RANGE, EX_REGSTR, EX_TRLBAR, EX_XFILE, EX_ZEROR, Exarg, do_cmdline, ends_excmd,
};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::keycodes::{K_SPECIAL, KE_FILLER, KS_SPECIAL, replace_termcodes};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_new_luaref, nlua_do_ucmd, nlua_funcref_str, nlua_set_sctx};
use crate::mapping::set_context_in_map_cmd;
use crate::memory::Arena;
use crate::menu::set_context_in_menu_cmd;
use crate::message::{
    emsg, message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_outtrans_special, msg_putchar,
    msg_puts, msg_puts_hl, msg_puts_title, semsg,
};
use crate::option_vars::*;
use crate::os::env::ExpandCtx;
use crate::os::input::line_breakcheck;
use crate::runtime::sourcing_lnum;
use crate::strings::gettext;
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT, prevwin_curwin, tabpage_index,
};
use std::ffi::{CStr, CString};

pub(crate) const UC_BUFFER: i32 = 1;

#[repr(C)]
pub struct Ucmd {
    pub uc_name: CString,
    pub uc_argt: u32,
    pub uc_rep: CString,
    pub uc_def: i64,
    pub uc_compl: i32,
    pub uc_addr_type: CmdAddr,
    pub uc_script_ctx: crate::runtime::Sctx,
    pub uc_compl_arg: Option<CString>,
    pub uc_compl_luaref: LuaRef,
    pub uc_preview_luaref: LuaRef,
    pub uc_luaref: LuaRef,
    pub uc_desc: Option<CString>,
}

impl Drop for Ucmd {
    fn drop(&mut self) {
        nlua_clear_ref(&mut self.uc_compl_luaref);
        nlua_clear_ref(&mut self.uc_luaref);
        nlua_clear_ref(&mut self.uc_preview_luaref);
    }
}

impl Ucmd {
    pub fn new() -> Self {
        Self {
            uc_name: CString::default(),
            uc_argt: 0,
            uc_rep: CString::default(),
            uc_def: 0,
            uc_compl: 0,
            uc_addr_type: CmdAddr::Lines,
            uc_script_ctx: crate::runtime::Sctx::new(),
            uc_compl_arg: None,
            uc_compl_luaref: 0,
            uc_preview_luaref: 0,
            uc_luaref: 0,
            uc_desc: None,
        }
    }
}

impl Default for Ucmd {
    fn default() -> Self {
        Self::new()
    }
}

const E_ARGUMENT_REQUIRED_FOR_STR: &CStr = c"E179: Argument required for %s";
const E_NO_SUCH_USER_DEFINED_COMMAND_STR: &CStr = c"E184: No such user-defined command: %s";
const E_COMPLETE_USED_WITHOUT_ALLOWING_ARGUMENTS: &CStr =
    c"E1208: -complete used without allowing arguments";
const E_NO_SUCH_USER_DEFINED_COMMAND_IN_CURRENT_BUFFER_STR: &CStr =
    c"E1237: No such user-defined command in current buffer: %s";

thread_local! {
    static UCMDS_STATE: std::cell::RefCell<Vec<Ucmd>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

#[inline]
pub(crate) fn with_ucmds<R>(f: impl FnOnce(&[Ucmd]) -> R) -> R {
    UCMDS_STATE.with(|cell| f(&cell.borrow()))
}

#[inline]
pub(crate) fn with_ucmds_mut<R>(f: impl FnOnce(&mut Vec<Ucmd>) -> R) -> R {
    UCMDS_STATE.with(|cell| f(&mut cell.borrow_mut()))
}

const COMMAND_COMPLETE_LEN: usize = 64;

fn command_complete_entry(idx: i32) -> Option<&'static CStr> {
    match idx {
        EXPAND_ARGLIST => Some(c"arglist"),
        EXPAND_AUGROUP => Some(c"augroup"),
        EXPAND_BUFFERS => Some(c"buffer"),
        EXPAND_CHECKHEALTH => Some(c"checkhealth"),
        EXPAND_COLORS => Some(c"color"),
        EXPAND_COMMANDS => Some(c"command"),
        EXPAND_COMPILER => Some(c"compiler"),
        EXPAND_USER_DEFINED => Some(c"custom"),
        EXPAND_USER_LIST => Some(c"customlist"),
        EXPAND_USER_LUA => Some(c"<Lua function>"),
        EXPAND_DIFF_BUFFERS => Some(c"diff_buffer"),
        EXPAND_DIRECTORIES => Some(c"dir"),
        EXPAND_ENV_VARS => Some(c"environment"),
        EXPAND_EVENTS => Some(c"event"),
        EXPAND_EXPRESSION => Some(c"expression"),
        EXPAND_FILES => Some(c"file"),
        EXPAND_FILES_IN_PATH => Some(c"file_in_path"),
        EXPAND_FILETYPE => Some(c"filetype"),
        EXPAND_FILETYPECMD => Some(c"filetypecmd"),
        EXPAND_FUNCTIONS => Some(c"function"),
        EXPAND_HELP => Some(c"help"),
        EXPAND_HIGHLIGHT => Some(c"highlight"),
        EXPAND_HISTORY => Some(c"history"),
        EXPAND_KEYMAP => Some(c"keymap"),
        EXPAND_LOCALES => Some(c"locale"),
        EXPAND_LUA => Some(c"lua"),
        EXPAND_MAPCLEAR => Some(c"mapclear"),
        EXPAND_MAPPINGS => Some(c"mapping"),
        EXPAND_MENUS => Some(c"menu"),
        EXPAND_MESSAGES => Some(c"messages"),
        EXPAND_OWNSYNTAX => Some(c"syntax"),
        EXPAND_SYNTIME => Some(c"syntime"),
        EXPAND_SETTINGS => Some(c"option"),
        EXPAND_PACKADD => Some(c"packadd"),
        EXPAND_RETAB => Some(c"retab"),
        EXPAND_RUNTIME => Some(c"runtime"),
        EXPAND_SHELLCMD => Some(c"shellcmd"),
        EXPAND_SHELLCMDLINE => Some(c"shellcmdline"),
        EXPAND_SIGN => Some(c"sign"),
        EXPAND_TAGS => Some(c"tag"),
        EXPAND_TAGS_LISTFILES => Some(c"tag_listfiles"),
        EXPAND_USER => Some(c"user"),
        EXPAND_USER_VARS => Some(c"var"),
        EXPAND_BREAKPOINT => Some(c"breakpoint"),
        EXPAND_SCRIPTNAMES => Some(c"scriptnames"),
        EXPAND_DIRS_IN_CDPATH => Some(c"dir_in_path"),
        _ => None,
    }
}

struct AddrTypeComplete {
    expand: CmdAddr,
    name: Option<&'static CStr>,
    shortname: Option<&'static CStr>,
}

const ADDR_TYPE_COMPLETE: [AddrTypeComplete; 9] = [
    AddrTypeComplete {
        expand: ADDR_ARGUMENTS,
        name: Some(c"arguments"),
        shortname: Some(c"arg"),
    },
    AddrTypeComplete {
        expand: ADDR_LINES,
        name: Some(c"lines"),
        shortname: Some(c"line"),
    },
    AddrTypeComplete {
        expand: ADDR_LOADED_BUFFERS,
        name: Some(c"loaded_buffers"),
        shortname: Some(c"load"),
    },
    AddrTypeComplete {
        expand: ADDR_TABS,
        name: Some(c"tabs"),
        shortname: Some(c"tab"),
    },
    AddrTypeComplete {
        expand: ADDR_BUFFERS,
        name: Some(c"buffers"),
        shortname: Some(c"buf"),
    },
    AddrTypeComplete {
        expand: ADDR_WINDOWS,
        name: Some(c"windows"),
        shortname: Some(c"win"),
    },
    AddrTypeComplete {
        expand: ADDR_QUICKFIX,
        name: Some(c"quickfix"),
        shortname: Some(c"qf"),
    },
    AddrTypeComplete {
        expand: ADDR_OTHER,
        name: Some(c"other"),
        shortname: Some(c"?"),
    },
    AddrTypeComplete {
        expand: ADDR_NONE,
        name: None,
        shortname: None,
    },
];

/// Search for a user command that matches `eap->cmd` (`find_ucmd`).
pub fn find_ucmd(
    eap: &mut Exarg,
    len: usize,
    mut full: Option<&mut bool>,
    mut xp: Option<&mut ExpandCtx>,
    mut complp: Option<&mut i32>,
) -> Option<usize> {
    let len = len as i32;
    let mut matchlen: i32 = 0;
    let mut found = false;
    let mut possible = false;
    let mut amb_local = false;

    let mut is_global = false;
    let cmd_all = eap.cmd.as_bytes();
    let cmd_bytes = &cmd_all[..(len.max(0) as usize).min(cmd_all.len())];
    with_ucmds(|global_ucmds| {
        let mut gap: &[Ucmd] = &prevwin_curwin().buf_mut().b_ucmds.borrow();
        loop {
            let mut j: i32 = 0;
            while (j as usize) < gap.len() {
                let uc = &gap[j as usize];
                let name_bytes = uc.uc_name.as_bytes();
                let k = cmd_bytes
                    .iter()
                    .zip(name_bytes)
                    .take_while(|(a, b)| a == b)
                    .count() as i32;
                let np_empty = (k as usize) >= name_bytes.len();
                if k == len || (np_empty && cmd_all.get(k as usize).is_some_and(u8::is_ascii_digit))
                {
                    if k == len && found && !np_empty {
                        if is_global {
                            return None;
                        }
                        amb_local = true;
                    }

                    if !found || (k == len && np_empty) {
                        if k == len {
                            found = true;
                        } else {
                            possible = true;
                        }

                        if is_global {
                            eap.cmdidx = Cmdidx::CmdUser;
                        } else {
                            eap.cmdidx = Cmdidx::CmdUserBuf;
                        }
                        eap.argt = uc.uc_argt;
                        eap.useridx = j;
                        eap.addr_type = uc.uc_addr_type;

                        if let Some(ref mut p) = complp {
                            **p = uc.uc_compl;
                        }
                        if xp.is_some() {
                            xp.as_deref_mut().unwrap().xp_luaref = uc.uc_compl_luaref;
                            xp.as_deref_mut().unwrap().xp_arg = uc.uc_compl_arg.clone();
                            xp.as_deref_mut().unwrap().xp_script_ctx = uc.uc_script_ctx;
                            xp.as_deref_mut().unwrap().xp_script_ctx.sc_lnum += sourcing_lnum();
                        }
                        matchlen = k;
                        if k == len && np_empty {
                            if let Some(ref mut p) = full {
                                **p = true;
                            }
                            amb_local = false;
                            break;
                        }
                    }
                }
                j += 1;
            }

            if (j as usize) < gap.len() || is_global {
                break;
            }
            is_global = true;
            gap = global_ucmds;
        }

        if amb_local {
            if let Some(xp) = xp {
                xp.xp_context = EXPAND_UNSUCCESSFUL;
            }
            return None;
        }

        if found || possible {
            return Some(matchlen as usize);
        }
        Some(len as usize)
    })
}

/// Set completion context for `:command` (`set_context_in_user_cmd`).
pub fn set_context_in_user_cmd<'a>(xp: &mut ExpandCtx, arg_in: &'a CStr) -> Option<&'a CStr> {
    let full_bytes = arg_in.to_bytes();
    let mut rem = full_bytes;

    while let Some(after_dash) = rem.strip_prefix(b"-") {
        rem = after_dash;
        let arg_cstr = crate::memory::CStrSubslice::advance(arg_in, full_bytes.len() - rem.len());
        let p_rem = crate::charset::skiptowhite_slice(rem);
        if p_rem.is_empty() {
            let Some(eq_idx) = rem.iter().position(|&b| b == b'=') else {
                xp.xp_context = EXPAND_USER_CMD_FLAGS;
                xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg_cstr);
                return None;
            };
            let attr = &rem[..eq_idx];
            let pat_ptr = crate::strings::OwnedSlice::from_cstr(
                crate::memory::CStrSubslice::advance(arg_cstr, eq_idx + 1),
            );
            if attr.len() <= 8 && attr.eq_ignore_ascii_case(&b"complete"[..attr.len()]) {
                xp.xp_context = EXPAND_USER_COMPLETE;
                xp.xp_pattern = pat_ptr;
            } else if attr.len() <= 5 && attr.eq_ignore_ascii_case(&b"nargs"[..attr.len()]) {
                xp.xp_context = EXPAND_USER_NARGS;
                xp.xp_pattern = pat_ptr;
            } else if attr.len() <= 4 && attr.eq_ignore_ascii_case(&b"addr"[..attr.len()]) {
                xp.xp_context = EXPAND_USER_ADDR_TYPE;
                xp.xp_pattern = pat_ptr;
            }
            return None;
        }
        rem = crate::charset::skipwhite_slice(p_rem);
    }

    let arg_cstr = crate::memory::CStrSubslice::advance(arg_in, full_bytes.len() - rem.len());
    let p_rem = crate::charset::skiptowhite_slice(rem);
    if p_rem.is_empty() {
        xp.xp_context = EXPAND_USER_COMMANDS;
        xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg_cstr);
        return None;
    }

    let rest = crate::charset::skipwhite_slice(p_rem);
    Some(crate::memory::CStrSubslice::advance(
        arg_in,
        full_bytes.len() - rest.len(),
    ))
}

/// Set the completion context for the argument of a user defined command (`set_context_in_user_cmdarg`).
pub fn set_context_in_user_cmdarg<'a>(
    cmd: &CStr,
    arg_in: &'a CStr,
    argt: u32,
    context: i32,
    xp: &mut ExpandCtx,
    forceit: bool,
) -> Option<&'a CStr> {
    let mut arg = arg_in;
    if context == EXPAND_NOTHING || (argt & EX_XFILE) != 0 {
        return None;
    }

    if context == EXPAND_MENUS {
        set_context_in_menu_cmd(xp, cmd, arg, forceit);
        return None;
    }
    if context == EXPAND_COMMANDS {
        return Some(arg_in);
    }
    if context == EXPAND_MAPPINGS {
        return set_context_in_map_cmd(
            xp,
            c"map",
            arg_in,
            forceit,
            false,
            false,
            crate::cmdexpand::Cmdidx::CmdMap,
        );
    }
    if (argt & EX_ARGSPACE) == 0 {
        let full_bytes = arg_in.to_bytes();
        let mut rem = full_bytes;
        while !rem.is_empty() {
            if rem[0] == b' ' {
                arg =
                    crate::memory::CStrSubslice::advance(arg_in, full_bytes.len() - rem.len() + 1);
            } else if rem[0] == b'\\' && rem.len() >= 2 {
                rem = &rem[1..];
            }
            let clen = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
            rem = &rem[clen..];
        }
    }
    xp.xp_pattern = crate::strings::OwnedSlice::from_cstr(arg);
    xp.xp_context = context;

    None
}

pub fn expand_user_command_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    get_user_commands(xp, idx - Cmdidx::CmdSize as i32)
}

/// Function given to `ExpandGeneric()` to obtain the list of user command names (`get_user_commands`).
pub fn get_user_commands(xp: &mut ExpandCtx, mut idx: i32) -> Option<&CStr> {
    let buf = prevwin_curwin().buf_mut();
    let b_len = buf.b_ucmds.borrow().len() as i32;

    if idx < b_len {
        return Some(xp.set_item(buf.b_ucmds.borrow()[idx as usize].uc_name.clone()));
    }

    idx -= b_len;
    with_ucmds(|u| {
        let u_len = u.len() as i32;
        if idx < u_len {
            let name = &u[idx as usize].uc_name;

            for cmd in &*buf.b_ucmds.borrow() {
                if name.as_bytes() == cmd.uc_name.as_bytes() {
                    return Some(c"");
                }
            }
            return Some(xp.set_item(name.clone()));
        }
        None
    })
}

/// Get the name of user command `idx` (`get_user_command_name`).
pub fn get_user_command_name(idx: i32, cmdidx: Cmdidx) -> Option<CString> {
    if cmdidx == Cmdidx::CmdUser && idx >= 0 {
        return with_ucmds(|u| u.get(idx as usize).map(|c| c.uc_name.clone()));
    }
    if cmdidx == Cmdidx::CmdUserBuf {
        let buf = prevwin_curwin().buf_mut();
        if idx >= 0 && (idx as usize) < buf.b_ucmds.borrow().len() {
            return Some(buf.b_ucmds.borrow()[idx as usize].uc_name.clone());
        }
    }
    None
}

/// Function given to `ExpandGeneric()` to obtain the list of user address type names (`get_user_cmd_addr_type`).
pub fn get_user_cmd_addr_type(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx < 0 || (idx as usize) >= ADDR_TYPE_COMPLETE.len() {
        return None;
    }
    ADDR_TYPE_COMPLETE[idx as usize].name
}

/// Function given to `ExpandGeneric()` to obtain the list of user command attributes (`get_user_cmd_flags`).
pub fn get_user_cmd_flags(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    const USER_CMD_FLAGS: [&CStr; 10] = [
        c"addr",
        c"bang",
        c"bar",
        c"buffer",
        c"complete",
        c"count",
        c"nargs",
        c"range",
        c"register",
        c"keepscript",
    ];
    if idx < 0 {
        return None;
    }
    USER_CMD_FLAGS.get(idx as usize).copied()
}

/// Function given to `ExpandGeneric()` to obtain the list of values for `-nargs` (`get_user_cmd_nargs`).
pub fn get_user_cmd_nargs(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    const USER_CMD_NARGS: [&CStr; 6] = [c"0", c"1", c"_", c"*", c"?", c"+"];
    if idx < 0 {
        return None;
    }
    USER_CMD_NARGS.get(idx as usize).copied()
}

fn get_command_complete(arg: i32) -> Option<&'static CStr> {
    if arg < 0 || (arg as usize) >= COMMAND_COMPLETE_LEN {
        return None;
    }
    command_complete_entry(arg)
}

/// Function given to `ExpandGeneric()` to obtain the list of values for `-complete` (`get_user_cmd_complete`).
pub fn get_user_cmd_complete(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    if idx < 0 || (idx as usize) >= COMMAND_COMPLETE_LEN {
        return None;
    }
    if idx == EXPAND_USER_LUA {
        return Some(c"");
    }
    Some(get_command_complete(idx).unwrap_or(c""))
}

/// Get the name of completion type `expand` as an allocated string (`cmdcomplete_type_to_str`).
pub fn cmdcomplete_type_to_str(expand: i32, compl_arg: Option<&[u8]>) -> Option<CString> {
    if expand == EXPAND_USER_LUA {
        return None;
    }
    let cmd_compl = get_command_complete(expand)?;

    if expand == EXPAND_USER_LIST || expand == EXPAND_USER_DEFINED {
        let compl_bytes = cmd_compl.to_bytes();
        let arg_bytes = compl_arg.unwrap_or(&[]);
        let mut bytes = Vec::with_capacity(compl_bytes.len() + arg_bytes.len() + 1);
        bytes.extend_from_slice(compl_bytes);
        bytes.push(b',');
        bytes.extend_from_slice(arg_bytes);
        return Some(CString::new(bytes).unwrap_or_default());
    }

    Some(cmd_compl.to_owned())
}

pub fn cmdcomplete_str_to_type(complete_bytes: &[u8]) -> i32 {
    if complete_bytes.starts_with(b"custom,") {
        return EXPAND_USER_DEFINED;
    }
    if complete_bytes.starts_with(b"customlist,") {
        return EXPAND_USER_LIST;
    }

    for i in 0..(COMMAND_COMPLETE_LEN as i32) {
        let Some(cmd_compl) = get_command_complete(i) else {
            continue;
        };
        if complete_bytes == cmd_compl.to_bytes() {
            return i;
        }
    }

    EXPAND_NOTHING
}

fn uc_list(name_prefix: &[u8]) {
    let name_len = name_prefix.len();
    with_ucmds(|global_ucmds| {
        let mut found = false;

        msg_ext_set_kind(c"list_cmd");
        let mut is_global = false;
        let mut gap: &[Ucmd] = &prevwin_curwin().buf_mut().b_ucmds.borrow();
        loop {
            let mut i: i32 = 0;
            while (i as usize) < gap.len() {
                let cmd = &gap[i as usize];
                let a = cmd.uc_argt;
                let cmd_b = cmd.uc_name.as_bytes();

                if name_prefix != &cmd_b[..name_len.min(cmd_b.len())]
                    || message_filtered(&cmd.uc_name)
                {
                    i += 1;
                    continue;
                }

                if !found {
                    msg_puts_title(gettext(
                        c"\n    Name              Args Address Complete    Definition",
                    ));
                }
                found = true;
                msg_putchar(b'\n' as i32);
                if g_st().got_int.get() {
                    break;
                }

                let mut len: usize = 4;
                if (a & EX_BANG) != 0 {
                    msg_putchar(b'!' as i32);
                    len -= 1;
                }
                if (a & EX_REGSTR) != 0 {
                    msg_putchar(b'"' as i32);
                    len -= 1;
                }
                if !is_global {
                    msg_putchar(b'b' as i32);
                    len -= 1;
                }
                if (a & EX_TRLBAR) != 0 {
                    msg_putchar(b'|' as i32);
                    len -= 1;
                }
                if len != 0 {
                    msg_puts(crate::memory::CStrSubslice::advance(c"    ", 4 - len));
                }

                msg_outtrans(&cmd.uc_name, HLF_D, false);
                len = cmd.uc_name.as_bytes().len() + 4;

                if len < 21 {
                    const SPACES: &CStr = c"                 ";
                    msg_puts(crate::memory::CStrSubslice::advance(SPACES, len - 4));
                    len = 21;
                }
                msg_putchar(b' ' as i32);
                len += 1;

                let over = (len as i64) - 22;
                len = 0;
                {
                    let iobuff = &mut *g_st().iobuff_bytes_mut();

                    match a & (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) {
                        0 => {
                            iobuff[len] = b'0';
                            len += 1;
                        }
                        EX_EXTRA => {
                            iobuff[len] = b'*';
                            len += 1;
                        }
                        x if x == (EX_EXTRA | EX_NOSPC) => {
                            iobuff[len] = b'?';
                            len += 1;
                        }
                        x if x == (EX_EXTRA | EX_NEEDARG) => {
                            iobuff[len] = b'+';
                            len += 1;
                        }
                        x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG) => {
                            iobuff[len] = b'1';
                            len += 1;
                        }
                        x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) => {
                            iobuff[len] = b'_';
                            len += 1;
                        }
                        _ => {}
                    }

                    loop {
                        iobuff[len] = b' ';
                        len += 1;
                        if (len as i64) >= 5 - over {
                            break;
                        }
                    }

                    if (a & (EX_RANGE | EX_COUNT)) != 0 {
                        if (a & EX_COUNT) != 0 {
                            let s = format!("{}c", cmd.uc_def);
                            let bytes = s.as_bytes();
                            let copy_len = bytes.len().min(IOSIZE - 1 - len);
                            iobuff[len..(len + copy_len)].copy_from_slice(&bytes[..copy_len]);
                            len += copy_len;
                        } else if (a & EX_DFLALL) != 0 {
                            iobuff[len] = b'%';
                            len += 1;
                        } else if cmd.uc_def >= 0 {
                            let s = format!("{}", cmd.uc_def);
                            let bytes = s.as_bytes();
                            let copy_len = bytes.len().min(IOSIZE - 1 - len);
                            iobuff[len..(len + copy_len)].copy_from_slice(&bytes[..copy_len]);
                            len += copy_len;
                        } else {
                            iobuff[len] = b'.';
                            len += 1;
                        }
                    }

                    loop {
                        iobuff[len] = b' ';
                        len += 1;
                        if (len as i64) >= 8 - over {
                            break;
                        }
                    }

                    for entry in &ADDR_TYPE_COMPLETE {
                        if entry.expand == ADDR_NONE {
                            break;
                        }
                        if entry.expand != ADDR_LINES && entry.expand == cmd.uc_addr_type {
                            if let Some(short) = entry.shortname {
                                let bytes = short.to_bytes();
                                let copy_len = bytes.len().min(IOSIZE - 1 - len);
                                iobuff[len..(len + copy_len)].copy_from_slice(&bytes[..copy_len]);
                                len += copy_len;
                            }
                            break;
                        }
                    }

                    loop {
                        iobuff[len] = b' ';
                        len += 1;
                        if (len as i64) >= 13 - over {
                            break;
                        }
                    }

                    if let Some(cmd_compl) = get_command_complete(cmd.uc_compl) {
                        let bytes = cmd_compl.to_bytes();
                        let copy_len = bytes.len().min(IOSIZE - 1 - len);
                        iobuff[len..(len + copy_len)].copy_from_slice(&bytes[..copy_len]);
                        len += copy_len;
                    }

                    loop {
                        iobuff[len] = b' ';
                        len += 1;
                        if (len as i64) >= 25 - over {
                            break;
                        }
                    }

                    iobuff[len] = 0;
                }
                msg_outtrans(g_st().iobuff_cstr(), 0, false);

                if cmd.uc_luaref != LUA_NOREF {
                    let fn_str = nlua_funcref_str(cmd.uc_luaref, true);
                    msg_puts_hl(&fn_str, HLF_8, false);
                    if !cmd.uc_rep.as_bytes().is_empty() {
                        msg_puts(c"\n                                               ");
                    }
                }

                msg_outtrans_special(
                    cmd.uc_rep.as_bytes(),
                    false,
                    if name_len == 0 {
                        g_st().columns_global.get() - 47
                    } else {
                        0
                    },
                );
                if opt_vars().p_verbose.get() > 0 {
                    last_set_msg(cmd.uc_script_ctx);
                }
                line_breakcheck();
                if g_st().got_int.get() {
                    break;
                }
                i += 1;
            }
            if is_global || (i as usize) < gap.len() {
                break;
            }
            is_global = true;
            gap = global_ucmds;
        }

        if !found {
            msg(gettext(c"No user-defined commands found"), 0);
        }
    });
}

/// Parse address type argument (`parse_addr_type_arg`).
pub fn parse_addr_type_arg(val_slice: &[u8], addr_type_arg: &mut CmdAddr) -> bool {
    let val_slice = crate::memory::bytes_until_nul(val_slice);
    for entry in &ADDR_TYPE_COMPLETE {
        if entry.expand == ADDR_NONE {
            break;
        }
        if let Some(name) = entry.name
            && name.to_bytes() == val_slice
        {
            *addr_type_arg = entry.expand;
            return true;
        }
    }

    let i = val_slice
        .iter()
        .take_while(|&&b| !ascii_iswhite(i32::from(b)))
        .count();
    let err_cstr = CString::new(&val_slice[..i]).unwrap_or_default();

    semsg!(gettext(c"E180: Invalid address type value: %s"), &err_cstr);
    false
}

/// Parse a completion argument `value[vallen]` (`parse_compl_arg`).
pub fn parse_compl_arg(
    val_slice: &[u8],
    complp: &mut i32,
    argt: &mut u32,
    compl_arg: &mut Option<CString>,
) -> bool {
    let val_slice = crate::memory::bytes_until_nul(val_slice);
    let (name_slice, arg_slice_opt) =
        if let Some(comma_pos) = val_slice.iter().position(|&b| b == b',') {
            (&val_slice[..comma_pos], Some(&val_slice[comma_pos + 1..]))
        } else {
            (val_slice, None)
        };

    let mut matched = false;
    for i in 0..(COMMAND_COMPLETE_LEN as i32) {
        let Some(cmd_compl) = get_command_complete(i) else {
            continue;
        };
        if cmd_compl.to_bytes() == name_slice {
            *complp = i;
            if i == EXPAND_BUFFERS {
                *argt |= EX_BUFNAME;
            } else if i == EXPAND_DIRECTORIES || i == EXPAND_FILES || i == EXPAND_SHELLCMDLINE {
                *argt |= EX_XFILE;
            }
            matched = true;
            break;
        }
    }

    if !matched {
        let err_cstr = CString::new(val_slice).unwrap_or_default();
        semsg!(gettext(c"E180: Invalid complete value: %s"), &err_cstr);
        return false;
    }

    if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST && arg_slice_opt.is_some() {
        emsg(gettext(
            c"E468: Completion argument only allowed for custom completion",
        ));
        return false;
    }

    if (*complp == EXPAND_USER_DEFINED || *complp == EXPAND_USER_LIST) && arg_slice_opt.is_none() {
        emsg(gettext(
            c"E467: Custom completion requires a function argument",
        ));
        return false;
    }

    if let Some(arg_s) = arg_slice_opt {
        *compl_arg = Some(CString::new(arg_s).unwrap_or_default());
    }
    true
}

fn uc_scan_attr(
    attr_full: &[u8],
    argt: &mut u32,
    def: &mut i32,
    flags: &mut i32,
    complp: &mut i32,
    compl_arg: &mut Option<CString>,
    addr_type_arg: &mut CmdAddr,
) -> bool {
    let len = attr_full.len();
    if len == 0 {
        emsg(gettext(c"E175: No attribute specified"));
        return false;
    }

    let prefix_ic = |s: &[u8], target: &[u8]| {
        s.len() <= target.len() && s.eq_ignore_ascii_case(&target[..s.len()])
    };

    if prefix_ic(attr_full, b"bang") {
        *argt |= EX_BANG;
    } else if prefix_ic(attr_full, b"buffer") {
        *flags |= UC_BUFFER;
    } else if prefix_ic(attr_full, b"register") {
        *argt |= EX_REGSTR;
    } else if prefix_ic(attr_full, b"keepscript") {
        *argt |= EX_KEEPSCRIPT;
    } else if prefix_ic(attr_full, b"bar") {
        *argt |= EX_TRLBAR;
    } else {
        let (attr_name, val_opt) = if let Some(eq_pos) = attr_full.iter().position(|&b| b == b'=') {
            (&attr_full[..eq_pos], Some(&attr_full[eq_pos + 1..]))
        } else {
            (attr_full, None)
        };
        let vallen = val_opt.map_or(0, <[u8]>::len);

        if !attr_name.is_empty() && prefix_ic(attr_name, b"nargs") {
            if let Some(&[val_b]) = val_opt {
                match val_b {
                    b'0' => {}
                    b'1' => *argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG,
                    b'*' => *argt |= EX_EXTRA,
                    b'?' => *argt |= EX_EXTRA | EX_NOSPC,
                    b'+' => *argt |= EX_EXTRA | EX_NEEDARG,
                    b'_' => *argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE,
                    _ => {
                        emsg(gettext(c"E176: Invalid number of arguments"));
                        return false;
                    }
                }
            } else {
                emsg(gettext(c"E176: Invalid number of arguments"));
                return false;
            }
        } else if !attr_name.is_empty() && prefix_ic(attr_name, b"range") {
            *argt |= EX_RANGE;
            if val_opt == Some(b"%") {
                *argt |= EX_DFLALL;
            } else if let Some(mut rem) = val_opt {
                if *def >= 0 {
                    emsg(gettext(c"E177: Count cannot be specified twice"));
                    return false;
                }

                *def = getdigits_int(&mut rem, true, 0);
                *argt |= EX_ZEROR;

                if !rem.is_empty() || vallen == 0 {
                    emsg(gettext(c"E178: Invalid default value for count"));
                    return false;
                }
            }
            if *addr_type_arg == ADDR_NONE {
                *addr_type_arg = ADDR_LINES;
            }
        } else if !attr_name.is_empty() && prefix_ic(attr_name, b"count") {
            *argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
            if *addr_type_arg == ADDR_NONE {
                *addr_type_arg = ADDR_OTHER;
            }

            if let Some(mut rem) = val_opt {
                if *def >= 0 {
                    emsg(gettext(c"E177: Count cannot be specified twice"));
                    return false;
                }

                *def = getdigits_int(&mut rem, true, 0);

                if !rem.is_empty() {
                    emsg(gettext(c"E178: Invalid default value for count"));
                    return false;
                }
            }

            if *def < 0 {
                *def = 0;
            }
        } else if !attr_name.is_empty() && prefix_ic(attr_name, b"complete") {
            let Some(val) = val_opt else {
                semsg!(gettext(E_ARGUMENT_REQUIRED_FOR_STR), c"-complete",);
                return false;
            };

            if !parse_compl_arg(val, complp, argt, compl_arg) {
                return false;
            }
        } else if !attr_name.is_empty() && prefix_ic(attr_name, b"addr") {
            *argt |= EX_RANGE;
            let Some(val) = val_opt else {
                semsg!(gettext(E_ARGUMENT_REQUIRED_FOR_STR), c"-addr",);
                return false;
            };
            if !parse_addr_type_arg(val, addr_type_arg) {
                return false;
            }
            if *addr_type_arg != ADDR_LINES {
                *argt |= EX_ZEROR;
            }
        } else {
            let attr_cstr =
                CString::new(crate::memory::bytes_until_nul(attr_full)).unwrap_or_default();
            semsg!(gettext(c"E181: Invalid attribute: %s"), &attr_cstr);

            return false;
        }
    }

    true
}

/// Check for a valid user command name (`uc_validate_name`).
pub fn uc_validate_name(bytes: &[u8]) -> Option<usize> {
    let bytes = crate::memory::bytes_until_nul(bytes);
    let mut idx = 0usize;
    if bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        idx = bytes
            .iter()
            .take_while(|&&b| b.is_ascii_alphanumeric())
            .count();
    }
    let next_b = bytes.get(idx).copied().unwrap_or(0);
    if ends_excmd(i32::from(next_b)) == 0 && !ascii_iswhite(i32::from(next_b)) {
        return None;
    }
    Some(idx)
}

/// Create a new user command `name`, if one doesn't already exist (`uc_add_command`).
pub fn uc_add_command(
    name_prefix: &[u8],
    rep: &[u8],
    argt: u32,
    def: i64,
    flags: i32,
    context: i32,
    compl_arg: Option<CString>,
    mut compl_luaref: LuaRef,
    mut preview_luaref: LuaRef,
    addr_type: CmdAddr,
    mut luaref: LuaRef,
    desc: Option<&CStr>,
    force: bool,
) -> bool {
    let name_prefix = crate::memory::bytes_until_nul(name_prefix);
    let name_len = name_prefix.len();
    let rep = crate::memory::bytes_until_nul(rep);
    let mut cmp: i32 = 1;
    let rep_cstr = replace_termcodes(rep, false, 0, 0, None, opt_vars().p_cpo.as_bytes())
        .unwrap_or_else(|| CString::new(rep).unwrap_or_default());

    let insert_into = |gap: &mut Vec<Ucmd>| -> bool {
        let mut i: usize = 0;
        while i < gap.len() {
            let cmd_b = gap[i].uc_name.as_bytes();
            let len = cmd_b.len();
            cmp = name_prefix.cmp(&cmd_b[..name_len.min(len)]) as i32;
            if cmp == 0 {
                if name_len < len {
                    cmp = -1;
                } else if name_len > len {
                    cmp = 1;
                }
            }

            if cmp == 0 {
                if !force
                    && (gap[i].uc_script_ctx.sc_sid != g_st().current_sctx.get().sc_sid
                        || gap[i].uc_script_ctx.sc_seq == g_st().current_sctx.get().sc_seq)
                {
                    let name_cstr = CString::new(name_prefix).unwrap_or_default();
                    semsg!(
                        gettext(c"E174: Command already exists: add ! to replace it: %s"),
                        &name_cstr,
                    );
                    nlua_clear_ref(&mut luaref);
                    nlua_clear_ref(&mut compl_luaref);
                    nlua_clear_ref(&mut preview_luaref);
                    return false;
                }

                let cmd = &mut gap[i];
                cmd.uc_rep = CString::default();
                cmd.uc_compl_arg = None;
                cmd.uc_desc = None;
                nlua_clear_ref(&mut cmd.uc_luaref);
                nlua_clear_ref(&mut cmd.uc_compl_luaref);
                nlua_clear_ref(&mut cmd.uc_preview_luaref);
                break;
            }

            if cmp < 0 {
                break;
            }
            i += 1;
        }

        if cmp != 0 {
            let mut empty_ucmd = Ucmd::new();
            empty_ucmd.uc_name = CString::new(name_prefix).unwrap();
            gap.insert(i, empty_ucmd);
        }
        let cmd = &mut gap[i];

        cmd.uc_rep = rep_cstr;
        cmd.uc_desc = desc.filter(|d| !d.is_empty()).map(CStr::to_owned);
        cmd.uc_argt = argt;
        cmd.uc_def = def;
        cmd.uc_compl = context;
        cmd.uc_script_ctx = g_st().current_sctx.get();
        cmd.uc_script_ctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&mut cmd.uc_script_ctx);
        cmd.uc_compl_arg = compl_arg;
        cmd.uc_compl_luaref = compl_luaref;
        cmd.uc_preview_luaref = preview_luaref;
        cmd.uc_addr_type = addr_type;
        cmd.uc_luaref = luaref;

        true
    };

    if (flags & UC_BUFFER) != 0 {
        insert_into(&mut curbuf_mut().b_ucmds.borrow_mut())
    } else {
        with_ucmds_mut(insert_into)
    }
}

/// `:command ...` (`ex_command`).
pub fn ex_command(eap: &mut Exarg) {
    let mut argt: u32 = 0;
    let mut def: i32 = -1;
    let mut flags: i32 = 0;
    let mut context: i32 = EXPAND_NOTHING;
    let mut compl_arg: Option<CString> = None;
    let mut addr_type_arg: CmdAddr = ADDR_NONE;
    let has_attr = eap.arg.first_byte() == b'-';

    let mut p = eap.arg.clone();

    while (p).first_byte() == b'-' {
        p = p.wrapping_add(1);
        let end = p.skiptowhite();
        let attr_len = ((end).offset_from(&p) as isize) as usize;
        if !uc_scan_attr(
            (p).slice(attr_len),
            &mut argt,
            &mut def,
            &mut flags,
            &mut context,
            &mut compl_arg,
            &mut addr_type_arg,
        ) {
            return;
        }
        p = end.skipwhite();
    }

    let name = p;
    let Some(name_len) = uc_validate_name((name).as_bytes()) else {
        emsg(gettext(c"E182: Invalid command name"));
        return;
    };
    let end = name.wrapping_add(name_len);
    let name_slice = (name).slice(name_len);

    p = end.skipwhite();
    if !has_attr && ends_excmd(i32::from((p).first_byte())) != 0 {
        uc_list(name_slice);
    } else if !ascii_isupper((name).first_byte()) {
        emsg(gettext(
            c"E183: User defined commands must start with an uppercase letter",
        ));
    } else if name_len <= 4 && name_slice == &b"Next"[..name_len] {
        emsg(gettext(
            c"E841: Reserved name, cannot be used for user defined command",
        ));
    } else if context > 0 && (argt & EX_EXTRA) == 0 {
        emsg(gettext(E_COMPLETE_USED_WITHOUT_ALLOWING_ARGUMENTS));
    } else {
        uc_add_command(
            name_slice,
            (p).as_bytes(),
            argt,
            i64::from(def),
            flags,
            context,
            compl_arg,
            LUA_NOREF,
            LUA_NOREF,
            addr_type_arg,
            LUA_NOREF,
            None,
            eap.forceit,
        );
    }
}

/// `:comclear` (`ex_comclear`).
pub fn ex_comclear(_eap: &mut Exarg) {
    with_ucmds_mut(Vec::clear);
    if !g_st().curbuf.get().is_null() {
        curbuf_mut().b_ucmds.borrow_mut().clear();
    }
}

pub fn ex_delcommand(eap: &mut Exarg) {
    let mut arg = eap.arg.as_cstr();
    let mut buffer_only = false;

    let arg_bytes = arg.to_bytes();
    if arg_bytes.starts_with(b"-buffer")
        && arg_bytes
            .get(7)
            .is_some_and(|&b| ascii_iswhite(i32::from(b)))
    {
        buffer_only = true;
        arg = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(arg, 7));
    }

    let find_and_remove = |gap: &mut Vec<Ucmd>| -> bool {
        if let Some(pos) = gap
            .iter()
            .position(|c| c.uc_name.as_bytes() == arg.to_bytes())
        {
            gap.remove(pos);
            true
        } else {
            false
        }
    };

    if find_and_remove(&mut curbuf_mut().b_ucmds.borrow_mut()) {
        return;
    }
    if !buffer_only && with_ucmds_mut(find_and_remove) {
        return;
    }

    semsg!(
        gettext(if buffer_only {
            E_NO_SUCH_USER_DEFINED_COMMAND_IN_CURRENT_BUFFER_STR
        } else {
            E_NO_SUCH_USER_DEFINED_COMMAND_STR
        }),
        arg,
    );
}

/// Split a string by unescaped whitespace (space & tab) (`uc_split_args_iter`).
pub fn uc_split_args_iter(
    arg_bytes: &[u8],
    end: &mut usize,
    buf: &mut [u8],
    len: &mut usize,
) -> bool {
    let arg_bytes = crate::memory::bytes_until_nul(arg_bytes);
    let arglen = arg_bytes.len();
    if arglen == 0 {
        return true;
    }

    let mut pos = *end;
    while pos < arglen && ascii_iswhite(i32::from(arg_bytes[pos])) {
        pos += 1;
    }

    let mut l: usize = 0;
    while pos < arglen - 1 {
        if arg_bytes[pos] == b'\\'
            && (arg_bytes[pos + 1] == b'\\' || ascii_iswhite(i32::from(arg_bytes[pos + 1])))
        {
            pos += 1;
        }

        buf[l] = arg_bytes[pos];

        l += 1;
        if arg_bytes
            .get(pos + 1)
            .is_some_and(|&b| ascii_iswhite(i32::from(b)))
        {
            *end = pos + 1;
            *len = l;
            return false;
        }
        pos += 1;
    }

    if pos < arglen && !ascii_iswhite(i32::from(arg_bytes[pos])) {
        buf[l] = arg_bytes[pos];

        l += 1;
    }

    *len = l;
    true
}

pub fn uc_nargs_upper_bound(arg_bytes: &[u8]) -> usize {
    let arg_bytes = crate::memory::bytes_until_nul(arg_bytes);
    let mut was_white = true;
    let mut nargs: usize = 0;
    for &b in arg_bytes {
        let is_white = ascii_iswhite(i32::from(b));
        if was_white && !is_white {
            nargs += 1;
        }
        was_white = is_white;
    }
    nargs
}

/// Split and quote args for `<f-args>` (`uc_split_args`).
fn uc_split_args(eap: &Exarg, lenp: &mut usize) -> CString {
    let mut buf_vec: Vec<u8> = Vec::new();
    buf_vec.push(b'"');

    if eap.args.is_empty() {
        let mut rem = eap.arg.as_bytes();
        while !rem.is_empty() {
            if rem.starts_with(b"\\\\") {
                buf_vec.extend_from_slice(b"\\\\");
                rem = &rem[2..];
            } else if rem[0] == b'\\' && rem.get(1).is_some_and(|&b| ascii_iswhite(i32::from(b))) {
                buf_vec.push(rem[1]);
                rem = &rem[2..];
            } else if matches!(rem[0], b'\\' | b'"') {
                buf_vec.push(b'\\');
                buf_vec.push(rem[0]);
                rem = &rem[1..];
            } else if ascii_iswhite(i32::from(rem[0])) {
                rem = crate::charset::skipwhite_slice(rem);
                if rem.is_empty() {
                    break;
                }
                buf_vec.extend_from_slice(b"\", \"");
            } else {
                let charlen = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
                buf_vec.extend_from_slice(&rem[..charlen]);
                rem = &rem[charlen..];
            }
        }
    } else {
        for (i, (arg_i, &arglen_i)) in eap.args.iter().zip(&eap.arglens).enumerate() {
            let mut rem = (arg_i).slice(arglen_i);
            while !rem.is_empty() {
                if matches!(rem[0], b'\\' | b'"') {
                    buf_vec.push(b'\\');
                    buf_vec.push(rem[0]);
                    rem = &rem[1..];
                } else {
                    let charlen = crate::mbyte::utfc_slice2len(rem).max(1).min(rem.len());
                    buf_vec.extend_from_slice(&rem[..charlen]);
                    rem = &rem[charlen..];
                }
            }
            if i != eap.args.len() - 1 {
                buf_vec.extend_from_slice(b"\", \"");
            }
        }
    }

    buf_vec.push(b'"');
    *lenp = buf_vec.len();
    buf_vec.push(0);
    crate::memory::cstring_until_nul(&buf_vec)
}

fn add_cmd_modifier(buf: &mut Vec<u8>, mod_str: &[u8], multi_mods: &mut bool) -> usize {
    let mut result = mod_str.len();
    if *multi_mods {
        result += 1;
        buf.push(b' ');
    }
    buf.extend_from_slice(mod_str);
    *multi_mods = true;
    result
}

/// Add modifiers from `cmod->cmod_split` to `buf` (`add_win_cmd_modifiers`).
pub fn add_win_cmd_modifiers(buf: &mut Vec<u8>, cmod: &Cmdmod, multi_mods: &mut bool) -> usize {
    let mut result: usize = 0;

    if (cmod.cmod_split & WSP_ABOVE) != 0 {
        result += add_cmd_modifier(buf, b"aboveleft", multi_mods);
    }
    if (cmod.cmod_split & WSP_BELOW) != 0 {
        result += add_cmd_modifier(buf, b"belowright", multi_mods);
    }
    if (cmod.cmod_split & WSP_BOT) != 0 {
        result += add_cmd_modifier(buf, b"botright", multi_mods);
    }

    if cmod.cmod_tab > 0 {
        let tabnr = cmod.cmod_tab - 1;
        if tabnr == tabpage_index(g_st().curtab.get().opt()) {
            result += add_cmd_modifier(buf, b"tab", multi_mods);
        } else {
            let s = format!("{tabnr}tab");
            result += add_cmd_modifier(buf, s.as_bytes(), multi_mods);
        }
    }

    if (cmod.cmod_split & WSP_TOP) != 0 {
        result += add_cmd_modifier(buf, b"topleft", multi_mods);
    }
    if (cmod.cmod_split & WSP_VERT) != 0 {
        result += add_cmd_modifier(buf, b"vertical", multi_mods);
    }
    if (cmod.cmod_split & WSP_HOR) != 0 {
        result += add_cmd_modifier(buf, b"horizontal", multi_mods);
    }
    result
}

/// Generate text for the `cmod` command modifiers (`uc_mods`).
pub fn uc_mods(buf: &mut Vec<u8>, cmod: &Cmdmod, quote: bool) -> usize {
    let mut multi_mods = false;

    const MOD_ENTRIES: [(i32, &[u8]); 12] = [
        (CMOD_BROWSE, b"browse"),
        (CMOD_CONFIRM, b"confirm"),
        (CMOD_HIDE, b"hide"),
        (CMOD_KEEPALT, b"keepalt"),
        (CMOD_KEEPJUMPS, b"keepjumps"),
        (CMOD_KEEPMARKS, b"keepmarks"),
        (CMOD_KEEPPATTERNS, b"keeppatterns"),
        (CMOD_LOCKMARKS, b"lockmarks"),
        (CMOD_NOSWAPFILE, b"noswapfile"),
        (CMOD_UNSILENT, b"unsilent"),
        (CMOD_NOAUTOCMD, b"noautocmd"),
        (CMOD_SANDBOX, b"sandbox"),
    ];

    let mut result: usize = if quote { 2 } else { 0 };
    if quote {
        buf.push(b'"');
    }

    for &(flag, name) in &MOD_ENTRIES {
        if (cmod.cmod_flags & flag) != 0 {
            result += add_cmd_modifier(buf, name, &mut multi_mods);
        }
    }

    if (cmod.cmod_flags & CMOD_SILENT) != 0 {
        let s: &[u8] = if (cmod.cmod_flags & CMOD_ERRSILENT) != 0 {
            b"silent!"
        } else {
            b"silent"
        };
        result += add_cmd_modifier(buf, s, &mut multi_mods);
    }

    if cmod.cmod_verbose > 0 {
        let verbose_value = cmod.cmod_verbose - 1;
        if verbose_value == 1 {
            result += add_cmd_modifier(buf, b"verbose", &mut multi_mods);
        } else {
            let s = format!("{verbose_value}verbose");
            result += add_cmd_modifier(buf, s.as_bytes(), &mut multi_mods);
        }
    }

    result += add_win_cmd_modifiers(buf, cmod, &mut multi_mods);

    if quote {
        buf.push(b'"');
    }
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CodeType {
    Args,
    Bang,
    Count,
    Line1,
    Line2,
    Range,
    Mods,
    Register,
    Lt,
    None,
}

/// Check for a `<>` code in a user command (`uc_check_code`).
fn uc_check_code(
    code: &[u8],
    out: &mut Vec<u8>,
    uc_def: i64,
    eap: &mut Exarg,
    split_buf: &mut Option<CString>,
    split_len: &mut usize,
) -> usize {
    let mut result: usize = 0;
    let mut s = &code[1..];
    let mut quote: i32 = 0;
    let mut type_ = CodeType::None;

    if s.len() >= 2 && matches!(s[0], b'q' | b'Q' | b'f' | b'F') && s[1] == b'-' {
        quote = if matches!(s[0], b'q' | b'Q') { 1 } else { 2 };
        s = &s[2..];
    }

    let prefix_ic =
        |a: &[u8], b: &[u8]| a.len() <= b.len() && a.eq_ignore_ascii_case(&b[..a.len()]);
    if s.len() <= 1 {
        // type_ = CodeType::None
    } else if prefix_ic(s, b"args>") {
        type_ = CodeType::Args;
    } else if prefix_ic(s, b"bang>") {
        type_ = CodeType::Bang;
    } else if prefix_ic(s, b"count>") {
        type_ = CodeType::Count;
    } else if prefix_ic(s, b"line1>") {
        type_ = CodeType::Line1;
    } else if prefix_ic(s, b"line2>") {
        type_ = CodeType::Line2;
    } else if prefix_ic(s, b"range>") {
        type_ = CodeType::Range;
    } else if prefix_ic(s, b"lt>") {
        type_ = CodeType::Lt;
    } else if prefix_ic(s, b"reg>") || prefix_ic(s, b"register>") {
        type_ = CodeType::Register;
    } else if prefix_ic(s, b"mods>") {
        type_ = CodeType::Mods;
    }

    match type_ {
        CodeType::Args => {
            if eap.arg.is_empty() {
                if quote == 1 {
                    result = 2;
                    out.extend_from_slice(b"''");
                } else {
                    result = 0;
                }
            } else {
                if (eap.argt & EX_NOSPC) != 0 && quote == 2 {
                    quote = 1;
                }

                match quote {
                    0 => {
                        let bytes = eap.arg.as_bytes();
                        result = bytes.len();
                        out.extend_from_slice(bytes);
                    }
                    1 => {
                        let bytes = eap.arg.as_bytes();
                        let start_len = out.len();
                        out.push(b'"');
                        for &scan_byte in bytes {
                            if scan_byte == b'\\' || scan_byte == b'"' {
                                out.push(b'\\');
                            }
                            out.push(scan_byte);
                        }
                        out.push(b'"');
                        result = out.len() - start_len;
                    }
                    2 => {
                        if split_buf.is_none() {
                            *split_buf = Some(uc_split_args(eap, split_len));
                        }

                        result = *split_len;
                        if result != 0 {
                            out.extend_from_slice(split_buf.as_ref().unwrap().as_bytes());
                        }
                    }
                    _ => {}
                }
            }
        }
        CodeType::Bang => {
            result = if eap.forceit { 1 } else { 0 };
            if quote != 0 {
                result += 2;
                out.push(b'"');
            }
            if eap.forceit {
                out.push(b'!');
            }
            if quote != 0 {
                out.push(b'"');
            }
        }
        CodeType::Line1 | CodeType::Line2 | CodeType::Range | CodeType::Count => {
            let num: i64 = match type_ {
                CodeType::Line1 => i64::from(eap.line1),
                CodeType::Line2 => i64::from(eap.line2),
                CodeType::Range => i64::from(eap.addr_count),
                _ => {
                    if eap.addr_count > 0 {
                        i64::from(eap.line2)
                    } else {
                        uc_def
                    }
                }
            };
            let num_str = format!("{num}");
            let num_bytes = num_str.as_bytes();
            result = num_bytes.len();
            if quote != 0 {
                result += 2;
                out.push(b'"');
            }
            out.extend_from_slice(num_bytes);
            if quote != 0 {
                out.push(b'"');
            }
        }
        CodeType::Mods => {
            result = uc_mods(out, &g_st().cmdmod.borrow(), quote != 0);
        }
        CodeType::Register => {
            result = if eap.regname != 0 { 1 } else { 0 };
            if quote != 0 {
                result += 2;
                out.push(b'\'');
            }
            if eap.regname != 0 {
                out.push(eap.regname as u8);
            }
            if quote != 0 {
                out.push(b'\'');
            }
        }
        CodeType::Lt => {
            result = 1;
            out.push(b'<');
        }
        CodeType::None => {
            result = usize::MAX;
            out.push(b'<');
        }
    }

    result
}

pub fn do_ucmd(eap: &mut Exarg, preview: bool) -> i32 {
    let mut split_len: usize = 0;
    let mut split_buf: Option<CString> = None;

    let (uc_name, uc_rep, uc_argt, uc_def, uc_script_ctx, uc_luaref, uc_preview_luaref) =
        if eap.cmdidx == Cmdidx::CmdUser {
            with_ucmds(|u| {
                let c = &u[eap.useridx as usize];
                (
                    c.uc_name.clone(),
                    c.uc_rep.as_bytes().to_vec(),
                    c.uc_argt,
                    c.uc_def,
                    c.uc_script_ctx,
                    c.uc_luaref,
                    c.uc_preview_luaref,
                )
            })
        } else {
            let c = &prevwin_curwin().buf_mut().b_ucmds.borrow()[eap.useridx as usize];
            (
                c.uc_name.clone(),
                c.uc_rep.as_bytes().to_vec(),
                c.uc_argt,
                c.uc_def,
                c.uc_script_ctx,
                c.uc_luaref,
                c.uc_preview_luaref,
            )
        };

    if preview {
        debug_assert!(uc_preview_luaref > 0);
        return nlua_do_ucmd(&uc_name, uc_preview_luaref, uc_argt, uc_def, eap, true);
    }

    if uc_luaref > 0 {
        nlua_do_ucmd(&uc_name, uc_luaref, uc_argt, uc_def, eap, false);
        return 0;
    }

    let rep_bytes = uc_rep;
    let mut buf_vec: Vec<u8> = Vec::with_capacity(rep_bytes.len() + 32);
    let mut rem: &[u8] = &rep_bytes;

    loop {
        let start_pos = rem.iter().position(|&b| b == b'<');
        let end_pos = start_pos.and_then(|sp| {
            rem[sp + 1..]
                .iter()
                .position(|&b| b == b'>')
                .map(|ep| sp + 1 + ep)
        });
        let ksp_pos = rem.iter().position(|&b| b == K_SPECIAL);
        if let Some(kp) = ksp_pos
            && (start_pos.is_none_or(|sp| kp < sp) || end_pos.is_none())
            && rem.get(kp + 1) == Some(&KS_SPECIAL)
            && rem.get(kp + 2) == Some(&KE_FILLER)
        {
            if kp > 0 {
                buf_vec.extend_from_slice(&rem[..kp]);
            }
            buf_vec.push(K_SPECIAL);
            rem = &rem[kp + 3..];
            continue;
        }

        let (Some(sp), Some(ep)) = (start_pos, end_pos) else {
            break;
        };

        if sp > 0 {
            buf_vec.extend_from_slice(&rem[..sp]);
        }

        let code_slice = &rem[sp..=ep];
        let code_len = uc_check_code(
            code_slice,
            &mut buf_vec,
            uc_def,
            eap,
            &mut split_buf,
            &mut split_len,
        );
        if code_len == usize::MAX {
            rem = &rem[sp + 1..];
        } else {
            rem = &rem[ep + 1..];
        }
    }
    buf_vec.extend_from_slice(rem);
    buf_vec.push(0);

    let mut save_current_sctx = g_st().current_sctx.get();
    let mut restore_current_sctx = false;
    if (uc_argt & EX_KEEPSCRIPT) == 0 {
        restore_current_sctx = true;
        save_current_sctx = g_st().current_sctx.get();
        {
            let mut v = g_st().current_sctx.get();
            v.sc_sid = uc_script_ctx.sc_sid;
            g_st().current_sctx.set(v);
        }
    }
    do_cmdline(
        Some(crate::memory::bytes_as_cstr(&buf_vec)),
        eap.ea_getline.clone(),
        DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_KEYTYPED,
    );

    if restore_current_sctx {
        g_st().current_sctx.set(save_current_sctx);
    }

    0
}

#[inline]
fn arena_str_obj(arena: &mut Arena, bytes: &[u8]) -> Object {
    string_obj(arena_bytes(arena, bytes))
}

/// Gets a map of maps describing user-commands defined for buffer `buf` or
/// defined globally if `buf` is NULL (`commands_array`).
pub fn commands_array(buf: Option<&FileBuf>, arena: &mut Arena) -> Dict {
    with_ucmds(|u| {
        let buf_ucmds = buf.map(|b| b.b_ucmds.borrow());
        let gap: &[Ucmd] = buf_ucmds.as_deref().map_or(u, Vec::as_slice);

        let mut rv = arena_dict(arena, gap.len());
        for cmd in gap {
            let mut arg_ch: u8 = 0;
            let mut d = arena_dict(arena, 16);

            put_c(
                &mut d,
                c"name",
                arena_str_obj(arena, cmd.uc_name.as_bytes()),
            );
            put_c(
                &mut d,
                c"definition",
                arena_str_obj(arena, cmd.uc_rep.as_bytes()),
            );
            put_c(
                &mut d,
                c"desc",
                cmd.uc_desc
                    .as_ref()
                    .map_or(string_obj(NvimString::default()), |desc| {
                        arena_str_obj(arena, desc.as_bytes())
                    }),
            );
            put_c(
                &mut d,
                c"script_id",
                integer_obj(i64::from(cmd.uc_script_ctx.sc_sid)),
            );
            put_c(&mut d, c"bang", boolean_obj((cmd.uc_argt & EX_BANG) != 0));
            put_c(&mut d, c"bar", boolean_obj((cmd.uc_argt & EX_TRLBAR) != 0));
            put_c(
                &mut d,
                c"register",
                boolean_obj((cmd.uc_argt & EX_REGSTR) != 0),
            );
            put_c(
                &mut d,
                c"keepscript",
                boolean_obj((cmd.uc_argt & EX_KEEPSCRIPT) != 0),
            );

            if cmd.uc_preview_luaref != LUA_NOREF {
                put_c(
                    &mut d,
                    c"preview",
                    luaref_obj(api_new_luaref(cmd.uc_preview_luaref)),
                );
            }

            if cmd.uc_luaref != LUA_NOREF {
                put_c(
                    &mut d,
                    c"callback",
                    luaref_obj(api_new_luaref(cmd.uc_luaref)),
                );
            }

            match cmd.uc_argt & (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) {
                0 => arg_ch = b'0',
                EX_EXTRA => arg_ch = b'*',
                x if x == (EX_EXTRA | EX_NOSPC) => arg_ch = b'?',
                x if x == (EX_EXTRA | EX_NEEDARG) => arg_ch = b'+',
                x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG) => arg_ch = b'1',
                x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) => arg_ch = b'_',
                _ => {}
            }
            let arg_slice: &[u8] = if arg_ch == 0 {
                &[]
            } else {
                std::slice::from_ref(&arg_ch)
            };
            put_c(&mut d, c"nargs", arena_str_obj(arena, arg_slice));

            if cmd.uc_compl_luaref != LUA_NOREF {
                put_c(
                    &mut d,
                    c"complete",
                    luaref_obj(api_new_luaref(cmd.uc_compl_luaref)),
                );
            } else {
                put_c(
                    &mut d,
                    c"complete",
                    match get_command_complete(cmd.uc_compl) {
                        Some(cmd_compl) => cstr_as_obj(Some(cmd_compl)),
                        None => nil_obj(),
                    },
                );
            }
            put_c(
                &mut d,
                c"complete_arg",
                match &cmd.uc_compl_arg {
                    Some(arg) => arena_str_obj(arena, arg.as_bytes()),
                    None => nil_obj(),
                },
            );

            let mut obj = nil_obj();
            if (cmd.uc_argt & EX_COUNT) != 0 {
                if cmd.uc_def >= 0 {
                    let s = format!("{}", cmd.uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(Some(c"0"));
                }
            }
            put_c(&mut d, c"count", obj);

            obj = nil_obj();
            if (cmd.uc_argt & EX_RANGE) != 0 {
                if (cmd.uc_argt & EX_DFLALL) != 0 {
                    obj = cstr_as_obj(Some(c"%"));
                } else if cmd.uc_def >= 0 {
                    let s = format!("{}", cmd.uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(Some(c"."));
                }
            }
            put_c(&mut d, c"range", obj);

            obj = nil_obj();
            for entry in &ADDR_TYPE_COMPLETE {
                if entry.expand == ADDR_NONE {
                    break;
                }
                if entry.expand != ADDR_LINES && entry.expand == cmd.uc_addr_type {
                    if let Some(name) = entry.name {
                        obj = cstr_as_obj(Some(name));
                    }
                    break;
                }
            }
            put_c(&mut d, c"addr", obj);

            let cmd_key = arena_bytes(arena, cmd.uc_name.as_bytes());
            put_c(&mut rv, cmd_key, dict_obj(d));
        }
        let _ = size_of::<KeyValuePair>();
        rv
    })
}
