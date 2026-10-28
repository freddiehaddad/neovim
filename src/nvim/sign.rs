//! Functions for managing signs (`src/nvim/sign.c`).

use crate::api::extmark::{describe_ns, nvim_create_namespace};
use crate::api::private::helpers::cstr_as_string;

use crate::buffer::{buf_T, buf_hide, buf_meta_total, buflist_findname_exp, buflist_findnr};
use crate::charset::{
    NUL, ascii_isdigit, ascii_iswhite, backslash_halve, getdigits_int, skiptowhite,
    skiptowhite_esc, skipwhite, vim_isprintc,
};
use crate::cursor::{check_cursor_lnum, linenr_T};
use crate::decoration::{
    DECOR_SIGN_HIGHLIGHT_INIT, DecorInline, DecorPriority, DecorSignHighlight, NS, SIGN_WIDTH,
    SignItem, decor_find_sign, decor_put_sh, kSHIsSign, sign_item_cmp,
};
use crate::drawscreen::{UPD_NOT_VALID, redraw_buf_later};
use crate::errors::*;
use crate::eval::funcs::get_buf_arg;
use crate::eval::typval::{
    EvalFuncData, VarType, dict_T, list_T, tv_check_for_nonnull_dict_arg,
    tv_check_for_opt_dict_arg, tv_check_for_string_arg, tv_dict_add_list, tv_dict_add_nr,
    tv_dict_add_str, tv_dict_add_str_len, tv_dict_alloc, tv_dict_find, tv_dict_get_number,
    tv_dict_get_number_def, tv_dict_get_string, tv_get_lnum, tv_get_number_chk, tv_get_string,
    tv_get_string_chk, tv_list_alloc, tv_list_alloc_ret, tv_list_append_dict,
    tv_list_append_number, typval_T,
};
use crate::ex_cmds::{BL_WHITE, ECMD_HIDE, do_ecmd};
use crate::ex_docmd::exarg_T;
use crate::extmark::{extmark_del, extmark_del_id, extmark_set};
use crate::fold::foldOpenCursor;
use crate::globals::*;
use crate::grid::{schar_T, schar_from_ascii, schar_from_cluster, schar_get};
use crate::highlight_group::{get_highlight_name_ext, syn_check_group};
use crate::insert::beginline;
use crate::mark::setpcmark;
use crate::marktree::{
    DecorExt, DecorInlineData, MT_FLAG_DECOR_SIGNHL, MT_FLAG_DECOR_SIGNTEXT, MTKey, MTPair,
    MarkTreeIter, kMTMetaSignHL, kMTMetaSignText, marktree_itr_current, marktree_itr_get,
    marktree_itr_get_overlap, marktree_itr_next, marktree_itr_step_overlap, marktree_lookup_ns,
    mt_decor, mt_decor_sign, mt_end,
};
use crate::mbyte::{utf_ClusterInfo, utf_ptr2StrCharInfo};
use crate::message::{
    emsg, msg_outtrans, msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, semsg, smsg,
};
use crate::msgpack_rpc::packer::Integer;
use crate::strings::{gettext, vim_snprintf, vim_strchr};
use crate::window::buf_jump_open_win;
use std::cmp::Ordering;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

const SIGN_DEF_PRIO: c_int = 10;

const SIGNCMD_DEFINE: c_int = 0;
const SIGNCMD_UNDEFINE: c_int = 1;
const SIGNCMD_LIST: c_int = 2;
const SIGNCMD_PLACE: c_int = 3;
const SIGNCMD_UNPLACE: c_int = 4;
const SIGNCMD_JUMP: c_int = 5;
const SIGNCMD_LAST: c_int = 6;

pub(crate) const EXP_SUBCMD: c_int = 0;
const EXP_DEFINE: c_int = 1;
const EXP_PLACE: c_int = 2;
const EXP_LIST: c_int = 3;
const EXP_UNPLACE: c_int = 4;
const EXP_SIGN_NAMES: c_int = 5;
const EXP_SIGN_GROUPS: c_int = 6;

#[repr(C)]
pub struct sign_T {
    pub sn_name: std::ffi::CString,
    pub sn_icon: Option<std::ffi::CString>,
    pub sn_text: [schar_T; SIGN_WIDTH],
    pub sn_line_hl: c_int,
    pub sn_text_hl: c_int,
    pub sn_cul_hl: c_int,
    pub sn_num_hl: c_int,
    pub sn_priority: c_int,
}

use crate::api::ui::MAX_SCHAR_SIZE;
use crate::cmdexpand::{
    EXPAND_BUFFERS, EXPAND_FILES, EXPAND_HIGHLIGHT, EXPAND_NOTHING, EXPAND_SIGN, HLF_D,
};
use crate::eval::kListLenMayKnow;
use crate::eval::userfunc::MSG_BUF_LEN;
use crate::os::env::expand_T;

struct SignModuleState {
    sign_map: Vec<sign_T>,
    sign_ns: Vec<Integer>,
    expand_what: c_int,
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

static SIGN_STATE: crate::globals::GlobalState<SignModuleState> = crate::globals::GlobalState::new(SignModuleState::new());

#[inline]
fn sign_st() -> &'static mut SignModuleState {
    unsafe { &mut *SIGN_STATE.as_ptr() }
}

const CMDS: [*const c_char; 7] = [
    c"define".as_ptr(),
    c"undefine".as_ptr(),
    c"list".as_ptr(),
    c"place".as_ptr(),
    c"unplace".as_ptr(),
    c"jump".as_ptr(),
    ptr::null(),
];

#[inline]
fn pmap_get_sign(name: *const c_char) -> *mut sign_T {
    if name.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let sm_ptr = &raw mut sign_st().sign_map;
        for sp in &mut *sm_ptr {
            if crate::memory::cstr_as_bytes(sp.sn_name.as_ptr()) == crate::memory::cstr_as_bytes(name) {
                return sp;
            }
        }
        ptr::null_mut()
    }
}

#[inline]
fn find_ns_by_name(group: *const c_char) -> c_int {
    if group.is_null() {
        return 0;
    }
    unsafe {
        let group_cstr = CStr::from_ptr(group);
        let ns_ptr = &raw const g_st().namespace_ids;
        (*ns_ptr)
            .iter()
            .find_map(|(k, v)| (k.as_c_str() == group_cstr).then_some(*v))
            .unwrap_or(0)
    }
}

fn group_get_ns(group: *const c_char) -> i64 {
     {
        if group.is_null() {
            return 0;
        } else if crate::memory::cstr_as_bytes(group) == b"*" {
            return i64::from(u32::MAX);
        }
        let ns = find_ns_by_name(group);
        if ns != 0 { i64::from(ns) } else { -1 }
    }
}

fn sign_get_name(sh: &DecorSignHighlight) -> *const c_char {
    let Some(ref name) = sh.sign_name else {
            return c"".as_ptr();
        };
        if !pmap_get_sign(name.as_ptr()).is_null() {
            name.as_ptr()
        } else {
            c"[Deleted]".as_ptr()
        }
}

