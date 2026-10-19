//! Functions for Vim help (`src/nvim/help.c`).

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
use crate::globals::*;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::command::exarg_T;
use crate::api::private::validate::{Object, ObjectData, ObjectType};
use crate::api::tabpage::win_T;
use crate::clipboard::{TypvalVval, VarType, typval_T};
use crate::eval::deprecated::buf_T;
use crate::map_glyph_cache::NvimString;
use crate::memory::{xfree, xstrdup};

pub type colnr_T = i32;
pub type linenr_T = i32;
pub type OptIndex = c_int;
pub type OptInt = i64;
pub type scid_T = i32;

const NUL: c_char = 0;
const FAIL: c_int = 0;
const OK: c_int = 1;
const MAXCOL: c_int = 0x7fffffff;
const IOSIZE: usize = 1025;

const WSP_TOP: c_int = 0x08;
const WSP_BOT: c_int = 0x10;
const WSP_HELP: c_int = 0x20;

const ECMD_LASTL: linenr_T = 0;
const ECMD_HIDE: c_int = 0x01;
const ECMD_SET_HELP: c_int = 0x02;

const CMOD_KEEPALT: c_int = 0x0100;

const DT_HELP: c_int = 8;
const TAG_HELP: c_int = 1;
const TAG_NAMES: c_int = 2;
const TAG_REGEXP: c_int = 4;
const TAG_VERBOSE: c_int = 32;
const TAG_KEEP_LANG: c_int = 128;
const TAG_NO_TAGFUNC: c_int = 256;
const TAG_MANY: c_int = 300;

const OPT_LOCAL: c_int = 0x02;
const kOptBuftype: OptIndex = 29;
const kOptFoldmethod: OptIndex = 109;
const kOptIskeyword: OptIndex = 155;

const kBoolVarFalse: i64 = 0;
const kBoolVarTrue: i64 = 1;
const kSpecialVarNull: i64 = 0;

unsafe extern "C" {
    static mut p_hf: *mut c_char;
    static mut p_sb: c_int;
    static mut p_hh: OptInt;
    static mut p_hlg: *mut c_char;
    static mut IObuff: [c_char; IOSIZE];

    fn nlua_call_typval(
        module: *const c_char,
        func: *const c_char,
        argvars: *mut typval_T,
        rettv: *mut typval_T,
    );
    fn tv_clear(tv: *mut typval_T);
    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
    fn smsg(hl_id: c_int, s: *const c_char, ...) -> c_int;
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn FreeWild(count: c_int, files: *mut *mut c_char);
    fn bt_help(buf: *const buf_T) -> bool;
    fn win_enter(wp: *mut win_T, undo_sync: bool);
    fn os_fopen(path: *const c_char, flags: *const c_char) -> *mut c_void;
    fn fclose(stream: *mut c_void) -> c_int;
    fn win_split(size: c_int, flags: c_int) -> c_int;
    fn win_setheight(height: c_int);
    fn do_ecmd(
        fnum: c_int,
        ffname: *mut c_char,
        sfname: *mut c_char,
        eap: *mut exarg_T,
        newlnum: linenr_T,
        flags: c_int,
        oldwin: *mut win_T,
    ) -> c_int;
    fn do_tag(
        eap: *mut exarg_T,
        tag: *mut c_char,
        type_: c_int,
        count: c_int,
        forceit: c_int,
        verbose: bool,
    );
    fn buflist_findnr(nr: c_int) -> *mut buf_T;
    fn wipe_buffer(buf: *mut buf_T, aucmd: bool);
    fn win_close(win: *mut win_T, free_buf: bool, force: bool) -> c_int;
    fn find_tags(
        pat: *mut c_char,
        num_matches: *mut c_int,
        matchesp: *mut *mut *mut c_char,
        flags: c_int,
        mincount: c_int,
        buf_ffname: *mut c_char,
    ) -> c_int;
    fn qsort(
        base: *mut c_void,
        num: usize,
        size: usize,
        compar: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int,
    );
    fn xstrlcpy(dst: *mut c_char, src: *const c_char, dsize: usize) -> usize;
    fn set_option_direct(opt_idx: OptIndex, value: Object, opt_flags: c_int, set_sid: scid_T);
    fn check_buf_options(buf: *mut buf_T);
    fn buf_init_isk_chartab(buf: *mut buf_T) -> c_int;
    fn set_buflisted(on: c_int);
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn skipwhite(q: *const c_char) -> *mut c_char;
}

