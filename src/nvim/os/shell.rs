//! Shell execution and wildcard expansion (`src/nvim/os/shell.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::{K_UIMESSAGES, read_buffer_into};
use crate::charset::{NL, NUL, skipwhite};
#[cfg(unix)]
use crate::charset::{ascii_iswhite, backslash_halve};
use crate::errors::*;
use crate::eval::vars::{VV_SHELL_ERROR, set_vim_var_nr};
use crate::event::r#loop::{
    NS_PER_SEC, loop_poll_events, loop_queues, main_loop_ptr, uv_err_name, uv_strerror,
};
use crate::event::multiqueue::{
    Event, multiqueue_empty, multiqueue_free, multiqueue_new_child, multiqueue_put_event,
};
use crate::event::proc::{LibuvProc, proc_spawn, proc_stop, proc_wait};
use crate::event::rstream::{RStream, rstream_init, rstream_start};
use crate::event::stream::{Stream, StreamCbData, stream_may_close};
use crate::event::wstream::{
    wstream_init, wstream_new_vec_buffer, wstream_set_write_cb, wstream_write,
};
use crate::ex_cmds::{K_SHELL_OPT_READ, K_SHELL_OPT_WRITE, check_secure, make_filter_cmd};
use crate::ex_docmd::CAR;
#[cfg(unix)]
use crate::fileio::fopen;
use crate::fileio::{SEEK_END, SEEK_SET, fclose, fread, vim_fseek, vim_ftell, vim_tempname};
use crate::globals::*;
#[cfg(windows)]
use crate::mbyte::CP_UTF8;
use crate::mbyte::{UTF8LEN_TAB_ZERO, utfc_ptr2len_len};
use crate::memline::ml_append;
use crate::memory::xstrlcpy;
#[cfg(unix)]
use crate::message::msg;
use crate::message::{
    emsg, msg_end, msg_ext_no_fast, msg_ext_set_append, msg_ext_set_kind, msg_multiline,
    msg_outnum, msg_outtrans, msg_putchar, msg_puts, msg_sb_eol, msg_schedule_semsg, msg_start,
    semsg, smsg, verbose_enter, verbose_leave,
};
use crate::option_vars::*;
#[cfg(windows)]
use crate::os::env::{os_getenv_cstr, os_setenv, restore_env_var};
use crate::os::fs::{MAXPATHL, os_fopen, os_remove};
#[cfg(unix)]
use crate::os::fs::{os_can_exe, os_isdir, os_path_exists};
use crate::os::signal::{signal_accept_deadly, signal_reject_deadly};
#[cfg(unix)]
use crate::os::time::os_delay;
use crate::os::time::os_hrtime;
#[cfg(unix)]
use crate::path::{
    EW_DIR, EW_EXEC, EW_FILE, EW_KEEPDOLLAR, EW_NOTFOUND, EW_SHELLCMD, EW_SILENT, add_pathsep,
    invocation_path_tail, path_has_wildcard, path_tail,
};
use crate::profile::{Proftime, prof_child_enter, prof_child_exit};
use crate::strings::{gettext, vim_strnsave_unquoted, vim_strsave_escaped_ext};
use crate::tag::tag_freematch;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_flush, ui_has};
#[cfg(unix)]
use std::ffi::CStr;
use std::ffi::c_char;
use std::ptr;

use crate::autocmd::PROF_YES;
use crate::change::TAB;
use crate::channel::{K_PROC_TYPE_UV, STDERR_FILENO, STDOUT_FILENO};
use crate::diff::{K_SHELL_OPT_DO_OUT, K_SHELL_OPT_SILENT};
use crate::eval::typval::Varnumber;
use crate::state::MODE_EXTERNCMD;

const OUT_DATA_THRESHOLD: usize = 1024 * 10;
const MAX_CHUNK_SIZE: usize = OUT_DATA_THRESHOLD / 2;

#[cfg(unix)]
const SHELL_SPECIAL: &CStr = c"\t \"&'$;<>()\\|\n";

const K_SHELL_OPT_EXPAND: i32 = 2;
const K_SHELL_OPT_HIDE_MESS: i32 = 64;

#[cfg(unix)]
const STYLE_ECHO: i32 = 0;
#[cfg(unix)]
const STYLE_GLOB: i32 = 1;
#[cfg(unix)]
const STYLE_VIMGLOB: i32 = 2;
#[cfg(unix)]
const STYLE_PRINT: i32 = 3;
#[cfg(unix)]
const STYLE_BT: i32 = 4;
#[cfg(unix)]
const STYLE_GLOBSTAR: i32 = 5;

const HLF_SE: i32 = 72;
const HLF_SO: i32 = 73;

struct ShellModuleState {
    #[cfg(unix)]
    did_find_nul: bool,
    throttle_started: u64,
    throttle_received: usize,
    throttle_visit: usize,
    throttle_pulse_msg: [c_char; 4],
    last_skipped: [c_char; MAX_CHUNK_SIZE],
    last_skipped_len: usize,
}

impl ShellModuleState {
    const fn new() -> Self {
        Self {
            #[cfg(unix)]
            did_find_nul: false,
            throttle_started: 0,
            throttle_received: 0,
            throttle_visit: 0,
            throttle_pulse_msg: [b' ' as c_char, b' ' as c_char, b' ' as c_char, NUL],
            last_skipped: [0; MAX_CHUNK_SIZE],
            last_skipped_len: 0,
        }
    }
}

static SH_STATE: crate::globals::GlobalState<ShellModuleState> =
    crate::globals::GlobalState::new(ShellModuleState::new());

#[inline]
fn sh_st() -> &'static mut ShellModuleState {
    unsafe { &mut *SH_STATE.as_ptr() }
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleOutputCP() -> u32;
    fn GetConsoleCP() -> u32;
    fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
    fn SetConsoleCP(wCodePageID: u32) -> i32;
}

