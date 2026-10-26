//! Shell execution and wildcard expansion (`src/nvim/os/shell.c`).

use crate::api::private::validate::NvimString;
use crate::buffer::{kUIMessages, read_buffer_into};
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
use crate::event::proc::{LibuvProc, Proc, proc_spawn, proc_stop, proc_wait};
use crate::event::rstream::{RStream, rstream_init, rstream_start, stream_read_cb};
use crate::event::stream::{Stream, StreamCbData, stream_may_close};
use crate::event::wstream::{
    WBufferFinalizer, wstream_init, wstream_new_buffer, wstream_set_write_cb, wstream_write,
};
use crate::ex_cmds::{check_secure, kShellOptRead, kShellOptWrite, make_filter_cmd};
use crate::ex_docmd::CAR;
#[cfg(unix)]
use crate::fileio::fopen;
use crate::fileio::{SEEK_END, SEEK_SET, fclose, fread, fseek, ftell, vim_tempname};
use crate::globals::*;
#[cfg(unix)]
use crate::math::{FAIL, OK};
#[cfg(windows)]
use crate::mbyte::CP_UTF8;
use crate::mbyte::{utf8len_tab_zero, utfc_ptr2len_len};
use crate::memline::ml_append;
use crate::memory::{cstr_take, xstrlcat, xstrlcpy};
#[cfg(unix)]
use crate::memory::{xmalloc, xstrdup};
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
use crate::profile::{prof_child_enter, prof_child_exit, proftime_T};
use crate::strings::{gettext, vim_strnsave_unquoted, vim_strsave_escaped_ext};
#[cfg(unix)]
use crate::strings::{strcat, strcpy, strlen, strstr, vim_strchr};
use crate::tag::tag_freematch;
use crate::ui::{ui_busy_start, ui_busy_stop, ui_flush, ui_has};
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::autocmd::PROF_YES;
use crate::change::TAB;
use crate::channel::{STDERR_FILENO, STDOUT_FILENO, kProcTypeUv};
use crate::diff::{kShellOptDoOut, kShellOptSilent};
use crate::eval::typval::varnumber_T;
use crate::state::MODE_EXTERNCMD;

const OUT_DATA_THRESHOLD: usize = 1024 * 10;
const MAX_CHUNK_SIZE: usize = OUT_DATA_THRESHOLD / 2;

#[cfg(unix)]
const SHELL_SPECIAL: &CStr = c"\t \"&'$;<>()\\|\n";

const kShellOptExpand: c_int = 2;
const kShellOptHideMess: c_int = 64;

#[cfg(unix)]
const STYLE_ECHO: c_int = 0;
#[cfg(unix)]
const STYLE_GLOB: c_int = 1;
#[cfg(unix)]
const STYLE_VIMGLOB: c_int = 2;
#[cfg(unix)]
const STYLE_PRINT: c_int = 3;
#[cfg(unix)]
const STYLE_BT: c_int = 4;
#[cfg(unix)]
const STYLE_GLOBSTAR: c_int = 5;

const HLF_SE: c_int = 72;
const HLF_SO: c_int = 73;

#[cfg(unix)]
static mut did_find_nul: bool = false;
static mut throttle_started: u64 = 0;
static mut throttle_received: usize = 0;
static mut throttle_visit: usize = 0;
static mut throttle_pulse_msg: [c_char; 4] = [b' ' as c_char, b' ' as c_char, b' ' as c_char, NUL];

static mut last_skipped: [c_char; MAX_CHUNK_SIZE] = [0; MAX_CHUNK_SIZE];
static mut last_skipped_len: usize = 0;

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleOutputCP() -> u32;
    fn GetConsoleCP() -> u32;
    fn SetConsoleOutputCP(wCodePageID: u32) -> c_int;
    fn SetConsoleCP(wCodePageID: u32) -> c_int;
}

#[cfg(unix)]
unsafe fn save_patterns(num_pat: c_int, pat: *mut *mut c_char, file: &mut Vec<*mut c_char>) {
    unsafe {
        file.clear();
        file.reserve(num_pat as usize);
        for i in 0..(num_pat as usize) {
            let s = xstrdup(*pat.add(i));
            backslash_halve(s);
            file.push(s);
        }
    }
}

