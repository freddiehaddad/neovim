//! Built-in `assert_*` and `test_*` functions (`src/nvim/testing.c`).

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE};
use crate::errors::e_cant_read_file_str;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::typval::{
    EvalFuncData, TypvalVval, VAR_UNLOCKED, VarType, tv_check_for_float_or_nr_arg,
    tv_check_for_opt_number_arg, tv_check_for_opt_string_arg, tv_check_for_opt_string_or_list_arg,
    tv_check_for_string_or_number_arg, tv_clear, tv_dict_add_tv, tv_dict_alloc, tv_dict_find,
    tv_equal, tv_get_float, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk,
    tv_get_string_chk, tv_list_first, tv_list_last, tv_list_len, typval_T,
};
use crate::eval::vars::{
    VV_ERRMSG, VV_EXCEPTION, VV_TESTING, assert_error, get_vim_var_nr, get_vim_var_str,
    get_vim_var_tv, set_vim_var_string,
};
use crate::eval::{garbage_collect, pattern_match};
use crate::ex_docmd::do_cmdline_cmd;
use crate::fileio::{fclose, fgetc};
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::hashtab::hashitem_empty;
use crate::mbyte::{mb_cptr2char_adv, utf_ptr2char};
use crate::memory::xstrlcpy;
use crate::message::{emsg, msg_reset_scroll};
use crate::os::fs::os_fopen;
use crate::regexp::NUMBUFLEN;
use crate::runtime::{ESTACK_NONE, estack_sfile, sourcing_lnum};
use crate::strings::{vim_snprintf, vim_snprintf_safelen};
use crate::viml::parser::expressions::{BS, CAR, ESC, NL, TAB};
use std::ffi::{c_char, c_int};

const EOF_VAL: c_int = -1;

pub(crate) const FF: u8 = 0x0c;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssertType {
    Equal,
    NotEqual,
    Match,
    NotMatch,
    Fails,
    Other,
}

#[inline]
pub fn ga_concat_literal(gap: &mut Vec<u8>, s: &[u8]) {
    gap.extend_from_slice(s);
}

#[inline]
fn buf_concat_cstr(gap: &mut Vec<u8>, s: *const c_char) {
     {
        if !s.is_null() {
            gap.extend_from_slice(crate::memory::cstr_as_bytes(s));
        }
    }
}

#[inline]
fn buf_concat_len(gap: &mut Vec<u8>, s: *const c_char, len: usize) {
    unsafe {
        if !s.is_null() && len > 0 {
            gap.extend_from_slice(std::slice::from_raw_parts(s.cast::<u8>(), len));
        }
    }
}

fn prepare_assert_error(gap: &mut Vec<u8>) {
    let sname = estack_sfile(ESTACK_NONE);
        let slnum = sourcing_lnum();

        gap.reserve(100);
        if let Some(ref sname_cstr) = sname {
            buf_concat_cstr(gap, sname_cstr.as_ptr());
            if slnum > 0 {
                ga_concat_literal(gap, b" ");
            }
        }
        if slnum > 0 {
            let line_str = format!("line {slnum}");
            ga_concat_literal(gap, line_str.as_bytes());
        }
        if sname.is_some() || slnum > 0 {
            ga_concat_literal(gap, b": ");
        }
}

fn ga_concat_esc(gap: &mut Vec<u8>, p: impl crate::strings::IntoCCharPtr, clen: c_int) {
    let p = p.into_c_char_ptr();
    unsafe {
        if clen > 1 {
            buf_concat_len(gap, p, clen as usize);
            return;
        }

        let b = *p as u8;
        match b {
            BS => ga_concat_literal(gap, b"\\b"),
            ESC => ga_concat_literal(gap, b"\\e"),
            FF => ga_concat_literal(gap, b"\\f"),
            NL => ga_concat_literal(gap, b"\\n"),
            TAB => ga_concat_literal(gap, b"\\t"),
            CAR => ga_concat_literal(gap, b"\\r"),
            b'\\' => ga_concat_literal(gap, b"\\\\"),
            _ => {
                if b < b' ' || b == 0x7f {
                    let s = format!("\\x{:02x}", *p as c_int);
                    ga_concat_literal(gap, s.as_bytes());
                } else {
                    gap.push(b);
                }
            }
        }
    }
}