#[cfg(unix)]
fn save_patterns(num_pat: i32, pat: *mut *mut c_char, file: &mut Vec<std::ffi::CString>) {
    unsafe {
        file.clear();
        file.reserve(num_pat as usize);
        for i in 0..(num_pat as usize) {
            let mut v: Vec<c_char> = crate::memory::cstr_to_cchar_vec(*pat.add(i));
            backslash_halve(v.as_mut_ptr());
            file.push(crate::memory::cstr_to_cstring(v.as_ptr()));
        }
    }
}

#[cfg(unix)]
fn have_wildcard(num: i32, file: *mut *mut c_char) -> bool {
    unsafe {
        for i in 0..(num as usize) {
            if path_has_wildcard(*file.add(i), true) {
                return true;
            }
        }
        false
    }
}

#[cfg(unix)]
fn have_dollars(num: i32, file: *mut *mut c_char) -> bool {
    unsafe {
        for i in 0..(num as usize) {
            if crate::strings::vim_haschr(*file.add(i), b'$' as i32) {
                return true;
            }
        }
        false
    }
}

/// Performs wildcard pattern matching using the shell.
#[cfg(unix)]
pub fn os_expand_wildcards(
    num_pat: i32,
    pat: *mut *mut c_char,
    file: &mut Vec<std::ffi::CString>,
    flags: i32,
) -> bool {
    const SH_VIMGLOB_FUNC: &CStr =
        c"vimglob() { while [ $# -ge 1 ]; do printf '%s\\n' \"$1\"; shift; done }; vimglob >";
    const SH_GLOBSTAR_OPT: &CStr = c"[[ ${BASH_VERSINFO[0]} -ge 4 ]] && shopt -s globstar; ";

    unsafe {
        let mut extra_shell_arg: *mut c_char = ptr::null_mut();
        let mut shellopts: i32 = K_SHELL_OPT_EXPAND | K_SHELL_OPT_SILENT;
        let mut shell_style: i32 = STYLE_ECHO;
        let mut ampersand = false;

        #[cfg(unix)]
        let is_fish_shell = {
            let tail = invocation_path_tail(opt_vars().p_sh, None);
            let bytes = crate::memory::cstr_as_bytes(tail);
            bytes.starts_with(b"fish")
        };
        #[cfg(not(unix))]
        let is_fish_shell = false;

        file.clear();

        if !have_wildcard(num_pat, pat) {
            save_patterns(num_pat, pat, file);
            return true;
        }

        if g_st().sandbox != 0 && check_secure() {
            return false;
        }

        if g_st().secure != 0 {
            for i in 0..(num_pat as usize) {
                if crate::strings::vim_haschr(*pat.add(i), b'`' as i32) && check_secure() {
                    return false;
                }
            }
        }

        let Some(tempname_cstr) = vim_tempname() else {
            emsg(gettext(E_NOTMP));
            return false;
        };
        let tempname = tempname_cstr.as_ptr();

        if num_pat == 1 && **pat == b'`' as c_char {
            let pat0_len = crate::memory::cstr_len(*pat);
            if pat0_len > 2 && crate::memory::cstr_byte_at(*pat, pat0_len - 1) == b'`' {
                shell_style = STYLE_BT;
            }
        }
        if shell_style != STYLE_BT {
            let sh_len = crate::memory::cstr_len(opt_vars().p_sh);
            if sh_len >= 3 {
                let tail3 = crate::memory::cstr_as_bytes(opt_vars().p_sh.wrapping_add(sh_len - 3));
                if tail3 == b"csh" {
                    shell_style = STYLE_GLOB;
                } else if tail3 == b"zsh" {
                    shell_style = STYLE_PRINT;
                }
            }
        }
        if shell_style == STYLE_ECHO {
            let sh_tail = crate::memory::cstr_as_bytes(path_tail(opt_vars().p_sh));
            if sh_tail.windows(4).any(|w| w == b"bash") {
                shell_style = STYLE_GLOBSTAR;
            } else if sh_tail.windows(2).any(|w| w == b"sh") {
                shell_style = STYLE_VIMGLOB;
            }
        }

        let mut len = crate::memory::cstr_len(tempname) + 29;
        if shell_style == STYLE_VIMGLOB {
            len += SH_VIMGLOB_FUNC.to_bytes().len();
        } else if shell_style == STYLE_GLOBSTAR {
            len += SH_VIMGLOB_FUNC.to_bytes().len() + SH_GLOBSTAR_OPT.to_bytes().len();
        }

        for i in 0..(num_pat as usize) {
            len += 1;
            let p_i = *pat.add(i);
            let mut j = 0;
            while crate::memory::cstr_byte_at(p_i, j) != 0 {
                if crate::strings::vim_haschr(
                    SHELL_SPECIAL,
                    crate::memory::cstr_byte_at(p_i, j) as i32,
                ) {
                    len += 1;
                }
                len += 1;
                j += 1;
            }
        }

        if is_fish_shell {
            len += b"egin; end".len();
        }

        let mut command_vec = vec![0 as c_char; len];
        let command = command_vec.as_mut_ptr();

        if shell_style == STYLE_BT {
            if is_fish_shell {
                {
                    let s = c"begin; ".as_ptr();
                    std::ptr::copy_nonoverlapping(
                        s,
                        command,
                        std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len(),
                    );
                };
            } else {
                crate::memory::cstr_cpy(command, c"(");
            }
            crate::memory::cstr_cat(command, (*pat).add(1));
            let mut p = command.add(crate::memory::cstr_len(command) - 1);
            if is_fish_shell {
                *p = b';' as c_char;
                p = p.sub(1);
                crate::memory::cstr_cat(command, c" end");
            } else {
                *p = b')' as c_char;
                p = p.sub(1);
            }
            while (p as usize) > (command as usize)
                && ascii_iswhite(crate::memory::cstr_first_char(p))
            {
                p = p.sub(1);
            }
            if crate::memory::cstr_first_byte(p) == b'&' {
                ampersand = true;
                *p = b' ' as c_char;
            }
            crate::memory::cstr_cat(command, c">");
        } else {
            *command = NUL;
            if shell_style == STYLE_GLOB {
                if (flags & EW_NOTFOUND) != 0 {
                    {
                        let d = command;
                        let s = c"set nonomatch; ".as_ptr();
                        std::ptr::copy_nonoverlapping(
                            s,
                            d.add(crate::memory::cstr_len(d)),
                            std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len(),
                        );
                    };
                } else {
                    {
                        let d = command;
                        let s = c"unset nonomatch; ".as_ptr();
                        std::ptr::copy_nonoverlapping(
                            s,
                            d.add(crate::memory::cstr_len(d)),
                            std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len(),
                        );
                    };
                }
            }
            if shell_style == STYLE_GLOB {
                crate::memory::cstr_cat(command, c"glob >");
            } else if shell_style == STYLE_PRINT {
                crate::memory::cstr_cat(command, c"print -N >");
            } else if shell_style == STYLE_VIMGLOB {
                crate::memory::cstr_cat(command, SH_VIMGLOB_FUNC);
            } else if shell_style == STYLE_GLOBSTAR {
                crate::memory::cstr_cat(command, SH_GLOBSTAR_OPT);
                crate::memory::cstr_cat(command, SH_VIMGLOB_FUNC);
            } else {
                crate::memory::cstr_cat(command, c"echo >");
            }
        }

        crate::memory::cstr_cat(command, tempname);

        if shell_style != STYLE_BT {
            for i in 0..(num_pat as usize) {
                let mut intick = false;
                let p_i = *pat.add(i);

                let mut p = command.add(crate::memory::cstr_len(command));
                *p = b' ' as c_char;
                p = p.add(1);
                let mut j = 0;
                while crate::memory::cstr_byte_at(p_i, j) != 0 {
                    let ch = *p_i.add(j);
                    if ch == b'`' as c_char {
                        intick = !intick;
                    } else if ch == b'\\' as c_char && crate::memory::cstr_byte_at(p_i, j + 1) != 0
                    {
                        let next_ch = *p_i.add(j + 1);
                        if intick
                            || crate::strings::vim_haschr(SHELL_SPECIAL, next_ch as u8 as i32)
                            || next_ch == b'`' as c_char
                        {
                            *p = b'\\' as c_char;
                            p = p.add(1);
                        }
                        j += 1;
                    } else if !intick
                        && ((flags & EW_KEEPDOLLAR) == 0 || ch != b'$' as c_char)
                        && crate::strings::vim_haschr(SHELL_SPECIAL, ch as u8 as i32)
                    {
                        *p = b'\\' as c_char;
                        p = p.add(1);
                    }

                    *p = *p_i.add(j);
                    p = p.add(1);
                    j += 1;
                }
                *p = NUL;
            }
        }

        if (flags & EW_SILENT) != 0 {
            shellopts |= K_SHELL_OPT_HIDE_MESS;
        }

        if ampersand {
            crate::memory::cstr_cat(command, c"&");
        }

        if shell_style == STYLE_PRINT {
            extra_shell_arg = c"-G".as_ptr().cast_mut();
        } else if shell_style == STYLE_GLOB && !have_dollars(num_pat, pat) {
            extra_shell_arg = c"-f".as_ptr().cast_mut();
        }

        let res = call_shell(command, shellopts, extra_shell_arg);

        if ampersand {
            os_delay(10, true);
        }

        drop(command_vec);

        if res != 0 {
            os_remove(tempname);
            if (flags & EW_SILENT) == 0 {
                msg_putchar(b'\n' as i32);
                g_st().cmdline_row = g_st().rows_global - 1;
                msg(gettext(E_WILDEXPAND), 0);
                msg_start();
            }

            if shell_style == STYLE_BT {
                return false;
            }
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return true;
            }
            return false;
        }

        let fd = fopen(tempname, c"rb".as_ptr());
        if fd.is_null() {
            if (flags & EW_SILENT) == 0 {
                msg(gettext(E_WILDEXPAND), 0);
                msg_start();
            }
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return true;
            }
            return false;
        }
        let fseek_res = vim_fseek(fd, 0, SEEK_END);
        if fseek_res < 0 {
            fclose(fd);
            return false;
        }
        let templen = vim_ftell(fd) as i64;
        if templen < 0 {
            fclose(fd);
            return false;
        }
        len = templen as usize;
        vim_fseek(fd, 0, SEEK_SET);
        let mut buffer_vec = vec![0 as c_char; len + 1];
        let buffer = buffer_vec.as_mut_ptr();
        let readlen = fread(buffer.cast(), 1, len, fd);
        fclose(fd);
        os_remove(tempname);
        if readlen != len {
            semsg!(gettext(E_CANT_READ_FILE_STR), tempname);
            return false;
        }
        drop(tempname_cstr);

        let mut count: i32 = 0;
        if shell_style == STYLE_ECHO {
            *buffer.add(len) = b'\n' as c_char;
            let mut p = buffer;
            while crate::memory::cstr_first_byte(p) != b'\n' {
                while crate::memory::cstr_first_byte(p) != b' '
                    && crate::memory::cstr_first_byte(p) != b'\n'
                {
                    p = p.add(1);
                }
                p = skipwhite(p);
                count += 1;
            }
        } else if shell_style == STYLE_BT
            || shell_style == STYLE_VIMGLOB
            || shell_style == STYLE_GLOBSTAR
        {
            *buffer.add(len) = NUL;
            let mut p = buffer;
            while !crate::memory::cstr_is_empty(p) {
                while crate::memory::cstr_first_byte(p) != b'\n' && !crate::memory::cstr_is_empty(p)
                {
                    p = p.add(1);
                }
                if !crate::memory::cstr_is_empty(p) {
                    p = p.add(1);
                }
                p = skipwhite(p);
                count += 1;
            }
        } else {
            let mut check_spaces = false;
            if shell_style == STYLE_PRINT && !sh_st().did_find_nul {
                *buffer.add(len) = NUL;
                if len != 0 && (crate::memory::cstr_len(buffer) as i32) < (len as i32) {
                    sh_st().did_find_nul = true;
                } else {
                    check_spaces = true;
                }
            }

            if len != 0 && crate::memory::cstr_byte_at(buffer, len - 1) == 0 {
                len -= 1;
            } else {
                *buffer.add(len) = NUL;
            }
            let mut p = buffer;
            while (p as usize) < (buffer.add(len) as usize) {
                if crate::memory::cstr_is_empty(p)
                    || (crate::memory::cstr_first_byte(p) == b' ' && check_spaces)
                {
                    count += 1;
                    *p = NUL;
                }
                p = p.add(1);
            }
            if len != 0 {
                count += 1;
            }
        }
        debug_assert!(
            crate::memory::cstr_byte_at(buffer, len) == 0
                || crate::memory::cstr_byte_at(buffer, len) == b'\n'
        );

        if count == 0 {
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return true;
            }
            return false;
        }
        let mut raw_entries: Vec<*mut c_char> = Vec::with_capacity(count as usize);

        let mut p = buffer;
        for _ in 0..(count as usize) {
            raw_entries.push(p);
            if shell_style == STYLE_ECHO
                || shell_style == STYLE_BT
                || shell_style == STYLE_VIMGLOB
                || shell_style == STYLE_GLOBSTAR
            {
                while !(shell_style == STYLE_ECHO && crate::memory::cstr_first_byte(p) == b' ')
                    && crate::memory::cstr_first_byte(p) != b'\n'
                    && !crate::memory::cstr_is_empty(p)
                {
                    p = p.add(1);
                }
                if p == buffer.add(len) {
                    *p = NUL;
                } else {
                    *p = NUL;
                    p = skipwhite(p.add(1));
                }
            } else {
                while !crate::memory::cstr_is_empty(p) && (p as usize) < (buffer.add(len) as usize)
                {
                    p = p.add(1);
                }
                p = p.add(1);
            }
        }

        for entry in raw_entries {
            if (flags & EW_NOTFOUND) == 0 && !os_path_exists(entry) {
                continue;
            }

            let dir = os_isdir(entry);
            if (dir && (flags & EW_DIR) == 0) || (!dir && (flags & EW_FILE) == 0) {
                continue;
            }

            if !dir
                && (flags & EW_EXEC) != 0
                && !os_can_exe(entry, None, (flags & EW_SHELLCMD) == 0)
            {
                continue;
            }

            let entry_len = crate::memory::cstr_len(entry);
            let mut new_vec = vec![0 as c_char; entry_len + 1 + usize::from(dir)];
            crate::memory::cstr_cpy(new_vec.as_mut_ptr(), entry);
            if dir {
                add_pathsep(new_vec.as_mut_ptr());
            }
            file.push(crate::memory::cstr_to_cstring(new_vec.as_ptr()));
        }
        drop(buffer_vec);

        if file.is_empty() {
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return true;
            }
            return false;
        }

        true
    }
}

