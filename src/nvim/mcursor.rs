//! Multicursor tracking, preview, and cascade replay (`src/nvim/mcursor.c`).

use crate::api::buffer::{LUA_INTERNAL_CALL, nvim_buf_set_text};
use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    api_clear_error, boolean_obj, cstr_as_string, static_cstr_as_string, string_to_array,
};
use crate::api::private::validate::{Error, ErrorType, NvimString};
use crate::api::vim::nvim_feedkeys;
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{MAXLNUM, buf_T, buf_get_changedtick, bufref_valid};
use crate::charset::to_special;
use crate::clipboard::set_clipboard;
use crate::context::{
    CONTEXT_INIT, Context, INIT_POS, ctx_free, ctx_load, ctx_save, handle_get_buffer, kCtxCursor,
    kCtxMarks, kCtxMergeReg, kCtxRegs, kCtxVisual,
};
use crate::cursor::{
    check_cursor, check_cursor_col, check_pos, colnr_T, equalpos, getvpos, linenr_T, lt, ltoreq,
    pos_T,
};
use crate::decoration::DecorInline;
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::eval::ESC;
use crate::eval::typval::{TypvalVval, VarType, typval_T, varnumber_T};
use crate::eval::vars::{VV_CASCADING, get_vim_var_tv, set_vim_var_bool};
use crate::eval::window::viewstate_T;
use crate::ex_docmd::{
    CMOD_KEEPJUMPS, exec_normal_cmd, restore_current_state, save_current_state, set_no_hlsearch,
};
use crate::extmark::{
    TriState, extmark_clear, extmark_del_id, extmark_from_id, extmark_get_pos, extmark_set,
    extmark_set_pos, kFalse, kNone, kTrue,
};
use crate::globals::*;
use crate::highlight_group::syn_check_group;
use crate::input::{
    NUL, RedoState, beep_flush, block_on_key, redo_keys, restore_redobuff, save_redobuff,
    unblock_on_key,
};
use crate::input_cmdatom::{
    CmdAtom, CmdOrigin, atom_cmd_spec, atom_composite_active, atom_composite_queued,
    atom_did_global_op, atom_key_class, atom_lhs_replay_queue, atom_push_raw, atom_suppress,
    atom_visual_reset, atom_visual_span, cmdarg_T, atom_st, kAInsertSpan, kAMotion, kAVisualSpan,
    kKeyInsFlush,
};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_COMMAND, K_SPECIAL, NL};
use crate::log::{LOGLVL_DBG, log_st, logmsg};
use crate::lua::executor::nlua_call_typval;
use crate::mapping::REMAP_NONE;
use crate::mark::mark_get_visual;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorInlineData, MT_FLAG_DECOR_HL, MTPair, MarkTreeIter,
    marktree_itr_current, marktree_itr_get, marktree_itr_next, mt_end,
};
use crate::memline::{dec, inc, ml_get_len, ml_region_text};
use crate::memory::{Arena, arena_finish, arena_mem_free};
use crate::menu::save_state_T;
use crate::r#move::{
    restore_viewstate, save_viewstate, set_topline, update_curswant, update_topline,
};
use crate::msgpack_rpc::packer::handle_T;
use crate::normal::{VisualState, end_visual_mode, visualinfo, visualinfo_T};
use crate::ops::{getregionpos_line, getregionpos_prep};
use crate::option::{OPT_LOCAL, set_option_value_give_err};
use crate::option_vars::*;
use crate::os::input::line_breakcheck;
use crate::os::time::{Timestamp, os_realtime};
use crate::plines::getvcols;
use crate::popupmenu::pum_visible;
use crate::register::{
    get_reg_contents_str, get_unname_register, get_y_previous, get_y_register, op_reg_index,
    reg_max_ts, write_reg_contents_ex,
};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::shada::shada_encode_regs;
use crate::state::{MODE_INSERT, virtual_active};
use crate::strings::{vim_snprintf};
use crate::textformat::{InsState, oparg_T};
use crate::undo::{u_forget_unchanged, u_save, uhp_mut};
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

const DECOR_INLINE_INIT: DecorInline = DecorInline {
    data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
};

pub(crate) const kOptFollow: c_int = 114;

const MC_REGS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789\"-\0";

use crate::digraph::{kBoolVarFalse, kBoolVarTrue};
use crate::eval::funcs::{kMTBlockWise, kMTCharWise, kMTLineWise, kMTUnknown};
use crate::ex_getln::K_LUA;
use crate::math::FAIL;
use crate::regexp::{Ctrl_V, MAXCOL};

#[repr(C)]
struct McSandbox {
    primary: Context,
    cursor_mark: u32,
    topline_mark: u32,
    view: viewstate_T,
    sst: save_state_T,
    redo: RedoState,
    visual: VisualState,
}

impl McSandbox {
    fn new() -> Self {
        Self {
            primary: CONTEXT_INIT,
            cursor_mark: 0,
            topline_mark: 0,
            view: viewstate_T::new(),
            sst: save_state_T::new(),
            redo: RedoState::new(),
            visual: VisualState::new(),
        }
    }
}

#[repr(C)]
struct McInsSaved {
    ins: InsState,
    last_changedtick: varnumber_T,
    last_changedtick_i: varnumber_T,
}

struct McInsSpan {
    origin: CmdOrigin,
    regions: Vec<u32>,
    active: bool,
    first: bool,
    frame: u64,
    done_len: usize,
    region: u32,
    vsel: Option<std::ffi::CString>,
}

struct McStart {
    time: Timestamp,
    ctx: Context,
}

struct McursorState {
    mc_ins_span: McInsSpan,
    mc_start: McStart,
    mc_vsel_buf: handle_T,
    mc_ins_joined: bool,
    mc_cursors: Vec<Context>,
    ns_mc: u32,
    ns_vcur: u32,
    ns_vsel: u32,
    ns_last: u32,
    ns_session: u32,
}

impl McursorState {
    const fn new() -> Self {
        Self {
            mc_ins_span: McInsSpan { origin: CmdOrigin { buf: crate::buffer::bufref_T { br_buf: ptr::null_mut(), br_fnum: 0, br_buf_free_count: 0 }, win: ptr::null(), pos: INIT_POS, mcursor: 0, tick: 0, maptick: 0 }, regions: Vec::new(), active: false, first: false, frame: 0, done_len: 0, region: 0, vsel: None },
            mc_start: McStart { time: 0, ctx: CONTEXT_INIT },
            mc_vsel_buf: 0,
            mc_ins_joined: false,
            mc_cursors: Vec::new(),
            ns_mc: 0,
            ns_vcur: 0,
            ns_vsel: 0,
            ns_last: 0,
            ns_session: 0,
        }
    }
}

static MC_STATE: GlobalState<McursorState> = GlobalState::new(McursorState::new());

#[inline]
fn mc_st() -> &'static mut McursorState {
    unsafe { &mut *MC_STATE.as_ptr() }
}

#[inline]
fn cursors() -> &'static [Context] {
    &mc_st().mc_cursors
}

#[inline]
fn cursors_mut() -> &'static mut Vec<Context> {
    &mut mc_st().mc_cursors
}

#[inline]
fn ins_regions() -> &'static [u32] {
    unsafe {
        let ptr = &raw const mc_st().mc_ins_span.regions;
        &*ptr
    }
}

#[inline]
fn ins_regions_mut() -> &'static mut Vec<u32> {
    unsafe {
        let ptr = &raw mut mc_st().mc_ins_span.regions;
        &mut *ptr
    }
}



pub fn mc_init() {
    mc_ns();
}

fn mc_ns() -> u32 {
    {
        if mc_st().ns_mc == 0 {
            mc_st().ns_mc = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor")) as u32;
        }
        mc_st().ns_mc
    }
}

fn mc_vcur_ns() -> u32 {
    {
        if mc_st().ns_vcur == 0 {
            mc_st().ns_vcur =
                nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.cursor")) as u32;
        }
        mc_st().ns_vcur
    }
}