#[cfg(unix)]
unsafe fn have_wildcard(num: c_int, file: *mut *mut c_char) -> bool {
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
unsafe fn have_dollars(num: c_int, file: *mut *mut c_char) -> bool {
    unsafe {
        for i in 0..(num as usize) {
            if !vim_strchr(*file.add(i), b'$' as c_int).is_null() {
                return true;
            }
        }
        false
    }
}

/// Performs wildcard pattern matching using the shell.
#[cfg(unix)]
pub unsafe fn os_expand_wildcards(
    num_pat: c_int,
    pat: *mut *mut c_char,
    file: &mut Vec<*mut c_char>,
    flags: c_int,
) -> c_int {
    const SH_VIMGLOB_FUNC: &CStr =
        c"vimglob() { while [ $# -ge 1 ]; do echo \"$1\"; shift; done }; vimglob >";
    const SH_GLOBSTAR_OPT: &CStr = c"[[ ${BASH_VERSINFO[0]} -ge 4 ]] && shopt -s globstar; ";

    unsafe {
        let mut extra_shell_arg: *mut c_char = ptr::null_mut();
        let mut shellopts: c_int = kShellOptExpand | kShellOptSilent;
        let mut shell_style: c_int = STYLE_ECHO;
        let mut ampersand = false;

        #[cfg(unix)]
        let is_fish_shell = {
            let tail = invocation_path_tail(p_sh, ptr::null_mut());
            let bytes = CStr::from_ptr(tail).to_bytes();
            bytes.starts_with(b"fish")
        };
        #[cfg(not(unix))]
        let is_fish_shell = false;

        file.clear();

        if !have_wildcard(num_pat, pat) {
            save_patterns(num_pat, pat, file);
            return OK;
        }

        if sandbox != 0 && check_secure() {
            return FAIL;
        }

        if secure != 0 {
            for i in 0..(num_pat as usize) {
                if !vim_strchr(*pat.add(i), b'`' as c_int).is_null() && check_secure() {
                    return FAIL;
                }
            }
        }

        let Some(tempname_cstr) = vim_tempname() else {
            emsg(gettext(e_notmp.as_ptr()));
            return FAIL;
        };
        let tempname = tempname_cstr.as_ptr();

        if num_pat == 1 && **pat == b'`' as c_char {
            let pat0_len = strlen(*pat);
            if pat0_len > 2 && *(*pat).add(pat0_len - 1) == b'`' as c_char {
                shell_style = STYLE_BT;
            }
        }
        if shell_style != STYLE_BT {
            let sh_len = strlen(p_sh);
            if sh_len >= 3 {
                let tail3 = CStr::from_ptr(p_sh.add(sh_len - 3)).to_bytes();
                if tail3 == b"csh" {
                    shell_style = STYLE_GLOB;
                } else if tail3 == b"zsh" {
                    shell_style = STYLE_PRINT;
                }
            }
        }
        if shell_style == STYLE_ECHO {
            if !strstr(path_tail(p_sh), c"bash".as_ptr()).is_null() {
                shell_style = STYLE_GLOBSTAR;
            } else if !strstr(path_tail(p_sh), c"sh".as_ptr()).is_null() {
                shell_style = STYLE_VIMGLOB;
            }
        }

        let mut len = strlen(tempname) + 29;
        if shell_style == STYLE_VIMGLOB {
            len += SH_VIMGLOB_FUNC.to_bytes().len();
        } else if shell_style == STYLE_GLOBSTAR {
            len += SH_VIMGLOB_FUNC.to_bytes().len() + SH_GLOBSTAR_OPT.to_bytes().len();
        }

        for i in 0..(num_pat as usize) {
            len += 1;
            let p_i = *pat.add(i);
            let mut j = 0;
            while *p_i.add(j) != NUL {
                if !vim_strchr(SHELL_SPECIAL.as_ptr(), *p_i.add(j) as u8 as c_int).is_null() {
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
                strcpy(command, c"begin; ".as_ptr());
            } else {
                strcpy(command, c"(".as_ptr());
            }
            strcat(command, (*pat).add(1));
            let mut p = command.add(strlen(command) - 1);
            if is_fish_shell {
                *p = b';' as c_char;
                p = p.sub(1);
                strcat(command, c" end".as_ptr());
            } else {
                *p = b')' as c_char;
                p = p.sub(1);
            }
            while (p as usize) > (command as usize) && ascii_iswhite(*p) {
                p = p.sub(1);
            }
            if *p == b'&' as c_char {
                ampersand = true;
                *p = b' ' as c_char;
            }
            strcat(command, c">".as_ptr());
        } else {
            *command = NUL;
            if shell_style == STYLE_GLOB {
                if (flags & EW_NOTFOUND) != 0 {
                    strcat(command, c"set nonomatch; ".as_ptr());
                } else {
                    strcat(command, c"unset nonomatch; ".as_ptr());
                }
            }
            if shell_style == STYLE_GLOB {
                strcat(command, c"glob >".as_ptr());
            } else if shell_style == STYLE_PRINT {
                strcat(command, c"print -N >".as_ptr());
            } else if shell_style == STYLE_VIMGLOB {
                strcat(command, SH_VIMGLOB_FUNC.as_ptr());
            } else if shell_style == STYLE_GLOBSTAR {
                strcat(command, SH_GLOBSTAR_OPT.as_ptr());
                strcat(command, SH_VIMGLOB_FUNC.as_ptr());
            } else {
                strcat(command, c"echo >".as_ptr());
            }
        }

        strcat(command, tempname);

        if shell_style != STYLE_BT {
            for i in 0..(num_pat as usize) {
                let mut intick = false;
                let p_i = *pat.add(i);

                let mut p = command.add(strlen(command));
                *p = b' ' as c_char;
                p = p.add(1);
                let mut j = 0;
                while *p_i.add(j) != NUL {
                    let ch = *p_i.add(j);
                    if ch == b'`' as c_char {
                        intick = !intick;
                    } else if ch == b'\\' as c_char && *p_i.add(j + 1) != NUL {
                        let next_ch = *p_i.add(j + 1);
                        if intick
                            || !vim_strchr(SHELL_SPECIAL.as_ptr(), next_ch as u8 as c_int).is_null()
                            || next_ch == b'`' as c_char
                        {
                            *p = b'\\' as c_char;
                            p = p.add(1);
                        }
                        j += 1;
                    } else if !intick
                        && ((flags & EW_KEEPDOLLAR) == 0 || ch != b'$' as c_char)
                        && !vim_strchr(SHELL_SPECIAL.as_ptr(), ch as u8 as c_int).is_null()
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
            shellopts |= kShellOptHideMess;
        }

        if ampersand {
            strcat(command, c"&".as_ptr());
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
                msg_putchar(b'\n' as c_int);
                cmdline_row = Rows - 1;
                msg(gettext(e_wildexpand.as_ptr()), 0);
                msg_start();
            }

            if shell_style == STYLE_BT {
                return FAIL;
            }
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return OK;
            }
            return FAIL;
        }

        let fd = fopen(tempname, c"rb".as_ptr());
        if fd.is_null() {
            if (flags & EW_SILENT) == 0 {
                msg(gettext(e_wildexpand.as_ptr()), 0);
                msg_start();
            }
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return OK;
            }
            return FAIL;
        }
        let fseek_res = fseek(fd, 0, SEEK_END);
        if fseek_res < 0 {
            fclose(fd);
            return FAIL;
        }
        let templen = ftell(fd) as i64;
        if templen < 0 {
            fclose(fd);
            return FAIL;
        }
        len = templen as usize;
        fseek(fd, 0, SEEK_SET);
        let mut buffer_vec = vec![0 as c_char; len + 1];
        let buffer = buffer_vec.as_mut_ptr();
        let readlen = fread(buffer.cast(), 1, len, fd);
        fclose(fd);
        os_remove(tempname);
        if readlen != len {
            semsg!(gettext(e_cant_read_file_str.as_ptr()), tempname);
            return FAIL;
        }
        drop(tempname_cstr);

        let mut count: c_int = 0;
        if shell_style == STYLE_ECHO {
            *buffer.add(len) = b'\n' as c_char;
            let mut p = buffer;
            while *p != b'\n' as c_char {
                while *p != b' ' as c_char && *p != b'\n' as c_char {
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
            while *p != NUL {
                while *p != b'\n' as c_char && *p != NUL {
                    p = p.add(1);
                }
                if *p != NUL {
                    p = p.add(1);
                }
                p = skipwhite(p);
                count += 1;
            }
        } else {
            let mut check_spaces = false;
            if shell_style == STYLE_PRINT && !did_find_nul {
                *buffer.add(len) = NUL;
                if len != 0 && (strlen(buffer) as c_int) < (len as c_int) {
                    did_find_nul = true;
                } else {
                    check_spaces = true;
                }
            }

            if len != 0 && *buffer.add(len - 1) == NUL {
                len -= 1;
            } else {
                *buffer.add(len) = NUL;
            }
            let mut p = buffer;
            while (p as usize) < (buffer.add(len) as usize) {
                if *p == NUL || (*p == b' ' as c_char && check_spaces) {
                    count += 1;
                    *p = NUL;
                }
                p = p.add(1);
            }
            if len != 0 {
                count += 1;
            }
        }
        debug_assert!(*buffer.add(len) == NUL || *buffer.add(len) == b'\n' as c_char);

        if count == 0 {
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return OK;
            }
            return FAIL;
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
                while !(shell_style == STYLE_ECHO && *p == b' ' as c_char)
                    && *p != b'\n' as c_char
                    && *p != NUL
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
                while *p != NUL && (p as usize) < (buffer.add(len) as usize) {
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

            let new_entry: *mut c_char = xmalloc(strlen(entry) + 1 + usize::from(dir)).cast();
            strcpy(new_entry, entry);
            if dir {
                add_pathsep(new_entry);
            }
            file.push(new_entry);
        }
        drop(buffer_vec);

        if file.is_empty() {
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, file);
                return OK;
            }
            return FAIL;
        }

        OK
    }
}

/// Builds the argument vector for running the user-configured 'shell' (p_sh)
/// with an optional command prefixed by 'shellcmdflag' (p_shcf).
pub unsafe fn shell_build_argv(
    cmd: *const c_char,
    extra_args: *const c_char,
) -> Vec<std::ffi::CString> {
    unsafe {
        let mut rv: Vec<std::ffi::CString> = Vec::new();
        tokenize(p_sh, &mut rv);

        if !extra_args.is_null() {
            rv.push(CStr::from_ptr(extra_args).to_owned());
        }

        if !cmd.is_null() {
            tokenize(p_shcf, &mut rv);
            rv.push(shell_xescape_xquote(cmd));
        }

        debug_assert!(!rv.is_empty());

        rv
    }
}

/// Joins shell arguments from `argv` into a fixed 256-byte string buffer.
pub unsafe fn shell_argv_to_str(argv: &[std::ffi::CString]) -> [c_char; 256] {
    unsafe {
        let mut n: usize = 0;
        let maxsize: usize = 256;
        let mut rv: [c_char; 256] = [0; 256];
        let rv_ptr = rv.as_mut_ptr();
        if argv.is_empty() {
            return rv;
        }
        for arg in argv {
            xstrlcat(rv_ptr, c"'".as_ptr(), maxsize);
            xstrlcat(rv_ptr, arg.as_ptr(), maxsize);
            n = xstrlcat(rv_ptr, c"' ".as_ptr(), maxsize);
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
}

/// Calls the user-configured 'shell' (p_sh) for running a command or wildcard expansion.
pub unsafe fn os_call_shell(cmd: *mut c_char, opts: c_int, extra_args: *mut c_char) -> c_int {
    unsafe {
        let mut input: Vec<c_char> = Vec::new();
        let mut output: Vec<c_char> = Vec::new();
        let mut want_output = false;
        let current_state = State;
        let mut forward_output = true;

        signal_reject_deadly();

        if (opts & (kShellOptHideMess | kShellOptExpand)) != 0 {
            forward_output = false;
        } else {
            State = MODE_EXTERNCMD;

            if (opts & kShellOptWrite) != 0 {
                read_input(&mut input);
            }

            if (opts & kShellOptRead) != 0 {
                want_output = true;
                forward_output = false;
            } else if (opts & kShellOptDoOut) != 0 {
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
            emsg_silent != 0,
            forward_output,
        );
        drop(input);

        if !output.is_empty() {
            let nread = output.len();
            write_output(output.as_mut_ptr(), nread, true);
        }

        if emsg_silent == 0 && exitcode != 0 && (opts & kShellOptSilent) == 0 {
            msg_ext_set_kind(c"shell_ret".as_ptr());
            msg_ext_no_fast();
            if !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }
            msg_puts(gettext(c"shell returned ".as_ptr()));
            msg_outnum(exitcode);
        }

        State = current_state;
        signal_accept_deadly();

        exitcode
    }
}

/// os_call_shell() wrapper. Handles 'verbose', :profile, and v:shell_error.
pub unsafe fn call_shell(cmd: *mut c_char, opts: c_int, extra_shell_arg: *mut c_char) -> c_int {
    unsafe {
        let mut wait_time: proftime_T = 0;

        if p_verbose > 3 {
            verbose_enter();
            smsg!(
                0,
                gettext(c"Executing command: \"%s\"".as_ptr()),
                if cmd.is_null() { p_sh } else { cmd },
            );
            if !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }
            verbose_leave();
        }

        if do_profiling == PROF_YES {
            prof_child_enter(&raw mut wait_time);
        }

        let retval = if *p_sh == NUL {
            emsg(gettext(e_shellempty.as_ptr()));
            -1
        } else {
            tag_freematch();
            os_call_shell(cmd, opts, extra_shell_arg)
        };

        set_vim_var_nr(VV_SHELL_ERROR, varnumber_T::from(retval));
        if do_profiling == PROF_YES {
            prof_child_exit(&raw mut wait_time);
        }

        retval
    }
}

/// Get the stdout of an external command.
pub unsafe fn get_cmd_output(
    cmd: *mut c_char,
    infile: *mut c_char,
    flags: c_int,
) -> Option<std::ffi::CString> {
    unsafe {
        if check_secure() {
            return None;
        }

        let Some(tempname_cstr) = vim_tempname() else {
            emsg(gettext(e_notmp.as_ptr()));
            return None;
        };
        let tempname = tempname_cstr.as_ptr().cast_mut();

        let command = make_filter_cmd(cmd, infile, tempname, false);

        no_check_timestamps += 1;
        call_shell(
            command.as_ptr().cast_mut(),
            kShellOptDoOut | kShellOptExpand | flags,
            ptr::null_mut(),
        );
        no_check_timestamps -= 1;

        drop(command);

        let fd = os_fopen(tempname, c"rb".as_ptr());
        let mut result: Option<std::ffi::CString> = None;

        'done: {
            if fd.is_null() || fseek(fd, 0, SEEK_END) == -1 {
                semsg!(gettext(e_cannot_read_from_str_2.as_ptr()), tempname);
                if !fd.is_null() {
                    fclose(fd);
                }
                break 'done;
            }
            let len_l = ftell(fd);
            if len_l == -1 || fseek(fd, 0, SEEK_SET) == -1 {
                semsg!(gettext(e_cannot_read_from_str_2.as_ptr()), tempname);
                fclose(fd);
                break 'done;
            }

            let len = len_l as usize;
            let mut buffer = vec![0u8; len];
            let i = fread(buffer.as_mut_ptr().cast(), 1, len, fd);
            fclose(fd);
            if i != len {
                semsg!(gettext(e_cant_read_file_str.as_ptr()), tempname);
            } else {
                for b in &mut buffer {
                    if *b == 0 {
                        *b = 1;
                    }
                }
                result = Some(std::ffi::CString::from_vec_unchecked(buffer));
            }
        }

        os_remove(tempname);
        drop(tempname_cstr);
        result
    }
}

/// os_system - synchronously execute a command in the shell
pub unsafe fn os_system(
    argv: Vec<std::ffi::CString>,
    input: *const c_char,
    len: usize,
    output: Option<&mut Vec<c_char>>,
) -> c_int {
    unsafe { do_os_system(argv, input, len, output, true, false) }
}

unsafe fn do_os_system(
    argv: Vec<std::ffi::CString>,
    input: *const c_char,
    len: usize,
    output: Option<&mut Vec<c_char>>,
    silent: bool,
    forward_output: bool,
) -> c_int {
    unsafe {
        let mut exitcode: c_int = -1;

        #[cfg(windows)]
        let (oldval, old_output_cp, old_input_cp) = {
            let ov = os_getenv_cstr(c"NoDefaultCurrentDirectoryInExePath".as_ptr());
            os_setenv(
                c"NoDefaultCurrentDirectoryInExePath".as_ptr(),
                c"1".as_ptr(),
                1,
            );
            let oocp = GetConsoleOutputCP();
            let oicp = GetConsoleCP();
            SetConsoleOutputCP(CP_UTF8);
            SetConsoleCP(CP_UTF8);
            (ov, oocp, oicp)
        };

        out_data_decide_throttle(0);
        out_data_ring(ptr::null(), 0);
        let has_input = !input.is_null() && len > 0;

        let mut buf: Vec<c_char> = Vec::new();
        let mut data_cb: stream_read_cb = Some(system_data_cb);

        if forward_output {
            data_cb = Some(out_data_cb);
        } else if output.is_none() {
            data_cb = None;
        }

        let mut prog = [0 as c_char; MAXPATHL];
        xstrlcpy(prog.as_mut_ptr(), argv[0].as_ptr(), MAXPATHL);

        let mloop = main_loop_ptr();
        let mut uvproc = LibuvProc::new();
        let proc_: *mut Proc = &raw mut uvproc.proc_;
        (*proc_).r#type = kProcTypeUv;
        (*proc_).loop_ = mloop;
        (*proc_).status = -1;
        (*proc_).out.s.fd = STDOUT_FILENO;
        (*proc_).err.s.fd = STDERR_FILENO;

        let qc = loop_queues(mloop);
        let events = multiqueue_new_child((*qc).events);
        (*proc_).events = events;
        (*proc_).argv = argv;

        'end: {
            let status = proc_spawn(proc_, has_input, true, true);
            if status != 0 {
                loop_poll_events(mloop, 0);
                if !silent {
                    msg_puts(gettext(c"\nshell failed to start: ".as_ptr()));
                    msg_outtrans(uv_strerror(status), 0, false);
                    msg_puts(c": ".as_ptr());
                    msg_outtrans(prog.as_ptr(), 0, false);
                    msg_putchar(b'\n' as c_int);
                }
                break 'end;
            }

            let s_in = &raw mut (*proc_).r#in;
            let r_out = &raw mut (*proc_).out;
            let r_err = &raw mut (*proc_).err;

            if has_input {
                wstream_init(s_in, 0);
            }
            rstream_init(r_out);
            rstream_start(r_out, data_cb, StreamCbData::Buf(&raw mut buf));
            rstream_init(r_err);
            rstream_start(r_err, data_cb, StreamCbData::Buf(&raw mut buf));

            if has_input {
                let input_buffer =
                    wstream_new_buffer(input.cast_mut(), len, WBufferFinalizer::None);
                if wstream_write(s_in, input_buffer) != 0 {
                    proc_stop(proc_);
                    break 'end;
                }
                wstream_set_write_cb(s_in, Some(shell_write_cb));
            }

            ui_busy_start();
            ui_flush();
            if forward_output {
                msg_sb_eol();
                msg_start();
                msg_no_more = true;
                lines_left = -1;
            }
            exitcode = proc_wait(proc_, -1, ptr::null_mut());
            if !got_int && out_data_decide_throttle(0) {
                out_data_ring(ptr::null(), usize::MAX);
            }
            if forward_output {
                no_wait_return += 1;
                msg_end();
                no_wait_return -= 1;
                msg_no_more = false;
            }

            ui_busy_stop();

            if let Some(out) = output {
                *out = buf;
            }

            debug_assert!(multiqueue_empty(events));
        }

        multiqueue_free(events);

        #[cfg(windows)]
        {
            restore_env_var(c"NoDefaultCurrentDirectoryInExePath".as_ptr(), oldval);
            SetConsoleOutputCP(old_output_cp);
            SetConsoleCP(old_input_cp);
        }

        exitcode
    }
}