/// Builds the argument vector for running the user-configured 'shell' (opt_vars().p_sh)
/// with an optional command prefixed by 'shellcmdflag' (opt_vars().p_shcf).
pub fn shell_build_argv(
    cmd: impl crate::strings::IntoCCharPtr,
    extra_args: impl crate::strings::IntoCCharPtr,
) -> Vec<std::ffi::CString> {
    let cmd = cmd.into_c_char_ptr();
    let extra_args = extra_args.into_c_char_ptr();
    {
        let mut rv: Vec<std::ffi::CString> = Vec::new();
        tokenize(opt_vars().p_sh, &mut rv);

        if !extra_args.is_null() {
            rv.push(crate::memory::cstr_to_cstring(extra_args));
        }

        if !cmd.is_null() {
            tokenize(opt_vars().p_shcf, &mut rv);
            rv.push(shell_xescape_xquote(cmd));
        }

        debug_assert!(!rv.is_empty());

        rv
    }
}

/// Joins shell arguments from `argv` into a fixed 256-byte string buffer.
pub fn shell_argv_to_str(argv: &[std::ffi::CString]) -> [c_char; 256] {
    let mut n: usize = 0;
    let maxsize: usize = 256;
    let mut rv: [c_char; 256] = [0; 256];
    let rv_ptr = rv.as_mut_ptr();
    if argv.is_empty() {
        return rv;
    }
    for arg in argv {
        {
            let _dst: *mut c_char = rv_ptr;
            let _dsize: usize = maxsize;
            let _dlen = crate::memory::cstr_len(_dst);
            crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), c"'", _dsize - _dlen);
        };
        {
            let _dst: *mut c_char = rv_ptr;
            let _dsize: usize = maxsize;
            let _dlen = crate::memory::cstr_len(_dst);
            crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), arg.as_ptr(), _dsize - _dlen);
        };
        n = {
            let _dst: *mut c_char = rv_ptr;
            let _dsize: usize = maxsize;
            let _dlen = crate::memory::cstr_len(_dst);
            crate::memory::xstrlcpy(_dst.wrapping_add(_dlen), c"' ", _dsize - _dlen) + _dlen
        };
        if n >= maxsize {
            break;
        }
    }
    if n < maxsize {
        rv[n - 1] = NUL;
    } else {
        rv[maxsize - 4] = b'.' as c_char;
        rv[maxsize - 3] = b'.' as c_char;
        rv[maxsize - 2] = b'.' as c_char;
        rv[maxsize - 1] = NUL;
    }
    rv
}

