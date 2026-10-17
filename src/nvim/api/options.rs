//! Options API functions (`src/nvim/api/options.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ptr;

use crate::api::autocmd::TryState;
use crate::api::private::converter::sctx_T;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType, api_err_conflict, api_err_exp,
};
use crate::api::tabpage::{pos_T, tabpage_T, win_T};
use crate::context::{ctx_restore, ctx_switch};
use crate::lua::converter::Boolean;
use crate::map_glyph_cache::NvimString;
use crate::memory::{Arena, arena_memdupz, xfree, xstrdup};
use crate::msgpack_rpc::packer::handle_T;

pub type Buffer = handle_T;
pub type Window = handle_T;
pub type Tabpage = handle_T;
pub type OptionalKeys = u64;
pub type OptIndex = c_int;
pub type OptScope = c_int;
pub type set_op_T = c_int;
pub type set_prefix_T = c_int;
pub type scid_T = c_int;
pub type event_T = c_int;
pub type linenr_T = i32;
pub type colnr_T = i32;

const OK: c_int = 1;
const FAIL: c_int = 0;
const IOSIZE: usize = 1024 + 1;
const SID_NONE: scid_T = -6;
const BLN_DUMMY: c_int = 4;
const BF_DUMMY: c_int = 0x80;
const EVENT_FILETYPE: event_T = 59;

const OPT_GLOBAL: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;

const PREFIX_NONE: set_prefix_T = 1;

const kOptScopeGlobal: OptScope = 0;
const kOptScopeWin: OptScope = 1;
const kOptScopeBuf: OptScope = 2;
const kOptScopeTab: OptScope = 3;

const OP_NONE: set_op_T = 0;
const OP_ADDING: set_op_T = 1;
const OP_PREPENDING: set_op_T = 2;
const OP_REMOVING: set_op_T = 3;

const kOptInvalid: OptIndex = -1;
const kOptBufhidden: OptIndex = 27;
const kOptBuftype: OptIndex = 29;
const kOptFiletype: OptIndex = 97;
const kOptModeline: OptIndex = 192;
const kOptSwapfile: OptIndex = 301;

const KEYSET_OPTIDX_option__buf: u64 = 1;
const KEYSET_OPTIDX_option__tab: u64 = 2;
const KEYSET_OPTIDX_option__win: u64 = 3;
const KEYSET_OPTIDX_option__scope: u64 = 4;
const KEYSET_OPTIDX_option__dry_run: u64 = 5;
const KEYSET_OPTIDX_option__filetype: u64 = 6;
const KEYSET_OPTIDX_option__operation: u64 = 7;

#[repr(C)]
pub struct memfile_T {
    pub mf_fname: *mut c_char,
    pub mf_ffname: *mut c_char,
    pub mf_fd: c_int,
    pub mf_flags: c_int,
    pub mf_reopen: bool,
    pub mf_free_first: *mut c_void,
}

#[repr(C)]
pub struct memline_T {
    pub ml_line_count: linenr_T,
    pub ml_mfp: *mut memfile_T,
    pub ml_stack: *mut c_void,
    pub ml_stack_top: c_int,
    pub ml_stack_size: c_int,
    pub ml_flags: c_int,
    pub ml_line_textlen: colnr_T,
    pub ml_line_lnum: linenr_T,
    pub ml_line_ptr: *mut c_char,
    pub ml_line_offset: usize,
    pub ml_line_offset_ff: c_int,
    pub ml_locked: *mut c_void,
    pub ml_locked_low: linenr_T,
    pub ml_locked_high: linenr_T,
    pub ml_locked_lineadd: c_int,
    pub ml_chunksize: *mut c_void,
    pub ml_numchunks: c_int,
    pub ml_usedchunks: c_int,
}

