//! Deprecated Vimscript evaluation functions (`src/nvim/eval/deprecated.c`).

use crate::api::private::converter::K_BOOL_VAR_TRUE;
use crate::channel::{CallbackReader, channel_close, channel_create_event, channel_job_start};
use crate::errors::{E_API_SPAWN_FAILED, E_INVARG, E_INVARG2};
use crate::eval::find_job;
use crate::eval::funcs::{f_jobstart, f_jobstop};
use crate::eval::typval::{
    Callback, EvalFuncData, TypVal, VarType, tv_dict_add_bool, tv_dict_alloc, tv_dict_free,
    tv_get_string,
};
use crate::ex_cmds::check_secure;
use crate::globals::*;
use crate::message::{emsg, semsg};

const K_CHANNEL_PART_RPC: i32 = 3;
const K_CHANNEL_STDIN_PIPE: i32 = 0;

#[inline]
const fn callback_none() -> Callback {
    Callback::NONE
}

#[inline]
const fn callback_reader_init() -> CallbackReader {
    CallbackReader::none()
}

/// `"rpcstart()"` function (`f_rpcstart`).
pub fn f_rpcstart(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    if check_secure() {
        return;
    }

    let arg0 = &argvars[0];
    let arg1 = &argvars[1];
    if arg0.v_type != VarType::VarString
        || (arg1.v_type != VarType::VarList && arg1.v_type != VarType::VarUnknown)
    {
        emsg(E_INVARG);
        return;
    }

    let mut args = None;
    let mut argsl: i32 = 0;
    if arg1.v_type == VarType::VarList {
        args = arg1.vval.v_list_opt();
        argsl = args.as_deref().map_or(0, |a| a.lv_len);
        for (i, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut(
            args.as_deref().and_then(|a| a.first_opt()),
        )) {
            if li.li_tv.v_type != VarType::VarString {
                semsg!(
                    c"E5010: List item %d of the second argument is not a string",
                    i,
                );
                return;
            }
        }
    }

    let prog = arg0.vval.v_string();
    if crate::memory::cstr_is_empty(prog) {
        emsg(E_API_SPAWN_FAILED);
        return;
    }

    let mut argv = Vec::with_capacity((argsl as usize) + 1);
    argv.push(crate::memory::cstr_to_cstring(prog));

    if argsl > 0
        && let Some(args) = args
    {
        for li in crate::eval::typval::list_iter_mut(args.first_opt()) {
            argv.push(tv_get_string(&li.li_tv).to_owned());
        }
    }

    let chan = channel_job_start(
        argv,
        None,
        callback_reader_init(),
        callback_reader_init(),
        callback_none(),
        false,
        true,
        false,
        false,
        K_CHANNEL_STDIN_PIPE,
        None,
        0,
        0,
        None,
        rettv.vval.v_number_mut(),
    );
    if let Some(chan) = chan {
        channel_create_event(chan);
    }
}

/// `"rpcstop()"` function (`f_rpcstop`).
pub fn f_rpcstop(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    if check_secure() {
        return;
    }

    let arg0 = &argvars[0];
    if arg0.v_type != VarType::VarNumber {
        emsg(E_INVARG);
        return;
    }

    let id = arg0.vval.v_number() as u64;
    if find_job(id, false).is_some() {
        f_jobstop(argvars, rettv, fptr);
    } else {
        let mut error = None;
        let closed = channel_close(id, K_CHANNEL_PART_RPC, Some(&mut error));
        rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(closed));
        if !closed && let Some(err) = error {
            emsg(err);
        }
    }
}

/// `"last_buffer_nr()"` function (`f_last_buffer_nr`).
pub fn f_last_buffer_nr(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut n: i32 = 0;
    for buf in for_all_buffers_mut() {
        if n < buf.b_fnum {
            n = buf.b_fnum;
        }
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(n as i64);
}

/// `"termopen(cmd[, cwd])"` function (`f_termopen`).
pub fn f_termopen(argvars: &mut [TypVal], rettv: &mut TypVal, fptr: EvalFuncData) {
    if check_secure() {
        return;
    }

    let mut must_free = false;
    let arg1 = &mut argvars[1];
    if arg1.v_type == VarType::VarUnknown {
        must_free = true;
        arg1.v_type = VarType::VarDict;
        arg1.vval =
            crate::eval::typval::TypvalVval::Dict(crate::globals::GraphPtr::new(tv_dict_alloc()));
    }

    if arg1.v_type != VarType::VarDict {
        semsg!(E_INVARG2, c"expected dictionary");
        return;
    }

    let dict = arg1.vval.v_dict();
    tv_dict_add_bool(arg1.vval.v_dict_ref(), c"term", K_BOOL_VAR_TRUE as i32);
    f_jobstart(argvars, rettv, fptr);
    if must_free && let Some(dict_ref) = dict.opt_mut() {
        tv_dict_free(dict_ref);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_reader_init_defaults_to_null_and_false() {
        let r = callback_reader_init();
        assert!(r.cb.data.as_funcref().is_none());
        assert_eq!(r.cb.cb_type(), 0);
        assert!(r.self_dict.is_null());
        assert!(!r.eof);
        assert!(!r.buffered);
        assert!(!r.fwd_err);
        assert!(r.reader_type.is_none());
    }
}