fn ga_concat_shorten_esc(gap: &mut Vec<u8>, str_ptr: *const c_char) {
    unsafe {
        if str_ptr.is_null() {
            ga_concat_literal(gap, b"NULL");
            return;
        }

        let mut p = str_ptr;
        while *p != 0 {
            let mut same_len: c_int = 1;
            let mut s = p;
            let c = mb_cptr2char_adv(&mut s);
            let clen = (s as usize) - (p as usize);
            while *s != 0 && c == utf_ptr2char(s) {
                same_len += 1;
                s = s.add(clen);
            }
            if same_len > 20 {
                ga_concat_literal(gap, b"\\[");
                ga_concat_esc(gap, p, clen as c_int);
                let tail = format!(" occurs {same_len} times]");
                ga_concat_literal(gap, tail.as_bytes());
                p = s;
            } else {
                ga_concat_esc(gap, p, clen as c_int);
                p = p.add(clen);
            }
        }
    }
}

fn fill_assert_error(
    gap: &mut Vec<u8>,opt_msg_tv: &mut typval_T,
    exp_str: *const c_char,
    exp_tv_arg: *mut typval_T,got_tv_arg: &mut typval_T,
    atype: AssertType,
) {
    unsafe {
        let exp_tv = exp_tv_arg;
        let got_tv = got_tv_arg;
        let mut did_copy = false;
        let mut omitted: c_int = 0;

        if opt_msg_tv.v_type != VarType::VAR_UNKNOWN
            && !(opt_msg_tv.v_type == VarType::VAR_STRING
                && (opt_msg_tv.vval.v_string().is_null() || *opt_msg_tv.vval.v_string() == 0))
        {
            let tofree = encode_tv2echo(opt_msg_tv, None);
            buf_concat_cstr(gap, tofree.as_ptr());
            ga_concat_literal(gap, b": ");
        }

        if atype == AssertType::Match || atype == AssertType::NotMatch {
            ga_concat_literal(gap, b"Pattern ");
        } else if atype == AssertType::NotEqual {
            ga_concat_literal(gap, b"Expected not equal to ");
        } else {
            ga_concat_literal(gap, b"Expected ");
        }

        if exp_str.is_null() {
            if atype != AssertType::NotEqual
                && (*exp_tv).v_type == VarType::VAR_DICT
                && got_tv.v_type == VarType::VAR_DICT
                && !(*exp_tv).vval.v_dict().is_null()
                && !got_tv.vval.v_dict().is_null()
            {
                let exp_d = (*exp_tv).vval.v_dict();
                let got_d = got_tv.vval.v_dict();

                did_copy = true;
                let new_exp_d = tv_dict_alloc();
                let new_got_d = tv_dict_alloc();
                (*exp_tv).vval = crate::eval::typval::TypvalVval::Dict(new_exp_d);
                got_tv.vval = crate::eval::typval::TypvalVval::Dict(new_got_d);

                let mut todo = (*exp_d).dv_hashtab.ht_used as c_int;
                let mut hi = (*exp_d).dv_hashtab.array_ptr();
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        let di1 = crate::eval::typval::tv_dict_hi2di(hi);
                        let hi_key = (&*hi).key_ptr();
                        let item2 = tv_dict_find(got_d, hi_key, -1);
                        if item2.is_null()
                            || !tv_equal(&(*di1).di_tv, &(*item2).di_tv, false)
                        {
                            let key_len = crate::memory::cstr_len(hi_key);
                            tv_dict_add_tv(&mut *new_exp_d, hi_key, key_len, &mut (*di1).di_tv);
                            if !item2.is_null() {
                                tv_dict_add_tv(
                                    &mut *new_got_d,
                                    hi_key,
                                    key_len,
                                    &mut (*item2).di_tv,
                                );
                            }
                        } else {
                            omitted += 1;
                        }
                        todo -= 1;
                    }
                    hi = hi.add(1);
                }

                todo = (*got_d).dv_hashtab.ht_used as c_int;
                hi = (*got_d).dv_hashtab.array_ptr();
                while todo > 0 {
                    if !hashitem_empty(&*hi) {
                        let hi_key = (&*hi).key_ptr();
                        let item2 = tv_dict_find(exp_d, hi_key, -1);
                        if item2.is_null() {
                            let di2 = crate::eval::typval::tv_dict_hi2di(hi);
                            let key_len = crate::memory::cstr_len(hi_key);
                            tv_dict_add_tv(&mut *new_got_d, hi_key, key_len, &mut (*di2).di_tv);
                        }
                        todo -= 1;
                    }
                    hi = hi.add(1);
                }
            }

            let tofree = encode_tv2string(&mut *exp_tv, None);
            ga_concat_shorten_esc(gap, tofree.as_ptr());
        } else {
            if atype == AssertType::Fails {
                ga_concat_literal(gap, b"'");
            }
            ga_concat_shorten_esc(gap, exp_str);
            if atype == AssertType::Fails {
                ga_concat_literal(gap, b"'");
            }
        }

        if atype != AssertType::NotEqual {
            if atype == AssertType::Match {
                ga_concat_literal(gap, b" does not match ");
            } else if atype == AssertType::NotMatch {
                ga_concat_literal(gap, b" does match ");
            } else {
                ga_concat_literal(gap, b" but got ");
            }
            let tofree = encode_tv2string(&mut *got_tv, None);
            ga_concat_shorten_esc(gap, tofree.as_ptr());

            if omitted != 0 {
                let s = if omitted == 1 {
                    " - 1 equal item omitted".to_string()
                } else {
                    format!(" - {omitted} equal items omitted")
                };
                ga_concat_literal(gap, s.as_bytes());
            }
        }

        if did_copy {
            tv_clear(&mut *exp_tv);
            tv_clear(&mut *got_tv);
        }
    }
}

