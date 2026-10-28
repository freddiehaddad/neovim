//! User defined command support (`src/nvim/usercmd.c`).

use crate::api::private::helpers::{
    arena_dict, boolean_obj, cstr_as_obj, dict_obj, integer_obj, luaref_obj, nil_obj, put_c,
    string_obj,
};
use crate::api::private::validate::{Dict, KeyValuePair, NvimString, Object};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, HLF_8};
use crate::buffer::FileBuf;
use crate::charset::{
    NUL, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_isupper, ascii_iswhite, getdigits_int,
    skiptowhite, skipwhite,
};
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
    ADDR_QUICKFIX, ADDR_TABS, ADDR_WINDOWS, CMD_SIZE, CMD_USER, CMD_USER_BUF, CMOD_BROWSE,
    CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS,
    CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE, CMOD_SANDBOX, CMOD_SILENT,
    CMOD_UNSILENT, Cmdmod, EX_ARGSPACE, EX_BANG, EX_BUFNAME, EX_COUNT, EX_DFLALL, EX_EXTRA,
    EX_KEEPSCRIPT, EX_NEEDARG, EX_NOSPC, EX_RANGE, EX_REGSTR, EX_TRLBAR, EX_XFILE, EX_ZEROR, Exarg,
    KE_FILLER, do_cmdline, ends_excmd,
};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::keycodes::{K_SPECIAL, KS_SPECIAL, replace_termcodes};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_new_luaref, nlua_do_ucmd, nlua_funcref_str, nlua_set_sctx};
use crate::mapping::set_context_in_map_cmd;
use crate::mbyte::{mb_copy_char, utfc_ptr2len};
use crate::memory::{Arena, arena_memdupz};
use crate::menu::set_context_in_menu_cmd;
use crate::message::{
    emsg, message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_outtrans_special, msg_putchar,
    msg_puts, msg_puts_hl, msg_puts_title, semsg,
};
use crate::option_vars::*;
use crate::os::env::ExpandCtx;
use crate::os::input::line_breakcheck;
use crate::runtime::sourcing_lnum;
use crate::strings::{gettext, vim_strchr};
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT, prevwin_curwin, tabpage_index,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;

pub(crate) const UC_BUFFER: c_int = 1;

#[repr(C)]
pub struct Ucmd {
    pub uc_name: CString,
    pub uc_argt: u32,
    pub uc_rep: CString,
    pub uc_def: i64,
    pub uc_compl: c_int,
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
            uc_addr_type: 0,
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

static UCMDS_STATE: GlobalState<Vec<Ucmd>> = GlobalState::new(Vec::new());

#[inline]
pub(crate) fn ucmds_mut() -> &'static mut Vec<Ucmd> {
    unsafe { &mut *UCMDS_STATE.as_ptr() }
}

const COMMAND_COMPLETE_LEN: usize = 64;

fn command_complete_entry(idx: c_int) -> Option<&'static CStr> {
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
    p: *mut c_char,
    mut full: Option<&mut c_int>,
    mut xp: Option<&mut ExpandCtx>,
    mut complp: Option<&mut c_int>,
) -> *mut c_char {
    unsafe {
        let len = p.offset_from(eap.cmd) as c_int;
        let mut matchlen: c_int = 0;
        let mut found = false;
        let mut possible = false;
        let mut amb_local = false;

        let mut is_global = false;
        let mut gap: &[Ucmd] = &(*prevwin_curwin().w_buffer).b_ucmds;
        loop {
            let mut j: c_int = 0;
            while (j as usize) < gap.len() {
                let uc = &gap[j as usize];
                let mut cp = eap.cmd;
                let mut np = uc.uc_name.as_ptr();
                let mut k: c_int = 0;
                while k < len && !crate::memory::cstr_is_empty(np) && {
                    let c1 = *cp;
                    let c2 = *np;
                    cp = cp.add(1);
                    np = np.add(1);
                    c1 == c2
                } {
                    k += 1;
                }
                if k == len
                    || (crate::memory::cstr_is_empty(np)
                        && ascii_isdigit(crate::memory::cstr_char_at(eap.cmd, k as usize)))
                {
                    if k == len && found && !crate::memory::cstr_is_empty(np) {
                        if is_global {
                            return ptr::null_mut();
                        }
                        amb_local = true;
                    }

                    if !found || (k == len && crate::memory::cstr_is_empty(np)) {
                        if k == len {
                            found = true;
                        } else {
                            possible = true;
                        }

                        if is_global {
                            eap.cmdidx = CMD_USER;
                        } else {
                            eap.cmdidx = CMD_USER_BUF;
                        }
                        eap.argt = uc.uc_argt;
                        eap.useridx = j;
                        eap.addr_type = uc.uc_addr_type;

                        if let Some(ref mut p) = complp {
                            **p = uc.uc_compl;
                        }
                        if xp.is_some() {
                            xp.as_deref_mut().unwrap().xp_luaref = uc.uc_compl_luaref;
                            xp.as_deref_mut().unwrap().xp_arg = uc
                                .uc_compl_arg
                                .as_ref()
                                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut());
                            xp.as_deref_mut().unwrap().xp_script_ctx = uc.uc_script_ctx;
                            xp.as_deref_mut().unwrap().xp_script_ctx.sc_lnum += sourcing_lnum();
                        }
                        matchlen = k;
                        if k == len && crate::memory::cstr_is_empty(np) {
                            if let Some(ref mut p) = full {
                                **p = 1;
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
            let u = ucmds_mut();
            gap = u;
        }

        if amb_local {
            if let Some(xp) = xp {
                xp.xp_context = EXPAND_UNSUCCESSFUL;
            }
            return ptr::null_mut();
        }

        if found || possible {
            return p.offset((matchlen - len) as isize);
        }
        p
    }
}

/// Set completion context for `:command` (`set_context_in_user_cmd`).
pub fn set_context_in_user_cmd(
    xp: &mut ExpandCtx,
    arg_in: impl crate::strings::IntoCCharPtr,
) -> *const c_char {
    let arg_in = arg_in.into_c_char_ptr();
    unsafe {
        let mut arg = arg_in;
        let mut p: *const c_char;

        while crate::memory::cstr_first_byte(arg) == b'-' {
            arg = arg.add(1);
            p = skiptowhite(arg);
            if crate::memory::cstr_is_empty(p) {
                p = crate::memory::cstr_as_bytes(arg)
                    .iter()
                    .position(|&b| b == b'=')
                    .map_or(ptr::null(), |i| arg.add(i));
                if p.is_null() {
                    xp.xp_context = EXPAND_USER_CMD_FLAGS;
                    xp.xp_pattern = arg.cast_mut();
                    return ptr::null();
                }

                let attr_len = p.offset_from(arg) as usize;
                if crate::memory::cstr_prefix_bytes(arg, attr_len)
                    .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"complete", attr_len))
                {
                    xp.xp_context = EXPAND_USER_COMPLETE;
                    xp.xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                } else if crate::memory::cstr_prefix_bytes(arg, attr_len)
                    .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"nargs", attr_len))
                {
                    xp.xp_context = EXPAND_USER_NARGS;
                    xp.xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                } else if crate::memory::cstr_prefix_bytes(arg, attr_len)
                    .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"addr", attr_len))
                {
                    xp.xp_context = EXPAND_USER_ADDR_TYPE;
                    xp.xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                }
                return ptr::null();
            }
            arg = skipwhite(p);
        }

        p = skiptowhite(arg);
        if crate::memory::cstr_is_empty(p) {
            xp.xp_context = EXPAND_USER_COMMANDS;
            xp.xp_pattern = arg.cast_mut();
            return ptr::null();
        }

        skipwhite(p)
    }
}