fn buf_set_sign(
    buf: &mut buf_T,
    id: &mut u32,
    group: *mut c_char,
    prio: c_int,
    lnum: linenr_T,
    sp: &mut sign_T,
) {
    unsafe {
        if !group.is_null() && find_ns_by_name(group) == 0 {
            let ns_vec_ptr = &raw mut sign_st().sign_ns;
            let ns_vec = &mut *ns_vec_ptr;
            ns_vec.push(nvim_create_namespace(cstr_as_string(group)));
        }

        let ns = if !group.is_null() {
            nvim_create_namespace(cstr_as_string(group)) as u32
        } else {
            0
        };
        let mut sign = DECOR_SIGN_HIGHLIGHT_INIT;

        sign.flags |= kSHIsSign;
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
                vt: ptr::null_mut(),
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
}

fn buf_mod_sign(
    buf: &mut buf_T,
    id: &mut u32,
    group: *mut c_char,
    prio: c_int,
    sp: &mut sign_T,
) -> linenr_T {
     {
        let ns = group_get_ns(group);
        if ns < 0 || (!group.is_null() && ns == 0) {
            return 0;
        }

        let mark = marktree_lookup_ns(&buf.b_marktree, ns as u32, *id, false, None);
        if mark.pos.row >= 0 {
            buf_set_sign( buf, id, group, prio, mark.pos.row + 1,  sp);
        }
        mark.pos.row + 1
    }
}

fn buf_findsign(buf: &mut buf_T, id: c_int, group: *mut c_char) -> c_int {
     {
        let ns = group_get_ns(group);
        if ns < 0 || (!group.is_null() && ns == 0) {
            return 0;
        }
        marktree_lookup_ns(&buf.b_marktree, ns as u32, id as u32, false, None)
            .pos
            .row
            + 1
    }
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

fn buf_delete_signs(
    buf: &mut buf_T,
    group: *mut c_char,
    id: c_int,
    atlnum: linenr_T,
) -> bool {
     {
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
                && (id == 0 || mark.id as c_int == id)
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
}

pub fn buf_has_signs(buf: &buf_T) -> bool {
     buf_meta_total(buf, kMTMetaSignHL) + buf_meta_total(buf, kMTMetaSignText) != 0
}

fn sign_list_placed(rbuf: *mut buf_T, group: *mut c_char) {
    unsafe {
        let mut lbuf = [0 as c_char; MSG_BUF_LEN];
        let mut namebuf = [0 as c_char; MSG_BUF_LEN];
        let mut groupbuf = [0 as c_char; MSG_BUF_LEN];
        let mut buf = if !rbuf.is_null() { rbuf } else { g_st().firstbuf };
        let ns = group_get_ns(group);

        msg_puts_title(gettext(c"\n--- Signs ---"));

        while !buf.is_null() && !g_st().got_int {
            if buf_has_signs( &*buf) {
                msg_putchar(b'\n' as c_int);
                let fname = if !(*buf).b_fname.is_null() {
                    (*buf).b_fname.cast_const()
                } else {
                    gettext(c"Untitled").cast_const()
                };
                vim_snprintf!(
                    lbuf.as_mut_ptr(),
                    MSG_BUF_LEN,
                    gettext(c"Signs for %s:"),
                    fname,
                );
                msg_puts_hl(lbuf.as_ptr(), HLF_D, false);
            }

            if ns >= 0 {
                let mut itr = MarkTreeIter::new();
                let mut signs: Vec<MTKey> = Vec::new();
                let b = &(*buf).b_marktree;
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
                    msg_putchar(b'\n' as c_int);

                    for (i, &mark) in signs.iter().enumerate() {
                        namebuf[0] = NUL;
                        groupbuf[0] = NUL;

                        let sh = decor_find_sign(mt_decor(mark)).unwrap();
                        if sh.sign_name.is_some() {
                            vim_snprintf!(
                                namebuf.as_mut_ptr(),
                                MSG_BUF_LEN,
                                gettext(c"  name=%s"),
                                sign_get_name(sh),
                            );
                        }
                        if mark.ns != 0 {
                            vim_snprintf!(
                                groupbuf.as_mut_ptr(),
                                MSG_BUF_LEN,
                                gettext(c"  group=%s"),
                                describe_ns(mark.ns as c_int, c""),
                            );
                        }
                        vim_snprintf!(
                            lbuf.as_mut_ptr(),
                            MSG_BUF_LEN,
                            gettext(c"    line=%d  id=%u%s%s  priority=%d"),
                            mark.pos.row + 1,
                            mark.id,
                            groupbuf.as_ptr(),
                            namebuf.as_ptr(),
                            c_int::from(sh.priority),
                        );
                        msg_puts(lbuf.as_ptr());
                        if i < signs.len() - 1 {
                            msg_putchar(b'\n' as c_int);
                        }
                    }
                }
            }

            if !rbuf.is_null() {
                return;
            }
            buf = (*buf).b_next;
        }
    }
}

fn sign_cmd_idx(begin_cmd: *mut c_char, end_cmd: *mut c_char) -> c_int {
    unsafe {
        let save = *end_cmd;
        *end_cmd = NUL;
        let mut idx: usize = 0;
        loop {
            let cmd = CMDS[idx];
            if cmd.is_null() || crate::memory::cstr_as_bytes(begin_cmd) == crate::memory::cstr_as_bytes(cmd) {
                break;
            }
            idx += 1;
        }
        *end_cmd = save;
        idx as c_int
    }
}

pub fn describe_sign_text(buf: *mut c_char, sign_text: *mut schar_T) -> usize {
    unsafe {
        let mut p: usize = 0;
        for i in 0..SIGN_WIDTH {
            schar_get(buf.add(p), *sign_text.add(i));
            let len = crate::memory::cstr_len(buf.add(p));
            if len == 0 {
                break;
            }
            p += len;
        }
        p
    }
}

pub fn init_sign_text(sp: Option<&sign_T>, sign_text: *mut schar_T, text: *mut c_char) -> bool {
    unsafe {
        let mut endp = text.add(crate::memory::cstr_len(text));
        let mut s = if sp.is_some() { text } else { endp };
        while s.add(1) < endp {
            if *s == b'\\' as c_char {
                let tail_len = crate::memory::cstr_len(s.add(1)) + 1;
                ptr::copy(s.add(1), s, tail_len);
                endp = endp.sub(1);
            }
            s = s.add(1);
        }

        let mut cells: usize = 0;
        let mut ci = utf_ptr2StrCharInfo(text);
        while *ci.ptr != NUL {
            let cli = utf_ClusterInfo(ci);
            if !vim_isprintc(ci.chr.value) || cells + (cli.cells as usize) > SIGN_WIDTH {
                if sp.is_some() {
                    semsg!(gettext(c"E239: Invalid sign text: %s"), text);
                }
                return false;
            }
            *sign_text.add(cells) = schar_from_cluster(ci, cli);
            debug_assert!(cli.cells <= 2);
            if cli.cells == 2 {
                *sign_text.add(cells + 1) = 0;
            }
            cells += cli.cells as usize;
            ci = cli.next;
        }

        if cells < 1 {
            *sign_text = 0;
        } else if cells == 1 {
            *sign_text.add(1) = schar_from_ascii(b' ');
        }

        true
    }
}

fn sign_define_by_name(
    name: *mut c_char,
    icon: *mut c_char,
    text: *mut c_char,
    linehl: *mut c_char,
    texthl: *mut c_char,
    culhl: *mut c_char,
    numhl: *mut c_char,
    prio: c_int,
) -> bool {
    unsafe {
        let existing = pmap_get_sign(name);
        let new_sign = existing.is_null();
        let sp = if new_sign {
            let sm_ptr = &raw mut sign_st().sign_map;
            (*sm_ptr).push(sign_T {
                sn_name: crate::memory::cstr_to_cstring(name),
                sn_icon: None,
                sn_text: [0; SIGN_WIDTH],
                sn_line_hl: 0,
                sn_text_hl: 0,
                sn_cul_hl: 0,
                sn_num_hl: 0,
                sn_priority: 0,
            });
            (*sm_ptr).last_mut().unwrap() as *mut sign_T
        } else {
            existing
        };

        if !icon.is_null() {
            let mut icon_vec = crate::memory::cstr_as_bytes_with_nul(icon).to_vec();
            backslash_halve(icon_vec.as_mut_ptr().cast());
            let end = icon_vec
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(icon_vec.len());
            icon_vec.truncate(end);
            (*sp).sn_icon = Some(std::ffi::CString::new(icon_vec).unwrap());
        }

        if !text.is_null() && !init_sign_text(sp.as_ref(), (*sp).sn_text.as_mut_ptr(), text) {
            return false;
        }

        (*sp).sn_priority = prio;

        let args = [linehl, texthl, culhl, numhl];
        let hls = [
            &raw mut (*sp).sn_line_hl,
            &raw mut (*sp).sn_text_hl,
            &raw mut (*sp).sn_cul_hl,
            &raw mut (*sp).sn_num_hl,
        ];
        for i in 0..4 {
            if !args[i].is_null() {
                *hls[i] = if *args[i] != NUL {
                    syn_check_group(args[i], crate::memory::cstr_len(args[i]))
                } else {
                    0
                };
            }
        }

        if !new_sign {
            let mut did_redraw = false;
            let items = &raw mut g_st().decor_items;
            for sh in (*items).iter_mut() {
                if sh
                    .sign_name
                    .as_ref()
                    .is_some_and(|sn| crate::memory::cstr_as_bytes(sn.as_ptr()) == crate::memory::cstr_as_bytes(name))
                {
                    sh.text = (*sp).sn_text;
                    sh.hl_id = (*sp).sn_text_hl;
                    sh.line_hl_id = (*sp).sn_line_hl;
                    sh.number_hl_id = (*sp).sn_num_hl;
                    sh.cursorline_hl_id = (*sp).sn_cul_hl;
                    if !did_redraw {
                        for wp in for_all_windows_mut() {
                            if buf_has_signs(wp.buf_mut()) {
                                redraw_buf_later(wp.w_buffer, UPD_NOT_VALID);
                            }
                        }
                        did_redraw = true;
                    }
                }
            }
        }
        true
    }
}

fn sign_undefine_by_name(name: *const c_char) -> bool {
    unsafe {
        let sm_ptr = &raw mut sign_st().sign_map;
        let Some(pos) = (*sm_ptr)
            .iter()
            .position(|sp| crate::memory::cstr_as_bytes(sp.sn_name.as_ptr()) == crate::memory::cstr_as_bytes(name))
        else {
            semsg!(gettext(c"E155: Unknown sign: %s"), name);
            return false;
        };
        (*sm_ptr).swap_remove(pos);
        true
    }
}

fn sign_list_defined(sp: &mut sign_T) {
    smsg!(0, c"sign %s", sp.sn_name.as_ptr());
        if let Some(ref icon) = sp.sn_icon {
            msg_puts(c" icon=");
            msg_outtrans(icon.as_ptr(), 0, false);
            msg_puts(gettext(c" (not supported)"));
        }
        if sp.sn_text[0] != 0 {
            msg_puts(c" text=");
            let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
            describe_sign_text(buf.as_mut_ptr(), sp.sn_text.as_mut_ptr());
            msg_outtrans(buf.as_ptr(), 0, false);
        }
        if sp.sn_priority > 0 {
            let mut lbuf = [0 as c_char; MSG_BUF_LEN];
            vim_snprintf!(
                lbuf.as_mut_ptr(),
                MSG_BUF_LEN,
                c" priority=%d",
                sp.sn_priority,
            );
            msg_puts(lbuf.as_ptr());
        }
        let args = [
            c" linehl=".as_ptr(),
            c" texthl=".as_ptr(),
            c" culhl=".as_ptr(),
            c" numhl=".as_ptr(),
        ];
        let hls = [
            sp.sn_line_hl,
            sp.sn_text_hl,
            sp.sn_cul_hl,
            sp.sn_num_hl,
        ];
        for i in 0..4 {
            if hls[i] > 0 {
                msg_puts(args[i]);
                let p = get_highlight_name_ext(&mut crate::os::env::expand_T::new(), hls[i] - 1, false);
                msg_puts(if !p.is_null() { p } else { c"NONE".as_ptr() });
            }
        }
}

fn sign_list_by_name(name: *mut c_char) {
    unsafe {
        let sp = pmap_get_sign(name);
        if !sp.is_null() {
            sign_list_defined(&mut *sp);
        } else {
            semsg!(gettext(c"E155: Unknown sign: %s"), name);
        }
    }
}

fn sign_place(
    id: &mut u32,
    group: *mut c_char,
    name: *mut c_char,
    buf: &mut buf_T,
    mut lnum: linenr_T,
    mut prio: c_int,
) -> bool {
    unsafe {
        if !group.is_null() && (*group == b'*' as c_char || *group == NUL) {
            return false;
        }

        let sp = pmap_get_sign(name);
        if sp.is_null() {
            semsg!(gettext(c"E155: Unknown sign: %s"), name);
            return false;
        }

        if prio == -1 {
            prio = if (*sp).sn_priority != -1 {
                (*sp).sn_priority
            } else {
                SIGN_DEF_PRIO
            };
        }

        if lnum > 0 {
            buf_set_sign( buf, id, group, prio, lnum,  &mut *sp);
        } else {
            lnum = buf_mod_sign( buf, &mut *id, group, prio,  &mut *sp);
        }
        if lnum <= 0 {
            semsg!(
                gettext(c"E885: Not possible to change sign %s"),
                name,
            );
            return false;
        }

        true
    }
}

fn sign_unplace_inner(
    buf: &mut buf_T,
    id: c_int,
    group: *mut c_char,
    atlnum: linenr_T,
) -> bool {
    unsafe {
        if !buf_has_signs( buf) {
            return false;
        }

        if id == 0 || atlnum > 0 || (!group.is_null() && *group == b'*' as c_char) {
            if !buf_delete_signs( buf, group, id, atlnum) {
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
}

fn sign_unplace(buf: Option<&mut buf_T>, id: c_int, group: *mut c_char, atlnum: linenr_T) -> bool {
     {
        if let Some(buf) = buf {
            sign_unplace_inner( buf, id, group, atlnum)
        } else {
            let mut retval = true;
            for cbuf in for_all_buffers_ptr() {
                if !sign_unplace_inner(unsafe { &mut *cbuf }, id, group, atlnum) {
                    retval = false;
                }
            }
            retval
        }
    }
}

fn sign_jump(id: c_int, group: *mut c_char, buf: &mut buf_T) -> linenr_T {
     {
        let lnum = buf_findsign( buf, id, group);
        if lnum <= 0 {
            semsg!(gettext(c"E157: Invalid sign ID: %d"), id);
            return -1;
        }

        if !buf_jump_open_win(&raw mut *buf).is_null() {
            curwin_mut().w_cursor.lnum = lnum;
            check_cursor_lnum(curwin_mut());
            beginline(BL_WHITE);
        } else {
            if buf.b_fname.is_null() {
                emsg(gettext(
                    c"E934: Cannot jump to a buffer that does not have a name",
                ));
                return -1;
            }
            setpcmark();
            if !do_ecmd(
                buf.b_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
                None,
                lnum,
                if buf_hide(curbuf_mut()) { ECMD_HIDE } else { 0 },
                curwin_ptr(),
            )
            {
                return -1;
            }
        }

        foldOpenCursor();
        lnum
    }
}

fn sign_define_cmd(name: *mut c_char, mut cmdline: *mut c_char) {
    unsafe {
        let mut icon: *mut c_char = ptr::null_mut();
        let mut text: *mut c_char = ptr::null_mut();
        let mut linehl: *mut c_char = ptr::null_mut();
        let mut texthl: *mut c_char = ptr::null_mut();
        let mut culhl: *mut c_char = ptr::null_mut();
        let mut numhl: *mut c_char = ptr::null_mut();
        let mut prio: c_int = -1;

        loop {
            let arg = skipwhite(cmdline);
            if *arg == NUL {
                break;
            }
            cmdline = skiptowhite_esc(arg);
            if crate::memory::cstr_prefix_bytes(arg, 5) == &b"icon="[..5] {
                icon = arg.add(5);
            } else if crate::memory::cstr_prefix_bytes(arg, 5) == &b"text="[..5] {
                text = arg.add(5);
            } else if crate::memory::cstr_prefix_bytes(arg, 7) == &b"linehl="[..7] {
                linehl = arg.add(7);
            } else if crate::memory::cstr_prefix_bytes(arg, 7) == &b"texthl="[..7] {
                texthl = arg.add(7);
            } else if crate::memory::cstr_prefix_bytes(arg, 6) == &b"culhl="[..6] {
                culhl = arg.add(6);
            } else if crate::memory::cstr_prefix_bytes(arg, 6) == &b"numhl="[..6] {
                numhl = arg.add(6);
            } else if crate::memory::cstr_prefix_bytes(arg, 9) == &b"priority="[..9] {
                prio = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(arg.add(9))) as c_int;
            } else {
                semsg!(gettext(e_invarg2), arg);
                return;
            }
            if *cmdline == NUL {
                break;
            }
            *cmdline = NUL;
            cmdline = cmdline.add(1);
        }

        sign_define_by_name(name, icon, text, linehl, texthl, culhl, numhl, prio);
    }
}

fn sign_place_cmd(
    buf: *mut buf_T,
    lnum: linenr_T,
    name: *mut c_char,
    id: c_int,
    group: *mut c_char,
    prio: c_int,
) {
    unsafe {
        if id <= 0 {
            if lnum >= 0 || !name.is_null() || (!group.is_null() && *group == NUL) {
                emsg(gettext(e_invarg));
            } else {
                sign_list_placed(buf, group);
            }
        } else {
            if name.is_null() || buf.is_null() || (!group.is_null() && *group == NUL) {
                emsg(gettext(e_invarg));
                return;
            }
            let mut uid = id as u32;
            sign_place(&mut uid, group, name,  &mut *buf, lnum, prio);
        }
    }
}

fn sign_unplace_cmd(
    mut buf: *mut buf_T,
    mut lnum: linenr_T,
    name: *const c_char,
    id: c_int,
    group: *mut c_char,
) {
    unsafe {
        if lnum >= 0 || !name.is_null() || (!group.is_null() && *group == NUL) {
            emsg(gettext(e_invarg));
            return;
        }

        if id == -1 {
            lnum = curwin_mut().w_cursor.lnum;
            buf = curwin_mut().w_buffer;
        }

        if !sign_unplace(buf.as_mut(), id.max(0), group, lnum) && lnum > 0 {
            emsg(gettext(c"E159: Missing sign number"));
        }
    }
}

fn sign_jump_cmd(
    buf: Option<&mut buf_T>,
    lnum: linenr_T,
    name: *const c_char,
    id: c_int,
    group: *mut c_char,
) {
    unsafe {
        if name.is_null() && group.is_null() && id == -1 {
            emsg(gettext(e_argreq));
            return;
        }

        if buf.is_none() || (!group.is_null() && *group == NUL) || lnum >= 0 || !name.is_null() {
            emsg(gettext(e_invarg));
            return;
        }

        sign_jump(id, group,  buf.unwrap());
    }
}

fn parse_sign_cmd_args(
    cmd: c_int,
    mut arg: *mut c_char,
    name: *mut *mut c_char,
    id: &mut c_int,
    group: *mut *mut c_char,
    prio: &mut c_int,
    buf: *mut *mut buf_T,
    lnum: &mut linenr_T,
) -> bool {
    unsafe {
        let arg1 = arg;
        let mut filename: *mut c_char = ptr::null_mut();
        let mut lnum_arg = false;

        if ascii_isdigit(*arg as c_int) {
            *id = getdigits_int(&raw mut arg, true, 0);
            if !ascii_iswhite(*arg as c_int) && *arg != NUL {
                *id = -1;
                arg = arg1;
            } else {
                arg = skipwhite(arg);
            }
        }

        while *arg != NUL {
            if crate::memory::cstr_prefix_bytes(arg, 5) == &b"line="[..5] {
                arg = arg.add(5);
                *lnum = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(arg)) as c_int;
                arg = skiptowhite(arg);
                lnum_arg = true;
            } else if (crate::memory::cstr_prefix_bytes(arg, 1) == &b"*"[..1]) && cmd == SIGNCMD_UNPLACE {
                if *id != -1 {
                    emsg(gettext(e_invarg));
                    return false;
                }
                *id = -2;
                arg = skiptowhite(arg.add(1));
            } else if crate::memory::cstr_prefix_bytes(arg, 5) == &b"name="[..5] {
                arg = arg.add(5);
                let mut namep = arg;
                arg = skiptowhite(arg);
                if *arg != NUL {
                    *arg = NUL;
                    arg = arg.add(1);
                }
                while *namep == b'0' as c_char && *namep.add(1) != NUL {
                    namep = namep.add(1);
                }
                *name = namep;
            } else if crate::memory::cstr_prefix_bytes(arg, 6) == &b"group="[..6] {
                arg = arg.add(6);
                *group = arg;
                arg = skiptowhite(arg);
                if *arg != NUL {
                    *arg = NUL;
                    arg = arg.add(1);
                }
            } else if crate::memory::cstr_prefix_bytes(arg, 9) == &b"priority="[..9] {
                arg = arg.add(9);
                *prio = crate::charset::parse_i64_prefix(crate::memory::cstr_as_bytes(arg)) as c_int;
                arg = skiptowhite(arg);
            } else if crate::memory::cstr_prefix_bytes(arg, 5) == &b"file="[..5] {
                arg = arg.add(5);
                filename = arg;
                *buf = buflist_findname_exp(arg);
                break;
            } else if crate::memory::cstr_prefix_bytes(arg, 7) == &b"buffer="[..7] {
                arg = arg.add(7);
                filename = arg;
                *buf = buflist_findnr(getdigits_int(&raw mut arg, true, 0));
                if *skipwhite(arg) != NUL {
                    semsg!(gettext(e_trailing_arg), arg);
                }
                break;
            } else {
                emsg(gettext(e_invarg));
                return false;
            }
            arg = skipwhite(arg);
        }

        if !filename.is_null() && (*buf).is_null() {
            semsg!(gettext(e_invalid_buffer_name_str), filename);
            return false;
        }

        if filename.is_null() && ((cmd == SIGNCMD_PLACE && lnum_arg) || cmd == SIGNCMD_JUMP) {
            *buf = curwin_mut().w_buffer;
        }
        true
    }
}

pub fn ex_sign(eap: &mut exarg_T) {
    unsafe {
        let mut arg = eap.arg;

        let mut p = skiptowhite(arg);
        let idx = sign_cmd_idx(arg, p);
        if idx == SIGNCMD_LAST {
            semsg!(gettext(c"E160: Unknown sign command: %s"), arg);
            return;
        }
        arg = skipwhite(p);

        if idx <= SIGNCMD_LIST {
            if idx == SIGNCMD_LIST && *arg == NUL {
                let sm_ptr = &raw mut sign_st().sign_map;
                for sp in &mut *sm_ptr {
                    sign_list_defined(sp);
                }
            } else if *arg == NUL {
                emsg(gettext(c"E156: Missing sign name"));
            } else {
                p = skiptowhite(arg);
                if *p != NUL {
                    *p = NUL;
                    p = p.add(1);
                }
                while *arg == b'0' as c_char && *arg.add(1) != NUL {
                    arg = arg.add(1);
                }

                if idx == SIGNCMD_DEFINE {
                    sign_define_cmd(arg, p);
                } else if idx == SIGNCMD_LIST {
                    sign_list_by_name(arg);
                } else {
                    sign_undefine_by_name(arg);
                }
            }
        } else {
            let mut id: c_int = -1;
            let mut lnum: linenr_T = -1;
            let mut name: *mut c_char = ptr::null_mut();
            let mut group: *mut c_char = ptr::null_mut();
            let mut prio: c_int = -1;
            let mut buf: *mut buf_T = ptr::null_mut();

            if !parse_sign_cmd_args(
                idx,
                arg,
                &raw mut name,
                &mut id,
                &raw mut group,
                &mut prio,
                &raw mut buf,
                &mut lnum,
            )
            {
                return;
            }

            if idx == SIGNCMD_PLACE {
                sign_place_cmd(buf, lnum, name, id, group, prio);
            } else if idx == SIGNCMD_UNPLACE {
                sign_unplace_cmd(buf, lnum, name, id, group);
            } else if idx == SIGNCMD_JUMP {
                sign_jump_cmd(buf.as_mut(), lnum, name, id, group);
            }
        }
    }
}

fn sign_get_info_dict(sp: &mut sign_T) -> *mut dict_T {
    let d = tv_dict_alloc();

        tv_dict_add_str(unsafe { &mut *d }, c"name", sp.sn_name.as_ptr());

        if let Some(ref icon) = sp.sn_icon {
            tv_dict_add_str(unsafe { &mut *d }, c"icon", icon.as_ptr());
        }
        if sp.sn_text[0] != 0 {
            let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
            describe_sign_text(buf.as_mut_ptr(), sp.sn_text.as_mut_ptr());
            tv_dict_add_str(unsafe { &mut *d }, c"text", buf.as_ptr());
        }
        if sp.sn_priority > 0 {
            tv_dict_add_nr(unsafe { &mut *d }, c"priority", i64::from(sp.sn_priority));
        }
        let args = [
            (c"linehl", sp.sn_line_hl),
            (c"texthl", sp.sn_text_hl),
            (c"culhl", sp.sn_cul_hl),
            (c"numhl", sp.sn_num_hl),
        ];
        for (name, hl) in args {
            if hl > 0 {
                let p = get_highlight_name_ext(&mut crate::os::env::expand_T::new(), hl - 1, false);
                if p.is_null() {
                    tv_dict_add_str_len(unsafe { &mut *d }, name, c"NONE", 4);
                } else {
                    tv_dict_add_str(unsafe { &mut *d }, name, p);
                }
            }
        }
        d
}

fn sign_get_placed_info_dict(mark: MTKey) -> *mut dict_T {
    let d = tv_dict_alloc();
        let sh = decor_find_sign(mt_decor(mark)).unwrap();

        tv_dict_add_str(unsafe { &mut *d }, c"name", sign_get_name(sh));
        tv_dict_add_nr(unsafe { &mut *d }, c"id", i64::from(mark.id as c_int));
        tv_dict_add_str(
            unsafe { &mut *d },
            c"group",
            describe_ns(mark.ns as c_int, c""),
        );
        tv_dict_add_nr(unsafe { &mut *d }, c"lnum", i64::from(mark.pos.row + 1));
        tv_dict_add_nr(unsafe { &mut *d }, c"priority", i64::from(sh.priority));
        d
}

pub fn get_buffer_signs(buf: &mut buf_T) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        let mut itr = MarkTreeIter::new();
        let b = &buf.b_marktree;
        marktree_itr_get(b, 0, 0, &mut itr);

        while itr.x.is_some() {
            let mark = marktree_itr_current(b, &itr);
            if !mt_end(mark) && mt_decor_sign(mark) {
                tv_list_append_dict( &mut *l, sign_get_placed_info_dict(mark));
            }
            marktree_itr_next(b, &mut itr);
        }

        l
    }
}

fn sign_get_placed_in_buf(
    buf: &mut buf_T,
    lnum: linenr_T,
    sign_id: c_int,
    group: *const c_char,
    retlist: &mut list_T,
) {
    unsafe {
        let d = tv_dict_alloc();
        tv_list_append_dict( &mut *retlist, d);

        tv_dict_add_nr(&mut *d, c"bufnr", i64::from(buf.b_fnum));

        let l = tv_list_alloc(kListLenMayKnow);
        tv_dict_add_list(&mut *d, c"signs", l);

        let ns = group_get_ns(group);
        if !buf_has_signs( buf) || ns < 0 {
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
                    || (lnum == 0 && sign_id == mark.id as c_int)
                    || (lnum == mark.pos.row + 1 && sign_id == mark.id as c_int))
                && mt_decor_sign(mark)
            {
                signs.push(mark);
            }
            marktree_itr_next(b, &mut itr);
        }

        if !signs.is_empty() {
            signs.sort_by(sign_row_cmp_slice);
            for mark in signs {
                tv_list_append_dict( &mut *l, sign_get_placed_info_dict(mark));
            }
        }
    }
}

