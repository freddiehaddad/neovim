//! Multicursor tracking, preview, and cascade replay (`src/nvim/mcursor.c`).
#![forbid(unsafe_code)]

use crate::api::buffer::{LUA_INTERNAL_CALL, nvim_buf_set_text};
use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    api_clear_error, boolean_obj, cstr_as_string, static_cstr_as_string, string_to_array,
};
use crate::api::private::validate::{Error, ErrorType, NvimString};
use crate::api::vim::nvim_feedkeys;
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{FileBuf, MAXLNUM, buf_get_changedtick, bufref_valid};
use crate::charset::to_special;
use crate::clipboard::set_clipboard;
use crate::context::{
    CONTEXT_INIT, Context, INIT_POS, K_CTX_COUNT, K_CTX_CURSOR, K_CTX_MARKS, K_CTX_MERGE_REG,
    K_CTX_REGS, K_CTX_VISUAL, ctx_free, ctx_load, ctx_save, handle_get_buffer,
    handle_get_buffer_opt,
};
use crate::cursor::{
    Colnr, CursorPos, Linenr, check_cursor, check_cursor_col, check_pos, equalpos, getvpos, lt,
    ltoreq,
};
use crate::decoration::DecorInline;
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::eval::ESC;
use crate::eval::typval::{TypVal, TypvalVval, VarType, Varnumber};
use crate::eval::vars::{VV_CASCADING, get_vim_var_tv, set_vim_var_bool};
use crate::eval::window::Viewstate;
use crate::ex_docmd::{
    CMOD_KEEPJUMPS, exec_normal_cmd, restore_current_state, save_current_state, set_no_hlsearch,
};
use crate::extmark::{
    K_FALSE, K_NONE, K_TRUE, TriState, extmark_clear, extmark_del_id, extmark_from_id,
    extmark_get_pos, extmark_set, extmark_set_pos,
};
use crate::globals::*;
use crate::highlight_group::syn_check_group;
use crate::input::{
    NUL, RedoState, beep_flush, block_on_key, redo_keys, restore_redobuff, save_redobuff,
    unblock_on_key,
};
use crate::input_cmdatom::{
    CmdAtom, CmdOrigin, Cmdarg, K_AINSERT_SPAN, K_AMOTION, K_AVISUAL_SPAN, K_KEY_INS_FLUSH,
    atom_cmd_spec, atom_composite_active, atom_composite_queued, atom_did_global_op,
    atom_key_class, atom_lhs_replay_queue, atom_push_raw, atom_suppress, atom_visual_reset,
    atom_visual_span, with_atom,
};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_COMMAND, K_SPECIAL, NL};
use crate::log::{LOGLVL_DBG, logmsg, min_log_level};
use crate::lua::executor::nlua_call_typval;
use crate::mapping::REMAP_NONE;
use crate::mark::mark_get_visual;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorInlineData, MT_FLAG_DECOR_HL, MTPair, MarkTreeIter,
    marktree_itr_current, marktree_itr_get, marktree_itr_next, mt_end,
};
use crate::memline::{dec, inc, ml_get_len, ml_region_text};
use crate::menu::SaveState;
use crate::r#move::{
    restore_viewstate, save_viewstate, set_topline, update_curswant, update_topline,
};
use crate::msgpack_rpc::packer::ObjHandle;
use crate::normal::{VisualState, Visualinfo, end_visual_mode, visualinfo};
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
use crate::strings::format_to_buf;
use crate::textformat::{InsState, Oparg};
use crate::undo::{u_forget_unchanged, u_save, uhp_mut};
use std::ffi::CStr;

const DECOR_INLINE_INIT: DecorInline = DecorInline {
    data: DecorInlineData::Hl(DECOR_HIGHLIGHT_INLINE_INIT),
};

pub(crate) const K_OPT_FOLLOW: i32 = 114;

const MC_REGS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789\"-\0";

use crate::digraph::{K_BOOL_VAR_FALSE_CONST, K_BOOL_VAR_TRUE_CONST};
use crate::eval::funcs::{MT_BLOCK_WISE, MT_CHAR_WISE, MT_LINE_WISE, MT_UNKNOWN};
use crate::ex_getln::K_LUA;
use crate::regexp::{CTRL_V_CONST, MAXCOL};

struct McSandbox {
    primary: Context,
    cursor_mark: u32,
    topline_mark: u32,
    view: Viewstate,
    sst: SaveState,
    redo: RedoState,
    visual: VisualState,
}

impl McSandbox {
    fn new() -> Self {
        Self {
            primary: CONTEXT_INIT,
            cursor_mark: 0,
            topline_mark: 0,
            view: Viewstate::new(),
            sst: SaveState::new(),
            redo: RedoState::new(),
            visual: VisualState::new(),
        }
    }
}

struct McInsSaved {
    ins: InsState,
    last_changedtick: Varnumber,
    last_changedtick_i: Varnumber,
}

struct McInsSpan {
    origin: crate::globals::CloneCell<CmdOrigin>,
    regions: std::cell::RefCell<Vec<u32>>,
    active: std::cell::Cell<bool>,
    first: std::cell::Cell<bool>,
    frame: std::cell::Cell<u64>,
    done_len: std::cell::Cell<usize>,
    region: std::cell::Cell<u32>,
    vsel: std::cell::RefCell<Option<std::ffi::CString>>,
}

struct McStart {
    time: std::cell::Cell<Timestamp>,
    ctx: std::cell::RefCell<Context>,
}

struct McursorState {
    mc_ins_span: McInsSpan,
    mc_start: McStart,
    mc_vsel_buf: std::cell::Cell<ObjHandle>,
    mc_ins_joined: std::cell::Cell<bool>,
    mc_cursors: std::cell::RefCell<Vec<Context>>,
    ns_mc: std::cell::Cell<u32>,
    ns_vcur: std::cell::Cell<u32>,
    ns_vsel: std::cell::Cell<u32>,
    ns_last: std::cell::Cell<u32>,
    ns_session: std::cell::Cell<u32>,
}

impl McursorState {
    const fn new() -> Self {
        Self {
            mc_ins_span: McInsSpan {
                origin: crate::globals::CloneCell::new(CmdOrigin {
                    buf: crate::buffer::Bufref {
                        br_buf: crate::globals::BufPtr::NULL,
                        br_fnum: 0,
                        br_buf_free_count: 0,
                    },
                    win: crate::globals::WinPtr::NULL,
                    pos: INIT_POS,
                    mcursor: 0,
                    tick: 0,
                    maptick: 0,
                }),
                regions: std::cell::RefCell::new(Vec::new()),
                active: std::cell::Cell::new(false),
                first: std::cell::Cell::new(false),
                frame: std::cell::Cell::new(0),
                done_len: std::cell::Cell::new(0),
                region: std::cell::Cell::new(0),
                vsel: std::cell::RefCell::new(None),
            },
            mc_start: McStart {
                time: std::cell::Cell::new(0),
                ctx: std::cell::RefCell::new(CONTEXT_INIT),
            },
            mc_vsel_buf: std::cell::Cell::new(0),
            mc_ins_joined: std::cell::Cell::new(false),
            mc_cursors: std::cell::RefCell::new(Vec::new()),
            ns_mc: std::cell::Cell::new(0),
            ns_vcur: std::cell::Cell::new(0),
            ns_vsel: std::cell::Cell::new(0),
            ns_last: std::cell::Cell::new(0),
            ns_session: std::cell::Cell::new(0),
        }
    }
}

thread_local! {
    static MC_STATE: McursorState = const { McursorState::new() };
}

#[inline]
fn with_mc<R>(f: impl FnOnce(&McursorState) -> R) -> R {
    MC_STATE.with(f)
}

#[inline]
fn cursors_len() -> usize {
    with_mc(|st| st.mc_cursors.borrow().len())
}

