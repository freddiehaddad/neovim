//! Functions for managing signs (`src/nvim/sign.c`).
#![forbid(unsafe_code)]

use crate::api::extmark::{describe_ns, nvim_create_namespace};
use crate::api::private::helpers::cstr_as_string;

use crate::buffer::{FileBuf, buf_hide, buf_meta_total, buflist_findname_exp, buflist_findnr};
use crate::charset::{ascii_isdigit, ascii_iswhite, backslash_halve, getdigits_int, vim_isprintc};
use crate::cursor::{Linenr, check_cursor_lnum};
use crate::decoration::{
    DECOR_SIGN_HIGHLIGHT_INIT, DecorInline, DecorPriority, DecorSignHighlight, K_SHIS_SIGN, NS,
    SIGN_WIDTH, SignItem, decor_find_sign, decor_put_sh, sign_item_cmp,
};
use crate::drawscreen::{UPD_NOT_VALID, redraw_buf_later};
use crate::errors::*;
use crate::eval::funcs::get_buf_arg;
use crate::eval::typval::{
    EvalFuncData, TvDict, TvList, TypVal, VarType, tv_check_for_nonnull_dict_arg,
    tv_check_for_opt_dict_arg, tv_check_for_string_arg, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_dict_find, tv_dict_find_opt,
    tv_dict_get_number, tv_dict_get_number_def, tv_dict_get_string, tv_get_lnum, tv_get_number_chk,
    tv_get_string, tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_number, tv_list_iter,
};
use crate::ex_cmds::{BL_WHITE, ECMD_HIDE, do_ecmd};
use crate::ex_docmd::Exarg;
use crate::extmark::{extmark_del, extmark_del_id, extmark_set};
use crate::fold::fold_open_cursor;
use crate::globals::*;
use crate::grid::{Schar, schar_from_ascii, schar_from_cluster};
use crate::highlight_group::{get_highlight_name_ext, syn_check_group};
use crate::insert::beginline;
use crate::mark::setpcmark;
use crate::marktree::{
    DecorExt, DecorInlineData, K_MTMETA_SIGN_HL, K_MTMETA_SIGN_TEXT, MT_FLAG_DECOR_SIGNHL,
    MT_FLAG_DECOR_SIGNTEXT, MTKey, MTPair, MarkTreeIter, marktree_itr_current, marktree_itr_get,
    marktree_itr_get_overlap, marktree_itr_next, marktree_itr_step_overlap, marktree_lookup_ns,
    mt_decor, mt_decor_sign, mt_end,
};
use crate::mbyte::{utf_cluster_info, utf_ptr2_str_char_info};
use crate::message::{
    emsg, msg_outtrans, msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, semsg, smsg,
};
use crate::msgpack_rpc::packer::Integer;
use crate::strings::{format_to_buf, gettext, vim_strchr};
use crate::window::buf_jump_open_win;
use std::cmp::Ordering;
use std::ffi::CStr;

const SIGN_DEF_PRIO: i32 = 10;

const SIGNCMD_DEFINE: i32 = 0;
const SIGNCMD_UNDEFINE: i32 = 1;
const SIGNCMD_LIST: i32 = 2;
const SIGNCMD_PLACE: i32 = 3;
const SIGNCMD_UNPLACE: i32 = 4;
const SIGNCMD_JUMP: i32 = 5;
const SIGNCMD_LAST: i32 = 6;

pub(crate) const EXP_SUBCMD: i32 = 0;
const EXP_DEFINE: i32 = 1;
const EXP_PLACE: i32 = 2;
const EXP_LIST: i32 = 3;
const EXP_UNPLACE: i32 = 4;
const EXP_SIGN_NAMES: i32 = 5;
const EXP_SIGN_GROUPS: i32 = 6;

#[repr(C)]
#[derive(Clone)]
pub struct Sign {
    pub sn_name: std::ffi::CString,
    pub sn_icon: Option<std::ffi::CString>,
    pub sn_text: [Schar; SIGN_WIDTH],
    pub sn_line_hl: i32,
    pub sn_text_hl: i32,
    pub sn_cul_hl: i32,
    pub sn_num_hl: i32,
    pub sn_priority: i32,
}

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::cmdexpand::{
    EXPAND_BUFFERS, EXPAND_FILES, EXPAND_HIGHLIGHT, EXPAND_NOTHING, EXPAND_SIGN, HLF_D,
};
use crate::eval::K_LIST_LEN_MAY_KNOW_CONST;
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::os::env::ExpandCtx;

struct SignModuleState {
    sign_map: std::cell::RefCell<Vec<Sign>>,
    sign_ns: std::cell::RefCell<Vec<Integer>>,
    expand_what: std::cell::Cell<i32>,
}

impl SignModuleState {
    const fn new() -> Self {
        Self {
            sign_map: std::cell::RefCell::new(Vec::new()),
            sign_ns: std::cell::RefCell::new(Vec::new()),
            expand_what: std::cell::Cell::new(EXP_SUBCMD),
        }
    }
}

std::thread_local! {
    static SIGN_STATE: SignModuleState = const { SignModuleState::new() };
}

const CMDS: [&CStr; 6] = [
    c"define",
    c"undefine",
    c"list",
    c"place",
    c"unplace",
    c"jump",
];

#[inline]
fn pmap_get_sign(name: &[u8]) -> Option<Sign> {
    let name_bytes = crate::memory::bytes_until_nul(name);
    if name_bytes.is_empty() {
        return None;
    }
    SIGN_STATE.with(|st| {
        st.sign_map
            .borrow()
            .iter()
            .find(|sp| sp.sn_name.as_bytes() == name_bytes)
            .cloned()
    })
}

#[inline]
fn find_ns_by_name(group: Option<&CStr>) -> i32 {
    let Some(group_cstr) = group else {
        return 0;
    };
    g_st()
        .namespace_ids
        .iter()
        .find_map(|(k, v)| (k.as_c_str() == group_cstr).then_some(*v))
        .unwrap_or(0)
}

fn group_get_ns(group: Option<&CStr>) -> i64 {
    let Some(group) = group else {
        return 0;
    };
    if group.to_bytes() == b"*" {
        return i64::from(u32::MAX);
    }
    let ns = find_ns_by_name(Some(group));
    if ns != 0 { i64::from(ns) } else { -1 }
}

fn sign_get_name(sh: &DecorSignHighlight) -> &CStr {
    let Some(ref name) = sh.sign_name else {
        return c"";
    };
    if pmap_get_sign(name.to_bytes()).is_some() {
        name.as_c_str()
    } else {
        c"[Deleted]"
    }
}

fn buf_set_sign(
    buf: &mut FileBuf,
    id: &mut u32,
    group: Option<&CStr>,
    prio: i32,
    lnum: Linenr,
    sp: &Sign,
) {
    if group.is_some() && find_ns_by_name(group) == 0 {
        let ns_id = nvim_create_namespace(cstr_as_string(group));
        SIGN_STATE.with(|st| st.sign_ns.borrow_mut().push(ns_id));
    }

    let ns = if group.is_some() {
        nvim_create_namespace(cstr_as_string(group)) as u32
    } else {
        0
    };
    let mut sign = DECOR_SIGN_HIGHLIGHT_INIT;

    sign.flags |= K_SHIS_SIGN;
    sign.text = sp.sn_text;
    sign.sign_name = Some(sp.sn_name.clone());
    sign.hl_id = sp.sn_text_hl;
    sign.line_hl_id = sp.sn_line_hl;
    sign.number_hl_id = sp.sn_num_hl;
    sign.cursorline_hl_id = sp.sn_cul_hl;
    sign.priority = prio as DecorPriority;

    let has_hl = sp.sn_line_hl != 0 || sp.sn_num_hl != 0 || sp.sn_cul_hl != 0;
    let decor_flags = if sp.sn_text[0] != 0 {
        MT_FLAG_DECOR_SIGNTEXT
    } else {
        0
    } | if has_hl { MT_FLAG_DECOR_SIGNHL } else { 0 };

    let decor = DecorInline {
        data: DecorInlineData::Ext(DecorExt {
            sh_idx: decor_put_sh(sign),
            vt: crate::decoration::DecorVirtId::NULL,
        }),
    };
    extmark_set(
        buf,
        ns,
        Some(&mut *id),
        buf.b_ml.ml_line_count.get().min(lnum) - 1,
        0,
        -1,
        -1,
        decor,
        decor_flags,
        true,
        false,
        true,
        true,
    );
}

