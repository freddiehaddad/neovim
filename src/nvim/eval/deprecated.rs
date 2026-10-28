//! Deprecated Vimscript evaluation functions (`src/nvim/eval/deprecated.c`).

use crate::api::private::converter::K_BOOL_VAR_TRUE;
use crate::channel::{CallbackReader, channel_close, channel_create_event, channel_job_start};
use crate::errors::{E_API_SPAWN_FAILED, E_INVARG, E_INVARG2};
use crate::eval::find_job;
use crate::eval::funcs::{f_jobstart, f_jobstop};
use crate::eval::typval::{
    Callback, EvalFuncData, TvList, TypVal, VarType, tv_dict_add_bool, tv_dict_alloc, tv_dict_free,
    tv_get_string,
};
use crate::ex_cmds::check_secure;
use crate::globals::*;
use crate::message::{emsg, semsg};
use std::ffi::{c_char, c_int};

const K_CHANNEL_PART_RPC: c_int = 3;
const K_CHANNEL_STDIN_PIPE: c_int = 0;

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
    unsafe {
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

        let mut args: *mut TvList = std::ptr::null_mut();
        let mut argsl: c_int = 0;
        if arg1.v_type == VarType::VarList {
            args = arg1.vval.v_list();
            argsl = if args.is_null() { 0 } else { (*args).lv_len };
            for (i, li) in
                (0 as c_int..).zip(crate::eval::typval::list_iter_mut(if args.is_null() {
                    std::ptr::null_mut()
                } else {
                    (*args).lv_first
                }))
            {
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
        if prog.is_null() || *prog == 0 {
            emsg(E_API_SPAWN_FAILED);
            return;
        }

        let mut argv = Vec::with_capacity((argsl as usize) + 1);
        argv.push(crate::memory::cstr_to_cstring(prog));

        if argsl > 0 && !args.is_null() {
            for li in crate::eval::typval::list_iter_mut((*args).lv_first) {
                argv.push(crate::memory::cstr_to_cstring(tv_get_string(&li.li_tv)));
            }
        }

        let chan = channel_job_start(
            argv,
            std::ptr::null(),
            callback_reader_init(),
            callback_reader_init(),
            callback_none(),
            false,
            true,
            false,
            false,
            K_CHANNEL_STDIN_PIPE,
            std::ptr::null(),
            0,
            0,
            std::ptr::null_mut(),
            rettv.vval.v_number_mut(),
        );
        if !chan.is_null() {
            channel_create_event(&mut *chan, std::ptr::null());
        }
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
    if !find_job(id, false).is_null() {
        f_jobstop(argvars, rettv, fptr);
    } else {
        let mut error: *const c_char = std::ptr::null();
        let closed = channel_close(id, K_CHANNEL_PART_RPC, &mut error);
        rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(closed));
        if !closed {
            emsg(error);
        }
    }
}

/// `"last_buffer_nr()"` function (`f_last_buffer_nr`).
pub fn f_last_buffer_nr(_argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    {
        let mut n: c_int = 0;
        for buf in for_all_buffers_mut() {
            if n < buf.b_fnum {
                n = buf.b_fnum;
            }
        }
        rettv.vval = crate::eval::typval::TypvalVval::Number(n as i64);
    }
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
        arg1.vval = crate::eval::typval::TypvalVval::Dict(tv_dict_alloc());
    }

    if arg1.v_type != VarType::VarDict {
        semsg!(E_INVARG2, c"expected dictionary");
        return;
    }

    let dict = arg1.vval.v_dict();
    tv_dict_add_bool(unsafe { &mut *dict }, c"term", K_BOOL_VAR_TRUE as c_int);
    f_jobstart(argvars, rettv, fptr);
    if must_free {
        tv_dict_free(dict);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_reader_init_defaults_to_null_and_false() {
        let r = callback_reader_init();
        assert!(r.cb.data.as_funcref().is_null());
        assert_eq!(r.cb.cb_type(), 0);
        assert!(r.self_dict.is_null());
        assert!(!r.eof);
        assert!(!r.buffered);
        assert!(!r.fwd_err);
        assert!(r.reader_type.is_none());
    }
}
