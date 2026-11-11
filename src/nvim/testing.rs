//! Built-in `assert_*` and `test_*` functions (`src/nvim/testing.c`).
#![forbid(unsafe_code)]

use crate::api::private::converter::{K_BOOL_VAR_FALSE, K_BOOL_VAR_TRUE};
use crate::errors::E_CANT_READ_FILE_STR;
use crate::eval::encode::{encode_tv2echo, encode_tv2string};
use crate::eval::typval::{
    EvalFuncData, TypVal, TypvalVval, VAR_UNLOCKED, VarType, tv_check_for_float_or_nr_arg,
    tv_check_for_opt_number_arg, tv_check_for_opt_string_arg, tv_check_for_opt_string_or_list_arg,
    tv_check_for_string_or_number_arg, tv_clear, tv_dict_add_tv, tv_dict_alloc, tv_equal,
    tv_get_float, tv_get_number_chk, tv_get_string, tv_get_string_buf_chk, tv_get_string_chk,
    tv_list_len,
};
use crate::eval::vars::{
    VV_ERRMSG, VV_EXCEPTION, VV_TESTING, assert_error, get_vim_var_nr, get_vim_var_str,
    get_vim_var_tv, set_vim_var_string,
};
use crate::eval::{garbage_collect, pattern_match};
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::IOSIZE;
use crate::globals::*;
use crate::mbyte::{mb_cptr2char_adv, utf_slice2char};
use crate::message::{emsg, msg_reset_scroll};
use crate::os::fs::CFile;
use crate::regexp::NUMBUFLEN;
use crate::runtime::{ESTACK_NONE, estack_sfile, sourcing_lnum};
use crate::strings::{format_to_buf, format_to_buf_safelen};
use crate::viml::parser::expressions::{BS, CAR, ESC, NL, TAB};
use std::ffi::CStr;

const EOF_VAL: i32 = -1;

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
fn buf_concat_cstr(gap: &mut Vec<u8>, s: Option<&CStr>) {
    if let Some(s) = s {
        gap.extend_from_slice(s.to_bytes());
    }
}

fn prepare_assert_error(gap: &mut Vec<u8>) {
    let sname = estack_sfile(ESTACK_NONE);
    let slnum = sourcing_lnum();

    gap.reserve(100);
    if let Some(ref sname_cstr) = sname {
        buf_concat_cstr(gap, Some(sname_cstr.as_c_str()));
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

fn ga_concat_esc(gap: &mut Vec<u8>, p: &[u8]) {
    if p.len() > 1 {
        gap.extend_from_slice(p);
        return;
    }

    let b = p.first().copied().unwrap_or(0);
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
                let s = format!("\\x{b:02x}");
                ga_concat_literal(gap, s.as_bytes());
            } else {
                gap.push(b);
            }
        }
    }
}

fn ga_concat_shorten_esc(gap: &mut Vec<u8>, str_cstr: Option<&CStr>) {
    let Some(str_cstr) = str_cstr else {
        ga_concat_literal(gap, b"NULL");
        return;
    };

    let mut rem = str_cstr.to_bytes();
    while !rem.is_empty() {
        let mut same_len: i32 = 1;
        let orig = rem;
        let c = mb_cptr2char_adv(&mut rem);
        let clen = orig.len() - rem.len();
        while !rem.is_empty() && c == utf_slice2char(rem) {
            same_len += 1;
            rem = &rem[clen.min(rem.len())..];
        }
        if same_len > 20 {
            ga_concat_literal(gap, b"\\[");
            ga_concat_esc(gap, &orig[..clen]);
            let tail = format!(" occurs {same_len} times]");
            ga_concat_literal(gap, tail.as_bytes());
        } else {
            ga_concat_esc(gap, &orig[..clen]);
            rem = &orig[clen..];
        }
    }
}