fn assert_equal_common(argvars: &mut [typval_T], atype: AssertType) -> c_int {
    if tv_equal(&argvars[0], &argvars[1], false) != (atype == AssertType::Equal) {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let [arg0, arg1, arg2, ..] = argvars else {
            unreachable!()
        };
        fill_assert_error(
            &mut ga,
            arg2,
            std::ptr::null(),
            &raw mut *arg0,
            arg1,
            atype,
        );
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_match_common(argvars: &mut [typval_T], atype: AssertType) -> c_int {
    let mut buf1 = [0 as c_char; NUMBUFLEN];
    let mut buf2 = [0 as c_char; NUMBUFLEN];
    let pat = tv_get_string_buf_chk(&argvars[0], buf1.as_mut_ptr());
    let text = tv_get_string_buf_chk(&argvars[1], buf2.as_mut_ptr());

    if !pat.is_null()
        && !text.is_null()
        && (pattern_match(pat, text, false) != 0) != (atype == AssertType::Match)
    {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let [arg0, arg1, arg2, ..] = argvars else {
            unreachable!()
        };
        fill_assert_error(
            &mut ga,
            arg2,
            std::ptr::null(),
            &raw mut *arg0,
            arg1,
            atype,
        );
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_bool(argvars: &mut [typval_T], is_true: bool) -> c_int {
    let mut error = false;
    let arg0 = &argvars[0];
    let expected_bool = if is_true {
        K_BOOL_VAR_TRUE
    } else {
        K_BOOL_VAR_FALSE
    };
    if (arg0.v_type != VarType::VAR_NUMBER
        || (tv_get_number_chk(&argvars[0], Some(&mut error)) == 0) == is_true
        || error)
        && (arg0.v_type != VarType::VAR_BOOL || arg0.vval.v_bool() != expected_bool as c_int)
    {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let (first, rest) = argvars.split_at_mut(1);
        fill_assert_error(
            &mut ga,
            &mut rest[0],
            if is_true {
                c"True".as_ptr()
            } else {
                c"False".as_ptr()
            },
            std::ptr::null_mut(),
            &mut first[0],
            AssertType::Other,
        );
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_append_cmd_or_arg(gap: &mut Vec<u8>,argvars: &mut [typval_T], cmd: *const c_char) {
     {
        if argvars[1].v_type != VarType::VAR_UNKNOWN
            && argvars[2].v_type != VarType::VAR_UNKNOWN
        {
            let tofree = encode_tv2echo(&mut argvars[2], None);
            buf_concat_cstr(gap, tofree.as_ptr());
        } else {
            buf_concat_cstr(gap, cmd);
        }
    }
}

fn assert_beeps(argvars: &mut [typval_T], no_beep: bool) -> c_int {
    let cmd = tv_get_string_chk(&argvars[0]);
        let mut ret: c_int = 0;

        let beeps = g_st().did_beep;
        g_st().suppress_errthrow = true;
        g_st().emsg_silent = 0;
        do_cmdline_cmd(cmd);
        let beeped = g_st().did_beep != beeps;
        if if no_beep { beeped } else { !beeped } {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            if no_beep {
                ga_concat_literal(&mut ga, b"command did beep: ");
            } else {
                ga_concat_literal(&mut ga, b"command did not beep: ");
            }
            buf_concat_cstr(&mut ga, cmd);
            assert_error(&ga);
            ret = 1;
        }

        g_st().suppress_errthrow = false;
        g_st().emsg_on_display = false;
        ret
}

/// `"assert_beeps(cmd [, error])"` function (`f_assert_beeps`).
pub fn f_assert_beeps(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_beeps(argvars, false) as i64);
}

/// `"assert_nobeep(cmd [, error])"` function (`f_assert_nobeep`).
pub fn f_assert_nobeep(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_beeps(argvars, true) as i64);
}

/// `"assert_equal(expected, actual[, msg])"` function (`f_assert_equal`).
pub fn f_assert_equal(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equal_common(argvars, AssertType::Equal) as i64);
}

fn assert_equalfile(argvars: &mut [typval_T]) -> c_int {
    unsafe {
        let mut buf1 = [0 as c_char; NUMBUFLEN];
        let mut buf2 = [0 as c_char; NUMBUFLEN];
        let fname1 = tv_get_string_buf_chk(&argvars[0], buf1.as_mut_ptr());
        let fname2 = tv_get_string_buf_chk(&argvars[1], buf2.as_mut_ptr());

        if fname1.is_null() || fname2.is_null() {
            return 0;
        }

        let mut iobuf = [0 as c_char; IOSIZE];
        let mut iobuflen = 0usize;
        let fd1 = os_fopen(fname1, c"rb");
        let mut line1 = [0 as c_char; 200];
        let mut line2 = [0 as c_char; 200];
        let mut lineidx: usize = 0;

        if fd1.is_null() {
            iobuflen = vim_snprintf_safelen!(
                iobuf.as_mut_ptr(),
                IOSIZE,
                e_cant_read_file_str,
                fname1,
            );
        } else {
            let fd2 = os_fopen(fname2, c"rb");
            if fd2.is_null() {
                fclose(fd1);
                iobuflen = vim_snprintf_safelen!(
                    iobuf.as_mut_ptr(),
                    IOSIZE,
                    e_cant_read_file_str,
                    fname2,
                );
            } else {
                let mut linecount: i64 = 1;
                let mut count: i64 = 0;
                loop {
                    let c1 = fgetc(fd1);
                    let c2 = fgetc(fd2);
                    if c1 == EOF_VAL {
                        if c2 != EOF_VAL {
                            iobuflen = xstrlcpy(
                                iobuf.as_mut_ptr(),
                                c"first file is shorter",
                                IOSIZE,
                            );
                        }
                        break;
                    } else if c2 == EOF_VAL {
                        iobuflen = xstrlcpy(
                            iobuf.as_mut_ptr(),
                            c"second file is shorter",
                            IOSIZE,
                        );
                        break;
                    } else {
                        line1[lineidx] = c1 as c_char;
                        line2[lineidx] = c2 as c_char;
                        lineidx += 1;
                        if c1 != c2 {
                            let msg = format!("difference at byte {count}, line {linecount}\0");
                            iobuflen =
                                xstrlcpy(iobuf.as_mut_ptr(), msg.as_ptr().cast::<c_char>(), IOSIZE);
                            break;
                        }
                    }
                    if c1 == (NL as c_int) {
                        linecount += 1;
                        lineidx = 0;
                    } else if lineidx + 2 == line1.len() {
                        line1.copy_within(100..lineidx, 0);
                        line2.copy_within(100..lineidx, 0);
                        lineidx -= 100;
                    }
                    count += 1;
                }
                fclose(fd1);
                fclose(fd2);
            }
        }

        if iobuflen > 0 {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            if argvars[2].v_type != VarType::VAR_UNKNOWN {
                let tofree = encode_tv2echo(&mut argvars[2], None);
                buf_concat_cstr(&mut ga, tofree.as_ptr());
                ga_concat_literal(&mut ga, b": ");
            }
            buf_concat_len(&mut ga, iobuf.as_ptr(), iobuflen);
            if lineidx > 0 {
                line1[lineidx] = 0;
                line2[lineidx] = 0;
                ga_concat_literal(&mut ga, b" after \"");
                buf_concat_len(&mut ga, line1.as_ptr(), lineidx);
                if line1[..lineidx] != line2[..lineidx] {
                    ga_concat_literal(&mut ga, b"\" vs \"");
                    buf_concat_len(&mut ga, line2.as_ptr(), lineidx);
                }
                ga_concat_literal(&mut ga, b"\"");
            }
            assert_error(&ga);
            return 1;
        }
        0
    }
}

/// `"assert_equalfile(fname-one, fname-two[, msg])"` function (`f_assert_equalfile`).
pub fn f_assert_equalfile(argvars: &mut [typval_T], rettv: &mut typval_T,
    _fptr: EvalFuncData,){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equalfile(argvars) as i64);
}

/// `"assert_notequal(expected, actual[, msg])"` function (`f_assert_notequal`).
pub fn f_assert_notequal(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equal_common(argvars, AssertType::NotEqual) as i64);
}

/// `"assert_exception(string[, msg])"` function (`f_assert_exception`).
pub fn f_assert_exception(argvars: &mut [typval_T], rettv: &mut typval_T,
    _fptr: EvalFuncData,){

    unsafe {
        let error = tv_get_string_chk(&argvars[0]);
        let v_exc = get_vim_var_str(VV_EXCEPTION);
        if *v_exc == 0 {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            ga_concat_literal(&mut ga, b"v:exception is not set");
            assert_error(&ga);
            rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        } else if !error.is_null() {
            let exc_bytes = crate::memory::cstr_as_bytes(v_exc);
            let err_bytes = crate::memory::cstr_as_bytes(error);
            let found = if err_bytes.is_empty() {
                true
            } else {
                exc_bytes.windows(err_bytes.len()).any(|w| w == err_bytes)
            };
            if !found {
                let mut ga = Vec::new();
                prepare_assert_error(&mut ga);
                let [arg0, arg1, ..] = argvars else {
                    unreachable!()
                };
                fill_assert_error(
                    &mut ga,
                    arg1,
                    std::ptr::null(),
                    &raw mut *arg0,
                    &mut *get_vim_var_tv(VV_EXCEPTION),
                    AssertType::Other,
                );
                assert_error(&ga);
                rettv.vval = crate::eval::typval::TypvalVval::Number(1);
            }
        }
    }
}

/// `"assert_fails(cmd [, error [, msg]])"` function (`f_assert_fails`).
pub fn f_assert_fails(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let save_trylevel = g_st().trylevel;
        let called_emsg_before = g_st().called_emsg;
        let mut wrong_arg_msg: *const c_char = std::ptr::null();
        let mut tofree: Option<std::ffi::CString> = None;

        if !tv_check_for_string_or_number_arg(argvars, 0)
            || !tv_check_for_opt_string_or_list_arg(argvars, 1)
            || (argvars[1].v_type != VarType::VAR_UNKNOWN
                && argvars[2].v_type != VarType::VAR_UNKNOWN
                && (!tv_check_for_opt_number_arg(argvars, 3)
                    || (argvars[3].v_type != VarType::VAR_UNKNOWN
                        && !tv_check_for_opt_string_arg(argvars, 4))))
        {
            return;
        }

        g_st().trylevel = 0;
        g_st().suppress_errthrow = true;
        g_st().in_assert_fails = true;
        g_st().no_wait_return += 1;

        let cmd = tv_get_string_chk(&argvars[0]);
        do_cmdline_cmd(cmd);

        g_st().trylevel = save_trylevel;
        g_st().suppress_errthrow = false;

        if g_st().called_emsg == called_emsg_before {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            ga_concat_literal(&mut ga, b"command did not fail: ");
            assert_append_cmd_or_arg(&mut ga, argvars, cmd);
            assert_error(&ga);
            rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        } else if argvars[1].v_type != VarType::VAR_UNKNOWN {
            let mut buf = [0 as c_char; NUMBUFLEN];
            let mut expected_str: *const c_char = std::ptr::null();
            let mut error_found = false;
            let mut error_found_index: usize = 1;
            let mut actual: *mut c_char = (&raw const g_st().emsg_assert_fails_msg)
                .as_ref()
                .unwrap()
                .as_ref()
                .map_or(c"[unknown]".as_ptr().cast_mut(), |s| s.as_ptr().cast_mut());

            let arg1 = &argvars[1];
            let mut valid_args = true;
            if arg1.v_type == VarType::VAR_STRING {
                let expected = tv_get_string_buf_chk(arg1, buf.as_mut_ptr());
                if expected.is_null() {
                    error_found = true;
                } else {
                    let act_bytes = crate::memory::cstr_as_bytes(actual);
                    let exp_bytes = crate::memory::cstr_as_bytes(expected);
                    let contains = if exp_bytes.is_empty() {
                        true
                    } else {
                        act_bytes.windows(exp_bytes.len()).any(|w| w == exp_bytes)
                    };
                    error_found = !contains;
                }
            } else if arg1.v_type == VarType::VAR_LIST {
                let list = arg1.vval.v_list();
                if list.is_null() || tv_list_len(list.as_ref()) < 1 || tv_list_len(list.as_ref()) > 2 {
                    wrong_arg_msg =
                        c"E856: \"assert_fails()\" second argument must be a string or a list with one or two strings"
                            .as_ptr();
                    valid_args = false;
                } else {
                    let first_tv = &(*tv_list_first(list.as_ref())).li_tv;
                    let expected = tv_get_string_buf_chk(first_tv, buf.as_mut_ptr());
                    if expected.is_null() {
                        valid_args = false;
                    } else if pattern_match(expected, actual, false) == 0 {
                        error_found = true;
                        expected_str = expected;
                    } else if tv_list_len(list.as_ref()) == 2 {
                        let s = crate::memory::cstr_to_cstring(get_vim_var_str(VV_ERRMSG));
                        actual = s.as_ptr().cast_mut();
                        tofree = Some(s);
                        let last_tv = &(*tv_list_last(list)).li_tv;
                        let expected2 = tv_get_string_buf_chk(last_tv, buf.as_mut_ptr());
                        if expected2.is_null() {
                            valid_args = false;
                        } else if pattern_match(expected2, actual, false) == 0 {
                            error_found = true;
                            expected_str = expected2;
                        }
                    }
                }
            } else {
                wrong_arg_msg =
                    c"E856: \"assert_fails()\" second argument must be a string or a list with one or two strings"
                        .as_ptr();
                valid_args = false;
            }

            if valid_args
                && !error_found
                && argvars[2].v_type != VarType::VAR_UNKNOWN
                && argvars[3].v_type != VarType::VAR_UNKNOWN
            {
                let arg3 = &argvars[3];
                if arg3.v_type != VarType::VAR_NUMBER {
                    wrong_arg_msg =
                        c"E1115: \"assert_fails()\" fourth argument must be a number".as_ptr();
                    valid_args = false;
                } else if arg3.vval.v_number() >= 0 && {
                    let expected: i64 = g_st().emsg_assert_fails_lnum as _;
                    arg3.vval.v_number() != expected
                } {
                    error_found = true;
                    error_found_index = 3;
                }
                if valid_args && !error_found && argvars[4].v_type != VarType::VAR_UNKNOWN {
                    let arg4 = &argvars[4];
                    if arg4.v_type != VarType::VAR_STRING {
                        wrong_arg_msg =
                            c"E1116: \"assert_fails()\" fifth argument must be a string".as_ptr();
                        valid_args = false;
                    } else if !arg4.vval.v_string().is_null()
                        && pattern_match(
                            arg4.vval.v_string(),
                            (&raw const g_st().emsg_assert_fails_context)
                                .as_ref()
                                .unwrap()
                                .as_ref()
                                .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut()),
                            false,
                        ) == 0
                    {
                        error_found = true;
                        error_found_index = 4;
                    }
                }
            }

            if valid_args && error_found {
                let mut actual_tv = typval_T {
                    v_type: VarType::VAR_UNKNOWN,
                    v_lock: VAR_UNLOCKED,
                    vval: TypvalVval::Number(0),
                };
                let mut ga = Vec::new();
                prepare_assert_error(&mut ga);
                if error_found_index == 3 {
                    actual_tv.v_type = VarType::VAR_NUMBER;
                    {
                        actual_tv.vval = crate::eval::typval::TypvalVval::Number(g_st().emsg_assert_fails_lnum as _);
                    }
                } else if error_found_index == 4 {
                    actual_tv.v_type = VarType::VAR_STRING;
                    actual_tv.vval = crate::eval::typval::TypvalVval::String((&raw const g_st().emsg_assert_fails_context)
                        .as_ref()
                        .unwrap()
                        .as_ref()
                        .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut()));
                } else {
                    actual_tv.v_type = VarType::VAR_STRING;
                    actual_tv.vval = crate::eval::typval::TypvalVval::String(actual);
                }
                let (first, rest) = argvars.split_at_mut(3);
                let exp_ptr = if error_found_index < 3 {
                    &raw mut first[error_found_index]
                } else {
                    &raw mut rest[error_found_index - 3]
                };
                fill_assert_error(
                    &mut ga,
                    &mut first[2],
                    expected_str,
                    exp_ptr,
                    &mut actual_tv,
                    AssertType::Fails,
                );
                ga_concat_literal(&mut ga, b": ");
                assert_append_cmd_or_arg(&mut ga, argvars, cmd);
                assert_error(&ga);
                rettv.vval = crate::eval::typval::TypvalVval::Number(1);
            }
        }

        g_st().trylevel = save_trylevel;
        g_st().suppress_errthrow = false;
        g_st().in_assert_fails = false;
        g_st().did_emsg = 0;
        g_st().got_int = false;
        g_st().msg_col = 0;
        g_st().no_wait_return -= 1;
        g_st().need_wait_return = false;
        g_st().emsg_on_display = false;
        msg_reset_scroll();
        g_st().lines_left = g_st().Rows;
        g_st().emsg_assert_fails_msg = None;
        drop(tofree);
        set_vim_var_string(VV_ERRMSG, std::ptr::null(), 0);
        if !wrong_arg_msg.is_null() {
            emsg(wrong_arg_msg);
        }
    }
}

