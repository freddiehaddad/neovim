//! Locale and message language management (`src/nvim/os/lang.c`).

use crate::buffer::maketitle;
use crate::charset::{
    buf_init_isk_chartab, init_isf_chartab, init_isi_chartab, skiptowhite, skipwhite,
};
use crate::eval::vars::{
    VV_COLLATE, VV_CTYPE, VV_LANG, VV_LC_TIME, VV_PROGPATH, get_vim_var_str, set_vim_var_string,
};
use crate::ex_docmd::exarg_T;
#[cfg(unix)]
use crate::garray::{ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::mbyte::mb_strnicmp;
#[cfg(unix)]
use crate::memory::xstrdup;
use crate::memory::{xfree, xstrlcpy};
use crate::message::{semsg, smsg};
use crate::option::set_helplang_default;
#[cfg(windows)]
use crate::os::env::os_getenv_noalloc;
use crate::os::env::{expand_T, os_setenv};
#[cfg(unix)]
use crate::os::shell::get_cmd_output;
use crate::path::{path_tail, path_tail_with_sep};
use crate::profile::time_msg;
use crate::strings::{gettext, setlocale};
use std::ffi::{c_char, c_int};
use std::ptr;

pub(crate) const LC_CTYPE: c_int = if cfg!(target_os = "linux") { 0 } else { 2 };

#[cfg(windows)]
const LC_ALL: c_int = 0;
#[cfg(windows)]
const LC_COLLATE: c_int = 1;
#[cfg(windows)]
const LC_NUMERIC: c_int = 4;
#[cfg(windows)]
const LC_TIME: c_int = 5;
#[cfg(windows)]
const VIM_LC_MESSAGES: c_int = 6789;

#[cfg(target_os = "linux")]
const LC_NUMERIC: c_int = 1;
#[cfg(target_os = "linux")]
const LC_TIME: c_int = 2;
#[cfg(target_os = "linux")]
const LC_COLLATE: c_int = 3;
#[cfg(target_os = "linux")]
const LC_MESSAGES: c_int = 5;
#[cfg(target_os = "linux")]
const LC_ALL: c_int = 6;
#[cfg(target_os = "linux")]
const VIM_LC_MESSAGES: c_int = LC_MESSAGES;

#[cfg(not(any(windows, target_os = "linux")))]
const LC_ALL: c_int = 0;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_COLLATE: c_int = 1;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_NUMERIC: c_int = 4;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_TIME: c_int = 5;
#[cfg(not(any(windows, target_os = "linux")))]
const LC_MESSAGES: c_int = 6;
#[cfg(not(any(windows, target_os = "linux")))]
const VIM_LC_MESSAGES: c_int = LC_MESSAGES;

static mut LOCALES: *mut *mut c_char = ptr::null_mut();
#[cfg(not(windows))]
static mut DID_INIT_LOCALES: bool = false;

unsafe extern "C" {

    #[cfg(target_os = "linux")]
    static mut _nl_msg_cat_cntr: c_int;

    #[cfg(windows)]
    fn libintl_bindtextdomain(domainname: *const c_char, dirname: *const c_char) -> *mut c_char;
    #[cfg(windows)]
    fn libintl_textdomain(domainname: *const c_char) -> *mut c_char;
    #[cfg(windows)]
    pub fn libintl_bind_textdomain_codeset(
        domainname: *const c_char,
        codeset: *const c_char,
    ) -> *mut c_char;

    #[cfg(not(windows))]
    pub fn bind_textdomain_codeset(
        domainname: *const c_char,
        codeset: *const c_char,
    ) -> *mut c_char;
    #[cfg(target_os = "linux")]
    fn bindtextdomain(domainname: *const c_char, dirname: *const c_char) -> *mut c_char;
    #[cfg(target_os = "linux")]
    fn textdomain(domainname: *const c_char) -> *mut c_char;
}

use crate::charset::NUL;
#[cfg(unix)]
use crate::diff::kShellOptSilent;
use crate::os::fs::MAXPATHL;
#[cfg(not(windows))]
use crate::strings::strtok_r;

#[inline]
unsafe fn get_locale_val(what: c_int) -> *mut c_char {
    unsafe { setlocale(what, ptr::null()) }
}

#[inline]
unsafe fn is_valid_mess_lang(lang: *const c_char) -> bool {
    if lang.is_null() {
        return false;
    }
    unsafe { (*lang as u8).is_ascii_alphabetic() && (*lang.add(1) as u8).is_ascii_alphabetic() }
}

/// Obtain the current messages language. Used to set the default for `'helplang'`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_mess_lang() -> *mut c_char {
    unsafe {
        #[cfg(not(windows))]
        let p = get_locale_val(LC_MESSAGES);
        #[cfg(windows)]
        let p = get_locale_val(LC_COLLATE);

        if is_valid_mess_lang(p) {
            p
        } else {
            ptr::null_mut()
        }
    }
}