fn buf_mod_sign(
    buf: &mut FileBuf,
    id: &mut u32,
    group: Option<&CStr>,
    prio: i32,
    sp: &Sign,
) -> Linenr {
    let ns = group_get_ns(group);
    if ns < 0 || (group.is_some() && ns == 0) {
        return 0;
    }

    let mark = marktree_lookup_ns(&buf.b_marktree, ns as u32, *id, false, None);
    if mark.pos.row >= 0 {
        buf_set_sign(buf, id, group, prio, mark.pos.row + 1, sp);
    }
    mark.pos.row + 1
}

fn buf_findsign(buf: &mut FileBuf, id: i32, group: Option<&CStr>) -> i32 {
    let ns = group_get_ns(group);
    if ns < 0 || (group.is_some() && ns == 0) {
        return 0;
    }
    marktree_lookup_ns(&buf.b_marktree, ns as u32, id as u32, false, None)
        .pos
        .row
        + 1
}

fn sign_row_cmp_slice(s1: &MTKey, s2: &MTKey) -> Ordering {
    if s1.pos.row != s2.pos.row {
        return if s1.pos.row > s2.pos.row {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }

    let sh1 = decor_find_sign(mt_decor(*s1)).unwrap();
    let sh2 = decor_find_sign(mt_decor(*s2)).unwrap();
    let si1 = SignItem { sh: sh1, id: s1.id };
    let si2 = SignItem { sh: sh2, id: s2.id };

    sign_item_cmp(&si1, &si2)
}

fn buf_delete_signs(buf: &mut FileBuf, group: Option<&CStr>, id: i32, atlnum: Linenr) -> bool {
    let ns = group_get_ns(group);
    if ns < 0 {
        return false;
    }

    let mut itr = MarkTreeIter::new();
    let row = if atlnum > 0 { atlnum - 1 } else { 0 };
    let mut signs: Vec<MTKey> = Vec::new();

    if atlnum > 0 {
        if !marktree_itr_get_overlap(&buf.b_marktree, row, 0, &mut itr) {
            return false;
        }

        let mut pair: MTPair = MTPair::new();
        while marktree_itr_step_overlap(&buf.b_marktree, &mut itr, &mut pair) {
            if (ns == i64::from(u32::MAX) || ns == i64::from(pair.start.ns))
                && mt_decor_sign(pair.start)
            {
                signs.push(pair.start);
            }
        }
    } else {
        marktree_itr_get(&buf.b_marktree, 0, 0, &mut itr);
    }

    while itr.x.is_some() {
        let mark = marktree_itr_current(&buf.b_marktree, &itr);
        if row != 0 && mark.pos.row > row {
            break;
        }
        if !mt_end(mark)
            && mt_decor_sign(mark)
            && (id == 0 || mark.id as i32 == id)
            && (ns == i64::from(u32::MAX) || ns == i64::from(mark.ns))
        {
            if atlnum > 0 {
                signs.push(mark);
                marktree_itr_next(&buf.b_marktree, &mut itr);
            } else {
                extmark_del(buf, &mut itr, mark, true);
            }
        } else {
            marktree_itr_next(&buf.b_marktree, &mut itr);
        }
    }

    if !signs.is_empty() {
        signs.sort_by(sign_row_cmp_slice);
        let first = signs[0];
        extmark_del_id(buf, first.ns, first.id);
    } else if atlnum > 0 {
        return false;
    }

    true
}

pub fn buf_has_signs(buf: &FileBuf) -> bool {
    buf_meta_total(buf, K_MTMETA_SIGN_HL) + buf_meta_total(buf, K_MTMETA_SIGN_TEXT) != 0
}

fn sign_list_placed(rbuf: Option<&FileBuf>, group: Option<&CStr>) {
    let mut lbuf = [0u8; MSG_BUF_LEN];
    let mut namebuf = [0u8; MSG_BUF_LEN];
    let mut groupbuf = [0u8; MSG_BUF_LEN];
    let mut buf = if let Some(rbuf) = rbuf {
        crate::globals::BufPtr::from(rbuf)
    } else {
        g_st().firstbuf.get()
    };
    let ns = group_get_ns(group);

    msg_puts_title(gettext(c"\n--- Signs ---"));

    while let Some(buf_ref) = buf.opt() {
        if g_st().got_int.get() {
            break;
        }
        if buf_has_signs(buf_ref) {
            msg_putchar(b'\n' as i32);
            let fname = buf_ref.fname_opt().unwrap_or_else(|| gettext(c"Untitled"));
            format_to_buf!(&mut lbuf[..], gettext(c"Signs for %s:"), fname);
            msg_puts_hl(crate::memory::bytes_as_cstr(&lbuf), HLF_D, false);
        }

        if ns >= 0 {
            let mut itr = MarkTreeIter::new();
            let mut signs: Vec<MTKey> = Vec::new();
            let b = &buf_ref.b_marktree;
            marktree_itr_get(b, 0, 0, &mut itr);

            while itr.x.is_some() {
                let mark = marktree_itr_current(b, &itr);
                if !mt_end(mark)
                    && mt_decor_sign(mark)
                    && (ns == i64::from(u32::MAX) || ns == i64::from(mark.ns))
                {
                    signs.push(mark);
                }
                marktree_itr_next(b, &mut itr);
            }

            if !signs.is_empty() {
                signs.sort_by(sign_row_cmp_slice);
                msg_putchar(b'\n' as i32);

                for (i, &mark) in signs.iter().enumerate() {
                    namebuf[0] = 0;
                    groupbuf[0] = 0;

                    let sh = decor_find_sign(mt_decor(mark)).unwrap();
                    if sh.sign_name.is_some() {
                        format_to_buf!(&mut namebuf[..], gettext(c"  name=%s"), sign_get_name(&sh));
                    }
                    if mark.ns != 0 {
                        format_to_buf!(
                            &mut groupbuf[..],
                            gettext(c"  group=%s"),
                            describe_ns(mark.ns as i32, c""),
                        );
                    }
                    format_to_buf!(
                        &mut lbuf[..],
                        gettext(c"    line=%d  id=%u%s%s  priority=%d"),
                        mark.pos.row + 1,
                        mark.id,
                        &groupbuf,
                        &namebuf,
                        i32::from(sh.priority),
                    );
                    msg_puts(crate::memory::bytes_as_cstr(&lbuf));
                    if i < signs.len() - 1 {
                        msg_putchar(b'\n' as i32);
                    }
                }
            }
        }

        if rbuf.is_some() {
            return;
        }
        buf = buf_ref.b_next.get();
    }
}

fn sign_cmd_idx(begin_bytes: &[u8]) -> i32 {
    let idx = CMDS
        .iter()
        .position(|cmd| cmd.to_bytes() == begin_bytes)
        .unwrap_or(CMDS.len());
    idx as i32
}

pub fn describe_sign_text(buf: &mut [u8], sign_text: &[Schar; SIGN_WIDTH]) -> usize {
    let mut p: usize = 0;
    for &sc in sign_text {
        let len = crate::grid::schar_get_bytes(&mut buf[p..], sc);
        if len == 0 {
            break;
        }
        p += len;
    }
    p
}

pub fn init_sign_text(sp: Option<&Sign>, sign_text: &mut [Schar; SIGN_WIDTH], text: &[u8]) -> bool {
    let mut unescaped: Vec<u8> = Vec::with_capacity(text.len() + 1);
    if sp.is_some() {
        let mut i = 0;
        while i < text.len() {
            if text[i] == b'\\' && i + 1 < text.len() {
                i += 1;
            }
            unescaped.push(text[i]);
            i += 1;
        }
    } else {
        unescaped.extend_from_slice(text);
    }
    unescaped.push(0);

    let mut cells: usize = 0;
    let mut ci = utf_ptr2_str_char_info(crate::memory::bytes_as_cstr(&unescaped));
    while !ci.ptr.is_empty() {
        let cli = utf_cluster_info(ci);
        if !vim_isprintc(ci.chr.value) || cells + (cli.cells as usize) > SIGN_WIDTH {
            if sp.is_some() {
                semsg!(gettext(c"E239: Invalid sign text: %s"), &unescaped);
            }
            return false;
        }
        sign_text[cells] = schar_from_cluster(ci, cli);
        debug_assert!(cli.cells <= 2);
        if cli.cells == 2 {
            sign_text[cells + 1] = 0;
        }
        cells += cli.cells as usize;
        ci = cli.next;
    }

    if cells < 1 {
        sign_text[0] = 0;
    } else if cells == 1 {
        sign_text[1] = schar_from_ascii(b' ');
    }

    true
}

fn sign_define_by_name(
    name: &CStr,
    icon: Option<&CStr>,
    text: Option<&CStr>,
    linehl: Option<&CStr>,
    texthl: Option<&CStr>,
    culhl: Option<&CStr>,
    numhl: Option<&CStr>,
    prio: i32,
) -> bool {
    let existing = pmap_get_sign(name.to_bytes());
    let new_sign = existing.is_none();
    let mut sp = existing.unwrap_or_else(|| Sign {
        sn_name: name.to_owned(),
        sn_icon: None,
        sn_text: [0; SIGN_WIDTH],
        sn_line_hl: 0,
        sn_text_hl: 0,
        sn_cul_hl: 0,
        sn_num_hl: 0,
        sn_priority: 0,
    });

    if let Some(icon_cstr) = icon {
        let mut icon_vec = icon_cstr.to_bytes_with_nul().to_vec();
        backslash_halve(&mut icon_vec);
        let end = icon_vec
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(icon_vec.len());
        icon_vec.truncate(end);
        sp.sn_icon = Some(std::ffi::CString::new(icon_vec).unwrap());
    }

    if let Some(text_cstr) = text {
        let mut sn_text = sp.sn_text;
        if !init_sign_text(Some(&sp), &mut sn_text, text_cstr.to_bytes()) {
            sp.sn_text = sn_text;
            SIGN_STATE.with(|st| {
                let mut map = st.sign_map.borrow_mut();
                if let Some(slot) = map.iter_mut().find(|item| item.sn_name.as_c_str() == name) {
                    *slot = sp;
                } else {
                    map.push(sp);
                }
            });
            return false;
        }
        sp.sn_text = sn_text;
    }

    sp.sn_priority = prio;

    for (hl_slot, arg) in [
        (&mut sp.sn_line_hl, linehl),
        (&mut sp.sn_text_hl, texthl),
        (&mut sp.sn_cul_hl, culhl),
        (&mut sp.sn_num_hl, numhl),
    ] {
        if let Some(arg_cstr) = arg {
            *hl_slot = if !arg_cstr.to_bytes().is_empty() {
                syn_check_group(arg_cstr.to_bytes())
            } else {
                0
            };
        }
    }

    if !new_sign {
        let mut did_redraw = false;
        crate::decoration::with_decor_items_mut(|items| {
            for sh in items.iter_mut() {
                if sh
                    .sign_name
                    .as_ref()
                    .is_some_and(|sn| sn.to_bytes() == name.to_bytes())
                {
                    sh.text = sp.sn_text;
                    sh.hl_id = sp.sn_text_hl;
                    sh.line_hl_id = sp.sn_line_hl;
                    sh.number_hl_id = sp.sn_num_hl;
                    sh.cursorline_hl_id = sp.sn_cul_hl;
                    if !did_redraw {
                        for wp in for_all_windows_mut() {
                            if buf_has_signs(wp.buf_mut()) {
                                redraw_buf_later(wp.buf_mut(), UPD_NOT_VALID);
                            }
                        }
                        did_redraw = true;
                    }
                }
            }
        });
    }

    SIGN_STATE.with(|st| {
        let mut map = st.sign_map.borrow_mut();
        if let Some(slot) = map.iter_mut().find(|item| item.sn_name.as_c_str() == name) {
            *slot = sp;
        } else {
            map.push(sp);
        }
    });

    true
}

fn sign_undefine_by_name(name: &CStr) -> bool {
    let removed = SIGN_STATE.with(|st| {
        let mut map = st.sign_map.borrow_mut();
        let pos = map.iter().position(|sp| sp.sn_name.as_c_str() == name)?;
        map.swap_remove(pos);
        Some(())
    });
    if removed.is_none() {
        semsg!(gettext(c"E155: Unknown sign: %s"), name);
        return false;
    }
    true
}

fn sign_list_defined(sp: &Sign) {
    smsg!(0, c"sign %s", &sp.sn_name);
    if let Some(ref icon) = sp.sn_icon {
        msg_puts(c" icon=");
        msg_outtrans(icon, 0, false);
        msg_puts(gettext(c" (not supported)"));
    }
    if sp.sn_text[0] != 0 {
        msg_puts(c" text=");
        let mut buf = [0u8; SIGN_WIDTH * MAX_SCHAR_SIZE];
        describe_sign_text(&mut buf, &sp.sn_text);
        msg_outtrans(crate::memory::bytes_as_cstr(&buf), 0, false);
    }
    if sp.sn_priority > 0 {
        let mut lbuf = [0u8; MSG_BUF_LEN];
        format_to_buf!(&mut lbuf[..], c" priority=%d", sp.sn_priority);
        msg_puts(crate::memory::bytes_as_cstr(&lbuf));
    }
    let args = [c" linehl=", c" texthl=", c" culhl=", c" numhl="];
    let hls = [sp.sn_line_hl, sp.sn_text_hl, sp.sn_cul_hl, sp.sn_num_hl];
    for i in 0..4 {
        if hls[i] > 0 {
            msg_puts(args[i]);
            let p = get_highlight_name_ext(hls[i] - 1, false);
            msg_puts(p.as_deref().unwrap_or(c"NONE"));
        }
    }
}

fn sign_list_by_name(name: &CStr) {
    if let Some(sp) = pmap_get_sign(name.to_bytes()) {
        sign_list_defined(&sp);
    } else {
        semsg!(gettext(c"E155: Unknown sign: %s"), name);
    }
}

fn sign_place(
    id: &mut u32,
    group: Option<&CStr>,
    name: &CStr,
    buf: &mut FileBuf,
    mut lnum: Linenr,
    mut prio: i32,
) -> bool {
    if let Some(g) = group
        && (g.to_bytes().first() == Some(&b'*') || g.to_bytes().is_empty())
    {
        return false;
    }

    let Some(sp) = pmap_get_sign(name.to_bytes()) else {
        semsg!(gettext(c"E155: Unknown sign: %s"), name);
        return false;
    };

    if prio == -1 {
        prio = if sp.sn_priority != -1 {
            sp.sn_priority
        } else {
            SIGN_DEF_PRIO
        };
    }

    if lnum > 0 {
        buf_set_sign(buf, id, group, prio, lnum, &sp);
    } else {
        lnum = buf_mod_sign(buf, &mut *id, group, prio, &sp);
    }
    if lnum <= 0 {
        semsg!(gettext(c"E885: Not possible to change sign %s"), name);
        return false;
    }

    true
}

fn sign_unplace_inner(buf: &mut FileBuf, id: i32, group: Option<&CStr>, atlnum: Linenr) -> bool {
    if !buf_has_signs(buf) {
        return false;
    }

    if id == 0 || atlnum > 0 || group.is_some_and(|g| g.to_bytes().first() == Some(&b'*')) {
        if !buf_delete_signs(buf, group, id, atlnum) {
            return false;
        }
    } else {
        let ns = group_get_ns(group);
        if ns < 0 || !extmark_del_id(buf, ns as u32, id as u32) {
            return false;
        }
    }

    true
}

fn sign_unplace(buf: Option<&mut FileBuf>, id: i32, group: Option<&CStr>, atlnum: Linenr) -> bool {
    if let Some(buf) = buf {
        sign_unplace_inner(buf, id, group, atlnum)
    } else {
        let mut retval = true;
        for cbuf in for_all_buffers_ptr() {
            if !sign_unplace_inner(&mut *cbuf, id, group, atlnum) {
                retval = false;
            }
        }
        retval
    }
}

fn sign_jump(id: i32, group: Option<&CStr>, buf: &mut FileBuf) -> Linenr {
    let lnum = buf_findsign(buf, id, group);
    if lnum <= 0 {
        semsg!(gettext(c"E157: Invalid sign ID: %d"), id);
        return -1;
    }

    if buf_jump_open_win(buf).is_some() {
        curwin_mut().w_cursor.lnum = lnum;
        check_cursor_lnum(curwin_mut());
        beginline(BL_WHITE);
    } else {
        if buf.fname_opt().is_none() {
            emsg(gettext(
                c"E934: Cannot jump to a buffer that does not have a name",
            ));
            return -1;
        }
        setpcmark();
        if !do_ecmd(
            buf.b_fnum.get(),
            None,
            None,
            None,
            lnum,
            if buf_hide(curbuf_mut()) { ECMD_HIDE } else { 0 },
            Some(curwin_mut()),
        ) {
            return -1;
        }
    }

    fold_open_cursor();
    lnum
}

fn sign_define_cmd(name: &CStr, cmdline_in: &mut [u8]) {
    let mut cmd_idx = 0usize;
    let mut icon_off: Option<usize> = None;
    let mut text_off: Option<usize> = None;
    let mut linehl_off: Option<usize> = None;
    let mut texthl_off: Option<usize> = None;
    let mut culhl_off: Option<usize> = None;
    let mut numhl_off: Option<usize> = None;
    let mut prio: i32 = -1;

    loop {
        let arg_idx = cmd_idx + crate::charset::skipwhite_len(&cmdline_in[cmd_idx..]);
        if cmdline_in.get(arg_idx).copied().unwrap_or(0) == 0 {
            break;
        }
        cmd_idx = arg_idx + crate::charset::skiptowhite_esc_len(&cmdline_in[arg_idx..]);
        let arg_cstr = crate::memory::bytes_as_cstr(&cmdline_in[arg_idx..]);
        let arg_bytes = arg_cstr.to_bytes();
        if arg_bytes.starts_with(b"icon=") {
            icon_off = Some(arg_idx + 5);
        } else if arg_bytes.starts_with(b"text=") {
            text_off = Some(arg_idx + 5);
        } else if arg_bytes.starts_with(b"linehl=") {
            linehl_off = Some(arg_idx + 7);
        } else if arg_bytes.starts_with(b"texthl=") {
            texthl_off = Some(arg_idx + 7);
        } else if arg_bytes.starts_with(b"culhl=") {
            culhl_off = Some(arg_idx + 6);
        } else if arg_bytes.starts_with(b"numhl=") {
            numhl_off = Some(arg_idx + 6);
        } else if let Some(rest) = arg_bytes.strip_prefix(b"priority=") {
            prio = crate::charset::parse_i64_prefix(rest) as i32;
        } else {
            semsg!(gettext(E_INVARG2), arg_cstr);
            return;
        }
        if cmdline_in.get(cmd_idx).copied().unwrap_or(0) == 0 {
            break;
        }

        cmdline_in[cmd_idx] = 0;
        cmd_idx += 1;
    }

    sign_define_by_name(
        name,
        icon_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        text_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        linehl_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        texthl_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        culhl_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        numhl_off.map(|off| crate::memory::bytes_as_cstr(&cmdline_in[off..])),
        prio,
    );
}

fn sign_place_cmd(
    buf: Option<&mut FileBuf>,
    lnum: Linenr,
    name: Option<&CStr>,
    id: i32,
    group: Option<&CStr>,
    prio: i32,
) {
    if id <= 0 {
        if lnum >= 0 || name.is_some() || group.is_some_and(|g| g.to_bytes().is_empty()) {
            emsg(gettext(E_INVARG));
        } else {
            sign_list_placed(buf.as_deref(), group);
        }
    } else {
        let (Some(name_cstr), Some(buf_ref)) = (name, buf) else {
            emsg(gettext(E_INVARG));
            return;
        };
        if group.is_some_and(|g| g.to_bytes().is_empty()) {
            emsg(gettext(E_INVARG));
            return;
        }
        let mut uid = id as u32;
        sign_place(&mut uid, group, name_cstr, buf_ref, lnum, prio);
    }
}

fn sign_unplace_cmd(
    mut buf: Option<&mut FileBuf>,
    mut lnum: Linenr,
    name: Option<&CStr>,
    id: i32,
    group: Option<&CStr>,
) {
    if lnum >= 0 || name.is_some() || group.is_some_and(|g| g.to_bytes().is_empty()) {
        emsg(gettext(E_INVARG));
        return;
    }

    if id == -1 {
        lnum = curwin_mut().w_cursor.lnum;
        buf = curwin_mut().w_buffer.get().opt_mut();
    }

    if !sign_unplace(buf, id.max(0), group, lnum) && lnum > 0 {
        emsg(gettext(c"E159: Missing sign number"));
    }
}

fn sign_jump_cmd(
    buf: Option<&mut FileBuf>,
    lnum: Linenr,
    name: Option<&CStr>,
    id: i32,
    group: Option<&CStr>,
) {
    if name.is_none() && group.is_none() && id == -1 {
        emsg(gettext(E_ARGREQ));
        return;
    }

    if buf.is_none()
        || group.is_some_and(|g| g.to_bytes().is_empty())
        || lnum >= 0
        || name.is_some()
    {
        emsg(gettext(E_INVARG));
        return;
    }

    sign_jump(id, group, buf.unwrap());
}

fn parse_sign_cmd_args<'a>(
    cmd: i32,
    arg_in: &'a mut [u8],
    name: &mut Option<&'a CStr>,
    id: &mut i32,
    group: &mut Option<&'a CStr>,
    prio: &mut i32,
    buf: &mut Option<&'static mut FileBuf>,
    lnum: &mut Linenr,
) -> bool {
    let mut arg_idx = 0usize;
    let mut filename_off: Option<usize> = None;
    let mut name_off: Option<usize> = None;
    let mut group_off: Option<usize> = None;
    let mut lnum_arg = false;

    if ascii_isdigit(arg_in.first().copied().unwrap_or(0)) {
        let arg_bytes = crate::memory::bytes_until_nul(arg_in);
        let mut rem = arg_bytes;
        *id = getdigits_int(&mut rem, true, 0);
        arg_idx = arg_bytes.len() - rem.len();
        let next_b = arg_in.get(arg_idx).copied().unwrap_or(0);
        if !ascii_iswhite(next_b) && next_b != 0 {
            *id = -1;
            arg_idx = 0;
        } else {
            arg_idx += crate::charset::skipwhite_len(&arg_in[arg_idx..]);
        }
    }

    while arg_in.get(arg_idx).copied().unwrap_or(0) != 0 {
        let arg_bytes = crate::memory::bytes_until_nul(&arg_in[arg_idx..]);
        if arg_bytes.starts_with(b"line=") {
            arg_idx += 5;
            *lnum = crate::charset::parse_i64_prefix(&arg_bytes[5..]) as i32;
            arg_idx += crate::charset::skiptowhite_len(&arg_in[arg_idx..]);
            lnum_arg = true;
        } else if arg_bytes.starts_with(b"*") && cmd == SIGNCMD_UNPLACE {
            if *id != -1 {
                emsg(gettext(E_INVARG));
                return false;
            }
            *id = -2;
            arg_idx += 1 + crate::charset::skiptowhite_len(&arg_in[arg_idx + 1..]);
        } else if arg_bytes.starts_with(b"name=") {
            arg_idx += 5;
            let mut namep = arg_idx;
            arg_idx += crate::charset::skiptowhite_len(&arg_in[arg_idx..]);
            if arg_in.get(arg_idx).copied().unwrap_or(0) != 0 {
                arg_in[arg_idx] = 0;
                arg_idx += 1;
            }
            while arg_in.get(namep).copied() == Some(b'0')
                && arg_in.get(namep + 1).copied().unwrap_or(0) != 0
            {
                namep += 1;
            }
            name_off = Some(namep);
        } else if arg_bytes.starts_with(b"group=") {
            arg_idx += 6;
            let groupp = arg_idx;
            arg_idx += crate::charset::skiptowhite_len(&arg_in[arg_idx..]);
            if arg_in.get(arg_idx).copied().unwrap_or(0) != 0 {
                arg_in[arg_idx] = 0;
                arg_idx += 1;
            }
            group_off = Some(groupp);
        } else if arg_bytes.starts_with(b"priority=") {
            arg_idx += 9;
            *prio = crate::charset::parse_i64_prefix(&arg_bytes[9..]) as i32;
            arg_idx += crate::charset::skiptowhite_len(&arg_in[arg_idx..]);
        } else if arg_bytes.starts_with(b"file=") {
            arg_idx += 5;
            filename_off = Some(arg_idx);
            *buf = buflist_findname_exp(Some(crate::memory::bytes_as_cstr(&arg_in[arg_idx..])));
            break;
        } else if arg_bytes.starts_with(b"buffer=") {
            arg_idx += 7;
            filename_off = Some(arg_idx);
            let arg_bytes = crate::memory::bytes_until_nul(&arg_in[arg_idx..]);
            let mut rem = arg_bytes;
            *buf = buflist_findnr(getdigits_int(&mut rem, true, 0));
            arg_idx += arg_bytes.len() - rem.len();
            if !crate::charset::skipwhite_slice(crate::memory::bytes_until_nul(&arg_in[arg_idx..]))
                .is_empty()
            {
                semsg!(
                    gettext(E_TRAILING_ARG),
                    crate::memory::bytes_as_cstr(&arg_in[arg_idx..]),
                );
            }
            break;
        } else {
            emsg(gettext(E_INVARG));
            return false;
        }
        arg_idx += crate::charset::skipwhite_len(&arg_in[arg_idx..]);
    }

    if let Some(off) = name_off {
        *name = Some(crate::memory::bytes_as_cstr(&arg_in[off..]));
    }
    if let Some(off) = group_off {
        *group = Some(crate::memory::bytes_as_cstr(&arg_in[off..]));
    }

    if let Some(off) = filename_off
        && buf.is_none()
    {
        semsg!(
            gettext(E_INVALID_BUFFER_NAME_STR),
            crate::memory::bytes_as_cstr(&arg_in[off..]),
        );
        return false;
    }

    if filename_off.is_none() && ((cmd == SIGNCMD_PLACE && lnum_arg) || cmd == SIGNCMD_JUMP) {
        *buf = curwin_mut().w_buffer.get().opt_mut();
    }
    true
}

