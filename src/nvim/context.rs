//! Editor state context and temporary window/buffer context switching (`src/nvim/context.c`).

use crate::api::extmark::nvim_create_namespace;

use crate::api::private::converter::object_to_vim;
use crate::api::private::helpers::{
    api_clear_error, api_free_array, api_set_error, arena_dict, array_obj, copy_array, copy_object,
    error_set, put_c, static_cstr_as_string, string_obj, string_to_array,
};
use crate::api::private::validate::{Array, Dict, Error, ErrorType, NvimString, ObjectType};
use crate::api::vimscript::{KeyDict_exec_opts, exec_impl};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{
    BCO_ENTER, BCO_NOHELP, MAXLNUM, bt_prompt, buf_T, bufref_valid, do_autochdir, set_bufref,
};
use crate::cursor::{check_cursor, check_pos, colnr_T, equalpos, pos_T};
use crate::eval::encode::encode_vim_list_to_buf;
use crate::eval::typval::{VarType, tv_clear, typval_T};
use crate::eval::userfunc::func_tbl_get;
use crate::eval::vars::vars_clear;
use crate::ex_docmd::do_cmdline_cmd;
use crate::extmark::{extmark_del_id, extmark_get_pos, extmark_set_pos};
use crate::fileio::shorten_fnames;
use crate::globals::*;
use crate::mark::{fmark_T, fmarkv_T};
use crate::memory::{Arena};
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::visualinfo_T;
use crate::option::buf_copy_options;
use crate::option_vars::*;
use crate::os::fs::{MAXPATHL, os_chdir, os_dirname};
use crate::register::{EMPTY_YANKREG, get_y_register};
use crate::shada::{
    kShaDaForceit, kShaDaNanos, kShaDaNoHistory, kShaDaNoOpt, kShaDaWantInfo, shada_encode_buflist,
    shada_encode_gvars, shada_encode_jumps, shada_encode_regs, shada_read_string,
};
use crate::window::{
    close_tabpage, entering_window, goto_tabpage_tp, leaving_window, tabpage_T, unuse_tabpage,
    update_cwd, use_tabpage, valid_tabpage, valid_tabpage_win, win_T, win_alloc_ctx_win,
    win_append, win_find_by_handle, win_goto, win_init_empty, win_remove, win_valid,
};
use crate::winfloat::win_config_float;
use std::ffi::{c_char, c_int};
use std::ptr;

use crate::api::buffer::VIML_INTERNAL_CALL;
use crate::api::win_config::{kCtxNoDisplay, kCtxNoEvents};
use crate::api::window::{kCtxKeepCwd, kCtxValidate};
use crate::keycodes::K_SPECIAL;
pub type CtxStateFlags = c_int;
pub type CtxLoadFlags = c_int;
pub type CtxSwitchFlags = c_int;
pub type CdCause = c_int;

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
    pub cs_new_curbuf: crate::buffer::bufref_T,
    pub cs_ctxwin_idx: c_int,
    pub cs_target_win: handle_T,
    pub cs_target_old_pos: pos_T,
    pub cs_did_chdir: bool,
    pub cs_dirs_tab: handle_T,
    pub cs_w_localdir: Option<std::ffi::CString>,
    pub cs_b_localdir: Option<std::ffi::CString>,
    pub cs_tp_localdir: Option<std::ffi::CString>,
    pub cs_globaldir: Option<std::ffi::CString>,
    pub cs_cwd: Option<std::ffi::CString>,
    pub cs_apply_acd: bool,
}

impl CtxSwitch {
    pub const fn new() -> Self {
        Self {
            cs_flags: 0,
            cs_mode: 0,
            cs_curwin: 0,
            cs_curwin_xtab: false,
            cs_prevwin: 0,
            cs_curtab: ptr::null_mut(),
            cs_same_win: false,
            cs_visual_active: false,
            cs_prompt_insert: 0,
            cs_new_curwin: 0,
            cs_new_curbuf: crate::buffer::bufref_T::new(),
            cs_ctxwin_idx: 0,
            cs_target_win: 0,
            cs_target_old_pos: pos_T::new(),
            cs_did_chdir: false,
            cs_dirs_tab: 0,
            cs_w_localdir: None,
            cs_b_localdir: None,
            cs_tp_localdir: None,
            cs_globaldir: None,
            cs_cwd: None,
            cs_apply_acd: false,
        }
    }
}