fn fill_assert_error(
    gap: &mut Vec<u8>,
    opt_msg_tv: &mut TypVal,
    exp_str: Option<&CStr>,
    mut exp_tv_arg: Option<&mut TypVal>,
    got_tv_arg: &mut TypVal,
    atype: AssertType,
) {
    let got_tv = got_tv_arg;
    let mut did_copy = false;
    let mut omitted: i32 = 0;

    if opt_msg_tv.v_type != VarType::VarUnknown
        && !(opt_msg_tv.v_type == VarType::VarString
            && opt_msg_tv
                .vval
                .v_string_cstr()
                .is_none_or(|s| s.to_bytes().is_empty()))
    {
        let tofree = encode_tv2echo(opt_msg_tv, None);
        buf_concat_cstr(gap, Some(tofree.as_c_str()));
        ga_concat_literal(gap, b": ");
    }

    if atype == AssertType::Match || atype == AssertType::NotMatch {
        ga_concat_literal(gap, b"Pattern ");
    } else if atype == AssertType::NotEqual {
        ga_concat_literal(gap, b"Expected not equal to ");
    } else {
        ga_concat_literal(gap, b"Expected ");
    }

    if let Some(exp_str) = exp_str {
        if atype == AssertType::Fails {
            ga_concat_literal(gap, b"'");
        }
        ga_concat_shorten_esc(gap, Some(exp_str));
        if atype == AssertType::Fails {
            ga_concat_literal(gap, b"'");
        }
    } else {
        let exp_tv = exp_tv_arg.as_deref_mut().unwrap();
        if atype != AssertType::NotEqual
            && exp_tv.v_type == VarType::VarDict
            && got_tv.v_type == VarType::VarDict
            && let (Some(exp_d), Some(got_d)) = (exp_tv.vval.v_dict_opt(), got_tv.vval.v_dict_opt())
        {
            did_copy = true;
            let new_exp_d = tv_dict_alloc();
            let new_got_d = tv_dict_alloc();
            exp_tv.vval = crate::eval::typval::TypvalVval::Dict(new_exp_d.dv_id);
            got_tv.vval = crate::eval::typval::TypvalVval::Dict(new_got_d.dv_id);

            for (_hi, di1) in exp_d.dv_hashtab.iter_items_mut() {
                let hi_key = CStr::from_bytes_until_nul(&di1.di_key).unwrap_or(c"");
                let hi_bytes = hi_key.to_bytes();
                let item2 = crate::eval::typval::tv_dict_find_opt(Some(&*got_d), hi_key, -1);
                if let Some(item2) = item2 {
                    if !tv_equal(&di1.di_tv, &item2.di_tv, false) {
                        tv_dict_add_tv(&mut *new_exp_d, hi_bytes, &mut di1.di_tv);
                        tv_dict_add_tv(&mut *new_got_d, hi_bytes, &mut item2.di_tv);
                    } else {
                        omitted += 1;
                    }
                } else {
                    tv_dict_add_tv(&mut *new_exp_d, hi_bytes, &mut di1.di_tv);
                }
            }

            for (_hi, di2) in got_d.dv_hashtab.iter_items_mut() {
                let hi_key = CStr::from_bytes_until_nul(&di2.di_key).unwrap_or(c"");
                let hi_bytes = hi_key.to_bytes();
                if crate::eval::typval::tv_dict_find_opt(Some(&*exp_d), hi_key, -1).is_none() {
                    tv_dict_add_tv(&mut *new_got_d, hi_bytes, &mut di2.di_tv);
                }
            }
        }

        let tofree = encode_tv2string(&mut *exp_tv, None);
        ga_concat_shorten_esc(gap, Some(tofree.as_c_str()));
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
        ga_concat_shorten_esc(gap, Some(tofree.as_c_str()));

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
        if let Some(exp_tv) = exp_tv_arg {
            tv_clear(exp_tv);
        }
        tv_clear(&mut *got_tv);
    }
}