fn sign_get_placed(
    buf: Option<&mut buf_T>,
    lnum: linenr_T,
    id: c_int,
    group: *const c_char,
    retlist: &mut list_T,
) {
    unsafe {
        if let Some(buf) = buf {
            sign_get_placed_in_buf( buf, lnum, id, group, retlist);
        } else {
            for cbuf in for_all_buffers_ptr() {
                if buf_has_signs( &*cbuf) {
                    sign_get_placed_in_buf( &mut *cbuf, 0, id, group, retlist);
                }
            }
        }
    }
}

pub fn free_signs() {
    sign_st().sign_map = Vec::new();
}

fn get_nth_sign_name(idx: c_int) -> *mut c_char {
    unsafe {
        let sm_ptr = &raw const sign_st().sign_map;
        if let Ok(idx_usize) = usize::try_from(idx)
            && let Some(sp) = (*sm_ptr).as_slice().get(idx_usize)
        {
            return sp.sn_name.as_ptr().cast_mut();
        }
        ptr::null_mut()
    }
}

fn get_nth_sign_group_name(idx: c_int) -> *mut c_char {
    if idx < 0 {
        return ptr::null_mut();
    }
    unsafe {
        let ns_ptr = &raw const sign_st().sign_ns;
        let ns_slice = (*ns_ptr).as_slice();
        ns_slice.get(idx as usize).map_or(ptr::null_mut(), |&ns| {
            describe_ns(ns as NS, c"").cast_mut()
        })
    }
}