impl Default for CtxSwitch {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

const NUM_SAVED_REGISTERS: c_int = 37;

pub(crate) const kCdCauseWindow: CdCause = 1;

pub(crate) const kCtxRegs: CtxStateFlags = 1;
pub(crate) const kCtxJumps: CtxStateFlags = 2;
pub(crate) const kCtxBufs: CtxStateFlags = 4;
pub(crate) const kCtxGVars: CtxStateFlags = 8;
pub(crate) const kCtxSFuncs: CtxStateFlags = 16;
pub(crate) const kCtxFuncs: CtxStateFlags = 32;
pub(crate) const kCtxCursor: CtxStateFlags = 64;
pub(crate) const kCtxMarks: CtxStateFlags = 128;
pub(crate) const kCtxVisual: CtxStateFlags = 256;

pub(crate) const kCtxMergeReg: CtxLoadFlags = 1;

pub(crate) const kCtxKeepDirs: CtxSwitchFlags = 2;

const kCtxSwitchNone: c_int = 0;
const kCtxSwitchWin: c_int = 1;
const kCtxSwitchBuf: c_int = 2;
const kCtxSwitchDirs: c_int = 3;

#[derive(Clone)]
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
    pub regs: Vec<u8>,
    pub jumps: Vec<u8>,
    pub bufs: Vec<u8>,
    pub gvars: Vec<u8>,
    pub funcs: Array,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CtxWin {
    pub cw_win: *mut win_T,
    pub cw_used: bool,
}

pub const INIT_POS: pos_T = pos_T {
    lnum: 0,
    col: 0,
    coladd: 0,
};

pub const INIT_FMARKV: fmarkv_T = fmarkv_T {
    topline_offset: MAXLNUM,
    skipcol: 0,
};

pub const INIT_FMARK: fmark_T = fmark_T {
    mark: INIT_POS,
    fnum: 0,
    timestamp: 0,
    view: INIT_FMARKV,
    additional_data: None,
};

pub const CONTEXT_INIT: Context = Context {
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
    regs: Vec::new(),
    jumps: Vec::new(),
    bufs: Vec::new(),
    gvars: Vec::new(),
    funcs: Array::new(),
};

struct ContextModuleState {
    CTX_SWITCH_DEPTH: c_int,
    CTX_SAVED_CURWIN: handle_T,
    CTX_DID_CHDIR: bool,
    CTX_MARKS_NS_ID: u32,
}

impl ContextModuleState {
    const fn new() -> Self {
        Self {
            CTX_SWITCH_DEPTH: 0,
            CTX_SAVED_CURWIN: 0,
            CTX_DID_CHDIR: false,
            CTX_MARKS_NS_ID: 0,
        }
    }
}

static CTX_STATE: GlobalState<ContextModuleState> = GlobalState::new(ContextModuleState::new());

#[inline]
fn ctx_st() -> &'static mut ContextModuleState {
    unsafe { &mut *CTX_STATE.as_ptr() }
}

#[inline]
pub fn handle_get_buffer(h: handle_T) -> *mut buf_T {
    unsafe {
        let map_ptr = &raw const g_st().buffer_handles;
        (*map_ptr).get(&h).copied().unwrap_or(ptr::null_mut())
    }
}

#[inline]
pub fn handle_get_window(h: handle_T) -> *mut win_T {
    unsafe {
        let map_ptr = &raw const g_st().window_handles;
        (*map_ptr).get(&h).copied().unwrap_or(ptr::null_mut())
    }
}

fn ctx_marks_ns() -> u32 {
    {
        if ctx_st().CTX_MARKS_NS_ID == 0 {
            ctx_st().CTX_MARKS_NS_ID = nvim_create_namespace(static_cstr_as_string(c"nvim.context")) as u32;
        }
        ctx_st().CTX_MARKS_NS_ID
    }
}

fn ctx_positions(ctx: &mut Context, flags: CtxStateFlags) -> [*mut pos_T; 6] {
     {
        let visual = (flags & kCtxVisual) != 0;
        let marks = (flags & kCtxMarks) != 0;
        [
            if visual {
                &raw mut ctx.visual.vi_start
            } else {
                ptr::null_mut()
            },
            if visual {
                &raw mut ctx.visual.vi_end
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut ctx.op_start
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut ctx.op_end
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut ctx.last_change.mark
            } else {
                ptr::null_mut()
            },
            if marks {
                &raw mut ctx.last_insert.mark
            } else {
                ptr::null_mut()
            },
        ]
    }
}