#[inline]
fn cursors_is_empty() -> bool {
    with_mc(|st| st.mc_cursors.borrow().is_empty())
}

#[inline]
fn cursor_clone(idx: usize) -> Context {
    with_mc(|st| st.mc_cursors.borrow()[idx].clone())
}

pub fn mc_init() {
    mc_ns();
}

fn mc_ns() -> u32 {
    let cur = with_mc(|st| st.ns_mc.get());
    if cur == 0 {
        let id = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor")) as u32;
        with_mc(|st| st.ns_mc.set(id));
        id
    } else {
        cur
    }
}

fn mc_vcur_ns() -> u32 {
    let cur = with_mc(|st| st.ns_vcur.get());
    if cur == 0 {
        let id = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.cursor")) as u32;
        with_mc(|st| st.ns_vcur.set(id));
        id
    } else {
        cur
    }
}

fn mc_vsel_ns() -> u32 {
    let cur = with_mc(|st| st.ns_vsel.get());
    if cur == 0 {
        let id = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.visual")) as u32;
        with_mc(|st| st.ns_vsel.set(id));
        id
    } else {
        cur
    }
}

fn mc_last_ns() -> u32 {
    let cur = with_mc(|st| st.ns_last.get());
    if cur == 0 {
        let id = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor.last")) as u32;
        with_mc(|st| st.ns_last.set(id));
        id
    } else {
        cur
    }
}

fn mc_session_ns() -> u32 {
    let cur = with_mc(|st| st.ns_session.get());
    if cur == 0 {
        let id = nvim_create_namespace(static_cstr_as_string(c"nvim.multicursor._session")) as u32;
        with_mc(|st| st.ns_session.set(id));
        id
    } else {
        cur
    }
}

pub fn mc_showcmd(buf: &mut [u8]) -> usize {
    let count = cursors_len();
    if count == 0 {
        if !buf.is_empty() {
            buf[0] = 0;
        }
        return 0;
    }
    let prefix = if mc_following() { c"=" } else { c"" };
    format_to_buf!(buf, c"%s%zu\xc3\x97 ", prefix, count) as usize
}

pub fn mc_replaying() -> bool {
    get_vim_var_tv(VV_CASCADING).tv().vval.v_bool() == K_BOOL_VAR_TRUE_CONST
}

fn mc_mark_upd(buf: &FileBuf, mark: &mut u32, pos: CursorPos) {
    extmark_set_pos(buf, mc_ns(), mark, pos, true, false, true);
}

fn mc_track_upd(buf: &FileBuf, mark: &mut u32, pos: CursorPos) {
    extmark_set_pos(buf, mc_session_ns(), mark, pos, true, true, false);
}

fn mc_sandbox_enter(sb: &mut McSandbox, save_regs: bool) {
    assert!(!mc_replaying());
    set_vim_var_bool(VV_CASCADING, K_BOOL_VAR_TRUE_CONST);
    sb.primary = CONTEXT_INIT;
    ctx_save(
        &mut sb.primary,
        K_CTX_CURSOR
            | K_CTX_VISUAL
            | K_CTX_MARKS
            | K_CTX_COUNT
            | if save_regs { K_CTX_REGS } else { 0 },
    );
    sb.cursor_mark = 0;
    mc_track_upd(&curbuf(), &mut sb.cursor_mark, sb.primary.pos);
    save_viewstate(&curwin(), &mut sb.view);
    sb.topline_mark = 0;
    mc_track_upd(
        &curbuf(),
        &mut sb.topline_mark,
        CursorPos {
            lnum: sb.view.vs_topline,
            col: 0,
            coladd: 0,
        },
    );
    sb.visual = *g_st().visual_global.borrow();
    save_current_state(&mut sb.sst);
    save_search_patterns();
    save_redobuff(&mut sb.redo);
}

fn mc_sandbox_leave(sb: &mut McSandbox) {
    if !sb.primary.regs.is_empty() {
        ctx_load(&mut sb.primary, K_CTX_REGS, 0);
    }
    ctx_load(&mut sb.primary, K_CTX_COUNT, 0);
    restore_redobuff(&mut sb.redo);
    restore_search_patterns();
    *g_st().visual_global.borrow_mut() = sb.visual;
    restore_current_state(&mut sb.sst);
    let mut topline = INIT_POS;
    let mut topline_valid = false;
    let is_curbuf = if let Some(buf) = handle_get_buffer_opt(sb.primary.buf) {
        extmark_get_pos(&buf, mc_session_ns(), sb.cursor_mark, &mut sb.primary.pos);
        extmark_del_id(&buf, mc_session_ns(), sb.cursor_mark);
        topline_valid = extmark_get_pos(&buf, mc_session_ns(), sb.topline_mark, &mut topline);
        extmark_del_id(&buf, mc_session_ns(), sb.topline_mark);
        g_st().curbuf.get() == buf
    } else {
        g_st().curbuf.get().is_null()
    };
    if is_curbuf {
        restore_viewstate(&curwin(), &sb.view);
        ctx_load(
            &mut sb.primary,
            K_CTX_CURSOR | K_CTX_VISUAL | K_CTX_MARKS,
            0,
        );
        check_cursor(&curwin());
        if topline_valid {
            set_topline(
                &curwin(),
                topline.lnum.min(curbuf().b_ml.ml_line_count.get()),
            );
            update_topline(&curwin());
        }
    }
    ctx_free(&mut sb.primary);
    set_vim_var_bool(VV_CASCADING, K_BOOL_VAR_FALSE_CONST);
}