pub fn get_sign_name(_xp: &mut expand_T, idx: c_int) -> *mut c_char {
    match sign_st().expand_what {
            EXP_SUBCMD => CMDS[idx as usize].cast_mut(),
            EXP_DEFINE => {
                const DEFINE_ARG: [*const c_char; 8] = [
                    c"culhl=".as_ptr(),
                    c"icon=".as_ptr(),
                    c"linehl=".as_ptr(),
                    c"numhl=".as_ptr(),
                    c"text=".as_ptr(),
                    c"texthl=".as_ptr(),
                    c"priority=".as_ptr(),
                    ptr::null(),
                ];
                DEFINE_ARG[idx as usize].cast_mut()
            }
            EXP_PLACE => {
                const PLACE_ARG: [*const c_char; 7] = [
                    c"line=".as_ptr(),
                    c"name=".as_ptr(),
                    c"group=".as_ptr(),
                    c"priority=".as_ptr(),
                    c"file=".as_ptr(),
                    c"buffer=".as_ptr(),
                    ptr::null(),
                ];
                PLACE_ARG[idx as usize].cast_mut()
            }
            EXP_LIST | EXP_UNPLACE => {
                const LIST_ARG: [*const c_char; 4] = [
                    c"group=".as_ptr(),
                    c"file=".as_ptr(),
                    c"buffer=".as_ptr(),
                    ptr::null(),
                ];
                LIST_ARG[idx as usize].cast_mut()
            }
            EXP_SIGN_NAMES => get_nth_sign_name(idx),
            EXP_SIGN_GROUPS => get_nth_sign_group_name(idx),
            _ => ptr::null_mut(),
        }
}