fn mc_vsel_ns() -> u32 {
    {
        if mc_st().ns_vsel == 0 {
            mc_st().ns_vsel =
                nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.visual")) as u32;
        }
        mc_st().ns_vsel
    }
}

fn mc_last_ns() -> u32 {
    {
        if mc_st().ns_last == 0 {
            mc_st().ns_last = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.last")) as u32;
        }
        mc_st().ns_last
    }
}

fn mc_session_ns() -> u32 {
    {
        if mc_st().ns_session == 0 {
            mc_st().ns_session =
                nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor._session")) as u32;
        }
        mc_st().ns_session
    }
}

pub fn mc_showcmd(buf: *mut c_char, size: usize) -> usize {
    unsafe {
        let count = cursors().len();
        if count == 0 {
            *buf = 0;
            return 0;
        }
        let prefix = if mc_following() {
            c"=".as_ptr()
        } else {
            c"".as_ptr()
        };
        vim_snprintf!(buf, size, c"%s%zu\xc3\x97 ".as_ptr(), prefix, count) as usize
    }
}

pub fn mc_replaying() -> bool {
    unsafe { (*get_vim_var_tv(VV_CASCADING)).vval.v_bool() == kBoolVarTrue }
}

fn mc_mark_upd(buf: *mut buf_T, mark: *mut u32, pos: pos_T) {
    extmark_set_pos(buf, mc_ns(), mark, pos, true, false, true);
}

fn mc_track_upd(buf: *mut buf_T, mark: *mut u32, pos: pos_T) {
    extmark_set_pos(buf, mc_session_ns(), mark, pos, true, true, false);
}

fn mc_sandbox_enter(sb: &mut McSandbox, save_regs: bool) {
    unsafe {
        assert!(!mc_replaying());
        set_vim_var_bool(VV_CASCADING, kBoolVarTrue);
        sb.primary = CONTEXT_INIT;
        ctx_save(
            &mut sb.primary,
            kCtxCursor | kCtxVisual | kCtxMarks | if save_regs { kCtxRegs } else { 0 },
        );
        sb.cursor_mark = 0;
        mc_track_upd(g_st().curbuf, &mut sb.cursor_mark, sb.primary.pos);
        save_viewstate(&mut **curwin.as_ptr(), &mut sb.view);
        sb.topline_mark = 0;
        mc_track_upd(
            g_st().curbuf,
            &mut sb.topline_mark,
            pos_T {
                lnum: sb.view.vs_topline,
                col: 0,
                coladd: 0,
            },
        );
        sb.visual = g_st().Visual;
        save_current_state(&mut sb.sst);
        save_search_patterns();
        save_redobuff(&mut sb.redo);
    }
}

fn mc_sandbox_leave(sb: &mut McSandbox) {
    unsafe {
        if !sb.primary.regs.is_empty() {
            ctx_load(&mut sb.primary, kCtxRegs, 0);
        }
        restore_redobuff(&mut sb.redo);
        restore_search_patterns();
        g_st().Visual = sb.visual;
        restore_current_state(&mut sb.sst);
        let buf = handle_get_buffer(sb.primary.buf);
        let mut topline = INIT_POS;
        let mut topline_valid = false;
        if !buf.is_null() {
            extmark_get_pos(
                buf,
                mc_session_ns(),
                sb.cursor_mark,
                &mut sb.primary.pos,
            );
            extmark_del_id(&mut *buf, mc_session_ns(), sb.cursor_mark);
            topline_valid = extmark_get_pos(buf, mc_session_ns(), sb.topline_mark, &mut topline);
            extmark_del_id(&mut *buf, mc_session_ns(), sb.topline_mark);
        }
        if buf == g_st().curbuf {
            restore_viewstate(&mut **curwin.as_ptr(), &sb.view);
            ctx_load(&mut sb.primary, kCtxCursor | kCtxVisual | kCtxMarks, 0);
            check_cursor(*curwin.as_ptr());
            if topline_valid {
                set_topline(&mut **curwin.as_ptr(), topline.lnum.min((*g_st().curbuf).b_ml.ml_line_count));
                update_topline(*curwin.as_ptr());
            }
        }
        ctx_free(&mut sb.primary);
        set_vim_var_bool(VV_CASCADING, kBoolVarFalse);
    }
}

fn mc_execute(cursoridx: usize, atomidx: usize) {
    unsafe {
        let mut ctx = cursors()[cursoridx].clone();
        let (atom_keys, atom_type, atom_cmd, atom_cmd2, atom_remap) = {
            let ga = &raw const atom_st().g_atoms;
            let a = &(*ga).as_slice()[atomidx];
            (
                a.keys.as_ref().map_or(ptr::null(), |k| k.as_ptr()),
                a.r#type,
                a.spec.cmd,
                a.spec.cmd2,
                a.remap,
            )
        };

        if handle_get_buffer(ctx.buf) != g_st().curbuf {
            return;
        }

        if ctx.mark != 0 && !extmark_get_pos(g_st().curbuf, mc_ns(), ctx.mark, &mut ctx.pos) {
            return;
        }
        if ctx.visual.vi_start.lnum == 0 && (crate::memory::cstr_prefix_bytes(atom_keys, 2) == &b"gv"[..2]) {
            return;
        }

        let swap_regs = atom_type != kAMotion;
        let mut regs_ts: Timestamp = 0;
        if swap_regs {
            if reg_max_ts(false) >= mc_st().mc_start.time {
                ctx_load(&mut mc_st().mc_start.ctx, kCtxRegs, 0);
            }
            if !ctx.regs.is_empty() {
                ctx_load(&mut ctx, kCtxRegs, kCtxMergeReg);
            }
            regs_ts = reg_max_ts(false);
        }

        let mark_cmd = if atom_cmd == b'g' as c_int {
            atom_cmd2
        } else {
            atom_cmd
        };
        let swap_marks = atom_type != kAMotion
            || atom_remap
            || mark_cmd == b'\'' as c_int
            || mark_cmd == b'`' as c_int;

        ctx_load(
            &mut ctx,
            if swap_marks {
                kCtxVisual | kCtxMarks | kCtxCursor
            } else {
                kCtxCursor
            },
            0,
        );

        if atom_type == kAInsertSpan {
            check_pos(&mut *g_st().curbuf, &mut (**curwin.as_ptr()).w_cursor);
        } else {
            check_cursor(*curwin.as_ptr());
        }

        if atom_type == kAVisualSpan {
            g_st().Visual.active = false;
            g_st().Visual.select = false;
        }

        if atom_type == kAInsertSpan {
            g_st().State = MODE_INSERT;
        }

        let beeps = g_st().did_beep;
        let tick = buf_get_changedtick(&*g_st().curbuf);
        let mode_str = if atom_remap {
            c"ix".as_ptr()
        } else {
            c"nix".as_ptr()
        };
        nvim_feedkeys(cstr_as_string(atom_keys), cstr_as_string(mode_str), false);

        if atom_type == kAInsertSpan
            && mc_st().mc_ins_span.first
            && g_st().did_beep != beeps
            && buf_get_changedtick(&*g_st().curbuf) == tick
        {
            extmark_del_id(&mut *g_st().curbuf, mc_ns(), ctx.mark);
            return;
        }

        if g_st().Visual.active {
            if atom_type == kAVisualSpan {
                (*g_st().curbuf).b_visual = visualinfo();
                (*g_st().curbuf).b_visual_mode_eval = g_st().Visual.mode;
            }
            g_st().Visual.active = false;
            g_st().Visual.select = false;
        }

        let wrote_regs = swap_regs && reg_max_ts(false) != regs_ts;

        if cursoridx >= cursors().len() {
            return;
        }
        if !extmark_get_pos(g_st().curbuf, mc_ns(), ctx.mark, &mut ctx.pos) {
            return;
        }

        if wrote_regs {
            ctx.regs = shada_encode_regs(false, mc_st().mc_start.time);
        }
        ctx_save(
            &mut ctx,
            if swap_marks {
                kCtxVisual | kCtxMarks | kCtxCursor
            } else {
                kCtxCursor
            },
        );

        if atom_type == kAInsertSpan && (*g_st().curbuf).b_last_insert.mark.lnum > 0 {
            ctx.pos = (*g_st().curbuf).b_last_insert.mark;
        }
        mc_mark_upd(g_st().curbuf, &mut ctx.mark, ctx.pos);
        cursors_mut()[cursoridx] = ctx;
    }
}