pub fn ex_sign(eap: &mut Exarg) {
    let arg_buf = eap.arg_cstr_buf_mut();
    let p_off = crate::charset::skiptowhite_len(arg_buf);
    let idx = sign_cmd_idx(&arg_buf[..p_off]);
    if idx == SIGNCMD_LAST {
        semsg!(
            gettext(c"E160: Unknown sign command: %s"),
            crate::memory::bytes_as_cstr(arg_buf),
        );
        return;
    }
    let mut arg_off = p_off + crate::charset::skipwhite_len(&arg_buf[p_off..]);

    if idx <= SIGNCMD_LIST {
        let first_b = arg_buf.get(arg_off).copied().unwrap_or(0);
        if idx == SIGNCMD_LIST && first_b == 0 {
            let signs = SIGN_STATE.with(|st| st.sign_map.borrow().clone());
            for sp in &signs {
                sign_list_defined(sp);
            }
        } else if first_b == 0 {
            emsg(gettext(c"E156: Missing sign name"));
        } else {
            let mut p_off = arg_off + crate::charset::skiptowhite_len(&arg_buf[arg_off..]);
            if arg_buf.get(p_off).copied().unwrap_or(0) != 0 {
                arg_buf[p_off] = 0;
                p_off += 1;
            }
            while arg_buf.get(arg_off).copied() == Some(b'0')
                && arg_buf.get(arg_off + 1).copied().unwrap_or(0) != 0
            {
                arg_off += 1;
            }

            if idx == SIGNCMD_DEFINE {
                let arg_cstr = crate::memory::cstring_until_nul(&arg_buf[arg_off..]);
                sign_define_cmd(arg_cstr.as_c_str(), &mut arg_buf[p_off..]);
            } else {
                let arg_cstr = crate::memory::bytes_as_cstr(&arg_buf[arg_off..]);
                if idx == SIGNCMD_LIST {
                    sign_list_by_name(arg_cstr);
                } else {
                    sign_undefine_by_name(arg_cstr);
                }
            }
        }
    } else {
        let mut id: i32 = -1;
        let mut lnum: Linenr = -1;
        let mut name: Option<&CStr> = None;
        let mut group: Option<&CStr> = None;
        let mut prio: i32 = -1;
        let mut buf: Option<&mut FileBuf> = None;

        if !parse_sign_cmd_args(
            idx,
            &mut arg_buf[arg_off..],
            &mut name,
            &mut id,
            &mut group,
            &mut prio,
            &mut buf,
            &mut lnum,
        ) {
            return;
        }

        if idx == SIGNCMD_PLACE {
            sign_place_cmd(buf, lnum, name, id, group, prio);
        } else if idx == SIGNCMD_UNPLACE {
            sign_unplace_cmd(buf, lnum, name, id, group);
        } else if idx == SIGNCMD_JUMP {
            sign_jump_cmd(buf, lnum, name, id, group);
        }
    }
}