pub fn set_context_in_sign_cmd(xp: &mut expand_T, arg: *mut c_char) {
    unsafe {
        xp.xp_context = EXPAND_SIGN;
        sign_st().expand_what = EXP_SUBCMD;
        xp.xp_pattern = arg;

        let end_subcmd = skiptowhite(arg);
        if *end_subcmd == NUL {
            return;
        }

        let cmd_idx = sign_cmd_idx(arg, end_subcmd);
        let begin_subcmd_args = skipwhite(end_subcmd);

        let mut last: *mut c_char;
        let mut p = begin_subcmd_args;
        loop {
            p = skipwhite(p);
            last = p;
            p = skiptowhite(p);
            if *p == NUL {
                break;
            }
        }

        p = vim_strchr(last, b'=' as c_int);
        if p.is_null() {
            xp.xp_pattern = last;
            match cmd_idx {
                SIGNCMD_DEFINE => sign_st().expand_what = EXP_DEFINE,
                SIGNCMD_PLACE => {
                    if ascii_isdigit(*begin_subcmd_args as c_int) {
                        sign_st().expand_what = EXP_PLACE;
                    } else {
                        sign_st().expand_what = EXP_LIST;
                    }
                }
                SIGNCMD_LIST | SIGNCMD_UNDEFINE => sign_st().expand_what = EXP_SIGN_NAMES,
                SIGNCMD_JUMP | SIGNCMD_UNPLACE => sign_st().expand_what = EXP_UNPLACE,
                _ => xp.xp_context = EXPAND_NOTHING,
            }
        } else {
            xp.xp_pattern = p.add(1);
            match cmd_idx {
                SIGNCMD_DEFINE => {
                    if (crate::memory::cstr_prefix_bytes(last, 6) == &b"texthl"[..6])
                        || (crate::memory::cstr_prefix_bytes(last, 6) == &b"linehl"[..6])
                        || (crate::memory::cstr_prefix_bytes(last, 5) == &b"culhl"[..5])
                        || (crate::memory::cstr_prefix_bytes(last, 5) == &b"numhl"[..5])
                    {
                        xp.xp_context = EXPAND_HIGHLIGHT;
                    } else if crate::memory::cstr_prefix_bytes(last, 4) == &b"icon"[..4] {
                        xp.xp_context = EXPAND_FILES;
                    } else {
                        xp.xp_context = EXPAND_NOTHING;
                    }
                }
                SIGNCMD_PLACE => {
                    if crate::memory::cstr_prefix_bytes(last, 4) == &b"name"[..4] {
                        sign_st().expand_what = EXP_SIGN_NAMES;
                    } else if crate::memory::cstr_prefix_bytes(last, 5) == &b"group"[..5] {
                        sign_st().expand_what = EXP_SIGN_GROUPS;
                    } else if crate::memory::cstr_prefix_bytes(last, 4) == &b"file"[..4] {
                        xp.xp_context = EXPAND_BUFFERS;
                    } else {
                        xp.xp_context = EXPAND_NOTHING;
                    }
                }
                SIGNCMD_UNPLACE | SIGNCMD_JUMP => {
                    if crate::memory::cstr_prefix_bytes(last, 5) == &b"group"[..5] {
                        sign_st().expand_what = EXP_SIGN_GROUPS;
                    } else if crate::memory::cstr_prefix_bytes(last, 4) == &b"file"[..4] {
                        xp.xp_context = EXPAND_BUFFERS;
                    } else {
                        xp.xp_context = EXPAND_NOTHING;
                    }
                }
                _ => xp.xp_context = EXPAND_NOTHING,
            }
        }
    }
}