fn mc_cascade() {
    unsafe {
        let ga = &raw mut atom_st().g_atoms;
        assert!(!(*ga).is_empty());
        let origin = (*ga).as_slice()[0].origin;
        let mcursor = if origin.buf.br_buf == g_st().curbuf {
            origin.mcursor
        } else {
            0
        };
        assert!(!cursors().is_empty());
        assert!(!mc_replaying());

        g_st().got_int = false;

        let mut edits = false;
        for i in 0..(*ga).len() {
            edits |= (*ga).as_slice()[i].r#type != kAMotion;
        }
        if edits {
            mc_cleanup(true, ptr::null(), 0);
            if cursors().is_empty() {
                (*ga).clear();
                return;
            }
        }

        let mut mcursor_pos = INIT_POS;
        if mcursor != 0 && extmark_get_pos(g_st().curbuf, mc_ns(), mcursor, &mut mcursor_pos) {
            let mut mark = mcursor;
            mc_mark_upd(g_st().curbuf, &mut mark, (**curwin.as_ptr()).w_cursor);
        }

        let mut sb = McSandbox::new();
        mc_sandbox_enter(&mut sb, edits);

        'outer: for ai in 0..(*ga).len() {
            let atom = &mut (&mut *ga)[ai];
            if !atom.origin.buf.br_buf.is_null()
                && (!bufref_valid(&raw mut atom.origin.buf) || atom.origin.buf.br_buf != g_st().curbuf)
            {
                continue;
            }
            let mut ci = 0;
            while ci < cursors().len() {
                line_breakcheck();
                if g_st().got_int {
                    break 'outer;
                }
                if mcursor == 0 || cursors()[ci].mark != mcursor {
                    mc_execute(ci, ai);
                }
                ci += 1;
            }
        }

        (*ga).clear();
        let bufnr = sb.primary.buf;
        mc_sandbox_leave(&mut sb);

        mc_cleanup(true, &(**curwin.as_ptr()).w_cursor, mcursor);
        if handle_get_buffer(bufnr) == g_st().curbuf
            && !(*g_st().curbuf).b_u_synced
            && let Some(nh) = (*g_st().curbuf).b_u_newhead
        {
            uhp_mut(&mut *g_st().curbuf, nh).uh_cursor_after = (**curwin.as_ptr()).w_cursor;
        }
    }
}

pub fn mc_clock_edge(map_edit: bool, map_moved: bool, follow: bool) {
    unsafe {
        let ga = &raw mut atom_st().g_atoms;
        if (map_edit || (follow && map_moved && !g_st().Visual.active))
            && !atom_composite_queued()
            && (*ga).is_empty()
            && atom_composite_active()
            && mc_buf_has_cursors(&mut *g_st().curbuf)
        {
            atom_lhs_replay_queue();
        }
        if mc_buf_has_cursors(&mut *g_st().curbuf) && !(*ga).is_empty() {
            let mut has_edit = map_edit;
            let mut i = 0;
            while !has_edit && i < (*ga).len() {
                has_edit = (*ga).as_slice()[i].r#type != kAMotion;
                i += 1;
            }
            if has_edit || follow || g_st().Visual.active {
                mc_cascade();
            } else {
                (*ga).clear();
            }
        }
    }
}

fn mc_cleanup(dedupe: bool, primary: *const pos_T, keep_mark: u32) {
    unsafe {
        let had_cursors = !cursors().is_empty();
        let mut n = 0;
        for i in 0..cursors().len() {
            let mut ctx = cursors()[i].clone();
            let buf = handle_get_buffer(ctx.buf);
            if buf.is_null()
                || (ctx.mark != 0 && !extmark_get_pos(buf, mc_ns(), ctx.mark, &mut ctx.pos))
            {
                ctx_free(&mut ctx);
                continue;
            }
            let dup = dedupe
                && ctx.mark != 0
                && ((!(*curwin.as_ptr()).is_null()
                    && buf == g_st().curbuf
                    && !primary.is_null()
                    && equalpos(ctx.pos, *primary)
                    && ctx.mark != keep_mark)
                    || mc_mark_at(buf, ctx.pos, 0) != ctx.mark);
            if dup {
                extmark_del_id(&mut *buf, mc_ns(), ctx.mark);
                ctx_free(&mut ctx);
            } else {
                cursors_mut()[n] = ctx;
                n += 1;
            }
        }
        cursors_mut().truncate(n);
        if n == 0 && had_cursors {
            ctx_free(&mut mc_st().mc_start.ctx);
            mc_st().mc_start.time = 0;
            mc_lua_enable(false);
        }
    }
}

pub fn mc_ins_join() {
    if !mc_replaying() {
            mc_st().mc_ins_joined = true;
        }
}

pub fn mc_ins_replay_can_join() -> bool {
    !mc_replaying() || mc_st().mc_ins_joined
}

pub fn mc_ins_cascade_start(
    cascade: bool,
    origin: CmdOrigin,
    root_frame: u64,
    vsel: Option<std::ffi::CString>,
) {
    unsafe {
        if mc_replaying() {
            return;
        }
        mc_st().mc_ins_span.vsel = vsel;
        mc_st().mc_ins_joined = false;
        mc_st().mc_ins_span.active = cascade && mc_buf_has_cursors(&mut *g_st().curbuf);
        mc_st().mc_ins_span.first = true;
        mc_st().mc_ins_span.frame = root_frame;
        mc_st().mc_ins_span.done_len = 0;
        mc_st().mc_ins_span.origin = origin;
        mc_st().mc_ins_span.region = 0;
        mc_ins_regions_clear();

        if mc_st().mc_ins_span.active {
            let mut lo = (**curwin.as_ptr()).w_cursor.lnum;
            let mut hi = (**curwin.as_ptr()).w_cursor.lnum;
            for ctx in cursors() {
                let mut pos = INIT_POS;
                if mc_ctx_resolve(ctx, &mut pos) {
                    lo = lo.min(pos.lnum);
                    hi = hi.max(pos.lnum);
                }
            }
            let _ = u_save(lo - 1, hi + 1);
        }
    }
}

fn mc_ins_save_state() -> McInsSaved {
    unsafe {
        let saved = McInsSaved {
            ins: g_st().Ins,
            last_changedtick: (*g_st().curbuf).b_last_changedtick,
            last_changedtick_i: (*g_st().curbuf).b_last_changedtick_i,
        };
        g_st().Ins.did_ai = false;
        g_st().Ins.ai_col = 0;
        g_st().Ins.end_comment_pending = NUL;
        g_st().Ins.did_si = false;
        g_st().Ins.can_si = false;
        g_st().Ins.can_si_back = false;
        saved
    }
}

fn mc_ins_restore_state(saved: &McInsSaved) {
    unsafe {
        g_st().Ins = saved.ins;
        (*g_st().curbuf).b_last_changedtick = saved.last_changedtick;
        (*g_st().curbuf).b_last_changedtick_i = saved.last_changedtick_i;
    }
}

fn mc_ins_span_push(keys: std::ffi::CString, text: Option<std::ffi::CString>) {
    unsafe {
        let cascade = mc_buf_has_cursors(&mut *g_st().curbuf);
        let mut atom = CmdAtom::new();
        atom.r#type = kAInsertSpan;
        atom.keys = Some(keys);
        atom.text = text;
        atom.origin = mc_st().mc_ins_span.origin;
        atom_push_raw(cascade, &mut atom);
        if cascade {
            let saved = mc_ins_save_state();
            block_autocmds();
            mc_cascade();
            unblock_autocmds();
            mc_ins_restore_state(&saved);
            mc_st().mc_ins_joined = false;
        }
        mc_st().mc_ins_span.first = false;
    }
}

