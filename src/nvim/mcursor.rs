//! Multicursor tracking, preview, and cascade replay (`src/nvim/mcursor.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use crate::api::buffer::{LUA_INTERNAL_CALL, nvim_buf_set_text};
use crate::api::extmark::nvim_create_namespace;
use crate::api::private::helpers::{
    api_clear_error, api_free_string, boolean_obj, cstr_as_string, cstr_to_string, string_to_array,
};
use crate::api::private::validate::{Error, ErrorType};
use crate::api::vim::{REMAP_NONE, kCtxRegs, kFalse, kNone, kTrue, nvim_feedkeys};
use crate::autocmd::{block_autocmds, unblock_autocmds};
use crate::buffer::{MAXLNUM, buf_T, buf_get_changedtick, bufref_valid};
use crate::charset::to_special;
use crate::clipboard::set_clipboard;
use crate::context::{
    CONTEXT_INIT, Context, INIT_POS, ctx_free, ctx_load, ctx_save, handle_get_buffer, kCtxCursor,
    kCtxMarks, kCtxMergeReg, kCtxVisual,
};
use crate::cursor::{
    check_cursor, check_cursor_col, check_pos, colnr_T, equalpos, getvpos, linenr_T, lt, ltoreq,
    pos_T,
};
use crate::decoration::DecorInline;
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::eval::buffer::varnumber_T;
use crate::eval::typval::{TypvalVval, VarType, typval_T};
use crate::eval::vars::{VV_CASCADING, get_vim_var_tv, set_vim_var_bool};
use crate::eval::window::viewstate_T;
use crate::ex_docmd::{
    exec_normal_cmd, restore_current_state, save_current_state, set_no_hlsearch,
};
use crate::extmark::{
    TriState, extmark_clear, extmark_del_id, extmark_from_id, extmark_get_pos, extmark_set,
    extmark_set_pos,
};
use crate::globals::*;
use crate::highlight_group::syn_check_group;
use crate::input::{
    RedoState, beep_flush, block_on_key, redo_keys, restore_redobuff, save_redobuff, unblock_on_key,
};
use crate::input_cmdatom::{
    CmdAtom, CmdOrigin, atom_cmd_spec, atom_composite_active, atom_composite_queued,
    atom_did_global_op, atom_key_class, atom_lhs_replay_queue, atom_push_raw, atom_suppress,
    atom_visual_reset, atom_visual_span, atoms_free, cmdarg_T, g_atoms, kAInsertSpan, kAMotion,
    kAVisualSpan, kKeyInsFlush, sb_concat,
};
use crate::insexpand::ins_compl_active;
use crate::keycodes::{K_COMMAND, K_SPECIAL};
use crate::kvec::{KVec, StringBuilder};
use crate::log::{LOGLVL_DBG, g_min_log_level, logmsg};
use crate::lua::executor::nlua_call_typval;
use crate::map_glyph_cache::NvimString;
use crate::mark::mark_get_visual;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DecorInlineData, MT_FLAG_DECOR_HL, MTPair, MarkTreeIter,
    marktree_itr_current, marktree_itr_get, marktree_itr_next, mt_end,
};
use crate::memline::{dec, inc, ml_get_len, ml_region_text};
use crate::memory::{Arena, arena_finish, arena_mem_free, xfree, xmemdupz, xstrdup};
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
    get_reg_contents, get_unname_register, get_y_previous, get_y_register, op_reg_index,
    reg_max_ts, write_reg_contents_ex,
};
use crate::search::{restore_search_patterns, save_search_patterns};
use crate::shada::shada_encode_regs;
use crate::state::{MODE_INSERT, virtual_active};
use crate::strings::{memcmp, strchr, strlen, strncmp, vim_snprintf};
use crate::textformat::{InsState, MotionType, oparg_T};
use crate::undo::{u_forget_unchanged, u_header_T, u_save};
use std::ffi::{c_char, c_int, c_long};
use std::ptr;

const DECOR_INLINE_INIT: DecorInline = DecorInline {
    ext: false,
    data: DecorInlineData {
        hl: DECOR_HIGHLIGHT_INLINE_INIT,
    },
};

const NUL: c_int = 0;
const NL: c_int = 10;
const ESC: c_char = 27;

pub(crate) const kOptFollow: c_int = 114;

const MC_REGS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789\"-\0";

use crate::api::command::CMOD_KEEPJUMPS;
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

#[repr(C)]
struct McInsSaved {
    ins: InsState,
    last_changedtick: varnumber_T,
    last_changedtick_i: varnumber_T,
}

struct McInsSpan {
    origin: CmdOrigin,
    regions: KVec<u32>,
    active: bool,
    first: bool,
    frame: u64,
    done_len: usize,
    region: u32,
    vsel: *const c_char,
}

struct McStart {
    time: Timestamp,
    ctx: Context,
}

static mut mc_ins_span: McInsSpan = McInsSpan {
    origin: CmdOrigin {
        buf: crate::buffer::bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        },
        win: ptr::null(),
        pos: INIT_POS,
        mcursor: 0,
        tick: 0,
        maptick: 0,
    },
    regions: KVec::new(),
    active: false,
    first: false,
    frame: 0,
    done_len: 0,
    region: 0,
    vsel: ptr::null(),
};

static mut mc_start: McStart = McStart {
    time: 0,
    ctx: CONTEXT_INIT,
};

static mut mc_vsel_buf: handle_T = 0;
static mut mc_ins_joined: bool = false;
static mut mc_cursors: KVec<Context> = KVec::new();

static mut ns_mc: u32 = 0;
static mut ns_vcur: u32 = 0;
static mut ns_vsel: u32 = 0;
static mut ns_last: u32 = 0;
static mut ns_session: u32 = 0;