fn sign_get_info_dict(sp: &Sign) -> crate::eval::typval::DictId {
    let d = tv_dict_alloc();

    tv_dict_add_str(&d, c"name", Some(sp.sn_name.as_c_str()));

    if let Some(ref icon) = sp.sn_icon {
        tv_dict_add_str(&d, c"icon", Some(icon.as_c_str()));
    }
    if sp.sn_text[0] != 0 {
        let mut buf = [0u8; SIGN_WIDTH * MAX_SCHAR_SIZE];
        describe_sign_text(&mut buf, &sp.sn_text);
        tv_dict_add_str(&d, c"text", Some(crate::memory::bytes_as_cstr(&buf)));
    }
    if sp.sn_priority > 0 {
        tv_dict_add_nr(&d, c"priority", i64::from(sp.sn_priority));
    }
    let args = [
        (c"linehl", sp.sn_line_hl),
        (c"texthl", sp.sn_text_hl),
        (c"culhl", sp.sn_cul_hl),
        (c"numhl", sp.sn_num_hl),
    ];
    for (name, hl) in args {
        if hl > 0 {
            if let Some(p) = get_highlight_name_ext(hl - 1, false) {
                tv_dict_add_str(&d, name, Some(&p));
            } else {
                tv_dict_add_str_len(&d, name, Some(b"NONE"));
            }
        }
    }
    d
}