pub fn ctx_free(ctx: &mut Context) {
    unsafe {
        let buf = handle_get_buffer(ctx.buf);
        if !buf.is_null() {
            for i in 0..ctx.pos_marks.len() {
                if ctx.pos_marks[i] != 0 {
                    extmark_del_id(&mut *buf, ctx_marks_ns(), ctx.pos_marks[i]);
                }
            }
        }
        api_free_array(ctx.funcs);
        *ctx = CONTEXT_INIT;
    }
}

pub fn ctx_save(ctx: &mut Context, flags: CtxStateFlags) {
    unsafe {
        assert!(ctx.buf == 0 || ctx.buf == curbuf_mut().b_fnum);
        ctx.buf = curbuf_mut().b_fnum;
        ctx.pos = curwin_mut().w_cursor;

        if (flags & kCtxCursor) != 0 {
            ctx.curswant = if curwin_mut().w_set_curswant {
                -1
            } else {
                curwin_mut().w_curswant
            };
        }

        if (flags & kCtxVisual) != 0 {
            ctx.visual = curbuf_mut().b_visual;
            ctx.visual_mode_eval = curbuf_mut().b_visual_mode_eval;
        }
        if (flags & kCtxMarks) != 0 {
            ctx.op_start = curbuf_mut().b_op_start;
            ctx.op_end = curbuf_mut().b_op_end;
            ctx.last_change = curbuf_mut().b_last_change.clone();
            ctx.last_insert = curbuf_mut().b_last_insert.clone();
        }
        if (flags & (kCtxVisual | kCtxMarks)) != 0 {
            let pos = ctx_positions(&mut *ctx, flags);
            for (i, &pos_ptr) in pos.iter().enumerate() {
                if pos_ptr.is_null() {
                    continue;
                }
                if (*pos_ptr).lnum == 0 {
                    if ctx.pos_marks[i] != 0 {
                        extmark_del_id(curbuf_mut(), ctx_marks_ns(), ctx.pos_marks[i]);
                        ctx.pos_marks[i] = 0;
                    }
                    continue;
                }
                let mut p = *pos_ptr;
                check_pos(curbuf_mut(), &mut p);
                extmark_set_pos(
                    curbuf_mut(),
                    ctx_marks_ns(),
                    &mut ctx.pos_marks[i],
                    p,
                    true,
                    true,
                    false,
                );
            }
        }

        if (flags & kCtxRegs) != 0 {
            ctx.regs = shada_encode_regs(false, 0);
        }
        if (flags & kCtxJumps) != 0 {
            ctx.jumps = shada_encode_jumps();
        }
        if (flags & kCtxBufs) != 0 {
            ctx.bufs = shada_encode_buflist();
        }
        if (flags & kCtxGVars) != 0 {
            ctx.gvars = shada_encode_gvars();
        }

        if (flags & (kCtxFuncs | kCtxSFuncs)) != 0 {
            let scriptonly = (flags & kCtxFuncs) == 0;
            ctx.funcs = Array::new();
            let mut err = Error::new();

            let ht = func_tbl_get();
            for name_bytes in ht.keys() {
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
                        ctx.funcs.push(string_obj(func_body));
                    }
                    api_clear_error(&mut err);
                }
            }
        }
    }
}

