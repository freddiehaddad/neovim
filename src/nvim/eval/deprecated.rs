//! Deprecated Vimscript evaluation functions (`src/nvim/eval/deprecated.c`).
#![forbid(unsafe_code)]

use crate::api::private::converter::{K_BOOL_VAR_TRUE, object_to_vim, vim_to_object};
use crate::api::private::helpers::api_clear_error;
use crate::api::private::validate::{Array, Error, ErrorType, Object, ObjectData, ObjectType};
use crate::channel::{CallbackReader, channel_close, channel_create_event, channel_job_start};
use crate::errors::{E_API_SPAWN_FAILED, E_INVARG, E_INVARG2};
use crate::eval::find_job;
use crate::eval::funcs::{MAX_FUNC_ARGS, f_jobstart, f_jobstop};
use crate::eval::typval::{
    Callback, EvalFuncData, TypVal, VarType, tv_dict_add_bool, tv_dict_alloc, tv_dict_free,
    tv_get_string,
};
use crate::eval::userfunc::{restore_funccal, save_funccal, set_current_funccal};
use crate::ex_cmds::check_secure;
use crate::globals::*;
use crate::message::{emsg, semsg, semsg_multiline};
use crate::msgpack_rpc::channel::{find_channel, get_client_info, rpc_send_call, rpc_send_event};
use crate::runtime::Sctx;
use crate::strings::gettext;

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

/// "rpcnotify()" function (DEPRECATED)
pub fn f_rpcnotify(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);

    if check_secure() {
        return;
    }
    if argvars[0].v_type != VarType::VarNumber || argvars[0].vval.v_number() < 0 {
        semsg!(gettext(E_INVARG2), c"Channel id must be a positive integer",);
        return;
    }
    if argvars[1].v_type != VarType::VarString {
        semsg!(gettext(E_INVARG2), c"Event type must be a string",);
        return;
    }

    let mut args_storage = [const {
        Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }
    }; MAX_FUNC_ARGS];
    let mut args = Array::from_buf(&mut args_storage);
    for tv in argvars[2..]
        .iter_mut()
        .take_while(|tv| tv.v_type != VarType::VarUnknown)
    {
        *args.next_slot_mut() = vim_to_object(tv);
        args.size += 1;
    }

    let ok = rpc_send_event(
        argvars[0].vval.v_number() as u64,
        &tv_get_string(&argvars[1]),
        args,
    );
    if !ok {
        semsg!(gettext(E_INVARG2), c"Channel doesn't exist",);
        return;
    }
    rettv.vval = crate::eval::typval::TypvalVval::Number(1);
}