fn sign_get_placed_info_dict(mark: MTKey) -> crate::eval::typval::DictId {
    let d = tv_dict_alloc();
    let sh = decor_find_sign(mt_decor(mark)).unwrap();

    tv_dict_add_str(&d, c"name", Some(sign_get_name(&sh)));
    tv_dict_add_nr(&d, c"id", i64::from(mark.id as i32));
    tv_dict_add_str(&d, c"group", Some(describe_ns(mark.ns as i32, c"")));
    tv_dict_add_nr(&d, c"lnum", i64::from(mark.pos.row + 1));
    tv_dict_add_nr(&d, c"priority", i64::from(sh.priority));
    d
}

pub fn get_buffer_signs(buf: &mut FileBuf) -> crate::eval::typval::ListId {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    let mut itr = MarkTreeIter::new();
    let b = &buf.b_marktree;
    marktree_itr_get(b, 0, 0, &mut itr);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if !mt_end(mark) && mt_decor_sign(mark) {
            tv_list_append_dict(&l, Some(&sign_get_placed_info_dict(mark)));
        }
        marktree_itr_next(b, &mut itr);
    }

    l
}

fn sign_get_placed_in_buf(
    buf: &mut FileBuf,
    lnum: Linenr,
    sign_id: i32,
    group: Option<&CStr>,
    retlist: &TvList,
) {
    let d = tv_dict_alloc();
    tv_list_append_dict(retlist, Some(&d));

    tv_dict_add_nr(&d, c"bufnr", i64::from(buf.b_fnum.get()));

    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    tv_dict_add_list(&d, c"signs", Some(&l));

    let ns = group_get_ns(group);
    if !buf_has_signs(buf) || ns < 0 {
        return;
    }

    let mut itr = MarkTreeIter::new();
    let mut signs: Vec<MTKey> = Vec::new();
    let b = &buf.b_marktree;
    marktree_itr_get(b, if lnum != 0 { lnum - 1 } else { 0 }, 0, &mut itr);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if lnum != 0 && mark.pos.row >= lnum {
            break;
        }
        if !mt_end(mark)
            && (ns == i64::from(u32::MAX) || ns == i64::from(mark.ns))
            && ((lnum == 0 && sign_id == 0)
                || (sign_id == 0 && lnum == mark.pos.row + 1)
                || (lnum == 0 && sign_id == mark.id as i32)
                || (lnum == mark.pos.row + 1 && sign_id == mark.id as i32))
            && mt_decor_sign(mark)
        {
            signs.push(mark);
        }
        marktree_itr_next(b, &mut itr);
    }

    if !signs.is_empty() {
        signs.sort_by(sign_row_cmp_slice);
        for mark in signs {
            tv_list_append_dict(&l, Some(&sign_get_placed_info_dict(mark)));
        }
    }
}