fn assert_equal_common(argvars: &mut [TypVal], atype: AssertType) -> i32 {
    if tv_equal(&argvars[0], &argvars[1], false) != (atype == AssertType::Equal) {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let [arg0, arg1, arg2, ..] = argvars else {
            unreachable!()
        };
        fill_assert_error(&mut ga, arg2, None, Some(arg0), arg1, atype);
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_match_common(argvars: &mut [TypVal], atype: AssertType) -> i32 {
    let mut buf1 = [0u8; NUMBUFLEN];
    let mut buf2 = [0u8; NUMBUFLEN];
    let pat = tv_get_string_buf_chk(&argvars[0], &mut buf1);
    let text = tv_get_string_buf_chk(&argvars[1], &mut buf2);

    if let (Some(pat), Some(text)) = (pat, text)
        && (pattern_match(pat, Some(text), false) != 0) != (atype == AssertType::Match)
    {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let [arg0, arg1, arg2, ..] = argvars else {
            unreachable!()
        };
        fill_assert_error(&mut ga, arg2, None, Some(arg0), arg1, atype);
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_bool(argvars: &mut [TypVal], is_true: bool) -> i32 {
    let mut error = false;
    let arg0 = &argvars[0];
    let expected_bool = if is_true {
        K_BOOL_VAR_TRUE
    } else {
        K_BOOL_VAR_FALSE
    };
    if (arg0.v_type != VarType::VarNumber
        || (tv_get_number_chk(&argvars[0], Some(&mut error)) == 0) == is_true
        || error)
        && (arg0.v_type != VarType::VarBool || arg0.vval.v_bool() != expected_bool as i32)
    {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        let (first, rest) = argvars.split_at_mut(1);
        fill_assert_error(
            &mut ga,
            &mut rest[0],
            Some(if is_true { c"True" } else { c"False" }),
            None,
            &mut first[0],
            AssertType::Other,
        );
        assert_error(&ga);
        return 1;
    }
    0
}

fn assert_append_cmd_or_arg(gap: &mut Vec<u8>, argvars: &mut [TypVal], cmd: Option<&CStr>) {
    if argvars[1].v_type != VarType::VarUnknown && argvars[2].v_type != VarType::VarUnknown {
        let tofree = encode_tv2echo(&mut argvars[2], None);
        buf_concat_cstr(gap, Some(tofree.as_c_str()));
    } else {
        buf_concat_cstr(gap, cmd);
    }
}

fn assert_beeps(argvars: &mut [TypVal], no_beep: bool) -> i32 {
    let cmd_owned = tv_get_string_chk(&argvars[0]).map(std::borrow::Cow::into_owned);
    let mut ret: i32 = 0;

    let beeps = g_st().did_beep;
    g_st().suppress_errthrow = true;
    g_st().emsg_silent = 0;
    do_cmdline_cmd(cmd_owned.as_deref().unwrap_or(c""));
    let beeped = g_st().did_beep != beeps;
    if if no_beep { beeped } else { !beeped } {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        if no_beep {
            ga_concat_literal(&mut ga, b"command did beep: ");
        } else {
            ga_concat_literal(&mut ga, b"command did not beep: ");
        }
        buf_concat_cstr(&mut ga, cmd_owned.as_deref());
        assert_error(&ga);
        ret = 1;
    }

    g_st().suppress_errthrow = false;
    g_st().emsg_on_display = false;
    ret
}

/// `"assert_beeps(cmd [, error])"` function (`f_assert_beeps`).
pub fn f_assert_beeps(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_beeps(argvars, false) as i64);
}

/// `"assert_nobeep(cmd [, error])"` function (`f_assert_nobeep`).
pub fn f_assert_nobeep(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_beeps(argvars, true) as i64);
}

/// `"assert_equal(expected, actual[, msg])"` function (`f_assert_equal`).
pub fn f_assert_equal(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equal_common(
        argvars,
        AssertType::Equal,
    ) as i64);
}

fn assert_equalfile(argvars: &mut [TypVal]) -> i32 {
    let mut buf1 = [0u8; NUMBUFLEN];
    let mut buf2 = [0u8; NUMBUFLEN];
    let fname1 = tv_get_string_buf_chk(&argvars[0], &mut buf1);
    let fname2 = tv_get_string_buf_chk(&argvars[1], &mut buf2);

    let (Some(fname1), Some(fname2)) = (fname1, fname2) else {
        return 0;
    };

    let mut iobuf = [0u8; IOSIZE];
    let mut iobuflen = 0usize;
    let mut line1 = [0u8; 200];
    let mut line2 = [0u8; 200];
    let mut lineidx: usize = 0;

    if let Some(fd1) = CFile::open(fname1, c"rb") {
        if let Some(fd2) = CFile::open(fname2, c"rb") {
            let mut linecount: i64 = 1;
            let mut count: i64 = 0;
            loop {
                let c1 = fd1.getc();
                let c2 = fd2.getc();
                if c1 == EOF_VAL {
                    if c2 != EOF_VAL {
                        iobuflen = crate::memory::copy_bytes_to_c_buf(
                            &mut iobuf,
                            b"first file is shorter",
                        );
                    }
                    break;
                } else if c2 == EOF_VAL {
                    iobuflen =
                        crate::memory::copy_bytes_to_c_buf(&mut iobuf, b"second file is shorter");
                    break;
                } else {
                    line1[lineidx] = c1 as u8;
                    line2[lineidx] = c2 as u8;
                    lineidx += 1;
                    if c1 != c2 {
                        let msg = format!("difference at byte {count}, line {linecount}");
                        iobuflen = crate::memory::copy_bytes_to_c_buf(&mut iobuf, msg.as_bytes());
                        break;
                    }
                }
                if c1 == (NL as i32) {
                    linecount += 1;
                    lineidx = 0;
                } else if lineidx + 2 == line1.len() {
                    line1.copy_within(100..lineidx, 0);
                    line2.copy_within(100..lineidx, 0);
                    lineidx -= 100;
                }
                count += 1;
            }
        } else {
            drop(fd1);
            iobuflen = format_to_buf_safelen!(&mut iobuf[..], E_CANT_READ_FILE_STR, fname2);
        }
    } else {
        iobuflen = format_to_buf_safelen!(&mut iobuf[..], E_CANT_READ_FILE_STR, fname1);
    }

    if iobuflen > 0 {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        if argvars[2].v_type != VarType::VarUnknown {
            let tofree = encode_tv2echo(&mut argvars[2], None);
            buf_concat_cstr(&mut ga, Some(tofree.as_c_str()));
            ga_concat_literal(&mut ga, b": ");
        }
        ga_concat_literal(&mut ga, &iobuf[..iobuflen]);
        if lineidx > 0 {
            ga_concat_literal(&mut ga, b" after \"");
            ga_concat_literal(&mut ga, &line1[..lineidx]);
            if line1[..lineidx] != line2[..lineidx] {
                ga_concat_literal(&mut ga, b"\" vs \"");
                ga_concat_literal(&mut ga, &line2[..lineidx]);
            }
            ga_concat_literal(&mut ga, b"\"");
        }
        assert_error(&ga);
        return 1;
    }
    0
}

/// `"assert_equalfile(fname-one, fname-two[, msg])"` function (`f_assert_equalfile`).
pub fn f_assert_equalfile(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equalfile(argvars) as i64);
}

