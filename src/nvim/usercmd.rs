//! User defined command support (`src/nvim/usercmd.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::private::helpers::{
    arena_dict, boolean_obj, cstr_as_obj, dict_obj, dict_put_cstr, integer_obj, luaref_obj,
    nil_obj, string_obj,
};
use crate::api::private::validate::{Dict, KeyValuePair, NvimString, Object};
use crate::autocmd::{DOCMD_NOWAIT, DOCMD_VERBOSE, HLF_8};
use crate::buffer::buf_T;
use crate::charset::{
    NUL, ascii_isalnum, ascii_isalpha, ascii_isdigit, ascii_isupper, ascii_iswhite, getdigits_int,
    skiptowhite, skipwhite,
};
use crate::cmdexpand::{
    EXPAND_ARGLIST, EXPAND_AUGROUP, EXPAND_BREAKPOINT, EXPAND_BUFFERS, EXPAND_CHECKHEALTH,
    EXPAND_COLORS, EXPAND_COMMANDS, EXPAND_COMPILER, EXPAND_DIFF_BUFFERS, EXPAND_DIRECTORIES,
    EXPAND_DIRS_IN_CDPATH, EXPAND_ENV_VARS, EXPAND_EVENTS, EXPAND_EXPRESSION, EXPAND_FILES,
    EXPAND_FILES_IN_PATH, EXPAND_FILETYPE, EXPAND_FILETYPECMD, EXPAND_FUNCTIONS, EXPAND_HELP,
    EXPAND_HIGHLIGHT, EXPAND_HISTORY, EXPAND_KEYMAP, EXPAND_LOCALES, EXPAND_LUA, EXPAND_MAPCLEAR,
    EXPAND_MAPPINGS, EXPAND_MENUS, EXPAND_MESSAGES, EXPAND_NOTHING, EXPAND_OWNSYNTAX,
    EXPAND_PACKADD, EXPAND_RETAB, EXPAND_RUNTIME, EXPAND_SCRIPTNAMES, EXPAND_SETTINGS,
    EXPAND_SHELLCMD, EXPAND_SHELLCMDLINE, EXPAND_SIGN, EXPAND_SYNTIME, EXPAND_TAGS,
    EXPAND_TAGS_LISTFILES, EXPAND_UNSUCCESSFUL, EXPAND_USER, EXPAND_USER_ADDR_TYPE,
    EXPAND_USER_CMD_FLAGS, EXPAND_USER_COMMANDS, EXPAND_USER_COMPLETE, EXPAND_USER_DEFINED,
    EXPAND_USER_LIST, EXPAND_USER_LUA, EXPAND_USER_NARGS, EXPAND_USER_VARS, HLF_D, cmd_addr_T,
};
use crate::eval::funcs::DOCMD_KEYTYPED;
use crate::eval::last_set_msg;
use crate::eval::typval::nlua_clear_ref;
use crate::ex_docmd::{
    ADDR_ARGUMENTS, ADDR_BUFFERS, ADDR_LINES, ADDR_LOADED_BUFFERS, ADDR_NONE, ADDR_OTHER,
    ADDR_QUICKFIX, ADDR_TABS, ADDR_WINDOWS, CMD_SIZE, CMD_USER, CMD_USER_BUF, CMOD_BROWSE,
    CMOD_CONFIRM, CMOD_ERRSILENT, CMOD_HIDE, CMOD_KEEPALT, CMOD_KEEPJUMPS, CMOD_KEEPMARKS,
    CMOD_KEEPPATTERNS, CMOD_LOCKMARKS, CMOD_NOAUTOCMD, CMOD_NOSWAPFILE, CMOD_SANDBOX, CMOD_SILENT,
    CMOD_UNSILENT, EX_ARGSPACE, EX_BANG, EX_BUFNAME, EX_COUNT, EX_DFLALL, EX_EXTRA, EX_KEEPSCRIPT,
    EX_NEEDARG, EX_NOSPC, EX_RANGE, EX_REGSTR, EX_TRLBAR, EX_XFILE, EX_ZEROR, KE_FILLER, cmdmod_T,
    do_cmdline, ends_excmd, exarg_T,
};
use crate::garray::{ga_clear, ga_grow, ga_init, garray_T};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::keycodes::{K_SPECIAL, KS_SPECIAL, replace_termcodes};
use crate::lua::converter::{LUA_NOREF, LuaRef};
use crate::lua::executor::{api_new_luaref, nlua_do_ucmd, nlua_funcref_str, nlua_set_sctx};
use crate::mapping::set_context_in_map_cmd;
use crate::math::{FAIL, OK};
use crate::mbyte::{mb_copy_char, utfc_ptr2len};
use crate::memory::{Arena, arena_memdupz, xfree, xmalloc, xstrdup};
use crate::menu::set_context_in_menu_cmd;
use crate::message::{
    emsg, message_filtered, msg, msg_ext_set_kind, msg_outtrans, msg_outtrans_special, msg_putchar,
    msg_puts, msg_puts_hl, msg_puts_title, semsg,
};
use crate::option_vars::*;
use crate::os::env::expand_T;
use crate::os::input::line_breakcheck;
use crate::runtime::sourcing_lnum;
use crate::strings::{
    gettext, strcat, strchr, strcmp, strcpy, strncmp, vim_strchr, vim_strnicmp_asc, xstrnsave,
};
use crate::window::{
    WSP_ABOVE, WSP_BELOW, WSP_BOT, WSP_HOR, WSP_TOP, WSP_VERT, prevwin_curwin, tabpage_index,
};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

pub(crate) const UC_BUFFER: c_int = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ucmd_T {
    pub uc_name: *mut c_char,
    pub uc_argt: u32,
    pub uc_rep: *mut c_char,
    pub uc_def: i64,
    pub uc_compl: c_int,
    pub uc_addr_type: cmd_addr_T,
    pub uc_script_ctx: crate::runtime::sctx_T,
    pub uc_compl_arg: *mut c_char,
    pub uc_compl_luaref: LuaRef,
    pub uc_preview_luaref: LuaRef,
    pub uc_luaref: LuaRef,
    pub uc_desc: *mut c_char,
}

const E_ARGUMENT_REQUIRED_FOR_STR: &CStr = c"E179: Argument required for %s";
const E_NO_SUCH_USER_DEFINED_COMMAND_STR: &CStr = c"E184: No such user-defined command: %s";
const E_COMPLETE_USED_WITHOUT_ALLOWING_ARGUMENTS: &CStr =
    c"E1208: -complete used without allowing arguments";
const E_NO_SUCH_USER_DEFINED_COMMAND_IN_CURRENT_BUFFER_STR: &CStr =
    c"E1237: No such user-defined command in current buffer: %s";

#[unsafe(no_mangle)]
pub static mut ucmds: garray_T = garray_T {
    ga_len: 0,
    ga_maxlen: 0,
    ga_itemsize: size_of::<ucmd_T>() as c_int,
    ga_growsize: 4,
    ga_data: ptr::null_mut(),
};

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
    expand: cmd_addr_T,
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

#[inline]
unsafe fn user_cmd(i: c_int) -> *mut ucmd_T {
    unsafe { ucmds.ga_data.cast::<ucmd_T>().add(i as usize) }
}

#[inline]
unsafe fn user_cmd_ga(gap: *const garray_T, i: c_int) -> *mut ucmd_T {
    unsafe { (*gap).ga_data.cast::<ucmd_T>().add(i as usize) }
}