fn sign_get_placed(
    buf: Option<&mut FileBuf>,
    lnum: Linenr,
    id: i32,
    group: Option<&CStr>,
    retlist: &TvList,
) {
    if let Some(buf) = buf {
        sign_get_placed_in_buf(buf, lnum, id, group, retlist);
    } else {
        for cbuf in for_all_buffers_ptr() {
            if buf_has_signs(&*cbuf) {
                sign_get_placed_in_buf(&mut *cbuf, 0, id, group, retlist);
            }
        }
    }
}

pub fn free_signs() {
    SIGN_STATE.with(|st| st.sign_map.borrow_mut().clear());
}

fn get_nth_sign_name(idx: i32) -> Option<std::ffi::CString> {
    let idx_usize = usize::try_from(idx).ok()?;
    SIGN_STATE.with(|st| {
        st.sign_map
            .borrow()
            .get(idx_usize)
            .map(|sp| sp.sn_name.clone())
    })
}

fn get_nth_sign_group_name(idx: i32) -> Option<&'static CStr> {
    let idx_usize = usize::try_from(idx).ok()?;
    let ns = SIGN_STATE.with(|st| st.sign_ns.borrow().get(idx_usize).copied())?;
    Some(describe_ns(ns as NS, c""))
}

pub fn get_sign_name(xp: &mut ExpandCtx, idx: i32) -> Option<&CStr> {
    let i = usize::try_from(idx).ok()?;
    match SIGN_STATE.with(|st| st.expand_what.get()) {
        EXP_SUBCMD => CMDS.get(i).copied(),
        EXP_DEFINE => {
            const DEFINE_ARG: [&CStr; 7] = [
                c"culhl=",
                c"icon=",
                c"linehl=",
                c"numhl=",
                c"text=",
                c"texthl=",
                c"priority=",
            ];
            DEFINE_ARG.get(i).copied()
        }
        EXP_PLACE => {
            const PLACE_ARG: [&CStr; 6] = [
                c"line=",
                c"name=",
                c"group=",
                c"priority=",
                c"file=",
                c"buffer=",
            ];
            PLACE_ARG.get(i).copied()
        }
        EXP_LIST | EXP_UNPLACE => {
            const LIST_ARG: [&CStr; 3] = [c"group=", c"file=", c"buffer="];
            LIST_ARG.get(i).copied()
        }
        EXP_SIGN_NAMES => get_nth_sign_name(idx).map(|s| xp.set_item(s)),
        EXP_SIGN_GROUPS => get_nth_sign_group_name(idx),
        _ => None,
    }
}