#[inline]
const fn ascii_iswhite(c: c_char) -> bool {
    c == b' ' as c_char || c == b'\t' as c_char
}

#[inline]
const fn ascii_isalpha(c: c_char) -> bool {
    (c as u8).is_ascii_alphabetic()
}

#[inline]
const fn ascii_isalnum(c: c_char) -> bool {
    (c as u8).is_ascii_alphanumeric()
}

#[inline]
unsafe fn cstr_as_obj(s: *const c_char) -> Object {
    unsafe {
        let len = if s.is_null() {
            0
        } else {
            CStr::from_ptr(s).to_bytes().len()
        };
        Object {
            obj_type: ObjectType::kObjectTypeString,
            data: ObjectData {
                string: NvimString {
                    data: s.cast_mut(),
                    size: len,
                },
            },
        }
    }
}

/// `:help`: open a read-only window on a help file (`ex_help`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_help(eap: *mut exarg_T) {
    unsafe {
        let mut arg: *mut c_char;
        let mut wp: *mut win_T;
        let mut num_matches: c_int = 0;
        let mut matches: *mut *mut c_char = ptr::null_mut();
        let mut empty_fnum: c_int = 0;
        let mut alt_fnum: c_int = 0;
        let old_KeyTyped = KeyTyped;

        if !eap.is_null() {
            arg = (*eap).arg;
            while *arg != NUL {
                if *arg == b'\n' as c_char
                    || *arg == b'\r' as c_char
                    || (*arg == b'|' as c_char
                        && *arg.add(1) != NUL
                        && *arg.add(1) != b'|' as c_char)
                {
                    *arg = NUL;
                    arg = arg.add(1);
                    (*eap).nextcmd = arg;
                    break;
                }
                arg = arg.add(1);
            }
            arg = (*eap).arg;

            if (*eap).skip != 0 {
                return;
            }
        } else {
            arg = c"".as_ptr().cast_mut();
        }

        // remove trailing blanks
        let arg_len = CStr::from_ptr(arg).to_bytes().len();
        if arg_len > 0 {
            let mut p = arg.add(arg_len - 1);
            while p > arg && ascii_iswhite(*p) && *p.sub(1) != b'\\' as c_char {
                *p = NUL;
                p = p.sub(1);
            }
        }

        // Check for a specified language
        let lang = check_help_lang(arg);

        // ":help!" (bang, no args).
        let helpbang = !eap.is_null() && (*eap).forceit != 0 && *arg == NUL;

        // When no argument given go to the index.
        if *arg == NUL && !helpbang {
            arg = c"help.txt".as_ptr().cast_mut();
        }

        // ":help!" (bang, no args): DWIM help, resolve best tag at cursor via Lua.
        let mut allocated_arg: *mut c_char = ptr::null_mut();
        if helpbang {
            let mut no_args = [typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            }];
            let mut rettv = typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            };
            nlua_call_typval(
                c"vim._core.help".as_ptr(),
                c"resolve_tag".as_ptr(),
                no_args.as_mut_ptr(),
                &raw mut rettv,
            );
            if rettv.v_type == VarType::VAR_STRING
                && !rettv.vval.v_string.is_null()
                && *rettv.vval.v_string != NUL
            {
                allocated_arg = rettv.vval.v_string;
                arg = allocated_arg;
            } else {
                tv_clear(&raw mut rettv);
                emsg(gettext((&raw const e_noident).cast()));
                return;
            }
        }

        // Check if there is a match for the argument.
        let mut n = find_help_tags(
            arg,
            &raw mut num_matches,
            &raw mut matches,
            !eap.is_null() && (*eap).forceit != 0,
        );

        let mut i: c_int = 0;
        if n != FAIL && !lang.is_null() {
            let lang_bytes = CStr::from_ptr(lang).to_bytes();
            while i < num_matches {
                let m = *matches.add(i as usize);
                let m_bytes = CStr::from_ptr(m).to_bytes();
                let len = m_bytes.len();
                if len > 3
                    && m_bytes[len - 3] == b'@'
                    && m_bytes[len - 2..].eq_ignore_ascii_case(lang_bytes)
                {
                    break;
                }
                i += 1;
            }
        }
        if i >= num_matches || n == FAIL {
            if !lang.is_null() {
                semsg(gettext(c"E661: No '%s' help for %s".as_ptr()), lang, arg);
            } else {
                semsg(gettext(c"E149: No help for %s".as_ptr()), arg);
            }
            if n != FAIL {
                FreeWild(num_matches, matches);
            }
            xfree(allocated_arg.cast());
            return;
        }

        // The first match (in the requested language) is the best match.
        let tag = xstrdup(*matches.add(i as usize));
        FreeWild(num_matches, matches);

        // Re-use an existing help window or open a new one.
        let mut early_ret = false;
        if !bt_help((*curwin).w_buffer) || cmdmod.cmod_tab != 0 {
            if cmdmod.cmod_tab != 0 {
                wp = ptr::null_mut();
            } else {
                wp = ptr::null_mut();
                let mut wp2 = firstwin;
                while !wp2.is_null() {
                    if bt_help((*wp2).w_buffer)
                        && !(*wp2).w_config.hide
                        && (*wp2).w_config.focusable
                    {
                        wp = wp2;
                        break;
                    }
                    wp2 = (*wp2).w_next;
                }
            }
            if !wp.is_null() && (*(*wp).w_buffer).b_nwindows > 0 {
                win_enter(wp, true);
            } else {
                let helpfd = os_fopen(p_hf, c"rb".as_ptr());
                if helpfd.is_null() {
                    smsg(0, gettext(c"Help file \"%s\" not found".as_ptr()), p_hf);
                    early_ret = true;
                } else {
                    fclose(helpfd);

                    n = WSP_HELP;
                    if cmdmod.cmod_split == 0
                        && (*curwin).w_width != Columns
                        && (*curwin).w_width < 80
                    {
                        n |= if p_sb != 0 { WSP_BOT } else { WSP_TOP };
                    }
                    if win_split(0, n) == FAIL {
                        early_ret = true;
                    } else {
                        if ((*curwin).w_height as OptInt) < p_hh {
                            win_setheight(p_hh as c_int);
                        }

                        alt_fnum = (*curbuf).b_fnum;
                        do_ecmd(
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ECMD_LASTL,
                            ECMD_HIDE + ECMD_SET_HELP,
                            ptr::null_mut(),
                        );

                        if (cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
                            (*curwin).w_alt_fnum = alt_fnum;
                        }
                        empty_fnum = (*curbuf).b_fnum;
                    }
                }
            }
        }

        if !early_ret {
            restart_edit = 0;
            KeyTyped = old_KeyTyped;

            do_tag(ptr::null_mut(), tag, DT_HELP, 1, 0, true);

            if empty_fnum != 0 && (*curbuf).b_fnum != empty_fnum {
                let buf = buflist_findnr(empty_fnum);
                if !buf.is_null() && (*buf).b_nwindows == 0 {
                    wipe_buffer(buf, true);
                }
            }

            if alt_fnum != 0
                && (*curwin).w_alt_fnum == empty_fnum
                && (cmdmod.cmod_flags & CMOD_KEEPALT) == 0
            {
                (*curwin).w_alt_fnum = alt_fnum;
            }
        }

        xfree(tag.cast());
        xfree(allocated_arg.cast());
    }
}