/// Search for a user command that matches `eap->cmd` (`find_ucmd`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn find_ucmd(
    eap: *mut exarg_T,
    p: *mut c_char,
    full: *mut c_int,
    xp: *mut expand_T,
    complp: *mut c_int,
) -> *mut c_char {
    unsafe {
        let len = p.offset_from((*eap).cmd) as c_int;
        let mut matchlen: c_int = 0;
        let mut found = false;
        let mut possible = false;
        let mut amb_local = false;

        let mut gap: *mut garray_T = &raw mut (*(*prevwin_curwin()).w_buffer).b_ucmds;
        loop {
            let mut j: c_int = 0;
            while j < (*gap).ga_len {
                let uc = user_cmd_ga(gap, j);
                let mut cp = (*eap).cmd;
                let mut np = (*uc).uc_name;
                let mut k: c_int = 0;
                while k < len && *np != NUL && {
                    let c1 = *cp;
                    let c2 = *np;
                    cp = cp.add(1);
                    np = np.add(1);
                    c1 == c2
                } {
                    k += 1;
                }
                if k == len || (*np == NUL && ascii_isdigit(*(*eap).cmd.add(k as usize))) {
                    if k == len && found && *np != NUL {
                        if ptr::eq(gap, &raw const ucmds) {
                            return ptr::null_mut();
                        }
                        amb_local = true;
                    }

                    if !found || (k == len && *np == NUL) {
                        if k == len {
                            found = true;
                        } else {
                            possible = true;
                        }

                        if ptr::eq(gap, &raw const ucmds) {
                            (*eap).cmdidx = CMD_USER;
                        } else {
                            (*eap).cmdidx = CMD_USER_BUF;
                        }
                        (*eap).argt = (*uc).uc_argt;
                        (*eap).useridx = j;
                        (*eap).addr_type = (*uc).uc_addr_type;

                        if !complp.is_null() {
                            *complp = (*uc).uc_compl;
                        }
                        if !xp.is_null() {
                            (*xp).xp_luaref = (*uc).uc_compl_luaref;
                            (*xp).xp_arg = (*uc).uc_compl_arg;
                            (*xp).xp_script_ctx = (*uc).uc_script_ctx;
                            (*xp).xp_script_ctx.sc_lnum += sourcing_lnum();
                        }
                        matchlen = k;
                        if k == len && *np == NUL {
                            if !full.is_null() {
                                *full = 1;
                            }
                            amb_local = false;
                            break;
                        }
                    }
                }
                j += 1;
            }

            if j < (*gap).ga_len || ptr::eq(gap, &raw const ucmds) {
                break;
            }
            gap = &raw mut ucmds;
        }

        if amb_local {
            if !xp.is_null() {
                (*xp).xp_context = EXPAND_UNSUCCESSFUL;
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_in_user_cmd(
    xp: *mut expand_T,
    arg_in: *const c_char,
) -> *const c_char {
    unsafe {
        let mut arg = arg_in;
        let mut p: *const c_char;

        while *arg == b'-' as c_char {
            arg = arg.add(1);
            p = skiptowhite(arg);
            if *p == NUL {
                p = strchr(arg, b'=' as c_int);
                if p.is_null() {
                    (*xp).xp_context = EXPAND_USER_CMD_FLAGS;
                    (*xp).xp_pattern = arg.cast_mut();
                    return ptr::null();
                }

                let attr_len = p.offset_from(arg) as usize;
                if vim_strnicmp_asc(arg, c"complete".as_ptr(), attr_len) == 0 {
                    (*xp).xp_context = EXPAND_USER_COMPLETE;
                    (*xp).xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                } else if vim_strnicmp_asc(arg, c"nargs".as_ptr(), attr_len) == 0 {
                    (*xp).xp_context = EXPAND_USER_NARGS;
                    (*xp).xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                } else if vim_strnicmp_asc(arg, c"addr".as_ptr(), attr_len) == 0 {
                    (*xp).xp_context = EXPAND_USER_ADDR_TYPE;
                    (*xp).xp_pattern = p.add(1).cast_mut();
                    return ptr::null();
                }
                return ptr::null();
            }
            arg = skipwhite(p);
        }

        p = skiptowhite(arg);
        if *p == NUL {
            (*xp).xp_context = EXPAND_USER_COMMANDS;
            (*xp).xp_pattern = arg.cast_mut();
            return ptr::null();
        }

        skipwhite(p)
    }
}

/// Set the completion context for the argument of a user defined command (`set_context_in_user_cmdarg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_in_user_cmdarg(
    cmd: *const c_char,
    mut arg: *const c_char,
    argt: u32,
    context: c_int,
    xp: *mut expand_T,
    forceit: bool,
) -> *const c_char {
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
                c"map".as_ptr().cast_mut(),
                arg.cast_mut(),
                forceit,
                false,
                false,
                crate::cmdexpand::cmdidx_T::CMD_map,
            );
        }
        if (argt & EX_ARGSPACE) == 0 {
            let mut p = arg;
            while *p != NUL {
                if *p == b' ' as c_char {
                    arg = p.add(1);
                } else if *p == b'\\' as c_char && *p.add(1) != NUL {
                    p = p.add(1);
                }
                p = p.add(utfc_ptr2len(p) as usize);
            }
        }
        (*xp).xp_pattern = arg.cast_mut();
        (*xp).xp_context = context;

        ptr::null()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expand_user_command_name(idx: c_int) -> *mut c_char {
    unsafe { get_user_commands(ptr::null_mut(), idx - CMD_SIZE) }
}

/// Function given to `ExpandGeneric()` to obtain the list of user command names (`get_user_commands`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_commands(_xp: *mut expand_T, mut idx: c_int) -> *mut c_char {
    unsafe {
        let buf = (*prevwin_curwin()).w_buffer;

        if idx < (*buf).b_ucmds.ga_len {
            return (*user_cmd_ga(&raw const (*buf).b_ucmds, idx)).uc_name;
        }

        idx -= (*buf).b_ucmds.ga_len;
        if idx < ucmds.ga_len {
            let name = (*user_cmd(idx)).uc_name;

            for i in 0..(*buf).b_ucmds.ga_len {
                if strcmp(name, (*user_cmd_ga(&raw const (*buf).b_ucmds, i)).uc_name) == 0 {
                    return c"".as_ptr().cast_mut();
                }
            }
            return name;
        }
        ptr::null_mut()
    }
}

/// Get the name of user command `idx` (`get_user_command_name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_command_name(idx: c_int, cmdidx: c_int) -> *mut c_char {
    unsafe {
        if cmdidx == CMD_USER && idx < ucmds.ga_len {
            return (*user_cmd(idx)).uc_name;
        }
        if cmdidx == CMD_USER_BUF {
            let buf = (*prevwin_curwin()).w_buffer;
            if idx < (*buf).b_ucmds.ga_len {
                return (*user_cmd_ga(&raw const (*buf).b_ucmds, idx)).uc_name;
            }
        }
        ptr::null_mut()
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user address type names (`get_user_cmd_addr_type`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_cmd_addr_type(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    if idx < 0 || (idx as usize) >= ADDR_TYPE_COMPLETE.len() {
        return ptr::null_mut();
    }
    match ADDR_TYPE_COMPLETE[idx as usize].name {
        Some(s) => s.as_ptr().cast_mut(),
        None => ptr::null_mut(),
    }
}

/// Function given to `ExpandGeneric()` to obtain the list of user command attributes (`get_user_cmd_flags`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_cmd_flags(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_cmd_nargs(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_user_cmd_complete(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmdcomplete_type_to_str(
    expand: c_int,
    compl_arg: *const c_char,
) -> *mut c_char {
    unsafe {
        let cmd_compl = get_command_complete(expand);
        if cmd_compl.is_null() || expand == EXPAND_USER_LUA {
            return ptr::null_mut();
        }

        if expand == EXPAND_USER_LIST || expand == EXPAND_USER_DEFINED {
            let compl_bytes = CStr::from_ptr(cmd_compl).to_bytes();
            let arg_bytes = CStr::from_ptr(compl_arg).to_bytes();
            let buflen = compl_bytes.len() + arg_bytes.len() + 2;
            let buffer = xmalloc(buflen).cast::<u8>();
            ptr::copy_nonoverlapping(compl_bytes.as_ptr(), buffer, compl_bytes.len());
            *buffer.add(compl_bytes.len()) = b',';
            ptr::copy_nonoverlapping(
                arg_bytes.as_ptr(),
                buffer.add(compl_bytes.len() + 1),
                arg_bytes.len(),
            );
            *buffer.add(buflen - 1) = 0;
            return buffer.cast::<c_char>();
        }

        xstrdup(cmd_compl)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cmdcomplete_str_to_type(complete_str: *const c_char) -> c_int {
    unsafe {
        if strncmp(complete_str, c"custom,".as_ptr(), 7) == 0 {
            return EXPAND_USER_DEFINED;
        }
        if strncmp(complete_str, c"customlist,".as_ptr(), 11) == 0 {
            return EXPAND_USER_LIST;
        }

        for i in 0..(COMMAND_COMPLETE_LEN as c_int) {
            let cmd_compl = get_command_complete(i);
            if cmd_compl.is_null() {
                continue;
            }
            if strcmp(complete_str, cmd_compl) == 0 {
                return i;
            }
        }

        EXPAND_NOTHING
    }
}

unsafe fn uc_list(name: *mut c_char, name_len: usize) {
    unsafe {
        let mut found = false;

        msg_ext_set_kind(c"list_cmd".as_ptr());
        let mut gap: *const garray_T = &raw const (*(*prevwin_curwin()).w_buffer).b_ucmds;
        loop {
            let mut i: c_int = 0;
            while i < (*gap).ga_len {
                let cmd = user_cmd_ga(gap, i);
                let a = (*cmd).uc_argt;

                if strncmp(name, (*cmd).uc_name, name_len) != 0 || message_filtered((*cmd).uc_name)
                {
                    i += 1;
                    continue;
                }

                if !found {
                    msg_puts_title(gettext(
                        c"\n    Name              Args Address Complete    Definition".as_ptr(),
                    ));
                }
                found = true;
                msg_putchar(b'\n' as c_int);
                if got_int {
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
                if !ptr::eq(gap, &raw const ucmds) {
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

                msg_outtrans((*cmd).uc_name, HLF_D, false);
                len = CStr::from_ptr((*cmd).uc_name).to_bytes().len() + 4;

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
                        IObuff[len] = b'0' as c_char;
                        len += 1;
                    }
                    EX_EXTRA => {
                        IObuff[len] = b'*' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC) => {
                        IObuff[len] = b'?' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NEEDARG) => {
                        IObuff[len] = b'+' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG) => {
                        IObuff[len] = b'1' as c_char;
                        len += 1;
                    }
                    x if x == (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) => {
                        IObuff[len] = b'_' as c_char;
                        len += 1;
                    }
                    _ => {}
                }

                loop {
                    IObuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 5 - over {
                        break;
                    }
                }

                if (a & (EX_RANGE | EX_COUNT)) != 0 {
                    if (a & EX_COUNT) != 0 {
                        let s = format!("{}c", (*cmd).uc_def);
                        let bytes = s.as_bytes();
                        let copy_len = bytes.len().min(IOSIZE - 1 - len);
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            (&raw mut IObuff[0]).add(len),
                            copy_len,
                        );
                        len += copy_len;
                    } else if (a & EX_DFLALL) != 0 {
                        IObuff[len] = b'%' as c_char;
                        len += 1;
                    } else if (*cmd).uc_def >= 0 {
                        let s = format!("{}", (*cmd).uc_def);
                        let bytes = s.as_bytes();
                        let copy_len = bytes.len().min(IOSIZE - 1 - len);
                        ptr::copy_nonoverlapping(
                            bytes.as_ptr().cast::<c_char>(),
                            (&raw mut IObuff[0]).add(len),
                            copy_len,
                        );
                        len += copy_len;
                    } else {
                        IObuff[len] = b'.' as c_char;
                        len += 1;
                    }
                }

                loop {
                    IObuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 8 - over {
                        break;
                    }
                }

                for entry in &ADDR_TYPE_COMPLETE {
                    if entry.expand == ADDR_NONE {
                        break;
                    }
                    if entry.expand != ADDR_LINES && entry.expand == (*cmd).uc_addr_type {
                        if let Some(short) = entry.shortname {
                            let bytes = short.to_bytes();
                            let copy_len = bytes.len().min(IOSIZE - 1 - len);
                            ptr::copy_nonoverlapping(
                                bytes.as_ptr().cast::<c_char>(),
                                (&raw mut IObuff[0]).add(len),
                                copy_len,
                            );
                            len += copy_len;
                        }
                        break;
                    }
                }

                loop {
                    IObuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 13 - over {
                        break;
                    }
                }

                let cmd_compl = get_command_complete((*cmd).uc_compl);
                if !cmd_compl.is_null() {
                    let bytes = CStr::from_ptr(cmd_compl).to_bytes();
                    let copy_len = bytes.len().min(IOSIZE - 1 - len);
                    ptr::copy_nonoverlapping(
                        bytes.as_ptr().cast::<c_char>(),
                        (&raw mut IObuff[0]).add(len),
                        copy_len,
                    );
                    len += copy_len;
                }

                loop {
                    IObuff[len] = b' ' as c_char;
                    len += 1;
                    if (len as i64) >= 25 - over {
                        break;
                    }
                }

                IObuff[len] = NUL;
                msg_outtrans(&raw const IObuff[0], 0, false);

                if (*cmd).uc_luaref != LUA_NOREF {
                    let fn_str = nlua_funcref_str((*cmd).uc_luaref, ptr::null_mut(), true);
                    msg_puts_hl(fn_str, HLF_8, false);
                    xfree(fn_str.cast());
                    if *(*cmd).uc_rep != NUL {
                        msg_puts(c"\n                                               ".as_ptr());
                    }
                }

                msg_outtrans_special(
                    (*cmd).uc_rep,
                    false,
                    if name_len == 0 { Columns - 47 } else { 0 },
                );
                if p_verbose > 0 {
                    last_set_msg((*cmd).uc_script_ctx);
                }
                line_breakcheck();
                if got_int {
                    break;
                }
                i += 1;
            }
            if ptr::eq(gap, &raw const ucmds) || i < (*gap).ga_len {
                break;
            }
            gap = &raw const ucmds;
        }

        if !found {
            msg(gettext(c"No user-defined commands found".as_ptr()), 0);
        }
    }
}

/// Parse address type argument (`parse_addr_type_arg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parse_addr_type_arg(
    value: *mut c_char,
    vallen: c_int,
    addr_type_arg: *mut cmd_addr_T,
) -> c_int {
    unsafe {
        for entry in &ADDR_TYPE_COMPLETE {
            if entry.expand == ADDR_NONE {
                break;
            }
            if let Some(name) = entry.name {
                let a = name.to_bytes().len() as c_int == vallen;
                let b = strncmp(value, name.as_ptr(), vallen as usize) == 0;
                if a && b {
                    *addr_type_arg = entry.expand;
                    return OK;
                }
            }
        }

        let err = value;
        let mut i = 0usize;
        while *err.add(i) != NUL && !ascii_iswhite(*err.add(i)) {
            i += 1;
        }
        *err.add(i) = NUL;
        semsg!(
            gettext(c"E180: Invalid address type value: %s".as_ptr()),
            err,
        );
        FAIL
    }
}

/// Parse a completion argument `value[vallen]` (`parse_compl_arg`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parse_compl_arg(
    value: *const c_char,
    vallen: c_int,
    complp: *mut c_int,
    argt: *mut u32,
    compl_arg: *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut arg: *const c_char = ptr::null();
        let mut arglen: usize = 0;
        let mut valend: c_int = vallen;

        for i in 0..vallen {
            if *value.add(i as usize) == b',' as c_char {
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
            if CStr::from_ptr(cmd_compl).to_bytes().len() as c_int == valend
                && strncmp(value, cmd_compl, valend as usize) == 0
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
            semsg!(gettext(c"E180: Invalid complete value: %s".as_ptr()), value);
            return FAIL;
        }

        if *complp != EXPAND_USER_DEFINED && *complp != EXPAND_USER_LIST && !arg.is_null() {
            emsg(gettext(
                c"E468: Completion argument only allowed for custom completion".as_ptr(),
            ));
            return FAIL;
        }

        if (*complp == EXPAND_USER_DEFINED || *complp == EXPAND_USER_LIST) && arg.is_null() {
            emsg(gettext(
                c"E467: Custom completion requires a function argument".as_ptr(),
            ));
            return FAIL;
        }

        if !arg.is_null() {
            *compl_arg = xstrnsave(arg, arglen);
        }
        OK
    }
}

unsafe fn uc_scan_attr(
    attr: *mut c_char,
    len: usize,
    argt: *mut u32,
    def: *mut c_int,
    flags: *mut c_int,
    complp: *mut c_int,
    compl_arg: *mut *mut c_char,
    addr_type_arg: *mut cmd_addr_T,
) -> c_int {
    unsafe {
        if len == 0 {
            emsg(gettext(c"E175: No attribute specified".as_ptr()));
            return FAIL;
        }

        if vim_strnicmp_asc(attr, c"bang".as_ptr(), len) == 0 {
            *argt |= EX_BANG;
        } else if vim_strnicmp_asc(attr, c"buffer".as_ptr(), len) == 0 {
            *flags |= UC_BUFFER;
        } else if vim_strnicmp_asc(attr, c"register".as_ptr(), len) == 0 {
            *argt |= EX_REGSTR;
        } else if vim_strnicmp_asc(attr, c"keepscript".as_ptr(), len) == 0 {
            *argt |= EX_KEEPSCRIPT;
        } else if vim_strnicmp_asc(attr, c"bar".as_ptr(), len) == 0 {
            *argt |= EX_TRLBAR;
        } else {
            let mut val: *mut c_char = ptr::null_mut();
            let mut vallen: usize = 0;
            let mut attrlen: usize = len;

            for i in 0..len {
                if *attr.add(i) == b'=' as c_char {
                    val = attr.add(i + 1);
                    vallen = len - i - 1;
                    attrlen = i;
                    break;
                }
            }

            if vim_strnicmp_asc(attr, c"nargs".as_ptr(), attrlen) == 0 {
                if vallen == 1 {
                    match *val as u8 {
                        b'0' => {}
                        b'1' => *argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG,
                        b'*' => *argt |= EX_EXTRA,
                        b'?' => *argt |= EX_EXTRA | EX_NOSPC,
                        b'+' => *argt |= EX_EXTRA | EX_NEEDARG,
                        b'_' => *argt |= EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE,
                        _ => {
                            emsg(gettext(c"E176: Invalid number of arguments".as_ptr()));
                            return FAIL;
                        }
                    }
                } else {
                    emsg(gettext(c"E176: Invalid number of arguments".as_ptr()));
                    return FAIL;
                }
            } else if vim_strnicmp_asc(attr, c"range".as_ptr(), attrlen) == 0 {
                *argt |= EX_RANGE;
                if vallen == 1 && *val == b'%' as c_char {
                    *argt |= EX_DFLALL;
                } else if !val.is_null() {
                    let mut p = val;
                    if *def >= 0 {
                        emsg(gettext(c"E177: Count cannot be specified twice".as_ptr()));
                        return FAIL;
                    }

                    *def = getdigits_int(&raw mut p, true, 0);
                    *argt |= EX_ZEROR;

                    if p != val.add(vallen) || vallen == 0 {
                        emsg(gettext(c"E178: Invalid default value for count".as_ptr()));
                        return FAIL;
                    }
                }
                if *addr_type_arg == ADDR_NONE {
                    *addr_type_arg = ADDR_LINES;
                }
            } else if vim_strnicmp_asc(attr, c"count".as_ptr(), attrlen) == 0 {
                *argt |= EX_COUNT | EX_ZEROR | EX_RANGE;
                if *addr_type_arg == ADDR_NONE {
                    *addr_type_arg = ADDR_OTHER;
                }

                if !val.is_null() {
                    let mut p = val;
                    if *def >= 0 {
                        emsg(gettext(c"E177: Count cannot be specified twice".as_ptr()));
                        return FAIL;
                    }

                    *def = getdigits_int(&raw mut p, true, 0);

                    if p != val.add(vallen) {
                        emsg(gettext(c"E178: Invalid default value for count".as_ptr()));
                        return FAIL;
                    }
                }

                if *def < 0 {
                    *def = 0;
                }
            } else if vim_strnicmp_asc(attr, c"complete".as_ptr(), attrlen) == 0 {
                if val.is_null() {
                    semsg!(
                        gettext(E_ARGUMENT_REQUIRED_FOR_STR.as_ptr()),
                        c"-complete".as_ptr(),
                    );
                    return FAIL;
                }

                if parse_compl_arg(val, vallen as c_int, complp, argt, compl_arg) == FAIL {
                    return FAIL;
                }
            } else if vim_strnicmp_asc(attr, c"addr".as_ptr(), attrlen) == 0 {
                *argt |= EX_RANGE;
                if val.is_null() {
                    semsg!(
                        gettext(E_ARGUMENT_REQUIRED_FOR_STR.as_ptr()),
                        c"-addr".as_ptr(),
                    );
                    return FAIL;
                }
                if parse_addr_type_arg(val, vallen as c_int, addr_type_arg) == FAIL {
                    return FAIL;
                }
                if *addr_type_arg != ADDR_LINES {
                    *argt |= EX_ZEROR;
                }
            } else {
                let ch = *attr.add(len);
                *attr.add(len) = NUL;
                semsg!(gettext(c"E181: Invalid attribute: %s".as_ptr()), attr);
                *attr.add(len) = ch;
                return FAIL;
            }
        }

        OK
    }
}

/// Check for a valid user command name (`uc_validate_name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_validate_name(mut name: *mut c_char) -> *mut c_char {
    unsafe {
        if ascii_isalpha(*name) {
            while ascii_isalnum(*name) {
                name = name.add(1);
            }
        }
        if ends_excmd(c_int::from(*name as u8)) == 0 && !ascii_iswhite(*name) {
            return ptr::null_mut();
        }
        name
    }
}