/// Set the completion context for the argument of a user defined command (`set_context_in_user_cmdarg`).
pub fn set_context_in_user_cmdarg(
    cmd: impl crate::strings::IntoCCharPtr,
    arg: impl crate::strings::IntoCCharPtr,
    argt: u32,
    context: c_int,
    xp: &mut ExpandCtx,
    forceit: bool,
) -> *const c_char {
    let cmd = cmd.into_c_char_ptr();
    let mut arg = arg.into_c_char_ptr();
    unsafe {
        if context == EXPAND_NOTHING {
            return ptr::null();
        }

        if (argt & EX_XFILE) != 0 {
            return ptr::null();
        }

        if context == EXPAND_MENUS {
            return set_context_in_menu_cmd(xp, cmd, arg.cast_mut(), forceit);
        }
        if context == EXPAND_COMMANDS {
            return arg;
        }
        if context == EXPAND_MAPPINGS {
            return set_context_in_map_cmd(
                xp,
                c"map",
                arg.cast_mut(),
                forceit,
                false,
                false,
                crate::cmdexpand::Cmdidx::CmdMap,
            );
        }
        if (argt & EX_ARGSPACE) == 0 {
            let mut p = arg;
            while !crate::memory::cstr_is_empty(p) {
                if crate::memory::cstr_first_byte(p) == b' ' {
                    arg = p.add(1);
                } else if crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) != 0
                {
                    p = p.add(1);
                }
                p = p.add(utfc_ptr2len(p) as usize);
            }
        }
        xp.xp_pattern = arg.cast_mut();
        xp.xp_context = context;

        ptr::null()
    }
}

pub fn expand_user_command_name(idx: c_int) -> *mut c_char {
    get_user_commands(&mut crate::os::env::ExpandCtx::new(), idx - CMD_SIZE)
}

/// Function given to `ExpandGeneric()` to obtain the list of user command names (`get_user_commands`).
pub fn get_user_commands(_xp: &mut ExpandCtx, mut idx: c_int) -> *mut c_char {
    unsafe {
        let buf = prevwin_curwin().w_buffer;
        let b_len = (*buf).b_ucmds.len() as c_int;

        if idx < b_len {
            return (&(*buf).b_ucmds)[idx as usize].uc_name.as_ptr().cast_mut();
        }

        idx -= b_len;
        let u = ucmds_mut();
        let u_len = u.len() as c_int;
        if idx < u_len {
            let name = (u)[idx as usize].uc_name.as_ptr().cast_mut();

            for cmd in &(*buf).b_ucmds {
                if crate::memory::cstr_as_bytes(name)
                    == crate::memory::cstr_as_bytes(cmd.uc_name.as_ptr())
                {
                    return c"".as_ptr().cast_mut();
                }
            }
            return name;
        }
        ptr::null_mut()
    }
}