pub fn ctx_load(ctx: &mut Context, flags: CtxStateFlags, loadflags: CtxLoadFlags) {
    unsafe {
        if (flags & kCtxCursor) != 0 {
            assert!(handle_get_buffer(ctx.buf) == g_st().curbuf);
            curwin_mut().w_cursor = ctx.pos;
            if ctx.curswant >= 0 {
                curwin_mut().w_curswant = ctx.curswant;
                curwin_mut().w_set_curswant = false;
            } else {
                curwin_mut().w_set_curswant = true;
            }
        }

        let buf = handle_get_buffer(ctx.buf);
        if (flags & (kCtxVisual | kCtxMarks)) != 0 && !buf.is_null() {
            let pos = ctx_positions(&mut *ctx, flags);
            for (i, &pos_ptr) in pos.iter().enumerate() {
                if pos_ptr.is_null() || ctx.pos_marks[i] == 0 {
                    continue;
                }
                extmark_get_pos( &mut *buf, ctx_marks_ns(), ctx.pos_marks[i], &mut *pos_ptr);
            }
            if (flags & kCtxVisual) != 0 {
                (*buf).b_visual = ctx.visual;
                (*buf).b_visual_mode_eval = ctx.visual_mode_eval;
            }
            if (flags & kCtxMarks) != 0 {
                (*buf).b_op_start = ctx.op_start;
                (*buf).b_op_end = ctx.op_end;
                (*buf).b_last_change = ctx.last_change.clone();
                (*buf).b_last_insert = ctx.last_insert.clone();
            }
        }

        if (flags & kCtxRegs) != 0 {
            if (loadflags & kCtxMergeReg) == 0 {
                for i in 0..NUM_SAVED_REGISTERS {
                    *get_y_register(i) = EMPTY_YANKREG;
                }
            }
            shada_read_string(
                &ctx.regs,
                kShaDaWantInfo | kShaDaForceit | kShaDaNanos | kShaDaNoHistory,
            );
        }

        if (flags & kCtxJumps) != 0 {
            shada_read_string(
                &ctx.jumps,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory,
            );
        }

        if (flags & kCtxBufs) != 0 {
            shada_read_string(
                &ctx.bufs,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory | kShaDaNoOpt,
            );
        }

        if (flags & kCtxGVars) != 0 {
            shada_read_string(
                &ctx.gvars,
                kShaDaWantInfo | kShaDaForceit | kShaDaNoHistory | kShaDaNoOpt,
            );
        }

        if (flags & kCtxFuncs) != 0 {
            for i in 0..ctx.funcs.size {
                do_cmdline_cmd((*ctx.funcs.items.add(i)).data.string().data);
            }
        }
    }
}

fn array_to_string(array: Array, err: &mut Error) -> Vec<u8> {
    unsafe {
        let mut sbuf = Vec::new();
        let mut list_tv: typval_T = typval_T::new();
        object_to_vim(array_obj(array), &mut list_tv, err);

        assert_eq!(list_tv.v_type, VarType::VAR_LIST);
        if let Some(buf) = encode_vim_list_to_buf(list_tv.vval.v_list().as_ref()) {
            sbuf = buf.into_iter().map(|c| c as u8).collect();
        } else {
            api_set_error!(
                err,
                ErrorType::kErrorTypeException,
                c"%s",
                c"E474: Failed to convert list to msgpack string buffer",
            );
        }

        tv_clear(&mut list_tv);
        sbuf
    }
}

pub fn ctx_to_dict(ctx: &mut Context, arena: &mut Arena) -> Dict {
    {
        
        let as_str = |v: &[u8]| NvimString {
            data: v.as_ptr().cast::<c_char>().cast_mut(),
            size: v.len(),
        };
        let mut rv = arena_dict(arena, 5);
        put_c(
            &mut rv,
            c"regs",
            array_obj(string_to_array(as_str(&ctx.regs), false, arena)),
        );
        put_c(
            &mut rv,
            c"jumps",
            array_obj(string_to_array(as_str(&ctx.jumps), false, arena)),
        );
        put_c(
            &mut rv,
            c"bufs",
            array_obj(string_to_array(as_str(&ctx.bufs), false, arena)),
        );
        put_c(
            &mut rv,
            c"gvars",
            array_obj(string_to_array(as_str(&ctx.gvars), false, arena)),
        );
        put_c(
            &mut rv,
            c"funcs",
            array_obj(copy_array(ctx.funcs, Some(arena))),
        );
        rv
    }
}

