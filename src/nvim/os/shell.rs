//! Shell execution and wildcard expansion (`src/nvim/os/shell.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::errors::*;
use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::ptr;

use crate::api::tabpage::win_T;
use crate::eval::deprecated::buf_T;
use crate::event::r#loop::{Loop, loop_poll_events, loop_queues};
use crate::event::multiqueue::{
    EVENT_HANDLER_MAX_ARGC, Event, multiqueue_empty, multiqueue_free, multiqueue_new_child,
    multiqueue_put_event,
};
use crate::event::proc::{
    Proc, proc_err, proc_header, proc_in, proc_out, proc_spawn, proc_stop, proc_tail, proc_wait,
};
use crate::event::rstream::{RStream, rstream_init, rstream_start, stream_read_cb};
use crate::event::stream::{Stream, stream_fields, stream_may_close};
use crate::event::wstream::{
    wstream_init, wstream_new_buffer, wstream_set_write_cb, wstream_write,
};
use crate::kvec::KVec;
use crate::map_glyph_cache::NvimString;
use crate::math::{FAIL, OK};
use crate::memory::{xcalloc, xfree, xmalloc, xmemdupz, xstrdup, xstrlcat, xstrlcpy};
use crate::os::signal::{signal_accept_deadly, signal_reject_deadly};
use crate::os::time::{os_delay, os_hrtime};
use crate::profile::{prof_child_enter, prof_child_exit};

pub type FILE = c_void;
pub type OptInt = i64;
pub type proftime_T = u64;
pub type linenr_T = i32;
pub type varnumber_T = i64;
pub type StringBuilder = KVec<c_char>;

const NUL: c_char = 0;
const TAB: c_char = b'\t' as c_char;
const NL: c_char = b'\n' as c_char;
const CAR: c_char = b'\r' as c_char;

const MAXPATHL: usize = 4096;
const OUT_DATA_THRESHOLD: usize = 1024 * 10;
const MAX_CHUNK_SIZE: usize = OUT_DATA_THRESHOLD / 2;
const NS_PER_SEC: u64 = 1_000_000_000;

const SHELL_SPECIAL: &CStr = c"\t \"&'$;<>()\\|\n";

const kShellOptFilter: c_int = 1;
const kShellOptExpand: c_int = 2;
const kShellOptDoOut: c_int = 4;
const kShellOptSilent: c_int = 8;
const kShellOptRead: c_int = 16;
const kShellOptWrite: c_int = 32;
const kShellOptHideMess: c_int = 64;

const EW_DIR: c_int = 0x01;
const EW_FILE: c_int = 0x02;
const EW_NOTFOUND: c_int = 0x04;
const EW_SILENT: c_int = 0x20;
const EW_EXEC: c_int = 0x40;
const EW_KEEPDOLLAR: c_int = 0x800;
const EW_SHELLCMD: c_int = 0x2000;

const STYLE_ECHO: c_int = 0;
const STYLE_GLOB: c_int = 1;
const STYLE_VIMGLOB: c_int = 2;
const STYLE_PRINT: c_int = 3;
const STYLE_BT: c_int = 4;
const STYLE_GLOBSTAR: c_int = 5;

const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

const STDOUT_FILENO: c_int = 1;
const STDERR_FILENO: c_int = 2;

const MODE_EXTERNCMD: c_int = 0x5000;
const PROF_YES: c_int = 1;
const VV_SHELL_ERROR: c_int = 6;
const kUIMessages: c_int = 4;
const HLF_SE: c_int = 72;
const HLF_SO: c_int = 73;
const kProcTypeUv: c_int = 0;

#[cfg(windows)]
const CP_UTF8: u32 = 65001;

#[repr(C, align(8))]
struct LibuvProcBuf([u8; 2048]);

static mut did_find_nul: bool = false;
static mut throttle_started: u64 = 0;
static mut throttle_received: usize = 0;
static mut throttle_visit: usize = 0;
static mut throttle_pulse_msg: [c_char; 4] = [b' ' as c_char, b' ' as c_char, b' ' as c_char, NUL];