fn sign_define_from_dict(mut name: *mut c_char, dict: *mut dict_T) -> c_int {
    unsafe {
        if name.is_null() {
            name = tv_dict_get_string(dict, c"name");
            if name.is_null() || *name == NUL {
                return -1;
            }
        }

        let mut icon: *mut c_char = ptr::null_mut();
        let mut linehl: *mut c_char = ptr::null_mut();
        let mut text: *mut c_char = ptr::null_mut();
        let mut texthl: *mut c_char = ptr::null_mut();
        let mut culhl: *mut c_char = ptr::null_mut();
        let mut numhl: *mut c_char = ptr::null_mut();
        let mut prio: c_int = -1;

        if !dict.is_null() {
            icon = tv_dict_get_string(dict, c"icon");
            linehl = tv_dict_get_string(dict, c"linehl");
            text = tv_dict_get_string(dict, c"text");
            texthl = tv_dict_get_string(dict, c"texthl");
            culhl = tv_dict_get_string(dict, c"culhl");
            numhl = tv_dict_get_string(dict, c"numhl");
            prio = tv_dict_get_number_def(dict, c"priority", -1) as c_int;
        }

        if sign_define_by_name(name, icon, text, linehl, texthl, culhl, numhl, prio) { 0 } else { -1 }
    }
}

