//! Editor state context and temporary window/buffer context switching (`src/nvim/context.c`).
#![forbid(unsafe_code)]

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
    BCO_ENTER, BCO_NOHELP, FileBuf, MAXLNUM, bt_prompt, bufref_valid, do_autochdir, set_bufref,
};
use crate::cursor::{Colnr, CursorPos, check_cursor, check_pos, equalpos};
use crate::eval::encode::encode_vim_list_to_buf;
use crate::eval::typval::{TypVal, VarType, tv_clear};
use crate::eval::userfunc::func_tbl_keys;
use crate::eval::vars::{
    VV_COUNT, VV_COUNT1, VV_PREVCOUNT, get_vim_var_nr, set_vim_var_nr, vars_clear,
};
use crate::ex_docmd::do_cmdline_cmd;
use crate::extmark::{extmark_del_id, extmark_get_pos, extmark_set_pos};
use crate::fileio::shorten_fnames;
use crate::globals::*;
use crate::mark::{Fmark, Fmarkv};
use crate::memory::Arena;
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::Visualinfo;
use crate::option::buf_copy_options;
use crate::option_vars::*;
use crate::os::fs::{os_chdir, os_dirname_cstr};
use crate::register::{EMPTY_YANKREG, get_y_register};
use crate::shada::{
    K_SHA_DA_FORCEIT, K_SHA_DA_NANOS, K_SHA_DA_NO_HISTORY, K_SHA_DA_NO_OPT, K_SHA_DA_WANT_INFO,
    shada_encode_buflist, shada_encode_gvars, shada_encode_jumps, shada_encode_regs,
    shada_read_string,
};
use crate::window::{
    TabPageInfo, WinInfoInfo, close_tabpage, entering_window, goto_tabpage_tp, leaving_window,
    unuse_tabpage, update_cwd, use_tabpage, valid_tabpage, valid_tabpage_win, win_alloc_ctx_win,
    win_append, win_find_by_handle, win_goto, win_init_empty, win_remove, win_valid,
};
use crate::winfloat::win_config_float;
use std::ptr;

use crate::api::buffer::VIML_INTERNAL_CALL;
use crate::api::win_config::{K_CTX_NO_DISPLAY, K_CTX_NO_EVENTS};
use crate::api::window::{K_CTX_KEEP_CWD, K_CTX_VALIDATE};
use crate::keycodes::K_SPECIAL;
pub type CtxStateFlags = i32;
pub type CtxLoadFlags = i32;
pub type CtxSwitchFlags = i32;
pub type CdCause = i32;