static mut last_skipped: [c_char; MAX_CHUNK_SIZE] = [0; MAX_CHUNK_SIZE];
static mut last_skipped_len: usize = 0;

#[inline]
unsafe fn c_strlen(s: *const c_char) -> usize {
    unsafe { CStr::from_ptr(s).to_bytes().len() }
}

unsafe extern "C" {
    static mut main_loop: [u8; 0];
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut State: c_int;
    static mut sandbox: c_int;
    static mut secure: c_int;
    static mut cmdline_row: c_int;
    static mut Rows: c_int;
    static mut emsg_silent: c_int;
    static mut do_profiling: c_int;
    static mut no_check_timestamps: c_int;
    static mut msg_no_more: bool;
    static mut lines_left: c_int;
    static mut got_int: bool;
    static mut no_wait_return: c_int;

    static mut p_sh: *mut c_char;
    static mut p_shcf: *mut c_char;
    static mut p_sxq: *mut c_char;
    static mut p_sxe: *mut c_char;
    static mut p_verbose: OptInt;

    static utf8len_tab_zero: [u8; 256];

    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn smsg(attr: c_int, fmt: *const c_char, ...) -> c_int;
    fn msg(s: *const c_char, attr: c_int) -> bool;
    fn msg_start();
    fn msg_end();
    fn msg_putchar(c: c_int);
    fn msg_puts(s: *const c_char);
    fn msg_outnum(n: i64);
    fn msg_outtrans(str: *const c_char, attr: c_int, ischanged: bool) -> c_int;
    fn msg_sb_eol();
    fn msg_ext_set_kind(msg_kind: *const c_char);
    fn msg_ext_set_append(append: bool);
    fn msg_ext_no_fast();
    fn msg_multiline(
        str: NvimString,
        attr: c_int,
        check_int: bool,
        hist: bool,
        need_clear: *mut bool,
    );
    fn msg_schedule_semsg(fmt: *const c_char, ...);
    fn verbose_enter();
    fn verbose_leave();

    fn ui_has(ext: c_int) -> bool;
    fn ui_busy_start();
    fn ui_busy_stop();
    fn ui_flush();

    fn backslash_halve(p: *mut c_char);
    fn path_has_wildcard(p: *const c_char, all: bool) -> bool;
    fn path_tail(fname: *const c_char) -> *mut c_char;
    fn add_pathsep(p: *mut c_char) -> bool;
    #[cfg(unix)]
    fn invocation_path_tail(invocation: *const c_char, len: *mut usize) -> *const c_char;
    fn vim_strchr(string: *const c_char, c: c_int) -> *mut c_char;
    fn vim_strnsave_unquoted(string: *const c_char, length: usize) -> *mut c_char;
    fn vim_strsave_escaped_ext(
        string: *const c_char,
        esc_chars: *const c_char,
        cc: c_char,
        bsl: bool,
    ) -> *mut c_char;
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;

    fn check_secure() -> bool;
    fn vim_tempname() -> *mut c_char;
    fn make_filter_cmd(
        cmd: *mut c_char,
        itmp: *mut c_char,
        otmp: *mut c_char,
        do_in: bool,
    ) -> *mut c_char;
    fn tag_freematch();
    fn set_vim_var_nr(idx: c_int, val: varnumber_T);
    fn read_buffer_into(buf: *mut buf_T, start: linenr_T, end: linenr_T, sb: *mut StringBuilder);
    fn ml_append(lnum: linenr_T, line: *mut c_char, len: c_int, newfile: bool) -> c_int;
    fn utfc_ptr2len_len(p: *const c_char, size: c_int) -> c_int;

    fn os_remove(path: *const c_char) -> c_int;
    fn os_path_exists(path: *const c_char) -> bool;
    fn os_isdir(name: *const c_char) -> bool;
    fn os_can_exe(name: *const c_char, abspath: *mut *mut c_char, use_path: bool) -> bool;
    fn os_fopen(path: *const c_char, flags: *const c_char) -> *mut FILE;

    #[cfg(windows)]
    fn os_getenv(name: *const c_char) -> *mut c_char;
    #[cfg(windows)]
    fn os_setenv(name: *const c_char, value: *const c_char, overwrite: c_int) -> c_int;
    #[cfg(windows)]
    fn restore_env_var(name: *const c_char, old_value: *mut c_char, must_free: bool);

    fn uv_strerror(err: c_int) -> *const c_char;
    fn uv_err_name(err: c_int) -> *const c_char;

    fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
    fn fseek(stream: *mut FILE, offset: c_long, whence: c_int) -> c_int;
    fn ftell(stream: *mut FILE) -> c_long;
    fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    fn fclose(stream: *mut FILE) -> c_int;
    fn strcat(dest: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleOutputCP() -> u32;
    fn GetConsoleCP() -> u32;
    fn SetConsoleOutputCP(wCodePageID: u32) -> c_int;
    fn SetConsoleCP(wCodePageID: u32) -> c_int;
}

#[inline]
unsafe fn main_loop_ptr() -> *mut Loop {
    (&raw mut main_loop).cast()
}

#[inline]
fn ascii_iswhite(c: c_char) -> bool {
    c == b' ' as c_char || c == TAB
}

#[inline]
unsafe fn skipwhite(mut p: *const c_char) -> *mut c_char {
    unsafe {
        while ascii_iswhite(*p) {
            p = p.add(1);
        }
        p.cast_mut()
    }
}

unsafe fn save_patterns(
    num_pat: c_int,
    pat: *mut *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
) {
    unsafe {
        let out: *mut *mut c_char = xmalloc(num_pat as usize * size_of::<*mut c_char>()).cast();
        *file = out;
        for i in 0..(num_pat as usize) {
            let s = xstrdup(*pat.add(i));
            backslash_halve(s);
            *out.add(i) = s;
        }
        *num_file = num_pat;
    }
}

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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_expand_wildcards(
    num_pat: c_int,
    pat: *mut *mut c_char,
    num_file: *mut c_int,
    file: *mut *mut *mut c_char,
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

        *num_file = 0;
        *file = ptr::null_mut();

        if !have_wildcard(num_pat, pat) {
            save_patterns(num_pat, pat, num_file, file);
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

        let tempname = vim_tempname();
        if tempname.is_null() {
            emsg(gettext(e_notmp.as_ptr()));
            return FAIL;
        }

        if num_pat == 1 && **pat == b'`' as c_char {
            let pat0_len = c_strlen(*pat);
            if pat0_len > 2 && *(*pat).add(pat0_len - 1) == b'`' as c_char {
                shell_style = STYLE_BT;
            }
        }
        if shell_style != STYLE_BT {
            let sh_len = c_strlen(p_sh);
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

        let mut len = c_strlen(tempname) + 29;
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

        let command: *mut c_char = xmalloc(len).cast();

        if shell_style == STYLE_BT {
            if is_fish_shell {
                strcpy(command, c"begin; ".as_ptr());
            } else {
                strcpy(command, c"(".as_ptr());
            }
            strcat(command, (*pat).add(1));
            let mut p = command.add(c_strlen(command) - 1);
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

                let mut p = command.add(c_strlen(command));
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

        xfree(command.cast());

        if res != 0 {
            os_remove(tempname);
            xfree(tempname.cast());
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
                save_patterns(num_pat, pat, num_file, file);
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
            xfree(tempname.cast());
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, num_file, file);
                return OK;
            }
            return FAIL;
        }
        let fseek_res = fseek(fd, 0, SEEK_END);
        if fseek_res < 0 {
            xfree(tempname.cast());
            fclose(fd);
            return FAIL;
        }
        let templen = ftell(fd) as i64;
        if templen < 0 {
            xfree(tempname.cast());
            fclose(fd);
            return FAIL;
        }
        len = templen as usize;
        fseek(fd, 0, SEEK_SET);
        let buffer: *mut c_char = xmalloc(len + 1).cast();
        let readlen = fread(buffer.cast(), 1, len, fd);
        fclose(fd);
        os_remove(tempname);
        if readlen != len {
            semsg(gettext(e_cant_read_file_str.as_ptr()), tempname);
            xfree(tempname.cast());
            xfree(buffer.cast());
            return FAIL;
        }
        xfree(tempname.cast());

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
                if len != 0 && (c_strlen(buffer) as c_int) < (len as c_int) {
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
            xfree(buffer.cast());
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, num_file, file);
                return OK;
            }
            return FAIL;
        }
        *num_file = count;
        let file_arr: *mut *mut c_char = xmalloc(size_of::<*mut c_char>() * count as usize).cast();
        *file = file_arr;

        let mut p = buffer;
        for i in 0..(count as usize) {
            *file_arr.add(i) = p;
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

        let mut j: usize = 0;
        for i in 0..(count as usize) {
            let entry = *file_arr.add(i);
            if (flags & EW_NOTFOUND) == 0 && !os_path_exists(entry) {
                continue;
            }

            let dir = os_isdir(entry);
            if (dir && (flags & EW_DIR) == 0) || (!dir && (flags & EW_FILE) == 0) {
                continue;
            }

            if !dir
                && (flags & EW_EXEC) != 0
                && !os_can_exe(entry, ptr::null_mut(), (flags & EW_SHELLCMD) == 0)
            {
                continue;
            }

            let new_entry: *mut c_char = xmalloc(c_strlen(entry) + 1 + usize::from(dir)).cast();
            strcpy(new_entry, entry);
            if dir {
                add_pathsep(new_entry);
            }
            *file_arr.add(j) = new_entry;
            j += 1;
        }
        xfree(buffer.cast());
        *num_file = j as c_int;

        if *num_file == 0 {
            xfree((*file).cast());
            *file = ptr::null_mut();
            if (flags & EW_NOTFOUND) != 0 {
                save_patterns(num_pat, pat, num_file, file);
                return OK;
            }
            return FAIL;
        }

        OK
    }
}