/// Get the name of user command `idx` (`get_user_command_name`).
pub fn get_user_command_name(idx: c_int, cmdidx: c_int) -> *mut c_char {
    unsafe {
        let u = ucmds_mut();
        let u_len = u.len() as c_int;
        if cmdidx == CMD_USER && idx < u_len {
            return (u)[idx as usize].uc_name.as_ptr().cast_mut();
        }
        if cmdidx == CMD_USER_BUF {
            let buf = prevwin_curwin().w_buffer;
            if (idx as usize) < (*buf).b_ucmds.len() {
                return (&(*buf).b_ucmds)[idx as usize].uc_name.as_ptr().cast_mut();
            }
        }
        ptr::null_mut()
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user address type names (`get_user_cmd_addr_type`).
pub fn get_user_cmd_addr_type(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= ADDR_TYPE_COMPLETE.len() {
        return ptr::null_mut();
    }
    match ADDR_TYPE_COMPLETE[idx as usize].name {
        Some(s) => s.as_ptr().cast_mut(),
        None => ptr::null_mut(),
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user command attributes (`get_user_cmd_flags`).
pub fn get_user_cmd_flags(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
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
    if idx < 0 || (idx as usize) >= USER_CMD_FLAGS.len() {
        return ptr::null_mut();
    }
    USER_CMD_FLAGS[idx as usize].as_ptr().cast_mut()
}

/// Function given to `ExpandGeneric()` to obtain the list of values for `-nargs` (`get_user_cmd_nargs`).
pub fn get_user_cmd_nargs(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
    const USER_CMD_NARGS: [&CStr; 6] = [c"0", c"1", c"_", c"*", c"?", c"+"];
    if idx < 0 || (idx as usize) >= USER_CMD_NARGS.len() {
        return ptr::null_mut();
    }
    USER_CMD_NARGS[idx as usize].as_ptr().cast_mut()
}

fn get_command_complete(arg: c_int) -> *mut c_char {
    if arg < 0 || (arg as usize) >= COMMAND_COMPLETE_LEN {
        return ptr::null_mut();
    }
    match command_complete_entry(arg) {
        Some(s) => s.as_ptr().cast_mut(),
        None => ptr::null_mut(),
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of values for `-complete` (`get_user_cmd_complete`).
pub fn get_user_cmd_complete(_xp: &mut ExpandCtx, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= COMMAND_COMPLETE_LEN {
        return ptr::null_mut();
    }
    let cmd_compl = get_command_complete(idx);
    if cmd_compl.is_null() || idx == EXPAND_USER_LUA {
        return c"".as_ptr().cast_mut();
    }
    cmd_compl
}

/// Get the name of completion type `expand` as an allocated string (`cmdcomplete_type_to_str`).
pub fn cmdcomplete_type_to_str(
    expand: c_int,
    compl_arg: impl crate::strings::IntoCCharPtr,
) -> Option<CString> {
    let compl_arg = compl_arg.into_c_char_ptr();
    {
        let cmd_compl = get_command_complete(expand);
        if cmd_compl.is_null() || expand == EXPAND_USER_LUA {
            return None;
        }

        if expand == EXPAND_USER_LIST || expand == EXPAND_USER_DEFINED {
            let compl_bytes = crate::memory::cstr_as_bytes(cmd_compl);
            let arg_bytes = crate::memory::cstr_as_bytes(compl_arg);
            let mut bytes = Vec::with_capacity(compl_bytes.len() + arg_bytes.len() + 1);
            bytes.extend_from_slice(compl_bytes);
            bytes.push(b',');
            bytes.extend_from_slice(arg_bytes);
            return Some(CString::new(bytes).unwrap_or_default());
        }

        Some(crate::memory::cstr_to_cstring(cmd_compl))
    }
}

pub fn cmdcomplete_str_to_type(complete_str: impl crate::strings::IntoCCharPtr) -> c_int {
    let complete_str = complete_str.into_c_char_ptr();
    {
        if crate::memory::cstr_prefix_bytes(complete_str, 7) == &b"custom,"[..7] {
            return EXPAND_USER_DEFINED;
        }
        if crate::memory::cstr_prefix_bytes(complete_str, 11) == &b"customlist,"[..11] {
            return EXPAND_USER_LIST;
        }

        for i in 0..(COMMAND_COMPLETE_LEN as c_int) {
            let cmd_compl = get_command_complete(i);
            if cmd_compl.is_null() {
                continue;
            }
            if crate::memory::cstr_as_bytes(complete_str) == crate::memory::cstr_as_bytes(cmd_compl)
            {
                return i;
            }
        }

        EXPAND_NOTHING
    }
}

fn uc_list(name: impl crate::strings::IntoCCharPtr, name_len: usize) {
    let name = name.into_c_char_ptr();
    unsafe {
        let mut found = false;

        msg_ext_set_kind(c"list_cmd");
        let mut is_global = false;
        let mut gap: &[Ucmd] = &(*prevwin_curwin().w_buffer).b_ucmds;
        loop {
            let mut i: c_int = 0;
            while (i as usize) < gap.len() {
                let cmd = &gap[i as usize];
                let a = cmd.uc_argt;

                if (crate::memory::cstr_prefix_bytes(name, name_len)
                    != crate::memory::cstr_prefix_bytes(cmd.uc_name.as_ptr(), name_len))
                    || message_filtered(cmd.uc_name.as_ptr())
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
                msg_putchar(b'\n' as c_int);
                if g_st().got_int {
                    break;
                }

                let mut len: usize = 4;
                if (a & EX_BANG) != 0 {
                    msg_putchar(b'!' as c_int);
                    len -= 1;
                }
                if (a & EX_REGSTR) != 0 {
                    msg_putchar(b'"' as c_int);
                    len -= 1;
                }
                if !is_global {
                    msg_putchar(b'b' as c_int);
                    len -= 1;
                }
                if (a & EX_TRLBAR) != 0 {
                    msg_putchar(b'|' as c_int);
                    len -= 1;
                }
                if len != 0 {
                    msg_puts(c"    ".as_ptr().add(4 - len));
                }

                msg_outtrans(cmd.uc_name.as_ptr(), HLF_D, false);
                len = cmd.uc_name.as_bytes().len() + 4;

                if len < 21 {
                    const SPACES: &CStr = c"                 ";
                    msg_puts(SPACES.as_ptr().add(len - 4));
                    len = 21;
                }
                msg_putchar(b' ' as c_int);
                len += 1;

                let over = (len as i64) - 22;
                len = 0;

                match a & (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) {
                    0 => {
                        g_st().iobuff[len] = b'0' as c_char;
                        len += 1;
                    }
                    EX_EXTRA => {
                        g_st().iobuff[len] = b'*' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC) => {
                        g_st().iobuff[len] = b'?' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NEEDARG) => {
                        g_st().iobuff[len] = b'+' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG) => {
                        g_st().iobuff[len] = b'1' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) => {
                        g_st().iobuff[len] = b'_' as c_char;
                        len += 1;
                    }
                    _ => {}
                }

                loop {
                    g_st().iobuff[len] = b' ' as c_char;
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
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            (g_st().iobuff.as_mut_ptr()).add(len),
                            copy_len,
                        );
                        len += copy_len;
                    } else if (a & EX_DFLALL) != 0 {
                        g_st().iobuff[len] = b'%' as c_char;
                        len += 1;
                    } else if cmd.uc_def >= 0 {
                        let s = format!("{}", cmd.uc_def);
                        let bytes = s.as_bytes();
                        let copy_len = bytes.len().min(IOSIZE - 1 - len);
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            (g_st().iobuff.as_mut_ptr()).add(len),
                            copy_len,
                        );
                        len += copy_len;
                    } else {
                        g_st().iobuff[len] = b'.' as c_char;
                        len += 1;
                    }
                }

                loop {
                    g_st().iobuff[len] = b' ' as c_char;
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
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr().cast::<c_char>(),
                                (g_st().iobuff.as_mut_ptr()).add(len),
                                copy_len,
                            );
                            len += copy_len;
                        }
                        break;
                    }
                }

                loop {
                    g_st().iobuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 13 - over {
                        break;
                    }
                }

                let cmd_compl = get_command_complete(cmd.uc_compl);
                if !cmd_compl.is_null() {
                    let bytes = crate::memory::cstr_as_bytes(cmd_compl);
                    let copy_len = bytes.len().min(IOSIZE - 1 - len);
                    ptr::copy_nonoverlapping(
                        bytes.as_ptr().cast::<c_char>(),
                        (g_st().iobuff.as_mut_ptr()).add(len),
                        copy_len,
                    );
                    len += copy_len;
                }

                loop {
                    g_st().iobuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 25 - over {
                        break;
                    }
                }

                g_st().iobuff[len] = NUL;
                msg_outtrans(g_st().iobuff.as_ptr(), 0, false);

                if cmd.uc_luaref != LUA_NOREF {
                    let fn_str = nlua_funcref_str(cmd.uc_luaref, true);
                    msg_puts_hl(fn_str.as_ptr(), HLF_8, false);
                    if !cmd.uc_rep.as_bytes().is_empty() {
                        msg_puts(c"\n                                               ");
                    }
                }

                msg_outtrans_special(
                    cmd.uc_rep.as_ptr(),
                    false,
                    if name_len == 0 {
                        g_st().columns_global - 47
                    } else {
                        0
                    },
                );
                if opt_vars().p_verbose > 0 {
                    last_set_msg(cmd.uc_script_ctx);
                }
                line_breakcheck();
                if g_st().got_int {
                    break;
                }
                i += 1;
            }
            if is_global || (i as usize) < gap.len() {
                break;
            }
            is_global = true;
            let u = ucmds_mut();
            gap = u;
        }

        if !found {
            msg(gettext(c"No user-defined commands found"), 0);
        }
    }
}

/// Parse address type argument (`parse_addr_type_arg`).
pub fn parse_addr_type_arg(value: *mut c_char, vallen: c_int, addr_type_arg: &mut CmdAddr) -> bool {
    unsafe {
        for entry in &ADDR_TYPE_COMPLETE {
            if entry.expand == ADDR_NONE {
                break;
            }
            if let Some(name) = entry.name {
                let a = name.to_bytes().len() as c_int == vallen;
                let b = crate::memory::cstr_prefix_bytes(value, vallen as usize)
                    == crate::memory::cstr_prefix_bytes(name.as_ptr(), vallen as usize);
                if a && b {
                    *addr_type_arg = entry.expand;
                    return true;
                }
            }
        }

        let err = value;
        let mut i = 0usize;
        while crate::memory::cstr_byte_at(err, i) != 0
            && !ascii_iswhite(crate::memory::cstr_char_at(err, i))
        {
            i += 1;
        }
        *err.add(i) = NUL;
        semsg!(gettext(c"E180: Invalid address type value: %s"), err,);
        false
    }
}