#[repr(C)]
pub struct CtxSwitch {
    pub cs_flags: i32,
    pub cs_mode: i32,
    pub cs_curwin: ObjHandle,
    pub cs_curwin_xtab: bool,
    pub cs_prevwin: ObjHandle,
    pub cs_curtab: crate::globals::TabpagePtr,
    pub cs_same_win: bool,
    pub cs_visual_active: bool,
    pub cs_prompt_insert: i32,
    pub cs_new_curwin: ObjHandle,
    pub cs_new_curbuf: crate::buffer::Bufref,
    pub cs_ctxwin_idx: i32,
    pub cs_target_win: ObjHandle,
    pub cs_target_old_pos: CursorPos,
    pub cs_did_chdir: bool,
    pub cs_dirs_tab: ObjHandle,
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
            cs_curtab: crate::globals::TabpagePtr::NULL,
            cs_same_win: false,
            cs_visual_active: false,
            cs_prompt_insert: 0,
            cs_new_curwin: 0,
            cs_new_curbuf: crate::buffer::Bufref::new(),
            cs_ctxwin_idx: 0,
            cs_target_win: 0,
            cs_target_old_pos: CursorPos::new(),
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

const NUM_SAVED_REGISTERS: i32 = 37;

pub(crate) const K_CD_CAUSE_WINDOW: CdCause = 1;

pub(crate) const K_CTX_REGS: CtxStateFlags = 1;
pub(crate) const K_CTX_JUMPS: CtxStateFlags = 2;
pub(crate) const K_CTX_BUFS: CtxStateFlags = 4;
pub(crate) const K_CTX_GVARS: CtxStateFlags = 8;
pub(crate) const K_CTX_SFUNCS: CtxStateFlags = 16;
pub(crate) const K_CTX_FUNCS: CtxStateFlags = 32;
pub(crate) const K_CTX_CURSOR: CtxStateFlags = 64;
pub(crate) const K_CTX_MARKS: CtxStateFlags = 128;
pub(crate) const K_CTX_VISUAL: CtxStateFlags = 256;
pub(crate) const K_CTX_COUNT: CtxStateFlags = 512;

pub(crate) const K_CTX_MERGE_REG: CtxLoadFlags = 1;

pub(crate) const K_CTX_KEEP_DIRS: CtxSwitchFlags = 2;

const K_CTX_SWITCH_NONE: i32 = 0;
const K_CTX_SWITCH_WIN: i32 = 1;
const K_CTX_SWITCH_BUF: i32 = 2;
const K_CTX_SWITCH_DIRS: i32 = 3;

#[derive(Clone)]
pub struct Context {
    pub pos: CursorPos,
    pub mark: u32,
    pub curswant: Colnr,
    pub visual_mode_eval: i32,
    pub visual: Visualinfo,
    pub op_start: CursorPos,
    pub op_end: CursorPos,
    pub last_change: Fmark,
    pub last_insert: Fmark,
    pub pos_marks: [u32; 6],
    pub count: i64,
    pub count1: i64,
    pub prevcount: i64,
    pub buf: ObjHandle,
    pub regs: Vec<u8>,
    pub jumps: Vec<u8>,
    pub bufs: Vec<u8>,
    pub gvars: Vec<u8>,
    pub funcs: Array,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CtxWin {
    pub cw_win: crate::globals::WinPtr,
    pub cw_used: bool,
}

pub const INIT_POS: CursorPos = CursorPos {
    lnum: 0,
    col: 0,
    coladd: 0,
};

pub const INIT_FMARKV: Fmarkv = Fmarkv {
    topline_offset: MAXLNUM,
    skipcol: 0,
};

pub const INIT_FMARK: Fmark = Fmark {
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
    visual: Visualinfo {
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
    count: 0,
    count1: 0,
    prevcount: 0,
    buf: 0,
    regs: Vec::new(),
    jumps: Vec::new(),
    bufs: Vec::new(),
    gvars: Vec::new(),
    funcs: Array::new(),
};

struct ContextModuleState {
    ctx_switch_depth: std::cell::Cell<i32>,
    ctx_saved_curwin: std::cell::Cell<ObjHandle>,
    ctx_did_chdir: std::cell::Cell<bool>,
    ctx_marks_ns_id: std::cell::Cell<u32>,
}

impl ContextModuleState {
    const fn new() -> Self {
        Self {
            ctx_switch_depth: std::cell::Cell::new(0),
            ctx_saved_curwin: std::cell::Cell::new(0),
            ctx_did_chdir: std::cell::Cell::new(false),
            ctx_marks_ns_id: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static CTX_STATE: ContextModuleState = const { ContextModuleState::new() };
}

#[inline]
pub fn handle_get_buffer(h: ObjHandle) -> Option<&'static mut FileBuf> {
    g_st()
        .buffer_handles
        .get(&h)
        .copied()
        .and_then(|p| p.opt_mut())
}

#[inline]
pub fn handle_get_buffer_opt(h: ObjHandle) -> Option<&'static mut FileBuf> {
    handle_get_buffer(h)
}

#[inline]
pub fn handle_get_window(h: ObjHandle) -> Option<&'static mut WinInfoInfo> {
    g_st()
        .window_handles
        .get(&h)
        .copied()
        .and_then(|p| p.opt_mut())
}

#[inline]
pub fn handle_get_window_opt(h: ObjHandle) -> Option<&'static mut WinInfoInfo> {
    handle_get_window(h)
}

fn ctx_marks_ns() -> u32 {
    let mut id = CTX_STATE.with(|st| st.ctx_marks_ns_id.get());
    if id == 0 {
        id = nvim_create_namespace(static_cstr_as_string(c"nvim.context")) as u32;
        CTX_STATE.with(|st| st.ctx_marks_ns_id.set(id));
    }
    id
}

fn ctx_positions(ctx: &mut Context, flags: CtxStateFlags) -> [Option<&mut CursorPos>; 6] {
    let visual = (flags & K_CTX_VISUAL) != 0;
    let marks = (flags & K_CTX_MARKS) != 0;
    let Context {
        visual: Visualinfo {
            vi_start, vi_end, ..
        },
        op_start,
        op_end,
        last_change: Fmark { mark: lc_mark, .. },
        last_insert: Fmark { mark: li_mark, .. },
        ..
    } = ctx;
    [
        visual.then_some(vi_start),
        visual.then_some(vi_end),
        marks.then_some(op_start),
        marks.then_some(op_end),
        marks.then_some(lc_mark),
        marks.then_some(li_mark),
    ]
}

pub fn ctx_free(ctx: &mut Context) {
    if let Some(buf) = handle_get_buffer_opt(ctx.buf) {
        for i in 0..ctx.pos_marks.len() {
            if ctx.pos_marks[i] != 0 {
                extmark_del_id(&mut *buf, ctx_marks_ns(), ctx.pos_marks[i]);
            }
        }
    }
    api_free_array(std::mem::take(&mut ctx.funcs));
    *ctx = CONTEXT_INIT;
}

pub fn ctx_save(ctx: &mut Context, flags: CtxStateFlags) {
    if (flags & (K_CTX_CURSOR | K_CTX_VISUAL | K_CTX_MARKS)) != 0 {
        assert!(ctx.buf == 0 || ctx.buf == curbuf_mut().b_fnum.get());
        ctx.buf = curbuf_mut().b_fnum.get();
        ctx.pos = curwin_mut().w_cursor;
    }

    if (flags & K_CTX_CURSOR) != 0 {
        ctx.curswant = if curwin_mut().w_set_curswant.get() {
            -1
        } else {
            curwin_mut().w_curswant.get()
        };
    }

    if (flags & K_CTX_COUNT) != 0 {
        ctx.count = get_vim_var_nr(VV_COUNT);
        ctx.count1 = get_vim_var_nr(VV_COUNT1);
        ctx.prevcount = get_vim_var_nr(VV_PREVCOUNT);
    }

    if (flags & K_CTX_VISUAL) != 0 {
        ctx.visual = curbuf_mut().b_visual;
        ctx.visual_mode_eval = curbuf_mut().b_visual_mode_eval.get();
    }
    if (flags & K_CTX_MARKS) != 0 {
        ctx.op_start = curbuf_mut().b_op_start;
        ctx.op_end = curbuf_mut().b_op_end;
        ctx.last_change = curbuf_mut().b_last_change.clone();
        ctx.last_insert = curbuf_mut().b_last_insert.clone();
    }
    if (flags & (K_CTX_VISUAL | K_CTX_MARKS)) != 0 {
        let mut pos_marks = ctx.pos_marks;
        let pos = ctx_positions(&mut *ctx, flags);
        for (i, pos_opt) in pos.into_iter().enumerate() {
            let Some(pos_ref) = pos_opt else {
                continue;
            };
            if pos_ref.lnum == 0 {
                if pos_marks[i] != 0 {
                    extmark_del_id(curbuf_mut(), ctx_marks_ns(), pos_marks[i]);
                    pos_marks[i] = 0;
                }
                continue;
            }
            let mut p = *pos_ref;
            check_pos(curbuf_mut(), &mut p);
            extmark_set_pos(
                curbuf_mut(),
                ctx_marks_ns(),
                &mut pos_marks[i],
                p,
                true,
                true,
                false,
            );
        }
        ctx.pos_marks = pos_marks;
    }

    if (flags & K_CTX_REGS) != 0 {
        ctx.regs = shada_encode_regs(false, 0);
    }
    if (flags & K_CTX_JUMPS) != 0 {
        ctx.jumps = shada_encode_jumps();
    }
    if (flags & K_CTX_BUFS) != 0 {
        ctx.bufs = shada_encode_buflist();
    }
    if (flags & K_CTX_GVARS) != 0 {
        ctx.gvars = shada_encode_gvars();
    }

    if (flags & (K_CTX_FUNCS | K_CTX_SFUNCS)) != 0 {
        let scriptonly = (flags & K_CTX_FUNCS) == 0;
        ctx.funcs = Array::new();
        let mut err = Error::new();

        for name_bytes in &func_tbl_keys() {
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
                    NvimString::from_bytes(&cmd_vec[..cmd_len]),
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

pub fn ctx_load(ctx: &mut Context, flags: CtxStateFlags, loadflags: CtxLoadFlags) {
    if (flags & K_CTX_CURSOR) != 0 {
        assert!(handle_get_buffer(ctx.buf).is_some_and(|b| g_st().curbuf.get() == b));
        curwin_mut().w_cursor = ctx.pos;
        if ctx.curswant >= 0 {
            curwin_mut().w_curswant.set(ctx.curswant);
            curwin_mut().w_set_curswant.set(false);
        } else {
            curwin_mut().w_set_curswant.set(true);
        }
    }

    if (flags & K_CTX_COUNT) != 0 {
        set_vim_var_nr(VV_COUNT, ctx.count);
        set_vim_var_nr(VV_COUNT1, ctx.count1);
        set_vim_var_nr(VV_PREVCOUNT, ctx.prevcount);
    }

    if (flags & (K_CTX_VISUAL | K_CTX_MARKS)) != 0
        && let Some(buf) = handle_get_buffer_opt(ctx.buf)
    {
        let pos_marks = ctx.pos_marks;
        let pos = ctx_positions(&mut *ctx, flags);
        for (i, pos_opt) in pos.into_iter().enumerate() {
            let Some(pos_ref) = pos_opt else {
                continue;
            };
            if pos_marks[i] == 0 {
                continue;
            }
            extmark_get_pos(&mut *buf, ctx_marks_ns(), pos_marks[i], pos_ref);
        }
        if (flags & K_CTX_VISUAL) != 0 {
            buf.b_visual = ctx.visual;
            buf.b_visual_mode_eval.set(ctx.visual_mode_eval);
        }
        if (flags & K_CTX_MARKS) != 0 {
            buf.b_op_start = ctx.op_start;
            buf.b_op_end = ctx.op_end;
            buf.b_last_change = ctx.last_change.clone();
            buf.b_last_insert = ctx.last_insert.clone();
        }
    }

    if (flags & K_CTX_REGS) != 0 {
        if (loadflags & K_CTX_MERGE_REG) == 0 {
            for i in 0..NUM_SAVED_REGISTERS {
                get_y_register(i).with_mut(|r| *r = EMPTY_YANKREG);
            }
        }
        shada_read_string(
            &ctx.regs,
            K_SHA_DA_WANT_INFO | K_SHA_DA_FORCEIT | K_SHA_DA_NANOS | K_SHA_DA_NO_HISTORY,
        );
    }

    if (flags & K_CTX_JUMPS) != 0 {
        shada_read_string(
            &ctx.jumps,
            K_SHA_DA_WANT_INFO | K_SHA_DA_FORCEIT | K_SHA_DA_NO_HISTORY,
        );
    }

    if (flags & K_CTX_BUFS) != 0 {
        shada_read_string(
            &ctx.bufs,
            K_SHA_DA_WANT_INFO | K_SHA_DA_FORCEIT | K_SHA_DA_NO_HISTORY | K_SHA_DA_NO_OPT,
        );
    }

    if (flags & K_CTX_GVARS) != 0 {
        shada_read_string(
            &ctx.gvars,
            K_SHA_DA_WANT_INFO | K_SHA_DA_FORCEIT | K_SHA_DA_NO_HISTORY | K_SHA_DA_NO_OPT,
        );
    }

    if (flags & K_CTX_FUNCS) != 0 {
        for i in 0..ctx.funcs.size {
            do_cmdline_cmd(ctx.funcs[i].data.string().as_cstr());
        }
    }
}

fn array_to_string(array: Array, err: &mut Error) -> Vec<u8> {
    let mut sbuf = Vec::new();
    let mut list_tv: TypVal = TypVal::new();
    object_to_vim(array_obj(array), &mut list_tv, err);

    assert_eq!(list_tv.v_type, VarType::VarList);
    if let Some(buf) = encode_vim_list_to_buf(list_tv.vval.v_list_opt()) {
        sbuf = buf;
    } else {
        api_set_error!(
            err,
            ErrorType::ErrorTypeException,
            c"%s",
            c"E474: Failed to convert list to msgpack string buffer",
        );
    }

    tv_clear(&mut list_tv);
    sbuf
}

pub fn ctx_to_dict(ctx: &mut Context, arena: &mut Arena) -> Dict {
    let as_str = NvimString::from_bytes;
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
        array_obj(copy_array(ctx.funcs.clone(), Some(arena))),
    );
    rv
}

pub fn ctx_from_dict(dict: Dict, ctx: &mut Context, err: &mut Error) -> CtxStateFlags {
    let mut types: CtxStateFlags = 0;
    let mut i = 0usize;
    while i < dict.size && !error_set(err) {
        let item = &dict[i];
        i += 1;
        if item.value.obj_type != ObjectType::ObjectTypeArray {
            continue;
        }
        if item.key.opt_bytes() == Some(b"regs") {
            types |= K_CTX_REGS;
            ctx.regs = array_to_string(item.value.data.array(), err);
        } else if item.key.opt_bytes() == Some(b"jumps") {
            types |= K_CTX_JUMPS;
            ctx.jumps = array_to_string(item.value.data.array(), err);
        } else if item.key.opt_bytes() == Some(b"bufs") {
            types |= K_CTX_BUFS;
            ctx.bufs = array_to_string(item.value.data.array(), err);
        } else if item.key.opt_bytes() == Some(b"gvars") {
            types |= K_CTX_GVARS;
            ctx.gvars = array_to_string(item.value.data.array(), err);
        } else if item.key.opt_bytes() == Some(b"funcs") {
            types |= K_CTX_FUNCS;
            ctx.funcs = copy_object(item.value.clone(), None).data.array();
        }
    }
    types
}

fn ctx_win_dirs_save(cs: &mut CtxSwitch, cw_win: &mut WinInfoInfo, buf: &mut FileBuf) {
    cw_win.w_localdir = None;
    cs.cs_b_localdir = buf.b_localdir.take();
    cs.cs_tp_localdir = curtab_mut().tp_localdir.replace(None);
    cs.cs_globaldir = g_st().globaldir.take();
}

fn ctx_localdirs_restore(
    cs: &mut CtxSwitch,
    cwp: Option<&WinInfoInfo>,
    tp: Option<&TabPageInfo>,
    persist: bool,
) {
    let did_chdir = CTX_STATE.with(|st| st.ctx_did_chdir.get());
    CTX_STATE.with(|st| st.ctx_did_chdir.set(persist && did_chdir));

    if let Some(dirs_win) = handle_get_window_opt(cs.cs_new_curwin) {
        dirs_win.w_localdir = cs.cs_w_localdir.take();
    } else {
        cs.cs_w_localdir = None;
    }

    let b = cs
        .cs_new_curbuf
        .buf_opt()
        .filter(|_| bufref_valid(Some(&cs.cs_new_curbuf)));
    if let Some(b) = b
        && !(persist && b.b_localdir.get().is_some())
    {
        b.b_localdir.set(cs.cs_b_localdir.take());
    } else {
        cs.cs_b_localdir = None;
    }

    if let Some(tp_ref) = tp
        && !(persist && tp_ref.tp_localdir.get().is_some())
    {
        tp_ref.tp_localdir.set(cs.cs_tp_localdir.take());
    } else {
        cs.cs_tp_localdir = None;
    }

    if cwp.is_some() && (did_chdir || cwp.unwrap().w_localdir.is_some()) {
        update_cwd(K_CD_CAUSE_WINDOW);
    }

    if !(persist && cs.cs_globaldir.is_none() && g_st().globaldir.get().is_some()) {
        g_st().globaldir.set(cs.cs_globaldir.take());
    }
}

fn ctx_dirs_save(
    cs: &mut CtxSwitch,
    wp: &mut WinInfoInfo,
    tp: &TabPageInfo,
    buf: Option<&mut FileBuf>,
) {
    if (cs.cs_flags & (K_CTX_KEEP_CWD | K_CTX_KEEP_DIRS)) == 0 {
        return;
    }

    cs.cs_globaldir = g_st().globaldir.get().clone();

    if (cs.cs_flags & K_CTX_KEEP_DIRS) != 0 {
        let target_buf = buf.unwrap_or_else(|| wp.buf_mut());
        cs.cs_dirs_tab = tp.handle;
        cs.cs_w_localdir = wp.w_localdir.clone();
        cs.cs_b_localdir = target_buf.b_localdir.get().clone();
        cs.cs_tp_localdir = tp.tp_localdir.get();
    }

    if (cs.cs_flags & K_CTX_KEEP_DIRS) != 0 || curwin_ptr() != (&raw mut *wp) {
        cs.cs_cwd = os_dirname_cstr();
    }

    if let Some(ref cs_cwd) = cs.cs_cwd
        && opt_vars().p_acd.get()
    {
        let saved_cwd_bytes = cs_cwd.as_bytes().to_vec();
        do_autochdir();
        if let Some(autocwd) = os_dirname_cstr() {
            cs.cs_apply_acd = saved_cwd_bytes.as_slice() == autocwd.as_bytes();
        }
    }
}

fn ctx_dirs_restore(cs: &mut CtxSwitch) {
    if cs.cs_ctxwin_idx >= 0 {
        return;
    }

    if (cs.cs_flags & K_CTX_KEEP_DIRS) != 0 {
        let dirs_tab = for_all_tabpages_mut().find(|tp| tp.handle == cs.cs_dirs_tab);
        ctx_localdirs_restore(&mut *cs, None, dirs_tab.as_deref(), false);
    } else if cs.cs_cwd.is_some() && !CTX_STATE.with(|st| st.ctx_did_chdir.get()) {
        g_st().globaldir.set(cs.cs_globaldir.take());
    }
    cs.cs_globaldir = None;

    if cs.cs_apply_acd {
        do_autochdir();
    } else if let Some(ref cs_cwd) = cs.cs_cwd
        && ((cs.cs_flags & K_CTX_KEEP_DIRS) != 0 || !CTX_STATE.with(|st| st.ctx_did_chdir.get()))
    {
        os_chdir(cs_cwd);
        shorten_fnames(true);
    }
    cs.cs_cwd = None;
}

pub fn ctx_did_chdir() {
    CTX_STATE.with(|st| st.ctx_did_chdir.set(true));
}

pub fn is_ctx_win(win: &WinInfoInfo) -> bool {
    g_st()
        .ctx_win_vec
        .iter()
        .any(|item| item.cw_used && item.cw_win == ptr::from_ref(win))
}

fn ctx_win_prep(cs: &mut CtxSwitch, buf: &mut FileBuf) -> &'static mut WinInfoInfo {
    let mut need_append = true;
    let idx = g_st()
        .ctx_win_vec
        .iter()
        .position(|item| !item.cw_used)
        .unwrap_or_else(|| {
            let idx = g_st().ctx_win_vec.len();
            g_st().ctx_win_vec.push(CtxWin {
                cw_win: crate::globals::WinPtr::NULL,
                cw_used: false,
            });
            idx
        });
    if g_st().ctx_win_vec.as_slice()[idx].cw_win.is_null() {
        win_alloc_ctx_win(idx as i32);
        need_append = false;
    }
    let cw_win = g_st().ctx_win_vec.as_slice()[idx].cw_win.borrow_mut();
    g_st().ctx_win_vec.as_mut_slice()[idx].cw_used = true;
    cs.cs_ctxwin_idx = idx as i32;

    cw_win.w_buffer = (&raw mut *buf).into();
    buf.b_nwindows.set(buf.b_nwindows.get() + (1));
    win_init_empty(&mut *cw_win);

    ctx_win_dirs_save(cs, &mut *cw_win, buf);

    if need_append {
        win_append(g_st().lastwin.get().opt_mut(), &mut *cw_win, None);
        g_st().window_handles.insert(cw_win.handle, cw_win.into());
        let cfg = cw_win.w_config.clone();
        win_config_float(cw_win, cfg);
    }

    cw_win
}

fn ctx_win_rest(cs: &mut CtxSwitch) -> &'static mut WinInfoInfo {
    let cwp = g_st().ctx_win_vec.as_slice()[cs.cs_ctxwin_idx as usize].cw_win;

    block_autocmds();
    if curwin_ptr() != cwp {
        'win_found: {
            for tp in for_all_tabpages_ptr() {
                for wp in for_all_windows_in_tab_ptr(&tp) {
                    if wp == cwp {
                        if g_st().curtab.get() != tp {
                            goto_tabpage_tp(Some(&tp), true, true);
                        }
                        win_goto(cwp.borrow_mut());
                        break 'win_found;
                    }
                }
            }
        }
    }