#[inline]
const fn static_cstr_as_string(s: &'static [u8]) -> NvimString {
    NvimString {
        data: s.as_ptr().cast_mut().cast(),
        size: s.len() - 1,
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_init() {
    unsafe {
        mc_ns();
    }
}

unsafe fn mc_ns() -> u32 {
    unsafe {
        if ns_mc == 0 {
            ns_mc = nvim_create_namespace(static_cstr_as_string(b"nvim.multicursor\0")) as u32;
        }
        ns_mc
    }
}

unsafe fn mc_vcur_ns() -> u32 {
    unsafe {
        if ns_vcur == 0 {
            ns_vcur =
                nvim_create_namespace(static_cstr_as_string(b"nvim.multicursor.cursor\0")) as u32;
        }
        ns_vcur
    }
}

unsafe fn mc_vsel_ns() -> u32 {
    unsafe {
        if ns_vsel == 0 {
            ns_vsel =
                nvim_create_namespace(static_cstr_as_string(b"nvim.multicursor.visual\0")) as u32;
        }
        ns_vsel
    }
}

unsafe fn mc_last_ns() -> u32 {
    unsafe {
        if ns_last == 0 {
            ns_last =
                nvim_create_namespace(static_cstr_as_string(b"nvim.multicursor.last\0")) as u32;
        }
        ns_last
    }
}

unsafe fn mc_session_ns() -> u32 {
    unsafe {
        if ns_session == 0 {
            ns_session =
                nvim_create_namespace(static_cstr_as_string(b"nvim.multicursor._session\0")) as u32;
        }
        ns_session
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_count() -> usize {
    unsafe { mc_cursors.size }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_showcmd(buf: *mut c_char, size: usize) -> usize {
    unsafe {
        if mc_cursors.size == 0 {
            *buf = 0;
            return 0;
        }
        let prefix = if mc_following() {
            c"=".as_ptr()
        } else {
            c"".as_ptr()
        };
        vim_snprintf!(
            buf,
            size,
            c"%s%zu\xc3\x97 ".as_ptr(),
            prefix,
            mc_cursors.size,
        ) as usize
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_replaying() -> bool {
    unsafe { (*get_vim_var_tv(VV_CASCADING)).vval.v_bool == kBoolVarTrue }
}

unsafe fn mc_mark_upd(buf: *mut buf_T, mark: *mut u32, pos: pos_T) {
    unsafe {
        extmark_set_pos(buf, mc_ns(), mark, pos, true, false, true);
    }
}

unsafe fn mc_track_upd(buf: *mut buf_T, mark: *mut u32, pos: pos_T) {
    unsafe {
        extmark_set_pos(buf, mc_session_ns(), mark, pos, true, true, false);
    }
}

unsafe fn mc_sandbox_enter(sb: *mut McSandbox, save_regs: bool) {
    unsafe {
        assert!(!mc_replaying());
        set_vim_var_bool(VV_CASCADING, kBoolVarTrue);
        (*sb).primary = CONTEXT_INIT;
        ctx_save(
            &mut (*sb).primary,
            kCtxCursor | kCtxVisual | kCtxMarks | if save_regs { kCtxRegs } else { 0 },
        );
        (*sb).cursor_mark = 0;
        mc_track_upd(curbuf, &mut (*sb).cursor_mark, (*sb).primary.pos);
        save_viewstate(curwin, &mut (*sb).view);
        (*sb).topline_mark = 0;
        mc_track_upd(
            curbuf,
            &mut (*sb).topline_mark,
            pos_T {
                lnum: (*sb).view.vs_topline,
                col: 0,
                coladd: 0,
            },
        );
        (*sb).visual = Visual;
        save_current_state(&mut (*sb).sst);
        save_search_patterns();
        save_redobuff(&mut (*sb).redo);
    }
}

unsafe fn mc_sandbox_leave(sb: *mut McSandbox) {
    unsafe {
        if !(*sb).primary.regs.data.is_null() {
            ctx_load(&mut (*sb).primary, kCtxRegs, 0);
        }
        restore_redobuff(&mut (*sb).redo);
        restore_search_patterns();
        Visual = (*sb).visual;
        restore_current_state(&mut (*sb).sst);
        let buf = handle_get_buffer((*sb).primary.buf);
        let mut topline = INIT_POS;
        let mut topline_valid = false;
        if !buf.is_null() {
            extmark_get_pos(
                buf,
                mc_session_ns(),
                (*sb).cursor_mark,
                &mut (*sb).primary.pos,
            );
            extmark_del_id(buf, mc_session_ns(), (*sb).cursor_mark);
            topline_valid = extmark_get_pos(buf, mc_session_ns(), (*sb).topline_mark, &mut topline);
            extmark_del_id(buf, mc_session_ns(), (*sb).topline_mark);
        }
        if buf == curbuf {
            restore_viewstate(curwin, &(*sb).view);
            ctx_load(&mut (*sb).primary, kCtxCursor | kCtxVisual | kCtxMarks, 0);
            check_cursor(curwin);
            if topline_valid {
                set_topline(curwin, topline.lnum.min((*curbuf).b_ml.ml_line_count));
                update_topline(curwin);
            }
        }
        ctx_free(&mut (*sb).primary);
        set_vim_var_bool(VV_CASCADING, kBoolVarFalse);
    }
}

unsafe fn mc_execute(cursoridx: usize, atomidx: usize) {
    unsafe {
        let mut ctx = *mc_cursors.items.add(cursoridx);
        let atom = *g_atoms.items.add(atomidx);

        if handle_get_buffer(ctx.buf) != curbuf {
            return;
        }

        if ctx.mark != 0 && !extmark_get_pos(curbuf, mc_ns(), ctx.mark, &mut ctx.pos) {
            return;
        }
        if ctx.visual.vi_start.lnum == 0 && strncmp(atom.keys, c"gv".as_ptr(), 2) == 0 {
            return;
        }

        let swap_regs = atom.r#type != kAMotion;
        let mut regs_ts: Timestamp = 0;
        if swap_regs {
            if reg_max_ts(false) >= mc_start.time {
                ctx_load(&raw mut mc_start.ctx, kCtxRegs, 0);
            }
            if !ctx.regs.data.is_null() {
                ctx_load(&mut ctx, kCtxRegs, kCtxMergeReg);
            }
            regs_ts = reg_max_ts(false);
        }

        let mark_cmd = if atom.spec.cmd == b'g' as c_int {
            atom.spec.cmd2
        } else {
            atom.spec.cmd
        };
        let swap_marks = atom.r#type != kAMotion
            || atom.remap
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

        if atom.r#type == kAInsertSpan {
            check_pos(curbuf, &mut (*curwin).w_cursor);
        } else {
            check_cursor(curwin);
        }

        if atom.r#type == kAVisualSpan {
            Visual.active = false;
            Visual.select = false;
        }

        if atom.r#type == kAInsertSpan {
            State = MODE_INSERT;
        }

        let beeps = did_beep;
        let tick = buf_get_changedtick(curbuf);
        let mode_str = if atom.remap {
            c"ix".as_ptr()
        } else {
            c"nix".as_ptr()
        };
        nvim_feedkeys(cstr_as_string(atom.keys), cstr_as_string(mode_str), false);

        if atom.r#type == kAInsertSpan
            && mc_ins_span.first
            && did_beep != beeps
            && buf_get_changedtick(curbuf) == tick
        {
            extmark_del_id(curbuf, mc_ns(), ctx.mark);
            return;
        }

        if Visual.active {
            if atom.r#type == kAVisualSpan {
                (*curbuf).b_visual = visualinfo();
                (*curbuf).b_visual_mode_eval = Visual.mode;
            }
            Visual.active = false;
            Visual.select = false;
        }

        let wrote_regs = swap_regs && reg_max_ts(false) != regs_ts;

        if cursoridx >= mc_cursors.size {
            return;
        }
        if !extmark_get_pos(curbuf, mc_ns(), ctx.mark, &mut ctx.pos) {
            return;
        }

        if wrote_regs {
            api_free_string(ctx.regs);
            ctx.regs = shada_encode_regs(false, mc_start.time);
        }
        ctx_save(
            &mut ctx,
            if swap_marks {
                kCtxVisual | kCtxMarks | kCtxCursor
            } else {
                kCtxCursor
            },
        );

        if atom.r#type == kAInsertSpan && (*curbuf).b_last_insert.mark.lnum > 0 {
            ctx.pos = (*curbuf).b_last_insert.mark;
        }
        mc_mark_upd(curbuf, &mut ctx.mark, ctx.pos);
        *mc_cursors.items.add(cursoridx) = ctx;
    }
}

unsafe fn mc_cascade() {
    unsafe {
        assert!(g_atoms.size >= 1);
        let origin = &(*g_atoms.items).origin;
        let mcursor = if origin.buf.br_buf == curbuf {
            origin.mcursor
        } else {
            0
        };
        assert!(mc_cursors.size > 0);
        assert!(!mc_replaying());

        got_int = false;

        let mut edits = false;
        for i in 0..g_atoms.size {
            edits |= (*g_atoms.items.add(i)).r#type != kAMotion;
        }
        if edits {
            mc_cleanup(true, ptr::null(), 0);
            if mc_cursors.size == 0 {
                atoms_free(&raw mut g_atoms);
                return;
            }
        }

        let mut mcursor_pos = INIT_POS;
        if mcursor != 0 && extmark_get_pos(curbuf, mc_ns(), mcursor, &mut mcursor_pos) {
            let mut mark = mcursor;
            mc_mark_upd(curbuf, &mut mark, (*curwin).w_cursor);
        }

        let mut sb: McSandbox = std::mem::zeroed();
        mc_sandbox_enter(&mut sb, edits);

        'outer: for ai in 0..g_atoms.size {
            let atom = g_atoms.items.add(ai);
            if !(*atom).origin.buf.br_buf.is_null()
                && (!bufref_valid(&raw mut (*atom).origin.buf)
                    || (*atom).origin.buf.br_buf != curbuf)
            {
                continue;
            }
            let mut ci = 0;
            while ci < mc_cursors.size {
                line_breakcheck();
                if got_int {
                    break 'outer;
                }
                if mcursor == 0 || (*mc_cursors.items.add(ci)).mark != mcursor {
                    mc_execute(ci, ai);
                }
                ci += 1;
            }
        }

        atoms_free(&raw mut g_atoms);
        let bufnr = sb.primary.buf;
        mc_sandbox_leave(&mut sb);

        mc_cleanup(true, &(*curwin).w_cursor, mcursor);
        if handle_get_buffer(bufnr) == curbuf
            && !(*curbuf).b_u_synced
            && !(*curbuf).b_u_newhead.is_null()
        {
            (*(*curbuf).b_u_newhead.cast::<u_header_T>()).uh_cursor_after = (*curwin).w_cursor;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_clock_edge(map_edit: bool, map_moved: bool, follow: bool) {
    unsafe {
        if (map_edit || (follow && map_moved && !Visual.active))
            && !atom_composite_queued()
            && g_atoms.size == 0
            && atom_composite_active()
            && mc_buf_has_cursors(curbuf)
        {
            atom_lhs_replay_queue();
        }
        if mc_buf_has_cursors(curbuf) && g_atoms.size > 0 {
            let mut has_edit = map_edit;
            let mut i = 0;
            while !has_edit && i < g_atoms.size {
                has_edit = (*g_atoms.items.add(i)).r#type != kAMotion;
                i += 1;
            }
            if has_edit || follow || Visual.active {
                mc_cascade();
            } else {
                atoms_free(&raw mut g_atoms);
            }
        }
    }
}

unsafe fn mc_cleanup(dedupe: bool, primary: *const pos_T, keep_mark: u32) {
    unsafe {
        let had_cursors = mc_cursors.size > 0;
        let mut n = 0;
        for i in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(i);
            let buf = handle_get_buffer((*ctx).buf);
            if buf.is_null()
                || ((*ctx).mark != 0
                    && !extmark_get_pos(buf, mc_ns(), (*ctx).mark, &mut (*ctx).pos))
            {
                ctx_free(ctx);
                continue;
            }
            let dup = dedupe
                && (*ctx).mark != 0
                && ((!curwin.is_null()
                    && buf == curbuf
                    && !primary.is_null()
                    && equalpos((*ctx).pos, *primary)
                    && (*ctx).mark != keep_mark)
                    || mc_mark_at(buf, (*ctx).pos, 0) != (*ctx).mark);
            if dup {
                extmark_del_id(buf, mc_ns(), (*ctx).mark);
                ctx_free(ctx);
            } else {
                *mc_cursors.items.add(n) = *ctx;
                n += 1;
            }
        }
        mc_cursors.size = n;
        if n == 0 && had_cursors {
            ctx_free(&raw mut mc_start.ctx);
            mc_start.time = 0;
            mc_lua_enable(false);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_join() {
    unsafe {
        if !mc_replaying() {
            mc_ins_joined = true;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_replay_can_join() -> bool {
    unsafe { !mc_replaying() || mc_ins_joined }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_cascade_start(
    cascade: bool,
    origin: CmdOrigin,
    root_frame: u64,
    vsel: *const c_char,
) {
    unsafe {
        if mc_replaying() {
            return;
        }
        mc_ins_span.vsel = vsel;
        mc_ins_joined = false;
        mc_ins_span.active = cascade && mc_buf_has_cursors(curbuf);
        mc_ins_span.first = true;
        mc_ins_span.frame = root_frame;
        mc_ins_span.done_len = 0;
        mc_ins_span.origin = origin;
        mc_ins_span.region = 0;
        mc_ins_regions_clear();

        if mc_ins_span.active {
            let mut lo = (*curwin).w_cursor.lnum;
            let mut hi = (*curwin).w_cursor.lnum;
            for i in 0..mc_cursors.size {
                let mut pos = INIT_POS;
                if mc_ctx_resolve(mc_cursors.items.add(i), &mut pos) {
                    lo = lo.min(pos.lnum);
                    hi = hi.max(pos.lnum);
                }
            }
            let _ = u_save(lo - 1, hi + 1);
        }
    }
}

unsafe fn mc_ins_save_state() -> McInsSaved {
    unsafe {
        let saved = McInsSaved {
            ins: Ins,
            last_changedtick: (*curbuf).b_last_changedtick,
            last_changedtick_i: (*curbuf).b_last_changedtick_i,
        };
        Ins.did_ai = false;
        Ins.ai_col = 0;
        Ins.end_comment_pending = NUL;
        Ins.did_si = false;
        Ins.can_si = false;
        Ins.can_si_back = false;
        saved
    }
}

unsafe fn mc_ins_restore_state(saved: *const McInsSaved) {
    unsafe {
        Ins = (*saved).ins;
        (*curbuf).b_last_changedtick = (*saved).last_changedtick;
        (*curbuf).b_last_changedtick_i = (*saved).last_changedtick_i;
    }
}

unsafe fn mc_ins_span_push(keys: *mut c_char, text: *mut c_char) {
    unsafe {
        let cascade = mc_buf_has_cursors(curbuf);
        let mut atom = CmdAtom {
            r#type: kAInsertSpan,
            keys,
            text,
            origin: mc_ins_span.origin,
            ..CmdAtom::default()
        };
        atom_push_raw(cascade, &mut atom);
        if cascade {
            let saved = mc_ins_save_state();
            block_autocmds();
            mc_cascade();
            unblock_autocmds();
            mc_ins_restore_state(&saved);
            mc_ins_joined = false;
        }
        mc_ins_span.first = false;
    }
}

unsafe fn mc_ins_regions_clear() {
    unsafe {
        while mc_ins_span.regions.size > 0 {
            mc_ins_span.regions.size -= 1;
            let id = *mc_ins_span.regions.items.add(mc_ins_span.regions.size);
            extmark_del_id(curbuf, mc_session_ns(), id);
        }
    }
}

#[inline]
unsafe fn mc_ctx_resolve(ctx: *const Context, pos: *mut pos_T) -> bool {
    unsafe {
        handle_get_buffer((*ctx).buf) == curbuf
            && (*ctx).mark != 0
            && extmark_get_pos(curbuf, mc_ns(), (*ctx).mark, pos)
    }
}

unsafe fn mc_region_mark_set(mark: *mut u32, pos: pos_T) {
    unsafe {
        extmark_set(
            curbuf,
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
            ptr::null_mut(),
        );
    }
}

unsafe fn mc_ins_preview_rebase() {
    unsafe {
        mc_region_mark_set(&raw mut mc_ins_span.region, (*curwin).w_cursor);
        mc_ins_regions_clear();
        for i in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(i);
            let mut pos = INIT_POS;
            let mut mark = 0;
            if mc_ctx_resolve(ctx, &mut pos) && (*ctx).mark != mc_ins_span.origin.mcursor {
                mc_region_mark_set(&mut mark, pos);
                crate::kvec::kv_push(&raw mut mc_ins_span.regions, mark);
            }
        }
    }
}

unsafe fn mc_ins_region_first() -> MTPair {
    unsafe {
        for i in 0..mc_ins_span.regions.size {
            let p = extmark_from_id(curbuf, mc_session_ns(), *mc_ins_span.regions.items.add(i));
            if p.start.id != 0 {
                return p;
            }
        }
        std::mem::zeroed()
    }
}

#[inline]
unsafe fn mc_region_range(p: MTPair, start: *mut pos_T, end: *mut pos_T) {
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

unsafe fn mc_region_mark_range(mark: u32, start: *mut pos_T, end: *mut pos_T) -> bool {
    unsafe {
        let p = extmark_from_id(curbuf, mc_session_ns(), mark);
        if p.start.id == 0 {
            return false;
        }
        mc_region_range(p, start, end);
        true
    }
}

unsafe fn mc_ins_preview_replace(start: pos_T, end: pos_T, text: *const NvimString) {
    unsafe {
        let mut arena = Arena {
            cur_blk: ptr::null_mut(),
            pos: 0,
            size: 0,
        };
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };
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
            if g_min_log_level <= LOGLVL_DBG {
                logmsg!(
                    LOGLVL_DBG,
                    ptr::null(),
                    c"mc_ins_preview_replace".as_ptr(),
                    747,
                    true,
                    c"preview replace failed: %s".as_ptr(),
                    err.msg,
                );
            }
            api_clear_error(&mut err);
        }
        arena_mem_free(arena_finish(&mut arena));
    }
}

unsafe fn mc_ins_preview_set(new_text: *const NvimString) {
    unsafe {
        let saved = mc_ins_save_state();
        let mut primary: u32 = 0;
        mc_track_upd(curbuf, &mut primary, (*curwin).w_cursor);
        let save_cmod_flags = cmdmod.cmod_flags;
        cmdmod.cmod_flags |= CMOD_KEEPJUMPS;
        set_vim_var_bool(VV_CASCADING, kBoolVarTrue);
        for i in 0..mc_ins_span.regions.size {
            let mut rs = INIT_POS;
            let mut re = INIT_POS;
            if !mc_region_mark_range(*mc_ins_span.regions.items.add(i), &mut rs, &mut re) {
                continue;
            }
            mc_ins_preview_replace(rs, re, new_text);
        }
        set_vim_var_bool(VV_CASCADING, kBoolVarFalse);
        cmdmod.cmod_flags = save_cmod_flags;
        let mut pos = (*curwin).w_cursor;
        if extmark_get_pos(curbuf, mc_session_ns(), primary, &mut pos) {
            (*curwin).w_cursor = pos;
        }
        extmark_del_id(curbuf, mc_session_ns(), primary);
        mc_ins_restore_state(&saved);
    }
}

unsafe fn mc_ins_keys_nonliteral(keys: *const c_char, len: usize) -> bool {
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

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_cascade_restart() {
    unsafe {
        if !mc_ins_span.active
            || mc_replaying()
            || (State & MODE_INSERT) == 0
            || !mc_buf_has_cursors(curbuf)
            || g_atoms.size != 0
        {
            return;
        }
        let ins = redo_keys(ptr::null());
        mc_ins_span.done_len = ins.size;
        api_free_string(ins);
        mc_ins_preview_rebase();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_replaying() -> bool {
    unsafe { mc_replaying() && mc_ins_cascading() }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_cascading() -> bool {
    unsafe { mc_ins_span.active }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_cascaded(root_frame: u64) -> bool {
    unsafe { !mc_ins_span.first && mc_ins_span.frame == root_frame }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_cascade() {
    unsafe {
        if !mc_ins_span.active
            || mc_replaying()
            || (State & MODE_INSERT) == 0
            || !mc_buf_has_cursors(curbuf)
        {
            return;
        }

        let mut start = INIT_POS;
        let mut end = INIT_POS;
        let ins = redo_keys(ptr::null());
        assert!(!ins.data.is_null() || ins.size == 0);

        if mc_ins_span.first {
            if !Ins.did_ai && !ins.data.is_null() && ins.size > 0 {
                let mut keys: StringBuilder = KVec::new();
                let mut skip = 0;
                if !mc_ins_span.vsel.is_null() {
                    assert!(ins.size >= 2 && strncmp(ins.data, c"1v".as_ptr(), 2) == 0);
                    sb_concat(&mut keys, mc_ins_span.vsel);
                    skip = 2;
                    mc_ins_span.vsel = ptr::null();
                }
                keys.concat_len(ins.data.add(skip), ins.size - skip);
                keys.push(ESC);
                keys.push(0);
                mc_ins_span.done_len = ins.size;
                mc_ins_span_push(keys.items, ptr::null_mut());
                mc_ins_preview_rebase();
            }
        } else if !ins.data.is_null() && ins.size < mc_ins_span.done_len {
            mc_ins_cascade_restart();
        } else if ins.size > mc_ins_span.done_len
            && mc_ins_keys_nonliteral(
                ins.data.add(mc_ins_span.done_len),
                ins.size - mc_ins_span.done_len,
            )
        {
            if !ins_compl_active() && !pum_visible() {
                mc_ins_span_flush(&ins, false);
            }
        } else if mc_region_mark_range(mc_ins_span.region, &mut start, &mut end) {
            let text = ml_region_text(curbuf, start, end);
            let mut update = true;
            let fp = mc_ins_region_first();
            if fp.start.id != 0 {
                let mut frs = INIT_POS;
                let mut fre = INIT_POS;
                mc_region_range(fp, &mut frs, &mut fre);
                let cur = ml_region_text(curbuf, frs, fre);
                update = cur.size != text.size
                    || (text.size > 0 && memcmp(cur.data.cast(), text.data.cast(), text.size) != 0);
                api_free_string(cur);
            }
            if update && (ins_compl_active() || pum_visible()) {
                for i in 0..mc_ins_span.regions.size {
                    let mut rs = INIT_POS;
                    let mut re = INIT_POS;
                    if !mc_region_mark_range(*mc_ins_span.regions.items.add(i), &mut rs, &mut re) {
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
                mc_ins_preview_set(&text);
            }
            api_free_string(text);
        }
        api_free_string(ins);
    }
}

unsafe fn mc_ins_span_flush(ins: *const NvimString, commit: bool) {
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
        let dlen = (*ins).size - mc_ins_span.done_len;
        let mut keys: StringBuilder = KVec::new();
        keys.push(b'i' as c_char);
        keys.concat_len((*ins).data.add(mc_ins_span.done_len), dlen);
        if !commit {
            keys.push(ESC);
        }
        keys.push(0);
        let text = if commit && dlen > 1 {
            xmemdupz((*ins).data.add(mc_ins_span.done_len).cast(), dlen - 1).cast()
        } else {
            ptr::null_mut()
        };
        mc_ins_span.done_len = (*ins).size;
        mc_ins_span_push(keys.items, text);
        if !commit {
            mc_ins_preview_rebase();
        }
    }
}

unsafe fn mc_reg_read(reg: c_int) -> *mut c_char {
    unsafe {
        let s = get_reg_contents(reg, 0).cast::<c_char>();
        if s.is_null() {
            return xstrdup(c"".as_ptr());
        }
        let len = strlen(s);
        if len > 0 && *s.add(len - 1) == NL as c_char {
            *s.add(len - 1) = 0;
        }
        s
    }
}

unsafe fn mc_reg_gather() {
    unsafe {
        if mc_cursors.size == 0 || exiting {
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
            if idx >= 0 && (*get_y_register(idx)).timestamp >= mc_start.time {
                gather[i] = true;
                any = true;
            }
            i += 1;
        }
        if !any {
            return;
        }

        let mut primary = CONTEXT_INIT;
        primary.pos = (*curwin).w_cursor;
        primary.regs = shada_encode_regs(false, mc_start.time);

        let mut order: KVec<*mut Context> = KVec::new();
        order.push(&mut primary);
        for c in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(c);
            if (*ctx).regs.size > 0 && handle_get_buffer((*ctx).buf) == curbuf {
                order.push(ctx);
            }
        }
        if order.size == 1 {
            order.destroy();
            api_free_string(primary.regs);
            return;
        }
        let order_slice = std::slice::from_raw_parts_mut(order.items, order.size);
        order_slice.sort_by(|&a, &b| {
            let ca = &*a;
            let cb = &*b;
            if ca.pos.lnum != cb.pos.lnum {
                ca.pos.lnum.cmp(&cb.pos.lnum)
            } else {
                ca.pos.col.cmp(&cb.pos.col)
            }
        });

        let mut save = CONTEXT_INIT;
        ctx_save(&mut save, kCtxRegs);

        let mut joined: [KVec<c_char>; MC_REGS.len()] = [KVec::new(); MC_REGS.len()];
        for o in 0..order.size {
            ctx_load(*order.items.add(o), kCtxRegs, 0);
            let mut idx = 0;
            while MC_REGS[idx] != 0 {
                if gather[idx] {
                    let text = mc_reg_read(c_int::from(MC_REGS[idx]));
                    if *text != 0 {
                        if joined[idx].size > 0 {
                            joined[idx].push(NL as c_char);
                        }
                        sb_concat(&mut joined[idx], text);
                    }
                    xfree(text.cast());
                }
                idx += 1;
            }
        }
        order.destroy();
        api_free_string(primary.regs);
        ctx_load(&mut save, kCtxRegs, 0);
        ctx_free(&mut save);

        let mut idx = 0;
        while MC_REGS[idx] != 0 {
            if gather[idx] && joined[idx].size > 0 {
                let r = c_int::from(MC_REGS[idx]);
                write_reg_contents_ex(
                    r,
                    joined[idx].items,
                    joined[idx].size as isize,
                    false,
                    crate::register::MotionType::kMTLineWise,
                    0,
                );
                if r == b'"' as c_int {
                    set_clipboard(NUL, get_y_previous());
                }
            }
            joined[idx].destroy();
            idx += 1;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_vsel_clear() {
    unsafe {
        let buf = handle_get_buffer(mc_vsel_buf);
        if !buf.is_null() {
            extmark_clear(buf, mc_vsel_ns(), 0, 0, MAXLNUM, MAXCOL);
            extmark_clear(buf, mc_vcur_ns(), 0, 0, MAXLNUM, MAXCOL);
        }
        mc_vsel_buf = 0;
    }
}

unsafe fn mc_vsel_mark(
    start_lnum: linenr_T,
    start_col: colnr_T,
    end_lnum: linenr_T,
    end_col: colnr_T,
) {
    unsafe {
        let mut decor = DECOR_INLINE_INIT;
        decor.data.hl.hl_id = syn_check_group(c"MCursorVisual".as_ptr(), 13);
        let mut mark: u32 = 0;
        extmark_set(
            curbuf,
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
            ptr::null_mut(),
        );
        mc_vsel_buf = (*curbuf).b_fnum;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_vsel_refresh() {
    unsafe {
        mc_vsel_clear();
        let span = atom_visual_span();
        if span.data.is_null() || span.size == 0 || !mc_buf_has_cursors(curbuf) {
            xfree(span.data.cast());
            return;
        }

        let mut sb: McSandbox = std::mem::zeroed();
        mc_sandbox_enter(&mut sb, false);
        block_autocmds();
        block_on_key();
        emsg_silent += 1;

        for i in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(i);
            if !mc_ctx_resolve(ctx, &mut (*ctx).pos) {
                continue;
            }
            ctx_load(ctx, kCtxCursor | kCtxVisual | kCtxMarks, 0);
            check_cursor(curwin);
            Visual.active = false;
            Visual.select = false;
            nvim_feedkeys(span, cstr_as_string(c"nix".as_ptr()), false);
            if !Visual.active {
                continue;
            }
            let mut s = Visual.start;
            let mut e = (*curwin).w_cursor;
            let r_type: MotionType = if Visual.mode == b'V' as c_int {
                kMTLineWise
            } else if Visual.mode == Ctrl_V {
                kMTBlockWise
            } else {
                kMTCharWise
            };
            let mut inclusive = true;
            let mut oa: oparg_T = std::mem::zeroed();
            virtual_op = c_int::from(virtual_active(curwin));
            getregionpos_prep(
                &mut s,
                &mut e,
                r_type,
                *p_sel == b'e' as c_char,
                0,
                &mut inclusive,
                &mut oa,
            );
            if r_type == kMTLineWise {
                mc_vsel_mark(s.lnum, 0, e.lnum, ml_get_len(e.lnum));
            } else if r_type == kMTBlockWise {
                if (*curwin).w_curswant == MAXCOL {
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
                curbuf,
                mc_vcur_ns(),
                &mut cmark,
                (*curwin).w_cursor,
                false,
                true,
                true,
            );
            mc_vsel_buf = (*curbuf).b_fnum;
            Visual.active = false;
        }

        emsg_silent -= 1;
        unblock_on_key();
        unblock_autocmds();
        mc_sandbox_leave(&mut sb);
        xfree(span.data.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_undo_time() {
    unsafe {
        atom_did_global_op();
        extmark_clear(curbuf, mc_ns(), 0, 0, MAXLNUM, MAXCOL);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ins_commit() -> bool {
    unsafe {
        let active = mc_ins_span.active;
        let ins_cascaded = active && !mc_ins_span.first;
        mc_ins_span.active = false;
        mc_ins_span.vsel = ptr::null();

        if ins_cascaded {
            let ins = redo_keys(ptr::null());
            if !ins.data.is_null()
                && ins.size > mc_ins_span.done_len
                && *ins.data.add(ins.size - 1) == ESC
            {
                mc_ins_span_flush(&ins, true);
            }
            api_free_string(ins);
        }

        if mc_ins_span.region != 0 {
            extmark_del_id(curbuf, mc_session_ns(), mc_ins_span.region);
            mc_ins_span.region = 0;
        }
        mc_ins_regions_clear();

        if active && buf_get_changedtick(curbuf) == mc_ins_span.origin.tick {
            u_forget_unchanged(curbuf);
        }

        if !ins_cascaded {
            return false;
        }
        (*curbuf).b_last_changedtick = buf_get_changedtick(curbuf);
        (*curbuf).b_last_changedtick_i = buf_get_changedtick(curbuf);
        for i in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(i);
            if !mc_ctx_resolve(ctx, &mut (*ctx).pos) {
                continue;
            }
            if (*ctx).mark == mc_ins_span.origin.mcursor {
                (*ctx).pos = (*curwin).w_cursor;
            } else if (*ctx).pos.col > 0 {
                dec(&mut (*ctx).pos);
            }
            mc_mark_upd(curbuf, &mut (*ctx).mark, (*ctx).pos);
        }
        true
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_buf_has_cursors(buf: *mut buf_T) -> bool {
    unsafe {
        for i in 0..mc_cursors.size {
            if (*mc_cursors.items.add(i)).buf == (*buf).b_fnum {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_following() -> bool {
    unsafe { (*curbuf).b_p_follow != 0 }
}

unsafe fn mc_lua_enable(enable: bool) {
    unsafe {
        if exiting {
            return;
        }
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_BOOL,
                v_lock: 0,
                vval: TypvalVval {
                    v_bool: if enable { kBoolVarTrue } else { kBoolVarFalse },
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        nlua_call_typval(
            c"vim._core.mcursor".as_ptr(),
            c"enable".as_ptr(),
            tv_args.as_mut_ptr(),
            ptr::null_mut(),
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_mark_at(buf: *mut buf_T, pos: pos_T, skip: u32) -> u32 {
    unsafe {
        if mc_cursors.size == 0 {
            return 0;
        }
        let mut itr: MarkTreeIter = std::mem::zeroed();
        marktree_itr_get(
            (*buf).b_marktree.as_mut_ptr(),
            pos.lnum - 1,
            pos.col,
            &mut itr,
        );
        loop {
            let k = marktree_itr_current(&mut itr);
            if k.id == 0 || k.pos.row != pos.lnum - 1 || k.pos.col != pos.col {
                break;
            }
            if k.ns == mc_ns() && !mt_end(k) && k.id != skip {
                return k.id;
            }
            if !marktree_itr_next((*buf).b_marktree.as_mut_ptr(), &mut itr) {
                break;
            }
        }
        0
    }
}

unsafe fn mc_zq_step(keys: *const c_char, pos: pos_T) -> pos_T {
    unsafe {
        (*curwin).w_cursor = pos;
        (*curwin).w_set_curswant = false;
        msg_silent += 1;
        emsg_silent += 1;
        let save_cmod_flags = cmdmod.cmod_flags;
        cmdmod.cmod_flags |= CMOD_KEEPJUMPS;
        atom_suppress(true);
        block_autocmds();
        block_on_key();
        let mut sst: save_state_T = std::mem::zeroed();
        save_current_state(&mut sst);
        let beeps = did_beep_silent;
        exec_normal_cmd(keys.cast_mut(), REMAP_NONE, true);
        let failed = did_beep_silent != beeps;
        restore_current_state(&mut sst);
        unblock_on_key();
        unblock_autocmds();
        atom_suppress(false);
        cmdmod.cmod_flags = save_cmod_flags;
        emsg_silent -= 1;
        msg_silent -= 1;
        if failed { INIT_POS } else { (*curwin).w_cursor }
    }
}

unsafe fn mc_zq_target(
    step: pos_T,
    search: bool,
    motion_type: MotionType,
    ctx: *mut Context,
) -> visualinfo_T {
    unsafe {
        let save = (*curbuf).b_visual;
        let save_mode_eval = (*curbuf).b_visual_mode_eval;
        if !search {
            (*curbuf).b_visual = visualinfo_T {
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
            (*curbuf).b_visual = visualinfo_T {
                vi_start: INIT_POS,
                vi_end: INIT_POS,
                vi_mode: 0,
                vi_curswant: 0,
            };
        }
        (*curbuf).b_visual_mode_eval = (*curbuf).b_visual.vi_mode;
        if !ctx.is_null() {
            (*curwin).w_cursor = step;
            ctx_save(ctx, kCtxVisual);
        }
        let target = (*curbuf).b_visual;
        (*curbuf).b_visual = save;
        (*curbuf).b_visual_mode_eval = save_mode_eval;
        target
    }
}

unsafe fn mc_zq_first(
    keys: *const c_char,
    search: bool,
    motion_type: MotionType,
    bounds: *const pos_T,
    step: *mut pos_T,
) -> bool {
    unsafe {
        let start = *bounds;
        let end_bound = *bounds.add(1);
        let curswant = (*curwin).w_curswant;
        let mut before = start;
        let bof = dec(&mut before) == -1;
        (*step).lnum = 0;
        if !bof {
            let save_ww = p_ww;
            p_ww = c"s,l,>".as_ptr().cast_mut();
            let s = mc_zq_step(keys, before);
            p_ww = save_ww;
            (*curwin).w_curswant = curswant;
            if ltoreq(start, s) && ltoreq(s, end_bound) {
                *step = s;
            }
        }
        (*curwin).w_cursor = start;
        check_cursor_col(curwin);
        let from = (*curwin).w_cursor;
        let s = mc_zq_step(keys, from);
        if s.lnum > from.lnum && !(*curwin).w_set_curswant {
            let mut c = from;
            getvpos(curwin, &mut c, (*curwin).w_curswant);
            if ltoreq(start, c) && ltoreq(c, end_bound) && ((*step).lnum == 0 || lt(c, *step)) {
                *step = c;
            }
        }
        (*curwin).w_curswant = curswant;
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

unsafe fn mc_zq_run(
    keys: *const c_char,
    search: bool,
    motion_type: MotionType,
    end: pos_T,
    first: pos_T,
) -> c_int {
    unsafe {
        let wrap = end.lnum == MAXLNUM;
        let mut wrapped = false;
        let mut placed: c_int = 0;
        let mut step = first;
        let curswant = (*curwin).w_curswant;
        while !got_int {
            mc_zq_target(step, search, motion_type, mc_add(curbuf, step));
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
        (*curwin).w_curswant = curswant;
        placed
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_zq_start(cap: *mut cmdarg_T) {
    unsafe {
        (*(*cap).oap).from_visual = Visual.active;
        (*(*cap).oap).motion_type = kMTUnknown;
        if Visual.active {
            atom_visual_reset();
            end_visual_mode();
            check_cursor_col(curwin);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_zq(oap: *mut oparg_T, cap: *mut cmdarg_T, origin: pos_T) {
    unsafe {
        atom_did_global_op();
        if mc_replaying()
            || (*oap).motion_type == kMTUnknown
            || (*cap).cmdchar == b'i' as c_int
            || (*cap).cmdchar == b'a' as c_int
            || (*cap).cmdchar == K_LUA
            || (*cap).cmdchar == K_COMMAND
            || (*cap).cmdchar == b':' as c_int
        {
            beep_flush();
            (*curwin).w_cursor = origin;
            return;
        }
        let mut primary = origin;

        let mut v = (*curbuf).b_visual;
        let mut bounds = [
            primary,
            pos_T {
                lnum: MAXLNUM,
                col: 0,
                coladd: 0,
            },
        ];
        if (*oap).from_visual {
            bounds[0] = (*mark_get_visual(curbuf, b'<' as c_int)).mark;
            bounds[1] = (*mark_get_visual(curbuf, b'>' as c_int)).mark;
        }

        let lines = (*oap).motion_type == kMTLineWise;
        if lines {
            bounds[0].col = 0;
            bounds[1].col = MAXCOL;
        }
        let mut vcols: [colnr_T; 2] = [0, MAXCOL];
        let blockwise = (*oap).from_visual && v.vi_mode == Ctrl_V && !lines;
        if blockwise {
            getvcols(
                curwin,
                &mut v.vi_start,
                &mut v.vi_end,
                &mut vcols[0],
                &mut vcols[1],
                0,
            );
            vcols[1] = if v.vi_curswant == MAXCOL {
                MAXCOL
            } else {
                vcols[1]
            };
        }
        (*curwin).w_cursor = primary;
        update_curswant();
        let mut view: viewstate_T = std::mem::zeroed();
        save_viewstate(curwin, &mut view);
        save_search_patterns();

        let search = ((*cap).cmdchar != NUL
            && !strchr(c"*#/?nN".as_ptr(), (*cap).cmdchar).is_null())
            || ((*cap).cmdchar == b'g' as c_int
                && (*cap).nchar != NUL
                && !strchr(c"*#nN".as_ptr(), (*cap).nchar).is_null());
        let mut placed: c_int = 0;
        let mut first = INIT_POS;
        let mut spec = atom_cmd_spec(cap);
        spec.regname = 0;
        let keys = if search {
            cstr_to_string(c"/\n".as_ptr())
        } else {
            redo_keys(&spec)
        };
        if !(*oap).from_visual && search {
            first = mc_zq_target(primary, true, (*oap).motion_type, ptr::null_mut()).vi_start;
            if first.lnum != 0 {
                placed = mc_zq_run(keys.data, search, (*oap).motion_type, bounds[1], first);
            }
        } else if !blockwise {
            if mc_zq_first(
                keys.data,
                search,
                (*oap).motion_type,
                bounds.as_ptr(),
                &mut first,
            ) {
                placed = mc_zq_run(keys.data, search, (*oap).motion_type, bounds[1], first);
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
                if getvpos(curwin, &mut line[0], vcols[0]) == FAIL {
                    line[0].col = ml_get_len(lnum);
                } else if vcols[1] != MAXCOL {
                    getvpos(curwin, &mut line[1], vcols[1]);
                }
                let mut step = INIT_POS;
                if mc_zq_first(
                    keys.data,
                    search,
                    (*oap).motion_type,
                    line.as_ptr(),
                    &mut step,
                ) {
                    if first.lnum == 0 {
                        first = step;
                    }
                    placed += mc_zq_run(keys.data, search, (*oap).motion_type, line[1], step);
                }
            }
        }

        api_free_string(keys);
        restore_search_patterns();
        restore_viewstate(curwin, &view);

        if placed > 0 {
            if mc_mark_at(curbuf, primary, 0) == 0 {
                primary = first;
                (*curwin).w_set_curswant = true;
            }
            (*curbuf).b_visual = mc_zq_target(primary, search, (*oap).motion_type, ptr::null_mut());
            (*curbuf).b_visual_mode_eval = (*curbuf).b_visual.vi_mode;
            set_no_hlsearch(true);
            redraw_all_later(UPD_SOME_VALID);
        }
        (*curwin).w_cursor = primary;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_counter(count1: c_long) {
    unsafe {
        atom_did_global_op();
        let mut tv_args = [
            typval_T {
                v_type: VarType::VAR_NUMBER,
                v_lock: 0,
                vval: TypvalVval {
                    v_number: count1 as varnumber_T,
                },
            },
            typval_T {
                v_type: VarType::VAR_UNKNOWN,
                v_lock: 0,
                vval: TypvalVval { v_number: 0 },
            },
        ];
        nlua_call_typval(
            c"vim._core.mcursor".as_ptr(),
            c"number".as_ptr(),
            tv_args.as_mut_ptr(),
            ptr::null_mut(),
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_follow_set(on: TriState) {
    unsafe {
        if mc_replaying() {
            return;
        }
        let follow = if on == kNone {
            (*curbuf).b_p_follow == 0
        } else {
            on == kTrue
        };
        if ((*curbuf).b_p_follow != 0) != follow {
            set_option_value_give_err(kOptFollow, boolean_obj(follow), OPT_LOCAL);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_toggle(buf: *mut buf_T, pos: pos_T, end_follow: bool, on: TriState) {
    unsafe {
        if mc_replaying() {
            return;
        }
        if end_follow {
            mc_follow_set(kFalse);
        }
        let mark = mc_mark_at(buf, pos, 0);
        if mark != 0 && on != kTrue {
            extmark_del_id(buf, mc_ns(), mark);
            mc_cleanup(false, ptr::null(), 0);
        } else if mark == 0 && on != kFalse {
            mc_add(buf, pos);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_add(buf: *mut buf_T, pos: pos_T) -> *mut Context {
    unsafe {
        let existing = mc_mark_at(buf, pos, 0);
        if existing != 0 {
            for i in 0..mc_cursors.size {
                let ctx = mc_cursors.items.add(i);
                if (*ctx).buf == (*buf).b_fnum && (*ctx).mark == existing {
                    return ctx;
                }
            }
        }
        let count = mc_cursors.size;
        let mut mark: u32 = 0;
        mc_mark_upd(buf, &mut mark, pos);
        if mc_cursors.size > count {
            mc_cursors.items.add(mc_cursors.size - 1)
        } else {
            ptr::null_mut()
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_on_extmark_set(buf: *mut buf_T, ns_id: u32, mut id: u32, pos: pos_T) {
    unsafe {
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
        if mc_cursors.size == 0 {
            mc_start.time = os_realtime() as Timestamp;
            ctx_save(&raw mut mc_start.ctx, kCtxRegs);
            mc_lua_enable(true);
        }
        let mut ctx = CONTEXT_INIT;
        ctx.buf = (*buf).b_fnum;
        ctx.pos = pos;
        ctx.mark = id;
        crate::kvec::kv_push(&raw mut mc_cursors, ctx);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_buf_clear(buf: *mut buf_T) {
    unsafe {
        if mc_replaying() {
            return;
        }
        extmark_clear(buf, mc_ns(), 0, 0, MAXLNUM, MAXCOL);
        extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_buf_free(buf: *mut buf_T) {
    unsafe {
        if mc_replaying() {
            return;
        }
        mc_cleanup(false, ptr::null(), 0);
        if mc_vsel_buf == (*buf).b_fnum {
            mc_vsel_buf = 0;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ns_clearing(buf: *mut buf_T, ns_id: u32) {
    unsafe {
        if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() {
            return;
        }
        for i in 0..mc_cursors.size {
            let ctx = mc_cursors.items.add(i);
            if (*ctx).buf == (*buf).b_fnum && (*ctx).mark != 0 {
                extmark_get_pos(buf, mc_ns(), (*ctx).mark, &mut (*ctx).pos);
            }
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_ns_cleared(buf: *mut buf_T, ns_id: u32) {
    unsafe {
        if (ns_id != mc_ns() && ns_id != 0) || mc_replaying() || !mc_buf_has_cursors(buf) {
            return;
        }

        let mut others = false;
        let mut i = 0;
        while i < mc_cursors.size && !others {
            let ctx = mc_cursors.items.add(i);
            let mut pos = INIT_POS;
            others = (*ctx).buf != (*buf).b_fnum
                || ((*ctx).mark != 0 && extmark_get_pos(buf, mc_ns(), (*ctx).mark, &mut pos));
            i += 1;
        }
        if !others {
            mc_reg_gather();
        }

        if !exiting {
            extmark_clear(buf, mc_last_ns(), 0, 0, MAXLNUM, MAXCOL);
            if curbuf == buf {
                let mut primary: u32 = 1;
                extmark_set_pos(
                    buf,
                    mc_last_ns(),
                    &mut primary,
                    (*curwin).w_cursor,
                    true,
                    false,
                    false,
                );
            }
            for idx in 0..mc_cursors.size {
                let ctx = mc_cursors.items.add(idx);
                if (*ctx).buf != (*buf).b_fnum {
                    continue;
                }
                let mut mark: u32 = 0;
                extmark_set_pos(buf, mc_last_ns(), &mut mark, (*ctx).pos, true, false, false);
            }
        }
        mc_cleanup(false, ptr::null(), 0);
        if mc_cursors.size == 0 {
            atoms_free(&raw mut g_atoms);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mc_free_all() {
    unsafe {
        while mc_cursors.size > 0 {
            mc_cursors.size -= 1;
            let mut ctx = *mc_cursors.items.add(mc_cursors.size);
            ctx_free(&mut ctx);
        }
        crate::kvec::kv_destroy(&raw mut mc_cursors);
        crate::kvec::kv_destroy(&raw mut mc_ins_span.regions);
        ctx_free(&raw mut mc_start.ctx);
    }
}