unsafe fn system_data_cb(
    _stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: StreamCbData,
    _eof: bool,
) -> usize {
    unsafe {
        if !buf.is_null() && count > 0 {
            let dbuf = data.as_buf();
            (*dbuf).extend_from_slice(std::slice::from_raw_parts(buf, count));
        }
        count
    }
}

unsafe fn out_data_decide_throttle(size: usize) -> bool {
    unsafe {
        if size == 0 {
            let previous_decision = throttle_visit > 0;
            throttle_started = 0;
            throttle_received = 0;
            throttle_visit = 0;
            return previous_decision;
        }

        throttle_received += size;
        if throttle_received < OUT_DATA_THRESHOLD
            || (throttle_started == 0 && throttle_received < size + 1000)
        {
            return false;
        } else if throttle_visit == 0 {
            throttle_started = os_hrtime();
        } else {
            let since = os_hrtime() - throttle_started;
            if since < (throttle_visit as u64) * (NS_PER_SEC / 10) {
                return true;
            }
            if since > 3 * NS_PER_SEC {
                throttle_received = 0;
                throttle_visit = 0;
                return false;
            }
        }

        throttle_visit += 1;
        let tick = throttle_visit % 4;
        throttle_pulse_msg[0] = if tick > 0 {
            b'.' as c_char
        } else {
            b' ' as c_char
        };
        throttle_pulse_msg[1] = if tick > 1 {
            b'.' as c_char
        } else {
            b' ' as c_char
        };
        throttle_pulse_msg[2] = if tick > 2 {
            b'.' as c_char
        } else {
            b' ' as c_char
        };
        if throttle_visit == 1 {
            msg_puts(c"...\n".as_ptr());
        }
        msg_putchar(b'\r' as c_int);
        msg_puts((&raw const throttle_pulse_msg).cast());
        msg_putchar(b'\r' as c_int);
        ui_flush();
        true
    }
}