pub fn ctx_from_dict(dict: Dict, ctx: &mut Context, err: &mut Error) -> CtxStateFlags {
    {
        
        let mut types: CtxStateFlags = 0;
        let mut i = 0usize;
        while i < dict.size && !error_set(err) {
            let item = dict[i];
            i += 1;
            if item.value.obj_type != ObjectType::kObjectTypeArray {
                continue;
            }
            if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"regs") {
                types |= kCtxRegs;
                ctx.regs = array_to_string(item.value.data.array(), err);
            } else if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"jumps") {
                types |= kCtxJumps;
                ctx.jumps = array_to_string(item.value.data.array(), err);
            } else if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"bufs") {
                types |= kCtxBufs;
                ctx.bufs = array_to_string(item.value.data.array(), err);
            } else if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"gvars") {
                types |= kCtxGVars;
                ctx.gvars = array_to_string(item.value.data.array(), err);
            } else if crate::memory::cstr_as_opt_bytes(item.key.data) == Some(b"funcs") {
                types |= kCtxFuncs;
                ctx.funcs = copy_object(item.value, None).data.array();
            }
        }
        types
    }
}

fn ctx_win_dirs_save(cs: &mut CtxSwitch, cw_win: &mut win_T, buf: &mut buf_T) {
     {
        cw_win.w_localdir = None;
        cs.cs_b_localdir = buf.b_localdir.take();
        cs.cs_tp_localdir = curtab_mut().tp_localdir.take();
        cs.cs_globaldir = g_st().globaldir.take();
    }
}

fn ctx_localdirs_restore(cs: &mut CtxSwitch,
    cwp: Option<&win_T>,
    mut tp: Option<&mut tabpage_T>,
    persist: bool,
) {
    unsafe {
        let did_chdir = ctx_st().CTX_DID_CHDIR;
        ctx_st().CTX_DID_CHDIR = persist && did_chdir;

        let dirs_win = handle_get_window(cs.cs_new_curwin);
        if !dirs_win.is_null() {
            (*dirs_win).w_localdir = cs.cs_w_localdir.take();
        } else {
            cs.cs_w_localdir = None;
        }

        let b = if bufref_valid(Some(&cs.cs_new_curbuf)) {
            cs.cs_new_curbuf.br_buf.cast::<buf_T>()
        } else {
            ptr::null_mut()
        };
        if !b.is_null() && !(persist && (*b).b_localdir.is_some()) {
            (*b).b_localdir = cs.cs_b_localdir.take();
        } else {
            cs.cs_b_localdir = None;
        }

        if tp.is_some() && !(persist && tp.as_deref_mut().unwrap().tp_localdir.is_some()) {
            tp.unwrap().tp_localdir = cs.cs_tp_localdir.take();
        } else {
            cs.cs_tp_localdir = None;
        }

        if cwp.is_some() && (did_chdir || cwp.unwrap().w_localdir.is_some()) {
            update_cwd(kCdCauseWindow);
        }

        if !(persist
            && cs.cs_globaldir.is_none()
            && g_st().globaldir.is_some())
        {
            g_st().globaldir = cs.cs_globaldir.take();
        }
    }
}

fn ctx_dirs_save(cs: &mut CtxSwitch, wp: &mut win_T, tp: *mut tabpage_T, buf: *mut buf_T) {
    unsafe {
        if (cs.cs_flags & (kCtxKeepCwd | kCtxKeepDirs)) == 0 {
            return;
        }

        cs.cs_globaldir = g_st().globaldir.clone();

        if (cs.cs_flags & kCtxKeepDirs) != 0 {
            let target_buf = if !buf.is_null() { buf } else { wp.w_buffer };
            cs.cs_dirs_tab = (*tp).handle;
            cs.cs_w_localdir = wp.w_localdir.clone();
            cs.cs_b_localdir = (*target_buf).b_localdir.clone();
            cs.cs_tp_localdir = (*tp).tp_localdir.clone();
        }

        let mut cwd = [0 as c_char; MAXPATHL];
        if ((cs.cs_flags & kCtxKeepDirs) != 0 || curwin_ptr() != (&raw mut *wp))
            && os_dirname(cwd.as_mut_ptr(), MAXPATHL)
        {
            cs.cs_cwd = Some(crate::memory::cstr_to_cstring(cwd.as_ptr()));
        }

        if let Some(ref cs_cwd) = cs.cs_cwd
            && opt_vars().p_acd != 0
        {
            let saved_cwd_bytes = cs_cwd.as_bytes().to_vec();
            do_autochdir();
            let mut autocwd = [0 as c_char; MAXPATHL];
            if os_dirname(autocwd.as_mut_ptr(), MAXPATHL) {
                cs.cs_apply_acd =
                    saved_cwd_bytes.as_slice() == crate::memory::cstr_as_bytes(autocwd.as_ptr());
            }
        }
    }
}

