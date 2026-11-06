//! Functions for managing signs (`src/nvim/sign.c`).

use crate::api::extmark::{describe_ns, nvim_create_namespace};
use crate::api::private::helpers::cstr_as_string;

use crate::buffer::{FileBuf, buf_hide, buf_meta_total, buflist_findname_exp, buflist_findnr};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, backslash_halve, getdigits_int, vim_isprintc,
};
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
use std::ptr;

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
    sign_map: Vec<Sign>,
    sign_ns: Vec<Integer>,
    expand_what: i32,
}

impl SignModuleState {
    const fn new() -> Self {
        Self {
            sign_map: Vec::new(),
            sign_ns: Vec::new(),
            expand_what: EXP_SUBCMD,
        }
    }
}

static SIGN_STATE: crate::globals::GlobalState<SignModuleState> =
    crate::globals::GlobalState::new(SignModuleState::new());

#[inline]
fn sign_st() -> crate::globals::GraphPtr<SignModuleState> {
    SIGN_STATE.handle()
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
fn pmap_get_sign(name: &[u8]) -> Option<&'static mut Sign> {
    let name_bytes = crate::memory::bytes_until_nul(name);
    if name_bytes.is_empty() {
        return None;
    }
    sign_st()
        .borrow_mut()
        .sign_map
        .iter_mut()
        .find(|sp| sp.sn_name.as_bytes() == name_bytes)
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
    sp: &mut Sign,
) {
    if group.is_some() && find_ns_by_name(group) == 0 {
        let ns_vec = &mut sign_st().sign_ns;
        ns_vec.push(nvim_create_namespace(cstr_as_string(group)));
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
            vt: crate::globals::GraphPtr::NULL,
        }),
    };
    extmark_set(
        buf,
        ns,
        Some(&mut *id),
        buf.b_ml.ml_line_count.min(lnum) - 1,
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
    sp: &mut Sign,
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
    let rbuf_ptr = rbuf.map_or(ptr::null(), |b| &raw const *b);
    let mut buf = crate::globals::GraphPtr::<FileBuf>::of(if !rbuf_ptr.is_null() {
        rbuf_ptr
    } else {
        g_st().firstbuf.as_ptr()
    })
    .as_ptr()
    .cast_const();
    let ns = group_get_ns(group);

    msg_puts_title(gettext(c"\n--- Signs ---"));

    while !buf.is_null() && !g_st().got_int {
        let buf_ref = crate::globals::GraphPtr::of(buf).borrow();
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
                        format_to_buf!(&mut namebuf[..], gettext(c"  name=%s"), sign_get_name(sh));
                    }
                    if mark.ns != 0 {
                        format_to_buf!(
                            &mut groupbuf[..],
                            gettext(c"  group=%s"),
                            crate::strings::ByteSlice::from_ptr(
                                describe_ns(mark.ns as i32, c"").as_ptr()
                            ),
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
        buf = buf_ref.b_next.as_ptr();
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
                semsg!(
                    gettext(c"E239: Invalid sign text: %s"),
                    crate::strings::ByteSlice::from_ptr(
                        unescaped.as_ptr().cast::<std::ffi::c_char>()
                    )
                );
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
    let sp = if let Some(sp) = existing {
        sp
    } else {
        sign_st().sign_map.push(Sign {
            sn_name: name.to_owned(),
            sn_icon: None,
            sn_text: [0; SIGN_WIDTH],
            sn_line_hl: 0,
            sn_text_hl: 0,
            sn_cul_hl: 0,
            sn_num_hl: 0,
            sn_priority: 0,
        });
        sign_st().borrow_mut().sign_map.last_mut().unwrap()
    };

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
        if !init_sign_text(Some(&*sp), &mut sn_text, text_cstr.to_bytes()) {
            sp.sn_text = sn_text;
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
        for sh in g_st().decor_items.iter_mut() {
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
    }

    true
}

fn sign_undefine_by_name(name: &CStr) -> bool {
    let Some(pos) = sign_st()
        .sign_map
        .iter()
        .position(|sp| sp.sn_name.as_c_str() == name)
    else {
        semsg!(gettext(c"E155: Unknown sign: %s"), name);
        return false;
    };
    sign_st().sign_map.swap_remove(pos);
    true
}

fn sign_list_defined(sp: &mut Sign) {
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
            let p =
                get_highlight_name_ext(&mut crate::os::env::ExpandCtx::new(), hls[i] - 1, false);
            msg_puts(p.unwrap_or(c"NONE"));
        }
    }
}

fn sign_list_by_name(name: &CStr) {
    if let Some(sp) = pmap_get_sign(name.to_bytes()) {
        sign_list_defined(sp);
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
        buf_set_sign(buf, id, group, prio, lnum, sp);
    } else {
        lnum = buf_mod_sign(buf, &mut *id, group, prio, sp);
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
            buf.b_fnum,
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
    let mut cmdline = cmdline_in.as_mut_ptr().cast::<std::ffi::c_char>();
    let mut icon = ptr::null_mut();
    let mut text = ptr::null_mut();
    let mut linehl = ptr::null_mut();
    let mut texthl = ptr::null_mut();
    let mut culhl = ptr::null_mut();
    let mut numhl = ptr::null_mut();
    let mut prio: i32 = -1;

    loop {
        let arg = crate::charset::skipwhite_ptr(cmdline);
        if crate::strings::AsByteSlice::is_empty_val(arg) {
            break;
        }
        cmdline = crate::charset::skiptowhite_esc_ptr(arg);
        let arg_bytes = crate::strings::ByteSlice::from_ptr(arg).as_bytes();
        if arg_bytes.starts_with(b"icon=") {
            icon = arg.wrapping_add(5);
        } else if arg_bytes.starts_with(b"text=") {
            text = arg.wrapping_add(5);
        } else if arg_bytes.starts_with(b"linehl=") {
            linehl = arg.wrapping_add(7);
        } else if arg_bytes.starts_with(b"texthl=") {
            texthl = arg.wrapping_add(7);
        } else if arg_bytes.starts_with(b"culhl=") {
            culhl = arg.wrapping_add(6);
        } else if arg_bytes.starts_with(b"numhl=") {
            numhl = arg.wrapping_add(6);
        } else if let Some(rest) = arg_bytes.strip_prefix(b"priority=") {
            prio = crate::charset::parse_i64_prefix(rest) as i32;
        } else {
            semsg!(gettext(E_INVARG2), crate::strings::ByteSlice::from_ptr(arg));
            return;
        }
        if crate::strings::AsByteSlice::is_empty_val(cmdline) {
            break;
        }

        *crate::globals::GraphPtr::of(cmdline).borrow_mut() = NUL as _;

        cmdline = cmdline.wrapping_add(1);
    }

    sign_define_by_name(
        name,
        crate::strings::ByteSlice::from_ptr(icon).as_opt_cstr(),
        crate::strings::ByteSlice::from_ptr(text).as_opt_cstr(),
        crate::strings::ByteSlice::from_ptr(linehl).as_opt_cstr(),
        crate::strings::ByteSlice::from_ptr(texthl).as_opt_cstr(),
        crate::strings::ByteSlice::from_ptr(culhl).as_opt_cstr(),
        crate::strings::ByteSlice::from_ptr(numhl).as_opt_cstr(),
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
        buf = curwin_mut().w_buffer.opt_mut();
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
    let mut arg = arg_in.as_mut_ptr().cast::<std::ffi::c_char>();
    let arg1 = arg;
    let mut filename = ptr::null_mut();
    let mut lnum_arg = false;

    if ascii_isdigit(crate::strings::AsByteSlice::first_byte_val(arg)) {
        let arg_bytes = crate::strings::ByteSlice::from_ptr(arg).as_bytes();
        let mut rem = arg_bytes;
        *id = getdigits_int(&mut rem, true, 0);
        arg = arg.wrapping_add(arg_bytes.len() - rem.len());
        if !ascii_iswhite(crate::strings::AsByteSlice::first_byte_val(arg))
            && !crate::strings::AsByteSlice::is_empty_val(arg)
        {
            *id = -1;
            arg = arg1;
        } else {
            arg = crate::charset::skipwhite_ptr(arg);
        }
    }

    while !crate::strings::AsByteSlice::is_empty_val(arg) {
        let arg_bytes = crate::strings::ByteSlice::from_ptr(arg).as_bytes();
        if arg_bytes.starts_with(b"line=") {
            arg = arg.wrapping_add(5);
            *lnum = crate::charset::parse_i64_prefix(&arg_bytes[5..]) as i32;
            arg = crate::charset::skiptowhite_ptr(arg);
            lnum_arg = true;
        } else if arg_bytes.starts_with(b"*") && cmd == SIGNCMD_UNPLACE {
            if *id != -1 {
                emsg(gettext(E_INVARG));
                return false;
            }
            *id = -2;
            arg = crate::charset::skiptowhite_ptr(arg.wrapping_add(1));
        } else if arg_bytes.starts_with(b"name=") {
            arg = arg.wrapping_add(5);
            let mut namep = arg;
            arg = crate::charset::skiptowhite_ptr(arg);
            if !crate::strings::AsByteSlice::is_empty_val(arg) {
                *crate::globals::GraphPtr::of(arg).borrow_mut() = NUL as _;
                arg = arg.wrapping_add(1);
            }
            while *crate::globals::GraphPtr::of(namep).borrow() as u8 == b'0'
                && *crate::globals::GraphPtr::of(namep.wrapping_add(1)).borrow() != 0
            {
                namep = namep.wrapping_add(1);
            }
            *name = Some(crate::strings::ByteSlice::from_ptr(namep).as_cstr());
        } else if arg_bytes.starts_with(b"group=") {
            arg = arg.wrapping_add(6);
            let groupp = arg;
            arg = crate::charset::skiptowhite_ptr(arg);
            if !crate::strings::AsByteSlice::is_empty_val(arg) {
                *crate::globals::GraphPtr::of(arg).borrow_mut() = NUL as _;
                arg = arg.wrapping_add(1);
            }
            *group = Some(crate::strings::ByteSlice::from_ptr(groupp).as_cstr());
        } else if arg_bytes.starts_with(b"priority=") {
            arg = arg.wrapping_add(9);
            *prio = crate::charset::parse_i64_prefix(&arg_bytes[9..]) as i32;
            arg = crate::charset::skiptowhite_ptr(arg);
        } else if arg_bytes.starts_with(b"file=") {
            arg = arg.wrapping_add(5);
            filename = arg;
            *buf = buflist_findname_exp(crate::strings::ByteSlice::from_ptr(arg).as_opt_cstr());
            break;
        } else if arg_bytes.starts_with(b"buffer=") {
            arg = arg.wrapping_add(7);
            filename = arg;
            let arg_bytes = crate::strings::ByteSlice::from_ptr(arg).as_bytes();
            let mut rem = arg_bytes;
            *buf = buflist_findnr(getdigits_int(&mut rem, true, 0));
            arg = arg.wrapping_add(arg_bytes.len() - rem.len());
            if *crate::globals::GraphPtr::of(crate::charset::skipwhite_ptr(arg)).borrow() != 0 {
                semsg!(
                    gettext(E_TRAILING_ARG),
                    crate::strings::ByteSlice::from_ptr(arg)
                );
            }
            break;
        } else {
            emsg(gettext(E_INVARG));
            return false;
        }
        arg = crate::charset::skipwhite_ptr(arg);
    }

    if !filename.is_null() && buf.is_none() {
        semsg!(
            gettext(E_INVALID_BUFFER_NAME_STR),
            crate::strings::ByteSlice::from_ptr(filename)
        );
        return false;
    }

    if filename.is_null() && ((cmd == SIGNCMD_PLACE && lnum_arg) || cmd == SIGNCMD_JUMP) {
        *buf = curwin_mut().w_buffer.opt_mut();
    }
    true
}

pub fn ex_sign(eap: &mut Exarg) {
    let mut arg = eap.arg.as_ptr();

    let mut p = crate::charset::skiptowhite_ptr(arg);
    let idx = sign_cmd_idx(crate::strings::AsByteSlice::slice_len(
        arg,
        (p as usize).saturating_sub(arg as usize),
    ));
    if idx == SIGNCMD_LAST {
        semsg!(
            gettext(c"E160: Unknown sign command: %s"),
            crate::strings::ByteSlice::from_ptr(arg)
        );
        return;
    }
    arg = crate::charset::skipwhite_ptr(p);

    if idx <= SIGNCMD_LIST {
        if idx == SIGNCMD_LIST && crate::strings::AsByteSlice::is_empty_val(arg) {
            for sp in &mut sign_st().sign_map {
                sign_list_defined(sp);
            }
        } else if crate::strings::AsByteSlice::is_empty_val(arg) {
            emsg(gettext(c"E156: Missing sign name"));
        } else {
            p = crate::charset::skiptowhite_ptr(arg);
            if !crate::strings::AsByteSlice::is_empty_val(p) {
                *crate::globals::GraphPtr::of(p).borrow_mut() = NUL as _;

                p = p.wrapping_add(1);
            }
            while *crate::globals::GraphPtr::of(arg).borrow() as u8 == b'0'
                && *crate::globals::GraphPtr::of(arg.wrapping_add(1)).borrow() != 0
            {
                arg = arg.wrapping_add(1);
            }
            let arg_cstr = crate::strings::ByteSlice::from_ptr(arg).as_cstr();

            if idx == SIGNCMD_DEFINE {
                let p_len = crate::strings::ByteSlice::from_ptr(p).as_bytes().len();
                let p_slice = crate::globals::GraphPtr::raw_slice_mut(p.cast::<u8>(), p_len + 1);
                sign_define_cmd(arg_cstr, p_slice);
            } else if idx == SIGNCMD_LIST {
                sign_list_by_name(arg_cstr);
            } else {
                sign_undefine_by_name(arg_cstr);
            }
        }
    } else {
        let mut id: i32 = -1;
        let mut lnum: Linenr = -1;
        let mut name: Option<&CStr> = None;
        let mut group: Option<&CStr> = None;
        let mut prio: i32 = -1;
        let mut buf: Option<&mut FileBuf> = None;

        let arg_len = crate::strings::ByteSlice::from_ptr(arg).as_bytes().len();
        let arg_slice = crate::globals::GraphPtr::raw_slice_mut(arg.cast::<u8>(), arg_len + 1);
        if !parse_sign_cmd_args(
            idx, arg_slice, &mut name, &mut id, &mut group, &mut prio, &mut buf, &mut lnum,
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

fn sign_get_info_dict(sp: &mut Sign) -> &'static mut TvDict {
    let d = tv_dict_alloc();

    tv_dict_add_str(&mut *d, c"name", Some(sp.sn_name.as_c_str()));

    if let Some(ref icon) = sp.sn_icon {
        tv_dict_add_str(&mut *d, c"icon", Some(icon.as_c_str()));
    }
    if sp.sn_text[0] != 0 {
        let mut buf = [0u8; SIGN_WIDTH * MAX_SCHAR_SIZE];
        describe_sign_text(&mut buf, &sp.sn_text);
        tv_dict_add_str(&mut *d, c"text", Some(crate::memory::bytes_as_cstr(&buf)));
    }
    if sp.sn_priority > 0 {
        tv_dict_add_nr(&mut *d, c"priority", i64::from(sp.sn_priority));
    }
    let args = [
        (c"linehl", sp.sn_line_hl),
        (c"texthl", sp.sn_text_hl),
        (c"culhl", sp.sn_cul_hl),
        (c"numhl", sp.sn_num_hl),
    ];
    for (name, hl) in args {
        if hl > 0 {
            if let Some(p) =
                get_highlight_name_ext(&mut crate::os::env::ExpandCtx::new(), hl - 1, false)
            {
                tv_dict_add_str(&mut *d, name, Some(p));
            } else {
                tv_dict_add_str_len(&mut *d, name, Some(b"NONE"));
            }
        }
    }
    d
}

fn sign_get_placed_info_dict(mark: MTKey) -> &'static mut TvDict {
    let d = tv_dict_alloc();
    let sh = decor_find_sign(mt_decor(mark)).unwrap();

    tv_dict_add_str(&mut *d, c"name", Some(sign_get_name(sh)));
    tv_dict_add_nr(&mut *d, c"id", i64::from(mark.id as i32));
    tv_dict_add_str(&mut *d, c"group", Some(describe_ns(mark.ns as i32, c"")));
    tv_dict_add_nr(&mut *d, c"lnum", i64::from(mark.pos.row + 1));
    tv_dict_add_nr(&mut *d, c"priority", i64::from(sh.priority));
    d
}

pub fn get_buffer_signs(buf: &mut FileBuf) -> &'static mut TvList {
    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    let mut itr = MarkTreeIter::new();
    let b = &buf.b_marktree;
    marktree_itr_get(b, 0, 0, &mut itr);

    while itr.x.is_some() {
        let mark = marktree_itr_current(b, &itr);
        if !mt_end(mark) && mt_decor_sign(mark) {
            tv_list_append_dict(&mut *l, Some(sign_get_placed_info_dict(mark)));
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
    retlist: &mut TvList,
) {
    let d = tv_dict_alloc();
    tv_list_append_dict(&mut *retlist, Some(d));

    tv_dict_add_nr(&mut *d, c"bufnr", i64::from(buf.b_fnum));

    let l = tv_list_alloc(K_LIST_LEN_MAY_KNOW_CONST);
    tv_dict_add_list(&mut *d, c"signs", Some(l));

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
            tv_list_append_dict(&mut *l, Some(sign_get_placed_info_dict(mark)));
        }
    }
}

fn sign_get_placed(
    buf: Option<&mut FileBuf>,
    lnum: Linenr,
    id: i32,
    group: Option<&CStr>,
    retlist: &mut TvList,
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
    sign_st().sign_map = Vec::new();
}

fn get_nth_sign_name(idx: i32) -> Option<&'static CStr> {
    let idx_usize = usize::try_from(idx).ok()?;
    sign_st()
        .borrow_mut()
        .sign_map
        .as_slice()
        .get(idx_usize)
        .map(|sp| sp.sn_name.as_c_str())
}

fn get_nth_sign_group_name(idx: i32) -> Option<&'static CStr> {
    let idx_usize = usize::try_from(idx).ok()?;
    let ns_slice = sign_st().borrow_mut().sign_ns.as_slice();
    ns_slice
        .get(idx_usize)
        .map(|&ns| describe_ns(ns as NS, c""))
}

pub fn get_sign_name(_xp: &mut ExpandCtx, idx: i32) -> Option<&'static CStr> {
    let i = usize::try_from(idx).ok()?;
    match sign_st().expand_what {
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
        EXP_SIGN_NAMES => get_nth_sign_name(idx),
        EXP_SIGN_GROUPS => get_nth_sign_group_name(idx),
        _ => None,
    }
}