/// Calls the user-configured 'shell' (opt_vars().p_sh) for running a command or wildcard expansion.
pub fn os_call_shell(
    cmd: impl crate::strings::IntoCCharPtr,
    opts: i32,
    extra_args: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let cmd = cmd.into_c_char_ptr();
    let extra_args = extra_args.into_c_char_ptr();
    let mut input: Vec<c_char> = Vec::new();
    let mut output: Vec<c_char> = Vec::new();
    let mut want_output = false;
    let current_state = g_st().state_global;
    let mut forward_output = true;

    signal_reject_deadly();

    if (opts & (K_SHELL_OPT_HIDE_MESS | K_SHELL_OPT_EXPAND)) != 0 {
        forward_output = false;
    } else {
        g_st().state_global = MODE_EXTERNCMD;

        if (opts & K_SHELL_OPT_WRITE) != 0 {
            read_input(&mut input);
        }

        if (opts & K_SHELL_OPT_READ) != 0 {
            want_output = true;
            forward_output = false;
        } else if (opts & K_SHELL_OPT_DO_OUT) != 0 {
            forward_output = false;
        }
    }

    let exitcode = do_os_system(
        shell_build_argv(cmd, extra_args),
        if input.is_empty() {
            ptr::null()
        } else {
            input.as_ptr()
        },
        input.len(),
        want_output.then_some(&mut output),
        g_st().emsg_silent != 0,
        forward_output,
    );
    drop(input);

    if !output.is_empty() {
        let nread = output.len();
        write_output(output.as_mut_ptr(), nread, true);
    }

    if g_st().emsg_silent == 0 && exitcode != 0 && (opts & K_SHELL_OPT_SILENT) == 0 {
        msg_ext_set_kind(c"shell_ret");
        msg_ext_no_fast();
        if !ui_has(K_UIMESSAGES) {
            msg_putchar(b'\n' as i32);
        }
        msg_puts(gettext(c"shell returned "));
        msg_outnum(exitcode);
    }

    g_st().state_global = current_state;
    signal_accept_deadly();

    exitcode
}