/// `:helpclose`: Close one help window (`ex_helpclose`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_helpclose(eap: *mut exarg_T) {
    unsafe {
        let mut win = firstwin;
        while !win.is_null() {
            if bt_help((*win).w_buffer) {
                win_close(win, false, (*eap).forceit != 0);
                return;
            }
            win = (*win).w_next;
        }
    }
}

/// Search for a language specifier in the form `@xx` (`check_help_lang`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_help_lang(arg: *mut c_char) -> *mut c_char {
    unsafe {
        let len = CStr::from_ptr(arg).to_bytes().len();
        if len >= 3
            && *arg.add(len - 3) == b'@' as c_char
            && ascii_isalpha(*arg.add(len - 2))
            && ascii_isalpha(*arg.add(len - 1))
        {
            *arg.add(len - 3) = NUL;
            return arg.add(len - 2);
        }
        ptr::null_mut()
    }
}

/// Return a heuristic indicating how well the given string matches (`help_heuristic`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn help_heuristic(
    matched_string: *mut c_char,
    mut offset: c_int,
    wrong_case: bool,
) -> c_int {
    unsafe {
        let mut num_letters: c_int = 0;
        let mut p = matched_string;
        while *p != NUL {
            if ascii_isalnum(*p) {
                num_letters += 1;
            }
            p = p.add(1);
        }

        if offset > 0
            && ascii_isalnum(*matched_string.add(offset as usize))
            && ascii_isalnum(*matched_string.add((offset - 1) as usize))
        {
            offset += 10000;
        } else if offset > 2 {
            offset *= 200;
        }
        if wrong_case {
            offset += 5000;
        }
        if *matched_string == b'+' as c_char && *matched_string.add(1) != NUL {
            offset += 100;
        }
        let len = CStr::from_ptr(matched_string).to_bytes().len() as c_int;
        100 * num_letters + len + offset
    }
}

