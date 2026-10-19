//! Editor state context and temporary window/buffer context switching (`src/nvim/context.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc
)]

use crate::globals::*;
use crate::option_vars::*;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::api::extmark::nvim_create_namespace;
use crate::api::options::CtxSwitch;
use crate::api::private::converter::object_to_vim;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, Object, ObjectData, ObjectType,
};
use crate::api::tabpage::{WinConfig, pos_T, tabpage_T, win_T};
use crate::api::vimscript::{KeyDict_exec_opts, exec_impl};
use crate::clipboard::{VarType, typval_T, yankreg_T};
use crate::eval::deprecated::{buf_T, fmark_T, fmarkv_T, visualinfo_T};
use crate::eval::encode::encode_vim_list_to_buf;
use crate::hashtab::{hash_init, hashitem_empty, hashtab_T};
use crate::map::{map_del_int_ptr_t, map_put_ref_int_ptr_t, mh_get_int};
use crate::map_glyph_cache::{MH_TOMBSTONE, NvimString};
use crate::memory::{Arena, xfree, xstrdup};
use crate::msgpack_rpc::packer::handle_T;
use crate::shada::{
    shada_encode_buflist, shada_encode_gvars, shada_encode_jumps, shada_encode_regs,
    shada_read_string,
};

pub type colnr_T = c_int;
pub type CtxStateFlags = c_int;
pub type CtxLoadFlags = c_int;
pub type CtxSwitchFlags = c_int;
pub type CdCause = c_int;

const OK: c_int = 1;
const MAXLNUM: i32 = 0x7fffffff;
const MAXPATHL: usize = 4096;
const K_SPECIAL: u8 = 0x80;
const NUM_SAVED_REGISTERS: c_int = 37;
const VIML_INTERNAL_CALL: u64 = 1u64 << 63;

const BCO_ENTER: c_int = 1;
const BCO_NOHELP: c_int = 4;

const kCdCauseWindow: CdCause = 1;

const kCtxRegs: CtxStateFlags = 1;
const kCtxJumps: CtxStateFlags = 2;
const kCtxBufs: CtxStateFlags = 4;
const kCtxGVars: CtxStateFlags = 8;
const kCtxSFuncs: CtxStateFlags = 16;
const kCtxFuncs: CtxStateFlags = 32;
const kCtxCursor: CtxStateFlags = 64;
const kCtxMarks: CtxStateFlags = 128;
const kCtxVisual: CtxStateFlags = 256;

const kCtxMergeReg: CtxLoadFlags = 1;

const kCtxKeepCwd: CtxSwitchFlags = 1;
const kCtxKeepDirs: CtxSwitchFlags = 2;
const kCtxNoDisplay: CtxSwitchFlags = 4;
const kCtxNoEvents: CtxSwitchFlags = 8;
const kCtxValidate: CtxSwitchFlags = 16;

const kCtxSwitchNone: c_int = 0;
const kCtxSwitchWin: c_int = 1;
const kCtxSwitchBuf: c_int = 2;
const kCtxSwitchDirs: c_int = 3;

const kShaDaWantInfo: c_int = 1;
const kShaDaForceit: c_int = 4;
const kShaDaNanos: c_int = 32;
const kShaDaNoHistory: c_int = 64;
const kShaDaNoOpt: c_int = 128;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    pub pos: pos_T,
    pub mark: u32,
    pub curswant: colnr_T,
    pub visual_mode_eval: c_int,
    pub visual: visualinfo_T,
    pub op_start: pos_T,
    pub op_end: pos_T,
    pub last_change: fmark_T,
    pub last_insert: fmark_T,
    pub pos_marks: [u32; 6],
    pub buf: handle_T,
    pub regs: NvimString,
    pub jumps: NvimString,
    pub bufs: NvimString,
    pub gvars: NvimString,
    pub funcs: Array,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CtxWin {
    pub cw_win: *mut win_T,
    pub cw_used: bool,
}

const INIT_POS: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

const INIT_FMARK: fmark_T = fmark_T {
    mark: INIT_POS,
    fnum: 0,
    timestamp: 0,
    view: fmarkv_T {
        topline_offset: MAXLNUM,
        skipcol: 0,
    },
    additional_data: ptr::null_mut(),
};

const CONTEXT_INIT: Context = Context {
    pos: INIT_POS,
    mark: 0,
    curswant: -1,
    visual_mode_eval: 0,
    visual: visualinfo_T {
        vi_start: INIT_POS,
        vi_end: INIT_POS,
        vi_mode: 0,
        vi_curswant: 0,
    },
    op_start: INIT_POS,
    op_end: INIT_POS,
    last_change: INIT_FMARK,
    last_insert: INIT_FMARK,
    pos_marks: [0; 6],
    buf: 0,
    regs: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    jumps: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    bufs: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    gvars: NvimString {
        data: ptr::null_mut(),
        size: 0,
    },
    funcs: Array::new(),
};