fn mc_ins_regions_clear() {
    unsafe {
        while let Some(id) = ins_regions_mut().pop() {
            extmark_del_id(&mut *g_st().curbuf, mc_session_ns(), id);
        }
    }
}

#[inline]
fn mc_ctx_resolve(ctx: &Context, pos: &mut pos_T) -> bool {
    handle_get_buffer(ctx.buf) == g_st().curbuf
            && ctx.mark != 0
            && extmark_get_pos(g_st().curbuf, mc_ns(), ctx.mark, pos)
}

fn mc_region_mark_set(mark: *mut u32, pos: pos_T) {
    unsafe {
        extmark_set(
            &mut *g_st().curbuf,
            mc_session_ns(),
            mark,
            pos.lnum - 1,
            pos.col,
            pos.lnum - 1,
            pos.col,
            DECOR_INLINE_INIT,
            0,
            false,
            true,
            true,
            false,
        );
    }
}

fn mc_ins_preview_rebase() {
    unsafe {
        mc_region_mark_set(&raw mut mc_st().mc_ins_span.region, (**curwin.as_ptr()).w_cursor);
        mc_ins_regions_clear();
        for i in 0..cursors().len() {
            let ctx = &cursors()[i];
            let mut pos = INIT_POS;
            let mut mark = 0;
            if mc_ctx_resolve(ctx, &mut pos) && ctx.mark != mc_st().mc_ins_span.origin.mcursor {
                mc_region_mark_set(&mut mark, pos);
                ins_regions_mut().push(mark);
            }
        }
    }
}

fn mc_ins_region_first() -> MTPair {
    unsafe {
        for &id in ins_regions() {
            let p = extmark_from_id(&mut *g_st().curbuf, mc_session_ns(), id);
            if p.start.id != 0 {
                return p;
            }
        }
        MTPair::new()
    }
}

#[inline]
fn mc_region_range(p: MTPair, start: *mut pos_T, end: *mut pos_T) {
    unsafe {
        *start = pos_T {
            lnum: p.start.pos.row + 1,
            col: p.start.pos.col,
            coladd: 0,
        };
        *end = pos_T {
            lnum: p.end_pos.row + 1,
            col: p.end_pos.col,
            coladd: 0,
        };
    }
}

fn mc_region_mark_range(mark: u32, start: &mut pos_T, end: &mut pos_T) -> bool {
    unsafe {
        let p = extmark_from_id(&mut *g_st().curbuf, mc_session_ns(), mark);
        if p.start.id == 0 {
            return false;
        }
        mc_region_range(p, &raw mut *start, &raw mut *end);
        true
    }
}

fn mc_ins_preview_replace(start: pos_T, end: pos_T, text: *const NvimString) {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut err = Error::new();
        let input = if !(*text).data.is_null() {
            *text
        } else {
            NvimString {
                data: ptr::null_mut(),
                size: 0,
            }
        };
        let lines = string_to_array(input, false, &mut arena);
        nvim_buf_set_text(
            LUA_INTERNAL_CALL,
            0,
            i64::from(start.lnum - 1),
            i64::from(start.col),
            i64::from(end.lnum - 1),
            i64::from(end.col),
            lines,
            &mut arena,
            &mut err,
        );
        if err.err_type != ErrorType::kErrorTypeNone {
            if log_st().g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"mc_ins_preview_replace".as_ptr(),
                    747,
                    true,
                    c"preview replace failed: %s".as_ptr(),
                    err.msg_ptr(),
                );
            }
            api_clear_error(&mut err);
        }
        arena_mem_free(arena_finish(&mut arena));
    }
}

fn mc_ins_preview_set(new_text: *const NvimString) {
    unsafe {
        let saved = mc_ins_save_state();
        let mut primary: u32 = 0;
        mc_track_upd(g_st().curbuf, &mut primary, (**curwin.as_ptr()).w_cursor);
        let save_cmod_flags = g_st().cmdmod.cmod_flags;
        g_st().cmdmod.cmod_flags |= CMOD_KEEPJUMPS;
        set_vim_var_bool(VV_CASCADING, kBoolVarTrue);
        for i in 0..ins_regions().len() {
            let mut rs = INIT_POS;
            let mut re = INIT_POS;
            if !mc_region_mark_range(ins_regions()[i], &mut rs, &mut re) {
                continue;
            }
            mc_ins_preview_replace(rs, re, new_text);
        }
        set_vim_var_bool(VV_CASCADING, kBoolVarFalse);
        g_st().cmdmod.cmod_flags = save_cmod_flags;
        let mut pos = (**curwin.as_ptr()).w_cursor;
        if extmark_get_pos(g_st().curbuf, mc_session_ns(), primary, &mut pos) {
            (**curwin.as_ptr()).w_cursor = pos;
        }
        extmark_del_id(&mut *g_st().curbuf, mc_session_ns(), primary);
        mc_ins_restore_state(&saved);
    }
}

fn mc_ins_keys_nonliteral(keys: *const c_char, len: usize) -> bool {
    unsafe {
        let mut i = 0;
        while i < len {
            let mut key = c_int::from(*keys.add(i) as u8);
            if key == c_int::from(K_SPECIAL) && i + 2 < len {
                key = to_special(
                    c_int::from(*keys.add(i + 1) as u8),
                    c_int::from(*keys.add(i + 2) as u8),
                );
                i += 2;
            }
            if (atom_key_class(key, NUL, NUL) & kKeyInsFlush) != 0 {
                return true;
            }
            i += 1;
        }
        false
    }
}

pub fn mc_ins_cascade_restart() {
    unsafe {
        let ga = &raw const atom_st().g_atoms;
        if !mc_st().mc_ins_span.active
            || mc_replaying()
            || (g_st().State & MODE_INSERT) == 0
            || !mc_buf_has_cursors(&mut *g_st().curbuf)
            || !(*ga).is_empty()
        {
            return;
        }
        let ins = redo_keys(ptr::null());
        mc_st().mc_ins_span.done_len = ins.as_ref().map_or(0, |s| s.as_bytes().len());
        mc_ins_preview_rebase();
    }
}

pub fn mc_ins_replaying() -> bool {
    mc_replaying() && mc_ins_cascading()
}

pub fn mc_ins_cascading() -> bool {
    mc_st().mc_ins_span.active
}

pub fn mc_ins_cascaded(root_frame: u64) -> bool {
    !mc_st().mc_ins_span.first && mc_st().mc_ins_span.frame == root_frame
}