/// Create a new user command `name`, if one doesn't already exist (`uc_add_command`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_add_command(
    name: *mut c_char,
    name_len: usize,
    rep: *const c_char,
    argt: u32,
    def: i64,
    flags: c_int,
    context: c_int,
    compl_arg: *mut c_char,
    mut compl_luaref: LuaRef,
    mut preview_luaref: LuaRef,
    addr_type: cmd_addr_T,
    mut luaref: LuaRef,
    desc: *const c_char,
    force: bool,
) -> c_int {
    unsafe {
        let mut cmd: *mut ucmd_T = ptr::null_mut();
        let mut cmp: c_int = 1;
        let mut rep_buf: *mut c_char = ptr::null_mut();

        replace_termcodes(
            rep,
            CStr::from_ptr(rep).to_bytes().len(),
            &raw mut rep_buf,
            0,
            0,
            ptr::null_mut(),
            p_cpo,
        );
        if rep_buf.is_null() {
            rep_buf = xstrdup(rep);
        }

        let gap: *mut garray_T = if (flags & UC_BUFFER) != 0 {
            let g = &raw mut (*curbuf).b_ucmds;
            if (*g).ga_itemsize == 0 {
                ga_init(g, size_of::<ucmd_T>() as c_int, 4);
            }
            g
        } else {
            &raw mut ucmds
        };

        let mut i: c_int = 0;
        while i < (*gap).ga_len {
            cmd = user_cmd_ga(gap, i);
            let len = CStr::from_ptr((*cmd).uc_name).to_bytes().len();
            cmp = strncmp(name, (*cmd).uc_name, name_len);
            if cmp == 0 {
                if name_len < len {
                    cmp = -1;
                } else if name_len > len {
                    cmp = 1;
                }
            }

            if cmp == 0 {
                if !force
                    && ((*cmd).uc_script_ctx.sc_sid != current_sctx.sc_sid
                        || (*cmd).uc_script_ctx.sc_seq == current_sctx.sc_seq)
                {
                    semsg!(
                        gettext(c"E174: Command already exists: add ! to replace it: %s".as_ptr()),
                        name,
                    );
                    xfree(rep_buf.cast());
                    xfree(compl_arg.cast());
                    nlua_clear_ref(&mut luaref);
                    nlua_clear_ref(&mut compl_luaref);
                    nlua_clear_ref(&mut preview_luaref);
                    return FAIL;
                }

                xfree((*cmd).uc_rep.cast());
                (*cmd).uc_rep = ptr::null_mut();
                xfree((*cmd).uc_compl_arg.cast());
                (*cmd).uc_compl_arg = ptr::null_mut();
                xfree((*cmd).uc_desc.cast());
                (*cmd).uc_desc = ptr::null_mut();
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
            ga_grow(gap, 1);

            let p = xstrnsave(name, name_len);

            cmd = user_cmd_ga(gap, i);
            ptr::copy(cmd, cmd.add(1), ((*gap).ga_len - i) as usize);

            (*gap).ga_len += 1;

            (*cmd).uc_name = p;
        }

        (*cmd).uc_rep = rep_buf;
        (*cmd).uc_desc = if !desc.is_null() && *desc != NUL {
            xstrdup(desc)
        } else {
            ptr::null_mut()
        };
        (*cmd).uc_argt = argt;
        (*cmd).uc_def = def;
        (*cmd).uc_compl = context;
        (*cmd).uc_script_ctx = current_sctx;
        (*cmd).uc_script_ctx.sc_lnum += sourcing_lnum();
        nlua_set_sctx(&raw mut (*cmd).uc_script_ctx);
        (*cmd).uc_compl_arg = compl_arg;
        (*cmd).uc_compl_luaref = compl_luaref;
        (*cmd).uc_preview_luaref = preview_luaref;
        (*cmd).uc_addr_type = addr_type;
        (*cmd).uc_luaref = luaref;

        OK
    }
}

/// `:command ...` (`ex_command`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_command(eap: *mut exarg_T) {
    unsafe {
        let mut argt: u32 = 0;
        let mut def: c_int = -1;
        let mut flags: c_int = 0;
        let mut context: c_int = EXPAND_NOTHING;
        let mut compl_arg: *mut c_char = ptr::null_mut();
        let mut addr_type_arg: cmd_addr_T = ADDR_NONE;
        let has_attr = *(*eap).arg == b'-' as c_char;

        let mut p = (*eap).arg;

        while *p == b'-' as c_char {
            p = p.add(1);
            let end = skiptowhite(p);
            if uc_scan_attr(
                p,
                end.offset_from(p) as usize,
                &raw mut argt,
                &raw mut def,
                &raw mut flags,
                &raw mut context,
                &raw mut compl_arg,
                &raw mut addr_type_arg,
            ) == FAIL
            {
                xfree(compl_arg.cast());
                return;
            }
            p = skipwhite(end);
        }

        let name = p;
        let end = uc_validate_name(name);
        if end.is_null() {
            emsg(gettext(c"E182: Invalid command name".as_ptr()));
            xfree(compl_arg.cast());
            return;
        }
        let name_len = end.offset_from(name) as usize;

        p = skipwhite(end);
        if !has_attr && ends_excmd(c_int::from(*p as u8)) != 0 {
            uc_list(name, name_len);
        } else if !ascii_isupper(*name) {
            emsg(gettext(
                c"E183: User defined commands must start with an uppercase letter".as_ptr(),
            ));
        } else if name_len <= 4 && strncmp(name, c"Next".as_ptr(), name_len) == 0 {
            emsg(gettext(
                c"E841: Reserved name, cannot be used for user defined command".as_ptr(),
            ));
        } else if context > 0 && (argt & EX_EXTRA) == 0 {
            emsg(gettext(E_COMPLETE_USED_WITHOUT_ALLOWING_ARGUMENTS.as_ptr()));
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
                (*eap).forceit != 0,
            );
            return;
        }

        xfree(compl_arg.cast());
    }
}