fn ctx_dirs_restore(cs: &mut CtxSwitch) {
    unsafe {
        if cs.cs_ctxwin_idx >= 0 {
            return;
        }

        if (cs.cs_flags & kCtxKeepDirs) != 0 {
            let mut dirs_tab: *mut tabpage_T = ptr::null_mut();
            for tp in for_all_tabpages_ptr() {
                if (*tp).handle == cs.cs_dirs_tab {
                    dirs_tab = tp;
                    break;
                }
            }
            ctx_localdirs_restore(&mut *cs, None, dirs_tab.as_mut(), false);
        } else if cs.cs_cwd.is_some() && !ctx_st().CTX_DID_CHDIR {
            g_st().globaldir = cs.cs_globaldir.take();
        }
        cs.cs_globaldir = None;

        if cs.cs_apply_acd {
            do_autochdir();
        } else if let Some(ref cs_cwd) = cs.cs_cwd
            && ((cs.cs_flags & kCtxKeepDirs) != 0 || !ctx_st().CTX_DID_CHDIR)
        {
            os_chdir(cs_cwd.as_ptr());
            shorten_fnames(1);
        }
        cs.cs_cwd = None;
    }
}

pub fn ctx_did_chdir() {
    ctx_st().CTX_DID_CHDIR = true;
}

pub fn is_ctx_win(win: *mut win_T) -> bool {
    unsafe {
        let vec = &raw const g_st().ctx_win_vec;
        (*vec).iter().any(|item| item.cw_used && item.cw_win == win)
    }
}

fn ctx_win_prep(cs: &mut CtxSwitch, buf: &mut buf_T) -> *mut win_T {
    unsafe {
        let mut need_append = true;
        let vec = &raw mut g_st().ctx_win_vec;

        let idx = (*vec)
            .iter()
            .position(|item| !item.cw_used)
            .unwrap_or_else(|| {
                let idx = (*vec).len();
                (*vec).push(CtxWin {
                    cw_win: ptr::null_mut(),
                    cw_used: false,
                });
                idx
            });
        if (*vec).as_slice()[idx].cw_win.is_null() {
            win_alloc_ctx_win(idx as c_int);
            need_append = false;
        }
        let cw_win = (*vec).as_slice()[idx].cw_win;
        (*vec).as_mut_slice()[idx].cw_used = true;
        cs.cs_ctxwin_idx = idx as c_int;

        (*cw_win).w_buffer = (&raw mut *buf);
        buf.b_nwindows += 1;
        win_init_empty(&mut *cw_win);

        ctx_win_dirs_save(cs, &mut *cw_win, buf);

        if need_append {
            win_append(g_st().lastwin, cw_win, ptr::null_mut());
            let map_ptr = &raw mut g_st().window_handles;
            (*map_ptr).insert((*cw_win).handle, cw_win);
            win_config_float(&mut *cw_win, (*cw_win).w_config.clone());
        }

        cw_win
    }
}

fn ctx_win_rest(cs: &mut CtxSwitch) -> *mut win_T {
    unsafe {
        let vec = &raw mut g_st().ctx_win_vec;
        let cwp = (*vec).as_slice()[cs.cs_ctxwin_idx as usize].cw_win;

        block_autocmds();
        if curwin_ptr() != cwp {
            'win_found: {
                for tp in for_all_tabpages_ptr() {
                    for wp in for_all_windows_in_tab_ptr( &*tp) {
                        if wp == cwp {
                            if tp != g_st().curtab {
                                goto_tabpage_tp(tp, true, true);
                            }
                            win_goto(cwp);
                            break 'win_found;
                        }
                    }
                }
            }
        }

        curbuf_mut().b_nwindows -= 1;
        win_remove(curwin_mut(), ptr::null_mut());
        let map_ptr = &raw mut g_st().window_handles;
        (*map_ptr).remove(&curwin_mut().handle);

        (*vec).as_mut_slice()[cs.cs_ctxwin_idx as usize].cw_used = false;

        if valid_tabpage_win(curtab_mut()) == 0 {
            close_tabpage(curtab_mut());
        }

        unblock_autocmds();
        cwp
    }
}

