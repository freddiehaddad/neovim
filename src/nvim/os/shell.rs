#![forbid(unsafe_code)]
//! Shell execution and wildcard expansion (`src/nvim/os/shell.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::{K_UIMESSAGES, read_buffer_into};
use crate::charset::NL;
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
use crate::event::rstream::{rstream_init, rstream_start};
use crate::event::stream::{Stream, StreamCbData, stream_may_close};
use crate::event::wstream::{
    wstream_init, wstream_new_vec_buffer, wstream_set_write_cb, wstream_write,
};
use crate::ex_cmds::{K_SHELL_OPT_READ, K_SHELL_OPT_WRITE, check_secure, make_filter_cmd};
use crate::ex_docmd::CAR;
use crate::fileio::{SEEK_END, SEEK_SET, vim_tempname};
use crate::globals::*;
#[cfg(windows)]
use crate::mbyte::CP_UTF8;
use crate::mbyte::UTF8LEN_TAB_ZERO;
use crate::memline::ml_append;
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
use crate::os::fs::{CFile, MAXPATHL, os_remove};
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
use std::ffi::CStr;

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
    throttle_pulse_msg: [u8; 4],
    last_skipped: [u8; MAX_CHUNK_SIZE],
    last_skipped_len: usize,
    system_buf: Vec<u8>,
}

impl ShellModuleState {
    const fn new() -> Self {
        Self {
            #[cfg(unix)]
            did_find_nul: false,
            throttle_started: 0,
            throttle_received: 0,
            throttle_visit: 0,
            throttle_pulse_msg: [b' ', b' ', b' ', 0],
            last_skipped: [0; MAX_CHUNK_SIZE],
            last_skipped_len: 0,
            system_buf: Vec::new(),
        }
    }
}

std::thread_local! {
    static SH_STATE: std::cell::RefCell<ShellModuleState> =
        const { std::cell::RefCell::new(ShellModuleState::new()) };
}

#[cfg(windows)]
use crate::os::os_win_console::{
    os_get_console_cp, os_get_console_output_cp, os_set_console_cp, os_set_console_output_cp,
};

#[cfg(unix)]
fn save_patterns(pat: &[&CStr], file: &mut Vec<std::ffi::CString>) {
    file.clear();
    file.reserve(pat.len());
    for &p in pat {
        let mut v: Vec<u8> = p.to_bytes_with_nul().to_vec();
        backslash_halve(&mut v);
        file.push(crate::memory::cstring_until_nul(&v));
    }
}

#[cfg(unix)]
fn have_wildcard(file: &[&CStr]) -> bool {
    file.iter().any(|p| path_has_wildcard(p.to_bytes(), true))
}

#[cfg(unix)]
fn have_dollars(file: &[&CStr]) -> bool {
    file.iter().any(|p| p.to_bytes().contains(&b'$'))
}