unsafe extern "C" fn help_compare(s1: *const c_void, s2: *const c_void) -> c_int {
    unsafe {
        let str1 = *s1.cast::<*const c_char>();
        let str2 = *s2.cast::<*const c_char>();
        let b1 = CStr::from_ptr(str1).to_bytes();
        let b2 = CStr::from_ptr(str2).to_bytes();
        let p1 = CStr::from_ptr(str1.add(b1.len() + 1)).to_bytes();
        let p2 = CStr::from_ptr(str2.add(b2.len() + 1)).to_bytes();

        match p1.cmp(p2) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => match b1.cmp(b2) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Equal => 0,
            },
        }
    }
}

/// Find all help tags matching `arg`, sort them and return in `matches[]` (`find_help_tags`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn find_help_tags(
    arg: *const c_char,
    num_matches: *mut c_int,
    matches: *mut *mut *mut c_char,
    keep_lang: bool,
) -> c_int {
    unsafe {
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_STRING,
                v_lock: 0,
                vval: TypvalVval {
                    v_string: arg.cast_mut(),
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        let mut rettv = typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        };
        nlua_call_typval(
            c"vim._core.help".as_ptr(),
            c"escape_subject".as_ptr(),
            tv_args.as_mut_ptr(),
            &raw mut rettv,
        );
        if rettv.v_type != VarType::VAR_STRING || rettv.vval.v_string.is_null() {
            tv_clear(&raw mut rettv);
            return FAIL;
        }
        xstrlcpy((&raw mut IObuff).cast(), rettv.vval.v_string, IOSIZE);
        tv_clear(&raw mut rettv);

        *matches = ptr::null_mut();
        *num_matches = 0;
        let mut flags = TAG_HELP | TAG_REGEXP | TAG_NAMES | TAG_VERBOSE | TAG_NO_TAGFUNC;
        if keep_lang {
            flags |= TAG_KEEP_LANG;
        }
        if find_tags(
            (&raw mut IObuff).cast(),
            num_matches,
            matches,
            flags,
            MAXCOL,
            ptr::null_mut(),
        ) == OK
            && *num_matches > 0
        {
            qsort(
                (*matches).cast(),
                *num_matches as usize,
                size_of::<*mut c_char>(),
                help_compare,
            );
            while *num_matches > TAG_MANY {
                *num_matches -= 1;
                xfree((*(*matches).add(*num_matches as usize)).cast());
            }
        }
        OK
    }
}

/// Cleanup matches for help tags (`cleanup_help_tags`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn cleanup_help_tags(num_file: c_int, file: *mut *mut c_char) {
    unsafe {
        let mut buf = [0u8; 3];
        let has_default_lang =
            *p_hlg != NUL && (*p_hlg != b'e' as c_char || *p_hlg.add(1) != b'n' as c_char);
        if has_default_lang {
            buf[0] = b'@';
            buf[1] = *p_hlg as u8;
            buf[2] = *p_hlg.add(1) as u8;
        }

        for i in 0..num_file {
            let fi_ptr = *file.add(i as usize);
            let fi_bytes = CStr::from_ptr(fi_ptr).to_bytes();
            if fi_bytes.len() <= 3 {
                continue;
            }
            let len = fi_bytes.len() - 3;
            if &fi_bytes[len..] == b"@en" {
                let mut found_other = false;
                for j in 0..num_file {
                    if j != i {
                        let fj_bytes = CStr::from_ptr(*file.add(j as usize)).to_bytes();
                        if fj_bytes.len() == len + 3 && fi_bytes[..=len] == fj_bytes[..=len] {
                            found_other = true;
                            break;
                        }
                    }
                }
                if !found_other {
                    *fi_ptr.add(len) = NUL;
                }
            }
        }

        if has_default_lang {
            for i in 0..num_file {
                let fi_ptr = *file.add(i as usize);
                let fi_bytes = CStr::from_ptr(fi_ptr).to_bytes();
                if fi_bytes.len() <= 3 {
                    continue;
                }
                let len = fi_bytes.len() - 3;
                if fi_bytes[len..] == buf {
                    *fi_ptr.add(len) = NUL;
                }
            }
        }
    }
}