/// Parse a completion argument `value[vallen]` (`parse_compl_arg`).
pub fn parse_compl_arg(
    value: impl crate::strings::IntoCCharPtr,
    vallen: c_int,
    complp: &mut c_int,
    argt: &mut u32,
    compl_arg: &mut Option<CString>,
) -> bool {
    let value = value.into_c_char_ptr();
    unsafe {
        let mut arg: *const c_char = ptr::null();
        let mut arglen: usize = 0;
        let mut valend: c_int = vallen;

        for i in 0..vallen {
            if crate::memory::cstr_byte_at(value, i as usize) == b',' {
                arg = value.add((i + 1) as usize);
                arglen = (vallen - i - 1) as usize;
                valend = i;
                break;
            }
        }

        let mut matched = false;
        for i in 0..(COMMAND_COMPLETE_LEN as c_int) {
            let cmd_compl = get_command_complete(i);
            if cmd_compl.is_null() {
                continue;
            }
            if crate::memory::cstr_len(cmd_compl) as c_int == valend
                && (crate::memory::cstr_prefix_bytes(value, valend as usize)
                    == crate::memory::cstr_prefix_bytes(cmd_compl, valend as usize))
            {
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
            semsg!(gettext(c"E180: Invalid complete value: %s"), value);
            return false;
        }

        if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST && !arg.is_null() {
            emsg(gettext(
                c"E468: Completion argument only allowed for custom completion",
            ));
            return false;
        }

        if (*complp == EXPAND_USER_DEFINED || *complp == EXPAND_USER_LIST) && arg.is_null() {
            emsg(gettext(
                c"E467: Custom completion requires a function argument",
            ));
            return false;
        }

        if !arg.is_null() {
            *compl_arg =
                Some(CString::new(crate::memory::cstr_slice(arg, arglen)).unwrap_or_default());
        }
        true
    }
}

fn uc_scan_attr(
    attr: *mut c_char,
    len: usize,
    argt: &mut u32,
    def: &mut c_int,
    flags: &mut c_int,
    complp: &mut c_int,
    compl_arg: &mut Option<CString>,
    addr_type_arg: &mut CmdAddr,
) -> bool {
    unsafe {
        if len == 0 {
            emsg(gettext(c"E175: No attribute specified"));
            return false;
        }

        if crate::memory::cstr_prefix_bytes(attr, len)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"bang", len))
        {
            *argt |= EX_BANG;
        } else if crate::memory::cstr_prefix_bytes(attr, len)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"buffer", len))
        {
            *flags |= UC_BUFFER;
        } else if crate::memory::cstr_prefix_bytes(attr, len)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"register", len))
        {
            *argt |= EX_REGSTR;
        } else if crate::memory::cstr_prefix_bytes(attr, len)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"keepscript", len))
        {
            *argt |= EX_KEEPSCRIPT;
        } else if crate::memory::cstr_prefix_bytes(attr, len)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"bar", len))
        {
            *argt |= EX_TRLBAR;
        } else {
            let mut val: *mut c_char = ptr::null_mut();
            let mut vallen: usize = 0;
            let mut attrlen: usize = len;

            for i in 0..len {
                if crate::memory::cstr_byte_at(attr, i) == b'=' {
                    val = attr.add(i + 1);
                    vallen = len - i - 1;
                    attrlen = i;
                    break;
                }
            }

            if crate::memory::cstr_prefix_bytes(attr, attrlen)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"nargs", attrlen))
            {
                if vallen == 1 {
                    match crate::memory::cstr_first_byte(val) {
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
            } else if crate::memory::cstr_prefix_bytes(attr, attrlen)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"range", attrlen))
            {
                *argt |= EX_RANGE;
                if vallen == 1 && crate::memory::cstr_first_byte(val) == b'%' {
                    *argt |= EX_DFLALL;
                } else if !val.is_null() {
                    let mut p = val;
                    if *def >= 0 {
                        emsg(gettext(c"E177: Count cannot be specified twice"));
                        return false;
                    }

                    *def = getdigits_int(&raw mut p, true, 0);
                    *argt |= EX_ZEROR;

                    if p != val.add(vallen) || vallen == 0 {
                        emsg(gettext(c"E178: Invalid default value for count"));
                        return false;
                    }
                }
                if *addr_type_arg == ADDR_NONE {
                    *addr_type_arg = ADDR_LINES;
                }
            } else if crate::memory::cstr_prefix_bytes(attr, attrlen)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"count", attrlen))
            {
                *argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
                if *addr_type_arg == ADDR_NONE {
                    *addr_type_arg = ADDR_OTHER;
                }

                if !val.is_null() {
                    let mut p = val;
                    if *def >= 0 {
                        emsg(gettext(c"E177: Count cannot be specified twice"));
                        return false;
                    }

                    *def = getdigits_int(&raw mut p, true, 0);

                    if p != val.add(vallen) {
                        emsg(gettext(c"E178: Invalid default value for count"));
                        return false;
                    }
                }

                if *def < 0 {
                    *def = 0;
                }
            } else if crate::memory::cstr_prefix_bytes(attr, attrlen)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"complete", attrlen))
            {
                if val.is_null() {
                    semsg!(gettext(E_ARGUMENT_REQUIRED_FOR_STR), c"-complete",);
                    return false;
                }

                if !parse_compl_arg(val, vallen as c_int, complp, argt, compl_arg) {
                    return false;
                }
            } else if crate::memory::cstr_prefix_bytes(attr, attrlen)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"addr", attrlen))
            {
                *argt |= EX_RANGE;
                if val.is_null() {
                    semsg!(gettext(E_ARGUMENT_REQUIRED_FOR_STR), c"-addr",);
                    return false;
                }
                if !parse_addr_type_arg(val, vallen as c_int, addr_type_arg) {
                    return false;
                }
                if *addr_type_arg != ADDR_LINES {
                    *argt |= EX_ZEROR;
                }
            } else {
                let ch = *attr.add(len);
                *attr.add(len) = NUL;
                semsg!(gettext(c"E181: Invalid attribute: %s"), attr);
                *attr.add(len) = ch;
                return false;
            }
        }

        true
    }
}

/// Check for a valid user command name (`uc_validate_name`).
pub fn uc_validate_name(mut name: *mut c_char) -> *mut c_char {
    unsafe {
        if ascii_isalpha(crate::memory::cstr_first_char(name)) {
            while ascii_isalnum(crate::memory::cstr_first_char(name)) {
                name = name.add(1);
            }
        }
        if ends_excmd(c_int::from(crate::memory::cstr_first_byte(name))) == 0
            && !ascii_iswhite(crate::memory::cstr_first_char(name))
        {
            return ptr::null_mut();
        }
        name
    }
}