/// os_call_shell() wrapper. Handles 'verbose', :profile, and v:shell_error.
pub fn call_shell(
    cmd: impl crate::strings::IntoCCharPtr,
    opts: i32,
    extra_shell_arg: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let cmd = cmd.into_c_char_ptr();
    let extra_shell_arg = extra_shell_arg.into_c_char_ptr();
    {
        let mut wait_time: Proftime = 0;

        if opt_vars().p_verbose > 3 {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Executing command: \"%s\""),
                if cmd.is_null() { opt_vars().p_sh } else { cmd },
            );
            if !ui_has(K_UIMESSAGES) {
                msg_putchar(b'\n' as i32);
            }
            verbose_leave();
        }

        if g_st().do_profiling == PROF_YES {
            prof_child_enter(&mut wait_time);
        }

        let retval = if crate::memory::cstr_is_empty(opt_vars().p_sh) {
            emsg(gettext(E_SHELLEMPTY));
            -1
        } else {
            tag_freematch();
            os_call_shell(cmd, opts, extra_shell_arg)
        };

        set_vim_var_nr(VV_SHELL_ERROR, Varnumber::from(retval));
        if g_st().do_profiling == PROF_YES {
            prof_child_exit(&mut wait_time);
        }

        retval
    }
}

/// Get the stdout of an external command.
pub fn get_cmd_output(
    cmd: impl crate::strings::IntoCCharPtr,
    infile: impl crate::strings::IntoCCharPtr,
    flags: i32,
) -> Option<std::ffi::CString> {
    let infile = infile.into_c_char_ptr();
    let cmd = cmd.into_c_char_ptr().cast_mut();
    {
        if check_secure() {
            return None;
        }

        let Some(tempname_cstr) = vim_tempname() else {
            emsg(gettext(E_NOTMP));
            return None;
        };
        let tempname = tempname_cstr.as_ptr().cast_mut();

        let command = make_filter_cmd(cmd, infile, tempname, false);

        g_st().no_check_timestamps += 1;
        call_shell(
            command.as_ptr().cast_mut(),
            K_SHELL_OPT_DO_OUT | K_SHELL_OPT_EXPAND | flags,
            ptr::null_mut(),
        );
        g_st().no_check_timestamps -= 1;

        drop(command);

        let fd = os_fopen(tempname, c"rb");
        let mut result: Option<std::ffi::CString> = None;

        'done: {
            if fd.is_null() || unsafe { vim_fseek(fd, 0, SEEK_END) } == -1 {
                semsg!(gettext(E_CANNOT_READ_FROM_STR_2), tempname);
                if !fd.is_null() {
                    unsafe { fclose(fd) };
                }
                break 'done;
            }
            let len_l = unsafe { vim_ftell(fd) };
            if len_l == -1 || unsafe { vim_fseek(fd, 0, SEEK_SET) } == -1 {
                semsg!(gettext(E_CANNOT_READ_FROM_STR_2), tempname);
                unsafe { fclose(fd) };
                break 'done;
            }

            let len = len_l as usize;
            let mut buffer = vec![0u8; len];
            let i = unsafe { fread(buffer.as_mut_ptr().cast(), 1, len, fd) };
            unsafe { fclose(fd) };
            if i != len {
                semsg!(gettext(E_CANT_READ_FILE_STR), tempname);
            } else {
                for b in &mut buffer {
                    if *b == 0 {
                        *b = 1;
                    }
                }
                result = Some(unsafe { std::ffi::CString::from_vec_unchecked(buffer) });
            }
        }

        os_remove(tempname);
        drop(tempname_cstr);
        result
    }
}

/// os_system - synchronously execute a command in the shell
pub fn os_system(
    argv: Vec<std::ffi::CString>,
    input: impl crate::strings::IntoCCharPtr,
    len: usize,
    output: Option<&mut Vec<c_char>>,
) -> i32 {
    let input = input.into_c_char_ptr();
    do_os_system(argv, input, len, output, true, false)
}