pub fn set_context_in_sign_cmd(xp: &mut ExpandCtx, arg_cstr: &CStr) {
    let arg = crate::strings::ByteSlice::from_cstr(arg_cstr);
    xp.xp_context = EXPAND_SIGN;
    sign_st().expand_what = EXP_SUBCMD;
    xp.xp_pattern = arg;

    let end_subcmd = arg.skiptowhite();
    if end_subcmd.is_empty() {
        return;
    }

    let cmd_idx = sign_cmd_idx(arg.slice(end_subcmd.offset_from(arg)));
    let begin_subcmd_args = end_subcmd.skipwhite();

    let mut last;
    let mut p = begin_subcmd_args;
    loop {
        p = p.skipwhite();
        last = p;
        p = p.skiptowhite();
        if p.is_empty() {
            break;
        }
    }

    let Some(eq_cstr) = vim_strchr(last.as_cstr(), b'=' as i32) else {
        xp.xp_pattern = last;
        match cmd_idx {
            SIGNCMD_DEFINE => sign_st().expand_what = EXP_DEFINE,
            SIGNCMD_PLACE => {
                if ascii_isdigit(begin_subcmd_args.first_byte()) {
                    sign_st().expand_what = EXP_PLACE;
                } else {
                    sign_st().expand_what = EXP_LIST;
                }
            }
            SIGNCMD_LIST | SIGNCMD_UNDEFINE => sign_st().expand_what = EXP_SIGN_NAMES,
            SIGNCMD_JUMP | SIGNCMD_UNPLACE => sign_st().expand_what = EXP_UNPLACE,
            _ => xp.xp_context = EXPAND_NOTHING,
        }
        return;
    };
    {
        xp.xp_pattern = crate::strings::ByteSlice::from_cstr(eq_cstr).wrapping_add(1);
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
                    sign_st().expand_what = EXP_SIGN_NAMES;
                } else if last_bytes.starts_with(b"group") {
                    sign_st().expand_what = EXP_SIGN_GROUPS;
                } else if last_bytes.starts_with(b"file") {
                    xp.xp_context = EXPAND_BUFFERS;
                } else {
                    xp.xp_context = EXPAND_NOTHING;
                }
            }
            SIGNCMD_UNPLACE | SIGNCMD_JUMP => {
                if last_bytes.starts_with(b"group") {
                    sign_st().expand_what = EXP_SIGN_GROUPS;
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

fn sign_define_multiple(l: Option<&TvList>, retlist: &mut TvList) {
    for li in tv_list_iter(l) {
        let mut retval: i32 = -1;
        if li.li_tv.v_type == VarType::VarDict {
            retval = sign_define_from_dict(None, li.li_tv.vval.v_dict_opt().as_deref());
        } else {
            emsg(gettext(E_DICTREQ));
        }
        tv_list_append_number(retlist, i64::from(retval));
    }
}

pub fn f_sign_define(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarList && argvars[1].v_type == VarType::VarUnknown {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        sign_define_multiple(
            argvars[0].vval.v_list_opt().as_deref(),
            rettv.vval.v_list_ref(),
        );
        return;
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_opt_dict_arg(argvars, 1) {
        return;
    }

    let Some(name_cstr) = tv_get_string_chk(&argvars[0]) else {
        return;
    };
    let name_owned = name_cstr.to_owned();

    let d = if argvars[1].v_type == VarType::VarDict {
        argvars[1].vval.v_dict_opt().map(|r| &*r)
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
        for sp in &mut sign_st().sign_map {
            tv_list_append_dict(rettv.vval.v_list_ref(), Some(sign_get_info_dict(sp)));
        }
    } else if let Some(sp) = pmap_get_sign(tv_get_string(&argvars[0]).to_bytes()) {
        tv_list_append_dict(rettv.vval.v_list_ref(), Some(sign_get_info_dict(sp)));
    }
}

pub fn f_sign_getplaced(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut buf = crate::globals::GraphPtr::<FileBuf>::NULL;
    let mut lnum: Linenr = 0;
    let mut sign_id: i32 = 0;
    let mut group: Option<&CStr> = None;
    let mut notanum = false;

    tv_list_alloc_ret(rettv, 0);

    if argvars[0].v_type != VarType::VarUnknown {
        buf = crate::globals::GraphPtr::from(get_buf_arg(&mut argvars[0]));
        if buf.is_null() {
            return;
        }

        if argvars[1].v_type != VarType::VarUnknown {
            if !tv_check_for_nonnull_dict_arg(argvars, 1) {
                return;
            }
            let dict = argvars[1].vval.v_dict_opt();
            if let Some(di_lnum) = tv_dict_find_opt(dict.as_deref(), c"lnum", -1) {
                lnum = tv_get_lnum(&di_lnum.di_tv);
                if lnum <= 0 {
                    return;
                }
            }
            if let Some(di_id) = tv_dict_find_opt(dict.as_deref(), c"id", -1) {
                sign_id = tv_get_number_chk(&di_id.di_tv, Some(&mut notanum)) as i32;
                if notanum {
                    return;
                }
            }
            if let Some(di_group) = tv_dict_find_opt(dict.as_deref(), c"group", -1) {
                let Some(g) = tv_get_string_chk(&di_group.di_tv) else {
                    return;
                };
                if !g.is_empty() {
                    group = Some(g);
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
    let group_owned = (!group_cstr.is_empty()).then(|| group_cstr.to_owned());

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
    mut id_tv: Option<&TypVal>,
    mut group_tv: Option<&TypVal>,
    mut name_tv: Option<&TypVal>,
    mut buf_tv: Option<&mut TypVal>,
    dict: Option<&TvDict>,
) -> i32 {
    let mut id: i32 = 0;
    let mut notanum = false;
    if id_tv.is_none()
        && let Some(di) = tv_dict_find_opt(dict, c"id", -1)
    {
        id_tv = Some(&di.di_tv);
    }
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
    if group_tv.is_none()
        && let Some(di) = tv_dict_find_opt(dict, c"group", -1)
    {
        group_tv = Some(&di.di_tv);
    }
    if let Some(group_tv) = group_tv {
        let Some(g) = tv_get_string_chk(group_tv) else {
            return -1;
        };
        if !g.is_empty() {
            group = Some(g);
        }
    }

    if name_tv.is_none()
        && let Some(di) = tv_dict_find_opt(dict, c"name", -1)
    {
        name_tv = Some(&di.di_tv);
    }
    let Some(name_tv) = name_tv else {
        return -1;
    };
    let Some(name_cstr) = tv_get_string_chk(name_tv) else {
        return -1;
    };

    if buf_tv.is_none()
        && let Some(di) = tv_dict_find_opt(dict, c"buffer", -1)
    {
        buf_tv = Some(&mut di.di_tv);
    }
    let Some(buf_tv) = buf_tv else {
        return -1;
    };
    let Some(buf_ref) = get_buf_arg(buf_tv) else {
        return -1;
    };

    let mut lnum: Linenr = 0;
    if let Some(di_lnum) = tv_dict_find_opt(dict, c"lnum", -1) {
        lnum = tv_get_lnum(&di_lnum.di_tv);
        if lnum <= 0 {
            emsg(gettext(E_INVARG));
            return -1;
        }
    }

    let mut prio: i32 = -1;
    if let Some(di_prio) = tv_dict_find_opt(dict, c"priority", -1) {
        prio = tv_get_number_chk(&di_prio.di_tv, Some(&mut notanum)) as i32;
        if notanum {
            return -1;
        }
    }

    let mut uid = id as u32;
    if sign_place(&mut uid, group, name_cstr, buf_ref, lnum, prio) {
        return uid as i32;
    }

    -1
}

pub fn f_sign_place(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    let mut dict: Option<&TvDict> = None;

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if argvars[4].v_type != VarType::VarUnknown {
        if !tv_check_for_nonnull_dict_arg(argvars, 4) {
            return;
        }
        dict = argvars[4].vval.v_dict_opt().map(|r| &*r);
    }

    let [arg0, arg1, arg2, arg3, ..] = argvars else {
        unreachable!()
    };
    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_place_from_dict(
        Some(arg0),
        Some(arg1),
        Some(arg2),
        Some(arg3),
        dict,
    )));
}

pub fn f_sign_placelist(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);

    if argvars[0].v_type != VarType::VarList {
        emsg(gettext(E_LISTREQ));
        return;
    }

    let l = argvars[0].vval.v_list_opt();
    for li in tv_list_iter(l.as_deref()) {
        let mut sign_id: i32 = -1;
        if li.li_tv.v_type == VarType::VarDict {
            sign_id = sign_place_from_dict(
                None,
                None,
                None,
                None,
                li.li_tv.vval.v_dict_opt().as_deref(),
            );
        } else {
            emsg(gettext(E_DICTREQ));
        }
        tv_list_append_number(rettv.vval.v_list_ref(), i64::from(sign_id));
    }
}

fn sign_undefine_multiple(l: Option<&TvList>, retlist: &mut TvList) {
    for li in tv_list_iter(l) {
        let mut retval: i32 = -1;
        if let Some(name) = tv_get_string_chk(&li.li_tv)
            && sign_undefine_by_name(name)
        {
            retval = 0;
        }
        tv_list_append_number(retlist, i64::from(retval));
    }
}

pub fn f_sign_undefine(argvars: &mut [TypVal], rettv: &mut TypVal, _fptr: EvalFuncData) {
    if argvars[0].v_type == VarType::VarList && argvars[1].v_type == VarType::VarUnknown {
        tv_list_alloc_ret(rettv, K_LIST_LEN_MAY_KNOW_CONST);
        sign_undefine_multiple(
            argvars[0].vval.v_list_opt().as_deref(),
            rettv.vval.v_list_ref(),
        );
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
        if sign_undefine_by_name(name) {
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        }
    }
}

fn sign_unplace_from_dict(group_tv: Option<&TypVal>, dict: Option<&TvDict>) -> i32 {
    let mut id: i32 = 0;
    let mut buf = crate::globals::GraphPtr::<FileBuf>::NULL;
    let group_cstr = if let Some(group_tv) = group_tv {
        Some(tv_get_string(group_tv))
    } else {
        tv_dict_get_string(dict, c"group")
    };
    let group = group_cstr.filter(|s| !s.to_bytes().is_empty());

    if dict.is_some() {
        if let Some(di_buf) = tv_dict_find_opt(dict, c"buffer", -1) {
            buf = crate::globals::GraphPtr::from(get_buf_arg(&mut di_buf.di_tv));
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

    if sign_unplace(buf.opt_mut(), id, group, 0) {
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
        dict = argvars[1].vval.v_dict_opt().map(|r| &*r);
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
    for li in tv_list_iter(l.as_deref()) {
        let mut retval: i32 = -1;
        if li.li_tv.v_type == VarType::VarDict {
            retval = sign_unplace_from_dict(None, li.li_tv.vval.v_dict_opt().as_deref());
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