/// `"assert_notequal(expected, actual[, msg])"` function (`f_assert_notequal`).
pub fn f_assert_notequal(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_equal_common(
        argvars,
        AssertType::NotEqual,
    ) as i64);
}

/// `"assert_exception(string[, msg])"` function (`f_assert_exception`).
pub fn f_assert_exception(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let error = tv_get_string_chk(&argvars[0]);
    let v_exc = get_vim_var_str(VV_EXCEPTION);
    if v_exc.is_empty() {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        ga_concat_literal(&mut ga, b"v:exception is not set");
        assert_error(&ga);
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    } else if let Some(error) = error {
        let exc_bytes = v_exc.to_bytes();
        let err_bytes = error.to_bytes();
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
                None,
                Some(arg0),
                &mut *get_vim_var_tv(VV_EXCEPTION),
                AssertType::Other,
            );
            assert_error(&ga);
            rettv.vval = crate::eval::typval::TypvalVval::Number(1);
        }
    }
}

/// `"assert_fails(cmd [, error [, msg]])"` function (`f_assert_fails`).
pub fn f_assert_fails(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let save_trylevel = g_st().trylevel;
    let called_emsg_before = g_st().called_emsg;
    let mut wrong_arg_msg: Option<&CStr> = None;
    let mut tofree: Option<std::ffi::CString> = None;

    if !tv_check_for_string_or_number_arg(argvars, 0)
        || !tv_check_for_opt_string_or_list_arg(argvars, 1)
        || (argvars[1].v_type != VarType::VarUnknown
            && argvars[2].v_type != VarType::VarUnknown
            && (!tv_check_for_opt_number_arg(argvars, 3)
                || (argvars[3].v_type != VarType::VarUnknown
                    && !tv_check_for_opt_string_arg(argvars, 4))))
    {
        return;
    }

    g_st().trylevel = 0;
    g_st().suppress_errthrow = true;
    g_st().in_assert_fails = true;
    g_st().no_wait_return += 1;

    let cmd_owned = tv_get_string_chk(&argvars[0]).map(std::borrow::Cow::into_owned);
    do_cmdline_cmd(cmd_owned.as_deref().unwrap_or(c""));

    g_st().trylevel = save_trylevel;
    g_st().suppress_errthrow = false;

    if g_st().called_emsg == called_emsg_before {
        let mut ga = Vec::new();
        prepare_assert_error(&mut ga);
        ga_concat_literal(&mut ga, b"command did not fail: ");
        assert_append_cmd_or_arg(&mut ga, argvars, cmd_owned.as_deref());
        assert_error(&ga);
        rettv.vval = crate::eval::typval::TypvalVval::Number(1);
    } else if argvars[1].v_type != VarType::VarUnknown {
        let mut buf = [0u8; NUMBUFLEN];
        let mut buf2 = [0u8; NUMBUFLEN];
        let mut expected_str: Option<&CStr> = None;
        let mut error_found = false;
        let mut error_found_index: usize = 1;
        let actual_owned = g_st().emsg_assert_fails_msg.clone();
        let mut actual_cstr: &CStr = actual_owned.as_deref().unwrap_or(c"[unknown]");

        let arg1 = &argvars[1];
        let mut valid_args = true;
        if arg1.v_type == VarType::VarString {
            if let Some(expected) = tv_get_string_buf_chk(arg1, &mut buf) {
                let act_bytes = actual_cstr.to_bytes();
                let exp_bytes = expected.to_bytes();
                let contains = if exp_bytes.is_empty() {
                    true
                } else {
                    act_bytes.windows(exp_bytes.len()).any(|w| w == exp_bytes)
                };
                error_found = !contains;
            } else {
                error_found = true;
            }
        } else if arg1.v_type == VarType::VarList {
            let list = arg1.vval.v_list_opt();
            let list_len = tv_list_len(list.as_deref());
            if !(1..=2).contains(&list_len) {
                wrong_arg_msg = Some(
                    c"E856: \"assert_fails()\" second argument must be a string or a list with one or two strings",
                );
                valid_args = false;
            } else {
                let list_ref = list.unwrap();
                let first_tv = &list_ref.first_opt().unwrap().li_tv;
                let expected = tv_get_string_buf_chk(first_tv, &mut buf);
                if let Some(expected) = expected {
                    if pattern_match(expected, Some(actual_cstr), false) == 0 {
                        error_found = true;
                        expected_str = Some(expected);
                    } else if list_len == 2 {
                        let s = get_vim_var_str(VV_ERRMSG).into_owned();
                        tofree = Some(s);
                        actual_cstr = tofree.as_deref().unwrap_or(c"");
                        let last_tv = &list_ref.last_opt().unwrap().li_tv;
                        let expected2 = tv_get_string_buf_chk(last_tv, &mut buf2);
                        if let Some(expected2) = expected2 {
                            if pattern_match(expected2, Some(actual_cstr), false) == 0 {
                                error_found = true;
                                expected_str = Some(expected2);
                            }
                        } else {
                            valid_args = false;
                        }
                    }
                } else {
                    valid_args = false;
                }
            }
        } else {
            wrong_arg_msg = Some(
                c"E856: \"assert_fails()\" second argument must be a string or a list with one or two strings",
            );
            valid_args = false;
        }

        if valid_args
            && !error_found
            && argvars[2].v_type != VarType::VarUnknown
            && argvars[3].v_type != VarType::VarUnknown
        {
            let arg3 = &argvars[3];
            if arg3.v_type != VarType::VarNumber {
                wrong_arg_msg = Some(c"E1115: \"assert_fails()\" fourth argument must be a number");
                valid_args = false;
            } else if arg3.vval.v_number() >= 0 && {
                let expected: i64 = g_st().emsg_assert_fails_lnum as _;
                arg3.vval.v_number() != expected
            } {
                error_found = true;
                error_found_index = 3;
            }
            if valid_args && !error_found && argvars[4].v_type != VarType::VarUnknown {
                let arg4 = &argvars[4];
                if arg4.v_type != VarType::VarString {
                    wrong_arg_msg =
                        Some(c"E1116: \"assert_fails()\" fifth argument must be a string");
                    valid_args = false;
                } else if !arg4.vval.v_string_is_null()
                    && pattern_match(
                        arg4.vval.v_string_cstr_or_empty(),
                        g_st().emsg_assert_fails_context.as_deref(),
                        false,
                    ) == 0
                {
                    error_found = true;
                    error_found_index = 4;
                }
            }
        }

        if valid_args && error_found {
            let mut actual_tv = TypVal {
                v_type: VarType::VarUnknown,
                v_lock: VAR_UNLOCKED,
                vval: TypvalVval::Number(0),
            };
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            if error_found_index == 3 {
                actual_tv.v_type = VarType::VarNumber;
                {
                    actual_tv.vval =
                        crate::eval::typval::TypvalVval::Number(g_st().emsg_assert_fails_lnum as _);
                }
            } else if error_found_index == 4 {
                actual_tv.v_type = VarType::VarString;
                actual_tv.vval = crate::eval::typval::TypvalVval::from_opt_cstr(
                    g_st().emsg_assert_fails_context.as_deref(),
                );
            } else {
                actual_tv.v_type = VarType::VarString;
                actual_tv.vval = crate::eval::typval::TypvalVval::from_cstr(actual_cstr);
            }
            let (first, rest) = argvars.split_at_mut(3);
            let exp_ref = if error_found_index < 3 {
                let (head, mid) = first.split_at_mut(2);
                fill_assert_error(
                    &mut ga,
                    &mut mid[0],
                    expected_str,
                    Some(&mut head[error_found_index]),
                    &mut actual_tv,
                    AssertType::Fails,
                );
                None
            } else {
                Some(&mut rest[error_found_index - 3])
            };
            if let Some(exp_ref) = exp_ref {
                fill_assert_error(
                    &mut ga,
                    &mut first[2],
                    expected_str,
                    Some(exp_ref),
                    &mut actual_tv,
                    AssertType::Fails,
                );
            }
            ga_concat_literal(&mut ga, b": ");
            assert_append_cmd_or_arg(&mut ga, argvars, cmd_owned.as_deref());
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
    g_st().lines_left = g_st().rows_global;
    g_st().emsg_assert_fails_msg = None;
    drop(tofree);
    set_vim_var_string(VV_ERRMSG, None, 0);
    if let Some(msg) = wrong_arg_msg {
        emsg(msg);
    }
}

/// `"assert_false(actual[, msg])"` function (`f_assert_false`).
pub fn f_assert_false(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_bool(argvars, false) as i64);
}

fn assert_inrange(argvars: &mut [TypVal]) -> i32 {
    let mut error = false;
    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    let arg2 = &argvars[2];

    if arg0.v_type == VarType::VarFloat
        || arg1.v_type == VarType::VarFloat
        || arg2.v_type == VarType::VarFloat
    {
        let flower = tv_get_float(arg0);
        let fupper = tv_get_float(arg1);
        let factual = tv_get_float(arg2);

        if factual < flower || factual > fupper {
            let mut ga = Vec::new();
            prepare_assert_error(&mut ga);
            let mut expected_str = [0u8; 200];
            format_to_buf!(&mut expected_str[..], c"range %g - %g,", flower, fupper,);
            let (first, rest) = argvars.split_at_mut(3);
            fill_assert_error(
                &mut ga,
                &mut rest[0],
                Some(CStr::from_bytes_until_nul(&expected_str).unwrap_or(c"")),
                None,
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
                Some(CStr::from_bytes_with_nul(expected_str.as_bytes()).unwrap_or(c"")),
                None,
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
pub fn f_assert_inrange(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
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
pub fn f_assert_match(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_match_common(
        argvars,
        AssertType::Match,
    ) as i64);
}

/// `"assert_notmatch(pattern, actual[, msg])"` function (`f_assert_notmatch`).
pub fn f_assert_notmatch(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_match_common(
        argvars,
        AssertType::NotMatch,
    ) as i64);
}

/// `"assert_report(msg)"` function (`f_assert_report`).
pub fn f_assert_report(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut ga = Vec::new();
    prepare_assert_error(&mut ga);
    buf_concat_cstr(&mut ga, Some(&tv_get_string(&argvars[0])));
    assert_error(&ga);
    rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// `"assert_true(actual[, msg])"` function (`f_assert_true`).
pub fn f_assert_true(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(assert_bool(argvars, true) as i64);
}

/// `"test_garbagecollect_now()"` function (`f_test_garbagecollect_now`).
pub fn f_test_garbagecollect_now(
    _argvars: &mut [TypVal],
    _rettv: &mut TypVal,
    _fptr: EvalFuncData,
) {
    if get_vim_var_nr(VV_TESTING) == 0 {
        emsg(c"E1142: Calling test_garbagecollect_now() while v:testing is not set");
    } else {
        garbage_collect(true);
    }
}

/// `"test_write_list_log()"` function (`f_test_write_list_log`).
pub fn f_test_write_list_log(argvars: &mut [TypVal], _rettv: &mut TypVal, _fptr: EvalFuncData) {
    let _fname = tv_get_string_chk(&argvars[0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ga_concat_esc_formats_control_chars_and_printable_ascii() {
        let mut ga = Vec::new();
        ga_concat_esc(&mut ga, b"\n");
        ga_concat_esc(&mut ga, b"\x01");
        ga_concat_esc(&mut ga, b"A");
        assert_eq!(&ga[..], b"\\n\\x01A");
    }
}