/// Performs wildcard pattern matching using the shell.
#[cfg(unix)]
pub fn os_expand_wildcards(pat: &[&CStr], file: &mut Vec<std::ffi::CString>, flags: i32) -> bool {
    const SH_VIMGLOB_FUNC: &CStr =
        c"vimglob() { while [ $# -ge 1 ]; do printf '%s\\n' \"$1\"; shift; done }; vimglob >";
    const SH_GLOBSTAR_OPT: &CStr = c"[[ ${BASH_VERSINFO[0]} -ge 4 ]] && shopt -s globstar; ";

    let mut extra_shell_arg: Option<&CStr> = None;
    let mut shellopts: i32 = K_SHELL_OPT_EXPAND | K_SHELL_OPT_SILENT;
    let mut shell_style: i32 = STYLE_ECHO;
    let mut ampersand = false;

    #[cfg(unix)]
    let is_fish_shell = {
        let p_sh = opt_vars().p_sh.to_owned();
        let tail = invocation_path_tail(&p_sh, None);
        tail.to_bytes().starts_with(b"fish")
    };
    #[cfg(not(unix))]
    let is_fish_shell = false;

    file.clear();

    if !have_wildcard(pat) {
        save_patterns(pat, file);
        return true;
    }

    if g_st().sandbox != 0 && check_secure() {
        return false;
    }

    if g_st().secure != 0 {
        for &p in pat {
            if p.to_bytes().contains(&b'`') && check_secure() {
                return false;
            }
        }
    }

    let Some(tempname_cstr) = vim_tempname() else {
        emsg(gettext(E_NOTMP));
        return false;
    };

    if pat.len() == 1 && pat[0].to_bytes().first() == Some(&b'`') {
        let pat0_bytes = pat[0].to_bytes();
        if pat0_bytes.len() > 2 && pat0_bytes.last() == Some(&b'`') {
            shell_style = STYLE_BT;
        }
    }
    if shell_style != STYLE_BT {
        let p_sh = opt_vars().p_sh.to_owned();
        let sh_bytes = p_sh.as_bytes();
        if sh_bytes.ends_with(b"csh") {
            shell_style = STYLE_GLOB;
        } else if sh_bytes.ends_with(b"zsh") {
            shell_style = STYLE_PRINT;
        }
    }
    if shell_style == STYLE_ECHO {
        let p_sh = opt_vars().p_sh.to_owned();
        let sh_tail = path_tail(&p_sh).to_bytes();
        if sh_tail.windows(4).any(|w| w == b"bash") {
            shell_style = STYLE_GLOBSTAR;
        } else if sh_tail.windows(2).any(|w| w == b"sh") {
            shell_style = STYLE_VIMGLOB;
        }
    }

    let mut len = tempname_cstr.as_bytes().len() + 29;
    if shell_style == STYLE_VIMGLOB {
        len += SH_VIMGLOB_FUNC.to_bytes().len();
    } else if shell_style == STYLE_GLOBSTAR {
        len += SH_VIMGLOB_FUNC.to_bytes().len() + SH_GLOBSTAR_OPT.to_bytes().len();
    }

    for &p in pat {
        len += 1;
        for &b in p.to_bytes() {
            if crate::strings::vim_haschr(SHELL_SPECIAL, i32::from(b)) {
                len += 1;
            }
            len += 1;
        }
    }

    if is_fish_shell {
        len += b"egin; end".len();
    }

    let mut command_vec = Vec::with_capacity(len);

    if shell_style == STYLE_BT {
        if is_fish_shell {
            command_vec.extend_from_slice(b"begin; ");
        } else {
            command_vec.push(b'(');
        }
        command_vec.extend_from_slice(&pat[0].to_bytes()[1..]);
        if let Some(last) = command_vec.last_mut() {
            if is_fish_shell {
                *last = b';';
            } else {
                *last = b')';
            }
        }
        let mut idx = command_vec.len().saturating_sub(2);
        if is_fish_shell {
            command_vec.extend_from_slice(b" end");
        }
        while idx > 0 && ascii_iswhite(i32::from(command_vec[idx])) {
            idx -= 1;
        }
        if command_vec.get(idx) == Some(&b'&') {
            ampersand = true;
            command_vec[idx] = b' ';
        }
        command_vec.push(b'>');
    } else {
        if shell_style == STYLE_GLOB {
            if (flags & EW_NOTFOUND) != 0 {
                command_vec.extend_from_slice(b"set nonomatch; ");
            } else {
                command_vec.extend_from_slice(b"unset nonomatch; ");
            }
        }
        if shell_style == STYLE_GLOB {
            command_vec.extend_from_slice(b"glob >");
        } else if shell_style == STYLE_PRINT {
            command_vec.extend_from_slice(b"print -N >");
        } else if shell_style == STYLE_VIMGLOB {
            command_vec.extend_from_slice(SH_VIMGLOB_FUNC.to_bytes());
        } else if shell_style == STYLE_GLOBSTAR {
            command_vec.extend_from_slice(SH_GLOBSTAR_OPT.to_bytes());
            command_vec.extend_from_slice(SH_VIMGLOB_FUNC.to_bytes());
        } else {
            command_vec.extend_from_slice(b"echo >");
        }
    }

    command_vec.extend_from_slice(tempname_cstr.as_bytes());

    if shell_style != STYLE_BT {
        for &p in pat {
            let mut intick = false;
            let p_i = p.to_bytes();

            command_vec.push(b' ');
            let mut j = 0;
            while j < p_i.len() {
                let ch = p_i[j];
                if ch == b'`' {
                    intick = !intick;
                } else if ch == b'\\' && j + 1 < p_i.len() {
                    let next_ch = p_i[j + 1];
                    if intick
                        || crate::strings::vim_haschr(SHELL_SPECIAL, i32::from(next_ch))
                        || next_ch == b'`'
                    {
                        command_vec.push(b'\\');
                    }
                    j += 1;
                } else if !intick
                    && ((flags & EW_KEEPDOLLAR) == 0 || ch != b'$')
                    && crate::strings::vim_haschr(SHELL_SPECIAL, i32::from(ch))
                {
                    command_vec.push(b'\\');
                }

                command_vec.push(p_i[j]);
                j += 1;
            }
        }
    }

    if (flags & EW_SILENT) != 0 {
        shellopts |= K_SHELL_OPT_HIDE_MESS;
    }

    if ampersand {
        command_vec.push(b'&');
    }
    command_vec.push(0);

    if shell_style == STYLE_PRINT {
        extra_shell_arg = Some(c"-G");
    } else if shell_style == STYLE_GLOB && !have_dollars(pat) {
        extra_shell_arg = Some(c"-f");
    }

    let res = call_shell(
        Some(crate::memory::bytes_as_cstr(&command_vec)),
        shellopts,
        extra_shell_arg,
    );

    if ampersand {
        os_delay(10, true);
    }

    drop(command_vec);

    if res != 0 {
        os_remove(tempname_cstr.as_c_str());
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
            save_patterns(pat, file);
            return true;
        }
        return false;
    }

    let Some(f) = CFile::open_libc(tempname_cstr.as_c_str(), c"rb") else {
        if (flags & EW_SILENT) == 0 {
            msg(gettext(E_WILDEXPAND), 0);
            msg_start();
        }
        if (flags & EW_NOTFOUND) != 0 {
            save_patterns(pat, file);
            return true;
        }
        return false;
    };
    if f.seek(0, SEEK_END) < 0 {
        return false;
    }
    let templen = f.tell();
    if templen < 0 {
        return false;
    }
    len = templen as usize;
    f.seek(0, SEEK_SET);
    let mut buffer_vec = vec![0u8; len + 1];
    let readlen = f.read_bytes(&mut buffer_vec[..len]);
    drop(f);
    os_remove(tempname_cstr.as_c_str());
    if readlen != len {
        semsg!(gettext(E_CANT_READ_FILE_STR), tempname_cstr.as_c_str());
        return false;
    }
    drop(tempname_cstr);

    let mut count: i32 = 0;
    if shell_style == STYLE_ECHO {
        buffer_vec[len] = b'\n';
        let mut p_idx = 0usize;
        while buffer_vec[p_idx] != b'\n' {
            while buffer_vec[p_idx] != b' ' && buffer_vec[p_idx] != b'\n' {
                p_idx += 1;
            }
            while ascii_iswhite(buffer_vec[p_idx]) {
                p_idx += 1;
            }
            count += 1;
        }
    } else if shell_style == STYLE_BT
        || shell_style == STYLE_VIMGLOB
        || shell_style == STYLE_GLOBSTAR
    {
        buffer_vec[len] = 0;
        let mut p_idx = 0usize;
        while buffer_vec[p_idx] != 0 {
            while buffer_vec[p_idx] != b'\n' && buffer_vec[p_idx] != 0 {
                p_idx += 1;
            }
            if buffer_vec[p_idx] != 0 {
                p_idx += 1;
            }
            while ascii_iswhite(buffer_vec[p_idx]) {
                p_idx += 1;
            }
            count += 1;
        }
    } else {
        let mut check_spaces = false;
        let did_find_nul = SH_STATE.with_borrow(|st| st.did_find_nul);
        if shell_style == STYLE_PRINT && !did_find_nul {
            buffer_vec[len] = 0;
            if len != 0 && (crate::memory::bytes_until_nul(&buffer_vec).len() as i32) < (len as i32)
            {
                SH_STATE.with_borrow_mut(|st| st.did_find_nul = true);
            } else {
                check_spaces = true;
            }
        }

        if len != 0 && buffer_vec[len - 1] == 0 {
            len -= 1;
        } else {
            buffer_vec[len] = 0;
        }
        let mut p_idx = 0usize;
        while p_idx < len {
            if buffer_vec[p_idx] == 0 || (buffer_vec[p_idx] == b' ' && check_spaces) {
                count += 1;
                buffer_vec[p_idx] = 0;
            }
            p_idx += 1;
        }
        if len != 0 {
            count += 1;
        }
    }
    debug_assert!(buffer_vec[len] == 0 || buffer_vec[len] == b'\n');

    if count == 0 {
        if (flags & EW_NOTFOUND) != 0 {
            save_patterns(pat, file);
            return true;
        }
        return false;
    }
    let mut raw_entries: Vec<usize> = Vec::with_capacity(count as usize);

    let mut p_idx = 0usize;
    for _ in 0..(count as usize) {
        raw_entries.push(p_idx);
        if shell_style == STYLE_ECHO
            || shell_style == STYLE_BT
            || shell_style == STYLE_VIMGLOB
            || shell_style == STYLE_GLOBSTAR
        {
            while !(shell_style == STYLE_ECHO && buffer_vec[p_idx] == b' ')
                && buffer_vec[p_idx] != b'\n'
                && buffer_vec[p_idx] != 0
            {
                p_idx += 1;
            }
            if p_idx == len {
                buffer_vec[p_idx] = 0;
            } else {
                buffer_vec[p_idx] = 0;
                p_idx += 1;
                while ascii_iswhite(buffer_vec[p_idx]) {
                    p_idx += 1;
                }
            }
        } else {
            while buffer_vec[p_idx] != 0 && p_idx < len {
                p_idx += 1;
            }
            p_idx += 1;
        }
    }

    for entry_idx in raw_entries {
        let entry_cstr = crate::memory::bytes_as_cstr(&buffer_vec[entry_idx..]);
        if (flags & EW_NOTFOUND) == 0 && !os_path_exists(entry_cstr) {
            continue;
        }

        let dir = os_isdir(entry_cstr);
        if (dir && (flags & EW_DIR) == 0) || (!dir && (flags & EW_FILE) == 0) {
            continue;
        }

        if !dir
            && (flags & EW_EXEC) != 0
            && !os_can_exe(entry_cstr, None, (flags & EW_SHELLCMD) == 0)
        {
            continue;
        }

        let entry_bytes = entry_cstr.to_bytes_with_nul();
        let mut new_vec = vec![0u8; entry_bytes.len() + usize::from(dir)];
        new_vec[..entry_bytes.len()].copy_from_slice(entry_bytes);
        if dir {
            add_pathsep(&mut new_vec);
        }
        file.push(crate::memory::cstring_until_nul(&new_vec));
    }
    drop(buffer_vec);

    if file.is_empty() {
        if (flags & EW_NOTFOUND) != 0 {
            save_patterns(pat, file);
            return true;
        }
        return false;
    }

    true
}