/// Create a new user command `name`, if one doesn't already exist (`uc_add_command`).
pub fn uc_add_command(
    name: impl crate::strings::IntoCCharPtr,
    name_len: usize,
    rep: impl crate::strings::IntoCCharPtr,
    argt: u32,
    def: i64,
    flags: c_int,
    context: c_int,
    compl_arg: Option<CString>,
    mut compl_luaref: LuaRef,
    mut preview_luaref: LuaRef,
    addr_type: CmdAddr,
    mut luaref: LuaRef,
    desc: impl crate::strings::IntoCCharPtr,
    force: bool,
) -> bool {
    let name = name.into_c_char_ptr();
    let rep = rep.into_c_char_ptr();
    let desc = desc.into_c_char_ptr();
    unsafe {
        let mut cmd: *mut Ucmd = ptr::null_mut();
        let mut cmp: c_int = 1;
        let rep_cstr = replace_termcodes(
            rep,
            crate::memory::cstr_len(rep),
            false,
            0,
            0,
            None,
            opt_vars().p_cpo,
        )
        .unwrap_or_else(|| crate::memory::cstr_to_cstring(rep));

        let u = ucmds_mut();
        let gap: &mut Vec<Ucmd> = if (flags & UC_BUFFER) != 0 {
            &mut curbuf_mut().b_ucmds
        } else {
            u
        };

        let mut i: c_int = 0;
        while (i as usize) < gap.len() {
            cmd = &raw mut gap[i as usize];
            let len = (*cmd).uc_name.as_bytes().len();
            cmp = crate::memory::cstr_prefix_bytes(name, name_len).cmp(
                crate::memory::cstr_prefix_bytes((*cmd).uc_name.as_ptr(), name_len),
            ) as c_int;
            if cmp == 0 {
                if name_len < len {
                    cmp = -1;
                } else if name_len > len {
                    cmp = 1;
                }
            }

            if cmp == 0 {
                if !force
                    && ((*cmd).uc_script_ctx.sc_sid != g_st().current_sctx.sc_sid
                        || (*cmd).uc_script_ctx.sc_seq == g_st().current_sctx.sc_seq)
                {
                    semsg!(
                        gettext(c"E174: Command already exists: add ! to replace it: %s"),
                        name,
                    );
                    nlua_clear_ref(&mut luaref);
                    nlua_clear_ref(&mut compl_luaref);
                    nlua_clear_ref(&mut preview_luaref);
                    return false;
                }

                (*cmd).uc_rep = CString::default();
                (*cmd).uc_compl_arg = None;
                (*cmd).uc_desc = None;
                nlua_clear_ref(&mut (*cmd).uc_luaref);
                nlua_clear_ref(&mut (*cmd).uc_compl_luaref);
                nlua_clear_ref(&mut (*cmd).uc_preview_luaref);
                break;
            }

            if cmp < 0 {
                break;
            }
            i += 1;
        }

        if cmp != 0 {
            let name_slice = crate::memory::cstr_slice(name, name_len);
            let mut empty_ucmd = Ucmd::new();
            empty_ucmd.uc_name = CString::new(name_slice).unwrap();
            gap.insert(i as usize, empty_ucmd);
            cmd = &raw mut gap[i as usize];
        }

        (*cmd).uc_rep = rep_cstr;
        (*cmd).uc_desc = if !crate::memory::cstr_is_empty(desc) {
            Some(crate::memory::cstr_to_cstring(desc))
        } else {
            None
        };
        (*cmd).uc_argt = argt;
        (*cmd).uc_def = def;
        (*cmd).uc_compl = context;
        (*cmd).uc_script_ctx = g_st().current_sctx;
        (*cmd).uc_script_ctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&mut (*cmd).uc_script_ctx);
        (*cmd).uc_compl_arg = compl_arg;
        (*cmd).uc_compl_luaref = compl_luaref;
        (*cmd).uc_preview_luaref = preview_luaref;
        (*cmd).uc_addr_type = addr_type;
        (*cmd).uc_luaref = luaref;

        true
    }
}

/// `:command ...` (`ex_command`).
pub fn ex_command(eap: &mut Exarg) {
    unsafe {
        let mut argt: u32 = 0;
        let mut def: c_int = -1;
        let mut flags: c_int = 0;
        let mut context: c_int = EXPAND_NOTHING;
        let mut compl_arg: Option<CString> = None;
        let mut addr_type_arg: CmdAddr = ADDR_NONE;
        let has_attr = crate::memory::cstr_first_byte(eap.arg) == b'-';

        let mut p = eap.arg;

        while crate::memory::cstr_first_byte(p) == b'-' {
            p = p.add(1);
            let end = skiptowhite(p);
            if !uc_scan_attr(
                p,
                end.offset_from(p) as usize,
                &mut argt,
                &mut def,
                &mut flags,
                &mut context,
                &mut compl_arg,
                &mut addr_type_arg,
            ) {
                return;
            }
            p = skipwhite(end);
        }

        let name = p;
        let end = uc_validate_name(name);
        if end.is_null() {
            emsg(gettext(c"E182: Invalid command name"));
            return;
        }
        let name_len = end.offset_from(name) as usize;

        p = skipwhite(end);
        if !has_attr && ends_excmd(c_int::from(crate::memory::cstr_first_byte(p))) != 0 {
            uc_list(name, name_len);
        } else if !ascii_isupper(crate::memory::cstr_first_char(name)) {
            emsg(gettext(
                c"E183: User defined commands must start with an uppercase letter",
            ));
        } else if name_len <= 4
            && (crate::memory::cstr_prefix_bytes(name, name_len)
                == crate::memory::cstr_prefix_bytes(c"Next", name_len))
        {
            emsg(gettext(
                c"E841: Reserved name, cannot be used for user defined command",
            ));
        } else if context > 0 && (argt & EX_EXTRA) == 0 {
            emsg(gettext(E_COMPLETE_USED_WITHOUT_ALLOWING_ARGUMENTS));
        } else {
            uc_add_command(
                name,
                name_len,
                p,
                argt,
                i64::from(def),
                flags,
                context,
                compl_arg,
                LUA_NOREF,
                LUA_NOREF,
                addr_type_arg,
                LUA_NOREF,
                ptr::null(),
                eap.forceit != 0,
            );
        }
    }
}

/// `:comclear` (`ex_comclear`).
pub fn ex_comclear(_eap: &mut Exarg) {
    {
        let u = ucmds_mut();
        u.clear();
        if !g_st().curbuf.is_null() {
            curbuf_mut().b_ucmds.clear();
        }
    }
}

pub fn ex_delcommand(eap: &mut Exarg) {
    unsafe {
        let mut i: c_int;
        let mut res: c_int = -1;
        let mut arg: *const c_char = eap.arg;
        let mut buffer_only = false;

        if (crate::memory::cstr_prefix_bytes(arg, 7) == &b"-buffer"[..7])
            && ascii_iswhite(crate::memory::cstr_char_at(arg, 7))
        {
            buffer_only = true;
            arg = skipwhite(arg.add(7));
        }

        let mut is_global = false;
        let mut gap: &mut Vec<Ucmd> = &mut curbuf_mut().b_ucmds;
        loop {
            i = 0;
            while (i as usize) < gap.len() {
                res = crate::memory::cstr_as_bytes(arg).cmp(crate::memory::cstr_as_bytes(
                    gap[i as usize].uc_name.as_ptr(),
                )) as c_int;
                if res <= 0 {
                    break;
                }
                i += 1;
            }
            if is_global || res == 0 || buffer_only {
                break;
            }
            is_global = true;
            let u = ucmds_mut();
            gap = u;
        }

        if res != 0 {
            let _ = &mut i;
            semsg!(
                gettext(if buffer_only {
                    E_NO_SUCH_USER_DEFINED_COMMAND_IN_CURRENT_BUFFER_STR
                } else {
                    E_NO_SUCH_USER_DEFINED_COMMAND_STR
                }),
                arg,
            );
            return;
        }

        let _ = gap.remove(i as usize);
    }
}

/// Split a string by unescaped whitespace (space & tab) (`uc_split_args_iter`).
pub fn uc_split_args_iter(
    arg: impl crate::strings::IntoCCharPtr,
    arglen: usize,
    end: &mut usize,
    buf: *mut c_char,
    len: &mut usize,
) -> bool {
    let arg = arg.into_c_char_ptr();
    unsafe {
        if arglen == 0 {
            return true;
        }

        let mut pos = *end;
        while pos < arglen && ascii_iswhite(crate::memory::cstr_char_at(arg, pos)) {
            pos += 1;
        }

        let mut l: usize = 0;
        while pos < arglen - 1 {
            if crate::memory::cstr_byte_at(arg, pos) == b'\\'
                && (crate::memory::cstr_byte_at(arg, pos + 1) == b'\\'
                    || ascii_iswhite(crate::memory::cstr_char_at(arg, pos + 1)))
            {
                pos += 1;
                *buf.add(l) = *arg.add(pos);
                l += 1;
            } else {
                *buf.add(l) = *arg.add(pos);
                l += 1;
            }
            if ascii_iswhite(crate::memory::cstr_char_at(arg, pos + 1)) {
                *end = pos + 1;
                *len = l;
                return false;
            }
            pos += 1;
        }

        if pos < arglen && !ascii_iswhite(crate::memory::cstr_char_at(arg, pos)) {
            *buf.add(l) = *arg.add(pos);
            l += 1;
        }

        *len = l;
        true
    }
}