fn sign_define_multiple(l: Option<&list_T>, retlist: &mut list_T) {
     {
        if l.is_none() {
            return;
        }
        for li in crate::eval::typval::list_iter_mut(l.unwrap().lv_first) {
            let mut retval: c_int = -1;
            if li.li_tv.v_type == VarType::VAR_DICT {
                retval = sign_define_from_dict(
                    ptr::null_mut(),
                    li.li_tv.vval.v_dict(),
                );
            } else {
                emsg(gettext(e_dictreq));
            }
            tv_list_append_number(retlist, i64::from(retval));
        }
    }
}

pub fn f_sign_define(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        if argvars[0].v_type == VarType::VAR_LIST
            && argvars[1].v_type == VarType::VAR_UNKNOWN
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            sign_define_multiple(unsafe { argvars[0].vval.v_list().as_ref() }, unsafe { &mut *rettv.vval.v_list() });
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let name = tv_get_string_chk(&argvars[0]).cast_mut();
        if name.is_null() {
            return;
        }

        if !tv_check_for_opt_dict_arg(argvars, 1) {
            return;
        }

        let d = if argvars[1].v_type == VarType::VAR_DICT {
            argvars[1].vval.v_dict()
        } else {
            ptr::null_mut()
        };
        rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_define_from_dict(name, d)));
    }
}

pub fn f_sign_getdefined(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        tv_list_alloc_ret(rettv, 0);

        if argvars[0].v_type == VarType::VAR_UNKNOWN {
            let sm_ptr = &raw mut sign_st().sign_map;
            for sp in &mut *sm_ptr {
                tv_list_append_dict( &mut *rettv.vval.v_list(), sign_get_info_dict(sp));
            }
        } else {
            let sp = pmap_get_sign(tv_get_string(&argvars[0]));
            if !sp.is_null() {
                tv_list_append_dict( &mut *rettv.vval.v_list(), sign_get_info_dict(&mut *sp));
            }
        }
    }
}

pub fn f_sign_getplaced(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut lnum: linenr_T = 0;
        let mut sign_id: c_int = 0;
        let mut group: *const c_char = ptr::null();
        let mut notanum = false;

        tv_list_alloc_ret(rettv, 0);

        if argvars[0].v_type != VarType::VAR_UNKNOWN {
            buf = get_buf_arg(&mut argvars[0]);
            if buf.is_null() {
                return;
            }

            if argvars[1].v_type != VarType::VAR_UNKNOWN {
                if !tv_check_for_nonnull_dict_arg(argvars, 1) {
                    return;
                }
                let dict = argvars[1].vval.v_dict();
                let di_lnum = tv_dict_find(dict.as_ref(), c"lnum", -1);
                if !di_lnum.is_null() {
                    lnum = tv_get_lnum(&(*di_lnum).di_tv);
                    if lnum <= 0 {
                        return;
                    }
                }
                let di_id = tv_dict_find(dict.as_ref(), c"id", -1);
                if !di_id.is_null() {
                    sign_id =
                        tv_get_number_chk(&(*di_id).di_tv, Some(&mut notanum)) as c_int;
                    if notanum {
                        return;
                    }
                }
                let di_group = tv_dict_find(dict.as_ref(), c"group", -1);
                if !di_group.is_null() {
                    group = tv_get_string_chk(&(*di_group).di_tv);
                    if group.is_null() {
                        return;
                    }
                    if *group == NUL {
                        group = ptr::null();
                    }
                }
            }
        }

        sign_get_placed(buf.as_mut(), lnum, sign_id, group,  &mut *rettv.vval.v_list());
    }
}

pub fn f_sign_jump(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

    unsafe {
        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        let mut notanum = false;
        let id = tv_get_number_chk(&argvars[0], Some(&mut notanum)) as c_int;
        if notanum {
            return;
        }
        if id <= 0 {
            emsg(gettext(e_invarg));
            return;
        }

        let mut group = tv_get_string_chk(&argvars[1]).cast_mut();
        if group.is_null() {
            return;
        }
        if *group == NUL {
            group = ptr::null_mut();
        }

        let buf = get_buf_arg(&mut argvars[2]);
        if buf.is_null() {
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_jump(id, group,  &mut *buf)));
    }
}