pub fn set_context_in_sign_cmd(xp: &mut ExpandCtx, arg_cstr: &CStr) {
    let arg = crate::strings::OwnedSlice::from_cstr(arg_cstr);
    xp.xp_context = EXPAND_SIGN;
    SIGN_STATE.with(|st| st.expand_what.set(EXP_SUBCMD));
    xp.xp_pattern = arg.clone();

    let end_subcmd = arg.skiptowhite();
    if end_subcmd.is_empty() {
        return;
    }

    let cmd_idx = sign_cmd_idx(arg.slice(end_subcmd.offset_from(&arg)));
    let begin_subcmd_args = end_subcmd.skipwhite();

    let mut last;
    let mut p = begin_subcmd_args.clone();
    loop {
        p = p.skipwhite();
        last = p.clone();
        p = p.skiptowhite();
        if p.is_empty() {
            break;
        }
    }

    let Some(eq_cstr) = vim_strchr(last.as_cstr(), b'=' as i32) else {
        xp.xp_pattern = last;
        match cmd_idx {
            SIGNCMD_DEFINE => SIGN_STATE.with(|st| st.expand_what.set(EXP_DEFINE)),
            SIGNCMD_PLACE => {
                if ascii_isdigit(begin_subcmd_args.first_byte()) {
                    SIGN_STATE.with(|st| st.expand_what.set(EXP_PLACE));
                } else {
                    SIGN_STATE.with(|st| st.expand_what.set(EXP_LIST));
                }
            }
            SIGNCMD_LIST | SIGNCMD_UNDEFINE => {
                SIGN_STATE.with(|st| st.expand_what.set(EXP_SIGN_NAMES))
            }
            SIGNCMD_JUMP | SIGNCMD_UNPLACE => SIGN_STATE.with(|st| st.expand_what.set(EXP_UNPLACE)),
            _ => xp.xp_context = EXPAND_NOTHING,
        }
        return;
    };
    {
        xp.xp_pattern = last.advance_to_cstr(eq_cstr).wrapping_add(1);
        let last_bytes = last.as_bytes();
        match cmd_idx {
            SIGNCMD_DEFINE => {
                if last_bytes.starts_with(b"texthl")
                    || last_bytes.starts_with(b"linehl")
                    || last_bytes.starts_with(b"culhl")
                    || last_bytes.starts_with(b"numhl")
                {
                    xp.xp_context = EXPAND_HIGHLIGHT;
                } else if last_bytes.starts_with(b"icon") {
                    xp.xp_context = EXPAND_FILES;
                } else {
                    xp.xp_context = EXPAND_NOTHING;
                }
            }
            SIGNCMD_PLACE => {
                if last_bytes.starts_with(b"name") {
                    SIGN_STATE.with(|st| st.expand_what.set(EXP_SIGN_NAMES));
                } else if last_bytes.starts_with(b"group") {
                    SIGN_STATE.with(|st| st.expand_what.set(EXP_SIGN_GROUPS));
                } else if last_bytes.starts_with(b"file") {
                    xp.xp_context = EXPAND_BUFFERS;
                } else {
                    xp.xp_context = EXPAND_NOTHING;
                }
            }
            SIGNCMD_UNPLACE | SIGNCMD_JUMP => {
                if last_bytes.starts_with(b"group") {
                    SIGN_STATE.with(|st| st.expand_what.set(EXP_SIGN_GROUPS));
                } else if last_bytes.starts_with(b"file") {
                    xp.xp_context = EXPAND_BUFFERS;
                } else {
                    xp.xp_context = EXPAND_NOTHING;
                }
            }
            _ => xp.xp_context = EXPAND_NOTHING,
        }
    }
}

fn sign_define_from_dict(name: Option<&CStr>, dict: Option<&TvDict>) -> i32 {
    let name_owned: Option<std::ffi::CString>;
    let name_cstr = if let Some(n) = name {
        n
    } else {
        name_owned = crate::eval::typval::tv_dict_get_cstring(dict, c"name");
        let Some(ref dict_name) = name_owned else {
            return -1;
        };
        if dict_name.as_bytes().is_empty() {
            return -1;
        }
        dict_name.as_c_str()
    };

    let mut icon = None;
    let mut linehl = None;
    let mut text = None;
    let mut texthl = None;
    let mut culhl = None;
    let mut numhl = None;
    let mut prio: i32 = -1;

    if dict.is_some() {
        icon = crate::eval::typval::tv_dict_get_cstring(dict, c"icon");
        linehl = crate::eval::typval::tv_dict_get_cstring(dict, c"linehl");
        text = crate::eval::typval::tv_dict_get_cstring(dict, c"text");
        texthl = crate::eval::typval::tv_dict_get_cstring(dict, c"texthl");
        culhl = crate::eval::typval::tv_dict_get_cstring(dict, c"culhl");
        numhl = crate::eval::typval::tv_dict_get_cstring(dict, c"numhl");
        prio = tv_dict_get_number_def(dict, c"priority", -1) as i32;
    }

    if sign_define_by_name(
        name_cstr,
        icon.as_deref(),
        text.as_deref(),
        linehl.as_deref(),
        texthl.as_deref(),
        culhl.as_deref(),
        numhl.as_deref(),
        prio,
    ) {
        0
    } else {
        -1
    }
}

fn sign_define_multiple(l: Option<&TvList>, retlist: &TvList) {
    for li in tv_list_iter(l) {
        let mut retval: i32 = -1;
        let li_tv = li.tv();
        if li_tv.v_type == VarType::VarDict {
            retval = sign_define_from_dict(None, li_tv.vval.v_dict_opt());
        } else {
            emsg(gettext(E_DICTREQ));
        }
        tv_list_append_number(retlist, i64::from(retval));
    }
}

pub fn f_sign_define(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarList && argvars[1].v_type == VarType::VarUnknown {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        sign_define_multiple(argvars[0].vval.v_list_opt(), rettv.vval.v_list_ref());
        return;
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_opt_dict_arg(argvars, 1) {
        return;
    }

    let Some(name_cstr) = tv_get_string_chk(&argvars[0]) else {
        return;
    };
    let name_owned = name_cstr.into_owned();

    let d = if argvars[1].v_type == VarType::VarDict {
        argvars[1].vval.v_dict_opt()
    } else {
        None
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_define_from_dict(
        Some(&name_owned),
        d,
    )));
}

pub fn f_sign_getdefined(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, 0);

    if argvars[0].v_type == VarType::VarUnknown {
        let signs = SIGN_STATE.with(|st| st.sign_map.borrow().clone());
        for sp in &signs {
            tv_list_append_dict(rettv.vval.v_list_ref(), Some(&sign_get_info_dict(sp)));
        }
    } else if let Some(sp) = pmap_get_sign(tv_get_string(&argvars[0]).to_bytes()) {
        tv_list_append_dict(rettv.vval.v_list_ref(), Some(&sign_get_info_dict(&sp)));
    }
}

pub fn f_sign_getplaced(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = crate::globals::BufPtr::NULL;
    let mut lnum: Linenr = 0;
    let mut sign_id: i32 = 0;
    let mut group: Option<&CStr> = None;
    let group_cow;
    let mut notanum = false;

    tv_list_alloc_ret(rettv, 0);

    if argvars[0].v_type != VarType::VarUnknown {
        buf = crate::globals::BufPtr::from(get_buf_arg(&mut argvars[0]));
        if buf.is_null() {
            return;
        }

        if argvars[1].v_type != VarType::VarUnknown {
            if !tv_check_for_nonnull_dict_arg(argvars, 1) {
                return;
            }
            let dict = argvars[1].vval.v_dict_opt();
            if let Some(di_lnum) = tv_dict_find_opt(dict, c"lnum", -1) {
                lnum = tv_get_lnum(&di_lnum.tv());
                if lnum <= 0 {
                    return;
                }
            }
            if let Some(di_id) = tv_dict_find_opt(dict, c"id", -1) {
                sign_id = tv_get_number_chk(&di_id.tv(), Some(&mut notanum)) as i32;
                if notanum {
                    return;
                }
            }
            if let Some(di_group) = tv_dict_find_opt(dict, c"group", -1) {
                let di_tv = di_group.tv();
                let Some(g) = tv_get_string_chk(&di_tv) else {
                    return;
                };
                if !g.is_empty() {
                    group_cow = std::borrow::Cow::Owned(g.into_owned());
                    group = Some(&group_cow);
                }
            }
        }
    }

    sign_get_placed(buf.opt_mut(), lnum, sign_id, group, rettv.vval.v_list_ref());
}

pub fn f_sign_jump(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    let mut notanum = false;
    let id = tv_get_number_chk(&argvars[0], Some(&mut notanum)) as i32;
    if notanum {
        return;
    }
    if id <= 0 {
        emsg(gettext(E_INVARG));
        return;
    }

    let Some(group_cstr) = tv_get_string_chk(&argvars[1]) else {
        return;
    };
    let group_owned = (!group_cstr.is_empty()).then(|| group_cstr.into_owned());

    let Some(buf_ref) = get_buf_arg(&mut argvars[2]) else {
        return;
    };

    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_jump(
        id,
        group_owned.as_deref(),
        buf_ref,
    )));
}