fn do_os_system(
    argv: Vec<std::ffi::CString>,
    input: impl crate::strings::IntoCCharPtr,
    len: usize,
    output: Option<&mut Vec<c_char>>,
    silent: bool,
    forward_output: bool,
) -> i32 {
    let input = input.into_c_char_ptr();
    {
        let mut exitcode: i32 = -1;

        #[cfg(windows)]
        let (oldval, old_output_cp, old_input_cp) = {
            let ov = os_getenv_cstr(c"NoDefaultCurrentDirectoryInExePath");
            os_setenv(c"NoDefaultCurrentDirectoryInExePath", c"1", 1);
            let oocp = unsafe { GetConsoleOutputCP() };
            let oicp = unsafe { GetConsoleCP() };
            unsafe { SetConsoleOutputCP(CP_UTF8) };
            unsafe { SetConsoleCP(CP_UTF8) };
            (ov, oocp, oicp)
        };

        out_data_decide_throttle(0);
        out_data_ring(ptr::null(), 0);
        let has_input = !input.is_null() && len > 0;

        let mut buf: Vec<c_char> = Vec::new();
        let cb_data = if forward_output {
            StreamCbData::ShellOut
        } else if output.is_none() {
            StreamCbData::None
        } else {
            StreamCbData::ShellBuf(&raw mut buf)
        };

        let mut prog = [0 as c_char; MAXPATHL];
        xstrlcpy(prog.as_mut_ptr(), argv[0].as_ptr(), MAXPATHL);

        let mloop = main_loop_ptr();
        let mut uvproc = LibuvProc::new();
        let proc_ = &mut uvproc.proc_;
        proc_.r#type = K_PROC_TYPE_UV;
        proc_.loop_ = mloop;
        proc_.status = -1;
        proc_.out.s.fd = STDOUT_FILENO;
        proc_.err.s.fd = STDERR_FILENO;

        let qc = loop_queues(&mut *mloop);
        let events = multiqueue_new_child(qc.events);
        proc_.events = events;
        proc_.argv = argv;

        'end: {
            let status = proc_spawn(proc_, has_input, true, true);
            if status != 0 {
                loop_poll_events(&mut *mloop, 0);
                if !silent {
                    msg_puts(gettext(c"\nshell failed to start: "));
                    msg_outtrans(unsafe { uv_strerror(status) }, 0, false);
                    msg_puts(c": ");
                    msg_outtrans(prog.as_ptr(), 0, false);
                    msg_putchar(b'\n' as i32);
                }
                break 'end;
            }

            let s_in = &mut proc_.r#in;
            let r_out = &mut proc_.out;
            let r_err = &mut proc_.err;
            if has_input {
                wstream_init(s_in, 0);
            }
            rstream_init(r_out);
            rstream_start(r_out, cb_data);
            rstream_init(r_err);
            rstream_start(r_err, cb_data);

            if has_input {
                let input_buffer = wstream_new_vec_buffer(
                    unsafe { std::slice::from_raw_parts(input, len) }.to_vec(),
                );
                if wstream_write(s_in, input_buffer) != 0 {
                    proc_stop(proc_);
                    break 'end;
                }
                wstream_set_write_cb(s_in);
            }

            ui_busy_start();
            ui_flush();
            if forward_output {
                msg_sb_eol();
                msg_start();
                g_st().msg_no_more = true;
                g_st().lines_left = -1;
            }
            exitcode = proc_wait(proc_, -1, ptr::null_mut());
            if !g_st().got_int && out_data_decide_throttle(0) {
                out_data_ring(ptr::null(), usize::MAX);
            }
            if forward_output {
                g_st().no_wait_return += 1;
                msg_end();
                g_st().no_wait_return -= 1;
                g_st().msg_no_more = false;
            }

            ui_busy_stop();

            if let Some(out) = output {
                *out = buf;
            }

            debug_assert!(multiqueue_empty(&*events));
        }

        multiqueue_free(events);

        #[cfg(windows)]
        {
            restore_env_var(c"NoDefaultCurrentDirectoryInExePath", oldval);
            unsafe { SetConsoleOutputCP(old_output_cp) };
            unsafe { SetConsoleCP(old_input_cp) };
        }

        exitcode
    }
}

pub(crate) fn system_data_cb(
    _stream: &mut RStream,
    buf: impl crate::strings::IntoCCharPtr,
    count: usize,
    dbuf: &mut Vec<c_char>,
    _eof: bool,
) -> usize {
    let buf = buf.into_c_char_ptr();
    {
        if !buf.is_null() && count > 0 {
            dbuf.extend_from_slice(unsafe { std::slice::from_raw_parts(buf, count) });
        }
        count
    }
}

fn out_data_decide_throttle(size: usize) -> bool {
    if size == 0 {
        let previous_decision = sh_st().throttle_visit > 0;
        sh_st().throttle_started = 0;
        sh_st().throttle_received = 0;
        sh_st().throttle_visit = 0;
        return previous_decision;
    }

    sh_st().throttle_received += size;
    if sh_st().throttle_received < OUT_DATA_THRESHOLD
        || (sh_st().throttle_started == 0 && sh_st().throttle_received < size + 1000)
    {
        return false;
    } else if sh_st().throttle_visit == 0 {
        sh_st().throttle_started = os_hrtime();
    } else {
        let since = os_hrtime() - sh_st().throttle_started;
        if since < (sh_st().throttle_visit as u64) * (NS_PER_SEC / 10) {
            return true;
        }
        if since > 3 * NS_PER_SEC {
            sh_st().throttle_received = 0;
            sh_st().throttle_visit = 0;
            return false;
        }
    }

    sh_st().throttle_visit += 1;
    let tick = sh_st().throttle_visit % 4;
    sh_st().throttle_pulse_msg[0] = if tick > 0 {
        b'.' as c_char
    } else {
        b' ' as c_char
    };
    sh_st().throttle_pulse_msg[1] = if tick > 1 {
        b'.' as c_char
    } else {
        b' ' as c_char
    };
    sh_st().throttle_pulse_msg[2] = if tick > 2 {
        b'.' as c_char
    } else {
        b' ' as c_char
    };
    if sh_st().throttle_visit == 1 {
        msg_puts(c"...\n");
    }
    msg_putchar(b'\r' as i32);
    msg_puts((&raw const sh_st().throttle_pulse_msg).cast());
    msg_putchar(b'\r' as i32);
    ui_flush();
    true
}

fn out_data_ring(output: impl crate::strings::IntoCCharPtr, size: usize) {
    let output = output.into_c_char_ptr();
    {
        debug_assert!(!output.is_null() || size == 0 || size == usize::MAX);

        if output.is_null() && size == 0 {
            sh_st().last_skipped_len = 0;
            return;
        }

        let ring_ptr = (&raw mut sh_st().last_skipped).cast::<c_char>();
        if output.is_null() && size == usize::MAX {
            out_data_append_to_screen(ring_ptr, &mut sh_st().last_skipped_len, STDOUT_FILENO, true);
            return;
        }

        if size >= MAX_CHUNK_SIZE {
            let start = size - MAX_CHUNK_SIZE;
            unsafe {
                ptr::copy_nonoverlapping(output.wrapping_add(start), ring_ptr, MAX_CHUNK_SIZE)
            };
            sh_st().last_skipped_len = MAX_CHUNK_SIZE;
        } else if size > 0 {
            let keep_len = sh_st().last_skipped_len.min(MAX_CHUNK_SIZE - size);
            let keep_start = sh_st().last_skipped_len - keep_len;
            if keep_start != 0 {
                unsafe { ptr::copy(ring_ptr.wrapping_add(keep_start), ring_ptr, keep_len) };
            }
            unsafe { ptr::copy_nonoverlapping(output, ring_ptr.wrapping_add(keep_len), size) };
            sh_st().last_skipped_len = keep_len + size;
        }
    }
}