pub fn mc_ins_cascade() {
    unsafe {
        if !mc_st().mc_ins_span.active
            || mc_replaying()
            || (g_st().State & MODE_INSERT) == 0
            || !mc_buf_has_cursors(&mut *g_st().curbuf)
        {
            return;
        }

        let mut start = INIT_POS;
        let mut end = INIT_POS;
        let ins_cstr = redo_keys(ptr::null());
        let ins = NvimString {
            data: ins_cstr
                .as_ref()
                .map_or(ptr::null_mut(), |s| s.as_ptr().cast_mut()),
            size: ins_cstr.as_ref().map_or(0, |s| s.as_bytes().len()),
        };

        if mc_st().mc_ins_span.first {
            if !g_st().Ins.did_ai && !ins.data.is_null() && ins.size > 0 {
                let mut keys: Vec<u8> = Vec::new();
                let mut skip = 0;
                if let Some(vsel) = mc_st().mc_ins_span.vsel.take() {
                    assert!(ins.size >= 2 && (crate::memory::cstr_prefix_bytes(ins.data, 2) == &b"1v"[..2]));
                    keys.extend_from_slice(vsel.to_bytes());
                    skip = 2;
                }
                keys.extend_from_slice(std::slice::from_raw_parts(
                    ins.data.add(skip).cast::<u8>(),
                    ins.size - skip,
                ));
                keys.push(ESC as u8);
                mc_st().mc_ins_span.done_len = ins.size;
                mc_ins_span_push(std::ffi::CString::new(keys).unwrap(), None);
                mc_ins_preview_rebase();
            }
        } else if !ins.data.is_null() && ins.size < mc_st().mc_ins_span.done_len {
            mc_ins_cascade_restart();
        } else if ins.size > mc_st().mc_ins_span.done_len
            && mc_ins_keys_nonliteral(
                ins.data.add(mc_st().mc_ins_span.done_len),
                ins.size - mc_st().mc_ins_span.done_len,
            )
        {
            if !ins_compl_active() && !pum_visible() {
                mc_ins_span_flush(&ins, false);
            }
        } else if mc_region_mark_range(mc_st().mc_ins_span.region, &mut start, &mut end) {
            let text = ml_region_text(g_st().curbuf, start, end);
            let mut update = true;
            let fp = mc_ins_region_first();
            if fp.start.id != 0 {
                let mut frs = INIT_POS;
                let mut fre = INIT_POS;
                mc_region_range(fp, &mut frs, &mut fre);
                let cur = ml_region_text(g_st().curbuf, frs, fre);
                update = cur != text;
            }
            if update && (ins_compl_active() || pum_visible()) {
                for &id in ins_regions() {
                    let mut rs = INIT_POS;
                    let mut re = INIT_POS;
                    if !mc_region_mark_range(id, &mut rs, &mut re) {
                        continue;
                    }
                    if !lt(end, rs)
                        && (re.lnum >= start.lnum || rs.lnum != re.lnum || start.lnum != end.lnum)
                    {
                        update = false;
                        break;
                    }
                }
            }
            if update {
                let text_str = NvimString {
                    data: text.as_ptr().cast::<c_char>().cast_mut(),
                    size: text.len(),
                };
                mc_ins_preview_set(&text_str);
            }
        }
        drop(ins_cstr);
    }
}

fn mc_ins_span_flush(ins: *const NvimString, commit: bool) {
    unsafe {
        let empty = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        let fp = mc_ins_region_first();
        if fp.start.id != 0
            && (fp.start.pos.row != fp.end_pos.row || fp.start.pos.col != fp.end_pos.col)
        {
            mc_ins_preview_set(&empty);
        }
        let dlen = (*ins).size - mc_st().mc_ins_span.done_len;
        let mut keys: Vec<c_char> = Vec::with_capacity(dlen + 2);
        keys.push(b'i' as c_char);
        if dlen > 0 {
            keys.extend_from_slice(std::slice::from_raw_parts(
                (*ins).data.add(mc_st().mc_ins_span.done_len),
                dlen,
            ));
        }
        if !commit {
            keys.push(ESC);
        }
        let text = if commit && dlen > 1 {
            let text_slice = std::slice::from_raw_parts(
                (*ins).data.add(mc_st().mc_ins_span.done_len).cast::<u8>(),
                dlen - 1,
            );
            Some(std::ffi::CString::new(text_slice).unwrap())
        } else {
            None
        };
        mc_st().mc_ins_span.done_len = (*ins).size;
        let keys_slice = std::slice::from_raw_parts(keys.as_ptr().cast::<u8>(), keys.len());
        mc_ins_span_push(std::ffi::CString::new(keys_slice).unwrap(), text);
        if !commit {
            mc_ins_preview_rebase();
        }
    }
}

fn mc_reg_read(reg: c_int) -> std::ffi::CString {
    let Some(s) = get_reg_contents_str(reg, 0) else {
            return std::ffi::CString::default();
        };
        let mut bytes = s.into_bytes();
        if bytes.last() == Some(&(NL as u8)) {
            bytes.pop();
        }
        std::ffi::CString::new(bytes).unwrap_or_default()
}

fn mc_reg_gather() {
    unsafe {
        if cursors().is_empty() || g_st().exiting {
            return;
        }
        let mut gather = [false; MC_REGS.len()];
        let mut any = false;
        let mut i = 0;
        while MC_REGS[i] != 0 {
            let r = c_int::from(MC_REGS[i]);
            let idx = if r == b'"' as c_int {
                get_unname_register()
            } else {
                op_reg_index(r)
            };
            if idx >= 0 && (*get_y_register(idx)).timestamp >= mc_st().mc_start.time {
                gather[i] = true;
                any = true;
            }
            i += 1;
        }
        if !any {
            return;
        }

        let mut primary = CONTEXT_INIT;
        primary.pos = (**curwin.as_ptr()).w_cursor;
        primary.regs = shada_encode_regs(false, mc_st().mc_start.time);

        let mut order: Vec<*mut Context> = Vec::new();
        order.push(&mut primary);
        for ctx in cursors_mut().iter_mut() {
            if !ctx.regs.is_empty() && handle_get_buffer(ctx.buf) == g_st().curbuf {
                order.push(ctx);
            }
        }
        if order.len() == 1 {
            return;
        }
        order.sort_by(|&a, &b| {
            let ca = &*a;
            let cb = &*b;
            ca.pos
                .lnum
                .cmp(&cb.pos.lnum)
                .then_with(|| ca.pos.col.cmp(&cb.pos.col))
        });

        let mut save = CONTEXT_INIT;
        ctx_save(&mut save, kCtxRegs);

        let mut joined: [Vec<c_char>; MC_REGS.len()] = [const { Vec::new() }; MC_REGS.len()];
        for &ctx_ptr in &order {
            ctx_load(&mut *ctx_ptr, kCtxRegs, 0);
            let mut idx = 0;
            while MC_REGS[idx] != 0 {
                if gather[idx] {
                    let text = mc_reg_read(c_int::from(MC_REGS[idx]));
                    let text_bytes = text.to_bytes();
                    if !text_bytes.is_empty() {
                        if !joined[idx].is_empty() {
                            joined[idx].push(NL as c_char);
                        }
                        joined[idx].extend_from_slice(std::slice::from_raw_parts(
                            text.as_ptr(),
                            text_bytes.len(),
                        ));
                    }
                }
                idx += 1;
            }
        }
        drop(primary);
        ctx_load(&mut save, kCtxRegs, 0);
        ctx_free(&mut save);

        let mut idx = 0;
        while MC_REGS[idx] != 0 {
            if gather[idx] && !joined[idx].is_empty() {
                let r = c_int::from(MC_REGS[idx]);
                write_reg_contents_ex(
                    r,
                    joined[idx].as_ptr(),
                    joined[idx].len() as isize,
                    false,
                    crate::register::MotionType::kMTLineWise,
                    0,
                );
                if r == b'"' as c_int {
                    set_clipboard(NUL, get_y_previous());
                }
            }
            idx += 1;
        }
    }
}

pub fn mc_vsel_clear() {
    let buf = handle_get_buffer(mc_st().mc_vsel_buf);
        if !buf.is_null() {
            extmark_clear(buf, mc_vsel_ns(), 0, 0, MAXLNUM, MAXCOL);
            extmark_clear(buf, mc_vcur_ns(), 0, 0, MAXLNUM, MAXCOL);
        }
        mc_st().mc_vsel_buf = 0;
}

fn mc_vsel_mark(
    start_lnum: linenr_T,
    start_col: colnr_T,
    end_lnum: linenr_T,
    end_col: colnr_T,
) {
    unsafe {
        let mut decor = DECOR_INLINE_INIT;
        decor.data.hl_mut().hl_id = syn_check_group(c"MCursorVisual".as_ptr(), 13);
        let mut mark: u32 = 0;
        extmark_set(
            &mut *g_st().curbuf,
            mc_vsel_ns(),
            &mut mark,
            start_lnum - 1,
            start_col,
            end_lnum - 1,
            end_col,
            decor,
            MT_FLAG_DECOR_HL,
            false,
            false,
            true,
            false,
        );
        mc_st().mc_vsel_buf = (*g_st().curbuf).b_fnum;
    }
}