/// "rpcrequest()" function (DEPRECATED)
pub fn f_rpcrequest(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.v_type = VarType::VarNumber;
    rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    let l_provider_call_nesting = g_st().provider_call_nesting.get();

    if check_secure() {
        return;
    }
    if argvars[0].v_type != VarType::VarNumber || argvars[0].vval.v_number() <= 0 {
        semsg!(gettext(E_INVARG2), c"Channel id must be a positive integer",);
        return;
    }
    if argvars[1].v_type != VarType::VarString {
        semsg!(gettext(E_INVARG2), c"Method name must be a string",);
        return;
    }

    let mut args_storage = [const {
        Object {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Boolean(false),
        }
    }; MAX_FUNC_ARGS];
    let mut args = Array::from_buf(&mut args_storage);
    for tv in argvars[2..]
        .iter_mut()
        .take_while(|tv| tv.v_type != VarType::VarUnknown)
    {
        *args.next_slot_mut() = vim_to_object(tv);
        args.size += 1;
    }

    let mut save_current_sctx = Sctx::new();
    let mut save_autocmd_fname: Option<std::ffi::CString> = None;
    let mut save_autocmd_match: Option<std::ffi::CString> = None;
    let mut save_autocmd_fname_full = false;
    let mut save_autocmd_bufnr: i32 = 0;

    if l_provider_call_nesting != 0 {
        save_current_sctx = g_st().current_sctx.get();
        save_autocmd_fname = g_st().autocmd_fname.take();
        save_autocmd_match = g_st().autocmd_match.take();
        save_autocmd_fname_full = g_st().autocmd_fname_full.get();
        save_autocmd_bufnr = g_st().autocmd_bufnr.get();
        save_funccal();

        g_st()
            .current_sctx
            .set(g_st().provider_caller_scope.borrow().script_ctx);
        let es_entry = g_st().provider_caller_scope.borrow().es_entry.clone();
        crate::runtime::estack_push_entry(es_entry);
        g_st()
            .autocmd_fname
            .set(g_st().provider_caller_scope.borrow().autocmd_fname.clone());
        g_st()
            .autocmd_match
            .set(g_st().provider_caller_scope.borrow().autocmd_match.clone());
        g_st()
            .autocmd_fname_full
            .set(g_st().provider_caller_scope.borrow().autocmd_fname_full);
        g_st()
            .autocmd_bufnr
            .set(g_st().provider_caller_scope.borrow().autocmd_bufnr);
        set_current_funccal(g_st().provider_caller_scope.borrow().funccalp);
    }

    let mut err = Error::new();
    let chan_id = argvars[0].vval.v_number() as u64;
    let method = tv_get_string(&argvars[1]);
    let result = rpc_send_call(chan_id, &method, args, -1, &mut err);
    if l_provider_call_nesting != 0 {
        g_st().current_sctx.set(save_current_sctx);
        crate::runtime::estack_pop_raw();
        g_st().autocmd_fname.set(save_autocmd_fname);
        g_st().autocmd_match.set(save_autocmd_match);
        g_st().autocmd_fname_full.set(save_autocmd_fname_full);
        g_st().autocmd_bufnr.set(save_autocmd_bufnr);
        restore_funccal();
    }

    if err.err_type != ErrorType::ErrorTypeNone {
        let mut name: Option<&std::ffi::CStr> = None;
        if let Some(chan) = find_channel(chan_id) {
            name = get_client_info(chan, c"name");
        }
        if let Some(name) = name {
            semsg_multiline!(
                c"rpc_error",
                c"Invoking '%s' on channel %llu (%s):\n%s",
                method,
                chan_id,
                name,
                err.msg_cstr(),
            );
        } else {
            semsg_multiline!(
                c"rpc_error",
                c"Invoking '%s' on channel %llu:\n%s",
                method,
                chan_id,
                err.msg_cstr(),
            );
        }
    } else {
        object_to_vim(result, &mut *rettv, &mut err);
    }
    api_clear_error(&mut err);
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
        argsl = args.map_or(0, |a| a.lv_len.get());
        for (i, li) in (0_i32..).zip(crate::eval::typval::list_iter_mut(
            args.and_then(|a| a.first_opt()),
        )) {
            if li.tv().v_type != VarType::VarString {
                semsg!(
                    c"E5010: List item %d of the second argument is not a string",
                    i,
                );
                return;
            }
        }
    }

    if arg0.vval.v_string_is_empty() {
        emsg(E_API_SPAWN_FAILED);
        return;
    }

    let mut argv = Vec::with_capacity((argsl as usize) + 1);
    argv.push(arg0.vval.v_string_owned());

    if argsl > 0
        && let Some(args) = args
    {
        for li in crate::eval::typval::list_iter_mut(args.first_opt()) {
            argv.push(tv_get_string(&li.tv()).into_owned());
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
        if n < buf.b_fnum.get() {
            n = buf.b_fnum.get();
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
        arg1.vval = crate::eval::typval::TypvalVval::Dict(tv_dict_alloc());
    }

    if arg1.v_type != VarType::VarDict {
        semsg!(E_INVARG2, c"expected dictionary");
        return;
    }

    let dict = arg1.vval.v_dict();
    tv_dict_add_bool(arg1.vval.v_dict_ref(), c"term", K_BOOL_VAR_TRUE as i32);
    f_jobstart(argvars, rettv, fptr);
    if must_free && let Some(dict_ref) = dict.opt() {
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