static mut CTX_SWITCH_DEPTH: c_int = 0;
static mut CTX_SAVED_CURWIN: handle_T = 0;
static mut CTX_DID_CHDIR: bool = false;
static mut CTX_MARKS_NS_ID: u32 = 0;

unsafe extern "C" {

    fn extmark_del_id(buf: *mut buf_T, ns_id: u32, id: u32) -> bool;
    fn extmark_set_pos(
        buf: *mut buf_T,
        ns_id: u32,
        mark: *mut u32,
        pos: pos_T,
        right_gravity: bool,
        no_undo: bool,
        ui_watched: bool,
    );
    fn extmark_get_pos(buf: *mut buf_T, ns_id: u32, id: u32, pos: *mut pos_T) -> bool;
    fn api_free_string(value: NvimString);
    fn api_free_array(value: Array);
    fn api_clear_error(value: *mut Error);
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn check_pos(buf: *mut buf_T, pos: *mut pos_T);
    fn func_tbl_get() -> *mut hashtab_T;
    fn free_register(reg: *mut yankreg_T);
    fn get_y_register(reg: c_int) -> *mut yankreg_T;
    fn do_cmdline_cmd(cmd: *const c_char) -> c_int;
    fn tv_clear(varp: *mut typval_T);
    fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict;
    fn string_to_array(input: NvimString, crlf: bool, arena: *mut Arena) -> Array;
    fn copy_array(array: Array, arena: *mut Arena) -> Array;
    fn copy_object(obj: Object, arena: *mut Arena) -> Object;
    fn bufref_valid(bufref: *mut crate::api::options::bufref_T) -> bool;
    fn set_bufref(bufref: *mut crate::api::options::bufref_T, buf: *mut buf_T);
    fn update_cwd(cause: CdCause);
    fn os_dirname(buf: *mut c_char, len: usize) -> c_int;
    fn do_autochdir();
    fn os_chdir(path: *const c_char) -> c_int;
    fn shorten_fnames(force: c_int);
    fn win_alloc_ctx_win(idx: c_int);
    fn win_init_empty(wp: *mut win_T);
    fn win_append(after: *mut win_T, wp: *mut win_T, tp: *mut tabpage_T);
    fn win_config_float(wp: *mut win_T, fconfig: WinConfig);
    fn block_autocmds();
    fn unblock_autocmds();
    fn goto_tabpage_tp(
        tp: *mut tabpage_T,
        trigger_enter_autocmds: bool,
        trigger_leave_autocmds: bool,
    );
    fn win_goto(wp: *mut win_T);
    fn win_remove(wp: *mut win_T, tp: *mut tabpage_T);
    fn valid_tabpage_win(tpc: *mut tabpage_T) -> c_int;
    fn close_tabpage(tab: *mut tabpage_T);
    fn win_find_by_handle(handle: handle_T) -> *mut win_T;
    fn win_valid(win: *const win_T) -> bool;
    fn bt_prompt(buf: *mut buf_T) -> bool;
    fn unuse_tabpage(tp: *mut tabpage_T);
    fn use_tabpage(tp: *mut tabpage_T);
    fn leaving_window(win: *mut win_T);
    fn entering_window(win: *mut win_T);
    fn buf_copy_options(buf: *mut buf_T, flags: c_int);
    fn check_cursor(wp: *mut win_T);
    fn valid_tabpage(tpc: *mut tabpage_T) -> bool;
    fn vars_clear(ht: *mut hashtab_T);
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
const fn cstr_literal(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
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
const fn equalpos(a: pos_T, b: pos_T) -> bool {
    a.lnum == b.lnum && a.col == b.col && a.coladd == b.coladd
}

#[inline]
unsafe fn put_c(dict: &mut Dict, key: &'static CStr, value: Object) {
    unsafe {
        *dict.items.add(dict.size) = KeyValuePair {
            key: cstr_literal(key),
            value,
        };
        dict.size += 1;
    }
}

#[inline]
pub unsafe fn handle_get_buffer(h: handle_T) -> *mut buf_T {
    unsafe {
        let k = mh_get_int(&raw mut buffer_handles.set, h);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*buffer_handles.values.add(k as usize)).cast()
        }
    }
}

#[inline]
pub unsafe fn handle_get_window(h: handle_T) -> *mut win_T {
    unsafe {
        let k = mh_get_int(&raw mut window_handles.set, h);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*window_handles.values.add(k as usize)).cast()
        }
    }
}

unsafe fn ctx_marks_ns() -> u32 {
    unsafe {
        if CTX_MARKS_NS_ID == 0 {
            CTX_MARKS_NS_ID = nvim_create_namespace(cstr_literal(c"nvim.context")) as u32;
        }
        CTX_MARKS_NS_ID
    }
}