fn sign_place_from_dict(
    mut id_tv: Option<&typval_T>,
    mut group_tv: Option<&typval_T>,
    mut name_tv: Option<&typval_T>,
    mut buf_tv: Option<&mut typval_T>,
    dict: *mut dict_T,
) -> c_int {
    unsafe {
        let mut id: c_int = 0;
        let mut notanum = false;
        if id_tv.is_none() {
            let di = tv_dict_find(dict.as_ref(), c"id", -1);
            if !di.is_null() {
                id_tv = Some(&(*di).di_tv);
            }
        }
        if let Some(id_tv) = id_tv {
            id = tv_get_number_chk(id_tv, Some(&mut notanum)) as c_int;
            if notanum {
                return -1;
            }
            if id < 0 {
                emsg(gettext(e_invarg));
                return -1;
            }
        }

        let mut group: *mut c_char = ptr::null_mut();
        if group_tv.is_none() {
            let di = tv_dict_find(dict.as_ref(), c"group", -1);
            if !di.is_null() {
                group_tv = Some(&(*di).di_tv);
            }
        }
        if let Some(group_tv) = group_tv {
            group = tv_get_string_chk(group_tv).cast_mut();
            if group.is_null() {
                return -1;
            }
            if *group == NUL {
                group = ptr::null_mut();
            }
        }

        if name_tv.is_none() {
            let di = tv_dict_find(dict.as_ref(), c"name", -1);
            if !di.is_null() {
                name_tv = Some(&(*di).di_tv);
            }
        }
        let Some(name_tv) = name_tv else {
            return -1;
        };
        let name = tv_get_string_chk(name_tv).cast_mut();
        if name.is_null() {
            return -1;
        }

        if buf_tv.is_none() {
            let di = tv_dict_find(dict.as_ref(), c"buffer", -1);
            if !di.is_null() {
                buf_tv = Some(&mut (*di).di_tv);
            }
        }
        let Some(buf_tv) = buf_tv else {
            return -1;
        };
        let buf = get_buf_arg(buf_tv);
        if buf.is_null() {
            return -1;
        }

        let mut lnum: linenr_T = 0;
        let di_lnum = tv_dict_find(dict.as_ref(), c"lnum", -1);
        if !di_lnum.is_null() {
            lnum = tv_get_lnum(&(*di_lnum).di_tv);
            if lnum <= 0 {
                emsg(gettext(e_invarg));
                return -1;
            }
        }

        let mut prio: c_int = -1;
        let di_prio = tv_dict_find(dict.as_ref(), c"priority", -1);
        if !di_prio.is_null() {
            prio = tv_get_number_chk(&(*di_prio).di_tv, Some(&mut notanum)) as c_int;
            if notanum {
                return -1;
            }
        }

        let mut uid = id as u32;
        if sign_place(&mut uid, group, name,  &mut *buf, lnum, prio) {
            return uid as c_int;
        }

        -1
    }
}

pub fn f_sign_place(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    let mut dict: *mut dict_T = ptr::null_mut();

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if argvars[4].v_type != VarType::VAR_UNKNOWN {
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
        dict,
    )));
}

pub fn f_sign_placelist(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if argvars[0].v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq));
            return;
        }

        let l = argvars[0].vval.v_list();
        if !l.is_null() {
            for li in crate::eval::typval::list_iter_mut((*l).lv_first) {
                let mut sign_id: c_int = -1;
                if li.li_tv.v_type == VarType::VAR_DICT {
                    sign_id = sign_place_from_dict(
                        None,
                        None,
                        None,
                        None,
                        li.li_tv.vval.v_dict(),
                    );
                } else {
                    emsg(gettext(e_dictreq));
                }
                tv_list_append_number( &mut *rettv.vval.v_list(), i64::from(sign_id));
            }
        }
    }
}

fn sign_undefine_multiple(l: Option<&list_T>, retlist: &mut list_T) {
     {
        if l.is_none() {
            return;
        }
        for li in crate::eval::typval::list_iter_mut(l.unwrap().lv_first) {
            let mut retval: c_int = -1;
            let name = tv_get_string_chk(&li.li_tv);
            if !name.is_null() && sign_undefine_by_name(name) {
                retval = 0;
            }
            tv_list_append_number(retlist, i64::from(retval));
        }
    }
}

pub fn f_sign_undefine(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData){

     {
        if argvars[0].v_type == VarType::VAR_LIST
            && argvars[1].v_type == VarType::VAR_UNKNOWN
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            sign_undefine_multiple(unsafe { argvars[0].vval.v_list().as_ref() }, unsafe { &mut *rettv.vval.v_list() });
            return;
        }

        rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

        if argvars[0].v_type == VarType::VAR_UNKNOWN {
            free_signs();
            rettv.vval = crate::eval::typval::TypvalVval::Number(0);
        } else {
            let name = tv_get_string_chk(&argvars[0]);
            if name.is_null() {
                return;
            }
            if sign_undefine_by_name(name) {
                rettv.vval = crate::eval::typval::TypvalVval::Number(0);
            }
        }
    }
}

fn sign_unplace_from_dict(group_tv: Option<&typval_T>, dict: *mut dict_T) -> c_int {
    unsafe {
        let mut id: c_int = 0;
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut group = if let Some(group_tv) = group_tv {
            tv_get_string(group_tv).cast_mut()
        } else {
            tv_dict_get_string(dict, c"group")
        };
        if !group.is_null() && *group == NUL {
            group = ptr::null_mut();
        }

        if !dict.is_null() {
            let di_buf = tv_dict_find(dict.as_ref(), c"buffer", -1);
            if !di_buf.is_null() {
                buf = get_buf_arg(&mut (*di_buf).di_tv);
                if buf.is_null() {
                    return -1;
                }
            }
            if !tv_dict_find(dict.as_ref(), c"id", -1).is_null() {
                id = tv_dict_get_number(dict, c"id") as c_int;
                if id <= 0 {
                    emsg(gettext(e_invarg));
                    return -1;
                }
            }
        }

        if sign_unplace(buf.as_mut(), id, group, 0) { 0 } else { -1 }
    }
}

pub fn f_sign_unplace(argvars: &mut [typval_T], rettv: &mut typval_T, _fptr: EvalFuncData) {
    let mut dict: *mut dict_T = ptr::null_mut();

    rettv.vval = crate::eval::typval::TypvalVval::Number(-1);

    if !tv_check_for_string_arg(argvars, 0)
        || !tv_check_for_opt_dict_arg(argvars, 1)
    {
        return;
    }

    if argvars[1].v_type != VarType::VAR_UNKNOWN {
        dict = argvars[1].vval.v_dict();
    }

    rettv.vval = crate::eval::typval::TypvalVval::Number(i64::from(sign_unplace_from_dict(
        Some(&argvars[0]),
        dict,
    )));
}

pub fn f_sign_unplacelist(argvars: &mut [typval_T], rettv: &mut typval_T,
    _fptr: EvalFuncData,){

    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if argvars[0].v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq));
            return;
        }

        let l = argvars[0].vval.v_list();
        if !l.is_null() {
            for li in crate::eval::typval::list_iter_mut((*l).lv_first) {
                let mut retval: c_int = -1;
                if li.li_tv.v_type == VarType::VAR_DICT {
                    retval = sign_unplace_from_dict(
                        None,
                        li.li_tv.vval.v_dict(),
                    );
                } else {
                    emsg(gettext(e_dictreq));
                }
                tv_list_append_number( &mut *rettv.vval.v_list(), i64::from(retval));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_cmd_idx_and_describe_sign_text_work() {
        unsafe {
            let mut cmd = *b"define\0";
            assert_eq!(
                sign_cmd_idx(cmd.as_mut_ptr().cast(), cmd.as_mut_ptr().add(6).cast()),
                SIGNCMD_DEFINE
            );
            let mut st = [schar_from_ascii(b'>'), schar_from_ascii(b'>')];
            let mut buf = [0 as c_char; 16];
            let len = describe_sign_text(buf.as_mut_ptr(), st.as_mut_ptr());
            assert_eq!(len, 2);
            assert_eq!(crate::memory::cstr_as_bytes(buf.as_ptr()), b">>");
        }
    }
}