/// Builds the argument vector for running the user-configured 'shell' (opt_vars().p_sh)
/// with an optional command prefixed by 'shellcmdflag' (opt_vars().p_shcf).
pub fn shell_build_argv(cmd: Option<&CStr>, extra_args: Option<&CStr>) -> Vec<std::ffi::CString> {
    let mut rv: Vec<std::ffi::CString> = Vec::new();
    tokenize(Some(opt_vars().p_sh.as_cstr()), &mut rv);

    if let Some(extra_args) = extra_args {
        rv.push(extra_args.to_owned());
    }

    if let Some(cmd) = cmd {
        tokenize(Some(opt_vars().p_shcf.as_cstr()), &mut rv);
        rv.push(shell_xescape_xquote(cmd));
    }

    debug_assert!(!rv.is_empty());

    rv
}

/// Joins shell arguments from `argv` into a `CString` (truncated at 255 bytes with `"..."`).
pub fn shell_argv_to_str(argv: &[std::ffi::CString]) -> std::ffi::CString {
    const MAXSIZE: usize = 256;
    if argv.is_empty() {
        return std::ffi::CString::default();
    }
    let mut out = Vec::with_capacity(MAXSIZE);
    let mut overflow = false;
    let push_bytes = |out: &mut Vec<u8>, bytes: &[u8], overflow: &mut bool| {
        let avail = (MAXSIZE - 1).saturating_sub(out.len());
        if bytes.len() > avail {
            out.extend_from_slice(&bytes[..avail]);
            *overflow = true;
        } else {
            out.extend_from_slice(bytes);
        }
    };
    for arg in argv {
        push_bytes(&mut out, b"'", &mut overflow);
        push_bytes(&mut out, arg.as_bytes(), &mut overflow);
        push_bytes(&mut out, b"' ", &mut overflow);
        if overflow {
            break;
        }
    }
    if !overflow {
        out.pop();
    } else if out.len() >= MAXSIZE - 4 {
        out.truncate(MAXSIZE - 4);
        out.extend_from_slice(b"...");
    }
    std::ffi::CString::new(out).unwrap_or_default()
}