pub fn out_data_event(mut str_copy: Vec<c_char>, fd: i32) {
    let mut need_clear = true;
    let hl = if fd == STDERR_FILENO { HLF_SE } else { HLF_SO };
    msg_ext_set_kind(if fd == STDERR_FILENO {
        c"shell_err".as_ptr()
    } else {
        c"shell_out".as_ptr()
    });
    msg_ext_set_append(true);
    msg_ext_no_fast();
    let str_arg = NvimString {
        data: str_copy.as_mut_ptr(),
        size: str_copy.len(),
    };
    msg_multiline(str_arg, hl, false, false, &mut need_clear);
    drop(str_copy);
    ui_flush();
}

fn out_data_append_to_screen(
    output: impl crate::strings::IntoCCharPtr,
    count: &mut usize,
    fd: i32,
    eof: bool,
) {
    let output = output.into_c_char_ptr();
    {
        let mut p = output;
        let end = output.wrapping_add(*count);
        while (p as usize) < (end as usize) {
            let off = (p as usize).wrapping_sub(output as usize);
            let i = if !crate::memory::cstr_is_empty(p) {
                utfc_ptr2len_len(p, (*count - off) as i32)
            } else {
                1
            };
            if !eof
                && i == 1
                && usize::from(UTF8LEN_TAB_ZERO[crate::memory::cstr_first_byte(p) as usize])
                    > (end as usize).wrapping_sub(p as usize)
            {
                *count = off;
                break;
            }
            p = p.wrapping_add(i as usize);
        }

        let str_copy = unsafe { std::slice::from_raw_parts(output, *count) }.to_vec();
        if ui_has(K_UIMESSAGES) {
            let qc = loop_queues(&mut *main_loop_ptr());
            multiqueue_put_event(
                unsafe { &mut *qc.fast_events },
                Event::ShellOutData { str_copy, fd },
            );
        } else {
            out_data_event(str_copy, fd);
        }
    }
}

pub(crate) fn out_data_cb(
    stream: &mut RStream,
    ptr_data: impl crate::strings::IntoCCharPtr,
    mut count: usize,
    eof: bool,
) -> usize {
    let ptr_data = ptr_data.into_c_char_ptr();
    if count > 0 && out_data_decide_throttle(count) {
        out_data_ring(ptr_data, count);
    } else if count > 0 {
        let fd = stream.s.fd;
        out_data_append_to_screen(ptr_data, &mut count, fd, eof);
    }
    count
}

fn tokenize(str_in: impl crate::strings::IntoCCharPtr, argv: &mut Vec<std::ffi::CString>) {
    let str_in = str_in.into_c_char_ptr();
    {
        let mut p = str_in;

        while !crate::memory::cstr_is_empty(p) {
            let len = word_length(p);
            argv.push(vim_strnsave_unquoted(crate::memory::cstr_slice(p, len)));
            p = skipwhite(p.wrapping_add(len));
        }
    }
}

pub(crate) fn word_length(str_in: impl crate::strings::IntoCCharPtr) -> usize {
    let str_in = str_in.into_c_char_ptr();
    {
        let mut p = str_in;
        let mut inquote = false;
        let mut length: usize = 0;

        while !crate::memory::cstr_is_empty(p)
            && (inquote || (crate::memory::cstr_first_byte(p) != b' ' && unsafe { *p } != TAB))
        {
            if crate::memory::cstr_first_byte(p) == b'"' {
                inquote = !inquote;
            } else if crate::memory::cstr_first_byte(p) == b'\\' && inquote {
                p = p.wrapping_add(1);
                length += 1;
            }

            p = p.wrapping_add(1);
            length += 1;
        }

        length
    }
}

fn read_input(buf: &mut Vec<c_char>) {
    read_buffer_into(
        Some(crate::globals::curbuf_mut()),
        curbuf_mut().b_op_start.lnum,
        curbuf_mut().b_op_end.lnum,
        buf,
    );
}

fn write_output(mut output: *mut c_char, mut remaining: usize, eof: bool) -> usize {
    if output.is_null() {
        return 0;
    }
    {
        let start = output;
        let mut off: usize = 0;
        while off < remaining {
            if unsafe { *output.wrapping_add(off) } == CAR
                && off + 1 < remaining
                && unsafe { *output.wrapping_add(off + 1) } == NL
                && curbuf_mut().b_p_bin == 0
            {
                unsafe {
                    *output.wrapping_add(off) = NUL;
                }
                let lnum = curwin_mut().w_cursor.lnum;
                curwin_mut().w_cursor.lnum += 1;
                ml_append(lnum, output, off as i32 + 1, false);
                let skip = off + 2;
                output = output.wrapping_add(skip);
                remaining -= skip;
                off = 0;
                continue;
            } else if (unsafe { *output.wrapping_add(off) } == CAR && curbuf_mut().b_p_bin == 0)
                || unsafe { *output.wrapping_add(off) } == NL
            {
                unsafe {
                    *output.wrapping_add(off) = NUL;
                }
                let lnum = curwin_mut().w_cursor.lnum;
                curwin_mut().w_cursor.lnum += 1;
                ml_append(lnum, output, off as i32 + 1, false);
                let skip = off + 1;
                output = output.wrapping_add(skip);
                remaining -= skip;
                off = 0;
                continue;
            }

            if crate::memory::cstr_byte_at(output, off) == 0 {
                unsafe {
                    *output.wrapping_add(off) = NL;
                }
            }
            off += 1;
        }

        if eof {
            if remaining != 0 {
                let lnum = curwin_mut().w_cursor.lnum;
                curwin_mut().w_cursor.lnum += 1;
                ml_append(lnum, output, 0, false);
                curbuf_mut().b_no_eol_lnum = curwin_mut().w_cursor.lnum;
                output = output.wrapping_add(remaining);
            } else {
                curbuf_mut().b_no_eol_lnum = 0;
            }
        }

        ui_flush();

        (output as usize).wrapping_sub(start as usize)
    }
}