    curbuf_mut()
        .b_nwindows
        .set(curbuf_mut().b_nwindows.get() - (1));
    win_remove(curwin_mut(), None);
    g_st().window_handles.remove(&curwin_mut().handle);

    g_st().ctx_win_vec.as_mut_slice()[cs.cs_ctxwin_idx as usize].cw_used = false;

    if valid_tabpage_win(&curtab_mut()) == 0 {
        close_tabpage(&curtab_mut());
    }

    unblock_autocmds();
    cwp.borrow_mut()
}

pub fn ctx_saved_curwin() -> Option<&'static mut WinInfoInfo> {
    let saved = CTX_STATE.with(|st| st.ctx_saved_curwin.get());
    if saved == 0 {
        None
    } else {
        win_find_by_handle(saved)
    }
}

pub fn ctx_switch(
    cs: &mut CtxSwitch,
    wp_opt: Option<&mut WinInfoInfo>,
    tp_opt: Option<&TabPageInfo>,
    buf_opt: Option<&mut FileBuf>,
    flags: CtxSwitchFlags,
) -> bool {
    let mut wp = crate::globals::WinPtr::from(wp_opt);
    let tp = crate::globals::TabpagePtr::from(tp_opt);
    let buf = crate::globals::BufPtr::from(buf_opt);
    assert!((wp.is_null() != buf.is_null()) || (wp.is_null() && (flags & K_CTX_KEEP_DIRS) != 0));
    assert!(buf.is_null() || tp.is_null());
    *cs = CtxSwitch::new();
    cs.cs_flags = flags;
    cs.cs_mode = if !buf.is_null() {
        K_CTX_SWITCH_BUF
    } else if !wp.is_null() {
        K_CTX_SWITCH_WIN
    } else {
        K_CTX_SWITCH_DIRS
    };
    cs.cs_ctxwin_idx = -1;
    cs.cs_did_chdir = CTX_STATE.with(|st| st.ctx_did_chdir.replace(false));
    if cs.cs_mode == K_CTX_SWITCH_DIRS {
        wp = curwin_ptr();
    }

    if !buf.is_null() {
        if buf == g_st().curbuf.get().as_ptr() {
            wp = curwin_ptr();
        } else {
            for wp2 in for_all_windows_ptr() {
                if wp2.w_buffer == buf {
                    wp = wp2.into();
                    break;
                }
            }
        }
    }

    if let Some(wp_ref) = wp.opt_mut() {
        if (flags & K_CTX_VALIDATE) != 0 {
            cs.cs_target_win = wp_ref.handle;
            cs.cs_target_old_pos = wp_ref.w_cursor;
        }
        let ct = curtab_mut();
        ctx_dirs_save(cs, wp_ref, tp.opt().unwrap_or(&ct), buf.opt_mut());
    }

    cs.cs_curwin = curwin_mut().handle;
    cs.cs_curwin_xtab = !win_valid(curwin_ptr().opt());
    cs.cs_prevwin = if g_st().prevwin.get().is_null() {
        0
    } else {
        crate::globals::prevwin_mut().handle
    };
    cs.cs_same_win = wp == curwin_ptr();
    if bt_prompt(Some(crate::globals::curbuf_mut())) {
        cs.cs_prompt_insert = curbuf_mut().b_prompt_insert.get();
    }
    if !cs.cs_same_win {
        cs.cs_visual_active = g_st().visual_global.active;
        g_st().visual_global.active = false;
    }

    if (flags & K_CTX_NO_EVENTS) != 0 {
        block_autocmds();
    }
    if !tp.is_null() {
        cs.cs_curtab = g_st().curtab.get().clone();
        if (flags & K_CTX_NO_DISPLAY) != 0 {
            unuse_tabpage(&curtab_mut());
            use_tabpage(tp.borrow());
        } else {
            goto_tabpage_tp(tp.opt(), false, false);
        }
    }

    if let Some(buf_ref) = buf.opt_mut() {
        if wp.is_null() {
            wp = ctx_win_prep(cs, &mut *buf_ref).into();
            leaving_window(curwin_mut());
            buf_copy_options(&mut *buf_ref, BCO_ENTER | BCO_NOHELP);
            g_st().prevwin.set(curwin_ptr());
        }
        assert!(wp == curwin_ptr() || win_valid(wp.opt()));
    } else if !win_valid(wp.opt()) {
        return false;
    }
    set_curwin(wp);
    g_st().curbuf.set(curwin_mut().w_buffer);
    cs.cs_new_curwin = curwin_mut().handle;
    set_bufref(&mut cs.cs_new_curbuf, g_st().curbuf.get().opt_mut());

    if cs.cs_mode == K_CTX_SWITCH_BUF && cs.cs_new_curwin != cs.cs_curwin {
        CTX_STATE.with(|st| {
            let depth = st.ctx_switch_depth.get();
            if depth == 0 {
                st.ctx_saved_curwin.set(cs.cs_curwin);
            }
            st.ctx_switch_depth.set(depth + 1);
        });
    }
    if (flags & K_CTX_VALIDATE) != 0 {
        check_cursor(curwin_mut());
    }
    true
}