unsafe fn out_data_ring(output: *const c_char, size: usize) {
    unsafe {
        debug_assert!(!output.is_null() || size == 0 || size == usize::MAX);

        if output.is_null() && size == 0 {
            last_skipped_len = 0;
            return;
        }

        let ring_ptr = (&raw mut last_skipped).cast::<c_char>();
        if output.is_null() && size == usize::MAX {
            out_data_append_to_screen(ring_ptr, &raw mut last_skipped_len, STDOUT_FILENO, true);
            return;
        }

        if size >= MAX_CHUNK_SIZE {
            let start = size - MAX_CHUNK_SIZE;
            ptr::copy_nonoverlapping(output.add(start), ring_ptr, MAX_CHUNK_SIZE);
            last_skipped_len = MAX_CHUNK_SIZE;
        } else if size > 0 {
            let keep_len = last_skipped_len.min(MAX_CHUNK_SIZE - size);
            let keep_start = last_skipped_len - keep_len;
            if keep_start != 0 {
                ptr::copy(ring_ptr.add(keep_start), ring_ptr, keep_len);
            }
            ptr::copy_nonoverlapping(output, ring_ptr.add(keep_len), size);
            last_skipped_len = keep_len + size;
        }
    }
}

pub unsafe fn out_data_event(mut str_copy: Vec<c_char>, fd: c_int) {
    unsafe {
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
        msg_multiline(str_arg, hl, false, false, &raw mut need_clear);
        drop(str_copy);
        ui_flush();
    }
}