/// Builds the argument vector for running the user-configured 'shell' (p_sh)
/// with an optional command prefixed by 'shellcmdflag' (p_shcf).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shell_build_argv(
    cmd: *const c_char,
    extra_args: *const c_char,
) -> *mut *mut c_char {
    unsafe {
        let argc = tokenize(p_sh, ptr::null_mut())
            + if !cmd.is_null() {
                tokenize(p_shcf, ptr::null_mut())
            } else {
                0
            };
        let rv: *mut *mut c_char = xmalloc((argc + 4) * size_of::<*mut c_char>()).cast();

        let mut i = tokenize(p_sh, rv);

        if !extra_args.is_null() {
            *rv.add(i) = xstrdup(extra_args);
            i += 1;
        }

        if !cmd.is_null() {
            i += tokenize(p_shcf, rv.add(i));
            *rv.add(i) = shell_xescape_xquote(cmd);
            i += 1;
        }

        *rv.add(i) = ptr::null_mut();
        debug_assert!(!(*rv).is_null());

        rv
    }
}

/// Releases the memory allocated by `shell_build_argv`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shell_free_argv(argv: *mut *mut c_char) {
    if argv.is_null() {
        return;
    }
    unsafe {
        let mut p = argv;
        while !(*p).is_null() {
            xfree((*p).cast());
            p = p.add(1);
        }
        xfree(argv.cast());
    }
}