/// `"assert_false(actual[, msg])"` function (`f_assert_false`).
pub fn f_assert_false(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_bool(argvars, false) as i64);
}

fn assert_inrange(argvars: &mut [typval_T]) -> c_int {
    let mut error = false;
    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    let arg2 = &argvars[2];

    if arg0.v_type == VarType::VAR_FLOAT
        || arg1.v_type == VarType::VAR_FLOAT
        || arg2.v_type == VarType::VAR_FLOAT
    {
        let flower = tv_get_float(arg0);
        let fupper = tv_get_float(arg1);
        let factual = tv_get_float(arg2);

        if factual < flower || factual > fupper {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            let mut expected_str = [0 as c_char; 200];
            vim_snprintf!(
                expected_str.as_mut_ptr(),
                expected_str.len(),
                c"range %g - %g,",
                flower,
                fupper,
            );
            let (first, rest) = argvars.split_at_mut(3);
            fill_assert_error(
                &mut ga,
                &mut rest[0],
                expected_str.as_ptr(),
                std::ptr::null_mut(),
                &mut first[2],
                AssertType::Other,
            );
            assert_error(&ga);
            return 1;
        }
    } else {
        let lower = tv_get_number_chk(arg0, Some(&mut error));
        let upper = tv_get_number_chk(arg1, Some(&mut error));
        let actual = tv_get_number_chk(arg2, Some(&mut error));

        if error {
            return 0;
        }
        if actual < lower || actual > upper {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            let expected_str = format!("range {lower} - {upper},\0");
            let (first, rest) = argvars.split_at_mut(3);
            fill_assert_error(
                &mut ga,
                &mut rest[0],
                expected_str.as_ptr().cast::<c_char>(),
                std::ptr::null_mut(),
                &mut first[2],
                AssertType::Other,
            );
            assert_error(&ga);
            return 1;
        }
    }
    0
}