unsafe fn out_data_append_to_screen(
    output: *const c_char,
    count: *mut usize,
    fd: c_int,
    eof: bool,
) {
    unsafe {
        let mut p = output;
        let end = output.add(*count);
        while (p as usize) < (end as usize) {
            let off = (p as usize).wrapping_sub(output as usize);
            let i = if *p != NUL {
                utfc_ptr2len_len(p, (*count - off) as c_int)
            } else {
                1
            };
            if !eof
                && i == 1
                && usize::from(utf8len_tab_zero[*p as u8 as usize])
                    > (end as usize).wrapping_sub(p as usize)
            {
                *count = off;
                break;
            }
            p = p.add(i as usize);
        }

        let str_copy = std::slice::from_raw_parts(output, *count).to_vec();
        if ui_has(kUIMessages) {
            let qc = loop_queues(main_loop_ptr());
            multiqueue_put_event((*qc).fast_events, Event::ShellOutData { str_copy, fd });
        } else {
            out_data_event(str_copy, fd);
        }
    }
}

unsafe fn out_data_cb(
    stream: *mut RStream,
    ptr_data: *const c_char,
    mut count: usize,
    _data: StreamCbData,
    eof: bool,
) -> usize {
    unsafe {
        if count > 0 && out_data_decide_throttle(count) {
            out_data_ring(ptr_data, count);
        } else if count > 0 {
            let fd = (*stream).s.fd;
            out_data_append_to_screen(ptr_data, &raw mut count, fd, eof);
        }
        count
    }
}