/// Joins shell arguments from `argv` into a new string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn shell_argv_to_str(argv: *mut *mut c_char) -> *mut c_char {
    unsafe {
        let mut n: usize = 0;
        let mut p = argv;
        let maxsize: usize = 256;
        let rv: *mut c_char = xcalloc(maxsize, size_of::<c_char>()).cast();
        if (*p).is_null() {
            return rv;
        }
        while !(*p).is_null() {
            xstrlcat(rv, c"'".as_ptr(), maxsize);
            xstrlcat(rv, *p, maxsize);
            n = xstrlcat(rv, c"' ".as_ptr(), maxsize);
            if n >= maxsize {
                break;
            }
            p = p.add(1);
        }
        if n < maxsize {
            *rv.add(n - 1) = NUL;
        } else {
            *rv.add(maxsize - 4) = b'.' as c_char;
            *rv.add(maxsize - 3) = b'.' as c_char;
            *rv.add(maxsize - 2) = b'.' as c_char;
            *rv.add(maxsize - 1) = NUL;
        }
        rv
    }
}

/// Calls the user-configured 'shell' (p_sh) for running a command or wildcard expansion.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_call_shell(
    cmd: *mut c_char,
    opts: c_int,
    extra_args: *mut c_char,
) -> c_int {
    unsafe {
        let mut input = StringBuilder::new();
        let mut output: *mut c_char = ptr::null_mut();
        let mut output_ptr: *mut *mut c_char = ptr::null_mut();
        let current_state = State;
        let mut forward_output = true;

        signal_reject_deadly();

        if (opts & (kShellOptHideMess | kShellOptExpand)) != 0 {
            forward_output = false;
        } else {
            State = MODE_EXTERNCMD;

            if (opts & kShellOptWrite) != 0 {
                read_input(&raw mut input);
            }

            if (opts & kShellOptRead) != 0 {
                output_ptr = &raw mut output;
                forward_output = false;
            } else if (opts & kShellOptDoOut) != 0 {
                forward_output = false;
            }
        }

        let mut nread: usize = 0;
        let exitcode = do_os_system(
            shell_build_argv(cmd, extra_args),
            input.items,
            input.size,
            output_ptr,
            &raw mut nread,
            emsg_silent != 0,
            forward_output,
        );
        input.destroy();

        if !output.is_null() {
            write_output(output, nread, true);
            xfree(output.cast());
        }

        if emsg_silent == 0 && exitcode != 0 && (opts & kShellOptSilent) == 0 {
            msg_ext_set_kind(c"shell_ret".as_ptr());
            msg_ext_no_fast();
            if !ui_has(kUIMessages) {
                msg_putchar(b'\n' as c_int);
            }
            msg_puts(gettext(c"shell returned ".as_ptr()));
            msg_outnum(i64::from(exitcode));
        }

        State = current_state;
        signal_accept_deadly();

        exitcode
    }
}