/// Calls the user-configured 'shell' (opt_vars().p_sh) for running a command or wildcard expansion.
pub fn os_call_shell(cmd: Option<&CStr>, opts: i32, extra_args: Option<&CStr>) -> i32 {
    let mut input: Vec<u8> = Vec::new();
    let mut output: Vec<u8> = Vec::new();
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
        &input,
        want_output.then_some(&mut output),
        g_st().emsg_silent != 0,
        forward_output,
    );
    drop(input);

    if !output.is_empty() {
        write_output(&mut output, true);
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
pub fn call_shell(cmd: Option<&CStr>, opts: i32, extra_shell_arg: Option<&CStr>) -> i32 {
    let mut wait_time: Proftime = 0;

    if opt_vars().p_verbose.get() > 3 {
        verbose_enter();
        let p_sh = opt_vars().p_sh.to_owned();
        smsg!(
            0,
            gettext(c"Executing command: \"%s\""),
            cmd.unwrap_or(&p_sh),
        );
        if !ui_has(K_UIMESSAGES) {
            msg_putchar(b'\n' as i32);
        }
        verbose_leave();
    }

    if g_st().do_profiling == PROF_YES {
        prof_child_enter(&mut wait_time);
    }

    let retval = if opt_vars().p_sh.is_empty() {
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

/// Get the stdout of an external command.
pub fn get_cmd_output(
    cmd: Option<&CStr>,
    infile: Option<&CStr>,
    flags: i32,
) -> Option<std::ffi::CString> {
    if check_secure() {
        return None;
    }

    let Some(tempname_cstr) = vim_tempname() else {
        emsg(gettext(E_NOTMP));
        return None;
    };
    let tempname = tempname_cstr.as_c_str();

    let command = make_filter_cmd(cmd.unwrap_or(c""), infile, Some(tempname), false);

    g_st().no_check_timestamps += 1;
    call_shell(
        Some(command.as_c_str()),
        K_SHELL_OPT_DO_OUT | K_SHELL_OPT_EXPAND | flags,
        None,
    );
    g_st().no_check_timestamps -= 1;

    drop(command);

    let mut result: Option<std::ffi::CString> = None;

    if let Some(f) = CFile::open(tempname, c"rb") {
        'done: {
            if f.seek(0, SEEK_END) == -1 {
                semsg!(gettext(E_CANNOT_READ_FROM_STR_2), tempname);
                break 'done;
            }
            let len_l = f.tell();
            if len_l == -1 || f.seek(0, SEEK_SET) == -1 {
                semsg!(gettext(E_CANNOT_READ_FROM_STR_2), tempname);
                break 'done;
            }

            let len = len_l as usize;
            let mut buffer = vec![0u8; len];
            let i = f.read_bytes(&mut buffer);
            drop(f);
            if i != len {
                semsg!(gettext(E_CANT_READ_FILE_STR), tempname);
            } else {
                for b in &mut buffer {
                    if *b == 0 {
                        *b = 1;
                    }
                }
                result = std::ffi::CString::new(buffer).ok();
            }
        }
    } else {
        semsg!(gettext(E_CANNOT_READ_FROM_STR_2), tempname);
    }

    os_remove(tempname);
    drop(tempname_cstr);
    result
}

/// os_system - synchronously execute a command in the shell
pub fn os_system(argv: Vec<std::ffi::CString>, input: &[u8], output: Option<&mut Vec<u8>>) -> i32 {
    do_os_system(argv, input, output, true, false)
}

fn do_os_system(
    argv: Vec<std::ffi::CString>,
    input: &[u8],
    output: Option<&mut Vec<u8>>,
    silent: bool,
    forward_output: bool,
) -> i32 {
    let mut exitcode: i32 = -1;

    #[cfg(windows)]
    let (oldval, old_output_cp, old_input_cp) = {
        let ov = os_getenv_cstr(c"NoDefaultCurrentDirectoryInExePath");
        os_setenv(c"NoDefaultCurrentDirectoryInExePath", c"1", 1);
        let oocp = os_get_console_output_cp();
        let oicp = os_get_console_cp();
        os_set_console_output_cp(CP_UTF8);
        os_set_console_cp(CP_UTF8);
        (ov, oocp, oicp)
    };

    out_data_decide_throttle(0);
    out_data_ring(Some(&[]));
    let has_input = !input.is_empty();

    let saved_buf = SH_STATE.with_borrow_mut(|st| std::mem::take(&mut st.system_buf));
    let cb_data = if forward_output {
        StreamCbData::ShellOut
    } else if output.is_none() {
        StreamCbData::None
    } else {
        StreamCbData::ShellBuf
    };

    let mut prog = [0u8; MAXPATHL];
    crate::memory::copy_bytes_to_c_buf(&mut prog, argv[0].as_bytes());

    let mloop = main_loop_ptr();
    let mut uvproc = LibuvProc::new();
    let proc_ = &mut uvproc.proc_;
    proc_.r#type = K_PROC_TYPE_UV;
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
                msg_outtrans(uv_strerror(status), 0, false);
                msg_puts(c": ");
                msg_outtrans(crate::memory::bytes_as_cstr(&prog), 0, false);
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
            let input_buffer = wstream_new_vec_buffer(input.to_vec());
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
        exitcode = proc_wait(proc_, -1, None);
        if !g_st().got_int && out_data_decide_throttle(0) {
            out_data_ring(None);
        }
        if forward_output {
            g_st().no_wait_return += 1;
            msg_end();
            g_st().no_wait_return -= 1;
            g_st().msg_no_more = false;
        }

        ui_busy_stop();

        debug_assert!(multiqueue_empty(events));
    }

    let buf = SH_STATE.with_borrow_mut(|st| std::mem::replace(&mut st.system_buf, saved_buf));
    if let Some(out) = output {
        *out = buf;
    }

    multiqueue_free(events);

    #[cfg(windows)]
    {
        restore_env_var(c"NoDefaultCurrentDirectoryInExePath", oldval);
        os_set_console_output_cp(old_output_cp);
        os_set_console_cp(old_input_cp);
    }

    exitcode
}

pub(crate) fn system_data_cb(buf: &[u8], _eof: bool) -> usize {
    SH_STATE.with_borrow_mut(|st| st.system_buf.extend_from_slice(buf));
    buf.len()
}

fn out_data_decide_throttle(size: usize) -> bool {
    let mut pulse_msg = [0u8; 4];
    let mut first_visit = false;
    let should_pulse = SH_STATE.with_borrow_mut(|st| {
        if size == 0 {
            let previous_decision = st.throttle_visit > 0;
            st.throttle_started = 0;
            st.throttle_received = 0;
            st.throttle_visit = 0;
            return Err(previous_decision);
        }

        st.throttle_received += size;
        if st.throttle_received < OUT_DATA_THRESHOLD
            || (st.throttle_started == 0 && st.throttle_received < size + 1000)
        {
            return Err(false);
        } else if st.throttle_visit == 0 {
            st.throttle_started = os_hrtime();
        } else {
            let since = os_hrtime() - st.throttle_started;
            if since < (st.throttle_visit as u64) * (NS_PER_SEC / 10) {
                return Err(true);
            }
            if since > 3 * NS_PER_SEC {
                st.throttle_received = 0;
                st.throttle_visit = 0;
                return Err(false);
            }
        }

        st.throttle_visit += 1;
        let tick = st.throttle_visit % 4;
        st.throttle_pulse_msg[0] = if tick > 0 { b'.' } else { b' ' };
        st.throttle_pulse_msg[1] = if tick > 1 { b'.' } else { b' ' };
        st.throttle_pulse_msg[2] = if tick > 2 { b'.' } else { b' ' };
        pulse_msg = st.throttle_pulse_msg;
        first_visit = st.throttle_visit == 1;
        Ok(())
    });
    if let Err(res) = should_pulse {
        return res;
    }
    if first_visit {
        msg_puts(c"...\n");
    }
    msg_putchar(b'\r' as i32);
    msg_puts(std::ffi::CStr::from_bytes_until_nul(&pulse_msg).unwrap_or(c""));
    msg_putchar(b'\r' as i32);
    ui_flush();
    true
}

fn out_data_ring(output: Option<&[u8]>) {
    let Some(output) = output else {
        let (ring_buf, len) = SH_STATE.with_borrow(|st| (st.last_skipped, st.last_skipped_len));
        let mut count = len;
        out_data_append_to_screen(&ring_buf[..len], &mut count, STDOUT_FILENO, true);
        return;
    };

    SH_STATE.with_borrow_mut(|st| {
        if output.is_empty() {
            st.last_skipped_len = 0;
            return;
        }

        let size = output.len();
        if size >= MAX_CHUNK_SIZE {
            let start = size - MAX_CHUNK_SIZE;
            st.last_skipped.copy_from_slice(&output[start..]);
            st.last_skipped_len = MAX_CHUNK_SIZE;
        } else {
            let keep_len = st.last_skipped_len.min(MAX_CHUNK_SIZE - size);
            let keep_start = st.last_skipped_len - keep_len;
            if keep_start != 0 {
                st.last_skipped
                    .copy_within(keep_start..keep_start + keep_len, 0);
            }
            st.last_skipped[keep_len..keep_len + size].copy_from_slice(output);
            st.last_skipped_len = keep_len + size;
        }
    });
}

pub fn out_data_event(str_copy: Vec<u8>, fd: i32) {
    let mut need_clear = true;
    let hl = if fd == STDERR_FILENO { HLF_SE } else { HLF_SO };
    msg_ext_set_kind(if fd == STDERR_FILENO {
        c"shell_err"
    } else {
        c"shell_out"
    });
    msg_ext_set_append(true);
    msg_ext_no_fast();
    let str_arg = NvimString::from_bytes(&str_copy[..str_copy.len().saturating_sub(1)]);
    msg_multiline(str_arg, hl, false, false, &mut need_clear);
    drop(str_copy);
    ui_flush();
}

fn out_data_append_to_screen(output: &[u8], count: &mut usize, fd: i32, eof: bool) {
    let mut off = 0usize;
    while off < *count {
        let slice = &output[off..*count];
        let i = if slice[0] != 0 {
            crate::mbyte::utfc_slice2len(slice)
        } else {
            1
        };
        if !eof && i == 1 && usize::from(UTF8LEN_TAB_ZERO[slice[0] as usize]) > slice.len() {
            *count = off;
            break;
        }
        off += i;
    }

    if *count == 0 {
        return;
    }

    let mut str_copy = Vec::with_capacity(*count + 1);
    str_copy.extend_from_slice(&output[..*count]);
    str_copy.push(0);
    if ui_has(K_UIMESSAGES) {
        let qc = loop_queues(&mut *main_loop_ptr());
        multiqueue_put_event(
            qc.fast_events.borrow_mut(),
            Event::ShellOutData { str_copy, fd },
        );
    } else {
        out_data_event(str_copy, fd);
    }
}

pub(crate) fn out_data_cb(stream_fd: i32, buf: &[u8], eof: bool) -> usize {
    let mut count = buf.len();
    if count > 0 && out_data_decide_throttle(count) {
        out_data_ring(Some(buf));
    } else if count > 0 {
        out_data_append_to_screen(buf, &mut count, stream_fd, eof);
    }
    count
}

fn tokenize(str_in: Option<&CStr>, argv: &mut Vec<std::ffi::CString>) {
    let Some(mut p) = str_in else {
        return;
    };

    while !p.to_bytes().is_empty() {
        let len = word_length(p);
        argv.push(vim_strnsave_unquoted(&p.to_bytes()[..len]));
        p = crate::charset::skipwhite(crate::memory::CStrSubslice::advance(p, len));
    }
}

pub(crate) fn word_length(str_in: &CStr) -> usize {
    let bytes = str_in.to_bytes();
    let mut inquote = false;
    let mut length: usize = 0;

    while length < bytes.len() && (inquote || (bytes[length] != b' ' && bytes[length] != TAB)) {
        if bytes[length] == b'"' {
            inquote = !inquote;
        } else if bytes[length] == b'\\' && inquote && length + 1 < bytes.len() {
            length += 1;
        }

        length += 1;
    }

    length
}

fn read_input(buf: &mut Vec<u8>) {
    read_buffer_into(
        Some(crate::globals::curbuf_mut()),
        curbuf_mut().b_op_start.lnum,
        curbuf_mut().b_op_end.lnum,
        buf,
    );
}

fn write_output(output: &mut Vec<u8>, eof: bool) -> usize {
    let mut remaining = output.len();
    if remaining == 0 {
        return 0;
    }
    output.push(0);
    let mut base: usize = 0;
    let mut off: usize = 0;
    while off < remaining {
        if output[base + off] == CAR
            && off + 1 < remaining
            && output[base + off + 1] == NL as u8
            && !curbuf_mut().b_p_bin.get()
        {
            output[base + off] = 0;
            let lnum = curwin_mut().w_cursor.lnum;
            curwin_mut().w_cursor.lnum += 1;
            ml_append(lnum, &output[base..base + off], off as i32 + 1, false);
            let skip = off + 2;
            base += skip;
            remaining -= skip;
            off = 0;
            continue;
        } else if (output[base + off] == CAR && !curbuf_mut().b_p_bin.get())
            || output[base + off] == NL as u8
        {
            output[base + off] = 0;
            let lnum = curwin_mut().w_cursor.lnum;
            curwin_mut().w_cursor.lnum += 1;
            ml_append(lnum, &output[base..base + off], off as i32 + 1, false);
            let skip = off + 1;
            base += skip;
            remaining -= skip;
            off = 0;
            continue;
        }

        if output[base + off] == 0 {
            output[base + off] = NL as u8;
        }
        off += 1;
    }

    if eof {
        if remaining != 0 {
            let lnum = curwin_mut().w_cursor.lnum;
            curwin_mut().w_cursor.lnum += 1;
            ml_append(
                lnum,
                crate::memory::bytes_until_nul(&output[base..]),
                0,
                false,
            );
            curbuf_mut().b_no_eol_lnum.set(curwin_mut().w_cursor.lnum);
            base += remaining;
        } else {
            curbuf_mut().b_no_eol_lnum.set(0);
        }
    }
    output.pop();

    ui_flush();

    base
}

pub(crate) fn shell_write_cb(stream: &mut Stream, status: i32) {
    if status != 0 {
        msg_schedule_semsg!(
            gettext(c"E5677: Error writing input to shell-command: %s"),
            uv_err_name(status),
        );
    }
    stream_may_close(stream);
}

fn shell_xescape_xquote(cmd: &CStr) -> std::ffi::CString {
    if opt_vars().p_sxq.is_empty() {
        return cmd.to_owned();
    }

    let cmd_bytes = cmd.to_bytes();
    let p_sxq = opt_vars().p_sxq.to_owned();
    let sxq_bytes = p_sxq.as_bytes();
    let owned_ecmd = if !opt_vars().p_sxe.is_empty() && sxq_bytes == b"(" {
        Some(vim_strsave_escaped_ext(
            cmd_bytes,
            opt_vars().p_sxe.as_bytes(),
            b'^' as _,
            false,
        ))
    } else {
        None
    };
    let ecmd_bytes = owned_ecmd
        .as_ref()
        .map_or(cmd_bytes, std::ffi::CString::as_bytes);
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

    crate::memory::bytes_to_cstring(&out)
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
        let _guard = crate::globals::TEST_GLOBAL_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let save_sh = opt_vars().p_sh.replace(OptString::from_cstr(c"/bin/sh"));
        let save_shcf = opt_vars().p_shcf.replace(OptString::from_cstr(c"-c"));
        let save_sxq = opt_vars().p_sxq.replace(OptString::new());
        let save_sxe = opt_vars().p_sxe.replace(OptString::new());

        let collect_argv = |cmd: Option<&CStr>, extra: Option<&CStr>| -> Vec<String> {
            shell_build_argv(cmd, extra)
                .into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect()
        };

        assert_eq!(collect_argv(None, None), vec!["/bin/sh"]);
        assert_eq!(
            collect_argv(Some(c"abc  def"), None),
            vec!["/bin/sh", "-c", "abc  def"]
        );
        assert_eq!(
            collect_argv(None, Some(c"ghi  jkl")),
            vec!["/bin/sh", "ghi  jkl"]
        );
        assert_eq!(
            collect_argv(Some(c"abc  def"), Some(c"ghi  jkl")),
            vec!["/bin/sh", "ghi  jkl", "-c", "abc  def"]
        );

        opt_vars()
            .p_sh
            .set_option_cstr(c"/Program\" \"Files/zsh -f");
        opt_vars()
            .p_shcf
            .set_option_cstr(c"-x -o \"sh word split\" \"-\"c");
        assert_eq!(
            collect_argv(Some(c"abc  def"), Some(c"ghi  jkl")),
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

        opt_vars().p_sh.set_option_cstr(c"/bin/sh");
        opt_vars().p_shcf.set_option_cstr(c"-c");
        opt_vars().p_sxq.set_option_cstr(c"(");
        opt_vars().p_sxe.set_option_cstr(c"\"&|<>()@^");
        assert_eq!(
            collect_argv(Some(c"echo &|<>()@^"), None),
            vec!["/bin/sh", "-c", "(echo ^&^|^<^>^(^)^@^^)"]
        );

        opt_vars().p_sxq.set_option_cstr(c"\"(");
        assert_eq!(
            collect_argv(Some(c"echo -n some text"), None),
            vec!["/bin/sh", "-c", "\"(echo -n some text)\""]
        );

        opt_vars().p_sxq.set_option_cstr(c"\"");
        opt_vars().p_sxe.clear_option_str();
        assert_eq!(
            collect_argv(Some(c"echo -n some text"), None),
            vec!["/bin/sh", "-c", "\"echo -n some text\""]
        );

        let argv_arr = [c"foo".to_owned(), c"".to_owned(), c"bar".to_owned()];
        let s = shell_argv_to_str(&argv_arr);
        assert_eq!(s.to_str().unwrap(), "'foo' '' 'bar'");

        opt_vars().p_sh.set(save_sh);
        opt_vars().p_shcf.set(save_shcf);
        opt_vars().p_sxq.set(save_sxq);
        opt_vars().p_sxe.set(save_sxe);
    }
}