pub(crate) fn shell_write_cb(stream: &mut Stream, status: i32) {
    if status != 0 {
        msg_schedule_semsg!(
            gettext(c"E5677: Error writing input to shell-command: %s"),
            unsafe { uv_err_name(status) },
        );
    }
    stream_may_close(stream);
}

fn shell_xescape_xquote(cmd: impl crate::strings::IntoCCharPtr) -> std::ffi::CString {
    let cmd = cmd.into_c_char_ptr();
    {
        if crate::memory::cstr_is_empty(opt_vars().p_sxq) {
            return crate::memory::cstr_to_cstring(cmd);
        }

        let sxq_bytes = crate::memory::cstr_as_bytes(opt_vars().p_sxq);
        let owned_ecmd = if !crate::memory::cstr_is_empty(opt_vars().p_sxe) && sxq_bytes == b"(" {
            Some(vim_strsave_escaped_ext(
                cmd,
                opt_vars().p_sxe,
                b'^' as c_char,
                false,
            ))
        } else {
            None
        };
        let ecmd_bytes = owned_ecmd
            .as_ref()
            .map_or_else(|| crate::memory::cstr_as_bytes(cmd), |s| s.as_bytes());
        let mut out = Vec::with_capacity(ecmd_bytes.len() + sxq_bytes.len() * 2);

        if sxq_bytes == b"(" {
            out.push(b'(');
            out.extend_from_slice(ecmd_bytes);
            out.push(b')');
        } else if sxq_bytes == b"\"(" {
            out.extend_from_slice(b"\"(");
            out.extend_from_slice(ecmd_bytes);
            out.extend_from_slice(b")\"");
        } else {
            out.extend_from_slice(sxq_bytes);
            out.extend_from_slice(ecmd_bytes);
            out.extend_from_slice(sxq_bytes);
        }

        crate::memory::cstr_from_bytes(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_length_handles_quotes_and_escaped_chars() {
        assert_eq!(word_length(c"foo bar"), 3);
        assert_eq!(word_length(c"\"foo bar\" baz"), 9);
        assert_eq!(word_length(c"/Program\" \"Files/zsh -f"), 20);
    }

    #[test]
    fn shell_build_argv_and_argv_to_str() {
        let save_sh = opt_vars().p_sh;
        let save_shcf = opt_vars().p_shcf;
        let save_sxq = opt_vars().p_sxq;
        let save_sxe = opt_vars().p_sxe;

        opt_vars().p_sh = c"/bin/sh".as_ptr().cast_mut();
        opt_vars().p_shcf = c"-c".as_ptr().cast_mut();
        opt_vars().p_sxq = c"".as_ptr().cast_mut();
        opt_vars().p_sxe = c"".as_ptr().cast_mut();

        let collect_argv = |cmd: *const c_char, extra: *const c_char| -> Vec<String> {
            shell_build_argv(cmd, extra)
                .into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect()
        };

        assert_eq!(collect_argv(ptr::null(), ptr::null()), vec!["/bin/sh"]);
        assert_eq!(
            collect_argv(c"abc  def".as_ptr(), ptr::null()),
            vec!["/bin/sh", "-c", "abc  def"]
        );
        assert_eq!(
            collect_argv(ptr::null(), c"ghi  jkl".as_ptr()),
            vec!["/bin/sh", "ghi  jkl"]
        );
        assert_eq!(
            collect_argv(c"abc  def".as_ptr(), c"ghi  jkl".as_ptr()),
            vec!["/bin/sh", "ghi  jkl", "-c", "abc  def"]
        );

        opt_vars().p_sh = c"/Program\" \"Files/zsh -f".as_ptr().cast_mut();
        opt_vars().p_shcf = c"-x -o \"sh word split\" \"-\"c".as_ptr().cast_mut();
        assert_eq!(
            collect_argv(c"abc  def".as_ptr(), c"ghi  jkl".as_ptr()),
            vec![
                "/Program Files/zsh",
                "-f",
                "ghi  jkl",
                "-x",
                "-o",
                "sh word split",
                "-c",
                "abc  def"
            ]
        );

        opt_vars().p_sh = c"/bin/sh".as_ptr().cast_mut();
        opt_vars().p_shcf = c"-c".as_ptr().cast_mut();
        opt_vars().p_sxq = c"(".as_ptr().cast_mut();
        opt_vars().p_sxe = c"\"&|<>()@^".as_ptr().cast_mut();
        assert_eq!(
            collect_argv(c"echo &|<>()@^".as_ptr(), ptr::null()),
            vec!["/bin/sh", "-c", "(echo ^&^|^<^>^(^)^@^^)"]
        );

        opt_vars().p_sxq = c"\"(".as_ptr().cast_mut();
        assert_eq!(
            collect_argv(c"echo -n some text".as_ptr(), ptr::null()),
            vec!["/bin/sh", "-c", "\"(echo -n some text)\""]
        );

        opt_vars().p_sxq = c"\"".as_ptr().cast_mut();
        opt_vars().p_sxe = c"".as_ptr().cast_mut();
        assert_eq!(
            collect_argv(c"echo -n some text".as_ptr(), ptr::null()),
            vec!["/bin/sh", "-c", "\"echo -n some text\""]
        );

        let argv_arr = [c"foo".to_owned(), c"".to_owned(), c"bar".to_owned()];
        let s = shell_argv_to_str(&argv_arr);
        assert_eq!(
            crate::memory::cstr_from_ptr(s.as_ptr()).to_str().unwrap(),
            "'foo' '' 'bar'"
        );

        opt_vars().p_sh = save_sh;
        opt_vars().p_shcf = save_shcf;
        opt_vars().p_sxq = save_sxq;
        opt_vars().p_sxe = save_sxe;
    }
}