pub fn mc_vsel_refresh() {
    unsafe {
        mc_vsel_clear();
        let Some(span_cstr) = atom_visual_span() else {
            return;
        };
        if span_cstr.as_bytes().is_empty() || !mc_buf_has_cursors(&mut *g_st().curbuf) {
            return;
        }
        let span = NvimString {
            data: span_cstr.as_ptr().cast_mut(),
            size: span_cstr.to_bytes().len(),
        };

        let mut sb = McSandbox::new();
        mc_sandbox_enter(&mut sb, false);
        block_autocmds();
        block_on_key();
        g_st().emsg_silent += 1;

        for i in 0..cursors().len() {
            let mut pos = INIT_POS;
            if !mc_ctx_resolve(&cursors()[i], &mut pos) {
                continue;
            }
            cursors_mut()[i].pos = pos;
            let mut ctx = cursors()[i].clone();
            ctx_load(&mut ctx, kCtxCursor | kCtxVisual | kCtxMarks, 0);
            check_cursor(*curwin.as_ptr());
            g_st().Visual.active = false;
            g_st().Visual.select = false;
            nvim_feedkeys(span, cstr_as_string(c"nix".as_ptr()), false);
            if !g_st().Visual.active {
                continue;
            }
            let mut s = g_st().Visual.start;
            let mut e = (**curwin.as_ptr()).w_cursor;
            let r_type: c_int = if g_st().Visual.mode == b'V' as c_int {
                kMTLineWise
            } else if g_st().Visual.mode == Ctrl_V {
                kMTBlockWise
            } else {
                kMTCharWise
            };
            let mut inclusive = true;
            let mut oa = oparg_T::new();
            g_st().virtual_op = c_int::from(virtual_active(*curwin.as_ptr()));
            getregionpos_prep(
                &mut s,
                &mut e,
                r_type,
                *opt_vars().p_sel == b'e' as c_char,
                0,
                &mut inclusive,
                &mut oa,
            );
            if r_type == kMTLineWise {
                mc_vsel_mark(s.lnum, 0, e.lnum, ml_get_len(e.lnum));
            } else if r_type == kMTBlockWise {
                if (**curwin.as_ptr()).w_curswant == MAXCOL {
                    oa.end_vcol = MAXCOL;
                }
                for lnum in s.lnum..=e.lnum {
                    let mut rs = INIT_POS;
                    let mut re = INIT_POS;
                    getregionpos_line(
                        lnum, s, e, inclusive, r_type, &mut oa, false, &mut rs, &mut re,
                    );
                    if rs.col > 0 {
                        mc_vsel_mark(lnum, rs.col - 1, lnum, re.col);
                    }
                }
            } else {
                mc_vsel_mark(s.lnum, s.col, e.lnum, e.col + 1);
            }
            let mut cmark: u32 = 0;
            extmark_set_pos(
                g_st().curbuf,
                mc_vcur_ns(),
                &mut cmark,
                (**curwin.as_ptr()).w_cursor,
                false,
                true,
                true,
            );
            mc_st().mc_vsel_buf = (*g_st().curbuf).b_fnum;
            g_st().Visual.active = false;
        }

        g_st().emsg_silent -= 1;
        unblock_on_key();
        unblock_autocmds();
        mc_sandbox_leave(&mut sb);
    }
}

pub fn mc_undo_time() {
    atom_did_global_op();
        extmark_clear(g_st().curbuf, mc_ns(), 0, 0, MAXLNUM, MAXCOL);
}

pub fn mc_ins_commit() -> bool {
    unsafe {
        let active = mc_st().mc_ins_span.active;
        let ins_cascaded = active && !mc_st().mc_ins_span.first;
        mc_st().mc_ins_span.active = false;
        mc_st().mc_ins_span.vsel = None;

        if ins_cascaded
            && let Some(ins_cstr) = redo_keys(ptr::null())
        {
            let ins = NvimString {
                data: ins_cstr.as_ptr().cast_mut(),
                size: ins_cstr.as_bytes().len(),
            };
            if ins.size > mc_st().mc_ins_span.done_len && *ins.data.add(ins.size - 1) == ESC {
                mc_ins_span_flush(&ins, true);
            }
        }

        if mc_st().mc_ins_span.region != 0 {
            extmark_del_id(&mut *g_st().curbuf, mc_session_ns(), mc_st().mc_ins_span.region);
            mc_st().mc_ins_span.region = 0;
        }
        mc_ins_regions_clear();

        if active && buf_get_changedtick(&*g_st().curbuf) == mc_st().mc_ins_span.origin.tick {
            u_forget_unchanged(&mut *g_st().curbuf);
        }

        if !ins_cascaded {
            return false;
        }
        (*g_st().curbuf).b_last_changedtick = buf_get_changedtick(&*g_st().curbuf);
        (*g_st().curbuf).b_last_changedtick_i = buf_get_changedtick(&*g_st().curbuf);
        for i in 0..cursors().len() {
            let mut pos = INIT_POS;
            if !mc_ctx_resolve(&cursors()[i], &mut pos) {
                continue;
            }
            let mut mark = cursors()[i].mark;
            if mark == mc_st().mc_ins_span.origin.mcursor {
                pos = (**curwin.as_ptr()).w_cursor;
            } else if pos.col > 0 {
                dec(&mut pos);
            }
            mc_mark_upd(g_st().curbuf, &mut mark, pos);
            cursors_mut()[i].pos = pos;
            cursors_mut()[i].mark = mark;
        }
        true
    }
}

pub fn mc_buf_has_cursors(buf: &mut buf_T) -> bool {
    cursors().iter().any(|ctx| ctx.buf == buf.b_fnum)
}

pub fn mc_following() -> bool {
    unsafe { (*g_st().curbuf).b_p_follow != 0 }
}

fn mc_lua_enable(enable: bool) {
    if g_st().exiting {
            return;
        }
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_BOOL,
                v_lock: 0,
                vval: TypvalVval::Bool(if enable { kBoolVarTrue } else { kBoolVarFalse }),
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];
        nlua_call_typval(
            c"vim._core.mcursor".as_ptr(),
            c"enable".as_ptr(),
            tv_args.as_mut_ptr(),
            ptr::null_mut(),
        );
}

pub fn mc_mark_at(buf: *mut buf_T, pos: pos_T, skip: u32) -> u32 {
    unsafe {
        if cursors().is_empty() {
            return 0;
        }
        let mut itr: MarkTreeIter = MarkTreeIter::new();
        marktree_itr_get(&(*buf).b_marktree, pos.lnum - 1, pos.col, &mut itr);
        loop {
            let k = marktree_itr_current(&(*buf).b_marktree, &itr);
            if k.id == 0 || k.pos.row != pos.lnum - 1 || k.pos.col != pos.col {
                break;
            }
            if k.ns == mc_ns() && !mt_end(k) && k.id != skip {
                return k.id;
            }
            if !marktree_itr_next(&(*buf).b_marktree, &mut itr) {
                break;
            }
        }
        0
    }
}

fn mc_zq_step(keys: *const c_char, pos: pos_T) -> pos_T {
    unsafe {
        (**curwin.as_ptr()).w_cursor = pos;
        (**curwin.as_ptr()).w_set_curswant = false;
        g_st().msg_silent += 1;
        g_st().emsg_silent += 1;
        let save_cmod_flags = g_st().cmdmod.cmod_flags;
        g_st().cmdmod.cmod_flags |= CMOD_KEEPJUMPS;
        atom_suppress(true);
        block_autocmds();
        block_on_key();
        let mut sst = save_state_T::new();
        save_current_state(&mut sst);
        let beeps = g_st().did_beep_silent;
        exec_normal_cmd(keys.cast_mut(), REMAP_NONE, true);
        let failed = g_st().did_beep_silent != beeps;
        restore_current_state(&mut sst);
        unblock_on_key();
        unblock_autocmds();
        atom_suppress(false);
        g_st().cmdmod.cmod_flags = save_cmod_flags;
        g_st().emsg_silent -= 1;
        g_st().msg_silent -= 1;
        if failed { INIT_POS } else { (**curwin.as_ptr()).w_cursor }
    }
}