pub fn uc_nargs_upper_bound(arg: impl crate::strings::IntoCCharPtr, arglen: usize) -> usize {
    let arg = arg.into_c_char_ptr();
    {
        let mut was_white = true;
        let mut nargs: usize = 0;
        for i in 0..arglen {
            let is_white = ascii_iswhite(crate::memory::cstr_char_at(arg, i));
            if was_white && !is_white {
                nargs += 1;
            }
            was_white = is_white;
        }
        nargs
    }
}

/// Split and quote args for `<f-args>` (`uc_split_args`).
fn uc_split_args(
    arg: impl crate::strings::IntoCCharPtr,
    args: &[*mut c_char],
    arglens: &[usize],
    lenp: &mut usize,
) -> CString {
    let arg = arg.into_c_char_ptr();
    unsafe {
        let mut len: c_int = 2;
        if args.is_empty() {
            let mut p = arg;
            while !crate::memory::cstr_is_empty(p) {
                if crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) == b'\\'
                {
                    len += 2;
                    p = p.add(2);
                } else if crate::memory::cstr_first_byte(p) == b'\\'
                    && ascii_iswhite(crate::memory::cstr_char_at(p, 1))
                {
                    len += 1;
                    p = p.add(2);
                } else if crate::memory::cstr_first_byte(p) == b'\\'
                    || crate::memory::cstr_first_byte(p) == b'"'
                {
                    len += 2;
                    p = p.add(1);
                } else if ascii_iswhite(crate::memory::cstr_first_char(p)) {
                    p = skipwhite(p);
                    if crate::memory::cstr_is_empty(p) {
                        break;
                    }
                    len += 4;
                } else {
                    let charlen = utfc_ptr2len(p);
                    len += charlen;
                    p = p.add(charlen as usize);
                }
            }
        } else {
            for (i, (&arg_i, &arglen_i)) in args.iter().zip(arglens).enumerate() {
                let mut p: *const c_char = arg_i;
                let arg_end = p.add(arglen_i);

                while p < arg_end {
                    if crate::memory::cstr_first_byte(p) == b'\\'
                        || crate::memory::cstr_first_byte(p) == b'"'
                    {
                        len += 2;
                        p = p.add(1);
                    } else {
                        let charlen = utfc_ptr2len(p);
                        len += charlen;
                        p = p.add(charlen as usize);
                    }
                }

                if i != args.len() - 1 {
                    len += 4;
                }
            }
        }

        let mut buf_vec: Vec<c_char> = vec![0; (len as usize) + 1];
        let mut q = buf_vec.as_mut_ptr();
        *q = b'"' as c_char;
        q = q.add(1);

        if args.is_empty() {
            let mut p = arg;
            while !crate::memory::cstr_is_empty(p) {
                if crate::memory::cstr_first_byte(p) == b'\\'
                    && crate::memory::cstr_byte_at(p, 1) == b'\\'
                {
                    *q = b'\\' as c_char;
                    *q.add(1) = b'\\' as c_char;
                    q = q.add(2);
                    p = p.add(2);
                } else if crate::memory::cstr_first_byte(p) == b'\\'
                    && ascii_iswhite(crate::memory::cstr_char_at(p, 1))
                {
                    *q = *p.add(1);
                    q = q.add(1);
                    p = p.add(2);
                } else if crate::memory::cstr_first_byte(p) == b'\\'
                    || crate::memory::cstr_first_byte(p) == b'"'
                {
                    *q = b'\\' as c_char;
                    *q.add(1) = *p;
                    q = q.add(2);
                    p = p.add(1);
                } else if ascii_iswhite(crate::memory::cstr_first_char(p)) {
                    p = skipwhite(p);
                    if crate::memory::cstr_is_empty(p) {
                        break;
                    }
                    *q = b'"' as c_char;
                    *q.add(1) = b',' as c_char;
                    *q.add(2) = b' ' as c_char;
                    *q.add(3) = b'"' as c_char;
                    q = q.add(4);
                } else {
                    mb_copy_char(&raw mut p, &raw mut q);
                }
            }
        } else {
            for (i, (&arg_i, &arglen_i)) in args.iter().zip(arglens).enumerate() {
                let mut p: *const c_char = arg_i;
                let arg_end = p.add(arglen_i);

                while p < arg_end {
                    if crate::memory::cstr_first_byte(p) == b'\\'
                        || crate::memory::cstr_first_byte(p) == b'"'
                    {
                        *q = b'\\' as c_char;
                        *q.add(1) = *p;
                        q = q.add(2);
                        p = p.add(1);
                    } else {
                        mb_copy_char(&raw mut p, &raw mut q);
                    }
                }
                if i != args.len() - 1 {
                    *q = b'"' as c_char;
                    *q.add(1) = b',' as c_char;
                    *q.add(2) = b' ' as c_char;
                    *q.add(3) = b'"' as c_char;
                    q = q.add(4);
                }
            }
        }

        *q = b'"' as c_char;
        *q.add(1) = NUL;

        *lenp = len as usize;
        crate::memory::cstr_to_cstring(buf_vec.as_ptr())
    }
}

fn add_cmd_modifier(
    buf: *mut c_char,
    mod_str: impl crate::strings::IntoCCharPtr,
    multi_mods: &mut bool,
) -> usize {
    let mod_str = mod_str.into_c_char_ptr();
    {
        let mut result = crate::memory::cstr_len(mod_str);
        if *multi_mods {
            result += 1;
        }

        if !buf.is_null() {
            if *multi_mods {
                crate::memory::cstr_cat(buf, c" ");
            }
            crate::memory::cstr_cat(buf, mod_str);
        }

        *multi_mods = true;
        result
    }
}

/// Add modifiers from `cmod->cmod_split` to `buf` (`add_win_cmd_modifiers`).
pub fn add_win_cmd_modifiers(buf: *mut c_char, cmod: &Cmdmod, multi_mods: &mut bool) -> usize {
    let mut result: usize = 0;

    if (cmod.cmod_split & WSP_ABOVE) != 0 {
        result += add_cmd_modifier(buf, c"aboveleft", multi_mods);
    }
    if (cmod.cmod_split & WSP_BELOW) != 0 {
        result += add_cmd_modifier(buf, c"belowright", multi_mods);
    }
    if (cmod.cmod_split & WSP_BOT) != 0 {
        result += add_cmd_modifier(buf, c"botright", multi_mods);
    }

    if cmod.cmod_tab > 0 {
        let tabnr = cmod.cmod_tab - 1;
        if tabnr == tabpage_index(g_st().curtab) {
            result += add_cmd_modifier(buf, c"tab", multi_mods);
        } else {
            let s = format!("{tabnr}tab\0");
            result += add_cmd_modifier(buf, s.as_ptr().cast::<c_char>(), multi_mods);
        }
    }

    if (cmod.cmod_split & WSP_TOP) != 0 {
        result += add_cmd_modifier(buf, c"topleft", multi_mods);
    }
    if (cmod.cmod_split & WSP_VERT) != 0 {
        result += add_cmd_modifier(buf, c"vertical", multi_mods);
    }
    if (cmod.cmod_split & WSP_HOR) != 0 {
        result += add_cmd_modifier(buf, c"horizontal", multi_mods);
    }
    result
}

