//! Deprecated Vimscript evaluation functions (`src/nvim/eval/deprecated.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    unused_imports,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_void};

use crate::clipboard::{TypvalVval, VarType, list_T, listitem_T, typval_T};
use crate::eval::decode::dict_T;
use crate::garray::garray_T;
use crate::memory::{xmalloc, xstrdup};
use crate::msgpack_rpc::packer::handle_T;

pub type EvalFuncData = *mut c_void;
pub type Channel = c_void;

const K_BOOL_VAR_TRUE: c_int = 1;
const K_CHANNEL_PART_RPC: c_int = 3;
const K_CHANNEL_STDIN_PIPE: c_int = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Callback {
    pub data: *mut c_void,
    pub cb_type: c_int,
}

#[repr(C)]
pub struct CallbackReader {
    pub cb: Callback,
    pub self_dict: *mut dict_T,
    pub buffer: garray_T,
    pub eof: bool,
    pub buffered: bool,
    pub fwd_err: bool,
    pub reader_type: *const c_char,
}

#[repr(C)]
pub struct memline_T {
    pub ml_line_count: i32,
    pub ml_mfp: *mut c_void,
    pub ml_stack: *mut c_void,
    pub ml_stack_top: c_int,
    pub ml_stack_size: c_int,
    pub ml_flags: c_int,
    pub ml_line_textlen: c_int,
    pub ml_line_lnum: i32,
    pub ml_line_ptr: *mut c_char,
    pub ml_line_offset: usize,
    pub ml_line_offset_ff: c_int,
    pub ml_locked: *mut c_void,
    pub ml_locked_low: i32,
    pub ml_locked_high: i32,
    pub ml_locked_lineadd: c_int,
    pub ml_chunksize: *mut c_void,
    pub ml_numchunks: c_int,
    pub ml_usedchunks: c_int,
}

#[repr(C)]
pub struct buf_T {
    pub b_fnum: handle_T,
    pub b_ml: memline_T,
    pub b_next: *mut buf_T,
    pub b_prev: *mut buf_T,
}

#[cfg(not(test))]
unsafe extern "C" {
    static mut firstbuf: *mut buf_T;

    fn check_secure() -> bool;
    fn tv_get_string(tv: *const typval_T) -> *const c_char;
    fn tv_dict_alloc() -> *mut dict_T;
    fn tv_dict_add_bool(d: *mut dict_T, key: *const c_char, key_len: usize, val: c_int) -> c_int;
    fn tv_dict_free(d: *mut dict_T);

    fn channel_job_start(
        argv: *mut *mut c_char,
        exepath: *const c_char,
        on_stdout: CallbackReader,
        on_stderr: CallbackReader,
        on_exit: Callback,
        pty: bool,
        rpc: bool,
        overlapped: bool,
        detach: bool,
        stdin_mode: c_int,
        cwd: *const c_char,
        pty_width: u16,
        pty_height: u16,
        env: *mut dict_T,
        status_out: *mut i64,
    ) -> *mut Channel;
    fn channel_create_event(chan: *mut Channel, ext_source: *const c_char);
    fn channel_close(id: u64, part: c_int, error: *mut *const c_char) -> bool;
    fn find_job(id: u64, show_error: bool) -> *mut Channel;
    fn f_jobstart(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData);
    fn f_jobstop(argvars: *mut typval_T, rettv: *mut typval_T, fptr: EvalFuncData);

    fn emsg(s: *const c_char) -> bool;
    fn semsg(fmt: *const c_char, ...) -> bool;
}

#[inline]
const fn callback_none() -> Callback {
    Callback {
        data: std::ptr::null_mut(),
        cb_type: 0,
    }
}

#[inline]
const fn callback_reader_init() -> CallbackReader {
    CallbackReader {
        cb: callback_none(),
        self_dict: std::ptr::null_mut(),
        buffer: garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: std::ptr::null_mut(),
        },
        eof: false,
        buffered: false,
        fwd_err: false,
        reader_type: std::ptr::null(),
    }
}