unsafe fn tokenize(str_in: *const c_char, argv: &mut Vec<std::ffi::CString>) {
    unsafe {
        let mut p = str_in;

        while *p != NUL {
            let len = word_length(p);
            argv.push(vim_strnsave_unquoted(p, len));
            p = skipwhite(p.add(len));
        }
    }
}

pub(crate) unsafe fn word_length(str_in: *const c_char) -> usize {
    unsafe {
        let mut p = str_in;
        let mut inquote = false;
        let mut length: usize = 0;

        while *p != NUL && (inquote || (*p != b' ' as c_char && *p != TAB)) {
            if *p == b'"' as c_char {
                inquote = !inquote;
            } else if *p == b'\\' as c_char && inquote {
                p = p.add(1);
                length += 1;
            }

            p = p.add(1);
            length += 1;
        }

        length
    }
}

unsafe fn read_input(buf: &mut Vec<c_char>) {
    unsafe {
        read_buffer_into(
            curbuf,
            (*curbuf).b_op_start.lnum,
            (*curbuf).b_op_end.lnum,
            buf,
        );
    }
}

unsafe fn write_output(mut output: *mut c_char, mut remaining: usize, eof: bool) -> usize {
    if output.is_null() {
        return 0;
    }
    unsafe {
        let start = output;
        let mut off: usize = 0;
        while off < remaining {
            if *output.add(off) == CAR
                && off + 1 < remaining
                && *output.add(off + 1) == NL
                && (*curbuf).b_p_bin == 0
            {
                *output.add(off) = NUL;
                let lnum = (*curwin).w_cursor.lnum;
                (*curwin).w_cursor.lnum += 1;
                ml_append(lnum, output, off as c_int + 1, false);
                let skip = off + 2;
                output = output.add(skip);
                remaining -= skip;
                off = 0;
                continue;
            } else if (*output.add(off) == CAR && (*curbuf).b_p_bin == 0) || *output.add(off) == NL
            {
                *output.add(off) = NUL;
                let lnum = (*curwin).w_cursor.lnum;
                (*curwin).w_cursor.lnum += 1;
                ml_append(lnum, output, off as c_int + 1, false);
                let skip = off + 1;
                output = output.add(skip);
                remaining -= skip;
                off = 0;
                continue;
            }

            if *output.add(off) == NUL {
                *output.add(off) = NL;
            }
            off += 1;
        }

        if eof {
            if remaining != 0 {
                let lnum = (*curwin).w_cursor.lnum;
                (*curwin).w_cursor.lnum += 1;
                ml_append(lnum, output, 0, false);
                (*curbuf).b_no_eol_lnum = (*curwin).w_cursor.lnum;
                output = output.add(remaining);
            } else {
                (*curbuf).b_no_eol_lnum = 0;
            }
        }

        ui_flush();

        (output as usize).wrapping_sub(start as usize)
    }
}