fn ctx_restore_curwin(cs: &mut CtxSwitch, fallback: Option<&'static mut WinInfoInfo>) {
    let mut save_curwin: Option<&'static mut WinInfoInfo> = if cs.cs_curwin_xtab {
        handle_get_window(cs.cs_curwin)
    } else {
        win_find_by_handle(cs.cs_curwin)
    };
    if save_curwin.is_none() {
        save_curwin = fallback;
    }
    if let Some(save_curwin) = save_curwin {
        set_curwin(save_curwin);
        g_st().curbuf.set(curwin_mut().w_buffer);
    }
    g_st()
        .prevwin
        .set((win_find_by_handle(cs.cs_prevwin).map_or(ptr::null_mut(), ptr::from_mut)).into());
}

pub fn ctx_restore(cs: &mut CtxSwitch) {
    if cs.cs_mode == K_CTX_SWITCH_NONE {
        return;
    }

    if cs.cs_mode == K_CTX_SWITCH_WIN {
        if !cs.cs_curtab.is_null() && valid_tabpage(cs.cs_curtab.opt()) {
            if (cs.cs_flags & K_CTX_NO_DISPLAY) != 0 {
                let old_tp_curwin = curtab_mut().tp_curwin.get();
                unuse_tabpage(&curtab_mut());
                curtab_mut().tp_curwin.set(old_tp_curwin);
                use_tabpage(cs.cs_curtab.borrow());
            } else {
                goto_tabpage_tp(cs.cs_curtab.opt(), false, false);
            }
        }
        ctx_restore_curwin(cs, None);
    } else if cs.cs_ctxwin_idx >= 0 {
        let cwp = ctx_win_rest(cs);
        ctx_restore_curwin(cs, g_st().firstwin.get().opt_mut());
        entering_window(curwin_mut());
        if bt_prompt(Some(crate::globals::curbuf_mut())) {
            curbuf_mut().b_prompt_insert.set(cs.cs_prompt_insert);
        }

        vars_clear(&cwp.w_vars().dv_hashtab);

        let free_dirs = (cs.cs_flags & K_CTX_KEEP_DIRS) == 0;

        ctx_localdirs_restore(
            &mut *cs,
            Some(cwp),
            Some(&crate::globals::curtab_mut()),
            free_dirs,
        );

        if curwin_mut().w_topline.get() > curbuf_mut().b_ml.ml_line_count.get() {
            curwin_mut()
                .w_topline
                .set(curbuf_mut().b_ml.ml_line_count.get());
            curwin_mut().w_topfill.set(0);
        }
    } else if cs.cs_mode == K_CTX_SWITCH_BUF {
        let saved_buf = cs.cs_new_curbuf.br_buf;
        if curwin_mut().handle == cs.cs_new_curwin
            && g_st().curbuf.get() != saved_buf
            && bufref_valid(Some(&cs.cs_new_curbuf))
            && saved_buf.b_ml.ml_mfp.is_some()
        {
            curbuf_mut()
                .b_nwindows
                .set(curbuf_mut().b_nwindows.get() - (1));
            g_st().curbuf.set(saved_buf);
            curwin_mut().w_buffer = g_st().curbuf.get();
            curbuf_mut()
                .b_nwindows
                .set(curbuf_mut().b_nwindows.get() + (1));
        }

        ctx_restore_curwin(cs, None);
    }

    if !cs.cs_same_win {
        g_st().visual_global.active = cs.cs_visual_active;
    }
    if cs.cs_mode == K_CTX_SWITCH_BUF {
        check_cursor(curwin_mut());
        if g_st().visual_global.active {
            check_pos(curbuf_mut(), &mut g_st().visual_global.start);
        }
    }

    if (cs.cs_flags & K_CTX_NO_EVENTS) != 0 {
        unblock_autocmds();
    }
    ctx_dirs_restore(cs);
    if cs.cs_ctxwin_idx < 0 && CTX_STATE.with(|st| st.ctx_did_chdir.get()) {
        update_cwd(K_CD_CAUSE_WINDOW);
    }
    CTX_STATE.with(|st| {
        st.ctx_did_chdir
            .set(st.ctx_did_chdir.get() || cs.cs_did_chdir)
    });
    if (cs.cs_flags & K_CTX_VALIDATE) != 0 {
        if let Some(wp) = win_find_by_handle(cs.cs_target_win)
            && !equalpos(cs.cs_target_old_pos, wp.w_cursor)
        {
            wp.w_redr_status.set(true);
        }
        check_cursor(curwin_mut());
        if g_st().visual_global.active {
            check_pos(curbuf_mut(), &mut g_st().visual_global.start);
        }
    }
    if cs.cs_mode == K_CTX_SWITCH_BUF && cs.cs_new_curwin != cs.cs_curwin {
        CTX_STATE.with(|st| {
            let depth = st.ctx_switch_depth.get();
            assert!(depth > 0);
            st.ctx_switch_depth.set(depth - 1);
            if depth - 1 == 0 {
                st.ctx_saved_curwin.set(0);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equalpos_compares_all_three_fields() {
        let a = CursorPos {
            lnum: 10,
            col: 5,
            coladd: 0,
        };
        let b = CursorPos {
            lnum: 10,
            col: 5,
            coladd: 0,
        };
        let c = CursorPos {
            lnum: 10,
            col: 5,
            coladd: 1,
        };
        assert!(equalpos(a, b));
        assert!(!equalpos(a, c));
    }
}