/// Called when starting to edit a buffer for a help file (`prepare_help_buffer`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn prepare_help_buffer() {
    unsafe {
        (*curbuf).b_help = true;
        set_option_direct(kOptBuftype, cstr_as_obj(c"help".as_ptr()), OPT_LOCAL, 0);

        let p = c"!-~,^*,^|,^\",192-255";
        if CStr::from_ptr((*curbuf).b_p_isk) != p {
            set_option_direct(kOptIskeyword, cstr_as_obj(p.as_ptr()), OPT_LOCAL, 0);
            check_buf_options(curbuf);
            buf_init_isk_chartab(curbuf);
        }

        set_option_direct(
            kOptFoldmethod,
            cstr_as_obj(c"manual".as_ptr()),
            OPT_LOCAL,
            0,
        );

        (*curbuf).b_p_ts = 8;
        (*curwin).w_onebuf_opt.wo_list = 0;

        (*curbuf).b_p_ma = 0;
        (*curbuf).b_p_bin = 0;
        (*curwin).w_onebuf_opt.wo_nu = 0;
        (*curwin).w_onebuf_opt.wo_rnu = 0;
        (*curwin).w_onebuf_opt.wo_scb = 0;
        (*curwin).w_onebuf_opt.wo_crb = 0;
        (*curwin).w_onebuf_opt.wo_arab = 0;
        (*curwin).w_onebuf_opt.wo_rl = 0;
        (*curwin).w_onebuf_opt.wo_fen = 0;
        (*curwin).w_onebuf_opt.wo_diff = 0;
        (*curwin).w_onebuf_opt.wo_spell = 0;

        set_buflisted(0);
    }
}

/// Populate `*local-additions*` in `help.txt` (`get_local_additions`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn get_local_additions() {
    unsafe {
        let mut no_args = [typval_T {
            v_type: VarType::VAR_UNKNOWN,
            v_lock: 0,
            vval: TypvalVval { v_number: 0 },
        }];
        nlua_call_typval(
            c"vim._core.help".as_ptr(),
            c"local_additions".as_ptr(),
            no_args.as_mut_ptr(),
            ptr::null_mut(),
        );
    }
}

/// `:exusage` (`ex_exusage`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_exusage(_eap: *mut exarg_T) {
    unsafe {
        do_cmdline_cmd(c"help ex-cmd-index".as_ptr());
    }
}

/// `:viusage` (`ex_viusage`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_viusage(_eap: *mut exarg_T) {
    unsafe {
        do_cmdline_cmd(c"help normal-index".as_ptr());
    }
}

/// `:helptags` (`ex_helptags`).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_helptags(eap: *mut exarg_T) {
    unsafe {
        let mut add_help_tags = false;

        let arg_bytes = CStr::from_ptr((*eap).arg).to_bytes();
        if arg_bytes.len() > 3 && &arg_bytes[..3] == b"++t" && ascii_iswhite(*(*eap).arg.add(3)) {
            add_help_tags = true;
            (*eap).arg = skipwhite((*eap).arg.add(3));
        }

        let mut tv_args = [
            if CStr::from_ptr((*eap).arg).to_bytes() == b"ALL" {
                typval_T {
                    v_type: VarType::VAR_SPECIAL,
                    v_lock: 0,
                    vval: TypvalVval {
                        v_number: kSpecialVarNull,
                    },
                }
            } else {
                typval_T {
                    v_type: VarType::VAR_STRING,
                    v_lock: 0,
                    vval: TypvalVval {
                        v_string: (*eap).arg,
                    },
                }
            },
            typval_T {
                v_type: VarType::VAR_BOOL,
                v_lock: 0,
                vval: TypvalVval {
                    v_number: if add_help_tags {
                        kBoolVarTrue
                    } else {
                        kBoolVarFalse
                    },
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        nlua_call_typval(
            c"vim._core.help".as_ptr(),
            c"gen_tags".as_ptr(),
            tv_args.as_mut_ptr(),
            ptr::null_mut(),
        );
    }
}