/// `:comclear` (`ex_comclear`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_comclear(_eap: *mut exarg_T) {
    unsafe {
        uc_clear(&raw mut ucmds);
        if !curbuf.is_null() {
            uc_clear(&raw mut (*curbuf).b_ucmds);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_ucmd(cmd: *mut ucmd_T) {
    unsafe {
        xfree((*cmd).uc_name.cast());
        xfree((*cmd).uc_rep.cast());
        xfree((*cmd).uc_compl_arg.cast());
        xfree((*cmd).uc_desc.cast());
        nlua_clear_ref(&mut (*cmd).uc_compl_luaref);
        nlua_clear_ref(&mut (*cmd).uc_luaref);
        nlua_clear_ref(&mut (*cmd).uc_preview_luaref);
    }
}

/// Clear all user commands for `gap` (`uc_clear`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_clear(gap: *mut garray_T) {
    unsafe {
        for i in 0..(*gap).ga_len {
            free_ucmd(user_cmd_ga(gap, i));
        }
        ga_clear(gap);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_delcommand(eap: *mut exarg_T) {
    unsafe {
        let mut i: c_int;
        let mut cmd: *mut ucmd_T = ptr::null_mut();
        let mut res: c_int = -1;
        let mut arg: *const c_char = (*eap).arg;
        let mut buffer_only = false;

        if strncmp(arg, c"-buffer".as_ptr(), 7) == 0 && ascii_iswhite(*arg.add(7)) {
            buffer_only = true;
            arg = skipwhite(arg.add(7));
        }

        let mut gap: *mut garray_T = &raw mut (*curbuf).b_ucmds;
        loop {
            i = 0;
            while i < (*gap).ga_len {
                cmd = user_cmd_ga(gap, i);
                res = strcmp(arg, (*cmd).uc_name);
                if res <= 0 {
                    break;
                }
                i += 1;
            }
            if ptr::eq(gap, &raw const ucmds) || res == 0 || buffer_only {
                break;
            }
            gap = &raw mut ucmds;
        }

        if res != 0 {
            let _ = &mut i;
            semsg!(
                gettext(if buffer_only {
                    E_NO_SUCH_USER_DEFINED_COMMAND_IN_CURRENT_BUFFER_STR.as_ptr()
                } else {
                    E_NO_SUCH_USER_DEFINED_COMMAND_STR.as_ptr()
                }),
                arg,
            );
            return;
        }

        free_ucmd(cmd);

        (*gap).ga_len -= 1;

        if i < (*gap).ga_len {
            ptr::copy(cmd.add(1), cmd, ((*gap).ga_len - i) as usize);
        }
    }
}

/// Split a string by unescaped whitespace (space & tab) (`uc_split_args_iter`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_split_args_iter(
    arg: *const c_char,
    arglen: usize,
    end: *mut usize,
    buf: *mut c_char,
    len: *mut usize,
) -> bool {
    unsafe {
        if arglen == 0 {
            return true;
        }

        let mut pos = *end;
        while pos < arglen && ascii_iswhite(*arg.add(pos)) {
            pos += 1;
        }

        let mut l: usize = 0;
        while pos < arglen - 1 {
            if *arg.add(pos) == b'\\' as c_char
                && (*arg.add(pos + 1) == b'\\' as c_char || ascii_iswhite(*arg.add(pos + 1)))
            {
                pos += 1;
                *buf.add(l) = *arg.add(pos);
                l += 1;
            } else {
                *buf.add(l) = *arg.add(pos);
                l += 1;
            }
            if ascii_iswhite(*arg.add(pos + 1)) {
                *end = pos + 1;
                *len = l;
                return false;
            }
            pos += 1;
        }

        if pos < arglen && !ascii_iswhite(*arg.add(pos)) {
            *buf.add(l) = *arg.add(pos);
            l += 1;
        }

        *len = l;
        true
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_nargs_upper_bound(arg: *const c_char, arglen: usize) -> usize {
    unsafe {
        let mut was_white = true;
        let mut nargs: usize = 0;
        for i in 0..arglen {
            let is_white = ascii_iswhite(*arg.add(i));
            if was_white && !is_white {
                nargs += 1;
            }
            was_white = is_white;
        }
        nargs
    }
}

/// Split and quote args for `<f-args>` (`uc_split_args`).
unsafe fn uc_split_args(
    arg: *const c_char,
    args: *mut *mut c_char,
    arglens: *const usize,
    argc: usize,
    lenp: *mut usize,
) -> *mut c_char {
    unsafe {
        let mut len: c_int = 2;
        if args.is_null() {
            let mut p = arg;
            while *p != NUL {
                if *p == b'\\' as c_char && *p.add(1) == b'\\' as c_char {
                    len += 2;
                    p = p.add(2);
                } else if *p == b'\\' as c_char && ascii_iswhite(*p.add(1)) {
                    len += 1;
                    p = p.add(2);
                } else if *p == b'\\' as c_char || *p == b'"' as c_char {
                    len += 2;
                    p = p.add(1);
                } else if ascii_iswhite(*p) {
                    p = skipwhite(p);
                    if *p == NUL {
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
            for i in 0..argc {
                let mut p: *const c_char = *args.add(i);
                let arg_end = p.add(*arglens.add(i));

                while p < arg_end {
                    if *p == b'\\' as c_char || *p == b'"' as c_char {
                        len += 2;
                        p = p.add(1);
                    } else {
                        let charlen = utfc_ptr2len(p);
                        len += charlen;
                        p = p.add(charlen as usize);
                    }
                }

                if i != argc - 1 {
                    len += 4;
                }
            }
        }

        let buf = xmalloc((len as usize) + 1).cast::<c_char>();
        let mut q = buf;
        *q = b'"' as c_char;
        q = q.add(1);

        if args.is_null() {
            let mut p = arg;
            while *p != NUL {
                if *p == b'\\' as c_char && *p.add(1) == b'\\' as c_char {
                    *q = b'\\' as c_char;
                    *q.add(1) = b'\\' as c_char;
                    q = q.add(2);
                    p = p.add(2);
                } else if *p == b'\\' as c_char && ascii_iswhite(*p.add(1)) {
                    *q = *p.add(1);
                    q = q.add(1);
                    p = p.add(2);
                } else if *p == b'\\' as c_char || *p == b'"' as c_char {
                    *q = b'\\' as c_char;
                    *q.add(1) = *p;
                    q = q.add(2);
                    p = p.add(1);
                } else if ascii_iswhite(*p) {
                    p = skipwhite(p);
                    if *p == NUL {
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
            for i in 0..argc {
                let mut p: *const c_char = *args.add(i);
                let arg_end = p.add(*arglens.add(i));

                while p < arg_end {
                    if *p == b'\\' as c_char || *p == b'"' as c_char {
                        *q = b'\\' as c_char;
                        *q.add(1) = *p;
                        q = q.add(2);
                        p = p.add(1);
                    } else {
                        mb_copy_char(&raw mut p, &raw mut q);
                    }
                }
                if i != argc - 1 {
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
        buf
    }
}

unsafe fn add_cmd_modifier(
    buf: *mut c_char,
    mod_str: *const c_char,
    multi_mods: *mut bool,
) -> usize {
    unsafe {
        let mut result = CStr::from_ptr(mod_str).to_bytes().len();
        if *multi_mods {
            result += 1;
        }

        if !buf.is_null() {
            if *multi_mods {
                strcat(buf, c" ".as_ptr());
            }
            strcat(buf, mod_str);
        }

        *multi_mods = true;
        result
    }
}

/// Add modifiers from `cmod->cmod_split` to `buf` (`add_win_cmd_modifiers`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn add_win_cmd_modifiers(
    buf: *mut c_char,
    cmod: *const cmdmod_T,
    multi_mods: *mut bool,
) -> usize {
    unsafe {
        let mut result: usize = 0;

        if ((*cmod).cmod_split & WSP_ABOVE) != 0 {
            result += add_cmd_modifier(buf, c"aboveleft".as_ptr(), multi_mods);
        }
        if ((*cmod).cmod_split & WSP_BELOW) != 0 {
            result += add_cmd_modifier(buf, c"belowright".as_ptr(), multi_mods);
        }
        if ((*cmod).cmod_split & WSP_BOT) != 0 {
            result += add_cmd_modifier(buf, c"botright".as_ptr(), multi_mods);
        }

        if (*cmod).cmod_tab > 0 {
            let tabnr = (*cmod).cmod_tab - 1;
            if tabnr == tabpage_index(curtab) {
                result += add_cmd_modifier(buf, c"tab".as_ptr(), multi_mods);
            } else {
                let s = format!("{tabnr}tab\0");
                result += add_cmd_modifier(buf, s.as_ptr().cast::<c_char>(), multi_mods);
            }
        }

        if ((*cmod).cmod_split & WSP_TOP) != 0 {
            result += add_cmd_modifier(buf, c"topleft".as_ptr(), multi_mods);
        }
        if ((*cmod).cmod_split & WSP_VERT) != 0 {
            result += add_cmd_modifier(buf, c"vertical".as_ptr(), multi_mods);
        }
        if ((*cmod).cmod_split & WSP_HOR) != 0 {
            result += add_cmd_modifier(buf, c"horizontal".as_ptr(), multi_mods);
        }
        result
    }
}

/// Generate text for the `cmod` command modifiers (`uc_mods`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uc_mods(
    mut buf: *mut c_char,
    cmod: *const cmdmod_T,
    quote: bool,
) -> usize {
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
            if ((*cmod).cmod_flags & flag) != 0 {
                result += add_cmd_modifier(buf, name.as_ptr(), &raw mut multi_mods);
            }
        }

        if ((*cmod).cmod_flags & CMOD_SILENT) != 0 {
            let s = if ((*cmod).cmod_flags & CMOD_ERRSILENT) != 0 {
                c"silent!"
            } else {
                c"silent"
            };
            result += add_cmd_modifier(buf, s.as_ptr(), &raw mut multi_mods);
        }

        if (*cmod).cmod_verbose > 0 {
            let verbose_value = (*cmod).cmod_verbose - 1;
            if verbose_value == 1 {
                result += add_cmd_modifier(buf, c"verbose".as_ptr(), &raw mut multi_mods);
            } else {
                let s = format!("{verbose_value}verbose\0");
                result += add_cmd_modifier(buf, s.as_ptr().cast::<c_char>(), &raw mut multi_mods);
            }
        }

        result += add_win_cmd_modifiers(buf, cmod, &raw mut multi_mods);

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
unsafe fn uc_check_code(
    code: *mut c_char,
    len: usize,
    mut buf: *mut c_char,
    cmd: *mut ucmd_T,
    eap: *mut exarg_T,
    split_buf: *mut *mut c_char,
    split_len: *mut usize,
) -> usize {
    unsafe {
        let mut result: usize = 0;
        let mut p = code.add(1);
        let mut l = len - 2;
        let mut quote: c_int = 0;
        let mut type_ = CodeType::None;

        if !vim_strchr(c"qQfF".as_ptr(), c_int::from(*p as u8)).is_null()
            && *p.add(1) == b'-' as c_char
        {
            quote = if *p == b'q' as c_char || *p == b'Q' as c_char {
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
        } else if vim_strnicmp_asc(p, c"args>".as_ptr(), l) == 0 {
            type_ = CodeType::Args;
        } else if vim_strnicmp_asc(p, c"bang>".as_ptr(), l) == 0 {
            type_ = CodeType::Bang;
        } else if vim_strnicmp_asc(p, c"count>".as_ptr(), l) == 0 {
            type_ = CodeType::Count;
        } else if vim_strnicmp_asc(p, c"line1>".as_ptr(), l) == 0 {
            type_ = CodeType::Line1;
        } else if vim_strnicmp_asc(p, c"line2>".as_ptr(), l) == 0 {
            type_ = CodeType::Line2;
        } else if vim_strnicmp_asc(p, c"range>".as_ptr(), l) == 0 {
            type_ = CodeType::Range;
        } else if vim_strnicmp_asc(p, c"lt>".as_ptr(), l) == 0 {
            type_ = CodeType::Lt;
        } else if vim_strnicmp_asc(p, c"reg>".as_ptr(), l) == 0
            || vim_strnicmp_asc(p, c"register>".as_ptr(), l) == 0
        {
            type_ = CodeType::Register;
        } else if vim_strnicmp_asc(p, c"mods>".as_ptr(), l) == 0 {
            type_ = CodeType::Mods;
        }

        match type_ {
            CodeType::Args => {
                if *(*eap).arg == NUL {
                    if quote == 1 {
                        result = 2;
                        if !buf.is_null() {
                            strcpy(buf, c"''".as_ptr());
                        }
                    } else {
                        result = 0;
                    }
                } else {
                    if ((*eap).argt & EX_NOSPC) != 0 && quote == 2 {
                        quote = 1;
                    }

                    match quote {
                        0 => {
                            result = CStr::from_ptr((*eap).arg).to_bytes().len();
                            if !buf.is_null() {
                                strcpy(buf, (*eap).arg);
                            }
                        }
                        1 => {
                            result = CStr::from_ptr((*eap).arg).to_bytes().len() + 2;
                            let mut scan = (*eap).arg;
                            while *scan != NUL {
                                if *scan == b'\\' as c_char || *scan == b'"' as c_char {
                                    result += 1;
                                }
                                scan = scan.add(1);
                            }

                            if !buf.is_null() {
                                *buf = b'"' as c_char;
                                buf = buf.add(1);
                                let mut scan = (*eap).arg;
                                while *scan != NUL {
                                    if *scan == b'\\' as c_char || *scan == b'"' as c_char {
                                        *buf = b'\\' as c_char;
                                        buf = buf.add(1);
                                    }
                                    *buf = *scan;
                                    buf = buf.add(1);
                                    scan = scan.add(1);
                                }
                                *buf = b'"' as c_char;
                            }
                        }
                        2 => {
                            if (*split_buf).is_null() {
                                *split_buf = uc_split_args(
                                    (*eap).arg,
                                    (*eap).args,
                                    (*eap).arglens,
                                    (*eap).argc,
                                    split_len,
                                );
                            }

                            result = *split_len;
                            if !buf.is_null() && result != 0 {
                                strcpy(buf, *split_buf);
                            }
                        }
                        _ => {}
                    }
                }
            }
            CodeType::Bang => {
                result = if (*eap).forceit != 0 { 1 } else { 0 };
                if quote != 0 {
                    result += 2;
                }
                if !buf.is_null() {
                    if quote != 0 {
                        *buf = b'"' as c_char;
                        buf = buf.add(1);
                    }
                    if (*eap).forceit != 0 {
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
                    CodeType::Line1 => i64::from((*eap).line1),
                    CodeType::Line2 => i64::from((*eap).line2),
                    CodeType::Range => i64::from((*eap).addr_count),
                    _ => {
                        if (*eap).addr_count > 0 {
                            i64::from((*eap).line2)
                        } else {
                            (*cmd).uc_def
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
                    strcpy(buf, num_str.as_ptr().cast::<c_char>());
                    buf = buf.add(num_len);
                    if quote != 0 {
                        *buf = b'"' as c_char;
                    }
                }
            }
            CodeType::Mods => {
                result = uc_mods(buf, &raw const cmdmod, quote != 0);
            }
            CodeType::Register => {
                result = if (*eap).regname != 0 { 1 } else { 0 };
                if quote != 0 {
                    result += 2;
                }
                if !buf.is_null() {
                    if quote != 0 {
                        *buf = b'\'' as c_char;
                        buf = buf.add(1);
                    }
                    if (*eap).regname != 0 {
                        *buf = (*eap).regname as c_char;
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn do_ucmd(eap: *mut exarg_T, preview: bool) -> c_int {
    unsafe {
        let mut end: *mut c_char = ptr::null_mut();
        let mut split_len: usize = 0;
        let mut split_buf: *mut c_char = ptr::null_mut();

        let cmd = if (*eap).cmdidx == CMD_USER {
            user_cmd((*eap).useridx)
        } else {
            user_cmd_ga(
                &raw const (*(*prevwin_curwin()).w_buffer).b_ucmds,
                (*eap).useridx,
            )
        };

        if preview {
            debug_assert!((*cmd).uc_preview_luaref > 0);
            return nlua_do_ucmd(cmd, eap, true);
        }

        if (*cmd).uc_luaref > 0 {
            nlua_do_ucmd(cmd, eap, false);
            return 0;
        }

        let mut buf: *mut c_char = ptr::null_mut();
        loop {
            let mut p = (*cmd).uc_rep;
            let mut q = buf;
            let mut totlen: usize = 0;

            loop {
                let start = vim_strchr(p, b'<' as c_int);
                if !start.is_null() {
                    end = vim_strchr(start.add(1), b'>' as c_int);
                }
                if !buf.is_null() {
                    let mut ksp = p;
                    while *ksp != NUL && (*ksp as u8) != K_SPECIAL {
                        ksp = ksp.add(1);
                    }
                    if (*ksp as u8) == K_SPECIAL
                        && (start.is_null() || ksp < start || end.is_null())
                        && (*ksp.add(1) as u8) == KS_SPECIAL
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
                    cmd,
                    eap,
                    &raw mut split_buf,
                    &raw mut split_len,
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
                strcpy(q, p);
                break;
            }

            totlen += CStr::from_ptr(p).to_bytes().len();
            buf = xmalloc(totlen + 1).cast::<c_char>();
        }

        let mut save_current_sctx = current_sctx;
        let mut restore_current_sctx = false;
        if ((*cmd).uc_argt & EX_KEEPSCRIPT) == 0 {
            restore_current_sctx = true;
            save_current_sctx = current_sctx;
            current_sctx.sc_sid = (*cmd).uc_script_ctx.sc_sid;
        }
        do_cmdline(
            buf,
            (*eap).ea_getline,
            (*eap).cookie,
            DOCMD_VERBOSE | DOCMD_NOWAIT | DOCMD_KEYTYPED,
        );

        if restore_current_sctx {
            current_sctx = save_current_sctx;
        }
        xfree(buf.cast());
        xfree(split_buf.cast());

        0
    }
}

#[inline]
unsafe fn arena_str_obj(arena: *mut Arena, bytes: &[u8]) -> Object {
    unsafe {
        let data = arena_memdupz(arena, bytes.as_ptr().cast(), bytes.len());
        string_obj(NvimString {
            data,
            size: bytes.len(),
        })
    }
}

/// Gets a map of maps describing user-commands defined for buffer `buf` or
/// defined globally if `buf` is NULL (`commands_array`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn commands_array(buf: *mut buf_T, arena: *mut Arena) -> Dict {
    unsafe {
        let gap: *const garray_T = if buf.is_null() {
            &raw const ucmds
        } else {
            &raw const (*buf).b_ucmds
        };

        let mut rv = arena_dict(arena, (*gap).ga_len as usize);
        for i in 0..(*gap).ga_len {
            let mut arg_ch: u8 = 0;
            let mut d = arena_dict(arena, 16);
            let cmd = user_cmd_ga(gap, i);

            dict_put_cstr(&mut d, c"name".as_ptr(), cstr_as_obj((*cmd).uc_name));
            dict_put_cstr(&mut d, c"definition".as_ptr(), cstr_as_obj((*cmd).uc_rep));
            dict_put_cstr(&mut d, c"desc".as_ptr(), cstr_as_obj((*cmd).uc_desc));
            dict_put_cstr(
                &mut d,
                c"script_id".as_ptr(),
                integer_obj(i64::from((*cmd).uc_script_ctx.sc_sid)),
            );
            dict_put_cstr(
                &mut d,
                c"bang".as_ptr(),
                boolean_obj(((*cmd).uc_argt & EX_BANG) != 0),
            );
            dict_put_cstr(
                &mut d,
                c"bar".as_ptr(),
                boolean_obj(((*cmd).uc_argt & EX_TRLBAR) != 0),
            );
            dict_put_cstr(
                &mut d,
                c"register".as_ptr(),
                boolean_obj(((*cmd).uc_argt & EX_REGSTR) != 0),
            );
            dict_put_cstr(
                &mut d,
                c"keepscript".as_ptr(),
                boolean_obj(((*cmd).uc_argt & EX_KEEPSCRIPT) != 0),
            );

            if (*cmd).uc_preview_luaref != LUA_NOREF {
                dict_put_cstr(
                    &mut d,
                    c"preview".as_ptr(),
                    luaref_obj(api_new_luaref((*cmd).uc_preview_luaref)),
                );
            }

            if (*cmd).uc_luaref != LUA_NOREF {
                dict_put_cstr(
                    &mut d,
                    c"callback".as_ptr(),
                    luaref_obj(api_new_luaref((*cmd).uc_luaref)),
                );
            }

            match (*cmd).uc_argt & (EX_EXTRA | EX_NOSPC | EX_NEEDARG | EX_ARGSPACE) {
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
            dict_put_cstr(&mut d, c"nargs".as_ptr(), arena_str_obj(arena, arg_slice));

            if (*cmd).uc_compl_luaref != LUA_NOREF {
                dict_put_cstr(
                    &mut d,
                    c"complete".as_ptr(),
                    luaref_obj(api_new_luaref((*cmd).uc_compl_luaref)),
                );
            } else {
                let cmd_compl = get_command_complete((*cmd).uc_compl);
                dict_put_cstr(
                    &mut d,
                    c"complete".as_ptr(),
                    if cmd_compl.is_null() {
                        nil_obj()
                    } else {
                        cstr_as_obj(cmd_compl)
                    },
                );
            }
            dict_put_cstr(
                &mut d,
                c"complete_arg".as_ptr(),
                if (*cmd).uc_compl_arg.is_null() {
                    nil_obj()
                } else {
                    cstr_as_obj((*cmd).uc_compl_arg)
                },
            );

            let mut obj = nil_obj();
            if ((*cmd).uc_argt & EX_COUNT) != 0 {
                if (*cmd).uc_def >= 0 {
                    let s = format!("{}", (*cmd).uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(c"0".as_ptr());
                }
            }
            dict_put_cstr(&mut d, c"count".as_ptr(), obj);

            obj = nil_obj();
            if ((*cmd).uc_argt & EX_RANGE) != 0 {
                if ((*cmd).uc_argt & EX_DFLALL) != 0 {
                    obj = cstr_as_obj(c"%".as_ptr());
                } else if (*cmd).uc_def >= 0 {
                    let s = format!("{}", (*cmd).uc_def);
                    obj = arena_str_obj(arena, s.as_bytes());
                } else {
                    obj = cstr_as_obj(c".".as_ptr());
                }
            }
            dict_put_cstr(&mut d, c"range".as_ptr(), obj);

            obj = nil_obj();
            for entry in &ADDR_TYPE_COMPLETE {
                if entry.expand == ADDR_NONE {
                    break;
                }
                if entry.expand != ADDR_LINES && entry.expand == (*cmd).uc_addr_type {
                    if let Some(name) = entry.name {
                        obj = cstr_as_obj(name.as_ptr());
                    }
                    break;
                }
            }
            dict_put_cstr(&mut d, c"addr".as_ptr(), obj);

            dict_put_cstr(&mut rv, (*cmd).uc_name, dict_obj(d));
        }
        let _ = size_of::<KeyValuePair>();
        rv
    }
}