pub fn ctx_saved_curwin() -> *mut win_T {
    if ctx_st().CTX_SAVED_CURWIN == 0 {
            ptr::null_mut()
        } else {
            win_find_by_handle(ctx_st().CTX_SAVED_CURWIN)
        }
}

pub fn ctx_switch(cs: &mut CtxSwitch,
    mut wp: *mut win_T,
    tp: *mut tabpage_T,
    buf: *mut buf_T,
    flags: CtxSwitchFlags,
) -> bool {
    unsafe {
        assert!((wp.is_null() != buf.is_null()) || (wp.is_null() && (flags & kCtxKeepDirs) != 0));
        assert!(buf.is_null() || tp.is_null());
        *cs = CtxSwitch::new();
        cs.cs_flags = flags;
        cs.cs_mode = if !buf.is_null() {
            kCtxSwitchBuf
        } else if !wp.is_null() {
            kCtxSwitchWin
        } else {
            kCtxSwitchDirs
        };
        cs.cs_ctxwin_idx = -1;
        cs.cs_did_chdir = ctx_st().CTX_DID_CHDIR;
        ctx_st().CTX_DID_CHDIR = false;
        if cs.cs_mode == kCtxSwitchDirs {
            wp = curwin_ptr();
        }

        if !buf.is_null() {
            if buf == g_st().curbuf {
                wp = curwin_ptr();
            } else {
                for wp2 in for_all_windows_ptr() {
                    if (*wp2).w_buffer == buf {
                        wp = wp2;
                        break;
                    }
                }
            }
        }

        if (flags & kCtxValidate) != 0 && !wp.is_null() {
            cs.cs_target_win = (*wp).handle;
            cs.cs_target_old_pos = (*wp).w_cursor;
        }
        if !wp.is_null() {
            ctx_dirs_save(cs,  &mut *wp, if tp.is_null() { g_st().curtab } else { tp }, buf);
        }

        cs.cs_curwin = curwin_mut().handle;
        cs.cs_curwin_xtab = !win_valid(curwin_ptr());
        cs.cs_prevwin = if g_st().prevwin.is_null() {
            0
        } else {
            (*g_st().prevwin).handle
        };
        cs.cs_same_win = wp == curwin_ptr();
        if bt_prompt(Some(crate::globals::curbuf_mut())) {
            cs.cs_prompt_insert = curbuf_mut().b_prompt_insert;
        }
        if !cs.cs_same_win {
            cs.cs_visual_active = g_st().Visual.active;
            g_st().Visual.active = false;
        }

        if (flags & kCtxNoEvents) != 0 {
            block_autocmds();
        }
        if !tp.is_null() {
            cs.cs_curtab = g_st().curtab;
            if (flags & kCtxNoDisplay) != 0 {
                unuse_tabpage(curtab_mut());
                use_tabpage(tp);
            } else {
                goto_tabpage_tp(tp, false, false);
            }
        }

        if !buf.is_null() {
            if wp.is_null() {
                wp = ctx_win_prep(cs,  &mut *buf);
                leaving_window(curwin_mut());
                buf_copy_options( &mut *buf, BCO_ENTER | BCO_NOHELP);
                g_st().prevwin = curwin_ptr();
            }
            assert!(wp == curwin_ptr() || win_valid(wp));
        } else if !win_valid(wp) {
            return false;
        }
        set_curwin(wp);
        g_st().curbuf = curwin_mut().w_buffer;
        cs.cs_new_curwin = curwin_mut().handle;
        set_bufref(&mut cs.cs_new_curbuf, g_st().curbuf.cast());

        if cs.cs_mode == kCtxSwitchBuf && cs.cs_new_curwin != cs.cs_curwin {
            if ctx_st().CTX_SWITCH_DEPTH == 0 {
                ctx_st().CTX_SAVED_CURWIN = cs.cs_curwin;
            }
            ctx_st().CTX_SWITCH_DEPTH += 1;
        }
        if (flags & kCtxValidate) != 0 {
            check_cursor(curwin_mut());
        }
        true
    }
}

