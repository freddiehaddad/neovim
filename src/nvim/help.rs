//! Functions for Vim help (`src/nvim/help.c`).

use crate::api::private::helpers::cstr_as_obj;
use crate::buffer::{bt_help, buflist_findnr, set_buflisted, wipe_buffer};
use crate::charset::{
    ascii_isalnum, ascii_isalpha, ascii_iswhite, buf_init_isk_chartab, skipwhite,
};
use crate::errors::*;
use crate::eval::K_BOOL_VAR_FALSE_CONST;
use crate::eval::K_BOOL_VAR_TRUE_CONST;
use crate::eval::typval::{TypVal, TypvalVval, VarType, tv_clear};
use crate::ex_cmds::{ECMD_HIDE, ECMD_LASTL, ECMD_SET_HELP, do_ecmd};
use crate::ex_docmd::{CMOD_KEEPALT, Exarg, do_cmdline_cmd};
use crate::fileio::fclose;
use crate::globals::*;
use crate::lua::executor::nlua_call_typval;
use crate::memory::xstrlcpy;
use crate::message::{emsg, semsg, smsg};
use crate::option::{K_OPT_BUFTYPE, OPT_LOCAL, OptInt, set_option_direct};
use crate::option_vars::*;
use crate::optionstr::{OptIndex, check_buf_options};
use crate::os::fs::os_fopen;
use crate::strings::gettext;
use crate::tag::{do_tag, find_tags};
use crate::window::{
    WSP_BOT, WSP_HELP, WSP_TOP, WinInfoInfo, win_close, win_enter, win_setheight, win_split,
};
use std::ffi::CStr;
use std::ptr;

use crate::cmdexpand::TAG_MANY;
use crate::diff::K_OPT_FOLDMETHOD;
use crate::globals::IOSIZE;
use crate::math::OK;

pub(crate) const DT_HELP: i32 = 8;
pub(crate) const TAG_HELP: i32 = 1;
pub(crate) const TAG_NAMES: i32 = 2;
pub(crate) const TAG_REGEXP: i32 = 4;
pub(crate) const TAG_VERBOSE: i32 = 32;
pub(crate) const TAG_KEEP_LANG: i32 = 128;
pub(crate) const TAG_NO_TAGFUNC: i32 = 256;

pub(crate) const K_OPT_ISKEYWORD: OptIndex = 155;

use crate::eval::funcs::K_SPECIAL_VAR_NULL_CONST;