/// Generate text for the `cmod` command modifiers (`uc_mods`).
pub fn uc_mods(mut buf: *mut c_char, cmod: &Cmdmod, quote: bool) -> usize {
    unsafe {
        let mut multi_mods = false;

        const MOD_ENTRIES: [(c_int, &CStr); 12] = [
            (CMOD_BROWSE, c"browse"),
            (CMOD_CONFIRM, c"confirm"),
            (CMOD_HIDE, c"hide"),
            (CMOD_KEEPALT, c"keepalt"),
            (CMOD_KEEPJUMPS, c"keepjumps"),
            (CMOD_KEEPMARKS, c"keepmarks"),
            (CMOD_KEEPPATTERNS, c"keeppatterns"),
            (CMOD_LOCKMARKS, c"lockmarks"),
            (CMOD_NOSWAPFILE, c"noswapfile"),
            (CMOD_UNSILENT, c"unsilent"),
            (CMOD_NOAUTOCMD, c"noautocmd"),
            (CMOD_SANDBOX, c"sandbox"),
        ];

        let mut result: usize = if quote { 2 } else { 0 };
        if !buf.is_null() {
            if quote {
                *buf = b'"' as c_char;
                buf = buf.add(1);
            }
            *buf = NUL;
        }

        for &(flag, name) in &MOD_ENTRIES {
            if (cmod.cmod_flags & flag) != 0 {
                result += add_cmd_modifier(buf, name.as_ptr(), &mut multi_mods);
            }
        }

        if (cmod.cmod_flags & CMOD_SILENT) != 0 {
            let s = if (cmod.cmod_flags & CMOD_ERRSILENT) != 0 {
                c"silent!"
            } else {
                c"silent"
            };
            result += add_cmd_modifier(buf, s.as_ptr(), &mut multi_mods);
        }

        if cmod.cmod_verbose > 0 {
            let verbose_value = cmod.cmod_verbose - 1;
            if verbose_value == 1 {
                result += add_cmd_modifier(buf, c"verbose", &mut multi_mods);
            } else {
                let s = format!("{verbose_value}verbose\0");
                result += add_cmd_modifier(buf, s.as_ptr().cast::<c_char>(), &mut multi_mods);
            }
        }

        result += add_win_cmd_modifiers(buf, cmod, &mut multi_mods);

        if quote && !buf.is_null() {
            buf = buf.add(result - 2);
            *buf = b'"' as c_char;
        }
        result
    }
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
    code: impl crate::strings::IntoCCharPtr,
    len: usize,
    mut buf: *mut c_char,
    cmd: &mut Ucmd,
    eap: &mut Exarg,
    split_buf: &mut Option<CString>,
    split_len: &mut usize,
) -> usize {
    let code = code.into_c_char_ptr();
    unsafe {
        let mut result: usize = 0;
        let mut p = code.add(1);
        let mut l = len - 2;
        let mut quote: c_int = 0;
        let mut type_ = CodeType::None;

        if !vim_strchr(c"qQfF", c_int::from(crate::memory::cstr_first_byte(p))).is_null()
            && crate::memory::cstr_byte_at(p, 1) == b'-'
        {
            quote = if crate::memory::cstr_first_byte(p) == b'q'
                || crate::memory::cstr_first_byte(p) == b'Q'
            {
                1
            } else {
                2
            };
            p = p.add(2);
            l -= 2;
        }

        l += 1;
        if l <= 1 {
            // type_ = CodeType::None
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"args>", l))
        {
            type_ = CodeType::Args;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"bang>", l))
        {
            type_ = CodeType::Bang;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"count>", l))
        {
            type_ = CodeType::Count;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"line1>", l))
        {
            type_ = CodeType::Line1;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"line2>", l))
        {
            type_ = CodeType::Line2;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"range>", l))
        {
            type_ = CodeType::Range;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"lt>", l))
        {
            type_ = CodeType::Lt;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"reg>", l))
            || crate::memory::cstr_prefix_bytes(p, l)
                .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"register>", l))
        {
            type_ = CodeType::Register;
        } else if crate::memory::cstr_prefix_bytes(p, l)
            .eq_ignore_ascii_case(crate::memory::cstr_prefix_bytes(c"mods>", l))
        {
            type_ = CodeType::Mods;
        }

        match type_ {
            CodeType::Args => {
                if crate::memory::cstr_is_empty(eap.arg) {
                    if quote == 1 {
                        result = 2;
                        if !buf.is_null() {
                            crate::memory::cstr_cpy(buf, c"''");
                        }
                    } else {
                        result = 0;
                    }
                } else {
                    if (eap.argt & EX_NOSPC) != 0 && quote == 2 {
                        quote = 1;
                    }

                    match quote {
                        0 => {
                            result = crate::memory::cstr_len(eap.arg);
                            if !buf.is_null() {
                                crate::memory::cstr_cpy(buf, eap.arg);
                            }
                        }
                        1 => {
                            result = crate::memory::cstr_len(eap.arg) + 2;
                            let scan = eap.arg;
                            for &scan_byte in crate::memory::cstr_as_bytes(scan) {
                                if scan_byte == b'\\' || scan_byte == b'"' {
                                    result += 1;
                                }
                            }

                            if !buf.is_null() {
                                *buf = b'"' as c_char;
                                buf = buf.add(1);
                                let scan = eap.arg;
                                for &scan_byte in crate::memory::cstr_as_bytes(scan) {
                                    if scan_byte == b'\\' || scan_byte == b'"' {
                                        *buf = b'\\' as c_char;
                                        buf = buf.add(1);
                                    }
                                    *buf = scan_byte as c_char;
                                    buf = buf.add(1);
                                }
                                *buf = b'"' as c_char;
                            }
                        }
                        2 => {
                            if split_buf.is_none() {
                                *split_buf = Some(uc_split_args(
                                    eap.arg,
                                    &eap.args,
                                    &eap.arglens,
                                    split_len,
                                ));
                            }

                            result = *split_len;
                            if !buf.is_null() && result != 0 {
                                crate::memory::cstr_cpy(buf, split_buf.as_ref().unwrap().as_ptr());
                            }
                        }
                        _ => {}
                    }
                }
            }
            CodeType::Bang => {
                result = if eap.forceit != 0 { 1 } else { 0 };
                if quote != 0 {
                    result += 2;
                }
                if !buf.is_null() {
                    if quote != 0 {
                        *buf = b'"' as c_char;
                        buf = buf.add(1);
                    }
                    if eap.forceit != 0 {
                        *buf = b'!' as c_char;
                        buf = buf.add(1);
                    }
                    if quote != 0 {
                        *buf = b'"' as c_char;
                    }
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
                            cmd.uc_def
                        }
                    }
                };
                let num_str = format!("{num}\0");
                let num_len = num_str.len() - 1;
                result = num_len;
                if quote != 0 {
                    result += 2;
                }
                if !buf.is_null() {
                    if quote != 0 {
                        *buf = b'"' as c_char;
                        buf = buf.add(1);
                    }
                    crate::memory::cstr_cpy(buf, num_str.as_ptr().cast::<c_char>());
                    buf = buf.add(num_len);
                    if quote != 0 {
                        *buf = b'"' as c_char;
                    }
                }
            }
            CodeType::Mods => {
                result = uc_mods(buf, &g_st().cmdmod, quote != 0);
            }
            CodeType::Register => {
                result = if eap.regname != 0 { 1 } else { 0 };
                if quote != 0 {
                    result += 2;
                }
                if !buf.is_null() {
                    if quote != 0 {
                        *buf = b'\'' as c_char;
                        buf = buf.add(1);
                    }
                    if eap.regname != 0 {
                        *buf = eap.regname as c_char;
                        buf = buf.add(1);
                    }
                    if quote != 0 {
                        *buf = b'\'' as c_char;
                    }
                }
            }
            CodeType::Lt => {
                result = 1;
                if !buf.is_null() {
                    *buf = b'<' as c_char;
                }
            }
            CodeType::None => {
                result = usize::MAX;
                if !buf.is_null() {
                    *buf = b'<' as c_char;
                }
            }
        }

        result
    }
}

