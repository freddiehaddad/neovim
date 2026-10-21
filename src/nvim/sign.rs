//! Functions for managing signs (`src/nvim/sign.c`).

use crate::api::extmark::{describe_ns, nvim_create_namespace};
use crate::api::private::helpers::cstr_as_string;
use crate::api::private::validate::NvimString;
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
use crate::grid::{schar_T, schar_from_ascii, schar_get};
use crate::highlight_group::{get_highlight_name_ext, syn_check_group};
use crate::insert::beginline;
use crate::kvec::KVec;
use crate::map::{
    MH_TOMBSTONE, Map, MapHash, Set, cstr_t, map_del_cstr_t_ptr_t, map_put_ref_cstr_t_ptr_t,
    map_ref_String_int, map_ref_cstr_t_ptr_t, mh_get_cstr_t, ptr_t,
};
use crate::mark::setpcmark;
use crate::marktree::{
    DecorExt, DecorInlineData, MT_FLAG_DECOR_SIGNHL, MT_FLAG_DECOR_SIGNTEXT, MTKey, MTPair,
    MarkTreeIter, kMTMetaSignHL, kMTMetaSignText, marktree_itr_current, marktree_itr_get,
    marktree_itr_get_overlap, marktree_itr_next, marktree_itr_step_overlap, marktree_lookup_ns,
    mt_decor, mt_decor_sign, mt_end,
};
use crate::mbyte::{utf_ptr2cells, utfc_ptr2len, utfc_ptr2schar};
use crate::memory::{xcalloc, xfree, xstrdup};
use crate::message::{
    emsg, msg_outtrans, msg_putchar, msg_puts, msg_puts_hl, msg_puts_title, semsg, smsg,
};
use crate::msgpack_rpc::packer::Integer;
use crate::strings::{atoi, gettext, strcmp, strncmp, vim_snprintf, vim_strchr};
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
    pub sn_name: *mut c_char,
    pub sn_icon: *mut c_char,
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
use crate::math::{FAIL, OK};
use crate::os::env::expand_T;

static mut sign_map: Map<cstr_t, ptr_t> = Map {
    set: Set {
        h: MapHash {
            n_buckets: 0,
            size: 0,
            n_occupied: 0,
            upper_bound: 0,
            n_keys: 0,
            keys_capacity: 0,
            hash: ptr::null_mut(),
        },
        keys: ptr::null_mut(),
    },
    values: ptr::null_mut(),
};

static mut sign_ns: KVec<Integer> = KVec {
    size: 0,
    capacity: 0,
    items: ptr::null_mut(),
};

static mut expand_what: c_int = EXP_SUBCMD;

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
unsafe fn map_get_string_int(key: NvimString) -> c_int {
    unsafe {
        let ptr = map_ref_String_int(&raw mut namespace_ids, key, ptr::null_mut());
        if ptr.is_null() { 0 } else { *ptr }
    }
}

#[inline]
unsafe fn pmap_get_sign(name: *const c_char) -> *mut sign_T {
    unsafe {
        let ptr = map_ref_cstr_t_ptr_t(&raw mut sign_map, name, ptr::null_mut());
        if ptr.is_null() {
            ptr::null_mut()
        } else {
            (*ptr).cast::<sign_T>()
        }
    }
}

unsafe fn group_get_ns(group: *const c_char) -> i64 {
    unsafe {
        if group.is_null() {
            return 0;
        } else if strcmp(group, c"*".as_ptr()) == 0 {
            return i64::from(u32::MAX);
        }
        let ns = map_get_string_int(cstr_as_string(group));
        if ns != 0 { i64::from(ns) } else { -1 }
    }
}

unsafe fn sign_get_name(sh: *mut DecorSignHighlight) -> *const c_char {
    unsafe {
        let name = (*sh).sign_name;
        if name.is_null() {
            c"".as_ptr()
        } else if mh_get_cstr_t(&raw mut sign_map.set, name) != MH_TOMBSTONE {
            name
        } else {
            c"[Deleted]".as_ptr()
        }
    }
}

unsafe fn buf_set_sign(
    buf: *mut buf_T,
    id: *mut u32,
    group: *mut c_char,
    prio: c_int,
    lnum: linenr_T,
    sp: *mut sign_T,
) {
    unsafe {
        if !group.is_null() && map_get_string_int(cstr_as_string(group)) == 0 {
            let ns_vec = &raw mut sign_ns;
            (*ns_vec).push(nvim_create_namespace(cstr_as_string(group)));
        }

        let ns = if !group.is_null() {
            nvim_create_namespace(cstr_as_string(group)) as u32
        } else {
            0
        };
        let mut sign = DECOR_SIGN_HIGHLIGHT_INIT;

        sign.flags |= kSHIsSign;
        sign.text = (*sp).sn_text;
        sign.sign_name = xstrdup((*sp).sn_name);
        sign.hl_id = (*sp).sn_text_hl;
        sign.line_hl_id = (*sp).sn_line_hl;
        sign.number_hl_id = (*sp).sn_num_hl;
        sign.cursorline_hl_id = (*sp).sn_cul_hl;
        sign.priority = prio as DecorPriority;

        let has_hl = (*sp).sn_line_hl != 0 || (*sp).sn_num_hl != 0 || (*sp).sn_cul_hl != 0;
        let decor_flags = if (*sp).sn_text[0] != 0 {
            MT_FLAG_DECOR_SIGNTEXT
        } else {
            0
        } | if has_hl { MT_FLAG_DECOR_SIGNHL } else { 0 };

        let decor = DecorInline {
            ext: true,
            data: DecorInlineData {
                ext: DecorExt {
                    sh_idx: decor_put_sh(sign),
                    vt: ptr::null_mut(),
                },
            },
        };
        extmark_set(
            buf,
            ns,
            id,
            (*buf).b_ml.ml_line_count.min(lnum) - 1,
            0,
            -1,
            -1,
            decor,
            decor_flags,
            true,
            false,
            true,
            true,
            ptr::null_mut(),
        );
    }
}

unsafe fn buf_mod_sign(
    buf: *mut buf_T,
    id: *mut u32,
    group: *mut c_char,
    prio: c_int,
    sp: *mut sign_T,
) -> linenr_T {
    unsafe {
        let ns = group_get_ns(group);
        if ns < 0 || (!group.is_null() && ns == 0) {
            return 0;
        }

        let mark = marktree_lookup_ns(
            (*buf).b_marktree.as_mut_ptr(),
            ns as u32,
            *id,
            false,
            ptr::null_mut(),
        );
        if mark.pos.row >= 0 {
            buf_set_sign(buf, id, group, prio, mark.pos.row + 1, sp);
        }
        mark.pos.row + 1
    }
}