fn sign_place_from_dict(
    id_tv: Option<&TypVal>,
    group_tv: Option<&TypVal>,
    name_tv: Option<&TypVal>,
    buf_tv: Option<&mut TypVal>,
    dict: Option<&TvDict>,
) -> i32 {
    let mut id: i32 = 0;
    let mut notanum = false;
    let id_di = if id_tv.is_none() {
        tv_dict_find_opt(dict, c"id", -1)
    } else {
        None
    };
    let id_di_tv = id_di.as_ref().map(|di| di.tv());
    let id_tv = id_tv.or(id_di_tv.as_deref());
    if let Some(id_tv) = id_tv {
        id = tv_get_number_chk(id_tv, Some(&mut notanum)) as i32;
        if notanum {
            return -1;
        }
        if id < 0 {
            emsg(gettext(E_INVARG));
            return -1;
        }
    }

    let mut group: Option<&CStr> = None;
    let group_cow;
    let group_di = if group_tv.is_none() {
        tv_dict_find_opt(dict, c"group", -1)
    } else {
        None
    };
    let group_di_tv = group_di.as_ref().map(|di| di.tv());
    let group_tv = group_tv.or(group_di_tv.as_deref());
    if let Some(group_tv) = group_tv {
        let Some(g) = tv_get_string_chk(group_tv) else {
            return -1;
        };
        if !g.is_empty() {
            group_cow = g;
            group = Some(&group_cow);
        }
    }

    let name_di = if name_tv.is_none() {
        tv_dict_find_opt(dict, c"name", -1)
    } else {
        None
    };
    let name_di_tv = name_di.as_ref().map(|di| di.tv());
    let name_tv = name_tv.or(name_di_tv.as_deref());
    let Some(name_tv) = name_tv else {
        return -1;
    };
    let Some(name_cstr) = tv_get_string_chk(name_tv) else {
        return -1;
    };

    let buf_di = if buf_tv.is_none() {
        tv_dict_find_opt(dict, c"buffer", -1)
    } else {
        None
    };
    let mut buf_di_tv = buf_di.as_ref().map(|di| di.tv_mut());
    let buf_tv = buf_tv.or(buf_di_tv.as_deref_mut());
    let Some(buf_tv) = buf_tv else {
        return -1;
    };
    let Some(buf_ref) = get_buf_arg(buf_tv) else {
        return -1;
    };

    let mut lnum: Linenr = 0;
    if let Some(di_lnum) = tv_dict_find_opt(dict, c"lnum", -1) {
        lnum = tv_get_lnum(&di_lnum.tv());
        if lnum <= 0 {
            emsg(gettext(E_INVARG));
            return -1;
        }
    }

    let mut prio: i32 = -1;
    if let Some(di_prio) = tv_dict_find_opt(dict, c"priority", -1) {
        prio = tv_get_number_chk(&di_prio.tv(), Some(&mut notanum)) as i32;
        if notanum {
            return -1;
        }
    }

    let mut uid = id as u32;
    if sign_place(&mut uid, group, &name_cstr, buf_ref, lnum, prio) {
        return uid as i32;
    }

    -1
}

pub fn f_sign_place(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut dict = crate::eval::typval::DictId::NULL;

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if argvars[4].v_type != VarType::VarUnknown {
        if !tv_check_for_nonnull_dict_arg(argvars, 4) {
            return;
        }
        dict = argvars[4].vval.v_dict();
    }

    let [arg0, arg1, arg2, arg3, ..] = argvars else {
        unreachable!()
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_place_from_dict(
        Some(arg0),
        Some(arg1),
        Some(arg2),
        Some(arg3),
        dict.opt(),
    )));
}

pub fn f_sign_placelist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

    if argvars[0].v_type != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return;
    }

    let l = argvars[0].vval.v_list_opt();
    for li in tv_list_iter(l) {
        let mut sign_id: i32 = -1;
        let li_tv = li.tv();
        if li_tv.v_type == VarType::VarDict {
            sign_id = sign_place_from_dict(None, None, None, None, li_tv.vval.v_dict_opt());
        } else {
            emsg(gettext(E_DICTREQ));
        }
        tv_list_append_number(rettv.vval.v_list_ref(), i64::from(sign_id));
    }
}

fn sign_undefine_multiple(l: Option<&TvList>, retlist: &TvList) {
    for li in tv_list_iter(l) {
        let mut retval: i32 = -1;
        if let Some(name) = tv_get_string_chk(&li.tv())
            && sign_undefine_by_name(&name)
        {
            retval = 0;
        }
        tv_list_append_number(retlist, i64::from(retval));
    }
}

pub fn f_sign_undefine(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarList && argvars[1].v_type == VarType::VarUnknown {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        sign_undefine_multiple(argvars[0].vval.v_list_opt(), rettv.vval.v_list_ref());
        return;
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if argvars[0].v_type == VarType::VarUnknown {
        free_signs();
        rettv.vval = crate::eval::typval::TypvalVval::Number(0);
    } else {
        let Some(name) = tv_get_string_chk(&argvars[0]) else {
            return;
        };
        if sign_undefine_by_name(&name) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    }
}

fn sign_unplace_from_dict(group_tv: Option<&TypVal>, dict: Option<&TvDict>) -> i32 {
    let mut id: i32 = 0;
    let mut buf = crate::globals::BufPtr::NULL;
    let group_cstr = if let Some(group_tv) = group_tv {
        Some(tv_get_string(group_tv))
    } else {
        tv_dict_get_string(dict, c"group")
    };
    let group = group_cstr.filter(|s| !s.to_bytes().is_empty());

    if dict.is_some() {
        if let Some(di_buf) = tv_dict_find_opt(dict, c"buffer", -1) {
            buf = crate::globals::BufPtr::from(get_buf_arg(&mut di_buf.tv_mut()));
            if buf.is_null() {
                return -1;
            }
        }
        if tv_dict_find(dict, c"id", -1).is_some() {
            id = tv_dict_get_number(dict, c"id") as i32;
            if id <= 0 {
                emsg(gettext(E_INVARG));
                return -1;
            }
        }
    }

    if sign_unplace(buf.opt_mut(), id, group.as_deref(), 0) {
        0
    } else {
        -1
    }
}

pub fn f_sign_unplace(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut dict: Option<&TvDict> = None;

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_string_arg(argvars, 0) || !tv_check_for_opt_dict_arg(argvars, 1) {
        return;
    }

    if argvars[1].v_type != VarType::VarUnknown {
        dict = argvars[1].vval.v_dict_opt();
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_unplace_from_dict(
        Some(&argvars[0]),
        dict,
    )));
}

pub fn f_sign_unplacelist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

    if argvars[0].v_type != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return;
    }

    let l = argvars[0].vval.v_list_opt();
    for li in tv_list_iter(l) {
        let mut retval: i32 = -1;
        let li_tv = li.tv();
        if li_tv.v_type == VarType::VarDict {
            retval = sign_unplace_from_dict(None, li_tv.vval.v_dict_opt());
        } else {
            emsg(gettext(E_DICTREQ));
        }
        tv_list_append_number(rettv.vval.v_list_ref(), i64::from(retval));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_cmd_idx_and_describe_sign_text_work() {
        let cmd = *b"define\0";
        assert_eq!(sign_cmd_idx(&cmd[..6]), SIGNCMD_DEFINE);
        let st = [schar_from_ascii(b'>'), schar_from_ascii(b'>')];
        let mut buf = [0u8; 16];
        let len = describe_sign_text(&mut buf, &st);
        assert_eq!(len, 2);
        assert_eq!(&buf[..len], b">>");
    }
}