pub fn do_ucmd(eap: &mut Exarg, preview: bool) -> c_int {
    unsafe {
        let mut end: *mut c_char = ptr::null_mut();
        let mut split_len: usize = 0;
        let mut split_buf: Option<CString> = None;

        let cmd = if eap.cmdidx == CMD_USER {
            let u = ucmds_mut();
            &raw mut (u)[eap.useridx as usize]
        } else {
            &raw mut (&mut (*prevwin_curwin().w_buffer).b_ucmds)[eap.useridx as usize]
        };

        if preview {
            debug_assert!((*cmd).uc_preview_luaref > 0);
            return nlua_do_ucmd(&mut *cmd, eap, true);
        }

        if (*cmd).uc_luaref > 0 {
            nlua_do_ucmd(&mut *cmd, eap, false);
            return 0;
        }

        let mut buf_vec: Vec<c_char>;
        let mut buf: *mut c_char = ptr::null_mut();
        loop {
            let mut p = (*cmd).uc_rep.as_ptr().cast_mut();
            let mut q = buf;
            let mut totlen: usize = 0;

            loop {
                let start = vim_strchr(p, b'<' as c_int);
                if !start.is_null() {
                    end = vim_strchr(start.add(1), b'>' as c_int);
                }
                if !buf.is_null() {
                    let mut ksp = p;
                    while !crate::memory::cstr_is_empty(ksp)
                        && (crate::memory::cstr_first_byte(ksp)) != K_SPECIAL
                    {
                        ksp = ksp.add(1);
                    }
                    if (crate::memory::cstr_first_byte(ksp)) == K_SPECIAL
                        && (start.is_null() || ksp < start || end.is_null())
                        && (crate::memory::cstr_byte_at(ksp, 1)) == KS_SPECIAL
                        && *ksp.add(2) == KE_FILLER
                    {
                        let len = ksp.offset_from(p) as usize;
                        if len > 0 {
                            ptr::copy(p, q, len);
                            q = q.add(len);
                        }
                        *q = K_SPECIAL as c_char;
                        q = q.add(1);
                        p = ksp.add(3);
                        continue;
                    }
                }

                if start.is_null() || end.is_null() {
                    break;
                }

                end = end.add(1);

                let mut len = start.offset_from(p) as usize;
                if buf.is_null() {
                    totlen += len;
                } else {
                    ptr::copy(p, q, len);
                    q = q.add(len);
                }

                len = uc_check_code(
                    start,
                    end.offset_from(start) as usize,
                    q,
                    &mut *cmd,
                    eap,
                    &mut split_buf,
                    &mut split_len,
                );
                if len == usize::MAX {
                    p = start.add(1);
                    len = 1;
                } else {
                    p = end;
                }
                if buf.is_null() {
                    totlen += len;
                } else {
                    q = q.add(len);
                }
            }
            if !buf.is_null() {
                crate::memory::cstr_cpy(q, p);
                break;
            }

            totlen += crate::memory::cstr_len(p);
            buf_vec = vec![0; totlen + 1];
            buf = buf_vec.as_mut_ptr();
        }

        let mut save_current_sctx = g_st().current_sctx;
        let mut restore_current_sctx = false;
        if ((*cmd).uc_argt & EX_KEEPSCRIPT) == 0 {
            restore_current_sctx = true;
            save_current_sctx = g_st().current_sctx;
            g_st().current_sctx.sc_sid = (*cmd).uc_script_ctx.sc_sid;
        }
        do_cmdline(
            buf,
            eap.ea_getline,
            DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_KEYTYPED,
        );

        if restore_current_sctx {
            g_st().current_sctx = save_current_sctx;
        }

        0
    }
}

#[inline]
fn arena_str_obj(arena: &mut Arena, bytes: &[u8]) -> Object {
    let data = arena_memdupz(arena, bytes.as_ptr().cast(), bytes.len());
    string_obj(NvimString {
        data,
        size: bytes.len(),
    })
}

/// Gets a map of maps describing user-commands defined for buffer `buf` or
/// defined globally if `buf` is NULL (`commands_array`).
pub fn commands_array(buf: Option<&FileBuf>, arena: &mut Arena) -> Dict {
    {
        let u = ucmds_mut();
        let gap: &[Ucmd] = buf.map_or(u, |buf| &buf.b_ucmds);

        let mut rv = arena_dict(arena, gap.len());
        for cmd in gap {
            let mut arg_ch: u8 = 0;
            let mut d = arena_dict(arena, 16);

            put_c(&mut d, c"name", cstr_as_obj(cmd.uc_name.as_ptr()));
            put_c(&mut d, c"definition", cstr_as_obj(cmd.uc_rep.as_ptr()));
            put_c(
                &mut d,
                c"desc",
                cstr_as_obj(cmd.uc_desc.as_ref().map_or(ptr::null(), |s| s.as_ptr())),
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
                let cmd_compl = get_command_complete(cmd.uc_compl);
                put_c(
                    &mut d,
                    c"complete",
                    if cmd_compl.is_null() {
                        nil_obj()
                    } else {
                        cstr_as_obj(cmd_compl)
                    },
                );
            }
            put_c(
                &mut d,
                c"complete_arg",
                match &cmd.uc_compl_arg {
                    Some(arg) => cstr_as_obj(arg.as_ptr()),
                    None => nil_obj(),
                },
            );

            let mut obj = nil_obj();
            if (cmd.uc_argt & EX_COUNT) != 0 {
                if cmd.uc_def >= 0 {
                    let s = format!("{}", cmd.uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(c"0");
                }
            }
            put_c(&mut d, c"count", obj);

            obj = nil_obj();
            if (cmd.uc_argt & EX_RANGE) != 0 {
                if (cmd.uc_argt & EX_DFLALL) != 0 {
                    obj = cstr_as_obj(c"%");
                } else if cmd.uc_def >= 0 {
                    let s = format!("{}", cmd.uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(c".");
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
                        obj = cstr_as_obj(name.as_ptr());
                    }
                    break;
                }
            }
            put_c(&mut d, c"addr", obj);

            put_c(&mut rv, cmd.uc_name.as_ptr(), dict_obj(d));
        }
        let _ = size_of::<KeyValuePair>();
        rv
    }
}