fn ctx_restore_curwin(cs: &mut CtxSwitch, fallback: *mut win_T) {
     {
        let mut save_curwin = if cs.cs_curwin_xtab {
            handle_get_window(cs.cs_curwin)
        } else {
            win_find_by_handle(cs.cs_curwin)
        };
        if save_curwin.is_null() {
            save_curwin = fallback;
        }
        if !save_curwin.is_null() {
            set_curwin(save_curwin);
            g_st().curbuf = curwin_mut().w_buffer;
        }
        g_st().prevwin = win_find_by_handle(cs.cs_prevwin);
    }
}

pub fn ctx_restore(cs: &mut CtxSwitch) {
    unsafe {
        if cs.cs_mode == kCtxSwitchNone {
            return;
        }

        if cs.cs_mode == kCtxSwitchWin {
            if !cs.cs_curtab.is_null() && valid_tabpage(cs.cs_curtab) {
                if (cs.cs_flags & kCtxNoDisplay) != 0 {
                    let old_tp_curwin = curtab_mut().tp_curwin;
                    unuse_tabpage(curtab_mut());
                    curtab_mut().tp_curwin = old_tp_curwin;
                    use_tabpage(cs.cs_curtab);
                } else {
                    goto_tabpage_tp(cs.cs_curtab, false, false);
                }
            }
            ctx_restore_curwin(cs, ptr::null_mut());
        } else if cs.cs_ctxwin_idx >= 0 {
            let cwp = ctx_win_rest(cs);
            ctx_restore_curwin(cs, g_st().firstwin);
            entering_window(curwin_mut());
            if bt_prompt(Some(crate::globals::curbuf_mut())) {
                curbuf_mut().b_prompt_insert = cs.cs_prompt_insert;
            }

            vars_clear(&mut (*(*cwp).w_vars()).dv_hashtab);

            let free_dirs = (cs.cs_flags & kCtxKeepDirs) == 0;
            ctx_localdirs_restore(&mut *cs, cwp.as_ref(), Some(crate::globals::curtab_mut()), free_dirs);

            if curwin_mut().w_topline > curbuf_mut().b_ml.ml_line_count {
                curwin_mut().w_topline = curbuf_mut().b_ml.ml_line_count;
                curwin_mut().w_topfill = 0;
            }
        } else if cs.cs_mode == kCtxSwitchBuf {
            let saved_buf = cs.cs_new_curbuf.br_buf.cast::<buf_T>();
            if curwin_mut().handle == cs.cs_new_curwin
                && g_st().curbuf != saved_buf
                && bufref_valid(Some(&cs.cs_new_curbuf))
                && (*saved_buf).b_ml.ml_mfp.is_some()
            {
                curbuf_mut().b_nwindows -= 1;
                g_st().curbuf = saved_buf;
                curwin_mut().w_buffer = g_st().curbuf;
                curbuf_mut().b_nwindows += 1;
            }

            ctx_restore_curwin(cs, ptr::null_mut());
        }

        if !cs.cs_same_win {
            g_st().Visual.active = cs.cs_visual_active;
        }
        if cs.cs_mode == kCtxSwitchBuf {
            check_cursor(curwin_mut());
            if g_st().Visual.active {
                check_pos(curbuf_mut(), &mut g_st().Visual.start);
            }
        }

        if (cs.cs_flags & kCtxNoEvents) != 0 {
            unblock_autocmds();
        }
        ctx_dirs_restore(cs);
        if cs.cs_ctxwin_idx < 0 && ctx_st().CTX_DID_CHDIR {
            update_cwd(kCdCauseWindow);
        }
        ctx_st().CTX_DID_CHDIR = ctx_st().CTX_DID_CHDIR || cs.cs_did_chdir;
        if (cs.cs_flags & kCtxValidate) != 0 {
            let wp = win_find_by_handle(cs.cs_target_win);
            if !wp.is_null() && !equalpos(cs.cs_target_old_pos, (*wp).w_cursor) {
                (*wp).w_redr_status = true;
            }
            check_cursor(curwin_mut());
            if g_st().Visual.active {
                check_pos(curbuf_mut(), &mut g_st().Visual.start);
            }
        }
        if cs.cs_mode == kCtxSwitchBuf && cs.cs_new_curwin != cs.cs_curwin {
            assert!(ctx_st().CTX_SWITCH_DEPTH > 0);
            ctx_st().CTX_SWITCH_DEPTH -= 1;
            if ctx_st().CTX_SWITCH_DEPTH == 0 {
                ctx_st().CTX_SAVED_CURWIN = 0;
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