unsafe fn get_mess_env() -> *mut c_char {
    unsafe {
        #[cfg(not(windows))]
        {
            get_locale_val(LC_MESSAGES)
        }
        #[cfg(windows)]
        {
            let mut p = os_getenv_noalloc(c"LC_ALL".as_ptr());
            if !p.is_null() {
                return p;
            }
            p = os_getenv_noalloc(c"LC_MESSAGES".as_ptr());
            if !p.is_null() {
                return p;
            }
            p = os_getenv_noalloc(c"LANG".as_ptr());
            if !p.is_null() && (*p as u8).is_ascii_digit() {
                p = ptr::null_mut();
            }
            if p.is_null() {
                p = get_locale_val(LC_CTYPE);
            }
            p
        }
    }
}

/// Set the `v:lang`, `v:lc_time`, `v:collate`, and `v:ctype` variables.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_lang_var() {
    unsafe {
        let mut loc: *const c_char = get_locale_val(LC_CTYPE);
        set_vim_var_string(VV_CTYPE, loc, -1);

        loc = get_mess_env();
        set_vim_var_string(VV_LANG, loc, -1);

        loc = get_locale_val(LC_TIME);
        set_vim_var_string(VV_LC_TIME, loc, -1);

        loc = get_locale_val(LC_COLLATE);
        set_vim_var_string(VV_COLLATE, loc, -1);
    }
}

/// Setup to use the current locale.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_locale() {
    unsafe {
        let _ = setlocale(LC_ALL, c"".as_ptr());
        let _ = setlocale(LC_NUMERIC, c"C".as_ptr());

        let mut localepath = [0 as c_char; MAXPATHL];
        xstrlcpy(
            localepath.as_mut_ptr(),
            get_vim_var_str(VV_PROGPATH),
            MAXPATHL,
        );
        let mut tail = path_tail_with_sep(localepath.as_mut_ptr());
        *tail = NUL;
        tail = path_tail(localepath.as_ptr());
        let offset = (tail as usize) - (localepath.as_ptr() as usize);
        xstrlcpy(tail, c"share/locale".as_ptr(), MAXPATHL - offset);

        #[cfg(windows)]
        {
            let _ = libintl_bindtextdomain(c"nvim".as_ptr(), localepath.as_ptr());
            let _ = libintl_textdomain(c"nvim".as_ptr());
        }
        #[cfg(target_os = "linux")]
        {
            let _ = bindtextdomain(c"nvim".as_ptr(), localepath.as_ptr());
            let _ = textdomain(c"nvim".as_ptr());
        }

        if !time_fd.is_null() {
            time_msg(c"locale set".as_ptr(), ptr::null());
        }
    }
}

/// `":language"`: Set the language (locale).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_language(eap: *mut exarg_T) {
    unsafe {
        let mut what = LC_ALL;
        let mut whatstr: *const c_char = c"".as_ptr();
        let mut name = (*eap).arg;

        let p = skiptowhite((*eap).arg);
        let arg_len = (p as usize) - ((*eap).arg as usize);
        if (*p == NUL || matches!(*p as u8, b' ' | b'\t')) && arg_len >= 3 {
            if mb_strnicmp((*eap).arg, c"messages".as_ptr(), arg_len) == 0 {
                what = VIM_LC_MESSAGES;
                name = skipwhite(p);
                whatstr = c"messages ".as_ptr();
            } else if mb_strnicmp((*eap).arg, c"ctype".as_ptr(), arg_len) == 0 {
                what = LC_CTYPE;
                name = skipwhite(p);
                whatstr = c"ctype ".as_ptr();
            } else if mb_strnicmp((*eap).arg, c"time".as_ptr(), arg_len) == 0 {
                what = LC_TIME;
                name = skipwhite(p);
                whatstr = c"time ".as_ptr();
            } else if mb_strnicmp((*eap).arg, c"collate".as_ptr(), arg_len) == 0 {
                what = LC_COLLATE;
                name = skipwhite(p);
                whatstr = c"collate ".as_ptr();
            }
        }

        if *name == NUL {
            let mut p_val = if what == VIM_LC_MESSAGES {
                get_mess_env()
            } else {
                setlocale(what, ptr::null())
            };
            if p_val.is_null() || *p_val == NUL {
                p_val = c"Unknown".as_ptr().cast_mut();
            }
            let _ = smsg!(
                0,
                gettext(c"Current %slanguage: \"%s\"".as_ptr()),
                whatstr,
                p_val,
            );
        } else {
            #[cfg(windows)]
            let loc = if what == VIM_LC_MESSAGES {
                c"".as_ptr().cast_mut()
            } else {
                let l = setlocale(what, name);
                let _ = setlocale(LC_NUMERIC, c"C".as_ptr());
                l
            };
            #[cfg(not(windows))]
            let loc = {
                let l = setlocale(what, name);
                let _ = setlocale(LC_NUMERIC, c"C".as_ptr());
                l
            };

            if loc.is_null() {
                let _ = semsg!(
                    gettext(c"E197: Cannot set language to \"%s\"".as_ptr()),
                    name,
                );
            } else {
                #[cfg(target_os = "linux")]
                {
                    _nl_msg_cat_cntr += 1;
                }
                let _ = os_setenv(c"LC_ALL".as_ptr(), c"".as_ptr(), 1);

                if what != LC_TIME && what != LC_COLLATE {
                    if what == LC_ALL {
                        let _ = os_setenv(c"LANG".as_ptr(), name, 1);
                        let _ = os_setenv(c"LANGUAGE".as_ptr(), c"".as_ptr(), 1);
                    }
                    if what != LC_CTYPE {
                        let _ = os_setenv(c"LC_MESSAGES".as_ptr(), name, 1);
                        set_helplang_default(name);
                    }
                }

                set_lang_var();
                maketitle();
            }
        }

        let _ = buf_init_isk_chartab(curbuf);
        let _ = init_isf_chartab();
        let _ = init_isi_chartab();
    }
}