fn mc_execute(cursoridx: usize, atomidx: usize) {
    let mut ctx = cursor_clone(cursoridx);
    let (atom_keys, atom_type, atom_cmd, atom_cmd2, atom_remap) = with_atom(|st| {
        let a = &st.g_atoms.borrow()[atomidx];
        (a.keys.clone(), a.r#type, a.spec.cmd, a.spec.cmd2, a.remap)
    });

    if !handle_get_buffer(ctx.buf).is_some_and(|b| g_st().curbuf.get() == b) {
        return;
    }

    if ctx.mark != 0 && !extmark_get_pos(&curbuf(), mc_ns(), ctx.mark, &mut ctx.pos) {
        return;
    }
    let atom_keys_opt = atom_keys.as_deref();
    if ctx.visual.vi_start.lnum == 0 && atom_keys_opt.unwrap_or(c"").to_bytes().starts_with(b"gv") {
        return;
    }

    let swap_regs = atom_type != K_AMOTION;
    let mut regs_ts: Timestamp = 0;
    if swap_regs {
        if reg_max_ts(false) >= with_mc(|st| st.mc_start.time.get()) {
            let mut start_ctx = with_mc(|st| st.mc_start.ctx.borrow().clone());
            ctx_load(&mut start_ctx, K_CTX_REGS, 0);
        }
        if !ctx.regs.is_empty() {
            ctx_load(&mut ctx, K_CTX_REGS, K_CTX_MERGE_REG);
        }
        regs_ts = reg_max_ts(false);
    }

    let mark_cmd = if atom_cmd == b'g' as i32 {
        atom_cmd2
    } else {
        atom_cmd
    };
    let swap_marks =
        atom_type != K_AMOTION || atom_remap || mark_cmd == b'\'' as i32 || mark_cmd == b'`' as i32;

    ctx_load(
        &mut ctx,
        if swap_marks {
            K_CTX_VISUAL | K_CTX_MARKS | K_CTX_CURSOR
        } else {
            K_CTX_CURSOR
        },
        0,
    );

    if atom_type == K_AINSERT_SPAN {
        curwin().w_cursor.update_mut(|c| check_pos(&curbuf(), c));
    } else {
        check_cursor(&curwin());
    }

    if atom_type == K_AVISUAL_SPAN {
        g_st().visual_global.borrow_mut().active = false;
        g_st().visual_global.borrow_mut().select = false;
    }

    if atom_type == K_AINSERT_SPAN {
        g_st().state_global.set(MODE_INSERT);
    }

    let beeps = g_st().did_beep.get();
    let tick = buf_get_changedtick(&curbuf());
    let mode_str = if atom_remap { c"ix" } else { c"nix" };
    nvim_feedkeys(
        cstr_as_string(atom_keys_opt),
        cstr_as_string(Some(mode_str)),
        false,
    );

    if atom_type == K_AINSERT_SPAN
        && with_mc(|st| st.mc_ins_span.first.get())
        && g_st().did_beep.get() != beeps
        && buf_get_changedtick(&curbuf()) == tick
    {
        extmark_del_id(&curbuf(), mc_ns(), ctx.mark);
        return;
    }

    if g_st().visual_global.borrow().active {
        if atom_type == K_AVISUAL_SPAN {
            *curbuf().b_visual.borrow_mut() = visualinfo();
            curbuf()
                .b_visual_mode_eval
                .set(g_st().visual_global.borrow().mode);
        }
        g_st().visual_global.borrow_mut().active = false;
        g_st().visual_global.borrow_mut().select = false;
    }

    let wrote_regs = swap_regs && reg_max_ts(false) != regs_ts;

    if cursoridx >= cursors_len() {
        return;
    }
    if !extmark_get_pos(&curbuf(), mc_ns(), ctx.mark, &mut ctx.pos) {
        return;
    }

    if wrote_regs {
        ctx.regs = shada_encode_regs(false, with_mc(|st| st.mc_start.time.get()));
    }
    ctx_save(
        &mut ctx,
        if swap_marks {
            K_CTX_VISUAL | K_CTX_MARKS | K_CTX_CURSOR
        } else {
            K_CTX_CURSOR
        },
    );

    if atom_type == K_AINSERT_SPAN && curbuf().b_last_insert.borrow().mark.lnum > 0 {
        ctx.pos = curbuf().b_last_insert.borrow().mark;
    }
    mc_mark_upd(&curbuf(), &mut ctx.mark, ctx.pos);
    with_mc(|st| st.mc_cursors.borrow_mut()[cursoridx] = ctx);
}

fn mc_cascade() {
    let (origin, mut edits, atoms_len) = with_atom(|st| {
        let g_atoms = st.g_atoms.borrow();
        assert!(!g_atoms.is_empty());
        let origin = g_atoms[0].origin.clone();
        let edits = g_atoms.iter().any(|a| a.r#type != K_AMOTION);
        (origin, edits, g_atoms.len())
    });
    let _ = &mut edits;
    let mcursor = if origin.buf.br_buf == g_st().curbuf.get() {
        origin.mcursor
    } else {
        0
    };
    assert!(!cursors_is_empty());
    assert!(!mc_replaying());

    g_st().got_int.set(false);

    if edits {
        mc_cleanup(true, None, 0);
        if cursors_is_empty() {
            with_atom(|st| st.g_atoms.borrow_mut().clear());
            return;
        }
    }

    let mut mcursor_pos = INIT_POS;
    if mcursor != 0 && extmark_get_pos(&curbuf(), mc_ns(), mcursor, &mut mcursor_pos) {
        let mut mark = mcursor;
        mc_mark_upd(&curbuf(), &mut mark, curwin().w_cursor.get());
    }

    let mut sb = McSandbox::new();
    mc_sandbox_enter(&mut sb, edits);

    'outer: for ai in 0..atoms_len {
        let atom_origin = with_atom(|st| st.g_atoms.borrow()[ai].origin.clone());
        if !atom_origin.buf.br_buf.is_null()
            && (!bufref_valid(Some(&atom_origin.buf))
                || atom_origin.buf.br_buf != g_st().curbuf.get())
        {
            continue;
        }
        let mut ci = 0;
        while ci < cursors_len() {
            line_breakcheck();
            if g_st().got_int.get() {
                break 'outer;
            }
            let cur_mark = with_mc(|st| st.mc_cursors.borrow()[ci].mark);
            if mcursor == 0 || cur_mark != mcursor {
                mc_execute(ci, ai);
            }
            ci += 1;
        }
    }

    with_atom(|st| st.g_atoms.borrow_mut().clear());
    let bufnr = sb.primary.buf;
    mc_sandbox_leave(&mut sb);

    mc_cleanup(true, Some(&curwin().w_cursor.get()), mcursor);
    if handle_get_buffer(bufnr).is_some_and(|b| g_st().curbuf.get() == b)
        && !curbuf().b_u_synced.get()
        && let Some(nh) = curbuf().b_u_newhead.get()
    {
        uhp_mut(&curbuf(), nh).uh_cursor_after = curwin().w_cursor.get();
    }
}

pub fn mc_clock_edge(map_edit: bool, map_moved: bool, follow: bool) {
    if (map_edit || (follow && map_moved && !g_st().visual_global.borrow().active))
        && !atom_composite_queued()
        && with_atom(|st| st.g_atoms.borrow().is_empty())
        && atom_composite_active()
        && mc_buf_has_cursors(&curbuf())
    {
        atom_lhs_replay_queue();
    }
    if mc_buf_has_cursors(&curbuf()) && with_atom(|st| !st.g_atoms.borrow().is_empty()) {
        let has_edit =
            map_edit || with_atom(|st| st.g_atoms.borrow().iter().any(|a| a.r#type != K_AMOTION));
        if has_edit || follow || g_st().visual_global.borrow().active {
            mc_cascade();
        } else {
            with_atom(|st| st.g_atoms.borrow_mut().clear());
        }
    }
}

fn mc_cleanup(dedupe: bool, primary: Option<&CursorPos>, keep_mark: u32) {
    let len = cursors_len();
    let had_cursors = len != 0;
    let mut n = 0;
    for i in 0..len {
        let mut ctx = cursor_clone(i);
        let Some(buf) = handle_get_buffer_opt(ctx.buf) else {
            ctx_free(&mut ctx);
            continue;
        };
        if ctx.mark != 0 && !extmark_get_pos(&buf, mc_ns(), ctx.mark, &mut ctx.pos) {
            ctx_free(&mut ctx);
            continue;
        }
        let dup = dedupe
            && ctx.mark != 0
            && ((!(curwin()).is_null()
                && g_st().curbuf.get() == buf
                && primary.is_some_and(|p| equalpos(ctx.pos, *p))
                && ctx.mark != keep_mark)
                || mc_mark_at(&buf, ctx.pos, 0) != ctx.mark);
        if dup {
            extmark_del_id(&buf, mc_ns(), ctx.mark);
            ctx_free(&mut ctx);
        } else {
            with_mc(|st| st.mc_cursors.borrow_mut()[n] = ctx);
            n += 1;
        }
    }
    with_mc(|st| st.mc_cursors.borrow_mut().truncate(n));
    if n == 0 && had_cursors {
        with_mc(|st| {
            ctx_free(&mut st.mc_start.ctx.borrow_mut());
            st.mc_start.time.set(0);
        });
        mc_lua_enable(false);
    }
}

pub fn mc_ins_join() {
    if !mc_replaying() {
        with_mc(|st| st.mc_ins_joined.set(true));
    }
}

pub fn mc_ins_replay_can_join() -> bool {
    !mc_replaying() || with_mc(|st| st.mc_ins_joined.get())
}

pub fn mc_ins_cascade_start(
    cascade: bool,
    origin: CmdOrigin,
    root_frame: u64,
    vsel: Option<std::ffi::CString>,
) {
    if mc_replaying() {
        return;
    }
    let active = cascade && mc_buf_has_cursors(&curbuf());
    with_mc(|st| {
        *st.mc_ins_span.vsel.borrow_mut() = vsel;
        st.mc_ins_joined.set(false);
        st.mc_ins_span.active.set(active);
        st.mc_ins_span.first.set(true);
        st.mc_ins_span.frame.set(root_frame);
        st.mc_ins_span.done_len.set(0);
        st.mc_ins_span.origin.set(origin);
        st.mc_ins_span.region.set(0);
    });
    mc_ins_regions_clear();

    if active {
        let mut lo = curwin().w_cursor.get().lnum;
        let mut hi = curwin().w_cursor.get().lnum;
        for i in 0..cursors_len() {
            let ctx = cursor_clone(i);
            let mut pos = INIT_POS;
            if mc_ctx_resolve(&ctx, &mut pos) {
                lo = lo.min(pos.lnum);
                hi = hi.max(pos.lnum);
            }
        }
        let _ = u_save(lo - 1, hi + 1);
    }
}

fn mc_ins_save_state() -> McInsSaved {
    let saved = McInsSaved {
        ins: *g_st().ins_global.borrow(),
        last_changedtick: curbuf().b_last_changedtick.get(),
        last_changedtick_i: curbuf().b_last_changedtick_i.get(),
    };
    g_st().ins_global.borrow_mut().did_ai = false;
    g_st().ins_global.borrow_mut().ai_col = 0;
    g_st().ins_global.borrow_mut().end_comment_pending = NUL;
    g_st().ins_global.borrow_mut().did_si = false;
    g_st().ins_global.borrow_mut().can_si = false;
    g_st().ins_global.borrow_mut().can_si_back = false;
    saved
}

fn mc_ins_restore_state(saved: &McInsSaved) {
    *g_st().ins_global.borrow_mut() = saved.ins;
    curbuf().b_last_changedtick.set(saved.last_changedtick);
    curbuf().b_last_changedtick_i.set(saved.last_changedtick_i);
}

fn mc_ins_span_push(keys: std::ffi::CString, text: Option<std::ffi::CString>) {
    let cascade = mc_buf_has_cursors(&curbuf());
    let mut atom = CmdAtom::new();
    atom.r#type = K_AINSERT_SPAN;
    atom.keys = Some(keys);
    atom.text = text;
    atom.origin = with_mc(|st| st.mc_ins_span.origin.get());
    atom_push_raw(cascade, &mut atom);
    if cascade {
        let saved = mc_ins_save_state();
        block_autocmds();
        mc_cascade();
        unblock_autocmds();
        mc_ins_restore_state(&saved);
        with_mc(|st| st.mc_ins_joined.set(false));
    }
    with_mc(|st| st.mc_ins_span.first.set(false));
}

fn mc_ins_regions_clear() {
    while let Some(id) = with_mc(|st| st.mc_ins_span.regions.borrow_mut().pop()) {
        extmark_del_id(&curbuf(), mc_session_ns(), id);
    }
}

#[inline]
fn mc_ctx_resolve(ctx: &Context, pos: &mut CursorPos) -> bool {
    handle_get_buffer(ctx.buf).is_some_and(|b| g_st().curbuf.get() == b)
        && ctx.mark != 0
        && extmark_get_pos(&curbuf(), mc_ns(), ctx.mark, pos)
}

fn mc_region_mark_set(mark: &mut u32, pos: CursorPos) {
    extmark_set(
        &curbuf(),
        mc_session_ns(),
        Some(&mut *mark),
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

fn mc_ins_preview_rebase() {
    let mut region = with_mc(|st| st.mc_ins_span.region.get());
    mc_region_mark_set(&mut region, curwin().w_cursor.get());
    with_mc(|st| st.mc_ins_span.region.set(region));
    mc_ins_regions_clear();
    let origin_mcursor = with_mc(|st| st.mc_ins_span.origin.get().mcursor);
    for i in 0..cursors_len() {
        let ctx = cursor_clone(i);
        let mut pos = INIT_POS;
        let mut mark = 0;
        if mc_ctx_resolve(&ctx, &mut pos) && ctx.mark != origin_mcursor {
            mc_region_mark_set(&mut mark, pos);
            with_mc(|st| st.mc_ins_span.regions.borrow_mut().push(mark));
        }
    }
}

fn mc_ins_region_first() -> MTPair {
    let regions = with_mc(|st| st.mc_ins_span.regions.borrow().clone());
    for id in regions {
        let p = extmark_from_id(&curbuf(), mc_session_ns(), id);
        if p.start.id != 0 {
            return p;
        }
    }
    MTPair::new()
}

#[inline]
fn mc_region_range(p: MTPair, start: &mut CursorPos, end: &mut CursorPos) {
    *start = CursorPos {
        lnum: p.start.pos.row + 1,
        col: p.start.pos.col,
        coladd: 0,
    };
    *end = CursorPos {
        lnum: p.end_pos.row + 1,
        col: p.end_pos.col,
        coladd: 0,
    };
}

fn mc_region_mark_range(mark: u32, start: &mut CursorPos, end: &mut CursorPos) -> bool {
    let p = extmark_from_id(&curbuf(), mc_session_ns(), mark);
    if p.start.id == 0 {
        return false;
    }
    mc_region_range(p, &mut *start, &mut *end);
    true
}

fn mc_ins_preview_replace(start: CursorPos, end: CursorPos, text: &NvimString) {
    let mut err = Error::new();
    let input = if !text.data.is_null() {
        text.clone()
    } else {
        NvimString::new()
    };
    let lines = string_to_array(input, false);
    nvim_buf_set_text(
        LUA_INTERNAL_CALL,
        0,
        i64::from(start.lnum - 1),
        i64::from(start.col),
        i64::from(end.lnum - 1),
        i64::from(end.col),
        lines,
        &mut err,
    );
    if err.err_type != ErrorType::ErrorTypeNone {
        if min_log_level() <= LOGLVL_DBG {
            logmsg!(
                LOGLVL_DBG,
                None,
                Some(c"mc_ins_preview_replace"),
                747,
                true,
                c"preview replace failed: %s",
                err.msg_cstr(),
            );
        }
        api_clear_error(&mut err);
    }
}

fn mc_ins_preview_set(new_text: &NvimString) {
    let saved = mc_ins_save_state();
    let mut primary: u32 = 0;
    mc_track_upd(&curbuf(), &mut primary, curwin().w_cursor.get());
    let save_cmod_flags = g_st().cmdmod.borrow().cmod_flags;
    g_st().cmdmod.borrow_mut().cmod_flags |= CMOD_KEEPJUMPS;
    set_vim_var_bool(VV_CASCADING, K_BOOL_VAR_TRUE_CONST);
    let regions = with_mc(|st| st.mc_ins_span.regions.borrow().clone());
    for region_id in regions {
        let mut rs = INIT_POS;
        let mut re = INIT_POS;
        if !mc_region_mark_range(region_id, &mut rs, &mut re) {
            continue;
        }
        mc_ins_preview_replace(rs, re, new_text);
    }
    set_vim_var_bool(VV_CASCADING, K_BOOL_VAR_FALSE_CONST);
    g_st().cmdmod.borrow_mut().cmod_flags = save_cmod_flags;
    let mut pos = curwin().w_cursor.get();
    if extmark_get_pos(&curbuf(), mc_session_ns(), primary, &mut pos) {
        curwin().w_cursor.set(pos);
    }
    extmark_del_id(&curbuf(), mc_session_ns(), primary);
    mc_ins_restore_state(&saved);
}

fn mc_ins_keys_nonliteral(keys: &[u8]) -> bool {
    let len = keys.len();
    let mut i = 0;
    while i < len {
        let mut key = i32::from(keys[i]);
        if key == i32::from(K_SPECIAL) && i + 2 < len {
            key = to_special(i32::from(keys[i + 1]), i32::from(keys[i + 2]));
            i += 2;
        }
        if (atom_key_class(key, NUL, NUL) & K_KEY_INS_FLUSH) != 0 {
            return true;
        }
        i += 1;
    }
    false
}

pub fn mc_ins_cascade_restart() {
    if !with_mc(|st| st.mc_ins_span.active.get())
        || mc_replaying()
        || (g_st().state_global.get() & MODE_INSERT) == 0
        || !mc_buf_has_cursors(&curbuf())
        || with_atom(|st| !st.g_atoms.borrow().is_empty())
    {
        return;
    }
    let ins = redo_keys(None);
    with_mc(|st| {
        st.mc_ins_span
            .done_len
            .set(ins.as_ref().map_or(0, |s| s.as_bytes().len()));
    });
    mc_ins_preview_rebase();
}

pub fn mc_ins_replaying() -> bool {
    mc_replaying() && mc_ins_cascading()
}

pub fn mc_ins_cascading() -> bool {
    with_mc(|st| st.mc_ins_span.active.get())
}

pub fn mc_ins_cascaded(root_frame: u64) -> bool {
    with_mc(|st| !st.mc_ins_span.first.get() && st.mc_ins_span.frame.get() == root_frame)
}

pub fn mc_ins_cascade() {
    if !with_mc(|st| st.mc_ins_span.active.get())
        || mc_replaying()
        || (g_st().state_global.get() & MODE_INSERT) == 0
        || !mc_buf_has_cursors(&curbuf())
    {
        return;
    }

    let mut start = INIT_POS;
    let mut end = INIT_POS;
    let ins_cstr = redo_keys(None);
    let ins = NvimString::from_opt_cstr(ins_cstr.as_deref());
    let (first, done_len, region) = with_mc(|st| {
        (
            st.mc_ins_span.first.get(),
            st.mc_ins_span.done_len.get(),
            st.mc_ins_span.region.get(),
        )
    });

    if first {
        if !g_st().ins_global.borrow().did_ai && !ins.data.is_null() && ins.size > 0 {
            let mut keys: Vec<u8> = Vec::new();
            let mut skip = 0;
            if let Some(vsel) = with_mc(|st| st.mc_ins_span.vsel.borrow_mut().take()) {
                assert!(ins.as_bytes().starts_with(b"1v"));
                keys.extend_from_slice(vsel.to_bytes());
                skip = 2;
            }
            keys.extend_from_slice(&ins.as_bytes()[skip..]);
            keys.push(ESC);
            with_mc(|st| st.mc_ins_span.done_len.set(ins.size));
            mc_ins_span_push(std::ffi::CString::new(keys).unwrap(), None);
            mc_ins_preview_rebase();
        }
    } else if !ins.data.is_null() && ins.size < done_len {
        mc_ins_cascade_restart();
    } else if ins.size > done_len && mc_ins_keys_nonliteral(&ins.as_bytes()[done_len..]) {
        if !ins_compl_active() && !pum_visible() {
            mc_ins_span_flush(&ins, false);
        }
    } else if mc_region_mark_range(region, &mut start, &mut end) {
        let text = ml_region_text(&crate::globals::curbuf(), start, end);
        let mut update = true;
        let fp = mc_ins_region_first();
        if fp.start.id != 0 {
            let mut frs = INIT_POS;
            let mut fre = INIT_POS;
            mc_region_range(fp, &mut frs, &mut fre);
            let cur = ml_region_text(&crate::globals::curbuf(), frs, fre);
            update = cur != text;
        }
        if update && (ins_compl_active() || pum_visible()) {
            let regions = with_mc(|st| st.mc_ins_span.regions.borrow().clone());
            for id in regions {
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
            let text_str = NvimString::from_bytes(&text);
            mc_ins_preview_set(&text_str);
        }
    }
    drop(ins_cstr);
}

fn mc_ins_span_flush(ins: &NvimString, commit: bool) {
    let empty = NvimString::new();
    let fp = mc_ins_region_first();
    if fp.start.id != 0
        && (fp.start.pos.row != fp.end_pos.row || fp.start.pos.col != fp.end_pos.col)
    {
        mc_ins_preview_set(&empty);
    }
    let done = with_mc(|st| st.mc_ins_span.done_len.get());
    let ins_bytes = ins.as_bytes();
    let dlen = ins_bytes.len() - done;
    let mut keys: Vec<u8> = Vec::with_capacity(dlen + 2);
    keys.push(b'i');
    if dlen > 0 {
        keys.extend_from_slice(&ins_bytes[done..]);
    }
    if !commit {
        keys.push(ESC);
    }
    let text = if commit && dlen > 1 {
        Some(std::ffi::CString::new(&ins_bytes[done..done + dlen - 1]).unwrap())
    } else {
        None
    };
    with_mc(|st| st.mc_ins_span.done_len.set(ins.size));
    mc_ins_span_push(std::ffi::CString::new(keys).unwrap(), text);
    if !commit {
        mc_ins_preview_rebase();
    }
}

fn mc_reg_read(reg: i32) -> std::ffi::CString {
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
    if cursors_is_empty() || g_st().exiting.get() {
        return;
    }
    let start_time = with_mc(|st| st.mc_start.time.get());
    let mut gather = [false; MC_REGS.len()];
    let mut any = false;
    let mut i = 0;
    while MC_REGS[i] != 0 {
        let r = i32::from(MC_REGS[i]);
        let idx = if r == b'"' as i32 {
            get_unname_register()
        } else {
            op_reg_index(r).map_or(-1, |idx| idx as i32)
        };
        if idx >= 0 && get_y_register(idx).with(|r| r.timestamp >= start_time) {
            gather[i] = true;
            any = true;
        }
        i += 1;
    }
    if !any {
        return;
    }

    let mut primary = CONTEXT_INIT;
    primary.pos = curwin().w_cursor.get();
    primary.regs = shada_encode_regs(false, start_time);

    let mut order: Vec<Context> = Vec::new();
    order.push(primary);
    for idx in 0..cursors_len() {
        let ctx = cursor_clone(idx);
        if !ctx.regs.is_empty()
            && handle_get_buffer(ctx.buf).is_some_and(|b| g_st().curbuf.get() == b)
        {
            order.push(ctx);
        }
    }
    if order.len() == 1 {
        return;
    }
    order.sort_by(|a, b| {
        a.pos
            .lnum
            .cmp(&b.pos.lnum)
            .then_with(|| a.pos.col.cmp(&b.pos.col))
    });

    let mut save = CONTEXT_INIT;
    ctx_save(&mut save, K_CTX_REGS);

    let mut joined: [Vec<u8>; MC_REGS.len()] = [const { Vec::new() }; MC_REGS.len()];
    for mut ctx in order {
        ctx_load(&mut ctx, K_CTX_REGS, 0);
        let mut idx = 0;
        while MC_REGS[idx] != 0 {
            if gather[idx] {
                let text = mc_reg_read(i32::from(MC_REGS[idx]));
                let text_bytes = text.to_bytes();
                if !text_bytes.is_empty() {
                    if !joined[idx].is_empty() {
                        joined[idx].push(b'\n');
                    }
                    joined[idx].extend_from_slice(text_bytes);
                }
            }
            idx += 1;
        }
    }
    ctx_load(&mut save, K_CTX_REGS, 0);
    ctx_free(&mut save);

    let mut idx = 0;
    while MC_REGS[idx] != 0 {
        if gather[idx] && !joined[idx].is_empty() {
            let r = i32::from(MC_REGS[idx]);
            write_reg_contents_ex(
                r,
                &joined[idx],
                joined[idx].len() as isize,
                false,
                crate::register::MotionType::MtLineWise,
                0,
            );
            if r == b'"' as i32 {
                set_clipboard(NUL, get_y_previous());
            }
        }
        idx += 1;
    }
}

pub fn mc_vsel_clear() {
    let vsel_buf = with_mc(|st| st.mc_vsel_buf.replace(0));
    if let Some(buf) = handle_get_buffer_opt(vsel_buf) {
        extmark_clear(&buf, mc_vsel_ns(), 0, 0, MAXLNUM, MAXCOL);
        extmark_clear(&buf, mc_vcur_ns(), 0, 0, MAXLNUM, MAXCOL);
    }
}

fn mc_vsel_mark(start_lnum: Linenr, start_col: Colnr, end_lnum: Linenr, end_col: Colnr) {
    let mut decor = DECOR_INLINE_INIT;
    decor.data.hl_mut().hl_id = syn_check_group(b"MCursorVisual");
    let mut mark: u32 = 0;
    extmark_set(
        &curbuf(),
        mc_vsel_ns(),
        Some(&mut mark),
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
    with_mc(|st| st.mc_vsel_buf.set(curbuf().b_fnum.get()));
}

pub fn mc_vsel_refresh() {
    mc_vsel_clear();
    let Some(span_cstr) = atom_visual_span() else {
        return;
    };
    if span_cstr.as_bytes().is_empty() || !mc_buf_has_cursors(&curbuf()) {
        return;
    }
    let span = NvimString::from_cstr(&span_cstr);

    let mut sb = McSandbox::new();
    mc_sandbox_enter(&mut sb, false);
    block_autocmds();
    block_on_key();
    g_st().emsg_silent.set(g_st().emsg_silent.get() + (1));

    for i in 0..cursors_len() {
        let mut ctx = cursor_clone(i);
        let mut pos = INIT_POS;
        if !mc_ctx_resolve(&ctx, &mut pos) {
            continue;
        }
        ctx.pos = pos;
        with_mc(|st| st.mc_cursors.borrow_mut()[i].pos = pos);
        ctx_load(&mut ctx, K_CTX_CURSOR | K_CTX_VISUAL | K_CTX_MARKS, 0);
        check_cursor(&curwin());
        g_st().visual_global.borrow_mut().active = false;
        g_st().visual_global.borrow_mut().select = false;
        nvim_feedkeys(span.clone(), cstr_as_string(Some(c"nix")), false);
        if !g_st().visual_global.borrow().active {
            continue;
        }
        let mut s = g_st().visual_global.borrow().start;
        let mut e = curwin().w_cursor.get();
        let r_type: i32 = if g_st().visual_global.borrow().mode == b'V' as i32 {
            MT_LINE_WISE
        } else if g_st().visual_global.borrow().mode == CTRL_V_CONST {
            MT_BLOCK_WISE
        } else {
            MT_CHAR_WISE
        };
        let mut inclusive = true;
        let mut oa = Oparg::new();
        g_st().virtual_op.set(i32::from(virtual_active(&curwin())));
        getregionpos_prep(
            &mut s,
            &mut e,
            r_type,
            opt_vars().p_sel.first_byte() == b'e',
            0,
            &mut inclusive,
            &mut oa,
        );
        if r_type == MT_LINE_WISE {
            mc_vsel_mark(s.lnum, 0, e.lnum, ml_get_len(e.lnum));
        } else if r_type == MT_BLOCK_WISE {
            if curwin().w_curswant.get() == MAXCOL {
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
            &curbuf(),
            mc_vcur_ns(),
            &mut cmark,
            curwin().w_cursor.get(),
            false,
            true,
            true,
        );
        with_mc(|st| st.mc_vsel_buf.set(curbuf().b_fnum.get()));
        g_st().visual_global.borrow_mut().active = false;
    }

    g_st().emsg_silent.set(g_st().emsg_silent.get() - (1));
    unblock_on_key();
    unblock_autocmds();
    mc_sandbox_leave(&mut sb);
}

pub fn mc_undo_time() {
    atom_did_global_op();
    extmark_clear(&curbuf(), mc_ns(), 0, 0, MAXLNUM, MAXCOL);
}

pub fn mc_ins_commit() -> bool {
    let (active, ins_cascaded) = with_mc(|st| {
        let active = st.mc_ins_span.active.replace(false);
        let ins_cascaded = active && !st.mc_ins_span.first.get();
        *st.mc_ins_span.vsel.borrow_mut() = None;
        (active, ins_cascaded)
    });

    if ins_cascaded && let Some(ins_cstr) = redo_keys(None) {
        let ins = NvimString::from_cstr(&ins_cstr);
        if ins.size > with_mc(|st| st.mc_ins_span.done_len.get())
            && ins.as_bytes().last() == Some(&ESC)
        {
            mc_ins_span_flush(&ins, true);
        }
    }

    let region = with_mc(|st| st.mc_ins_span.region.replace(0));
    if region != 0 {
        extmark_del_id(&curbuf(), mc_session_ns(), region);
    }
    mc_ins_regions_clear();

    let origin = with_mc(|st| st.mc_ins_span.origin.get());
    if active && buf_get_changedtick(&curbuf()) == origin.tick {
        u_forget_unchanged(&curbuf());
    }

    if !ins_cascaded {
        return false;
    }
    curbuf()
        .b_last_changedtick
        .set(buf_get_changedtick(&curbuf()));
    curbuf()
        .b_last_changedtick_i
        .set(buf_get_changedtick(&curbuf()));
    for i in 0..cursors_len() {
        let ctx = cursor_clone(i);
        let mut pos = INIT_POS;
        if !mc_ctx_resolve(&ctx, &mut pos) {
            continue;
        }
        let mut mark = ctx.mark;
        if mark == origin.mcursor {
            pos = curwin().w_cursor.get();
        } else if pos.col > 0 {
            dec(&mut pos);
        }
        mc_mark_upd(&curbuf(), &mut mark, pos);
        with_mc(|st| {
            let mut cursors = st.mc_cursors.borrow_mut();
            cursors[i].pos = pos;
            cursors[i].mark = mark;
        });
    }
    true
}

pub fn mc_buf_has_cursors(buf: &FileBuf) -> bool {
    with_mc(|st| {
        st.mc_cursors
            .borrow()
            .iter()
            .any(|ctx| ctx.buf == buf.b_fnum.get())
    })
}

pub fn mc_following() -> bool {
    curbuf().b_p_follow.get()
}

fn mc_lua_enable(enable: bool) {
    if g_st().exiting.get() {
        return;
    }
    let mut tv_args = [
        TypVal {
            v_type: VarType::VarBool,
            v_lock: 0,
            vval: TypvalVval::Bool(if enable {
                K_BOOL_VAR_TRUE_CONST
            } else {
                K_BOOL_VAR_FALSE_CONST
            }),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    nlua_call_typval(c"vim._core.mcursor", c"enable", &mut tv_args, None);
}

pub fn mc_mark_at(buf: &FileBuf, pos: CursorPos, skip: u32) -> u32 {
    if cursors_is_empty() {
        return 0;
    }
    let mut itr: MarkTreeIter = MarkTreeIter::new();
    marktree_itr_get(&buf.b_marktree.borrow(), pos.lnum - 1, pos.col, &mut itr);
    loop {
        let k = marktree_itr_current(&buf.b_marktree.borrow(), &itr);
        if k.id == 0 || k.pos.row != pos.lnum - 1 || k.pos.col != pos.col {
            break;
        }
        if k.ns == mc_ns() && !mt_end(k) && k.id != skip {
            return k.id;
        }
        if !marktree_itr_next(&buf.b_marktree.borrow(), &mut itr) {
            break;
        }
    }
    0
}

fn mc_zq_step(keys: &CStr, pos: CursorPos) -> CursorPos {
    curwin().w_cursor.set(pos);
    curwin().w_set_curswant.set(false);
    g_st().msg_silent.set(g_st().msg_silent.get() + (1));
    g_st().emsg_silent.set(g_st().emsg_silent.get() + (1));
    let save_cmod_flags = g_st().cmdmod.borrow().cmod_flags;
    g_st().cmdmod.borrow_mut().cmod_flags |= CMOD_KEEPJUMPS;
    atom_suppress(true);
    block_autocmds();
    block_on_key();
    let mut sst = SaveState::new();
    save_current_state(&mut sst);
    let beeps = g_st().did_beep_silent.get();
    exec_normal_cmd(keys, REMAP_NONE, true);
    let failed = g_st().did_beep_silent.get() != beeps;
    restore_current_state(&mut sst);
    unblock_on_key();
    unblock_autocmds();
    atom_suppress(false);
    g_st().cmdmod.borrow_mut().cmod_flags = save_cmod_flags;
    g_st().emsg_silent.set(g_st().emsg_silent.get() - (1));
    g_st().msg_silent.set(g_st().msg_silent.get() - (1));
    if failed {
        INIT_POS
    } else {
        curwin().w_cursor.get()
    }
}

fn mc_zq_target(
    step: CursorPos,
    search: bool,
    motion_type: i32,
    ctx_idx: Option<usize>,
) -> Visualinfo {
    let save = *curbuf().b_visual.borrow();
    let save_mode_eval = curbuf().b_visual_mode_eval.get();
    if !search {
        *curbuf().b_visual.borrow_mut() = Visualinfo {
            vi_start: step,
            vi_end: step,
            vi_curswant: step.col,
            vi_mode: if motion_type == MT_LINE_WISE {
                b'V' as i32
            } else {
                b'v' as i32
            },
        };
    } else if mc_zq_step(c"gn\x1b", step).lnum == 0 {
        *curbuf().b_visual.borrow_mut() = Visualinfo {
            vi_start: INIT_POS,
            vi_end: INIT_POS,
            vi_mode: 0,
            vi_curswant: 0,
        };
    }
    curbuf()
        .b_visual_mode_eval
        .set(curbuf().b_visual.borrow().vi_mode);
    if let Some(idx) = ctx_idx {
        curwin().w_cursor.set(step);
        with_mc(|st| ctx_save(&mut st.mc_cursors.borrow_mut()[idx], K_CTX_VISUAL));
    }
    let target = *curbuf().b_visual.borrow();
    *curbuf().b_visual.borrow_mut() = save;
    curbuf().b_visual_mode_eval.set(save_mode_eval);
    target
}

fn mc_zq_first(
    keys: &CStr,
    search: bool,
    motion_type: i32,
    bounds: &[CursorPos; 2],
    step: &mut CursorPos,
) -> bool {
    let start = bounds[0];
    let end_bound = bounds[1];
    let curswant = curwin().w_curswant.get();
    let mut before = start;
    let bof = dec(&mut before) == -1;
    step.lnum = 0;
    if !bof {
        let save_ww = opt_vars().p_ww.replace(OptString::from_cstr(c"s,l,>"));
        let s = mc_zq_step(keys, before);
        opt_vars().p_ww.set(save_ww);
        curwin().w_curswant.set(curswant);
        if ltoreq(start, s) && ltoreq(s, end_bound) {
            *step = s;
        }
    }
    curwin().w_cursor.set(start);
    check_cursor_col(&curwin());
    let from = curwin().w_cursor.get();
    let s = mc_zq_step(keys, from);
    if s.lnum > from.lnum && !curwin().w_set_curswant.get() {
        let mut c = from;
        getvpos(&curwin(), &mut c, curwin().w_curswant.get());
        if ltoreq(start, c) && ltoreq(c, end_bound) && (step.lnum == 0 || lt(c, *step)) {
            *step = c;
        }
    }
    curwin().w_curswant.set(curswant);
    if step.lnum != 0 {
        return true;
    }
    let failed = s.lnum == 0;
    if failed && motion_type != MT_LINE_WISE && equalpos(from, start) {
        return false;
    }
    if bof && (!search || equalpos(mc_zq_target(start, true, motion_type, None).vi_start, start)) {
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

fn mc_zq_run(keys: &CStr, search: bool, motion_type: i32, end: CursorPos, first: CursorPos) -> i32 {
    let wrap = end.lnum == MAXLNUM;
    let mut wrapped = false;
    let mut placed: i32 = 0;
    let mut step = first;
    let curswant = curwin().w_curswant.get();
    while !g_st().got_int.get() {
        mc_zq_target(step, search, motion_type, mc_add(&curbuf(), step));
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
    curwin().w_curswant.set(curswant);
    placed
}

pub fn mc_zq_start(cap: &mut Cmdarg) {
    cap.oap.from_visual = g_st().visual_global.borrow().active;
    cap.oap.motion_type = MT_UNKNOWN;
    if g_st().visual_global.borrow().active {
        atom_visual_reset();
        end_visual_mode();
        check_cursor_col(&curwin());
    }
}

pub fn mc_zq(cap: &Cmdarg, origin: CursorPos) {
    let oap = &cap.oap;
    atom_did_global_op();
    if mc_replaying()
        || oap.motion_type == MT_UNKNOWN
        || cap.cmdchar == b'i' as i32
        || cap.cmdchar == b'a' as i32
        || cap.cmdchar == K_LUA
        || cap.cmdchar == K_COMMAND
        || cap.cmdchar == b':' as i32
    {
        beep_flush();
        curwin().w_cursor.set(origin);
        return;
    }
    let mut primary = origin;

    let mut v = *curbuf().b_visual.borrow();
    let mut bounds = [
        primary,
        CursorPos {
            lnum: MAXLNUM,
            col: 0,
            coladd: 0,
        },
    ];
    if oap.from_visual {
        bounds[0] = mark_get_visual(&curbuf(), b'<' as i32).unwrap().mark;
        bounds[1] = mark_get_visual(&curbuf(), b'>' as i32).unwrap().mark;
    }

    let lines = oap.motion_type == MT_LINE_WISE;
    if lines {
        bounds[0].col = 0;
        bounds[1].col = MAXCOL;
    }
    let mut vcols: [Colnr; 2] = [0, MAXCOL];
    let blockwise = oap.from_visual && v.vi_mode == CTRL_V_CONST && !lines;
    if blockwise {
        let [vcol0, vcol1] = &mut vcols;
        getvcols(&curwin(), &mut v.vi_start, &mut v.vi_end, vcol0, vcol1, 0);
        vcols[1] = if v.vi_curswant == MAXCOL {
            MAXCOL
        } else {
            vcols[1]
        };
    }
    curwin().w_cursor.set(primary);
    update_curswant();
    let mut view = Viewstate::new();
    save_viewstate(&curwin(), &mut view);
    save_search_patterns();

    let search = (cap.cmdchar != NUL && b"*#/?nN".contains(&(cap.cmdchar as u8)))
        || (cap.cmdchar == b'g' as i32 && cap.nchar != NUL && b"*#nN".contains(&(cap.nchar as u8)));
    let mut placed: i32 = 0;
    let mut first = INIT_POS;
    let mut spec = atom_cmd_spec(cap);
    spec.regname = 0;
    let keys_cstr = if search {
        c"/\n".to_owned()
    } else {
        redo_keys(Some(&spec)).unwrap_or_default()
    };
    if !oap.from_visual && search {
        first = mc_zq_target(primary, true, oap.motion_type, None).vi_start;
        if first.lnum != 0 {
            placed = mc_zq_run(&keys_cstr, search, oap.motion_type, bounds[1], first);
        }
    } else if !blockwise {
        if mc_zq_first(&keys_cstr, search, oap.motion_type, &bounds, &mut first) {
            placed = mc_zq_run(&keys_cstr, search, oap.motion_type, bounds[1], first);
        }
    } else {
        for lnum in bounds[0].lnum..=bounds[1].lnum {
            let mut line = [
                CursorPos {
                    lnum,
                    col: 0,
                    coladd: 0,
                },
                CursorPos {
                    lnum,
                    col: MAXCOL,
                    coladd: 0,
                },
            ];
            if !getvpos(&curwin(), &mut line[0], vcols[0]) {
                line[0].col = ml_get_len(lnum);
            } else if vcols[1] != MAXCOL {
                getvpos(&curwin(), &mut line[1], vcols[1]);
            }
            let mut step = INIT_POS;
            if mc_zq_first(&keys_cstr, search, oap.motion_type, &line, &mut step) {
                if first.lnum == 0 {
                    first = step;
                }
                placed += mc_zq_run(&keys_cstr, search, oap.motion_type, line[1], step);
            }
        }
    }

    drop(keys_cstr);
    restore_search_patterns();
    restore_viewstate(&curwin(), &view);

    if placed > 0 {
        if mc_mark_at(&curbuf(), primary, 0) == 0 {
            primary = first;
            curwin().w_set_curswant.set(true);
        }
        *curbuf().b_visual.borrow_mut() = mc_zq_target(primary, search, oap.motion_type, None);
        curbuf()
            .b_visual_mode_eval
            .set(curbuf().b_visual.borrow().vi_mode);
        set_no_hlsearch(true);
        redraw_all_later(UPD_SOME_VALID);
    }
    curwin().w_cursor.set(primary);
}

pub fn mc_counter(count1: i64) {
    atom_did_global_op();
    let mut tv_args = [
        TypVal {
            v_type: VarType::VarNumber,
            v_lock: 0,
            vval: TypvalVval::Number(count1 as Varnumber),
        },
        TypVal {
            v_type: VarType::VarUnknown,
            v_lock: 0,
            vval: TypvalVval::Number(0),
        },
    ];
    nlua_call_typval(c"vim._core.mcursor", c"number", &mut tv_args, None);
}

pub fn mc_follow_set(on: TriState) {
    if mc_replaying() {
        return;
    }
    let follow = if on == K_NONE {
        !curbuf().b_p_follow.get()
    } else {
        on == K_TRUE
    };
    if (curbuf().b_p_follow.get()) != follow {
        set_option_value_give_err(K_OPT_FOLLOW, boolean_obj(follow), OPT_LOCAL);
    }
}

pub fn mc_toggle(buf: &FileBuf, pos: CursorPos, end_follow: bool, on: TriState) {
    if mc_replaying() {
        return;
    }
    if end_follow {
        mc_follow_set(K_FALSE);
    }
    let mark = mc_mark_at(buf, pos, 0);
    if mark != 0 && on != K_TRUE {
        extmark_del_id(buf, mc_ns(), mark);
        mc_cleanup(false, None, 0);
    } else if mark == 0 && on != K_FALSE {
        mc_add(buf, pos);
    }
}

pub fn mc_add(buf: &FileBuf, pos: CursorPos) -> Option<usize> {
    let existing = mc_mark_at(buf, pos, 0);
    if existing != 0 {
        let found = with_mc(|st| {
            st.mc_cursors
                .borrow()
                .iter()
                .position(|ctx| ctx.buf == buf.b_fnum.get() && ctx.mark == existing)
        });
        if found.is_some() {
            return found;
        }
    }
    let count = cursors_len();
    let mut mark: u32 = 0;
    mc_mark_upd(buf, &mut mark, pos);
    let new_len = cursors_len();
    if new_len > count {
        Some(new_len - 1)
    } else {
        None
    }
}

pub fn mc_on_extmark_set(buf: &FileBuf, ns_id: u32, mut id: u32, pos: CursorPos) {
    if ns_id != mc_ns() {
        return;
    }
    if mc_replaying() {
        extmark_del_id(buf, ns_id, id);
        return;
    }
    if mc_mark_at(buf, pos, id) != 0 {
        extmark_del_id(buf, ns_id, id);
        return;
    }
    mc_mark_upd(buf, &mut id, pos);
    atom_visual_reset();
    if cursors_is_empty() {
        with_mc(|st| {
            st.mc_start.time.set(os_realtime() as Timestamp);
            ctx_save(&mut st.mc_start.ctx.borrow_mut(), K_CTX_REGS);
        });
        mc_lua_enable(true);
    }
    let mut ctx = CONTEXT_INIT;
    ctx.buf = buf.b_fnum.get();
    ctx.pos = pos;
    ctx.mark = id;
    with_mc(|st| st.mc_cursors.borrow_mut().push(ctx));
}

pub fn mc_buf_clear(buf: &FileBuf) {
    if mc_replaying() {
        return;
    }
    extmark_clear(buf, mc_ns(), 0, 0, MAXLNUM, MAXCOL);
    extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
}

pub fn mc_buf_free(buf: &FileBuf) {
    if mc_replaying() {
        return;
    }
    mc_cleanup(false, None, 0);
    with_mc(|st| {
        if st.mc_vsel_buf.get() == buf.b_fnum.get() {
            st.mc_vsel_buf.set(0);
        }
    });
}

pub fn mc_ns_clearing(buf: &FileBuf, ns_id: u32) {
    if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() {
        return;
    }
    for i in 0..cursors_len() {
        let (ctx_buf, ctx_mark, mut pos) = with_mc(|st| {
            let ctx = &st.mc_cursors.borrow()[i];
            (ctx.buf, ctx.mark, ctx.pos)
        });
        if ctx_buf == buf.b_fnum.get() && ctx_mark != 0 {
            extmark_get_pos(buf, mc_ns(), ctx_mark, &mut pos);
            with_mc(|st| st.mc_cursors.borrow_mut()[i].pos = pos);
        }
    }
}

pub fn mc_ns_cleared(buf: &FileBuf, ns_id: u32) {
    if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() || !mc_buf_has_cursors(buf) {
        return;
    }

    let mut others = false;
    for idx in 0..cursors_len() {
        let (ctx_buf, ctx_mark) = with_mc(|st| {
            let ctx = &st.mc_cursors.borrow()[idx];
            (ctx.buf, ctx.mark)
        });
        let mut pos = INIT_POS;
        if ctx_buf != buf.b_fnum.get()
            || (ctx_mark != 0 && extmark_get_pos(buf, mc_ns(), ctx_mark, &mut pos))
        {
            others = true;
            break;
        }
    }
    if !others {
        mc_reg_gather();
    }

    if !g_st().exiting.get() {
        extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
        if g_st().curbuf.get() == (std::ptr::from_ref(buf).cast_mut()) {
            let mut primary: u32 = 1;
            extmark_set_pos(
                buf,
                mc_last_ns(),
                &mut primary,
                curwin().w_cursor.get(),
                true,
                false,
                false,
            );
        }
        for idx in 0..cursors_len() {
            let (ctx_buf, ctx_pos) = with_mc(|st| {
                let ctx = &st.mc_cursors.borrow()[idx];
                (ctx.buf, ctx.pos)
            });
            if ctx_buf != buf.b_fnum.get() {
                continue;
            }
            let mut mark: u32 = 0;
            extmark_set_pos(buf, mc_last_ns(), &mut mark, ctx_pos, true, false, false);
        }
    }
    mc_cleanup(false, None, 0);
    if cursors_is_empty() {
        with_atom(|st| *st.g_atoms.borrow_mut() = Vec::new());
    }
}

pub fn mc_free_all() {
    while let Some(mut ctx) = with_mc(|st| st.mc_cursors.borrow_mut().pop()) {
        ctx_free(&mut ctx);
    }
    with_mc(|st| {
        *st.mc_cursors.borrow_mut() = Vec::new();
        *st.mc_ins_span.regions.borrow_mut() = Vec::new();
        ctx_free(&mut st.mc_start.ctx.borrow_mut());
    });
}