#[repr(C)]
pub struct buf_T {
    pub handle: handle_T,
    pub b_ml: memline_T,
    pub b_next: *mut buf_T,
    pub b_prev: *mut buf_T,
    pub b_nwindows: c_int,
    pub b_flags: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct bufref_T {
    pub br_buf: *mut buf_T,
    pub br_fnum: c_int,
    pub br_buf_free_count: c_int,
}

#[repr(C)]
pub struct CtxSwitch {
    pub cs_flags: c_int,
    pub cs_mode: c_int,
    pub cs_curwin: handle_T,
    pub cs_curwin_xtab: bool,
    pub cs_prevwin: handle_T,
    pub cs_curtab: *mut tabpage_T,
    pub cs_same_win: bool,
    pub cs_visual_active: bool,
    pub cs_prompt_insert: c_int,
    pub cs_new_curwin: handle_T,
    pub cs_new_curbuf: bufref_T,
    pub cs_ctxwin_idx: c_int,
    pub cs_target_win: handle_T,
    pub cs_target_old_pos: pos_T,
    pub cs_did_chdir: bool,
    pub cs_dirs_tab: handle_T,
    pub cs_w_localdir: *mut c_char,
    pub cs_b_localdir: *mut c_char,
    pub cs_tp_localdir: *mut c_char,
    pub cs_globaldir: *mut c_char,
    pub cs_cwd: *mut c_char,
    pub cs_apply_acd: bool,
}

#[repr(C)]
pub struct CharBuf {
    pub data: *mut c_char,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union OptSetPrep {
    pub ptr: *mut c_void,
    pub flags: c_uint,
}

#[repr(C)]
pub struct optset_T {
    pub os_varp: *mut c_void,
    pub os_idx: OptIndex,
    pub os_flags: c_int,
    pub os_oldval: Object,
    pub os_newval: Object,
    pub os_prep: OptSetPrep,
    pub os_prep_free: Option<unsafe extern "C" fn(*mut c_void)>,
    pub os_value_checked: bool,
    pub os_value_changed: bool,
    pub os_restore_chartab: bool,
    pub os_errbuf: *const CharBuf,
    pub os_win: *mut win_T,
    pub os_buf: *mut buf_T,
}

#[repr(C)]
pub struct KeyDict_option {
    pub is_set__option_: OptionalKeys,
    pub scope: NvimString,
    pub win: Window,
    pub buf: Buffer,
    pub tab: Tabpage,
    pub filetype: NvimString,
    pub operation: NvimString,
    pub dry_run: Boolean,
}

#[repr(C)]
pub struct vimoption_T {
    _private: [u8; 0],
}

unsafe extern "C" {
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut current_sctx: sctx_T;

    fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T;
    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn find_tab_by_handle(tabpage: Tabpage, err: *mut Error) -> *mut tabpage_T;
    fn find_option(name: *const c_char) -> OptIndex;
    fn api_set_sctx(channel_id: u64) -> sctx_T;
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn api_free_object(value: Object);
    fn api_typename(t: ObjectType) -> *const c_char;
    fn copy_object(obj: Object, arena: *mut Arena) -> Object;
    fn option_has_type(opt_idx: OptIndex, t: ObjectType) -> bool;
    fn option_has_scope(opt_idx: OptIndex, scope: OptScope) -> bool;
    fn buflist_new(
        ffname_arg: *mut c_char,
        sfname_arg: *mut c_char,
        lnum: linenr_T,
        flags: c_int,
    ) -> *mut buf_T;
    fn ml_open(buf: *mut buf_T) -> c_int;
    fn set_bufref(bufref: *mut bufref_T, buf: *mut buf_T);
    fn bufref_valid(bufref: *mut bufref_T) -> bool;
    fn set_option_direct(opt_idx: OptIndex, value: Object, opt_flags: c_int, set_sid: scid_T);
    fn get_option_varp_scope_from(
        opt_idx: OptIndex,
        opt_flags: c_int,
        buf: *mut buf_T,
        win: *mut win_T,
    ) -> *mut c_void;
    fn has_event(event: event_T) -> bool;
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
    fn do_filetype_autocmd(buf: *mut buf_T, do_ftdetect: bool) -> bool;
    fn block_autocmds();
    fn unblock_autocmds();
    fn close_windows(buf: *mut buf_T, keep_curwin: bool);
    fn wipe_buffer(buf: *mut buf_T, aucmd: bool);
    fn get_option_value_for(
        opt_idx: OptIndex,
        opt_flags: c_int,
        scope: OptScope,
        from: *mut c_void,
        err: *mut Error,
    ) -> Object;
    fn get_option(opt_idx: OptIndex) -> *mut vimoption_T;
    fn get_varp_from(p: *mut vimoption_T, buf: *mut buf_T, win: *mut win_T) -> *mut c_void;
    fn optval_from_obj(opt_idx: OptIndex, o: Object, op: set_op_T, error: *mut bool) -> Object;
    fn optval_free(o: Object);
    fn escape_option_str_cmdline(var: *mut c_char) -> *mut c_char;
    fn optval_own(opt_idx: OptIndex, value: Object) -> Object;
    fn opt_from_varp(opt_idx: OptIndex, varp: *mut c_void) -> Object;
    fn get_option_newval(
        opt_idx: OptIndex,
        opt_flags: c_int,
        prefix: set_prefix_T,
        argp: *mut *mut c_char,
        nextchar: c_int,
        op: set_op_T,
        varp: *mut c_void,
        oldval_override: *mut Object,
        errmsg: *mut *const c_char,
    ) -> Object;
    fn validate_and_prepare_option_value(args: *mut optset_T) -> *const c_char;
    fn free_option_prepared(args: *mut optset_T);
    fn set_option_value_for(
        opt_idx: OptIndex,
        value: Object,
        opt_flags: c_int,
        scope: OptScope,
        from: *mut c_void,
        value_replaced: bool,
        err: *mut Error,
    );
    fn optval_to_obj(opt_idx: OptIndex, value: Object, arena: *mut Arena) -> Object;
    fn get_all_options_info(arena: *mut Arena) -> Dict;
    fn get_option_info(
        name: NvimString,
        opt_flags: c_int,
        buf: *mut buf_T,
        win: *mut win_T,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Dict;
}

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
const fn has_key(mask: OptionalKeys, idx: u64) -> bool {
    (mask & (1u64 << idx)) != 0
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn strequal(a: *const c_char, b: &str) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe { CStr::from_ptr(a).to_bytes() == b.as_bytes() }
}

#[inline]
const fn static_cstr_as_obj(s: &'static CStr) -> Object {
    let bytes = s.to_bytes();
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData {
            string: NvimString {
                data: bytes.as_ptr() as *mut c_char,
                size: bytes.len(),
            },
        },
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

unsafe fn with_script_context<F: FnOnce()>(channel_id: u64, f: F) {
    unsafe {
        let save_current_sctx = api_set_sctx(channel_id);
        f();
        current_sctx = save_current_sctx;
    }
}

unsafe fn validate_option_value_args(
    opts: *mut KeyDict_option,
    name: *mut c_char,
    allow_tab: bool,
    opt_idxp: *mut OptIndex,
    opt_flags: *mut c_int,
    scope: *mut OptScope,
    from: *mut *mut c_void,
    filetype: *mut *mut c_char,
    operation: *mut set_op_T,
    dry_run: *mut bool,
    err: *mut Error,
) -> c_int {
    unsafe {
        let mask = (*opts).is_set__option_;
        if has_key(mask, KEYSET_OPTIDX_option__filetype)
            && (has_key(mask, KEYSET_OPTIDX_option__scope)
                || has_key(mask, KEYSET_OPTIDX_option__buf)
                || has_key(mask, KEYSET_OPTIDX_option__win)
                || has_key(mask, KEYSET_OPTIDX_option__tab))
        {
            api_err_conflict(
                err,
                c"filetype".as_ptr(),
                c"'scope', 'buf', 'win' or 'tab'".as_ptr(),
            );
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) && !allow_tab {
            api_err_conflict(err, c"tab".as_ptr(), c"this function".as_ptr());
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab)
            && (has_key(mask, KEYSET_OPTIDX_option__win)
                || has_key(mask, KEYSET_OPTIDX_option__buf)
                || has_key(mask, KEYSET_OPTIDX_option__filetype)
                || has_key(mask, KEYSET_OPTIDX_option__scope))
        {
            api_err_conflict(
                err,
                c"tab".as_ptr(),
                c"'win', 'buf', 'filetype' or 'scope'".as_ptr(),
            );
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__win) && has_key(mask, KEYSET_OPTIDX_option__buf) {
            api_err_conflict(err, c"buf".as_ptr(), c"win".as_ptr());
            return FAIL;
        }

        if has_key(mask, KEYSET_OPTIDX_option__scope) {
            if strequal((*opts).scope.data, "local") {
                *opt_flags = OPT_LOCAL;
            } else if strequal((*opts).scope.data, "global") {
                *opt_flags = OPT_GLOBAL;
            } else {
                api_err_exp(
                    err,
                    c"scope".as_ptr(),
                    c"'local' or 'global'".as_ptr(),
                    ptr::null(),
                );
                return FAIL;
            }
        }

        *scope = kOptScopeGlobal;

        if !filetype.is_null() && has_key(mask, KEYSET_OPTIDX_option__filetype) {
            *filetype = (*opts).filetype.data;
        }

        if has_key(mask, KEYSET_OPTIDX_option__win) {
            *scope = kOptScopeWin;
            *from = find_window_by_handle((*opts).win, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_option__buf) {
            if has_key(mask, KEYSET_OPTIDX_option__scope) && *opt_flags == OPT_GLOBAL {
                api_err_conflict(err, c"buf".as_ptr(), c"global scope".as_ptr());
                return FAIL;
            }
            *opt_flags = OPT_LOCAL;
            *scope = kOptScopeBuf;
            *from = find_buffer_by_handle((*opts).buf, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) {
            *scope = kOptScopeTab;
            *from = find_tab_by_handle((*opts).tab, err).cast();
            if error_set(err) {
                return FAIL;
            }
        }

        *opt_idxp = find_option(name);
        if *opt_idxp == kOptInvalid {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"Unknown option '%s'".as_ptr(),
                name,
            );
            return FAIL;
        }

        if !operation.is_null() && has_key(mask, KEYSET_OPTIDX_option__operation) {
            if strequal((*opts).operation.data, "set") {
                *operation = OP_NONE;
            } else if strequal((*opts).operation.data, "append") {
                *operation = OP_ADDING;
            } else if strequal((*opts).operation.data, "prepend") {
                *operation = OP_PREPENDING;
            } else if strequal((*opts).operation.data, "remove") {
                *operation = OP_REMOVING;
            } else {
                api_err_exp(
                    err,
                    c"operation".as_ptr(),
                    c"'set', 'append', 'prepend', or 'remove'".as_ptr(),
                    ptr::null(),
                );
                return FAIL;
            }

            if !(*operation == OP_NONE
                || option_has_type(*opt_idxp, ObjectType::kObjectTypeString)
                || option_has_type(*opt_idxp, ObjectType::kObjectTypeInteger))
            {
                api_err_conflict(err, (*opts).operation.data, c"boolean options".as_ptr());
                return FAIL;
            }
        }

        if !dry_run.is_null() && has_key(mask, KEYSET_OPTIDX_option__dry_run) {
            *dry_run = (*opts).dry_run;
        }

        if has_key(mask, KEYSET_OPTIDX_option__tab) && !option_has_scope(*opt_idxp, kOptScopeTab) {
            api_err_conflict(err, c"tab".as_ptr(), name);
            return FAIL;
        }

        if (*scope == kOptScopeBuf || *scope == kOptScopeWin)
            && !option_has_scope(*opt_idxp, *scope)
        {
            let tgt = if *scope == kOptScopeBuf {
                c"buf".as_ptr()
            } else {
                c"win".as_ptr()
            };
            let global = if option_has_scope(*opt_idxp, kOptScopeGlobal) {
                c"global ".as_ptr()
            } else {
                c"".as_ptr()
            };
            let req = if option_has_scope(*opt_idxp, kOptScopeBuf) {
                c"buffer-local ".as_ptr()
            } else if option_has_scope(*opt_idxp, kOptScopeWin) {
                c"window-local ".as_ptr()
            } else {
                c"".as_ptr()
            };

            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"'%s' cannot be passed for %s%soption '%s'".as_ptr(),
                tgt,
                global,
                req,
                name,
            );
            return FAIL;
        }

        if error_set(err) { FAIL } else { OK }
    }
}

unsafe fn do_ft_buf(filetype: *const c_char, aco: *mut CtxSwitch, err: *mut Error) -> *mut buf_T {
    unsafe {
        if filetype.is_null() {
            return ptr::null_mut();
        }

        let ftbuf = buflist_new(ptr::null_mut(), ptr::null_mut(), 1, BLN_DUMMY);
        if ftbuf.is_null() {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Could not create internal buffer".as_ptr(),
            );
            return ptr::null_mut();
        }

        if ml_open(ftbuf) == FAIL {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Could not load internal buffer".as_ptr(),
            );
            return ftbuf;
        }

        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, ftbuf);

        ctx_switch(aco, ptr::null_mut(), ptr::null_mut(), ftbuf.cast(), 0);

        set_option_direct(
            kOptBufhidden,
            static_cstr_as_obj(c"hide"),
            OPT_LOCAL,
            SID_NONE,
        );
        set_option_direct(
            kOptBuftype,
            static_cstr_as_obj(c"nofile"),
            OPT_LOCAL,
            SID_NONE,
        );
        assert!((*(*ftbuf).b_ml.ml_mfp).mf_fd < 0);
        *(get_option_varp_scope_from(kOptSwapfile, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(kOptModeline, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut c_int) = 0;
        *(get_option_varp_scope_from(kOptFiletype, OPT_LOCAL, ftbuf, ptr::null_mut())
            as *mut *mut c_char) = xstrdup(filetype);

        if !has_event(EVENT_FILETYPE) {
            return ftbuf;
        }

        let mut did_au_ft = false;
        try_wrap(err, || {
            did_au_ft = do_filetype_autocmd(ftbuf, true);
        });

        if !bufref_valid(&mut bufref) {
            if !error_set(err) {
                api_set_error(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Internal buffer was deleted".as_ptr(),
                );
            }
            return ptr::null_mut();
        }

        if !did_au_ft && !error_set(err) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"Could not execute FileType autocommands".as_ptr(),
            );
        }
        ftbuf
    }
}

unsafe fn wipe_ft_buf(buf: *mut buf_T) {
    unsafe {
        block_autocmds();

        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&mut bufref, buf);

        close_windows(buf, false);
        if bufref_valid(&mut bufref) && buf != curbuf && (*buf).b_nwindows == 0 {
            wipe_buffer(buf, false);
        }
        if bufref_valid(&mut bufref) {
            (*buf).b_flags &= !BF_DUMMY;
        }

        unblock_autocmds();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_option_value(
    name: NvimString,
    opts: *mut KeyDict_option,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut from: *mut c_void = ptr::null_mut();
        let mut filetype: *mut c_char = ptr::null_mut();

        if validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            &mut filetype,
            ptr::null_mut(),
            ptr::null_mut(),
            err,
        ) == 0
        {
            return NIL;
        }

        let mut aco: CtxSwitch = std::mem::zeroed();

        let ftbuf = do_ft_buf(filetype, &mut aco, err);
        if error_set(err) {
            ctx_restore(&mut aco);
            if !ftbuf.is_null() {
                wipe_ft_buf(ftbuf);
            }
            return NIL;
        }

        if !ftbuf.is_null() {
            assert!(from.is_null());
            from = ftbuf.cast();
        }

        let value = get_option_value_for(opt_idx, opt_flags, scope, from, err);

        ctx_restore(&mut aco);
        if !ftbuf.is_null() {
            wipe_ft_buf(ftbuf);
        }

        if error_set(err) {
            api_free_object(value);
            return NIL;
        }

        value
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_option_value(
    channel_id: u64,
    name: NvimString,
    value: Object,
    opts: *mut KeyDict_option,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut operation: set_op_T = OP_NONE;
        let mut to: *mut c_void = ptr::null_mut();
        let mut dry_run = false;

        if validate_option_value_args(
            opts,
            name.data,
            true,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut to,
            ptr::null_mut(),
            &mut operation,
            &mut dry_run,
            err,
        ) == 0
        {
            return NIL;
        }

        if scope == kOptScopeWin && opt_flags == 0 && option_has_scope(opt_idx, kOptScopeGlobal) {
            opt_flags = OPT_LOCAL;
        }

        let mut merged_val = NIL;
        let mut errmsg: *const c_char = ptr::null();
        let option = get_option(opt_idx);
        let is_array = value.obj_type == ObjectType::kObjectTypeArray;
        let is_dict = value.obj_type == ObjectType::kObjectTypeDict;

        let buf = if scope == kOptScopeBuf {
            to as *mut buf_T
        } else {
            curbuf
        };
        let win = if scope == kOptScopeWin {
            to as *mut win_T
        } else {
            curwin
        };
        let varp = get_varp_from(option, buf, win);

        let nitems = if is_array {
            value.data.array.size
        } else if is_dict {
            value.data.dict.size
        } else {
            0
        };
        let per_item = operation == OP_REMOVING && nitems > 0;
        let count = if per_item { nitems } else { 1 };

        for i in 0..count {
            let mut item = value;
            if per_item && is_array {
                item = Object {
                    obj_type: ObjectType::kObjectTypeArray,
                    data: ObjectData {
                        array: Array {
                            items: value.data.array.items.add(i),
                            size: 1,
                            capacity: 0,
                        },
                    },
                };
            } else if per_item {
                item = Object {
                    obj_type: ObjectType::kObjectTypeDict,
                    data: ObjectData {
                        dict: Dict {
                            items: value.data.dict.items.add(i),
                            size: 1,
                            capacity: 0,
                        },
                    },
                };
            }

            let mut error = false;
            let optval_right = optval_from_obj(opt_idx, item, operation, &mut error);

            if error {
                api_err_exp(
                    err,
                    name.data,
                    c"a valid type".as_ptr(),
                    api_typename(value.obj_type),
                );
                optval_free(merged_val);
                return NIL;
            }

            let mut argp: *mut c_char = ptr::null_mut();

            match optval_right.obj_type {
                ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {}
                ObjectType::kObjectTypeString => {
                    let optval_escaped = escape_option_str_cmdline(optval_right.data.string.data);
                    let escaped_bytes = CStr::from_ptr(optval_escaped).to_bytes();
                    let mut buf_bytes = Vec::with_capacity(1 + escaped_bytes.len());
                    buf_bytes.push(b'=');
                    buf_bytes.extend_from_slice(escaped_bytes);
                    argp = arena_memdupz(arena, buf_bytes.as_ptr().cast(), buf_bytes.len());
                    xfree(optval_escaped.cast());
                }
                ObjectType::kObjectTypeInteger => {
                    let s = format!("={}", optval_right.data.integer);
                    argp = arena_memdupz(arena, s.as_ptr().cast(), s.len());
                }
                ObjectType::kObjectTypeBoolean => {
                    merged_val = optval_right;
                }
                ObjectType::kObjectTypeLuaRef => {
                    merged_val = copy_object(optval_right, ptr::null_mut());
                }
                _ => {
                    std::process::abort();
                }
            }

            optval_free(optval_right);

            if optval_right.obj_type == ObjectType::kObjectTypeInteger
                || optval_right.obj_type == ObjectType::kObjectTypeString
            {
                let mut oldval = if i == 0 {
                    optval_own(opt_idx, opt_from_varp(opt_idx, varp))
                } else {
                    merged_val
                };
                merged_val = get_option_newval(
                    opt_idx,
                    opt_flags,
                    PREFIX_NONE,
                    &mut argp,
                    0,
                    operation,
                    varp,
                    &mut oldval,
                    &mut errmsg,
                );
                optval_free(oldval);
                if !errmsg.is_null() {
                    api_set_error(err, ErrorType::kErrorTypeValidation, c"%s".as_ptr(), errmsg);
                    optval_free(merged_val);
                    return NIL;
                }
            }
        }

        if dry_run {
            let mut raw_buf = [0 as c_char; IOSIZE];
            let errbuf = CharBuf {
                data: raw_buf.as_mut_ptr(),
                size: IOSIZE,
            };
            let mut args = optset_T {
                os_varp: ptr::null_mut(),
                os_idx: opt_idx,
                os_flags: opt_flags,
                os_oldval: NIL,
                os_newval: merged_val,
                os_prep: OptSetPrep {
                    ptr: ptr::null_mut(),
                },
                os_prep_free: None,
                os_value_checked: false,
                os_value_changed: false,
                os_restore_chartab: false,
                os_errbuf: &errbuf,
                os_win: win,
                os_buf: buf,
            };
            errmsg = validate_and_prepare_option_value(&mut args);
            merged_val = args.os_newval;
            free_option_prepared(&mut args);
            if !errmsg.is_null() {
                api_set_error(err, ErrorType::kErrorTypeException, c"%s".as_ptr(), errmsg);
                optval_free(merged_val);
                return NIL;
            }
        } else {
            with_script_context(channel_id, || {
                set_option_value_for(
                    opt_idx,
                    merged_val,
                    opt_flags,
                    scope,
                    to,
                    operation == OP_NONE,
                    err,
                );
            });
        }

        let rv = optval_to_obj(opt_idx, merged_val, arena);
        optval_free(merged_val);
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_all_options_info(arena: *mut Arena, _err: *mut Error) -> Dict {
    unsafe { get_all_options_info(arena) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_option_info2(
    name: NvimString,
    opts: *mut KeyDict_option,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut opt_idx: OptIndex = 0;
        let mut opt_flags: c_int = 0;
        let mut scope: OptScope = kOptScopeGlobal;
        let mut from: *mut c_void = ptr::null_mut();

        if validate_option_value_args(
            opts,
            name.data,
            false,
            &mut opt_idx,
            &mut opt_flags,
            &mut scope,
            &mut from,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            err,
        ) == 0
        {
            return Dict::new();
        }

        let buf = if scope == kOptScopeBuf {
            from as *mut buf_T
        } else {
            curbuf
        };
        let win = if scope == kOptScopeWin {
            from as *mut win_T
        } else {
            curwin
        };

        get_option_info(name, opt_flags, buf, win, arena, err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyset_bitmask_and_strequal_helpers() {
        let mask = (1u64 << KEYSET_OPTIDX_option__buf) | (1u64 << KEYSET_OPTIDX_option__scope);
        assert!(has_key(mask, KEYSET_OPTIDX_option__buf));
        assert!(has_key(mask, KEYSET_OPTIDX_option__scope));
        assert!(!has_key(mask, KEYSET_OPTIDX_option__win));
        unsafe {
            assert!(strequal(c"local".as_ptr(), "local"));
            assert!(!strequal(c"local".as_ptr(), "global"));
            assert!(!strequal(ptr::null(), "local"));
        }
    }
}