#[cfg(not(windows))]
unsafe fn find_locales() -> *mut *mut c_char {
    unsafe {
        let mut locales_ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        let mut saveptr: *mut c_char = ptr::null_mut();

        let locale_a = get_cmd_output(
            c"locale -a".as_ptr().cast_mut(),
            ptr::null_mut(),
            kShellOptSilent,
            ptr::null_mut(),
        );
        if locale_a.is_null() {
            return ptr::null_mut();
        }
        ga_init(&raw mut locales_ga, size_of::<*mut c_char>() as c_int, 20);

        let mut loc = strtok_r(locale_a, c"\n".as_ptr(), &raw mut saveptr);
        while !loc.is_null() {
            loc = xstrdup(loc);
            ga_grow(&raw mut locales_ga, 1);
            *locales_ga
                .ga_data
                .cast::<*mut c_char>()
                .add(locales_ga.ga_len as usize) = loc;
            locales_ga.ga_len += 1;
            loc = strtok_r(ptr::null_mut(), c"\n".as_ptr(), &raw mut saveptr);
        }
        xfree(locale_a.cast());
        ga_grow(&raw mut locales_ga, 1);
        *locales_ga
            .ga_data
            .cast::<*mut c_char>()
            .add(locales_ga.ga_len as usize) = ptr::null_mut();
        locales_ga.ga_data.cast::<*mut c_char>()
    }
}

unsafe fn init_locales() {
    #[cfg(not(windows))]
    unsafe {
        if DID_INIT_LOCALES {
            return;
        }
        DID_INIT_LOCALES = true;
        LOCALES = find_locales();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_locales() {
    unsafe {
        if LOCALES.is_null() {
            return;
        }
        let mut i: usize = 0;
        while !(*LOCALES.add(i)).is_null() {
            xfree((*LOCALES.add(i)).cast());
            i += 1;
        }
        xfree(LOCALES.cast());
        LOCALES = ptr::null_mut();
    }
}

/// Function given to `ExpandGeneric()` to obtain the possible arguments of `":language"`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_lang_arg(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    match idx {
        0 => c"messages".as_ptr().cast_mut(),
        1 => c"ctype".as_ptr().cast_mut(),
        2 => c"time".as_ptr().cast_mut(),
        3 => c"collate".as_ptr().cast_mut(),
        _ => unsafe {
            init_locales();
            if LOCALES.is_null() {
                ptr::null_mut()
            } else {
                *LOCALES.add((idx - 4) as usize)
            }
        },
    }
}

/// Function given to `ExpandGeneric()` to obtain the available locales.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_locales(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        init_locales();
        if LOCALES.is_null() {
            ptr::null_mut()
        } else {
            *LOCALES.add(idx as usize)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lang_init() {
    #[cfg(target_os = "macos")]
    unsafe {
        if !os_env_exists(c"LANG".as_ptr(), true) {
            let _ = os_setenv(c"LANG".as_ptr(), c"C.UTF-8".as_ptr(), 1);
            let _ = setlocale(LC_ALL, c"C.UTF-8".as_ptr());
            let _ = setlocale(LC_NUMERIC, c"C".as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn is_valid_mess_lang_and_get_lang_arg_work() {
        unsafe {
            assert!(!is_valid_mess_lang(ptr::null()));
            assert!(!is_valid_mess_lang(c"C".as_ptr()));
            assert!(!is_valid_mess_lang(c"C.UTF-8".as_ptr()));
            assert!(is_valid_mess_lang(c"en_US.UTF-8".as_ptr()));

            assert_eq!(
                CStr::from_ptr(get_lang_arg(ptr::null_mut(), 0)).to_bytes(),
                b"messages"
            );
            assert_eq!(
                CStr::from_ptr(get_lang_arg(ptr::null_mut(), 3)).to_bytes(),
                b"collate"
            );
        }
    }
}