unsafe fn buf_findsign(buf: *mut buf_T, id: c_int, group: *mut c_char) -> c_int {
    unsafe {
        let ns = group_get_ns(group);
        if ns < 0 || (!group.is_null() && ns == 0) {
            return 0;
        }
        marktree_lookup_ns(
            (*buf).b_marktree.as_mut_ptr(),
            ns as u32,
            id as u32,
            false,
            ptr::null_mut(),
        )
        .pos
        .row + 1
    }
}

unsafe fn sign_row_cmp_slice(s1: &MTKey, s2: &MTKey) -> Ordering {
    unsafe {
        if s1.pos.row != s2.pos.row {
            return if s1.pos.row > s2.pos.row {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        let sh1 = decor_find_sign(mt_decor(*s1));
        let sh2 = decor_find_sign(mt_decor(*s2));
        assert!(!sh1.is_null() && !sh2.is_null());
        let si1 = SignItem { sh: sh1, id: s1.id };
        let si2 = SignItem { sh: sh2, id: s2.id };

        let cmp = sign_item_cmp((&raw const si1).cast(), (&raw const si2).cast());
        if cmp < 0 {
            Ordering::Less
        } else if cmp > 0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

unsafe fn buf_delete_signs(
    buf: *mut buf_T,
    group: *mut c_char,
    id: c_int,
    atlnum: linenr_T,
) -> c_int {
    unsafe {
        let ns = group_get_ns(group);
        if ns < 0 {
            return FAIL;
        }

        let mut itr: [MarkTreeIter; 1] = std::mem::zeroed();
        let row = if atlnum > 0 { atlnum - 1 } else { 0 };
        let mut signs: KVec<MTKey> = KVec::new();
        let b = (*buf).b_marktree.as_mut_ptr();

        if atlnum > 0 {
            if !marktree_itr_get_overlap(b, row, 0, itr.as_mut_ptr()) {
                return FAIL;
            }

            let mut pair: MTPair = std::mem::zeroed();
            while marktree_itr_step_overlap(b, itr.as_mut_ptr(), &raw mut pair) {
                if (ns == i64::from(u32::MAX) || ns == i64::from(pair.start.ns))
                    && mt_decor_sign(pair.start)
                {
                    signs.push(pair.start);
                }
            }
        } else {
            marktree_itr_get(b, 0, 0, itr.as_mut_ptr());
        }

        while !itr[0].x.is_null() {
            let mark = marktree_itr_current(itr.as_mut_ptr());
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
                    marktree_itr_next(b, itr.as_mut_ptr());
                } else {
                    extmark_del(buf, itr.as_mut_ptr(), mark, true);
                }
            } else {
                marktree_itr_next(b, itr.as_mut_ptr());
            }
        }

        if signs.size != 0 {
            let slice = std::slice::from_raw_parts_mut(signs.items, signs.size);
            slice.sort_by(|a, b| sign_row_cmp_slice(a, b));
            let first = *signs.items;
            extmark_del_id(buf, first.ns, first.id);
            signs.destroy();
        } else if atlnum > 0 {
            return FAIL;
        }

        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn buf_has_signs(buf: *const buf_T) -> bool {
    unsafe { buf_meta_total(buf, kMTMetaSignHL) + buf_meta_total(buf, kMTMetaSignText) != 0 }
}

unsafe fn sign_list_placed(rbuf: *mut buf_T, group: *mut c_char) {
    unsafe {
        let mut lbuf = [0 as c_char; MSG_BUF_LEN];
        let mut namebuf = [0 as c_char; MSG_BUF_LEN];
        let mut groupbuf = [0 as c_char; MSG_BUF_LEN];
        let mut buf = if !rbuf.is_null() { rbuf } else { firstbuf };
        let ns = group_get_ns(group);

        msg_puts_title(gettext(c"\n--- Signs ---".as_ptr()));

        while !buf.is_null() && !got_int {
            if buf_has_signs(buf) {
                msg_putchar(b'\n' as c_int);
                let fname = if !(*buf).b_fname.is_null() {
                    (*buf).b_fname.cast_const()
                } else {
                    gettext(c"Untitled".as_ptr()).cast_const()
                };
                vim_snprintf!(
                    lbuf.as_mut_ptr(),
                    MSG_BUF_LEN,
                    gettext(c"Signs for %s:".as_ptr()),
                    fname,
                );
                msg_puts_hl(lbuf.as_ptr(), HLF_D, false);
            }

            if ns >= 0 {
                let mut itr: [MarkTreeIter; 1] = std::mem::zeroed();
                let mut signs: KVec<MTKey> = KVec::new();
                let b = (*buf).b_marktree.as_mut_ptr();
                marktree_itr_get(b, 0, 0, itr.as_mut_ptr());

                while !itr[0].x.is_null() {
                    let mark = marktree_itr_current(itr.as_mut_ptr());
                    if !mt_end(mark)
                        && mt_decor_sign(mark)
                        && (ns == i64::from(u32::MAX) || ns == i64::from(mark.ns))
                    {
                        signs.push(mark);
                    }
                    marktree_itr_next(b, itr.as_mut_ptr());
                }

                if signs.size != 0 {
                    let slice = std::slice::from_raw_parts_mut(signs.items, signs.size);
                    slice.sort_by(|a, b| sign_row_cmp_slice(a, b));
                    msg_putchar(b'\n' as c_int);

                    for i in 0..signs.size {
                        namebuf[0] = NUL;
                        groupbuf[0] = NUL;
                        let mark = *signs.items.add(i);

                        let sh = decor_find_sign(mt_decor(mark));
                        if !(*sh).sign_name.is_null() {
                            vim_snprintf!(
                                namebuf.as_mut_ptr(),
                                MSG_BUF_LEN,
                                gettext(c"  name=%s".as_ptr()),
                                sign_get_name(sh),
                            );
                        }
                        if mark.ns != 0 {
                            vim_snprintf!(
                                groupbuf.as_mut_ptr(),
                                MSG_BUF_LEN,
                                gettext(c"  group=%s".as_ptr()),
                                describe_ns(mark.ns as c_int, c"".as_ptr()),
                            );
                        }
                        vim_snprintf!(
                            lbuf.as_mut_ptr(),
                            MSG_BUF_LEN,
                            gettext(c"    line=%d  id=%u%s%s  priority=%d".as_ptr()),
                            mark.pos.row + 1,
                            mark.id,
                            groupbuf.as_ptr(),
                            namebuf.as_ptr(),
                            c_int::from((*sh).priority),
                        );
                        msg_puts(lbuf.as_ptr());
                        if i < signs.size - 1 {
                            msg_putchar(b'\n' as c_int);
                        }
                    }
                    signs.destroy();
                }
            }

            if !rbuf.is_null() {
                return;
            }
            buf = (*buf).b_next;
        }
    }
}

unsafe fn sign_cmd_idx(begin_cmd: *mut c_char, end_cmd: *mut c_char) -> c_int {
    unsafe {
        let save = *end_cmd;
        *end_cmd = NUL;
        let mut idx: usize = 0;
        loop {
            let cmd = CMDS[idx];
            if cmd.is_null() || strcmp(begin_cmd, cmd) == 0 {
                break;
            }
            idx += 1;
        }
        *end_cmd = save;
        idx as c_int
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn describe_sign_text(buf: *mut c_char, sign_text: *mut schar_T) -> usize {
    unsafe {
        let mut p: usize = 0;
        for i in 0..SIGN_WIDTH {
            schar_get(buf.add(p), *sign_text.add(i));
            let len = CStr::from_ptr(buf.add(p)).to_bytes().len();
            if len == 0 {
                break;
            }
            p += len;
        }
        p
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_sign_text(
    sp: *mut sign_T,
    sign_text: *mut schar_T,
    text: *mut c_char,
) -> c_int {
    unsafe {
        let mut endp = text.add(CStr::from_ptr(text).to_bytes().len());
        let mut s = if !sp.is_null() { text } else { endp };
        while s.add(1) < endp {
            if *s == b'\\' as c_char {
                let tail_len = CStr::from_ptr(s.add(1)).to_bytes().len() + 1;
                ptr::copy(s.add(1), s, tail_len);
                endp = endp.sub(1);
            }
            s = s.add(1);
        }

        let mut cells: usize = 0;
        s = text;
        while s < endp {
            let mut c: c_int = 0;
            *sign_text.add(cells) = utfc_ptr2schar(s, &raw mut c);
            if !vim_isprintc(c) {
                break;
            }
            let width = utf_ptr2cells(s) as usize;
            if width == 2 {
                *sign_text.add(cells + 1) = 0;
            }
            cells += width;
            s = s.add(utfc_ptr2len(s) as usize);
        }

        if s != endp || cells > SIGN_WIDTH {
            if !sp.is_null() {
                semsg!(gettext(c"E239: Invalid sign text: %s".as_ptr()), text);
            }
            return FAIL;
        }

        if cells < 1 {
            *sign_text = 0;
        } else if cells == 1 {
            *sign_text.add(1) = schar_from_ascii(b' ');
        }

        OK
    }
}

unsafe fn sign_define_by_name(
    name: *mut c_char,
    icon: *mut c_char,
    text: *mut c_char,
    linehl: *mut c_char,
    texthl: *mut c_char,
    culhl: *mut c_char,
    numhl: *mut c_char,
    prio: c_int,
) -> c_int {
    unsafe {
        let mut key: *mut cstr_t = ptr::null_mut();
        let mut new_sign = false;
        let sp = map_put_ref_cstr_t_ptr_t(&raw mut sign_map, name, &raw mut key, &raw mut new_sign)
            .cast::<*mut sign_T>();

        if new_sign {
            *key = xstrdup(name);
            *sp = xcalloc(1, size_of::<sign_T>()).cast();
            (*(*sp)).sn_name = (*key).cast_mut();
        }

        if !icon.is_null() {
            xfree((*(*sp)).sn_icon.cast());
            (*(*sp)).sn_icon = xstrdup(icon);
            backslash_halve((*(*sp)).sn_icon);
        }

        if !text.is_null() && init_sign_text(*sp, (*(*sp)).sn_text.as_mut_ptr(), text) == FAIL {
            return FAIL;
        }

        (*(*sp)).sn_priority = prio;

        let args = [linehl, texthl, culhl, numhl];
        let hls = [
            &raw mut (*(*sp)).sn_line_hl,
            &raw mut (*(*sp)).sn_text_hl,
            &raw mut (*(*sp)).sn_cul_hl,
            &raw mut (*(*sp)).sn_num_hl,
        ];
        for i in 0..4 {
            if !args[i].is_null() {
                *hls[i] = if *args[i] != NUL {
                    syn_check_group(args[i], CStr::from_ptr(args[i]).to_bytes().len())
                } else {
                    0
                };
            }
        }

        if !new_sign {
            let mut did_redraw = false;
            for i in 0..decor_items.size {
                let sh = decor_items.items.add(i);
                if !(*sh).sign_name.is_null() && strcmp((*sh).sign_name, name) == 0 {
                    (*sh).text = (*(*sp)).sn_text;
                    (*sh).hl_id = (*(*sp)).sn_text_hl;
                    (*sh).line_hl_id = (*(*sp)).sn_line_hl;
                    (*sh).number_hl_id = (*(*sp)).sn_num_hl;
                    (*sh).cursorline_hl_id = (*(*sp)).sn_cul_hl;
                    if !did_redraw {
                        let mut wp = firstwin;
                        while !wp.is_null() {
                            if buf_has_signs((*wp).w_buffer) {
                                redraw_buf_later((*wp).w_buffer, UPD_NOT_VALID);
                            }
                            wp = (*wp).w_next;
                        }
                        did_redraw = true;
                    }
                }
            }
        }
        OK
    }
}

unsafe fn sign_undefine_by_name(name: *const c_char) -> c_int {
    unsafe {
        let sp = map_del_cstr_t_ptr_t(&raw mut sign_map, name, ptr::null_mut()).cast::<sign_T>();
        if sp.is_null() {
            semsg!(gettext(c"E155: Unknown sign: %s".as_ptr()), name);
            return FAIL;
        }

        xfree((*sp).sn_name.cast());
        xfree((*sp).sn_icon.cast());
        xfree(sp.cast());
        OK
    }
}

unsafe fn sign_list_defined(sp: *mut sign_T) {
    unsafe {
        smsg!(0, c"sign %s".as_ptr(), (*sp).sn_name);
        if !(*sp).sn_icon.is_null() {
            msg_puts(c" icon=".as_ptr());
            msg_outtrans((*sp).sn_icon, 0, false);
            msg_puts(gettext(c" (not supported)".as_ptr()));
        }
        if (*sp).sn_text[0] != 0 {
            msg_puts(c" text=".as_ptr());
            let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
            describe_sign_text(buf.as_mut_ptr(), (*sp).sn_text.as_mut_ptr());
            msg_outtrans(buf.as_ptr(), 0, false);
        }
        if (*sp).sn_priority > 0 {
            let mut lbuf = [0 as c_char; MSG_BUF_LEN];
            vim_snprintf!(
                lbuf.as_mut_ptr(),
                MSG_BUF_LEN,
                c" priority=%d".as_ptr(),
                (*sp).sn_priority,
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
            (*sp).sn_line_hl,
            (*sp).sn_text_hl,
            (*sp).sn_cul_hl,
            (*sp).sn_num_hl,
        ];
        for i in 0..4 {
            if hls[i] > 0 {
                msg_puts(args[i]);
                let p = get_highlight_name_ext(ptr::null_mut(), hls[i] - 1, false);
                msg_puts(if !p.is_null() { p } else { c"NONE".as_ptr() });
            }
        }
    }
}

unsafe fn sign_list_by_name(name: *mut c_char) {
    unsafe {
        let sp = pmap_get_sign(name);
        if !sp.is_null() {
            sign_list_defined(sp);
        } else {
            semsg!(gettext(c"E155: Unknown sign: %s".as_ptr()), name);
        }
    }
}

unsafe fn sign_place(
    id: *mut u32,
    group: *mut c_char,
    name: *mut c_char,
    buf: *mut buf_T,
    mut lnum: linenr_T,
    mut prio: c_int,
) -> c_int {
    unsafe {
        if !group.is_null() && (*group == b'*' as c_char || *group == NUL) {
            return FAIL;
        }

        let sp = pmap_get_sign(name);
        if sp.is_null() {
            semsg!(gettext(c"E155: Unknown sign: %s".as_ptr()), name);
            return FAIL;
        }

        if prio == -1 {
            prio = if (*sp).sn_priority != -1 {
                (*sp).sn_priority
            } else {
                SIGN_DEF_PRIO
            };
        }

        if lnum > 0 {
            buf_set_sign(buf, id, group, prio, lnum, sp);
        } else {
            lnum = buf_mod_sign(buf, id, group, prio, sp);
        }
        if lnum <= 0 {
            semsg!(
                gettext(c"E885: Not possible to change sign %s".as_ptr()),
                name,
            );
            return FAIL;
        }

        OK
    }
}

unsafe fn sign_unplace_inner(
    buf: *mut buf_T,
    id: c_int,
    group: *mut c_char,
    atlnum: linenr_T,
) -> c_int {
    unsafe {
        if !buf_has_signs(buf) {
            return FAIL;
        }

        if id == 0 || atlnum > 0 || (!group.is_null() && *group == b'*' as c_char) {
            if buf_delete_signs(buf, group, id, atlnum) == 0 {
                return FAIL;
            }
        } else {
            let ns = group_get_ns(group);
            if ns < 0 || !extmark_del_id(buf, ns as u32, id as u32) {
                return FAIL;
            }
        }

        OK
    }
}

unsafe fn sign_unplace(buf: *mut buf_T, id: c_int, group: *mut c_char, atlnum: linenr_T) -> c_int {
    unsafe {
        if !buf.is_null() {
            sign_unplace_inner(buf, id, group, atlnum)
        } else {
            let mut retval = OK;
            let mut cbuf = firstbuf;
            while !cbuf.is_null() {
                if sign_unplace_inner(cbuf, id, group, atlnum) == 0 {
                    retval = FAIL;
                }
                cbuf = (*cbuf).b_next;
            }
            retval
        }
    }
}

unsafe fn sign_jump(id: c_int, group: *mut c_char, buf: *mut buf_T) -> linenr_T {
    unsafe {
        let lnum = buf_findsign(buf, id, group);
        if lnum <= 0 {
            semsg!(gettext(c"E157: Invalid sign ID: %d".as_ptr()), id);
            return -1;
        }

        if !buf_jump_open_win(buf).is_null() {
            (*curwin).w_cursor.lnum = lnum;
            check_cursor_lnum(curwin);
            beginline(BL_WHITE);
        } else {
            if (*buf).b_fname.is_null() {
                emsg(gettext(
                    c"E934: Cannot jump to a buffer that does not have a name".as_ptr(),
                ));
                return -1;
            }
            setpcmark();
            if do_ecmd(
                (*buf).b_fnum,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                lnum,
                if buf_hide(curbuf) { ECMD_HIDE } else { 0 },
                curwin,
            ) == FAIL
            {
                return -1;
            }
        }

        foldOpenCursor();
        lnum
    }
}

unsafe fn sign_define_cmd(name: *mut c_char, mut cmdline: *mut c_char) {
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
            if strncmp(arg, c"icon=".as_ptr(), 5) == 0 {
                icon = arg.add(5);
            } else if strncmp(arg, c"text=".as_ptr(), 5) == 0 {
                text = arg.add(5);
            } else if strncmp(arg, c"linehl=".as_ptr(), 7) == 0 {
                linehl = arg.add(7);
            } else if strncmp(arg, c"texthl=".as_ptr(), 7) == 0 {
                texthl = arg.add(7);
            } else if strncmp(arg, c"culhl=".as_ptr(), 6) == 0 {
                culhl = arg.add(6);
            } else if strncmp(arg, c"numhl=".as_ptr(), 6) == 0 {
                numhl = arg.add(6);
            } else if strncmp(arg, c"priority=".as_ptr(), 9) == 0 {
                prio = atoi(arg.add(9));
            } else {
                semsg!(gettext(e_invarg2.as_ptr()), arg);
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

unsafe fn sign_place_cmd(
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
                emsg(gettext(e_invarg.as_ptr()));
            } else {
                sign_list_placed(buf, group);
            }
        } else {
            if name.is_null() || buf.is_null() || (!group.is_null() && *group == NUL) {
                emsg(gettext(e_invarg.as_ptr()));
                return;
            }
            let mut uid = id as u32;
            sign_place(&raw mut uid, group, name, buf, lnum, prio);
        }
    }
}

unsafe fn sign_unplace_cmd(
    mut buf: *mut buf_T,
    mut lnum: linenr_T,
    name: *const c_char,
    id: c_int,
    group: *mut c_char,
) {
    unsafe {
        if lnum >= 0 || !name.is_null() || (!group.is_null() && *group == NUL) {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        if id == -1 {
            lnum = (*curwin).w_cursor.lnum;
            buf = (*curwin).w_buffer;
        }

        if sign_unplace(buf, id.max(0), group, lnum) == 0 && lnum > 0 {
            emsg(gettext(c"E159: Missing sign number".as_ptr()));
        }
    }
}

unsafe fn sign_jump_cmd(
    buf: *mut buf_T,
    lnum: linenr_T,
    name: *const c_char,
    id: c_int,
    group: *mut c_char,
) {
    unsafe {
        if name.is_null() && group.is_null() && id == -1 {
            emsg(gettext(e_argreq.as_ptr()));
            return;
        }

        if buf.is_null() || (!group.is_null() && *group == NUL) || lnum >= 0 || !name.is_null() {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        sign_jump(id, group, buf);
    }
}

unsafe fn parse_sign_cmd_args(
    cmd: c_int,
    mut arg: *mut c_char,
    name: *mut *mut c_char,
    id: *mut c_int,
    group: *mut *mut c_char,
    prio: *mut c_int,
    buf: *mut *mut buf_T,
    lnum: *mut linenr_T,
) -> c_int {
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
            if strncmp(arg, c"line=".as_ptr(), 5) == 0 {
                arg = arg.add(5);
                *lnum = atoi(arg);
                arg = skiptowhite(arg);
                lnum_arg = true;
            } else if strncmp(arg, c"*".as_ptr(), 1) == 0 && cmd == SIGNCMD_UNPLACE {
                if *id != -1 {
                    emsg(gettext(e_invarg.as_ptr()));
                    return FAIL;
                }
                *id = -2;
                arg = skiptowhite(arg.add(1));
            } else if strncmp(arg, c"name=".as_ptr(), 5) == 0 {
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
            } else if strncmp(arg, c"group=".as_ptr(), 6) == 0 {
                arg = arg.add(6);
                *group = arg;
                arg = skiptowhite(arg);
                if *arg != NUL {
                    *arg = NUL;
                    arg = arg.add(1);
                }
            } else if strncmp(arg, c"priority=".as_ptr(), 9) == 0 {
                arg = arg.add(9);
                *prio = atoi(arg);
                arg = skiptowhite(arg);
            } else if strncmp(arg, c"file=".as_ptr(), 5) == 0 {
                arg = arg.add(5);
                filename = arg;
                *buf = buflist_findname_exp(arg);
                break;
            } else if strncmp(arg, c"buffer=".as_ptr(), 7) == 0 {
                arg = arg.add(7);
                filename = arg;
                *buf = buflist_findnr(getdigits_int(&raw mut arg, true, 0));
                if *skipwhite(arg) != NUL {
                    semsg!(gettext(e_trailing_arg.as_ptr()), arg);
                }
                break;
            } else {
                emsg(gettext(e_invarg.as_ptr()));
                return FAIL;
            }
            arg = skipwhite(arg);
        }

        if !filename.is_null() && (*buf).is_null() {
            semsg!(gettext(e_invalid_buffer_name_str.as_ptr()), filename);
            return FAIL;
        }

        if filename.is_null() && ((cmd == SIGNCMD_PLACE && lnum_arg) || cmd == SIGNCMD_JUMP) {
            *buf = (*curwin).w_buffer;
        }
        OK
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ex_sign(eap: *mut exarg_T) {
    unsafe {
        let mut arg = (*eap).arg;

        let mut p = skiptowhite(arg);
        let idx = sign_cmd_idx(arg, p);
        if idx == SIGNCMD_LAST {
            semsg!(gettext(c"E160: Unknown sign command: %s".as_ptr()), arg);
            return;
        }
        arg = skipwhite(p);

        if idx <= SIGNCMD_LIST {
            if idx == SIGNCMD_LIST && *arg == NUL {
                for i in 0..sign_map.set.h.n_keys {
                    let sp = (*sign_map.values.add(i as usize)).cast::<sign_T>();
                    sign_list_defined(sp);
                }
            } else if *arg == NUL {
                emsg(gettext(c"E156: Missing sign name".as_ptr()));
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

            if parse_sign_cmd_args(
                idx,
                arg,
                &raw mut name,
                &raw mut id,
                &raw mut group,
                &raw mut prio,
                &raw mut buf,
                &raw mut lnum,
            ) == FAIL
            {
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
}

unsafe fn sign_get_info_dict(sp: *mut sign_T) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();

        tv_dict_add_str(d, c"name".as_ptr(), 4, (*sp).sn_name);

        if !(*sp).sn_icon.is_null() {
            tv_dict_add_str(d, c"icon".as_ptr(), 4, (*sp).sn_icon);
        }
        if (*sp).sn_text[0] != 0 {
            let mut buf = [0 as c_char; SIGN_WIDTH * MAX_SCHAR_SIZE];
            describe_sign_text(buf.as_mut_ptr(), (*sp).sn_text.as_mut_ptr());
            tv_dict_add_str(d, c"text".as_ptr(), 4, buf.as_ptr());
        }
        if (*sp).sn_priority > 0 {
            tv_dict_add_nr(d, c"priority".as_ptr(), 8, i64::from((*sp).sn_priority));
        }
        let args = [
            (c"linehl".as_ptr(), 6usize, (*sp).sn_line_hl),
            (c"texthl".as_ptr(), 6usize, (*sp).sn_text_hl),
            (c"culhl".as_ptr(), 5usize, (*sp).sn_cul_hl),
            (c"numhl".as_ptr(), 5usize, (*sp).sn_num_hl),
        ];
        for (name, len, hl) in args {
            if hl > 0 {
                let p = get_highlight_name_ext(ptr::null_mut(), hl - 1, false);
                if p.is_null() {
                    tv_dict_add_str_len(d, name, len, c"NONE".as_ptr(), 4);
                } else {
                    tv_dict_add_str(d, name, len, p);
                }
            }
        }
        d
    }
}

unsafe fn sign_get_placed_info_dict(mark: MTKey) -> *mut dict_T {
    unsafe {
        let d = tv_dict_alloc();
        let sh = decor_find_sign(mt_decor(mark));

        tv_dict_add_str(d, c"name".as_ptr(), 4, sign_get_name(sh));
        tv_dict_add_nr(d, c"id".as_ptr(), 2, i64::from(mark.id as c_int));
        tv_dict_add_str(
            d,
            c"group".as_ptr(),
            5,
            describe_ns(mark.ns as c_int, c"".as_ptr()),
        );
        tv_dict_add_nr(d, c"lnum".as_ptr(), 4, i64::from(mark.pos.row + 1));
        tv_dict_add_nr(d, c"priority".as_ptr(), 8, i64::from((*sh).priority));
        d
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_buffer_signs(buf: *mut buf_T) -> *mut list_T {
    unsafe {
        let l = tv_list_alloc(kListLenMayKnow);
        let mut itr: [MarkTreeIter; 1] = std::mem::zeroed();
        let b = (*buf).b_marktree.as_mut_ptr();
        marktree_itr_get(b, 0, 0, itr.as_mut_ptr());

        while !itr[0].x.is_null() {
            let mark = marktree_itr_current(itr.as_mut_ptr());
            if !mt_end(mark) && mt_decor_sign(mark) {
                tv_list_append_dict(l, sign_get_placed_info_dict(mark));
            }
            marktree_itr_next(b, itr.as_mut_ptr());
        }

        l
    }
}

unsafe fn sign_get_placed_in_buf(
    buf: *mut buf_T,
    lnum: linenr_T,
    sign_id: c_int,
    group: *const c_char,
    retlist: *mut list_T,
) {
    unsafe {
        let d = tv_dict_alloc();
        tv_list_append_dict(retlist, d);

        tv_dict_add_nr(d, c"bufnr".as_ptr(), 5, i64::from((*buf).b_fnum));

        let l = tv_list_alloc(kListLenMayKnow);
        tv_dict_add_list(d, c"signs".as_ptr(), 5, l);

        let ns = group_get_ns(group);
        if !buf_has_signs(buf) || ns < 0 {
            return;
        }

        let mut itr: [MarkTreeIter; 1] = std::mem::zeroed();
        let mut signs: KVec<MTKey> = KVec::new();
        let b = (*buf).b_marktree.as_mut_ptr();
        marktree_itr_get(b, if lnum != 0 { lnum - 1 } else { 0 }, 0, itr.as_mut_ptr());

        while !itr[0].x.is_null() {
            let mark = marktree_itr_current(itr.as_mut_ptr());
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
            marktree_itr_next(b, itr.as_mut_ptr());
        }

        if signs.size != 0 {
            let slice = std::slice::from_raw_parts_mut(signs.items, signs.size);
            slice.sort_by(|a, b| sign_row_cmp_slice(a, b));
            for i in 0..signs.size {
                tv_list_append_dict(l, sign_get_placed_info_dict(*signs.items.add(i)));
            }
            signs.destroy();
        }
    }
}

unsafe fn sign_get_placed(
    buf: *mut buf_T,
    lnum: linenr_T,
    id: c_int,
    group: *const c_char,
    retlist: *mut list_T,
) {
    unsafe {
        if !buf.is_null() {
            sign_get_placed_in_buf(buf, lnum, id, group, retlist);
        } else {
            let mut cbuf = firstbuf;
            while !cbuf.is_null() {
                if buf_has_signs(cbuf) {
                    sign_get_placed_in_buf(cbuf, 0, id, group, retlist);
                }
                cbuf = (*cbuf).b_next;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_signs() {
    unsafe {
        let mut names: KVec<cstr_t> = KVec::new();
        for i in 0..sign_map.set.h.n_keys {
            names.push(*sign_map.set.keys.add(i as usize));
        }
        for i in 0..names.size {
            sign_undefine_by_name(*names.items.add(i));
        }
        names.destroy();
    }
}

unsafe fn get_nth_sign_name(idx: c_int) -> *mut c_char {
    unsafe {
        if idx >= 0 && (idx as u32) < sign_map.set.h.n_keys {
            return (*sign_map.set.keys.add(idx as usize)).cast_mut();
        }
        ptr::null_mut()
    }
}

unsafe fn get_nth_sign_group_name(idx: c_int) -> *mut c_char {
    unsafe {
        if idx >= 0 && (idx as usize) < sign_ns.size {
            return describe_ns(*sign_ns.items.add(idx as usize) as NS, c"".as_ptr()).cast_mut();
        }
        ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_sign_name(_xp: *mut expand_T, idx: c_int) -> *mut c_char {
    unsafe {
        match expand_what {
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
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_context_in_sign_cmd(xp: *mut expand_T, arg: *mut c_char) {
    unsafe {
        (*xp).xp_context = EXPAND_SIGN;
        expand_what = EXP_SUBCMD;
        (*xp).xp_pattern = arg;

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
            (*xp).xp_pattern = last;
            match cmd_idx {
                SIGNCMD_DEFINE => expand_what = EXP_DEFINE,
                SIGNCMD_PLACE => {
                    if ascii_isdigit(*begin_subcmd_args as c_int) {
                        expand_what = EXP_PLACE;
                    } else {
                        expand_what = EXP_LIST;
                    }
                }
                SIGNCMD_LIST | SIGNCMD_UNDEFINE => expand_what = EXP_SIGN_NAMES,
                SIGNCMD_JUMP | SIGNCMD_UNPLACE => expand_what = EXP_UNPLACE,
                _ => (*xp).xp_context = EXPAND_NOTHING,
            }
        } else {
            (*xp).xp_pattern = p.add(1);
            match cmd_idx {
                SIGNCMD_DEFINE => {
                    if strncmp(last, c"texthl".as_ptr(), 6) == 0
                        || strncmp(last, c"linehl".as_ptr(), 6) == 0
                        || strncmp(last, c"culhl".as_ptr(), 5) == 0
                        || strncmp(last, c"numhl".as_ptr(), 5) == 0
                    {
                        (*xp).xp_context = EXPAND_HIGHLIGHT;
                    } else if strncmp(last, c"icon".as_ptr(), 4) == 0 {
                        (*xp).xp_context = EXPAND_FILES;
                    } else {
                        (*xp).xp_context = EXPAND_NOTHING;
                    }
                }
                SIGNCMD_PLACE => {
                    if strncmp(last, c"name".as_ptr(), 4) == 0 {
                        expand_what = EXP_SIGN_NAMES;
                    } else if strncmp(last, c"group".as_ptr(), 5) == 0 {
                        expand_what = EXP_SIGN_GROUPS;
                    } else if strncmp(last, c"file".as_ptr(), 4) == 0 {
                        (*xp).xp_context = EXPAND_BUFFERS;
                    } else {
                        (*xp).xp_context = EXPAND_NOTHING;
                    }
                }
                SIGNCMD_UNPLACE | SIGNCMD_JUMP => {
                    if strncmp(last, c"group".as_ptr(), 5) == 0 {
                        expand_what = EXP_SIGN_GROUPS;
                    } else if strncmp(last, c"file".as_ptr(), 4) == 0 {
                        (*xp).xp_context = EXPAND_BUFFERS;
                    } else {
                        (*xp).xp_context = EXPAND_NOTHING;
                    }
                }
                _ => (*xp).xp_context = EXPAND_NOTHING,
            }
        }
    }
}

unsafe fn sign_define_from_dict(mut name: *mut c_char, dict: *mut dict_T) -> c_int {
    unsafe {
        if name.is_null() {
            name = tv_dict_get_string(dict, c"name".as_ptr(), false);
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
            icon = tv_dict_get_string(dict, c"icon".as_ptr(), false);
            linehl = tv_dict_get_string(dict, c"linehl".as_ptr(), false);
            text = tv_dict_get_string(dict, c"text".as_ptr(), false);
            texthl = tv_dict_get_string(dict, c"texthl".as_ptr(), false);
            culhl = tv_dict_get_string(dict, c"culhl".as_ptr(), false);
            numhl = tv_dict_get_string(dict, c"numhl".as_ptr(), false);
            prio = tv_dict_get_number_def(dict, c"priority".as_ptr(), -1) as c_int;
        }

        sign_define_by_name(name, icon, text, linehl, texthl, culhl, numhl, prio) - 1
    }
}

unsafe fn sign_define_multiple(l: *const list_T, retlist: *mut list_T) {
    unsafe {
        if l.is_null() {
            return;
        }
        let mut li = (*l).lv_first;
        while !li.is_null() {
            let mut retval: c_int = -1;
            if (*li).li_tv.v_type == VarType::VAR_DICT {
                retval = sign_define_from_dict(
                    ptr::null_mut(),
                    (*li).li_tv.vval.v_dict.cast::<dict_T>(),
                );
            } else {
                emsg(gettext(e_dictreq.as_ptr()));
            }
            tv_list_append_number(retlist, i64::from(retval));
            li = (*li).li_next;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_define(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_LIST
            && (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            sign_define_multiple((*argvars).vval.v_list, (*rettv).vval.v_list);
            return;
        }

        (*rettv).vval.v_number = -1;

        let name = tv_get_string_chk(argvars).cast_mut();
        if name.is_null() {
            return;
        }

        if tv_check_for_opt_dict_arg(argvars, 1) == FAIL {
            return;
        }

        let d = if (*argvars.add(1)).v_type == VarType::VAR_DICT {
            (*argvars.add(1)).vval.v_dict.cast::<dict_T>()
        } else {
            ptr::null_mut()
        };
        (*rettv).vval.v_number = i64::from(sign_define_from_dict(name, d));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_getdefined(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, 0);

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            for i in 0..sign_map.set.h.n_keys {
                let sp = (*sign_map.values.add(i as usize)).cast::<sign_T>();
                tv_list_append_dict((*rettv).vval.v_list, sign_get_info_dict(sp));
            }
        } else {
            let sp = pmap_get_sign(tv_get_string(argvars));
            if !sp.is_null() {
                tv_list_append_dict((*rettv).vval.v_list, sign_get_info_dict(sp));
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_getplaced(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut lnum: linenr_T = 0;
        let mut sign_id: c_int = 0;
        let mut group: *const c_char = ptr::null();
        let mut notanum = false;

        tv_list_alloc_ret(rettv, 0);

        if (*argvars).v_type != VarType::VAR_UNKNOWN {
            buf = get_buf_arg(argvars);
            if buf.is_null() {
                return;
            }

            if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
                if tv_check_for_nonnull_dict_arg(argvars, 1) == FAIL {
                    return;
                }
                let dict = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
                let di_lnum = tv_dict_find(dict, c"lnum".as_ptr(), -1);
                if !di_lnum.is_null() {
                    lnum = tv_get_lnum(&raw const (*di_lnum).di_tv);
                    if lnum <= 0 {
                        return;
                    }
                }
                let di_id = tv_dict_find(dict, c"id".as_ptr(), -1);
                if !di_id.is_null() {
                    sign_id =
                        tv_get_number_chk(&raw const (*di_id).di_tv, &raw mut notanum) as c_int;
                    if notanum {
                        return;
                    }
                }
                let di_group = tv_dict_find(dict, c"group".as_ptr(), -1);
                if !di_group.is_null() {
                    group = tv_get_string_chk(&raw const (*di_group).di_tv);
                    if group.is_null() {
                        return;
                    }
                    if *group == NUL {
                        group = ptr::null();
                    }
                }
            }
        }

        sign_get_placed(buf, lnum, sign_id, group, (*rettv).vval.v_list);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_jump(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        (*rettv).vval.v_number = -1;

        let mut notanum = false;
        let id = tv_get_number_chk(argvars, &raw mut notanum) as c_int;
        if notanum {
            return;
        }
        if id <= 0 {
            emsg(gettext(e_invarg.as_ptr()));
            return;
        }

        let mut group = tv_get_string_chk(argvars.add(1)).cast_mut();
        if group.is_null() {
            return;
        }
        if *group == NUL {
            group = ptr::null_mut();
        }

        let buf = get_buf_arg(argvars.add(2));
        if buf.is_null() {
            return;
        }

        (*rettv).vval.v_number = i64::from(sign_jump(id, group, buf));
    }
}

unsafe fn sign_place_from_dict(
    mut id_tv: *mut typval_T,
    mut group_tv: *mut typval_T,
    mut name_tv: *mut typval_T,
    mut buf_tv: *mut typval_T,
    dict: *mut dict_T,
) -> c_int {
    unsafe {
        let mut id: c_int = 0;
        let mut notanum = false;
        if id_tv.is_null() {
            let di = tv_dict_find(dict, c"id".as_ptr(), -1);
            if !di.is_null() {
                id_tv = &raw mut (*di).di_tv;
            }
        }
        if !id_tv.is_null() {
            id = tv_get_number_chk(id_tv, &raw mut notanum) as c_int;
            if notanum {
                return -1;
            }
            if id < 0 {
                emsg(gettext(e_invarg.as_ptr()));
                return -1;
            }
        }

        let mut group: *mut c_char = ptr::null_mut();
        if group_tv.is_null() {
            let di = tv_dict_find(dict, c"group".as_ptr(), -1);
            if !di.is_null() {
                group_tv = &raw mut (*di).di_tv;
            }
        }
        if !group_tv.is_null() {
            group = tv_get_string_chk(group_tv).cast_mut();
            if group.is_null() {
                return -1;
            }
            if *group == NUL {
                group = ptr::null_mut();
            }
        }

        if name_tv.is_null() {
            let di = tv_dict_find(dict, c"name".as_ptr(), -1);
            if !di.is_null() {
                name_tv = &raw mut (*di).di_tv;
            }
        }
        if name_tv.is_null() {
            return -1;
        }
        let name = tv_get_string_chk(name_tv).cast_mut();
        if name.is_null() {
            return -1;
        }

        if buf_tv.is_null() {
            let di = tv_dict_find(dict, c"buffer".as_ptr(), -1);
            if !di.is_null() {
                buf_tv = &raw mut (*di).di_tv;
            }
        }
        if buf_tv.is_null() {
            return -1;
        }
        let buf = get_buf_arg(buf_tv);
        if buf.is_null() {
            return -1;
        }

        let mut lnum: linenr_T = 0;
        let di_lnum = tv_dict_find(dict, c"lnum".as_ptr(), -1);
        if !di_lnum.is_null() {
            lnum = tv_get_lnum(&raw const (*di_lnum).di_tv);
            if lnum <= 0 {
                emsg(gettext(e_invarg.as_ptr()));
                return -1;
            }
        }

        let mut prio: c_int = -1;
        let di_prio = tv_dict_find(dict, c"priority".as_ptr(), -1);
        if !di_prio.is_null() {
            prio = tv_get_number_chk(&raw const (*di_prio).di_tv, &raw mut notanum) as c_int;
            if notanum {
                return -1;
            }
        }

        let mut uid = id as u32;
        if sign_place(&raw mut uid, group, name, buf, lnum, prio) == OK {
            return uid as c_int;
        }

        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_place(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut dict: *mut dict_T = ptr::null_mut();

        (*rettv).vval.v_number = -1;

        if (*argvars.add(4)).v_type != VarType::VAR_UNKNOWN {
            if tv_check_for_nonnull_dict_arg(argvars, 4) == FAIL {
                return;
            }
            dict = (*argvars.add(4)).vval.v_dict.cast::<dict_T>();
        }

        (*rettv).vval.v_number = i64::from(sign_place_from_dict(
            argvars,
            argvars.add(1),
            argvars.add(2),
            argvars.add(3),
            dict,
        ));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_placelist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list;
        if !l.is_null() {
            let mut li = (*l).lv_first;
            while !li.is_null() {
                let mut sign_id: c_int = -1;
                if (*li).li_tv.v_type == VarType::VAR_DICT {
                    sign_id = sign_place_from_dict(
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        (*li).li_tv.vval.v_dict.cast::<dict_T>(),
                    );
                } else {
                    emsg(gettext(e_dictreq.as_ptr()));
                }
                tv_list_append_number((*rettv).vval.v_list, i64::from(sign_id));
                li = (*li).li_next;
            }
        }
    }
}

unsafe fn sign_undefine_multiple(l: *const list_T, retlist: *mut list_T) {
    unsafe {
        if l.is_null() {
            return;
        }
        let mut li = (*l).lv_first;
        while !li.is_null() {
            let mut retval: c_int = -1;
            let name = tv_get_string_chk(&raw const (*li).li_tv);
            if !name.is_null() && sign_undefine_by_name(name) == OK {
                retval = 0;
            }
            tv_list_append_number(retlist, i64::from(retval));
            li = (*li).li_next;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_undefine(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        if (*argvars).v_type == VarType::VAR_LIST
            && (*argvars.add(1)).v_type == VarType::VAR_UNKNOWN
        {
            tv_list_alloc_ret(rettv, kListLenMayKnow);
            sign_undefine_multiple((*argvars).vval.v_list, (*rettv).vval.v_list);
            return;
        }

        (*rettv).vval.v_number = -1;

        if (*argvars).v_type == VarType::VAR_UNKNOWN {
            free_signs();
            (*rettv).vval.v_number = 0;
        } else {
            let name = tv_get_string_chk(argvars);
            if name.is_null() {
                return;
            }
            if sign_undefine_by_name(name) == OK {
                (*rettv).vval.v_number = 0;
            }
        }
    }
}

unsafe fn sign_unplace_from_dict(group_tv: *mut typval_T, dict: *mut dict_T) -> c_int {
    unsafe {
        let mut id: c_int = 0;
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut group = if !group_tv.is_null() {
            tv_get_string(group_tv).cast_mut()
        } else {
            tv_dict_get_string(dict, c"group".as_ptr(), false)
        };
        if !group.is_null() && *group == NUL {
            group = ptr::null_mut();
        }

        if !dict.is_null() {
            let di_buf = tv_dict_find(dict, c"buffer".as_ptr(), -1);
            if !di_buf.is_null() {
                buf = get_buf_arg(&raw mut (*di_buf).di_tv);
                if buf.is_null() {
                    return -1;
                }
            }
            if !tv_dict_find(dict, c"id".as_ptr(), -1).is_null() {
                id = tv_dict_get_number(dict, c"id".as_ptr()) as c_int;
                if id <= 0 {
                    emsg(gettext(e_invarg.as_ptr()));
                    return -1;
                }
            }
        }

        sign_unplace(buf, id, group, 0) - 1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_unplace(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        let mut dict: *mut dict_T = ptr::null_mut();

        (*rettv).vval.v_number = -1;

        if tv_check_for_string_arg(argvars, 0) == FAIL
            || tv_check_for_opt_dict_arg(argvars, 1) == FAIL
        {
            return;
        }

        if (*argvars.add(1)).v_type != VarType::VAR_UNKNOWN {
            dict = (*argvars.add(1)).vval.v_dict.cast::<dict_T>();
        }

        (*rettv).vval.v_number = i64::from(sign_unplace_from_dict(argvars, dict));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_sign_unplacelist(
    argvars: *mut typval_T,
    rettv: *mut typval_T,
    _fptr: EvalFuncData,
) {
    unsafe {
        tv_list_alloc_ret(rettv, kListLenMayKnow);

        if (*argvars).v_type != VarType::VAR_LIST {
            emsg(gettext(e_listreq.as_ptr()));
            return;
        }

        let l = (*argvars).vval.v_list;
        if !l.is_null() {
            let mut li = (*l).lv_first;
            while !li.is_null() {
                let mut retval: c_int = -1;
                if (*li).li_tv.v_type == VarType::VAR_DICT {
                    retval = sign_unplace_from_dict(
                        ptr::null_mut(),
                        (*li).li_tv.vval.v_dict.cast::<dict_T>(),
                    );
                } else {
                    emsg(gettext(e_dictreq.as_ptr()));
                }
                tv_list_append_number((*rettv).vval.v_list, i64::from(retval));
                li = (*li).li_next;
            }
        }
    }
}