/// `"rpcstart()"` function (`f_rpcstart`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_rpcstart(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv);

    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let arg0 = &*argvars;
        let arg1 = &*argvars.add(1);
        if arg0.v_type != VarType::VAR_STRING
            || (arg1.v_type != VarType::VAR_LIST && arg1.v_type != VarType::VAR_UNKNOWN)
        {
            emsg(c"E474: Invalid argument".as_ptr());
            return;
        }

        let mut args: *mut list_T = std::ptr::null_mut();
        let mut argsl: c_int = 0;
        if arg1.v_type == VarType::VAR_LIST {
            args = arg1.vval.v_list;
            argsl = if args.is_null() { 0 } else { (*args).lv_len };
            let mut i = 0 as c_int;
            let mut li: *const listitem_T = if args.is_null() {
                std::ptr::null()
            } else {
                (*args).lv_first
            };
            while !li.is_null() {
                if (*li).li_tv.v_type != VarType::VAR_STRING {
                    semsg(
                        c"E5010: List item %d of the second argument is not a string".as_ptr(),
                        i,
                    );
                    return;
                }
                i += 1;
                li = (*li).li_next;
            }
        }

        let prog = arg0.vval.v_string;
        if prog.is_null() || *prog == 0 {
            emsg(c"E903: Could not spawn API job".as_ptr());
            return;
        }

        let argvl = (argsl as usize) + 2;
        let argv = xmalloc(std::mem::size_of::<*mut c_char>() * argvl).cast::<*mut c_char>();
        *argv = xstrdup(prog);

        let mut idx = 1usize;
        if argsl > 0 && !args.is_null() {
            let mut li: *const listitem_T = (*args).lv_first;
            while !li.is_null() {
                *argv.add(idx) = xstrdup(tv_get_string(&(*li).li_tv));
                idx += 1;
                li = (*li).li_next;
            }
        }
        *argv.add(idx) = std::ptr::null_mut();

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
            std::ptr::addr_of_mut!((*rettv).vval.v_number),
        );
        if !chan.is_null() {
            channel_create_event(chan, std::ptr::null());
        }
    }
}

/// `"rpcstop()"` function (`f_rpcstop`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_rpcstop(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv, fptr);

    #[cfg(not(test))]
    unsafe {
        (*rettv).v_type = VarType::VAR_NUMBER;
        (*rettv).vval.v_number = 0;

        if check_secure() {
            return;
        }

        let arg0 = &*argvars;
        if arg0.v_type != VarType::VAR_NUMBER {
            emsg(c"E474: Invalid argument".as_ptr());
            return;
        }

        let id = arg0.vval.v_number as u64;
        if !find_job(id, false).is_null() {
            f_jobstop(argvars, rettv, fptr);
        } else {
            let mut error: *const c_char = std::ptr::null();
            let closed = channel_close(id, K_CHANNEL_PART_RPC, &mut error);
            (*rettv).vval.v_number = i64::from(closed);
            if !closed {
                emsg(error);
            }
        }
    }
}

/// `"last_buffer_nr()"` function (`f_last_buffer_nr`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_last_buffer_nr(
    _argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = rettv;

    #[cfg(not(test))]
    unsafe {
        let mut n: c_int = 0;
        let mut buf = firstbuf;
        while !buf.is_null() {
            if n < (*buf).b_fnum {
                n = (*buf).b_fnum;
            }
            buf = (*buf).b_next;
        }
        (*rettv).vval.v_number = n as i64;
    }
}

/// `"termopen(cmd[, cwd])"` function (`f_termopen`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_termopen(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    fptr: EvalFuncData,
) {
    #[cfg(test)]
    let _ = (argvars, rettv, fptr);

    #[cfg(not(test))]
    unsafe {
        if check_secure() {
            return;
        }

        let mut must_free = false;
        let arg1 = &mut *argvars.add(1);
        if arg1.v_type == VarType::VAR_UNKNOWN {
            must_free = true;
            arg1.v_type = VarType::VAR_DICT;
            arg1.vval.v_ptr = tv_dict_alloc().cast::<c_void>();
        }

        if arg1.v_type != VarType::VAR_DICT {
            semsg(
                c"E475: Invalid argument: %s".as_ptr(),
                c"expected dictionary".as_ptr(),
            );
            return;
        }

        let dict = arg1.vval.v_ptr.cast::<dict_T>();
        tv_dict_add_bool(dict, c"term".as_ptr(), 4, K_BOOL_VAR_TRUE);
        f_jobstart(argvars, rettv, fptr);
        if must_free {
            tv_dict_free(dict);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_reader_init_defaults_to_null_and_false() {
        let r = callback_reader_init();
        assert!(r.cb.data.is_null());
        assert_eq!(r.cb.cb_type, 0);
        assert!(r.self_dict.is_null());
        assert!(!r.eof);
        assert!(!r.buffered);
        assert!(!r.fwd_err);
        assert!(r.reader_type.is_null());
    }
}