fn mc_zq_target(
    step: pos_T,
    search: bool,
    motion_type: c_int,
    ctx: *mut Context,
) -> visualinfo_T {
    unsafe {
        let save = (*g_st().curbuf).b_visual;
        let save_mode_eval = (*g_st().curbuf).b_visual_mode_eval;
        if !search {
            (*g_st().curbuf).b_visual = visualinfo_T {
                vi_start: step,
                vi_end: step,
                vi_curswant: step.col,
                vi_mode: if motion_type == kMTLineWise {
                    b'V' as c_int
                } else {
                    b'v' as c_int
                },
            };
        } else if mc_zq_step(c"gn\x1b".as_ptr(), step).lnum == 0 {
            (*g_st().curbuf).b_visual = visualinfo_T {
                vi_start: INIT_POS,
                vi_end: INIT_POS,
                vi_mode: 0,
                vi_curswant: 0,
            };
        }
        (*g_st().curbuf).b_visual_mode_eval = (*g_st().curbuf).b_visual.vi_mode;
        if !ctx.is_null() {
            (**curwin.as_ptr()).w_cursor = step;
            ctx_save(&mut *ctx, kCtxVisual);
        }
        let target = (*g_st().curbuf).b_visual;
        (*g_st().curbuf).b_visual = save;
        (*g_st().curbuf).b_visual_mode_eval = save_mode_eval;
        target
    }
}

fn mc_zq_first(
    keys: *const c_char,
    search: bool,
    motion_type: c_int,
    bounds: *const pos_T,
    step: *mut pos_T,
) -> bool {
    unsafe {
        let start = *bounds;
        let end_bound = *bounds.add(1);
        let curswant = (**curwin.as_ptr()).w_curswant;
        let mut before = start;
        let bof = dec(&mut before) == -1;
        (*step).lnum = 0;
        if !bof {
            let save_ww = opt_vars().p_ww;
            opt_vars().p_ww = c"s,l,>".as_ptr().cast_mut();
            let s = mc_zq_step(keys, before);
            opt_vars().p_ww = save_ww;
            (**curwin.as_ptr()).w_curswant = curswant;
            if ltoreq(start, s) && ltoreq(s, end_bound) {
                *step = s;
            }
        }
        (**curwin.as_ptr()).w_cursor = start;
        check_cursor_col(&mut **curwin.as_ptr());
        let from = (**curwin.as_ptr()).w_cursor;
        let s = mc_zq_step(keys, from);
        if s.lnum > from.lnum && !(**curwin.as_ptr()).w_set_curswant {
            let mut c = from;
            getvpos(*curwin.as_ptr(), &mut c, (**curwin.as_ptr()).w_curswant);
            if ltoreq(start, c) && ltoreq(c, end_bound) && ((*step).lnum == 0 || lt(c, *step)) {
                *step = c;
            }
        }
        (**curwin.as_ptr()).w_curswant = curswant;
        if (*step).lnum != 0 {
            return true;
        }
        let failed = s.lnum == 0;
        if failed && motion_type != kMTLineWise && equalpos(from, start) {
            return false;
        }
        if bof
            && (!search
                || equalpos(
                    mc_zq_target(start, true, motion_type, ptr::null_mut()).vi_start,
                    start,
                ))
        {
            *step = start;
            return true;
        }
        if failed || equalpos(s, from) {
            *step = start;
            return true;
        }
        *step = s;
        ltoreq(start, s) && ltoreq(s, end_bound)
    }
}

fn mc_zq_run(
    keys: *const c_char,
    search: bool,
    motion_type: c_int,
    end: pos_T,
    first: pos_T,
) -> c_int {
    unsafe {
        let wrap = end.lnum == MAXLNUM;
        let mut wrapped = false;
        let mut placed: c_int = 0;
        let mut step = first;
        let curswant = (**curwin.as_ptr()).w_curswant;
        while !g_st().got_int {
            mc_zq_target(step, search, motion_type, mc_add(g_st().curbuf, step));
            placed += 1;
            let prev = step;
            let mut from = prev;
            step = mc_zq_step(keys, from);
            if equalpos(step, from) && inc(&mut from) != -1 {
                step = mc_zq_step(keys, from);
            }
            if step.lnum == 0 || equalpos(step, prev) || equalpos(step, from) {
                break;
            }
            if lt(step, prev) {
                if !wrap || wrapped {
                    break;
                }
                wrapped = true;
            }
            if if wrapped {
                ltoreq(first, step)
            } else {
                lt(end, step)
            } {
                break;
            }
        }
        (**curwin.as_ptr()).w_curswant = curswant;
        placed
    }
}

pub fn mc_zq_start(cap: &mut cmdarg_T) {
    unsafe {
        (*cap.oap).from_visual = g_st().Visual.active;
        (*cap.oap).motion_type = kMTUnknown;
        if g_st().Visual.active {
            atom_visual_reset();
            end_visual_mode();
            check_cursor_col(&mut **curwin.as_ptr());
        }
    }
}

pub fn mc_zq(oap: &mut oparg_T, cap: &mut cmdarg_T, origin: pos_T) {
    unsafe {
        atom_did_global_op();
        if mc_replaying()
            || oap.motion_type == kMTUnknown
            || cap.cmdchar == b'i' as c_int
            || cap.cmdchar == b'a' as c_int
            || cap.cmdchar == K_LUA
            || cap.cmdchar == K_COMMAND
            || cap.cmdchar == b':' as c_int
        {
            beep_flush();
            (**curwin.as_ptr()).w_cursor = origin;
            return;
        }
        let mut primary = origin;

        let mut v = (*g_st().curbuf).b_visual;
        let mut bounds = [
            primary,
            pos_T {
                lnum: MAXLNUM,
                col: 0,
                coladd: 0,
            },
        ];
        if oap.from_visual {
            bounds[0] = (*mark_get_visual(&mut *g_st().curbuf, b'<' as c_int)).mark;
            bounds[1] = (*mark_get_visual(&mut *g_st().curbuf, b'>' as c_int)).mark;
        }

        let lines = oap.motion_type == kMTLineWise;
        if lines {
            bounds[0].col = 0;
            bounds[1].col = MAXCOL;
        }
        let mut vcols: [colnr_T; 2] = [0, MAXCOL];
        let blockwise = oap.from_visual && v.vi_mode == Ctrl_V && !lines;
        if blockwise {
            let [vcol0, vcol1] = &mut vcols;
            getvcols(
                *curwin.as_ptr(),
                &mut v.vi_start,
                &mut v.vi_end,
                vcol0,
                vcol1,
                0,
            );
            vcols[1] = if v.vi_curswant == MAXCOL {
                MAXCOL
            } else {
                vcols[1]
            };
        }
        (**curwin.as_ptr()).w_cursor = primary;
        update_curswant();
        let mut view = viewstate_T::new();
        save_viewstate(&mut **curwin.as_ptr(), &mut view);
        save_search_patterns();

        let search = (cap.cmdchar != NUL
            && b"*#/?nN".contains(&(cap.cmdchar as u8)))
            || (cap.cmdchar == b'g' as c_int
                && cap.nchar != NUL
                && b"*#nN".contains(&(cap.nchar as u8)));
        let mut placed: c_int = 0;
        let mut first = INIT_POS;
        let mut spec = atom_cmd_spec(cap);
        spec.regname = 0;
        let keys_cstr = if search {
            c"/\n".to_owned()
        } else {
            redo_keys(&spec).unwrap_or_default()
        };
        let keys_data = keys_cstr.as_ptr().cast_mut();
        if !oap.from_visual && search {
            first = mc_zq_target(primary, true, oap.motion_type, ptr::null_mut()).vi_start;
            if first.lnum != 0 {
                placed = mc_zq_run(keys_data, search, oap.motion_type, bounds[1], first);
            }
        } else if !blockwise {
            if mc_zq_first(
                keys_data,
                search,
                oap.motion_type,
                bounds.as_ptr(),
                &mut first,
            ) {
                placed = mc_zq_run(keys_data, search, oap.motion_type, bounds[1], first);
            }
        } else {
            for lnum in bounds[0].lnum..=bounds[1].lnum {
                let mut line = [
                    pos_T {
                        lnum,
                        col: 0,
                        coladd: 0,
                    },
                    pos_T {
                        lnum,
                        col: MAXCOL,
                        coladd: 0,
                    },
                ];
                if getvpos(*curwin.as_ptr(), &mut line[0], vcols[0]) == FAIL {
                    line[0].col = ml_get_len(lnum);
                } else if vcols[1] != MAXCOL {
                    getvpos(*curwin.as_ptr(), &mut line[1], vcols[1]);
                }
                let mut step = INIT_POS;
                if mc_zq_first(
                    keys_data,
                    search,
                    oap.motion_type,
                    line.as_ptr(),
                    &mut step,
                ) {
                    if first.lnum == 0 {
                        first = step;
                    }
                    placed += mc_zq_run(keys_data, search, oap.motion_type, line[1], step);
                }
            }
        }

        drop(keys_cstr);
        restore_search_patterns();
        restore_viewstate(&mut **curwin.as_ptr(), &view);

        if placed > 0 {
            if mc_mark_at(g_st().curbuf, primary, 0) == 0 {
                primary = first;
                (**curwin.as_ptr()).w_set_curswant = true;
            }
            (*g_st().curbuf).b_visual = mc_zq_target(primary, search, oap.motion_type, ptr::null_mut());
            (*g_st().curbuf).b_visual_mode_eval = (*g_st().curbuf).b_visual.vi_mode;
            set_no_hlsearch(true);
            redraw_all_later(UPD_SOME_VALID);
        }
        (**curwin.as_ptr()).w_cursor = primary;
    }
}