/// os_call_shell() wrapper. Handles 'verbose', :profile, and v:shell_error.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn call_shell(
    cmd: *mut c_char,
    opts: c_int,
    extra_shell_arg: *mut c_char,
) -> c_int {
    unsafe {
        let mut wait_time: proftime_T = 0;

        if p_verbose > 3 {
            verbose_enter();
            smsg(
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
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_cmd_output(
    cmd: *mut c_char,
    infile: *mut c_char,
    flags: c_int,
    ret_len: *mut usize,
) -> *mut c_char {
    unsafe {
        let mut buffer: *mut c_char = ptr::null_mut();

        if check_secure() {
            return ptr::null_mut();
        }

        let tempname = vim_tempname();
        if tempname.is_null() {
            emsg(gettext(e_notmp.as_ptr()));
            return ptr::null_mut();
        }

        let command = make_filter_cmd(cmd, infile, tempname, false);

        no_check_timestamps += 1;
        call_shell(
            command,
            kShellOptDoOut | kShellOptExpand | flags,
            ptr::null_mut(),
        );
        no_check_timestamps -= 1;

        xfree(command.cast());

        let fd = os_fopen(tempname, c"rb".as_ptr());

        'done: {
            if fd.is_null() || fseek(fd, 0, SEEK_END) == -1 {
                semsg(gettext(e_cannot_read_from_str_2.as_ptr()), tempname);
                if !fd.is_null() {
                    fclose(fd);
                }
                break 'done;
            }
            let len_l = ftell(fd);
            if len_l == -1 || fseek(fd, 0, SEEK_SET) == -1 {
                semsg(gettext(e_cannot_read_from_str_2.as_ptr()), tempname);
                fclose(fd);
                break 'done;
            }

            let len = len_l as usize;
            buffer = xmalloc(len + 1).cast();
            let i = fread(buffer.cast(), 1, len, fd);
            fclose(fd);
            if i != len {
                semsg(gettext(e_cant_read_file_str.as_ptr()), tempname);
                xfree(buffer.cast());
                buffer = ptr::null_mut();
            } else if ret_len.is_null() {
                for idx in 0..len {
                    if *buffer.add(idx) == NUL {
                        *buffer.add(idx) = 1;
                    }
                }
                *buffer.add(len) = NUL;
            } else {
                *ret_len = len;
            }
        }

        os_remove(tempname);
        xfree(tempname.cast());
        buffer
    }
}

/// os_system - synchronously execute a command in the shell
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn os_system(
    argv: *mut *mut c_char,
    input: *const c_char,
    len: usize,
    output: *mut *mut c_char,
    nread: *mut usize,
) -> c_int {
    unsafe { do_os_system(argv, input, len, output, nread, true, false) }
}

unsafe fn do_os_system(
    argv: *mut *mut c_char,
    input: *const c_char,
    len: usize,
    output: *mut *mut c_char,
    nread: *mut usize,
    silent: bool,
    forward_output: bool,
) -> c_int {
    unsafe {
        let mut exitcode: c_int = -1;

        #[cfg(windows)]
        let (oldval, old_output_cp, old_input_cp) = {
            let ov = os_getenv(c"NoDefaultCurrentDirectoryInExePath".as_ptr());
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

        let mut buf = StringBuilder::new();
        let mut data_cb: stream_read_cb = Some(system_data_cb);
        if !nread.is_null() {
            *nread = 0;
        }

        if forward_output {
            data_cb = Some(out_data_cb);
        } else if output.is_null() {
            data_cb = None;
        }

        let mut prog = [0 as c_char; MAXPATHL];
        xstrlcpy(prog.as_mut_ptr(), *argv, MAXPATHL);

        let mloop = main_loop_ptr();
        let mut uvproc = LibuvProcBuf([0; 2048]);
        let proc_: *mut Proc = (&raw mut uvproc).cast();
        let hdr = proc_header(proc_);
        let ptail = proc_tail(proc_);
        (*hdr).r#type = kProcTypeUv;
        (*hdr).data = (&raw mut buf).cast();
        (*hdr).loop_ = mloop;
        (*hdr).status = -1;
        (*stream_fields(proc_out(proc_).cast())).fd = STDOUT_FILENO;
        (*stream_fields(proc_err(proc_).cast())).fd = STDERR_FILENO;

        let qc = loop_queues(mloop);
        let events = multiqueue_new_child((*qc).events);
        (*ptail).events = events;
        (*hdr).argv = argv;

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

            let s_in = proc_in(proc_);
            let r_out = proc_out(proc_);
            let r_err = proc_err(proc_);

            if has_input {
                wstream_init(s_in, 0);
            }
            rstream_init(r_out);
            rstream_start(r_out, data_cb, (&raw mut buf).cast());
            rstream_init(r_err);
            rstream_start(r_err, data_cb, (&raw mut buf).cast());

            if has_input {
                let input_buffer = wstream_new_buffer(input.cast_mut(), len, 1, None);
                if wstream_write(s_in, input_buffer) != 0 {
                    proc_stop(proc_);
                    break 'end;
                }
                wstream_set_write_cb(s_in, Some(shell_write_cb), ptr::null_mut());
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

            if !output.is_null() {
                debug_assert!(!nread.is_null());
                if buf.size == 0 {
                    *output = ptr::null_mut();
                    *nread = 0;
                    buf.destroy();
                } else {
                    *nread = buf.size;
                    buf.push(NUL);
                    *output = buf.items;
                }
            }

            debug_assert!(multiqueue_empty(events));
        }

        multiqueue_free(events);

        #[cfg(windows)]
        {
            restore_env_var(c"NoDefaultCurrentDirectoryInExePath".as_ptr(), oldval, true);
            SetConsoleOutputCP(old_output_cp);
            SetConsoleCP(old_input_cp);
        }

        exitcode
    }
}

unsafe extern "C" fn system_data_cb(
    _stream: *mut RStream,
    buf: *const c_char,
    count: usize,
    data: *mut c_void,
    _eof: bool,
) -> usize {
    unsafe {
        let dbuf = data.cast::<StringBuilder>();
        (*dbuf).concat_len(buf, count);
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

unsafe extern "C" fn out_data_event(argv: *mut *mut c_void) {
    unsafe {
        let mut need_clear = true;
        let fd = (*argv.add(2) as isize) as c_int;
        let hl = if fd == STDERR_FILENO { HLF_SE } else { HLF_SO };
        msg_ext_set_kind(if fd == STDERR_FILENO {
            c"shell_err".as_ptr()
        } else {
            c"shell_out".as_ptr()
        });
        msg_ext_set_append(true);
        msg_ext_no_fast();
        let str_arg = NvimString {
            data: (*argv).cast(),
            size: *argv.add(1) as usize,
        };
        msg_multiline(str_arg, hl, false, false, &raw mut need_clear);
        xfree(*argv);
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

        let str_copy: *mut c_char = xmemdupz(output.cast(), *count).cast();
        let mut argv = [ptr::null_mut(); EVENT_HANDLER_MAX_ARGC];
        argv[0] = str_copy.cast();
        argv[1] = (*count) as *mut c_void;
        argv[2] = (fd as isize) as *mut c_void;
        if ui_has(kUIMessages) {
            let qc = loop_queues(main_loop_ptr());
            multiqueue_put_event(
                (*qc).fast_events,
                Event {
                    handler: Some(out_data_event),
                    argv,
                },
            );
        } else {
            out_data_event(argv.as_mut_ptr());
        }
    }
}

unsafe extern "C" fn out_data_cb(
    stream: *mut RStream,
    ptr_data: *const c_char,
    mut count: usize,
    _data: *mut c_void,
    eof: bool,
) -> usize {
    unsafe {
        if count > 0 && out_data_decide_throttle(count) {
            out_data_ring(ptr_data, count);
        } else if count > 0 {
            let fd = (*stream_fields(stream.cast())).fd;
            out_data_append_to_screen(ptr_data, &raw mut count, fd, eof);
        }
        count
    }
}

unsafe fn tokenize(str_in: *const c_char, argv: *mut *mut c_char) -> usize {
    unsafe {
        let mut argc: usize = 0;
        let mut p = str_in;

        while *p != NUL {
            let len = word_length(p);

            if !argv.is_null() {
                *argv.add(argc) = vim_strnsave_unquoted(p, len);
            }

            argc += 1;
            p = skipwhite(p.add(len));
        }

        argc
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

unsafe fn read_input(buf: *mut StringBuilder) {
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

unsafe extern "C" fn shell_write_cb(stream: *mut Stream, _data: *mut c_void, status: c_int) {
    unsafe {
        if status != 0 {
            msg_schedule_semsg(
                gettext(c"E5677: Error writing input to shell-command: %s".as_ptr()),
                uv_err_name(status),
            );
        }
        stream_may_close(stream);
    }
}

unsafe fn shell_xescape_xquote(cmd: *const c_char) -> *mut c_char {
    unsafe {
        if *p_sxq == NUL {
            return xstrdup(cmd);
        }

        let sxq_bytes = CStr::from_ptr(p_sxq).to_bytes();
        let mut ecmd = cmd;
        if *p_sxe != NUL && sxq_bytes == b"(" {
            ecmd = vim_strsave_escaped_ext(cmd, p_sxe, b'^' as c_char, false);
        }
        let ncmd_size = c_strlen(ecmd) + sxq_bytes.len() * 2 + 1;
        let ncmd: *mut c_char = xmalloc(ncmd_size).cast();

        if sxq_bytes == b"(" {
            vim_snprintf(ncmd, ncmd_size, c"(%s)".as_ptr(), ecmd);
        } else if sxq_bytes == b"\"(" {
            vim_snprintf(ncmd, ncmd_size, c"\"(%s)\"".as_ptr(), ecmd);
        } else {
            vim_snprintf(ncmd, ncmd_size, c"%s%s%s".as_ptr(), p_sxq, ecmd, p_sxq);
        }

        if ecmd != cmd {
            xfree(ecmd.cast_mut().cast());
        }

        ncmd
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
}