/// `:help`: open a read-only window on a help file (`ex_help`).
pub fn ex_help(eap: &mut Exarg) {
    unsafe {
        let mut wp: *mut WinInfoInfo;
        let mut empty_fnum: i32 = 0;
        let mut alt_fnum: i32 = 0;
        let old_key_typed = g_st().key_typed;

        let mut lang: Option<std::ffi::CString> = None;
        let mut arg_cstr: &CStr = c"";
        if !eap.arg.is_null() {
            let full_len = crate::memory::cstr_len(eap.arg);
            let slice = std::slice::from_raw_parts_mut(eap.arg.cast::<u8>(), full_len + 1);
            for i in 0..full_len {
                let b = slice[i];
                if b == b'\n'
                    || b == b'\r'
                    || (b == b'|' && slice[i + 1] != 0 && slice[i + 1] != b'|')
                {
                    slice[i] = 0;
                    eap.nextcmd = eap.arg.wrapping_add(i + 1);
                    break;
                }
            }

            if eap.skip != 0 {
                return;
            }

            // remove trailing blanks
            let mut end = slice.iter().position(|&b| b == 0).unwrap_or(full_len);
            while end > 1
                && ascii_iswhite(i32::from(slice[end - 1]))
                && slice[end - 2] != b'\\'
            {
                end -= 1;
                slice[end] = 0;
            }

            // Check for a specified language
            lang = check_help_lang(slice).map(CStr::to_owned);
            arg_cstr = CStr::from_bytes_until_nul(slice).unwrap_or(c"");
        }

        // ":help!" (bang, no args).
        let helpbang = eap.forceit != 0 && arg_cstr.to_bytes().is_empty();

        // When no argument given go to the index.
        if arg_cstr.to_bytes().is_empty() && !helpbang {
            arg_cstr = c"help.txt";
        }

        // ":help!" (bang, no args): DWIM help, resolve best tag at cursor via Lua.
        let mut allocated_arg: Option<std::ffi::CString> = None;
        if helpbang {
            let mut no_args = [TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            }];
            let mut rettv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            };
            nlua_call_typval(
                c"vim._core.help",
                c"resolve_tag",
                no_args.as_mut_ptr(),
                Some(&mut rettv),
            );
            if rettv.v_type == VarType::VarString
                && !rettv.vval.v_string().is_null()
                && !crate::memory::cstr_is_empty(rettv.vval.v_string())
            {
                allocated_arg = crate::eval::typval::tv_take_string(rettv.vval.v_string());
                arg_cstr = allocated_arg.as_deref().unwrap();
            } else {
                tv_clear(&mut rettv);
                emsg(gettext(E_NOIDENT));
                return;
            }
        }

        // Check if there is a match for the argument.
        let mut matches: Vec<std::ffi::CString> = Vec::new();
        let found_tags = find_help_tags(arg_cstr, &mut matches, eap.forceit != 0);

        let mut i: usize = 0;
        if found_tags && let Some(ref lang_cstr) = lang {
            let lang_bytes = lang_cstr.to_bytes();
            while i < matches.len() {
                let m_bytes = matches[i].as_bytes();
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
        if i >= matches.len() || !found_tags {
            if let Some(ref lang_cstr) = lang {
                semsg!(
                    gettext(c"E661: No '%s' help for %s"),
                    lang_cstr.as_ptr(),
                    arg_cstr.as_ptr()
                );
            } else {
                semsg!(gettext(c"E149: No help for %s"), arg_cstr.as_ptr());
            }
            return;
        }

        // The first match (in the requested language) is the best match.
        let tag = matches.swap_remove(i);
        drop(matches);

        // Re-use an existing help window or open a new one.
        let mut early_ret = false;
        if !bt_help(curwin_mut().w_buffer.as_ref()) || g_st().cmdmod.cmod_tab != 0 {
            if g_st().cmdmod.cmod_tab != 0 {
                wp = ptr::null_mut();
            } else {
                wp = ptr::null_mut();
                for wp2 in for_all_windows_ptr() {
                    if bt_help((wp2.w_buffer).as_ref())
                        && !wp2.w_config.hide
                        && wp2.w_config.focusable
                    {
                        wp = wp2;
                        break;
                    }
                }
            }
            if !wp.is_null() && (*wp).buf_mut().b_nwindows > 0 {
                win_enter(wp, true);
            } else {
                let helpfd = os_fopen(opt_vars().p_hf, c"rb");
                if helpfd.is_null() {
                    smsg!(0, gettext(c"Help file \"%s\" not found"), opt_vars().p_hf);
                    early_ret = true;
                } else {
                    fclose(helpfd);

                    let mut n = WSP_HELP;
                    if g_st().cmdmod.cmod_split == 0
                        && curwin_mut().w_width != g_st().columns_global
                        && curwin_mut().w_width < 80
                    {
                        n |= if opt_vars().p_sb != 0 {
                            WSP_BOT
                        } else {
                            WSP_TOP
                        };
                    }
                    if !win_split(0, n) {
                        early_ret = true;
                    } else {
                        if (curwin_mut().w_height as OptInt) < opt_vars().p_hh {
                            win_setheight(opt_vars().p_hh as i32);
                        }

                        alt_fnum = curbuf_mut().b_fnum;
                        do_ecmd(
                            0,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            None,
                            ECMD_LASTL,
                            ECMD_HIDE + ECMD_SET_HELP,
                            ptr::null_mut(),
                        );

                        if (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0 {
                            curwin_mut().w_alt_fnum = alt_fnum;
                        }
                        empty_fnum = curbuf_mut().b_fnum;
                    }
                }
            }
        }

        if !early_ret {
            g_st().restart_edit = 0;
            g_st().key_typed = old_key_typed;

            do_tag(None, tag.as_ptr().cast_mut(), DT_HELP, 1, 0, true);

            if empty_fnum != 0 && curbuf_mut().b_fnum != empty_fnum {
                let buf = buflist_findnr(empty_fnum);
                if !buf.is_null() && (*buf).b_nwindows == 0 {
                    wipe_buffer(buf, true);
                }
            }

            if alt_fnum != 0
                && curwin_mut().w_alt_fnum == empty_fnum
                && (g_st().cmdmod.cmod_flags & CMOD_KEEPALT) == 0
            {
                curwin_mut().w_alt_fnum = alt_fnum;
            }
        }
        drop(allocated_arg);
    }
}

/// `:helpclose`: Close one help window (`ex_helpclose`).
pub fn ex_helpclose(eap: &mut Exarg) {
    for win in for_all_windows_ptr() {
        if bt_help(unsafe { (win.w_buffer).as_ref() }) {
            win_close(win, false, eap.forceit != 0);
            return;
        }
    }
}

/// Search for a language specifier in the form `@xx` (`check_help_lang`).
pub fn check_help_lang(arg: &mut [u8]) -> Option<&CStr> {
    let len = arg.iter().position(|&b| b == 0).unwrap_or(arg.len());
    if len >= 3
        && arg[len - 3] == b'@'
        && ascii_isalpha(i32::from(arg[len - 2]))
        && ascii_isalpha(i32::from(arg[len - 1]))
    {
        arg[len - 3] = 0;
        return CStr::from_bytes_until_nul(&arg[len - 2..]).ok();
    }
    None
}

/// Return a heuristic indicating how well the given string matches (`help_heuristic`).
pub fn help_heuristic(matched_string: &[u8], mut offset: i32, wrong_case: bool) -> i32 {
    let mut num_letters: i32 = 0;
    for &p_byte in matched_string {
        if ascii_isalnum(i32::from(p_byte)) {
            num_letters += 1;
        }
    }

    if offset > 0
        && matched_string
            .get(offset as usize)
            .is_some_and(|&b| ascii_isalnum(i32::from(b)))
        && matched_string
            .get((offset - 1) as usize)
            .is_some_and(|&b| ascii_isalnum(i32::from(b)))
    {
        offset += 10000;
    } else if offset > 2 {
        offset *= 200;
    }
    if wrong_case {
        offset += 5000;
    }
    if matched_string.first() == Some(&b'+') && matched_string.get(1).is_some_and(|&b| b != 0) {
        offset += 100;
    }
    let len = matched_string.len() as i32;
    100 * num_letters + len + offset
}

fn help_compare(str1: &[u8], str2: &[u8]) -> std::cmp::Ordering {
    let len1 = str1.iter().position(|&b| b == 0).unwrap_or(str1.len());
    let len2 = str2.iter().position(|&b| b == 0).unwrap_or(str2.len());
    let b1 = &str1[..len1];
    let b2 = &str2[..len2];
    let rest1 = str1.get(len1 + 1..).unwrap_or(&[]);
    let rest2 = str2.get(len2 + 1..).unwrap_or(&[]);
    let pri1_len = rest1.iter().position(|&b| b == 0).unwrap_or(rest1.len());
    let pri2_len = rest2.iter().position(|&b| b == 0).unwrap_or(rest2.len());
    let pri1 = &rest1[..pri1_len];
    let pri2 = &rest2[..pri2_len];

    pri1.cmp(pri2).then_with(|| b1.cmp(b2))
}

/// Find all help tags matching `arg`, sort them and return in `matches` (`find_help_tags`).
pub fn find_help_tags(
    arg: &CStr,
    matches: &mut Vec<std::ffi::CString>,
    keep_lang: bool,
) -> bool {
    let mut tv_args = [
        TypVal {
            v_type: VarType::VarString,
            v_lock: 0,
            vval: TypvalVval::String(arg.as_ptr().cast_mut()),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    let mut rettv = TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    };
    nlua_call_typval(
        c"vim._core.help",
        c"escape_subject",
        tv_args.as_mut_ptr(),
        Some(&mut rettv),
    );
    if rettv.v_type != VarType::VarString || rettv.vval.v_string().is_null() {
        tv_clear(&mut rettv);
        return false;
    }
    xstrlcpy(g_st().iobuff.as_mut_ptr(), rettv.vval.v_string(), IOSIZE);
    tv_clear(&mut rettv);

    matches.clear();
    let mut flags = TAG_HELP | TAG_REGEXP | TAG_NAMES | TAG_VERBOSE | TAG_NO_TAGFUNC;
    if keep_lang {
        flags |= TAG_KEEP_LANG;
    }
    let mut raw_matches: Vec<Vec<u8>> = Vec::new();
    if find_tags(
        g_st().iobuff.as_mut_ptr(),
        &mut raw_matches,
        flags,
        crate::regexp::MAXCOL,
        ptr::null_mut(),
    ) == OK
        && !raw_matches.is_empty()
    {
        raw_matches.sort_unstable_by(|a, b| help_compare(a, b));
        raw_matches.truncate(TAG_MANY as usize);
        matches.reserve_exact(raw_matches.len());
        for m in raw_matches {
            matches.push(crate::memory::cstr_to_cstring(m.as_ptr().cast()));
        }
    }
    true
}

/// Cleanup matches for help tags (`cleanup_help_tags`).
pub fn cleanup_help_tags(file: &mut [std::ffi::CString]) {
    let mut buf = [0u8; 3];
    let has_default_lang = !crate::memory::cstr_is_empty(opt_vars().p_hlg)
        && (crate::memory::cstr_first_byte(opt_vars().p_hlg) != b'e'
            || crate::memory::cstr_byte_at(opt_vars().p_hlg, 1) != b'n');
    if has_default_lang {
        buf[0] = b'@';
        buf[1] = crate::memory::cstr_first_byte(opt_vars().p_hlg);
        buf[2] = crate::memory::cstr_byte_at(opt_vars().p_hlg, 1);
    }

    for i in 0..file.len() {
        let fi_bytes = file[i].as_bytes();
        if fi_bytes.len() <= 3 {
            continue;
        }
        let len = fi_bytes.len() - 3;
        if &fi_bytes[len..] == b"@en" {
            let mut found_other = false;
            for (j, fj) in file.iter().enumerate() {
                if j != i {
                    let fj_bytes = fj.as_bytes();
                    if fj_bytes.len() == len + 3 && fi_bytes[..=len] == fj_bytes[..=len] {
                        found_other = true;
                        break;
                    }
                }
            }
            if !found_other {
                file[i] = crate::memory::cstr_from_bytes(&fi_bytes[..len]);
            }
        }
    }

    if has_default_lang {
        for slot in file.iter_mut() {
            let fi_bytes = slot.as_bytes();
            if fi_bytes.len() <= 3 {
                continue;
            }
            let len = fi_bytes.len() - 3;
            if fi_bytes[len..] == buf {
                *slot = crate::memory::cstr_from_bytes(&fi_bytes[..len]);
            }
        }
    }
}

/// Called when starting to edit a buffer for a help file (`prepare_help_buffer`).
pub fn prepare_help_buffer() {
    curbuf_mut().b_help = true;
    set_option_direct(K_OPT_BUFTYPE, cstr_as_obj(c"help"), OPT_LOCAL, 0);

    let p = c"!-~,^*,^|,^\",192-255";
    if crate::memory::cstr_as_bytes(curbuf_mut().b_p_isk) != p.to_bytes() {
        set_option_direct(K_OPT_ISKEYWORD, cstr_as_obj(p), OPT_LOCAL, 0);
        check_buf_options(curbuf_mut());
        buf_init_isk_chartab(curbuf_mut());
    }

    set_option_direct(K_OPT_FOLDMETHOD, cstr_as_obj(c"manual"), OPT_LOCAL, 0);

    curbuf_mut().b_p_ts = 8;
    curwin_mut().w_onebuf_opt.wo_list = 0;

    curbuf_mut().b_p_ma = 0;
    curbuf_mut().b_p_bin = 0;
    curwin_mut().w_onebuf_opt.wo_nu = 0;
    curwin_mut().w_onebuf_opt.wo_rnu = 0;
    curwin_mut().w_onebuf_opt.wo_scb = 0;
    curwin_mut().w_onebuf_opt.wo_crb = 0;
    curwin_mut().w_onebuf_opt.wo_arab = 0;
    curwin_mut().w_onebuf_opt.wo_rl = 0;
    curwin_mut().w_onebuf_opt.wo_fen = 0;
    curwin_mut().w_onebuf_opt.wo_diff = 0;
    curwin_mut().w_onebuf_opt.wo_spell = 0;

    set_buflisted(0);
}

/// Populate `*local-additions*` in `help.txt` (`get_local_additions`).
pub fn get_local_additions() {
    let mut no_args = [TypVal {
        v_type: VarType::VarUnknown,
        v_lock: 0,
        vval: TypvalVval::Number(0),
    }];
    nlua_call_typval(
        c"vim._core.help",
        c"local_additions",
        no_args.as_mut_ptr(),
        None,
    );
}

/// `:exusage` (`ex_exusage`).
pub fn ex_exusage(_eap: &mut Exarg) {
    do_cmdline_cmd(c"help ex-cmd-index");
}

/// `:viusage` (`ex_viusage`).
pub fn ex_viusage(_eap: &mut Exarg) {
    do_cmdline_cmd(c"help normal-index");
}

/// `:helptags` (`ex_helptags`).
pub fn ex_helptags(eap: &mut Exarg) {
    let mut add_help_tags = false;

    let arg_bytes = crate::memory::cstr_as_bytes(eap.arg);
    if arg_bytes.len() > 3
        && &arg_bytes[..3] == b"++t"
        && ascii_iswhite(crate::memory::cstr_char_at(eap.arg, 3))
    {
        add_help_tags = true;
        eap.arg = skipwhite(eap.arg.wrapping_add(3));
    }

    let mut tv_args = [
        if crate::memory::cstr_as_bytes(eap.arg) == b"ALL" {
            TypVal {
                v_type: VarType::VarSpecial,
                v_lock: 0,
                vval: TypvalVval::Special(K_SPECIAL_VAR_NULL_CONST),
            }
        } else {
            TypVal {
                v_type: VarType::VarString,
                v_lock: 0,
                vval: TypvalVval::String(eap.arg),
            }
        },
        TypVal {
            v_type: VarType::VarBool,
            v_lock: 0,
            vval: TypvalVval::Number(if add_help_tags {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
            }),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    nlua_call_typval(c"vim._core.help", c"gen_tags", tv_args.as_mut_ptr(), None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_help_lang_and_help_heuristic_work() {
        let mut buf = *b"options@en\0";
        let lang = check_help_lang(&mut buf);
        assert_eq!(lang.map(CStr::to_bytes), Some(&b"en"[..]));
        assert_eq!(
            CStr::from_bytes_until_nul(&buf).unwrap().to_bytes(),
            b"options"
        );

        let exact = help_heuristic(b"help", 0, false);
        let wrong_case = help_heuristic(b"Help", 0, true);
        assert!(wrong_case > exact);
    }
}