unsafe fn ctx_positions(ctx: *mut Context, flags: CtxStateFlags) -> [*mut pos_T; 6] {
    unsafe {
        let visual = (flags & kCtxVisual) != 0;
        let marks = (flags & kCtxMarks) != 0;
        [
            if visual {
                &raw mut (*ctx).visual.vi_start
            } else {
                ptr::null_mut()
            },
            if visual {
                &raw mut (*ctx).visual.vi_end
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut (*ctx).op_start
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut (*ctx).op_end
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut (*ctx).last_change.mark
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut (*ctx).last_insert.mark
            } else {
                ptr::null_mut()
            },
        ]
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_free(ctx: *mut Context) {
    unsafe {
        let buf = handle_get_buffer((*ctx).buf);
        if !buf.is_null() {
            for i in 0..(*ctx).pos_marks.len() {
                if (*ctx).pos_marks[i] != 0 {
                    extmark_del_id(buf, ctx_marks_ns(), (*ctx).pos_marks[i]);
                }
            }
        }
        api_free_string((*ctx).regs);
        api_free_string((*ctx).jumps);
        api_free_string((*ctx).bufs);
        api_free_string((*ctx).gvars);
        api_free_array((*ctx).funcs);
        *ctx = CONTEXT_INIT;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_save(ctx: *mut Context, flags: CtxStateFlags) {
    unsafe {
        assert!((*ctx).buf == 0 || (*ctx).buf == (*curbuf).b_fnum);
        (*ctx).buf = (*curbuf).b_fnum;
        (*ctx).pos = (*curwin).w_cursor;

        if (flags & kCtxCursor) != 0 {
            (*ctx).curswant = if (*curwin).w_set_curswant {
                -1
            } else {
                (*curwin).w_curswant
            };
        }

        if (flags & kCtxVisual) != 0 {
            (*ctx).visual = (*curbuf).b_visual;
            (*ctx).visual_mode_eval = (*curbuf).b_visual_mode_eval;
        }
        if (flags & kCtxMarks) != 0 {
            (*ctx).op_start = (*curbuf).b_op_start;
            (*ctx).op_end = (*curbuf).b_op_end;
            (*ctx).last_change = (*curbuf).b_last_change;
            (*ctx).last_insert = (*curbuf).b_last_insert;
        }
        if (flags & (kCtxVisual | kCtxMarks)) != 0 {
            let pos = ctx_positions(ctx, flags);
            for (i, &pos_ptr) in pos.iter().enumerate() {
                if pos_ptr.is_null() {
                    continue;
                }
                if (*pos_ptr).lnum == 0 {
                    if (*ctx).pos_marks[i] != 0 {
                        extmark_del_id(curbuf, ctx_marks_ns(), (*ctx).pos_marks[i]);
                        (*ctx).pos_marks[i] = 0;
                    }
                    continue;
                }
                let mut p = *pos_ptr;
                check_pos(curbuf, &mut p);
                extmark_set_pos(
                    curbuf,
                    ctx_marks_ns(),
                    &raw mut (*ctx).pos_marks[i],
                    p,
                    true,
                    true,
                    false,
                );
            }
        }

        if (flags & kCtxRegs) != 0 {
            (*ctx).regs = shada_encode_regs(false, 0);
        }
        if (flags & kCtxJumps) != 0 {
            (*ctx).jumps = shada_encode_jumps();
        }
        if (flags & kCtxBufs) != 0 {
            (*ctx).bufs = shada_encode_buflist();
        }
        if (flags & kCtxGVars) != 0 {
            (*ctx).gvars = shada_encode_gvars();
        }

        if (flags & (kCtxFuncs | kCtxSFuncs)) != 0 {
            let scriptonly = (flags & kCtxFuncs) == 0;
            (*ctx).funcs = Array::new();
            let mut err = Error {
                err_type: ErrorType::kErrorTypeNone,
                msg: ptr::null_mut(),
            };

            let ht = func_tbl_get();
            let mut todo = (*ht).ht_used;
            let mut idx = 0usize;
            while todo > 0 {
                let hi = &*(*ht).ht_array.add(idx);
                idx += 1;
                if !hashitem_empty(hi) {
                    todo -= 1;
                    let name_bytes = CStr::from_ptr(hi.hi_key).to_bytes();
                    let islambda = name_bytes.starts_with(b"<lambda>");
                    let isscript = !name_bytes.is_empty() && name_bytes[0] == K_SPECIAL;

                    if !islambda && (!scriptonly || isscript) {
                        let mut cmd_vec = Vec::with_capacity(6 + name_bytes.len() + 1);
                        cmd_vec.extend_from_slice(b"func! ");
                        cmd_vec.extend_from_slice(name_bytes);
                        let cmd_len = cmd_vec.len();
                        cmd_vec.push(0);
                        let mut opts = KeyDict_exec_opts { output: true };
                        let func_body = exec_impl(
                            VIML_INTERNAL_CALL,
                            NvimString {
                                data: cmd_vec.as_mut_ptr().cast::<c_char>(),
                                size: cmd_len,
                            },
                            &mut opts,
                            &mut err,
                        );
                        if !error_set(&err) {
                            (*ctx).funcs.push(string_obj(func_body));
                        }
                        api_clear_error(&mut err);
                    }
                }
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_load(
    ctx: *mut Context,
    flags: CtxStateFlags,
    loadflags: CtxLoadFlags,
) {
    unsafe {
        if (flags & kCtxCursor) != 0 {
            assert!(handle_get_buffer((*ctx).buf) == curbuf);
            (*curwin).w_cursor = (*ctx).pos;
            if (*ctx).curswant >= 0 {
                (*curwin).w_curswant = (*ctx).curswant;
                (*curwin).w_set_curswant = false;
            } else {
                (*curwin).w_set_curswant = true;
            }
        }

        let buf = handle_get_buffer((*ctx).buf);
        if (flags & (kCtxVisual | kCtxMarks)) != 0 && !buf.is_null() {
            let pos = ctx_positions(ctx, flags);
            for (i, &pos_ptr) in pos.iter().enumerate() {
                if pos_ptr.is_null() || (*ctx).pos_marks[i] == 0 {
                    continue;
                }
                extmark_get_pos(buf, ctx_marks_ns(), (*ctx).pos_marks[i], pos_ptr);
            }
            if (flags & kCtxVisual) != 0 {
                (*buf).b_visual = (*ctx).visual;
                (*buf).b_visual_mode_eval = (*ctx).visual_mode_eval;
            }
            if (flags & kCtxMarks) != 0 {
                (*buf).b_op_start = (*ctx).op_start;
                (*buf).b_op_end = (*ctx).op_end;
                (*buf).b_last_change = (*ctx).last_change;
                (*buf).b_last_insert = (*ctx).last_insert;
            }
        }

        if (flags & kCtxRegs) != 0 {
            if (loadflags & kCtxMergeReg) == 0 {
                for i in 0..NUM_SAVED_REGISTERS {
                    free_register(get_y_register(i));
                }
            }
            shada_read_string(
                (*ctx).regs,
                kShaDaWantInfo | kShaDaForceit | kShaDaNanos | kShaDaNoHistory,
            );
        }

        if (flags & kCtxJumps) != 0 {
            shada_read_string(
                (*ctx).jumps,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory,
            );
        }

        if (flags & kCtxBufs) != 0 {
            shada_read_string(
                (*ctx).bufs,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory | kShaDaNoOpt,
            );
        }

        if (flags & kCtxGVars) != 0 {
            shada_read_string(
                (*ctx).gvars,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory | kShaDaNoOpt,
            );
        }

        if (flags & kCtxFuncs) != 0 {
            for i in 0..(*ctx).funcs.size {
                do_cmdline_cmd((*(*ctx).funcs.items.add(i)).data.string.data);
            }
        }
    }
}

unsafe fn array_to_string(array: Array, err: *mut Error) -> NvimString {
    unsafe {
        let mut sbuf = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let mut list_tv: typval_T = std::mem::zeroed();
        object_to_vim(array_obj(array), &mut list_tv, err);

        assert_eq!(list_tv.v_type, VarType::VAR_LIST);
        if !encode_vim_list_to_buf(list_tv.vval.v_list, &mut sbuf.size, &mut sbuf.data) {
            api_set_error(
                err,
                ErrorType::kErrorTypeException,
                c"%s".as_ptr(),
                c"E474: Failed to convert list to msgpack string buffer".as_ptr(),
            );
        }

        tv_clear(&mut list_tv);
        sbuf
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_to_dict(ctx: *mut Context, arena: *mut Arena) -> Dict {
    unsafe {
        assert!(!ctx.is_null());
        let mut rv = arena_dict(arena, 5);
        put_c(
            &mut rv,
            c"regs",
            array_obj(string_to_array((*ctx).regs, false, arena)),
        );
        put_c(
            &mut rv,
            c"jumps",
            array_obj(string_to_array((*ctx).jumps, false, arena)),
        );
        put_c(
            &mut rv,
            c"bufs",
            array_obj(string_to_array((*ctx).bufs, false, arena)),
        );
        put_c(
            &mut rv,
            c"gvars",
            array_obj(string_to_array((*ctx).gvars, false, arena)),
        );
        put_c(
            &mut rv,
            c"funcs",
            array_obj(copy_array((*ctx).funcs, arena)),
        );
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_from_dict(
    dict: Dict,
    ctx: *mut Context,
    err: *mut Error,
) -> CtxStateFlags {
    unsafe {
        assert!(!ctx.is_null());
        let mut types: CtxStateFlags = 0;
        let mut i = 0usize;
        while i < dict.size && !error_set(err) {
            let item = *dict.items.add(i);
            i += 1;
            if item.value.obj_type != ObjectType::kObjectTypeArray {
                continue;
            }
            if strequal(item.key.data, "regs") {
                types |= kCtxRegs;
                (*ctx).regs = array_to_string(item.value.data.array, err);
            } else if strequal(item.key.data, "jumps") {
                types |= kCtxJumps;
                (*ctx).jumps = array_to_string(item.value.data.array, err);
            } else if strequal(item.key.data, "bufs") {
                types |= kCtxBufs;
                (*ctx).bufs = array_to_string(item.value.data.array, err);
            } else if strequal(item.key.data, "gvars") {
                types |= kCtxGVars;
                (*ctx).gvars = array_to_string(item.value.data.array, err);
            } else if strequal(item.key.data, "funcs") {
                types |= kCtxFuncs;
                (*ctx).funcs = copy_object(item.value, ptr::null_mut()).data.array;
            }
        }
        types
    }
}

unsafe fn ctx_win_dirs_save(cs: *mut CtxSwitch, cw_win: *mut win_T, buf: *mut buf_T) {
    unsafe {
        xfree((*cw_win).w_localdir.cast());
        (*cw_win).w_localdir = ptr::null_mut();
        (*cs).cs_b_localdir = (*buf).b_localdir;
        (*buf).b_localdir = ptr::null_mut();
        (*cs).cs_tp_localdir = (*curtab).tp_localdir;
        (*curtab).tp_localdir = ptr::null_mut();
        (*cs).cs_globaldir = globaldir;
        globaldir = ptr::null_mut();
    }
}

unsafe fn ctx_localdirs_restore(
    cs: *mut CtxSwitch,
    cwp: *mut win_T,
    tp: *mut tabpage_T,
    persist: bool,
) {
    unsafe {
        let did_chdir = CTX_DID_CHDIR;
        CTX_DID_CHDIR = persist && did_chdir;

        let dirs_win = handle_get_window((*cs).cs_new_curwin);
        if !dirs_win.is_null() {
            xfree((*dirs_win).w_localdir.cast());
            (*dirs_win).w_localdir = (*cs).cs_w_localdir;
        } else {
            xfree((*cs).cs_w_localdir.cast());
        }

        let b = if bufref_valid(&raw mut (*cs).cs_new_curbuf) {
            (*cs).cs_new_curbuf.br_buf.cast::<buf_T>()
        } else {
            ptr::null_mut()
        };
        if !b.is_null() && !(persist && !(*b).b_localdir.is_null()) {
            xfree((*b).b_localdir.cast());
            (*b).b_localdir = (*cs).cs_b_localdir;
        } else {
            xfree((*cs).cs_b_localdir.cast());
        }

        if !tp.is_null() && !(persist && !(*tp).tp_localdir.is_null()) {
            xfree((*tp).tp_localdir.cast());
            (*tp).tp_localdir = (*cs).cs_tp_localdir;
        } else {
            xfree((*cs).cs_tp_localdir.cast());
        }

        if !cwp.is_null() && (did_chdir || !(*cwp).w_localdir.is_null()) {
            update_cwd(kCdCauseWindow);
        }

        if !(persist && (*cs).cs_globaldir.is_null() && !globaldir.is_null()) {
            xfree(globaldir.cast());
            globaldir = (*cs).cs_globaldir;
            (*cs).cs_globaldir = ptr::null_mut();
        }
    }
}

unsafe fn ctx_dirs_save(cs: *mut CtxSwitch, wp: *mut win_T, tp: *mut tabpage_T, buf: *mut buf_T) {
    unsafe {
        if ((*cs).cs_flags & (kCtxKeepCwd | kCtxKeepDirs)) == 0 {
            return;
        }

        (*cs).cs_globaldir = if globaldir.is_null() {
            ptr::null_mut()
        } else {
            xstrdup(globaldir)
        };

        if ((*cs).cs_flags & kCtxKeepDirs) != 0 {
            let target_buf = if !buf.is_null() { buf } else { (*wp).w_buffer };
            (*cs).cs_dirs_tab = (*tp).handle;
            (*cs).cs_w_localdir = if (*wp).w_localdir.is_null() {
                ptr::null_mut()
            } else {
                xstrdup((*wp).w_localdir)
            };
            (*cs).cs_b_localdir = if (*target_buf).b_localdir.is_null() {
                ptr::null_mut()
            } else {
                xstrdup((*target_buf).b_localdir)
            };
            (*cs).cs_tp_localdir = if (*tp).tp_localdir.is_null() {
                ptr::null_mut()
            } else {
                xstrdup((*tp).tp_localdir)
            };
        }

        let mut cwd = [0 as c_char; MAXPATHL];
        if (((*cs).cs_flags & kCtxKeepDirs) != 0 || curwin != wp)
            && os_dirname(cwd.as_mut_ptr(), MAXPATHL) == OK
        {
            (*cs).cs_cwd = xstrdup(cwd.as_ptr());
        }

        if !(*cs).cs_cwd.is_null() && p_acd != 0 {
            do_autochdir();
            let mut autocwd = [0 as c_char; MAXPATHL];
            if os_dirname(autocwd.as_mut_ptr(), MAXPATHL) == OK {
                (*cs).cs_apply_acd =
                    CStr::from_ptr((*cs).cs_cwd) == CStr::from_ptr(autocwd.as_ptr());
            }
        }
    }
}

unsafe fn ctx_dirs_restore(cs: *mut CtxSwitch) {
    unsafe {
        if (*cs).cs_ctxwin_idx >= 0 {
            return;
        }

        if ((*cs).cs_flags & kCtxKeepDirs) != 0 {
            let mut dirs_tab: *mut tabpage_T = ptr::null_mut();
            let mut tp = first_tabpage;
            while !tp.is_null() {
                if (*tp).handle == (*cs).cs_dirs_tab {
                    dirs_tab = tp;
                    break;
                }
                tp = (*tp).tp_next;
            }
            ctx_localdirs_restore(cs, ptr::null_mut(), dirs_tab, false);
        } else if !(*cs).cs_cwd.is_null() && !CTX_DID_CHDIR {
            xfree(globaldir.cast());
            globaldir = (*cs).cs_globaldir;
            (*cs).cs_globaldir = ptr::null_mut();
        }
        xfree((*cs).cs_globaldir.cast());
        (*cs).cs_globaldir = ptr::null_mut();

        if (*cs).cs_apply_acd {
            do_autochdir();
        } else if !(*cs).cs_cwd.is_null()
            && (((*cs).cs_flags & kCtxKeepDirs) != 0 || !CTX_DID_CHDIR)
        {
            os_chdir((*cs).cs_cwd);
            shorten_fnames(1);
        }
        xfree((*cs).cs_cwd.cast());
        (*cs).cs_cwd = ptr::null_mut();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_did_chdir() {
    unsafe {
        CTX_DID_CHDIR = true;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn is_ctx_win(win: *mut win_T) -> bool {
    unsafe {
        for i in 0..ctx_win_vec.size {
            let item = *ctx_win_vec.items.add(i);
            if item.cw_used && item.cw_win == win {
                return true;
            }
        }
        false
    }
}

unsafe fn ctx_win_prep(cs: *mut CtxSwitch, buf: *mut buf_T) -> *mut win_T {
    unsafe {
        let mut need_append = true;

        let mut idx = 0usize;
        while idx < ctx_win_vec.size {
            if !(*ctx_win_vec.items.add(idx)).cw_used {
                break;
            }
            idx += 1;
        }
        if idx == ctx_win_vec.size {
            let vec_ptr = &raw mut ctx_win_vec;
            (*vec_ptr).push(CtxWin {
                cw_win: ptr::null_mut(),
                cw_used: false,
            });
        }
        if (*ctx_win_vec.items.add(idx)).cw_win.is_null() {
            win_alloc_ctx_win(idx as c_int);
            need_append = false;
        }
        let cw_win = (*ctx_win_vec.items.add(idx)).cw_win;
        (*ctx_win_vec.items.add(idx)).cw_used = true;
        (*cs).cs_ctxwin_idx = idx as c_int;

        (*cw_win).w_buffer = buf;
        (*cw_win).w_s = (&raw mut (*buf).b_s).cast();
        (*buf).b_nwindows += 1;
        win_init_empty(cw_win);

        ctx_win_dirs_save(cs, cw_win, buf);

        if need_append {
            win_append(lastwin, cw_win, ptr::null_mut());
            let val = map_put_ref_int_ptr_t(
                &raw mut window_handles,
                (*cw_win).handle,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            *val = cw_win.cast();
            win_config_float(cw_win, (*cw_win).w_config);
        }

        cw_win
    }
}

unsafe fn ctx_win_rest(cs: *mut CtxSwitch) -> *mut win_T {
    unsafe {
        let cwp = (*ctx_win_vec.items.add((*cs).cs_ctxwin_idx as usize)).cw_win;

        block_autocmds();
        if curwin != cwp {
            'win_found: {
                let mut tp = first_tabpage;
                while !tp.is_null() {
                    let mut wp = if tp == curtab {
                        firstwin
                    } else {
                        (*tp).tp_firstwin
                    };
                    while !wp.is_null() {
                        if wp == cwp {
                            if tp != curtab {
                                goto_tabpage_tp(tp, true, true);
                            }
                            win_goto(cwp);
                            break 'win_found;
                        }
                        wp = (*wp).w_next;
                    }
                    tp = (*tp).tp_next;
                }
            }
        }

        (*curbuf).b_nwindows -= 1;
        win_remove(curwin, ptr::null_mut());
        map_del_int_ptr_t(&raw mut window_handles, (*curwin).handle, ptr::null_mut());

        (*ctx_win_vec.items.add((*cs).cs_ctxwin_idx as usize)).cw_used = false;

        if valid_tabpage_win(curtab) == 0 {
            close_tabpage(curtab);
        }

        unblock_autocmds();
        cwp
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_saved_curwin() -> *mut win_T {
    unsafe {
        if CTX_SAVED_CURWIN == 0 {
            ptr::null_mut()
        } else {
            win_find_by_handle(CTX_SAVED_CURWIN)
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_switch(
    cs: *mut CtxSwitch,
    mut wp: *mut win_T,
    tp: *mut tabpage_T,
    buf: *mut buf_T,
    flags: CtxSwitchFlags,
) -> bool {
    unsafe {
        assert!((wp.is_null() != buf.is_null()) || (wp.is_null() && (flags & kCtxKeepDirs) != 0));
        assert!(buf.is_null() || tp.is_null());
        *cs = std::mem::zeroed();
        (*cs).cs_flags = flags;
        (*cs).cs_mode = if !buf.is_null() {
            kCtxSwitchBuf
        } else if !wp.is_null() {
            kCtxSwitchWin
        } else {
            kCtxSwitchDirs
        };
        (*cs).cs_ctxwin_idx = -1;
        (*cs).cs_did_chdir = CTX_DID_CHDIR;
        CTX_DID_CHDIR = false;
        if (*cs).cs_mode == kCtxSwitchDirs {
            wp = curwin;
        }

        if !buf.is_null() {
            if buf == curbuf {
                wp = curwin;
            } else {
                let mut wp2 = firstwin;
                while !wp2.is_null() {
                    if (*wp2).w_buffer == buf {
                        wp = wp2;
                        break;
                    }
                    wp2 = (*wp2).w_next;
                }
            }
        }

        if (flags & kCtxValidate) != 0 && !wp.is_null() {
            (*cs).cs_target_win = (*wp).handle;
            (*cs).cs_target_old_pos = (*wp).w_cursor;
        }
        if !wp.is_null() {
            ctx_dirs_save(cs, wp, if tp.is_null() { curtab } else { tp }, buf);
        }

        (*cs).cs_curwin = (*curwin).handle;
        (*cs).cs_curwin_xtab = !win_valid(curwin);
        (*cs).cs_prevwin = if prevwin.is_null() {
            0
        } else {
            (*prevwin).handle
        };
        (*cs).cs_same_win = wp == curwin;
        if bt_prompt(curbuf) {
            (*cs).cs_prompt_insert = (*curbuf).b_prompt_insert;
        }
        if !(*cs).cs_same_win {
            (*cs).cs_visual_active = Visual.active;
            Visual.active = false;
        }

        if (flags & kCtxNoEvents) != 0 {
            block_autocmds();
        }
        if !tp.is_null() {
            (*cs).cs_curtab = curtab;
            if (flags & kCtxNoDisplay) != 0 {
                unuse_tabpage(curtab);
                use_tabpage(tp);
            } else {
                goto_tabpage_tp(tp, false, false);
            }
        }

        if !buf.is_null() {
            if wp.is_null() {
                wp = ctx_win_prep(cs, buf);
                leaving_window(curwin);
                buf_copy_options(buf, BCO_ENTER | BCO_NOHELP);
                prevwin = curwin;
            }
            assert!(wp == curwin || win_valid(wp));
        } else if !win_valid(wp) {
            return false;
        }
        curwin = wp;
        curbuf = (*curwin).w_buffer;
        (*cs).cs_new_curwin = (*curwin).handle;
        set_bufref(&raw mut (*cs).cs_new_curbuf, curbuf.cast());

        if (*cs).cs_mode == kCtxSwitchBuf && (*cs).cs_new_curwin != (*cs).cs_curwin {
            if CTX_SWITCH_DEPTH == 0 {
                CTX_SAVED_CURWIN = (*cs).cs_curwin;
            }
            CTX_SWITCH_DEPTH += 1;
        }
        if (flags & kCtxValidate) != 0 {
            check_cursor(curwin);
        }
        true
    }
}

unsafe fn ctx_restore_curwin(cs: *mut CtxSwitch, fallback: *mut win_T) {
    unsafe {
        let mut save_curwin = if (*cs).cs_curwin_xtab {
            handle_get_window((*cs).cs_curwin)
        } else {
            win_find_by_handle((*cs).cs_curwin)
        };
        if save_curwin.is_null() {
            save_curwin = fallback;
        }
        if !save_curwin.is_null() {
            curwin = save_curwin;
            curbuf = (*curwin).w_buffer;
        }
        prevwin = win_find_by_handle((*cs).cs_prevwin);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ctx_restore(cs: *mut CtxSwitch) {
    unsafe {
        if (*cs).cs_mode == kCtxSwitchNone {
            return;
        }

        if (*cs).cs_mode == kCtxSwitchWin {
            if !(*cs).cs_curtab.is_null() && valid_tabpage((*cs).cs_curtab) {
                if ((*cs).cs_flags & kCtxNoDisplay) != 0 {
                    let old_tp_curwin = (*curtab).tp_curwin;
                    unuse_tabpage(curtab);
                    (*curtab).tp_curwin = old_tp_curwin;
                    use_tabpage((*cs).cs_curtab);
                } else {
                    goto_tabpage_tp((*cs).cs_curtab, false, false);
                }
            }
            ctx_restore_curwin(cs, ptr::null_mut());
        } else if (*cs).cs_ctxwin_idx >= 0 {
            let cwp = ctx_win_rest(cs);
            ctx_restore_curwin(cs, firstwin);
            entering_window(curwin);
            if bt_prompt(curbuf) {
                (*curbuf).b_prompt_insert = (*cs).cs_prompt_insert;
            }

            vars_clear(&raw mut (*(*cwp).w_vars).dv_hashtab);
            hash_init(&raw mut (*(*cwp).w_vars).dv_hashtab);

            ctx_localdirs_restore(cs, cwp, curtab, ((*cs).cs_flags & kCtxKeepDirs) == 0);

            if (*curwin).w_topline > (*curbuf).b_ml.ml_line_count {
                (*curwin).w_topline = (*curbuf).b_ml.ml_line_count;
                (*curwin).w_topfill = 0;
            }
        } else if (*cs).cs_mode == kCtxSwitchBuf {
            let saved_buf = (*cs).cs_new_curbuf.br_buf.cast::<buf_T>();
            if (*curwin).handle == (*cs).cs_new_curwin
                && curbuf != saved_buf
                && bufref_valid(&raw mut (*cs).cs_new_curbuf)
                && !(*saved_buf).b_ml.ml_mfp.is_null()
            {
                if (*curwin).w_s == (&raw mut (*curbuf).b_s).cast() {
                    (*curwin).w_s = (&raw mut (*saved_buf).b_s).cast();
                }
                (*curbuf).b_nwindows -= 1;
                curbuf = saved_buf;
                (*curwin).w_buffer = curbuf;
                (*curbuf).b_nwindows += 1;
            }

            ctx_restore_curwin(cs, ptr::null_mut());
        }

        if !(*cs).cs_same_win {
            Visual.active = (*cs).cs_visual_active;
        }
        if (*cs).cs_mode == kCtxSwitchBuf {
            check_cursor(curwin);
            if Visual.active {
                check_pos(curbuf, &raw mut Visual.start);
            }
        }

        if ((*cs).cs_flags & kCtxNoEvents) != 0 {
            unblock_autocmds();
        }
        ctx_dirs_restore(cs);
        if (*cs).cs_ctxwin_idx < 0 && CTX_DID_CHDIR {
            update_cwd(kCdCauseWindow);
        }
        CTX_DID_CHDIR = CTX_DID_CHDIR || (*cs).cs_did_chdir;
        if ((*cs).cs_flags & kCtxValidate) != 0 {
            let wp = win_find_by_handle((*cs).cs_target_win);
            if !wp.is_null() && !equalpos((*cs).cs_target_old_pos, (*wp).w_cursor) {
                (*wp).w_redr_status = true;
            }
            check_cursor(curwin);
            if Visual.active {
                check_pos(curbuf, &raw mut Visual.start);
            }
        }
        if (*cs).cs_mode == kCtxSwitchBuf && (*cs).cs_new_curwin != (*cs).cs_curwin {
            assert!(CTX_SWITCH_DEPTH > 0);
            CTX_SWITCH_DEPTH -= 1;
            if CTX_SWITCH_DEPTH == 0 {
                CTX_SAVED_CURWIN = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_compares_all_three_fields() {
        let a = pos_T {
            lnum: 10,
            col: 5,
            coladd: 0,
        };
        let b = pos_T {
            lnum: 10,
            col: 5,
            coladd: 0,
        };
        let c = pos_T {
            lnum: 10,
            col: 5,
            coladd: 1,
        };
        assert!(equalpos(a, b));
        assert!(!equalpos(a, c));
    }
}