pub fn mc_counter(count1: c_long) {
    atom_did_global_op();
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: 0,
                vval: TypvalVval::Number(count1 as varnumber_T),
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval::Number(0),
            },
        ];
        nlua_call_typval(
            c"vim._core.mcursor".as_ptr(),
            c"number".as_ptr(),
            tv_args.as_mut_ptr(),
            ptr::null_mut(),
        );
}

pub fn mc_follow_set(on: TriState) {
    unsafe {
        if mc_replaying() {
            return;
        }
        let follow = if on == kNone {
            (*g_st().curbuf).b_p_follow == 0
        } else {
            on == kTrue
        };
        if ((*g_st().curbuf).b_p_follow != 0) != follow {
            set_option_value_give_err(kOptFollow, boolean_obj(follow), OPT_LOCAL);
        }
    }
}

pub fn mc_toggle(buf: *mut buf_T, pos: pos_T, end_follow: bool, on: TriState) {
    unsafe {
        if mc_replaying() {
            return;
        }
        if end_follow {
            mc_follow_set(kFalse);
        }
        let mark = mc_mark_at(buf, pos, 0);
        if mark != 0 && on != kTrue {
            extmark_del_id(&mut *buf, mc_ns(), mark);
            mc_cleanup(false, ptr::null(), 0);
        } else if mark == 0 && on != kFalse {
            mc_add(buf, pos);
        }
    }
}

pub fn mc_add(buf: *mut buf_T, pos: pos_T) -> *mut Context {
    unsafe {
        let existing = mc_mark_at(buf, pos, 0);
        if existing != 0 {
            for ctx in cursors_mut().iter_mut() {
                if ctx.buf == (*buf).b_fnum && ctx.mark == existing {
                    return ctx;
                }
            }
        }
        let count = cursors().len();
        let mut mark: u32 = 0;
        mc_mark_upd(buf, &mut mark, pos);
        if cursors().len() > count {
            cursors_mut()
                .last_mut()
                .map_or(ptr::null_mut(), ptr::from_mut)
        } else {
            ptr::null_mut()
        }
    }
}

pub fn mc_on_extmark_set(buf: *mut buf_T, ns_id: u32, mut id: u32, pos: pos_T) {
    unsafe {
        if ns_id != mc_ns() {
            return;
        }
        if mc_replaying() {
            extmark_del_id(&mut *buf, ns_id, id);
            return;
        }
        if mc_mark_at(buf, pos, id) != 0 {
            extmark_del_id(&mut *buf, ns_id, id);
            return;
        }
        mc_mark_upd(buf, &mut id, pos);
        atom_visual_reset();
        if cursors().is_empty() {
            mc_st().mc_start.time = os_realtime() as Timestamp;
            ctx_save(&mut mc_st().mc_start.ctx, kCtxRegs);
            mc_lua_enable(true);
        }
        let mut ctx = CONTEXT_INIT;
        ctx.buf = (*buf).b_fnum;
        ctx.pos = pos;
        ctx.mark = id;
        cursors_mut().push(ctx);
    }
}

pub fn mc_buf_clear(buf: *mut buf_T) {
    if mc_replaying() {
            return;
        }
        extmark_clear(buf, mc_ns(), 0, 0, MAXLNUM, MAXCOL);
        extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
}

pub fn mc_buf_free(buf: *mut buf_T) {
    unsafe {
        if mc_replaying() {
            return;
        }
        mc_cleanup(false, ptr::null(), 0);
        if mc_st().mc_vsel_buf == (*buf).b_fnum {
            mc_st().mc_vsel_buf = 0;
        }
    }
}

pub fn mc_ns_clearing(buf: *mut buf_T, ns_id: u32) {
    unsafe {
        if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() {
            return;
        }
        for i in 0..cursors().len() {
            let (ctx_buf, ctx_mark, mut pos) = {
                let ctx = &cursors()[i];
                (ctx.buf, ctx.mark, ctx.pos)
            };
            if ctx_buf == (*buf).b_fnum && ctx_mark != 0 {
                extmark_get_pos(buf, mc_ns(), ctx_mark, &mut pos);
                cursors_mut()[i].pos = pos;
            }
        }
    }
}

pub fn mc_ns_cleared(buf: *mut buf_T, ns_id: u32) {
    unsafe {
        if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() || !mc_buf_has_cursors(&mut *buf) {
            return;
        }

        let others = cursors().iter().any(|ctx| {
            let mut pos = INIT_POS;
            ctx.buf != (*buf).b_fnum
                || (ctx.mark != 0 && extmark_get_pos(buf, mc_ns(), ctx.mark, &mut pos))
        });
        if !others {
            mc_reg_gather();
        }

        if !g_st().exiting {
            extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
            if g_st().curbuf == buf {
                let mut primary: u32 = 1;
                extmark_set_pos(
                    buf,
                    mc_last_ns(),
                    &mut primary,
                    (**curwin.as_ptr()).w_cursor,
                    true,
                    false,
                    false,
                );
            }
            for idx in 0..cursors().len() {
                let (ctx_buf, ctx_pos) = {
                    let ctx = &cursors()[idx];
                    (ctx.buf, ctx.pos)
                };
                if ctx_buf != (*buf).b_fnum {
                    continue;
                }
                let mut mark: u32 = 0;
                extmark_set_pos(buf, mc_last_ns(), &mut mark, ctx_pos, true, false, false);
            }
        }
        mc_cleanup(false, ptr::null(), 0);
        if cursors().is_empty() {
            atom_st().g_atoms = Vec::new();
        }
    }
}

pub fn mc_free_all() {
    while let Some(mut ctx) = cursors_mut().pop() {
            ctx_free(&mut ctx);
        }
        *cursors_mut() = Vec::new();
        *ins_regions_mut() = Vec::new();
        ctx_free(&mut mc_st().mc_start.ctx);
}