unsafe fn shell_write_cb(stream: *mut Stream, status: c_int) {
    unsafe {
        if status != 0 {
            msg_schedule_semsg!(
                gettext(c"E5677: Error writing input to shell-command: %s".as_ptr()),
                uv_err_name(status),
            );
        }
        stream_may_close(stream);
    }
}

unsafe fn shell_xescape_xquote(cmd: *const c_char) -> std::ffi::CString {
    unsafe {
        if *p_sxq == NUL {
            return CStr::from_ptr(cmd).to_owned();
        }

        let sxq_bytes = CStr::from_ptr(p_sxq).to_bytes();
        let owned_ecmd = if *p_sxe != NUL && sxq_bytes == b"(" {
            cstr_take(vim_strsave_escaped_ext(cmd, p_sxe, b'^' as c_char, false))
        } else {
            None
        };
        let ecmd_bytes = owned_ecmd
            .as_ref()
            .map_or_else(|| CStr::from_ptr(cmd).to_bytes(), |s| s.as_bytes());
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
        unsafe {
            assert_eq!(word_length(c"foo bar".as_ptr()), 3);
            assert_eq!(word_length(c"\"foo bar\" baz".as_ptr()), 9);
            assert_eq!(word_length(c"/Program\" \"Files/zsh -f".as_ptr()), 20);
        }
    }

    #[test]
    fn shell_build_argv_and_argv_to_str() {
        unsafe {
            let save_sh = p_sh;
            let save_shcf = p_shcf;
            let save_sxq = p_sxq;
            let save_sxe = p_sxe;

            p_sh = c"/bin/sh".as_ptr().cast_mut();
            p_shcf = c"-c".as_ptr().cast_mut();
            p_sxq = c"".as_ptr().cast_mut();
            p_sxe = c"".as_ptr().cast_mut();

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

            p_sh = c"/Program\" \"Files/zsh -f".as_ptr().cast_mut();
            p_shcf = c"-x -o \"sh word split\" \"-\"c".as_ptr().cast_mut();
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

            p_sh = c"/bin/sh".as_ptr().cast_mut();
            p_shcf = c"-c".as_ptr().cast_mut();
            p_sxq = c"(".as_ptr().cast_mut();
            p_sxe = c"\"&|<>()@^".as_ptr().cast_mut();
            assert_eq!(
                collect_argv(c"echo &|<>()@^".as_ptr(), ptr::null()),
                vec!["/bin/sh", "-c", "(echo ^&^|^<^>^(^)^@^^)"]
            );

            p_sxq = c"\"(".as_ptr().cast_mut();
            assert_eq!(
                collect_argv(c"echo -n some text".as_ptr(), ptr::null()),
                vec!["/bin/sh", "-c", "\"(echo -n some text)\""]
            );

            p_sxq = c"\"".as_ptr().cast_mut();
            p_sxe = c"".as_ptr().cast_mut();
            assert_eq!(
                collect_argv(c"echo -n some text".as_ptr(), ptr::null()),
                vec!["/bin/sh", "-c", "\"echo -n some text\""]
            );

            let argv_arr = [c"foo".to_owned(), c"".to_owned(), c"bar".to_owned()];
            let s = shell_argv_to_str(&argv_arr);
            assert_eq!(
                CStr::from_ptr(s.as_ptr()).to_str().unwrap(),
                "'foo' '' 'bar'"
            );

            p_sh = save_sh;
            p_shcf = save_shcf;
            p_sxq = save_sxq;
            p_sxe = save_sxe;
        }
    }
}