/// `"assert_inrange(lower, upper, actual[, msg])"` function (`f_assert_inrange`).
pub fn f_assert_inrange(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    if !tv_check_for_float_or_nr_arg(argvars, 0)
        || !tv_check_for_float_or_nr_arg(argvars, 1)
        || !tv_check_for_float_or_nr_arg(argvars, 2)
        || !tv_check_for_opt_string_arg(argvars, 3)
    {
        return;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_inrange(argvars) as i64);
}

/// `"assert_match(pattern, actual[, msg])"` function (`f_assert_match`).
pub fn f_assert_match(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_match_common(argvars, AssertType::Match) as i64);
}

/// `"assert_notmatch(pattern, actual[, msg])"` function (`f_assert_notmatch`).
pub fn f_assert_notmatch(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_match_common(argvars, AssertType::NotMatch) as i64);
}

/// `"assert_report(msg)"` function (`f_assert_report`).
pub fn f_assert_report(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        buf_concat_cstr(&mut ga, tv_get_string(&argvars[0]));
        assert_error(&ga);
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// `"assert_true(actual[, msg])"` function (`f_assert_true`).
pub fn f_assert_true(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_bool(argvars, true) as i64);
}

/// `"test_garbagecollect_now()"` function (`f_test_garbagecollect_now`).
pub fn f_test_garbagecollect_now(_argvars: &mut [typval_T], _rettv: &mut typval_T,
    _fptr: EvalFuncData,){
    if get_vim_var_nr(VV_TESTING) == 0 {
            emsg(c"E1142: Calling test_garbagecollect_now() while v:testing is not set");
        } else {
            garbage_collect(true);
        }
}

/// `"test_write_list_log()"` function (`f_test_write_list_log`).
pub fn f_test_write_list_log(argvars: &mut [typval_T], _rettv: &mut typval_T,
    _fptr: EvalFuncData,){

    let _fname = tv_get_string_chk(&argvars[0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ga_concat_esc_formats_control_chars_and_printable_ascii() {
        let mut ga = Vec::new();
        ga_concat_esc(&mut ga, c"\n", 1);
            ga_concat_esc(&mut ga, c"\x01", 1);
            ga_concat_esc(&mut ga, c"A", 1);
            assert_eq!(&ga[..], b"\\n\\x01A");
    }
}
