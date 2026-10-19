//! Code for spell checking (`src/nvim/spell.c`).
#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]
use crate::api::command::{exarg_T, regmatch_T};
use crate::api::options::bufref_T;
use crate::api::private::validate::{Object, ObjectData, ObjectType};
use crate::api::tabpage::win_T;
use crate::autocmd::apply_autocmds;
#[allow(unused_imports)]
use crate::buffer::{buf_is_empty, bufref_valid, set_bufref};
use crate::change::inserted_bytes;
use crate::charset::{getwhitecols, skipbin, skipdigits, skiphex, skipwhite, vim_is_fname_char};
use crate::cursor::{colnr_T, get_cursor_line_len, get_cursor_line_ptr, linenr_T};
use crate::decoration::{
    decor_redraw_col, decor_redraw_line, decor_redraw_reset, decor_state_free,
};
use crate::decoration_provider::decor_providers_invoke_spell;
use crate::drawscreen::redraw_later;
use crate::errors::*;
use crate::eval::deprecated::{buf_T, pos_T, synblock_T};
#[allow(unused_imports)]
use crate::ex_cmds::do_sub_msg;
#[allow(unused_imports)]
use crate::ex_docmd::do_cmdline_cmd;
use crate::extmark::TriState;
use crate::garray::{ga_clear, ga_clear_strings, ga_grow, ga_init, garray_T};
use crate::globals::*;
use crate::hashtab::{
    hash_T, hash_add_item, hash_clear_all, hash_find, hash_hash, hash_init, hash_lookup,
    hashitem_empty,
};
#[allow(unused_imports)]
use crate::insexpand::{ins_compl_add_infercase, ins_compl_check_keys, ins_compl_interrupted};
use crate::log::logmsg;
use crate::mbyte::{
    mb_charlen_len, mb_cptr2char_adv, mb_get_class, mb_islower, mb_isupper, mb_ptr2char_adv,
    mb_stricmp, mb_strnicmp, mb_toupper, utf_char2bytes, utf_class, utf_fold, utf_head_off,
    utf_ptr2char, utfc_ptr2len,
};
#[allow(unused_imports)]
use crate::memline::{
    ml_append, ml_close, ml_delete, ml_get_buf, ml_get_buf_len, ml_open, ml_open_file, ml_replace,
};
use crate::memory::{xcalloc, xfree, xmalloc, xmemcpyz, xmemdupz, xstrdup, xstrlcpy};
#[allow(unused_imports)]
use crate::message::{
    emsg, give_warning, msg_end, msg_ext_set_kind, msg_putchar, msg_puts, msg_start,
};
use crate::message::{semsg, smsg};
#[allow(unused_imports)]
use crate::option::copy_option_part;
#[allow(unused_imports)]
use crate::option::{
    get_option_value, optval_free, set_option_value_give_err, shortmess, valid_name,
};
use crate::option_vars::*;
use crate::os::fs::os_remove;
use crate::os::input::line_breakcheck;
#[cfg(windows)]
use crate::path::path_to_slash;
use crate::path::{PathCmpFlags, kPathCmpExpand, kPathCmpLiteral, path_equal, path_tail};
#[allow(unused_imports)]
use crate::regexp::{vim_regcomp, vim_regexec, vim_regexec_prog, vim_regfree};
#[allow(unused_imports)]
use crate::runtime::do_in_runtimepath;
#[allow(unused_imports)]
use crate::search::do_search;
#[allow(unused_imports)]
use crate::spellfile::spell_load_file;
use crate::spellsuggest::{
    fromto_T, idx_T, langp_T, salitem_T, slang_T, spell_suggest_list, spelltab_T, wordcount_T,
};
#[allow(unused_imports)]
use crate::strings::vim_snprintf;
#[allow(unused_imports)]
use crate::strings::{concat_str, vim_strchr, xstrnsave};
#[allow(unused_imports)]
use crate::syntax::{syn_get_id, syntax_present};
use crate::undo::u_save_cursor;
#[allow(unused_imports)]
use crate::window::win_valid_any_tab;
use std::ffi::{c_char, c_int, c_void};
use std::ptr;
pub type hlf_T = c_int;
pub type smt_T = c_int;
pub type Direction = c_int;
pub type OptIndex = c_int;
pub type event_T = c_int;
pub use crate::regexp::regprog_T;
const NUL: c_char = 0;
const TAB: c_char = b'\t' as c_char;
const FAIL: c_int = 0;
const OK: c_int = 1;
const FORWARD: c_int = 1;
const BACKWARD: c_int = -1;
const MAXCOL: colnr_T = 0x7fffffff;
const MAXPATHL: usize = 4096;
const IOSIZE: usize = 1025;
const MB_MAXBYTES: usize = 30;
const kNone: TriState = -1;
const kFalse: TriState = 0;
const kTrue: TriState = 1;
const MAXWLEN: usize = 254;
const MAXREGIONS: usize = 8;
const REGION_ALL: c_int = 0xff;
const MAXWORDCOUNT: u16 = 0xffff;
const SY_MAXLEN: usize = 30;
const SP_FORMERROR: c_int = -2;
const WF_REGION: c_int = 0x01;
const WF_ONECAP: c_int = 0x02;
const WF_ALLCAP: c_int = 0x04;
const WF_RARE: c_int = 0x08;
const WF_BANNED: c_int = 0x10;
const WF_FIXCAP: c_int = 0x40;
const WF_KEEPCAP: c_int = 0x80;
const WF_CAPMASK: c_int = WF_ONECAP | WF_ALLCAP | WF_KEEPCAP | WF_FIXCAP;
const WF_HAS_AFF: c_int = 0x0100;
const WF_NEEDCOMP: c_int = 0x0200;
const WF_COMPROOT: c_int = 0x0800;
const WF_NOCOMPBEF: c_int = 0x1000;
const WF_NOCOMPAFT: c_int = 0x2000;
const WFP_RARE: c_int = 0x01;
const WFP_NC: c_int = 0x02;
const WF_RAREPFX: c_int = WFP_RARE << 24;
const WF_PFX_NC: c_int = WFP_NC << 24;
const SP_BANNED: c_int = -1;
const SP_RARE: c_int = 0;
const SP_OK: c_int = 1;
const SP_LOCAL: c_int = 2;
const SP_BAD: c_int = 3;
const FIND_FOLDWORD: c_int = 0;
const FIND_KEEPWORD: c_int = 1;
const FIND_PREFIX: c_int = 2;
const FIND_COMPOUND: c_int = 3;
const FIND_KEEPCOMPOUND: c_int = 4;
const CHAR_OTHER: c_int = 0;
const CHAR_UPPER: c_int = 1;
const CHAR_DIGIT: c_int = 2;
const SMT_ALL: smt_T = 0;
const SMT_BAD: smt_T = 1;
const SMT_RARE: smt_T = 2;
const HLF_SPB: hlf_T = 37;
const HLF_SPC: hlf_T = 38;
const HLF_SPR: hlf_T = 39;
const HLF_SPL: hlf_T = 40;
const HLF_COUNT: hlf_T = 76;
const kOptSpoFlagCamel: u32 = 0x01;
const kOptSpoFlagNoplainbuffer: u32 = 0x02;
const kShmSearch: c_int = b's' as c_int;
const kOptSpell: OptIndex = 287;
const kOptSpelllang: OptIndex = 290;
const OPT_LOCAL: c_int = 0x02;
const UPD_NOT_VALID: c_int = 40;
const DIP_ALL: c_int = 0x01;
const SEARCH_KEEP: c_int = 0x400;
const RE_MAGIC: c_int = 1;
const EVENT_SPELLFILEMISSING: event_T = 108;
const DUMPFLAG_KEEPCASE: c_int = 1;
const DUMPFLAG_COUNT: c_int = 2;
const DUMPFLAG_ICASE: c_int = 4;
const DUMPFLAG_ONECAP: c_int = 8;
const DUMPFLAG_ALLCAP: c_int = 16;
#[repr(C)]
struct matchinf_T {
    mi_lp: *mut langp_T,
    mi_word: *mut c_char,
    mi_end: *mut c_char,
    mi_fend: *mut c_char,
    mi_cend: *mut c_char,
    mi_fword: [c_char; MAXWLEN + 1],
    mi_fwordlen: c_int,
    mi_prefarridx: c_int,
    mi_prefcnt: c_int,
    mi_prefixlen: c_int,
    mi_cprefixlen: c_int,
    mi_compoff: c_int,
    mi_compflags: [u8; MAXWLEN],
    mi_complen: c_int,
    mi_compextra: c_int,
    mi_result: c_int,
    mi_capflags: c_int,
    mi_win: *mut win_T,
    mi_result2: c_int,
    mi_end2: *mut c_char,
}
#[repr(C)]
struct spelload_T {
    sl_lang: [c_char; MAXWLEN + 1],
    sl_slang: *mut slang_T,
    sl_nobreak: c_int,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct syl_item_T {
    sy_chars: [c_char; SY_MAXLEN],
    sy_len: c_int,
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut first_lang: *mut slang_T = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut int_wordlist: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut spelltab: spelltab_T = spelltab_T {
    st_isw: [false; 256],
    st_isu: [false; 256],
    st_fold: [0; 256],
    st_upper: [0; 256],
};
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut did_set_spelltab: bool = false;
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut e_format: *mut c_char = c"E759: Format error in spell file".as_ptr().cast_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut repl_from: *mut c_char = ptr::null_mut();
#[cfg_attr(not(test), unsafe(no_mangle))]
pub static mut repl_to: *mut c_char = ptr::null_mut();
static mut parse_spelllang_recursive: bool = false;
static mut spell_expand_need_cap: bool = false;
unsafe extern "C" {
    fn gettext(msgid: *const c_char) -> *mut c_char;
    fn strlen(s: *const c_char) -> usize;
    fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strcat(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    fn strstr(haystack: *const c_char, needle: *const c_char) -> *const c_char;
}
#[inline]
const fn ascii_isdigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'9' as c_int
}
#[inline]
const fn ascii_isalpha(c: c_char) -> bool {
    (c as u8).is_ascii_alphabetic()
}
#[inline]
const fn ascii_iswhite(c: c_int) -> bool {
    c == b' ' as c_int || c == b'\t' as c_int
}
#[inline]
const fn boolean_obj(val: bool) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeBoolean,
        data: ObjectData { boolean: val },
    }
}
#[inline]
unsafe fn spell_isupper(c: c_int) -> bool {
    unsafe {
        if c >= 128 {
            mb_isupper(c)
        } else if c >= 0 {
            spelltab.st_isu[c as usize]
        } else {
            false
        }
    }
}
#[inline]
unsafe fn spell_tofold(c: c_int) -> c_int {
    unsafe {
        if c >= 128 {
            utf_fold(c)
        } else if c >= 0 {
            spelltab.st_fold[c as usize] as c_int
        } else {
            c
        }
    }
}
#[inline]
unsafe fn spell_toupper(c: c_int) -> c_int {
    unsafe {
        if c >= 128 {
            mb_toupper(c)
        } else if c >= 0 {
            spelltab.st_upper[c as usize] as c_int
        } else {
            c
        }
    }
}
#[inline]
unsafe fn mb_ptr_adv(p: &mut *mut c_char) {
    unsafe {
        *p = (*p).add(utfc_ptr2len(*p) as usize);
    }
}
#[inline]
unsafe fn mb_cptr_adv(p: &mut *const c_char) {
    unsafe {
        *p = (*p).add(utfc_ptr2len(*p) as usize);
    }
}
#[inline]
unsafe fn mb_ptr_back(s: *const c_char, p: &mut *mut c_char) {
    unsafe {
        let off = utf_head_off(s, (*p).sub(1)) + 1;
        *p = (*p).sub(off as usize);
    }
}
#[inline]
unsafe fn langp_entry(ga: *const garray_T, i: usize) -> *mut langp_T {
    unsafe { ((*ga).ga_data.cast::<langp_T>()).add(i) }
}
/// Main spell-checking function.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_check(
    wp: *mut win_T,
    ptr: *mut c_char,
    attrp: *mut hlf_T,
    capcol: *mut c_int,
    docount: bool,
) -> usize {
    unsafe {
        if (*ptr as u8) <= b' ' {
            return 1;
        }
        let w_s = (*wp).w_s.cast::<synblock_T>();
        if (*w_s).b_langp.ga_len <= 0 {
            return 1;
        }
        let mut nrlen: usize = 0;
        let mut wrongcaplen: usize = 0;
        let mut count_word = docount;
        let use_camel_case = ((*w_s).b_p_spo_flags & kOptSpoFlagCamel) != 0;
        let mut is_camel_case = false;
        let mut mi: matchinf_T = std::mem::MaybeUninit::zeroed().assume_init();
        if *ptr >= b'0' as c_char && *ptr <= b'9' as c_char {
            if *ptr == b'0' as c_char
                && (*ptr.add(1) == b'b' as c_char || *ptr.add(1) == b'B' as c_char)
            {
                mi.mi_end = skipbin(ptr.add(2)).cast_mut();
            } else if *ptr == b'0' as c_char
                && (*ptr.add(1) == b'x' as c_char || *ptr.add(1) == b'X' as c_char)
            {
                mi.mi_end = skiphex(ptr.add(2));
            } else {
                mi.mi_end = skipdigits(ptr);
            }
            nrlen = mi.mi_end.offset_from(ptr) as usize;
        }
        mi.mi_word = ptr;
        mi.mi_fend = ptr;
        if spell_iswordp(mi.mi_fend, wp) {
            if use_camel_case {
                mi.mi_fend = advance_camelcase_word(ptr, wp, &raw mut is_camel_case);
            } else {
                loop {
                    mb_ptr_adv(&mut mi.mi_fend);
                    if *mi.mi_fend == NUL || !spell_iswordp(mi.mi_fend, wp) {
                        break;
                    }
                }
            }
            if !capcol.is_null() && *capcol == 0 && !(*w_s).b_cap_prog.is_null() {
                let c = utf_ptr2char(ptr);
                if !spell_isupper(c) {
                    wrongcaplen = mi.mi_fend.offset_from(ptr) as usize;
                }
            }
        }
        if !capcol.is_null() {
            *capcol = -1;
        }
        mi.mi_end = mi.mi_fend;
        mi.mi_capflags = 0;
        mi.mi_cend = ptr::null_mut();
        mi.mi_win = wp;
        if *mi.mi_fend != NUL {
            mb_ptr_adv(&mut mi.mi_fend);
        }
        spell_casefold(
            wp,
            ptr,
            mi.mi_fend.offset_from(ptr) as c_int,
            (&raw mut mi.mi_fword).cast(),
            (MAXWLEN + 1) as c_int,
        );
        mi.mi_fwordlen = strlen((&raw const mi.mi_fword).cast()) as c_int;
        if is_camel_case && mi.mi_fwordlen > 0 {
            mi.mi_fword[mi.mi_fwordlen as usize - 1] = b' ' as c_char;
        }
        mi.mi_result = SP_BAD;
        mi.mi_result2 = SP_BAD;
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            mi.mi_lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            if (*(*mi.mi_lp).lp_slang).sl_fidxs.is_null() {
                continue;
            }
            find_word(&raw mut mi, FIND_FOLDWORD);
            find_word(&raw mut mi, FIND_KEEPWORD);
            find_prefix(&raw mut mi, FIND_FOLDWORD);
            if (*(*mi.mi_lp).lp_slang).sl_nobreak
                && mi.mi_result == SP_BAD
                && mi.mi_result2 != SP_BAD
            {
                mi.mi_result = mi.mi_result2;
                mi.mi_end = mi.mi_end2;
            }
            if count_word && mi.mi_result == SP_OK {
                count_common_word(
                    (*mi.mi_lp).lp_slang,
                    ptr,
                    mi.mi_end.offset_from(ptr) as c_int,
                    1,
                );
                count_word = false;
            }
        }
        if mi.mi_result != SP_OK {
            if nrlen > 0 {
                if mi.mi_result == SP_BAD || mi.mi_result == SP_BANNED {
                    return nrlen;
                }
            } else if !spell_iswordp_nmw(ptr, wp) {
                if !capcol.is_null() && !(*w_s).b_cap_prog.is_null() {
                    let mut regmatch: regmatch_T = std::mem::MaybeUninit::zeroed().assume_init();
                    regmatch.regprog = (*w_s).b_cap_prog.cast();
                    regmatch.rm_ic = false;
                    let r = vim_regexec(&raw mut regmatch, ptr, 0);
                    (*w_s).b_cap_prog = regmatch.regprog.cast();
                    if r {
                        *capcol = regmatch.endp[0].offset_from(ptr) as c_int;
                    }
                }
                return utfc_ptr2len(ptr) as usize;
            } else if mi.mi_end == ptr {
                mb_ptr_adv(&mut mi.mi_end);
            } else if mi.mi_result == SP_BAD
                && (*(*langp_entry(&raw const (*w_s).b_langp, 0)).lp_slang).sl_nobreak
            {
                let save_result = mi.mi_result;
                mi.mi_lp = langp_entry(&raw const (*w_s).b_langp, 0);
                if !(*(*mi.mi_lp).lp_slang).sl_fidxs.is_null() {
                    let mut p = mi.mi_word;
                    let mut fp = (&raw mut mi.mi_fword).cast::<c_char>();
                    loop {
                        mb_ptr_adv(&mut p);
                        mb_ptr_adv(&mut fp);
                        if p >= mi.mi_end {
                            break;
                        }
                        mi.mi_compoff =
                            fp.offset_from((&raw const mi.mi_fword).cast::<c_char>()) as c_int;
                        find_word(&raw mut mi, FIND_COMPOUND);
                        if mi.mi_result != SP_BAD {
                            mi.mi_end = p;
                            break;
                        }
                    }
                    mi.mi_result = save_result;
                }
            }
            if mi.mi_result == SP_BAD || mi.mi_result == SP_BANNED {
                *attrp = HLF_SPB;
            } else if mi.mi_result == SP_RARE {
                *attrp = HLF_SPR;
            } else {
                *attrp = HLF_SPL;
            }
        }
        if wrongcaplen > 0 && (mi.mi_result == SP_OK || mi.mi_result == SP_RARE) {
            *attrp = HLF_SPC;
            return wrongcaplen;
        }
        mi.mi_end.offset_from(ptr) as usize
    }
}
unsafe fn get_char_type(c: c_int) -> c_int {
    if ascii_isdigit(c) {
        return CHAR_DIGIT;
    }
    if unsafe { spell_isupper(c) } {
        return CHAR_UPPER;
    }
    CHAR_OTHER
}
unsafe fn advance_camelcase_word(
    str_: *mut c_char,
    wp: *mut win_T,
    is_camel_case: *mut bool,
) -> *mut c_char {
    unsafe {
        let mut end = str_;
        *is_camel_case = false;
        if *str_ == NUL {
            return str_;
        }
        let mut c = utf_ptr2char(end);
        mb_ptr_adv(&mut end);
        let mut last_last_type: c_int = -1;
        let mut last_type = get_char_type(c);
        while *end != NUL && spell_iswordp(end, wp) {
            c = utf_ptr2char(end);
            let this_type = get_char_type(c);
            if last_last_type == CHAR_UPPER && last_type == CHAR_UPPER && this_type == CHAR_OTHER {
                *is_camel_case = true;
                mb_ptr_back(str_, &mut end);
                break;
            } else if (this_type == CHAR_UPPER && last_type == CHAR_OTHER)
                || (this_type != last_type && (this_type == CHAR_DIGIT || last_type == CHAR_DIGIT))
            {
                *is_camel_case = true;
                break;
            }
            last_last_type = last_type;
            last_type = this_type;
            mb_ptr_adv(&mut end);
        }
        end
    }
}
unsafe fn find_word(mip: *mut matchinf_T, mode: c_int) {
    unsafe {
        let mut wlen: c_int = 0;
        let mut flen: c_int;
        let ptr: *mut c_char;
        let slang = (*(*mip).mi_lp).lp_slang;
        let byts: *mut u8;
        let idxs: *mut idx_T;
        if mode == FIND_KEEPWORD || mode == FIND_KEEPCOMPOUND {
            ptr = (*mip).mi_word;
            flen = 9999;
            byts = (*slang).sl_kbyts;
            idxs = (*slang).sl_kidxs;
            if mode == FIND_KEEPCOMPOUND {
                wlen += (*mip).mi_compoff;
            }
        } else {
            ptr = (&raw mut (*mip).mi_fword).cast();
            flen = (*mip).mi_fwordlen;
            byts = (*slang).sl_fbyts;
            idxs = (*slang).sl_fidxs;
            if mode == FIND_PREFIX {
                wlen = (*mip).mi_prefixlen;
                flen -= (*mip).mi_prefixlen;
            } else if mode == FIND_COMPOUND {
                wlen = (*mip).mi_compoff;
                flen -= (*mip).mi_compoff;
            }
        }
        if byts.is_null() {
            return;
        }
        let mut arridx: idx_T = 0;
        let mut endlen: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut endidx: [idx_T; MAXWLEN] = [0; MAXWLEN];
        let mut endidxcnt: usize = 0;
        loop {
            if flen <= 0 && *(*mip).mi_fend != NUL {
                flen = fold_more(mip);
            }
            let mut len = *byts.add(arridx as usize) as c_int;
            arridx += 1;
            if *byts.add(arridx as usize) == 0 {
                if endidxcnt == MAXWLEN {
                    emsg(gettext(e_format));
                    return;
                }
                endlen[endidxcnt] = wlen;
                endidx[endidxcnt] = arridx;
                endidxcnt += 1;
                arridx += 1;
                len -= 1;
                while len > 0 && *byts.add(arridx as usize) == 0 {
                    arridx += 1;
                    len -= 1;
                }
                if len == 0 {
                    break;
                }
            }
            if *ptr.add(wlen as usize) == NUL {
                break;
            }
            let mut c = *ptr.add(wlen as usize) as u8 as c_int;
            if c == TAB as u8 as c_int {
                c = b' ' as c_int;
            }
            let mut lo = arridx;
            let mut hi = arridx + len - 1;
            while lo < hi {
                let m = (lo + hi) / 2;
                if (*byts.add(m as usize) as c_int) > c {
                    hi = m - 1;
                } else if (*byts.add(m as usize) as c_int) < c {
                    lo = m + 1;
                } else {
                    lo = m;
                    hi = m;
                    break;
                }
            }
            if hi < lo || *byts.add(lo as usize) as c_int != c {
                break;
            }
            arridx = *idxs.add(lo as usize);
            wlen += 1;
            flen -= 1;
            if c == b' ' as c_int {
                loop {
                    if flen <= 0 && *(*mip).mi_fend != NUL {
                        flen = fold_more(mip);
                    }
                    let ch = *ptr.add(wlen as usize);
                    if ch != b' ' as c_char && ch != TAB {
                        break;
                    }
                    wlen += 1;
                    flen -= 1;
                }
            }
        }
        while endidxcnt > 0 {
            endidxcnt -= 1;
            arridx = endidx[endidxcnt];
            wlen = endlen[endidxcnt];
            if utf_head_off(ptr, ptr.add(wlen as usize)) > 0 {
                continue;
            }
            let word_ends = if spell_iswordp(ptr.add(wlen as usize), (*mip).mi_win) {
                if (*slang).sl_compprog.is_null() && !(*slang).sl_nobreak {
                    continue;
                }
                false
            } else {
                true
            };
            let mut prefix_found = false;
            if mode != FIND_KEEPWORD {
                let mut p = (*mip).mi_word;
                if strncmp(ptr, p, wlen as usize) != 0 {
                    let mut s = ptr;
                    let s_end = ptr.add(wlen as usize);
                    while s < s_end {
                        mb_ptr_adv(&mut p);
                        mb_ptr_adv(&mut s);
                    }
                    wlen = p.offset_from((*mip).mi_word) as c_int;
                }
            }
            let mut len = *byts.add((arridx - 1) as usize) as c_int;
            while len > 0 && *byts.add(arridx as usize) == 0 {
                let mut flags = *idxs.add(arridx as usize) as u32;
                len -= 1;
                arridx += 1;
                if mode == FIND_FOLDWORD {
                    if (*mip).mi_cend != (*mip).mi_word.add(wlen as usize) {
                        (*mip).mi_cend = (*mip).mi_word.add(wlen as usize);
                        (*mip).mi_capflags = captype((*mip).mi_word, (*mip).mi_cend);
                    }
                    if (*mip).mi_capflags == WF_KEEPCAP
                        || !spell_valid_case((*mip).mi_capflags, flags as c_int)
                    {
                        continue;
                    }
                } else if mode == FIND_PREFIX && !prefix_found {
                    let c = valid_word_prefix(
                        (*mip).mi_prefcnt,
                        (*mip).mi_prefarridx,
                        flags as c_int,
                        (*mip).mi_word.add((*mip).mi_cprefixlen as usize),
                        slang,
                        false,
                    );
                    if c == 0 {
                        continue;
                    }
                    if (c & WF_RAREPFX) != 0 {
                        flags |= WF_RARE as u32;
                    }
                    prefix_found = true;
                }
                if (*slang).sl_nobreak {
                    if (mode == FIND_COMPOUND || mode == FIND_KEEPCOMPOUND)
                        && (flags & WF_BANNED as u32) == 0
                    {
                        (*mip).mi_result = SP_OK;
                        break;
                    }
                } else if mode == FIND_COMPOUND || mode == FIND_KEEPCOMPOUND || !word_ends {
                    if (flags >> 24) == 0 || wlen - (*mip).mi_compoff < (*slang).sl_compminlen {
                        continue;
                    }
                    if (*slang).sl_compminlen > 0
                        && mb_charlen_len(
                            (*mip).mi_word.add((*mip).mi_compoff as usize),
                            wlen - (*mip).mi_compoff,
                        ) < (*slang).sl_compminlen
                    {
                        continue;
                    }
                    if !word_ends
                        && (*mip).mi_complen + (*mip).mi_compextra + 2 > (*slang).sl_compmax
                        && (*slang).sl_compsylmax == MAXWLEN as c_int
                    {
                        continue;
                    }
                    if (*mip).mi_complen > 0 && (flags & WF_NOCOMPBEF as u32) != 0 {
                        continue;
                    }
                    if !word_ends && (flags & WF_NOCOMPAFT as u32) != 0 {
                        continue;
                    }
                    if !byte_in_str(
                        if (*mip).mi_complen == 0 {
                            (*slang).sl_compstartflags
                        } else {
                            (*slang).sl_compallflags
                        },
                        (flags >> 24) as c_int,
                    ) {
                        continue;
                    }
                    if match_checkcompoundpattern(ptr, wlen, &raw mut (*slang).sl_comppat) {
                        continue;
                    }
                    if mode == FIND_COMPOUND {
                        let mut p: *mut c_char;
                        if strncmp(ptr, (*mip).mi_word, (*mip).mi_compoff as usize) != 0 {
                            p = (*mip).mi_word;
                            let mut s = ptr;
                            let s_end = ptr.add((*mip).mi_compoff as usize);
                            while s < s_end {
                                mb_ptr_adv(&mut p);
                                mb_ptr_adv(&mut s);
                            }
                        } else {
                            p = (*mip).mi_word.add((*mip).mi_compoff as usize);
                        }
                        let capflags = captype(p, (*mip).mi_word.add(wlen as usize));
                        if capflags == WF_KEEPCAP
                            || (capflags == WF_ALLCAP && (flags & WF_FIXCAP as u32) != 0)
                        {
                            continue;
                        }
                        if capflags != WF_ALLCAP {
                            mb_ptr_back((*mip).mi_word, &mut p);
                            if if spell_iswordp_nmw(p, (*mip).mi_win) {
                                capflags == WF_ONECAP
                            } else {
                                (flags & WF_ONECAP as u32) != 0 && capflags != WF_ONECAP
                            } {
                                continue;
                            }
                        }
                    }
                    (*mip).mi_compflags[(*mip).mi_complen as usize] = (flags >> 24) as u8;
                    (*mip).mi_compflags[(*mip).mi_complen as usize + 1] = 0;
                    if word_ends {
                        let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
                        if (*slang).sl_compsylmax < MAXWLEN as c_int {
                            if ptr == (*mip).mi_word {
                                spell_casefold(
                                    (*mip).mi_win,
                                    ptr,
                                    wlen,
                                    (&raw mut fword).cast(),
                                    MAXWLEN as c_int,
                                );
                            } else {
                                xmemcpyz(
                                    (&raw mut fword).cast(),
                                    ptr.cast(),
                                    endlen[endidxcnt] as usize,
                                );
                            }
                        }
                        if !can_compound(
                            slang,
                            (&raw const fword).cast(),
                            (&raw const (*mip).mi_compflags).cast(),
                        ) {
                            continue;
                        }
                    } else if !(*slang).sl_comprules.is_null()
                        && !match_compoundrule(slang, (&raw const (*mip).mi_compflags).cast())
                    {
                        continue;
                    }
                } else if (flags & WF_NEEDCOMP as u32) != 0 {
                    continue;
                }
                let mut nobreak_result = SP_OK;
                if !word_ends {
                    let save_result = (*mip).mi_result;
                    let save_end = (*mip).mi_end;
                    let save_lp = (*mip).mi_lp;
                    if (*slang).sl_nobreak {
                        (*mip).mi_result = SP_BAD;
                    }
                    (*mip).mi_compoff = endlen[endidxcnt];
                    if mode == FIND_KEEPWORD {
                        let mut p = (&raw mut (*mip).mi_fword).cast::<c_char>();
                        if strncmp(ptr, p, wlen as usize) != 0 {
                            let mut s = ptr;
                            let s_end = ptr.add(wlen as usize);
                            while s < s_end {
                                mb_ptr_adv(&mut p);
                                mb_ptr_adv(&mut s);
                            }
                            (*mip).mi_compoff = p
                                .offset_from((&raw const (*mip).mi_fword).cast::<c_char>())
                                as c_int;
                        }
                    }
                    (*mip).mi_complen += 1;
                    if (flags & WF_COMPROOT as u32) != 0 {
                        (*mip).mi_compextra += 1;
                    }
                    let w_s = (*(*mip).mi_win).w_s.cast::<synblock_T>();
                    for lpi in 0..((*w_s).b_langp.ga_len as usize) {
                        if (*slang).sl_nobreak {
                            (*mip).mi_lp = langp_entry(&raw const (*w_s).b_langp, lpi);
                            if (*(*(*mip).mi_lp).lp_slang).sl_fidxs.is_null()
                                || !(*(*(*mip).mi_lp).lp_slang).sl_nobreak
                            {
                                continue;
                            }
                        }
                        find_word(mip, FIND_COMPOUND);
                        if !(*slang).sl_nobreak || (*mip).mi_result == SP_BAD {
                            (*mip).mi_compoff = wlen;
                            find_word(mip, FIND_KEEPCOMPOUND);
                        }
                        if !(*slang).sl_nobreak {
                            break;
                        }
                    }
                    (*mip).mi_complen -= 1;
                    if (flags & WF_COMPROOT as u32) != 0 {
                        (*mip).mi_compextra -= 1;
                    }
                    (*mip).mi_lp = save_lp;
                    if (*slang).sl_nobreak {
                        nobreak_result = (*mip).mi_result;
                        (*mip).mi_result = save_result;
                        (*mip).mi_end = save_end;
                    } else {
                        if (*mip).mi_result == SP_OK {
                            break;
                        }
                        continue;
                    }
                }
                let res = if (flags & WF_BANNED as u32) != 0 {
                    SP_BANNED
                } else if (flags & WF_REGION as u32) != 0 {
                    if (((*(*mip).mi_lp).lp_region as u32) & (flags >> 16)) != 0 {
                        SP_OK
                    } else {
                        SP_LOCAL
                    }
                } else if (flags & WF_RARE as u32) != 0 {
                    SP_RARE
                } else {
                    SP_OK
                };
                let new_end = (*mip).mi_word.add(wlen as usize);
                if nobreak_result == SP_BAD {
                    if (*mip).mi_result2 > res {
                        (*mip).mi_result2 = res;
                        (*mip).mi_end2 = new_end;
                    } else if (*mip).mi_result2 == res && (*mip).mi_end2 < new_end {
                        (*mip).mi_end2 = new_end;
                    }
                } else if (*mip).mi_result > res {
                    (*mip).mi_result = res;
                    (*mip).mi_end = new_end;
                } else if (*mip).mi_result == res && (*mip).mi_end < new_end {
                    (*mip).mi_end = new_end;
                }
                if (*mip).mi_result == SP_OK {
                    break;
                }
            }
            if (*mip).mi_result == SP_OK {
                break;
            }
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn match_checkcompoundpattern(
    ptr: *mut c_char,
    wlen: c_int,
    gap: *mut garray_T,
) -> bool {
    unsafe {
        let mut i: c_int = 0;
        while i + 1 < (*gap).ga_len {
            let p2 = *((*gap).ga_data.cast::<*mut c_char>()).add((i + 1) as usize);
            if strncmp(ptr.add(wlen as usize), p2, strlen(p2)) == 0 {
                let p1 = *((*gap).ga_data.cast::<*mut c_char>()).add(i as usize);
                let len = strlen(p1) as c_int;
                if len <= wlen && strncmp(ptr.add((wlen - len) as usize), p1, len as usize) == 0 {
                    return true;
                }
            }
            i += 2;
        }
        false
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn can_compound(
    slang: *mut slang_T,
    word: *const c_char,
    flags: *const u8,
) -> bool {
    unsafe {
        let mut uflags: [c_char; MAXWLEN * 2] = [0; MAXWLEN * 2];
        if (*slang).sl_compprog.is_null() {
            return false;
        }
        let mut p = (&raw mut uflags).cast::<c_char>();
        let mut i = 0usize;
        while *flags.add(i) != 0 {
            p = p.add(utf_char2bytes(*flags.add(i) as c_int, p) as usize);
            i += 1;
        }
        *p = NUL;
        p = (&raw mut uflags).cast();
        if !vim_regexec_prog((&raw mut (*slang).sl_compprog).cast(), false, p, 0) {
            return false;
        }
        if (*slang).sl_compsylmax < MAXWLEN as c_int
            && count_syllables(slang, word) > (*slang).sl_compsylmax
        {
            return (strlen(flags.cast()) as c_int) < (*slang).sl_compmax;
        }
        true
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn match_compoundrule(slang: *mut slang_T, compflags: *const u8) -> bool {
    unsafe {
        let mut p = (*slang).sl_comprules.cast::<c_char>();
        while *p != NUL {
            let mut i = 0usize;
            loop {
                let c = *compflags.add(i);
                if c == 0 {
                    return true;
                }
                if *p == b'/' as c_char || *p == NUL {
                    break;
                }
                if *p == b'[' as c_char {
                    let mut matched = false;
                    p = p.add(1);
                    while *p != b']' as c_char && *p != NUL {
                        if *p as u8 == c {
                            matched = true;
                        }
                        p = p.add(1);
                    }
                    if !matched {
                        break;
                    }
                } else if *p as u8 != c {
                    break;
                }
                if *p != NUL {
                    p = p.add(1);
                }
                i += 1;
            }
            p = vim_strchr(p, b'/' as c_int);
            if p.is_null() {
                break;
            }
            p = p.add(1);
        }
        false
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_word_prefix(
    totprefcnt: c_int,
    arridx: c_int,
    flags: c_int,
    word: *mut c_char,
    slang: *mut slang_T,
    cond_req: bool,
) -> c_int {
    unsafe {
        let prefid = ((flags as u32) >> 24) as c_int;
        let mut prefcnt = totprefcnt - 1;
        while prefcnt >= 0 {
            let pidx = *(*slang).sl_pidxs.add((arridx + prefcnt) as usize);
            prefcnt -= 1;
            if prefid != (pidx & 0xff) {
                continue;
            }
            if (flags & WF_HAS_AFF) != 0 && (pidx & WF_PFX_NC) != 0 {
                continue;
            }
            let rp = (*slang)
                .sl_prefprog
                .add((((pidx as u32) >> 8) & 0xffff) as usize)
                .cast::<*mut regprog_T>();
            if !(*rp).is_null() {
                if !vim_regexec_prog(rp, false, word, 0) {
                    continue;
                }
            } else if cond_req {
                continue;
            }
            return pidx;
        }
        0
    }
}
unsafe fn find_prefix(mip: *mut matchinf_T, mode: c_int) {
    unsafe {
        let mut arridx: idx_T = 0;
        let mut wlen: c_int = 0;
        let slang = (*(*mip).mi_lp).lp_slang;
        let byts = (*slang).sl_pbyts;
        if byts.is_null() {
            return;
        }
        let mut ptr = (&raw mut (*mip).mi_fword).cast::<c_char>();
        let mut flen = (*mip).mi_fwordlen;
        if mode == FIND_COMPOUND {
            ptr = ptr.add((*mip).mi_compoff as usize);
            flen -= (*mip).mi_compoff;
        }
        let idxs = (*slang).sl_pidxs;
        loop {
            if flen == 0 && *(*mip).mi_fend != NUL {
                flen = fold_more(mip);
            }
            let mut len = *byts.add(arridx as usize) as c_int;
            arridx += 1;
            if *byts.add(arridx as usize) == 0 {
                (*mip).mi_prefarridx = arridx;
                (*mip).mi_prefcnt = len;
                while len > 0 && *byts.add(arridx as usize) == 0 {
                    arridx += 1;
                    len -= 1;
                }
                (*mip).mi_prefcnt -= len;
                (*mip).mi_prefixlen = wlen;
                if mode == FIND_COMPOUND {
                    (*mip).mi_prefixlen += (*mip).mi_compoff;
                }
                (*mip).mi_cprefixlen = nofold_len(
                    (&raw mut (*mip).mi_fword).cast(),
                    (*mip).mi_prefixlen,
                    (*mip).mi_word,
                );
                find_word(mip, FIND_PREFIX);
                if len == 0 {
                    break;
                }
            }
            if *ptr.add(wlen as usize) == NUL {
                break;
            }
            let c = *ptr.add(wlen as usize) as u8 as c_int;
            let mut lo = arridx;
            let mut hi = arridx + len - 1;
            while lo < hi {
                let m = (lo + hi) / 2;
                if (*byts.add(m as usize) as c_int) > c {
                    hi = m - 1;
                } else if (*byts.add(m as usize) as c_int) < c {
                    lo = m + 1;
                } else {
                    lo = m;
                    hi = m;
                    break;
                }
            }
            if hi < lo || *byts.add(lo as usize) as c_int != c {
                break;
            }
            arridx = *idxs.add(lo as usize);
            wlen += 1;
            flen -= 1;
        }
    }
}
unsafe fn fold_more(mip: *mut matchinf_T) -> c_int {
    unsafe {
        let p = (*mip).mi_fend;
        loop {
            mb_ptr_adv(&mut (*mip).mi_fend);
            if *(*mip).mi_fend == NUL || !spell_iswordp((*mip).mi_fend, (*mip).mi_win) {
                break;
            }
        }
        if *(*mip).mi_fend != NUL {
            mb_ptr_adv(&mut (*mip).mi_fend);
        }
        let dst = (&raw mut (*mip).mi_fword)
            .cast::<c_char>()
            .add((*mip).mi_fwordlen as usize);
        spell_casefold(
            (*mip).mi_win,
            p,
            (*mip).mi_fend.offset_from(p) as c_int,
            dst,
            MAXWLEN as c_int - (*mip).mi_fwordlen,
        );
        let flen = strlen(dst) as c_int;
        (*mip).mi_fwordlen += flen;
        flen
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_valid_case(wordflags: c_int, treeflags: c_int) -> bool {
    (wordflags == WF_ALLCAP && (treeflags & WF_FIXCAP) == 0)
        || ((treeflags & (WF_ALLCAP | WF_KEEPCAP)) == 0
            && ((treeflags & WF_ONECAP) == 0 || (wordflags & WF_ONECAP) != 0))
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_check_window(wp: *mut win_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        (*wp).w_onebuf_opt.wo_spell != 0
            && *(*w_s).b_p_spl != NUL
            && (*w_s).b_langp.ga_len > 0
            && !(*((*w_s).b_langp.ga_data.cast::<*mut c_char>())).is_null()
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn no_spell_checking(wp: *mut win_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        if (*wp).w_onebuf_opt.wo_spell == 0 || *(*w_s).b_p_spl == NUL || (*w_s).b_langp.ga_len <= 0
        {
            emsg(gettext((&raw const e_no_spell).cast()));
            return true;
        }
        false
    }
}
unsafe fn decor_spell_nav_col(
    wp: *mut win_T,
    lnum: linenr_T,
    decor_lnum: *mut linenr_T,
    col: c_int,
) -> TriState {
    unsafe {
        if *decor_lnum != lnum {
            decor_redraw_reset(wp, &raw mut decor_state);
            decor_providers_invoke_spell(wp, lnum - 1, col, lnum - 1, -1);
            decor_redraw_line(wp, lnum - 1, &raw mut decor_state);
            *decor_lnum = lnum;
        }
        decor_redraw_col(wp, col, 0, false, &raw mut decor_state, MAXCOL);
        decor_state.spell
    }
}
#[inline]
unsafe fn can_syn_spell(wp: *mut win_T, lnum: linenr_T, col: c_int) -> bool {
    unsafe {
        let mut can_spell = false;
        let _ = syn_get_id(wp, lnum, col, 0, &raw mut can_spell, 0);
        can_spell
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_move_to(
    wp: *mut win_T,
    dir: c_int,
    behaviour: smt_T,
    curline: bool,
    attrp: *mut hlf_T,
) -> usize {
    unsafe {
        if no_spell_checking(wp) {
            return 0;
        }
        let mut found_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut found_len: usize = 0;
        let mut attr: hlf_T = HLF_COUNT;
        let has_syntax = syntax_present(wp);
        let mut buf: *mut c_char = ptr::null_mut();
        let mut buflen: usize = 0;
        let mut skip: c_int = 0;
        let mut capcol: colnr_T = -1;
        let mut found_one = false;
        let mut wrapped = false;
        let mut ret: usize = 0;
        let mut lnum = (*wp).w_cursor.lnum;
        let saved_decor_start = ptr::read(&raw const decor_state);
        let mut decor_lnum: linenr_T = -1;
        decor_state = std::mem::MaybeUninit::zeroed().assume_init();
        'theend: {
            while !got_int {
                let mut line = ml_get_buf((*wp).w_buffer, lnum);
                let mut len = ml_get_buf_len((*wp).w_buffer, lnum) as usize;
                if buflen < len + MAXWLEN + 2 {
                    xfree(buf.cast());
                    buflen = len + MAXWLEN + 2;
                    buf = xmalloc(buflen).cast();
                }
                if lnum == 1 {
                    capcol = 0;
                }
                if capcol == 0 {
                    capcol = getwhitecols(line) as colnr_T;
                } else if curline && wp == curwin {
                    let col = getwhitecols(line) as colnr_T;
                    if check_need_cap(curwin, lnum, col) {
                        capcol = col;
                    }
                    line = ml_get_buf((*wp).w_buffer, lnum);
                }
                let empty_line = *skipwhite(line) == NUL;
                strcpy(buf, line);
                if lnum < (*(*wp).w_buffer).b_ml.ml_line_count {
                    spell_cat_line(
                        buf.add(strlen(buf)),
                        ml_get_buf((*wp).w_buffer, lnum + 1),
                        MAXWLEN as c_int,
                    );
                }
                let mut p = buf.add(skip as usize);
                let endp = buf.add(len);
                while p < endp {
                    if dir == BACKWARD
                        && lnum == (*wp).w_cursor.lnum
                        && !wrapped
                        && (p.offset_from(buf) as colnr_T) >= (*wp).w_cursor.col
                    {
                        break;
                    }
                    attr = HLF_COUNT;
                    len = spell_check(wp, p, &raw mut attr, &raw mut capcol, false);
                    if attr != HLF_COUNT
                        && (behaviour == SMT_ALL
                            || (behaviour == SMT_BAD && attr == HLF_SPB)
                            || (behaviour == SMT_RARE && attr == HLF_SPR))
                    {
                        let check_col = if curline {
                            (p.offset_from(buf) + len as isize) as colnr_T
                        } else {
                            p.offset_from(buf) as colnr_T
                        };
                        if dir == BACKWARD
                            || lnum != (*wp).w_cursor.lnum
                            || wrapped
                            || check_col > (*wp).w_cursor.col
                        {
                            let col = p.offset_from(buf) as colnr_T;
                            let w_s = (*wp).w_s.cast::<synblock_T>();
                            let no_plain_buffer =
                                ((*w_s).b_p_spo_flags & kOptSpoFlagNoplainbuffer) != 0;
                            let mut can_spell = !no_plain_buffer;
                            match decor_spell_nav_col(wp, lnum, &raw mut decor_lnum, col) {
                                kTrue => can_spell = true,
                                kFalse => can_spell = false,
                                kNone if has_syntax => {
                                    can_spell = can_syn_spell(wp, lnum, col);
                                }
                                _ => {}
                            }
                            if !can_spell {
                                attr = HLF_COUNT;
                            } else {
                                found_one = true;
                                found_pos = pos_T {
                                    lnum,
                                    col,
                                    coladd: 0,
                                };
                                if dir == FORWARD {
                                    (*wp).w_cursor = found_pos;
                                    if !attrp.is_null() {
                                        *attrp = attr;
                                    }
                                    ret = len;
                                    break 'theend;
                                } else if curline {
                                    found_pos.col += len as c_int;
                                }
                                found_len = len;
                            }
                        } else {
                            found_one = true;
                        }
                    }
                    p = p.add(len);
                    capcol -= len as c_int;
                }
                if dir == BACKWARD && found_pos.lnum != 0 {
                    (*wp).w_cursor = found_pos;
                    ret = found_len;
                    break 'theend;
                }
                if curline {
                    break;
                }
                if lnum == (*wp).w_cursor.lnum && wrapped {
                    break;
                }
                if dir == BACKWARD {
                    if lnum > 1 {
                        lnum -= 1;
                    } else if p_ws == 0 {
                        break;
                    } else {
                        lnum = (*(*wp).w_buffer).b_ml.ml_line_count;
                        wrapped = true;
                        if !shortmess(kShmSearch) {
                            give_warning(gettext((&raw const top_bot_msg).cast()), true, false);
                        }
                    }
                    capcol = -1;
                } else {
                    if lnum < (*(*wp).w_buffer).b_ml.ml_line_count {
                        lnum += 1;
                    } else if p_ws == 0 {
                        break;
                    } else {
                        lnum = 1;
                        wrapped = true;
                        if !shortmess(kShmSearch) {
                            give_warning(gettext((&raw const bot_top_msg).cast()), true, false);
                        }
                    }
                    if lnum == (*wp).w_cursor.lnum && !found_one {
                        break;
                    }
                    if attr == HLF_COUNT {
                        skip = p.offset_from(endp) as c_int;
                    } else {
                        skip = 0;
                    }
                    capcol -= 1;
                    if empty_line {
                        capcol = 0;
                    }
                }
                line_breakcheck();
            }
        }
        decor_state_free(&raw mut decor_state);
        decor_state = saved_decor_start;
        xfree(buf.cast());
        ret
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_cat_line(buf: *mut c_char, line: *mut c_char, maxlen: c_int) {
    unsafe {
        let mut p = skipwhite(line);
        while !vim_strchr(c"*#/\"\t".as_ptr(), *p as u8 as c_int).is_null() {
            p = skipwhite(p.add(1));
        }
        if *p == NUL {
            return;
        }
        let n = p.offset_from(line) as c_int + 1;
        if n < maxlen - 1 {
            ptr::write_bytes(buf.cast::<u8>(), b' ', n as usize);
            xstrlcpy(buf.add(n as usize), p, (maxlen - n) as usize);
        }
    }
}
unsafe fn spell_load_lang(lang: *mut c_char) {
    unsafe {
        let mut fname_enc: [c_char; 85] = [0; 85];
        let mut r: c_int = FAIL;
        let mut sl = spelload_T {
            sl_lang: [0; MAXWLEN + 1],
            sl_slang: ptr::null_mut(),
            sl_nobreak: 0,
        };
        strcpy((&raw mut sl.sl_lang).cast(), lang);
        (*curbuf).b_locked += 1;
        for round in 1..=2 {
            vim_snprintf!(
                (&raw mut fname_enc).cast(),
                fname_enc.len() - 5,
                c"spell/%s.%s.spl".as_ptr(),
                lang,
                spell_enc(),
            );
            r = do_in_runtimepath(
                (&raw mut fname_enc).cast(),
                0,
                Some(spell_load_cb),
                (&raw mut sl).cast(),
            );
            if r == FAIL && sl.sl_lang[0] != NUL {
                vim_snprintf!(
                    (&raw mut fname_enc).cast(),
                    fname_enc.len() - 5,
                    c"spell/%s.ascii.spl".as_ptr(),
                    lang,
                );
                r = do_in_runtimepath(
                    (&raw mut fname_enc).cast(),
                    0,
                    Some(spell_load_cb),
                    (&raw mut sl).cast(),
                );
                if r == FAIL
                    && sl.sl_lang[0] != NUL
                    && round == 1
                    && apply_autocmds(
                        EVENT_SPELLFILEMISSING,
                        lang,
                        (*curbuf).b_fname,
                        false,
                        curbuf,
                    )
                {
                    continue;
                }
                break;
            }
            break;
        }
        if r == FAIL {
            if starting != 0 {
                let mut autocmd_buf: [c_char; 512] = [0; 512];
                vim_snprintf!(
                    (&raw mut autocmd_buf).cast(),
                    autocmd_buf.len(),
                    c"autocmd VimEnter * call v:lua.require'nvim.spellfile'.get('%s')|set spell"
                        .as_ptr(),
                    lang,
                );
                do_cmdline_cmd((&raw const autocmd_buf).cast());
            } else {
                smsg!(
                    0,
                    gettext(
                        c"Warning: Cannot find word list \"%s.%s.spl\" or \"%s.ascii.spl\""
                            .as_ptr(),
                    ),
                    lang,
                    spell_enc(),
                    lang,
                );
            }
        } else if !sl.sl_slang.is_null() {
            let fn_ptr = (&raw mut fname_enc).cast::<c_char>();
            strcpy(fn_ptr.add(strlen(fn_ptr) - 3), c"add.spl".as_ptr());
            do_in_runtimepath(fn_ptr, DIP_ALL, Some(spell_load_cb), (&raw mut sl).cast());
        }
        (*curbuf).b_locked -= 1;
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_enc() -> *mut c_char {
    unsafe {
        if strlen(p_enc) < 60 && strcmp(p_enc, c"iso-8859-15".as_ptr()) != 0 {
            return p_enc;
        }
        c"latin1".as_ptr().cast_mut()
    }
}
unsafe fn int_wordlist_spl(fname: *mut c_char) {
    unsafe {
        vim_snprintf!(
            fname,
            MAXPATHL,
            c"%s.%s.spl".as_ptr(),
            int_wordlist,
            spell_enc(),
        );
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn slang_alloc(lang: *mut c_char) -> *mut slang_T {
    unsafe {
        let lp = xcalloc(1, size_of::<slang_T>()).cast::<slang_T>();
        if !lang.is_null() {
            (*lp).sl_name = xstrdup(lang);
        }
        ga_init(&raw mut (*lp).sl_rep, size_of::<fromto_T>() as c_int, 10);
        ga_init(&raw mut (*lp).sl_repsal, size_of::<fromto_T>() as c_int, 10);
        (*lp).sl_compmax = MAXWLEN as c_int;
        (*lp).sl_compsylmax = MAXWLEN as c_int;
        hash_init(&raw mut (*lp).sl_wordcount);
        lp
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn slang_free(lp: *mut slang_T) {
    unsafe {
        xfree((*lp).sl_name.cast());
        xfree((*lp).sl_fname.cast());
        slang_clear(lp);
        xfree(lp.cast());
    }
}
unsafe fn free_salitem(smp: *mut salitem_T) {
    unsafe {
        xfree((*smp).sm_lead.cast());
        xfree((*smp).sm_to.cast());
        xfree((*smp).sm_lead_w.cast());
        xfree((*smp).sm_oneof_w.cast());
        xfree((*smp).sm_to_w.cast());
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn free_sal_items(gap: *mut garray_T) {
    unsafe {
        for i in 0..((*gap).ga_len as usize) {
            free_salitem(((*gap).ga_data.cast::<salitem_T>()).add(i));
        }
        ga_clear(gap);
    }
}
unsafe fn free_fromto(ftp: *mut fromto_T) {
    unsafe {
        xfree((*ftp).ft_from.cast());
        xfree((*ftp).ft_to.cast());
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn slang_clear(lp: *mut slang_T) {
    unsafe {
        xfree((*lp).sl_fbyts.cast());
        (*lp).sl_fbyts = ptr::null_mut();
        xfree((*lp).sl_kbyts.cast());
        (*lp).sl_kbyts = ptr::null_mut();
        xfree((*lp).sl_pbyts.cast());
        (*lp).sl_pbyts = ptr::null_mut();
        xfree((*lp).sl_fidxs.cast());
        (*lp).sl_fidxs = ptr::null_mut();
        xfree((*lp).sl_kidxs.cast());
        (*lp).sl_kidxs = ptr::null_mut();
        xfree((*lp).sl_pidxs.cast());
        (*lp).sl_pidxs = ptr::null_mut();
        for i in 0..((*lp).sl_rep.ga_len as usize) {
            free_fromto(((*lp).sl_rep.ga_data.cast::<fromto_T>()).add(i));
        }
        ga_clear(&raw mut (*lp).sl_rep);
        for i in 0..((*lp).sl_repsal.ga_len as usize) {
            free_fromto(((*lp).sl_repsal.ga_data.cast::<fromto_T>()).add(i));
        }
        ga_clear(&raw mut (*lp).sl_repsal);
        let gap = &raw mut (*lp).sl_sal;
        if (*lp).sl_sofo {
            for i in 0..((*gap).ga_len as usize) {
                xfree(*((*gap).ga_data.cast::<*mut c_void>()).add(i));
            }
            ga_clear(gap);
        } else {
            free_sal_items(gap);
        }
        for i in 0..((*lp).sl_prefixcnt as usize) {
            vim_regfree((*(*lp).sl_prefprog.add(i)).cast());
        }
        (*lp).sl_prefixcnt = 0;
        xfree((*lp).sl_prefprog.cast());
        (*lp).sl_prefprog = ptr::null_mut();
        xfree((*lp).sl_info.cast());
        (*lp).sl_info = ptr::null_mut();
        xfree((*lp).sl_midword.cast());
        (*lp).sl_midword = ptr::null_mut();
        vim_regfree((*lp).sl_compprog.cast());
        (*lp).sl_compprog = ptr::null_mut();
        xfree((*lp).sl_comprules.cast());
        (*lp).sl_comprules = ptr::null_mut();
        xfree((*lp).sl_compstartflags.cast());
        (*lp).sl_compstartflags = ptr::null_mut();
        xfree((*lp).sl_compallflags.cast());
        (*lp).sl_compallflags = ptr::null_mut();
        xfree((*lp).sl_syllable.cast());
        (*lp).sl_syllable = ptr::null_mut();
        ga_clear(&raw mut (*lp).sl_syl_items);
        ga_clear_strings(&raw mut (*lp).sl_comppat);
        hash_clear_all(
            &raw mut (*lp).sl_wordcount,
            std::mem::offset_of!(wordcount_T, wc_word) as u32,
        );
        hash_init(&raw mut (*lp).sl_wordcount);
        hash_clear_all(&raw mut (*lp).sl_map_hash, 0);
        slang_clear_sug(lp);
        (*lp).sl_compmax = MAXWLEN as c_int;
        (*lp).sl_compminlen = 0;
        (*lp).sl_compsylmax = MAXWLEN as c_int;
        (*lp).sl_regions[0] = NUL;
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn slang_clear_sug(lp: *mut slang_T) {
    unsafe {
        xfree((*lp).sl_sbyts.cast());
        (*lp).sl_sbyts = ptr::null_mut();
        xfree((*lp).sl_sidxs.cast());
        (*lp).sl_sidxs = ptr::null_mut();
        close_spellbuf((*lp).sl_sugbuf);
        (*lp).sl_sugbuf = ptr::null_mut();
        (*lp).sl_sugloaded = false;
        (*lp).sl_sugtime = 0;
    }
}
unsafe extern "C" fn spell_load_cb(
    num_fnames: c_int,
    fnames: *mut *mut c_char,
    all: bool,
    cookie: *mut c_void,
) -> bool {
    unsafe {
        let slp = cookie.cast::<spelload_T>();
        for i in 0..(num_fnames as usize) {
            let slang = spell_load_file(
                *fnames.add(i),
                (&raw mut (*slp).sl_lang).cast(),
                ptr::null_mut(),
                false,
            );
            if slang.is_null() {
                continue;
            }
            if (*slp).sl_nobreak != 0 && (*slang).sl_add {
                (*slang).sl_nobreak = true;
            } else if (*slang).sl_nobreak {
                (*slp).sl_nobreak = 1;
            }
            (*slp).sl_slang = slang;
            if !all {
                break;
            }
        }
        num_fnames > 0
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn count_common_word(
    lp: *mut slang_T,
    word: *mut c_char,
    len: c_int,
    count: u8,
) {
    unsafe {
        let mut buf: [c_char; MAXWLEN] = [0; MAXWLEN];
        let p: *mut c_char = if len == -1 {
            word
        } else if len >= MAXWLEN as c_int {
            return;
        } else {
            xmemcpyz((&raw mut buf).cast(), word.cast(), len as usize);
            (&raw mut buf).cast()
        };
        let hash: hash_T = hash_hash(p);
        let p_len = strlen(p);
        let hi = hash_lookup(&raw mut (*lp).sl_wordcount, p, p_len, hash);
        if hashitem_empty(&*hi) {
            let wc_off = std::mem::offset_of!(wordcount_T, wc_word);
            let wc = xmalloc(wc_off + p_len + 1).cast::<wordcount_T>();
            let wc_word_ptr = wc.cast::<u8>().add(wc_off).cast::<c_char>();
            ptr::copy_nonoverlapping(p, wc_word_ptr, p_len + 1);
            (*wc).wc_count = count as u16;
            hash_add_item(&raw mut (*lp).sl_wordcount, hi, wc_word_ptr, hash);
        } else {
            let wc = (*hi)
                .hi_key
                .sub(std::mem::offset_of!(wordcount_T, wc_word))
                .cast::<wordcount_T>();
            let (new_count, overflow) = (*wc).wc_count.overflowing_add(count as u16);
            (*wc).wc_count = if overflow { MAXWORDCOUNT } else { new_count };
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn byte_in_str(str_: *mut u8, n: c_int) -> bool {
    unsafe {
        let mut p = str_;
        while *p != 0 {
            if *p as c_int == n {
                return true;
            }
            p = p.add(1);
        }
        false
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn init_syl_tab(slang: *mut slang_T) -> c_int {
    unsafe {
        ga_init(
            &raw mut (*slang).sl_syl_items,
            size_of::<syl_item_T>() as c_int,
            4,
        );
        let mut p = vim_strchr((*slang).sl_syllable, b'/' as c_int);
        while !p.is_null() {
            *p = NUL;
            p = p.add(1);
            if *p == NUL {
                break;
            }
            let s = p;
            p = vim_strchr(p, b'/' as c_int);
            let l = if p.is_null() {
                strlen(s)
            } else {
                p.offset_from(s) as usize
            };
            if l >= SY_MAXLEN {
                return SP_FORMERROR;
            }
            ga_grow(&raw mut (*slang).sl_syl_items, 1);
            let syl = ((*slang).sl_syl_items.ga_data.cast::<syl_item_T>())
                .add((*slang).sl_syl_items.ga_len as usize);
            (*slang).sl_syl_items.ga_len += 1;
            xmemcpyz((&raw mut (*syl).sy_chars).cast(), s.cast(), l);
            (*syl).sy_len = l as c_int;
        }
        OK
    }
}
unsafe fn count_syllables(slang: *mut slang_T, word: *const c_char) -> c_int {
    unsafe {
        if (*slang).sl_syllable.is_null() {
            return 0;
        }
        let mut cnt: c_int = 0;
        let mut skip = false;
        let mut p = word;
        while *p != NUL {
            if *p == b' ' as c_char {
                p = p.add(1);
                cnt = 0;
                continue;
            }
            let mut len: c_int = 0;
            for i in 0..((*slang).sl_syl_items.ga_len as usize) {
                let syl = ((*slang).sl_syl_items.ga_data.cast::<syl_item_T>()).add(i);
                if (*syl).sy_len > len
                    && strncmp(
                        p,
                        (&raw const (*syl).sy_chars).cast(),
                        (*syl).sy_len as usize,
                    ) == 0
                {
                    len = (*syl).sy_len;
                }
            }
            if len != 0 {
                cnt += 1;
                skip = false;
            } else {
                let c = utf_ptr2char(p);
                len = utfc_ptr2len(p);
                if vim_strchr((*slang).sl_syllable, c).is_null() {
                    skip = false;
                } else if !skip {
                    cnt += 1;
                    skip = true;
                }
            }
            p = p.add(len as usize);
        }
        cnt
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn parse_spelllang(wp: *mut win_T) -> *mut c_char {
    unsafe {
        let mut region_cp: [c_char; 3] = [0; 3];
        let mut lang: [c_char; MAXWLEN + 1] = [0; MAXWLEN + 1];
        let mut spf_name: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut use_region: *mut c_char = ptr::null_mut();
        let mut dont_use_region = false;
        let mut nobreak = false;
        let mut ret_msg: *mut c_char = ptr::null_mut();
        let mut bufref = bufref_T {
            br_buf: ptr::null_mut(),
            br_fnum: 0,
            br_buf_free_count: 0,
        };
        set_bufref(&raw mut bufref, (*wp).w_buffer);
        if parse_spelllang_recursive {
            return ptr::null_mut();
        }
        parse_spelllang_recursive = true;
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        ga_init(&raw mut ga, size_of::<langp_T>() as c_int, 2);
        clear_midword(wp);
        let w_s = (*wp).w_s.cast::<synblock_T>();
        let spl_copy = xstrdup((*w_s).b_p_spl);
        (*w_s).b_cjk = 0;
        'theend: {
            let mut splp = spl_copy;
            while *splp != NUL {
                let len = copy_option_part(
                    &raw mut splp,
                    (&raw mut lang).cast(),
                    MAXWLEN,
                    c",".as_ptr(),
                ) as c_int;
                let mut region: *mut c_char = ptr::null_mut();
                let lang_ptr = (&raw mut lang).cast::<c_char>();
                if !valid_spelllang(lang_ptr) {
                    continue;
                }
                if strcmp(lang_ptr, c"cjk".as_ptr()) == 0 {
                    (*w_s).b_cjk = 1;
                    continue;
                }
                let mut slang: *mut slang_T;
                let filename: bool;
                if len > 4
                    && path_equal(
                        lang_ptr.add((len - 4) as usize),
                        c".spl".as_ptr(),
                        kPathCmpLiteral as PathCmpFlags,
                    )
                {
                    filename = true;
                    let p = vim_strchr(path_tail(lang_ptr), b'_' as c_int);
                    if !p.is_null()
                        && ascii_isalpha(*p.add(1))
                        && ascii_isalpha(*p.add(2))
                        && !ascii_isalpha(*p.add(3))
                    {
                        xstrlcpy((&raw mut region_cp).cast(), p.add(1), 3);
                        ptr::copy(
                            p.add(3),
                            p,
                            (len as usize) - (p.offset_from(lang_ptr) as usize) - 2,
                        );
                        region = (&raw mut region_cp).cast();
                    } else {
                        dont_use_region = true;
                    }
                    slang = first_lang;
                    while !slang.is_null() {
                        if path_equal(lang_ptr, (*slang).sl_fname, kPathCmpExpand as PathCmpFlags) {
                            break;
                        }
                        slang = (*slang).sl_next;
                    }
                } else {
                    filename = false;
                    if len > 3 && lang[(len - 3) as usize] == b'_' as c_char {
                        region = lang_ptr.add((len - 2) as usize);
                        lang[(len - 3) as usize] = NUL;
                    } else {
                        dont_use_region = true;
                    }
                    slang = first_lang;
                    while !slang.is_null() {
                        if mb_stricmp(lang_ptr, (*slang).sl_name) == 0 {
                            break;
                        }
                        slang = (*slang).sl_next;
                    }
                }
                if !region.is_null() {
                    if !use_region.is_null() && strcmp(region, use_region) != 0 {
                        dont_use_region = true;
                    }
                    use_region = region;
                }
                if slang.is_null() {
                    if filename {
                        let _ = spell_load_file(lang_ptr, lang_ptr, ptr::null_mut(), false);
                    } else {
                        spell_load_lang(lang_ptr);
                        if !bufref_valid(&raw mut bufref) || !win_valid_any_tab(wp) {
                            ret_msg = c"E797: SpellFileMissing autocommand deleted buffer"
                                .as_ptr()
                                .cast_mut();
                            ga_clear(&raw mut ga);
                            break 'theend;
                        }
                    }
                }
                slang = first_lang;
                while !slang.is_null() {
                    let matched = if filename {
                        path_equal(lang_ptr, (*slang).sl_fname, kPathCmpExpand as PathCmpFlags)
                    } else {
                        mb_stricmp(lang_ptr, (*slang).sl_name) == 0
                    };
                    if matched {
                        let mut region_mask = REGION_ALL;
                        if !filename && !region.is_null() {
                            let c = find_region((&raw const (*slang).sl_regions).cast(), region);
                            if c == REGION_ALL {
                                if (*slang).sl_add {
                                    if (*slang).sl_regions[0] != NUL {
                                        region_mask = 0;
                                    }
                                } else {
                                    smsg!(
                                        0,
                                        gettext(c"Warning: region %s not supported".as_ptr()),
                                        region,
                                    );
                                }
                            } else {
                                region_mask = 1 << c;
                            }
                        }
                        if region_mask != 0 {
                            ga_grow(&raw mut ga, 1);
                            let p_ = langp_entry(&raw const ga, ga.ga_len as usize);
                            ga.ga_len += 1;
                            (*p_).lp_slang = slang;
                            (*p_).lp_sallang = ptr::null_mut();
                            (*p_).lp_replang = ptr::null_mut();
                            (*p_).lp_region = region_mask;
                            use_midword(slang, wp);
                            if (*slang).sl_nobreak {
                                nobreak = true;
                            }
                        }
                    }
                    slang = (*slang).sl_next;
                }
            }
            let cur_w_s = (*curwin).w_s.cast::<synblock_T>();
            let mut spf = (*cur_w_s).b_p_spf;
            let mut round = 0;
            while round == 0 || *spf != NUL {
                let cur_round = round;
                round += 1;
                let spf_ptr = (&raw mut spf_name).cast::<c_char>();
                if cur_round == 0 {
                    if int_wordlist.is_null() {
                        continue;
                    }
                    int_wordlist_spl(spf_ptr);
                } else {
                    let len = copy_option_part(&raw mut spf, spf_ptr, MAXPATHL - 4, c",".as_ptr());
                    strcpy(spf_ptr.add(len), c".spl".as_ptr());
                    let mut c = 0;
                    while c < ga.ga_len as usize {
                        let p = (*(*langp_entry(&raw const ga, c)).lp_slang).sl_fname;
                        if !p.is_null() && path_equal(spf_ptr, p, kPathCmpExpand as PathCmpFlags) {
                            break;
                        }
                        c += 1;
                    }
                    if c < ga.ga_len as usize {
                        continue;
                    }
                }
                let mut slang = first_lang;
                while !slang.is_null() {
                    if path_equal(spf_ptr, (*slang).sl_fname, kPathCmpExpand as PathCmpFlags) {
                        break;
                    }
                    slang = (*slang).sl_next;
                }
                if slang.is_null() {
                    let lang_ptr = (&raw mut lang).cast::<c_char>();
                    if cur_round == 0 {
                        strcpy(lang_ptr, c"internal wordlist".as_ptr());
                    } else {
                        xstrlcpy(lang_ptr, path_tail(spf_ptr), MAXWLEN + 1);
                        let p = vim_strchr(lang_ptr, b'.' as c_int);
                        if !p.is_null() {
                            *p = NUL;
                        }
                    }
                    slang = spell_load_file(spf_ptr, lang_ptr, ptr::null_mut(), true);
                    if !slang.is_null() && nobreak {
                        (*slang).sl_nobreak = true;
                    }
                }
                if !slang.is_null() {
                    let mut region_mask = REGION_ALL;
                    if !use_region.is_null() && !dont_use_region {
                        let c = find_region((&raw const (*slang).sl_regions).cast(), use_region);
                        if c != REGION_ALL {
                            region_mask = 1 << c;
                        } else if (*slang).sl_regions[0] != NUL {
                            region_mask = 0;
                        }
                    }
                    if region_mask != 0 {
                        ga_grow(&raw mut ga, 1);
                        let p_ = langp_entry(&raw const ga, ga.ga_len as usize);
                        ga.ga_len += 1;
                        (*p_).lp_slang = slang;
                        (*p_).lp_sallang = ptr::null_mut();
                        (*p_).lp_replang = ptr::null_mut();
                        (*p_).lp_region = region_mask;
                        use_midword(slang, wp);
                    }
                }
            }
            ga_clear(&raw mut (*w_s).b_langp);
            (*w_s).b_langp = ga;
            for i in 0..(ga.ga_len as usize) {
                let lp = langp_entry(&raw const ga, i);
                if (*(*lp).lp_slang).sl_sal.ga_len > 0 {
                    (*lp).lp_sallang = (*lp).lp_slang;
                } else {
                    for j in 0..(ga.ga_len as usize) {
                        let lp2 = langp_entry(&raw const ga, j);
                        if (*(*lp2).lp_slang).sl_sal.ga_len > 0
                            && strncmp((*(*lp).lp_slang).sl_name, (*(*lp2).lp_slang).sl_name, 2)
                                == 0
                        {
                            (*lp).lp_sallang = (*lp2).lp_slang;
                            break;
                        }
                    }
                }
                if (*(*lp).lp_slang).sl_rep.ga_len > 0 {
                    (*lp).lp_replang = (*lp).lp_slang;
                } else {
                    for j in 0..(ga.ga_len as usize) {
                        let lp2 = langp_entry(&raw const ga, j);
                        if (*(*lp2).lp_slang).sl_rep.ga_len > 0
                            && strncmp((*(*lp).lp_slang).sl_name, (*(*lp2).lp_slang).sl_name, 2)
                                == 0
                        {
                            (*lp).lp_replang = (*lp2).lp_slang;
                            break;
                        }
                    }
                }
            }
            redraw_later(wp, UPD_NOT_VALID);
        }
        xfree(spl_copy.cast());
        parse_spelllang_recursive = false;
        ret_msg
    }
}
unsafe fn clear_midword(wp: *mut win_T) {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        (*w_s).b_spell_ismw = [false; 256];
        xfree((*w_s).b_spell_ismw_mb.cast());
        (*w_s).b_spell_ismw_mb = ptr::null_mut();
    }
}
unsafe fn use_midword(lp: *mut slang_T, wp: *mut win_T) {
    unsafe {
        if (*lp).sl_midword.is_null() {
            return;
        }
        let w_s = (*wp).w_s.cast::<synblock_T>();
        let mut p = (*lp).sl_midword;
        while *p != NUL {
            let c = utf_ptr2char(p);
            let l = utfc_ptr2len(p);
            if (0..256).contains(&c) && l <= 2 {
                (*w_s).b_spell_ismw[c as usize] = true;
            } else if (*w_s).b_spell_ismw_mb.is_null() {
                (*w_s).b_spell_ismw_mb = xmemdupz(p.cast(), l as usize).cast();
            } else {
                let n = strlen((*w_s).b_spell_ismw_mb);
                let bp = xstrnsave((*w_s).b_spell_ismw_mb, n + l as usize);
                xfree((*w_s).b_spell_ismw_mb.cast());
                (*w_s).b_spell_ismw_mb = bp;
                xmemcpyz(bp.add(n).cast(), p.cast(), l as usize);
            }
            p = p.add(l as usize);
        }
    }
}
unsafe fn find_region(rp: *const c_char, region: *const c_char) -> c_int {
    unsafe {
        let mut i: usize = 0;
        loop {
            if *rp.add(i) == NUL {
                return REGION_ALL;
            }
            if *rp.add(i) == *region && *rp.add(i + 1) == *region.add(1) {
                break;
            }
            i += 2;
        }
        (i / 2) as c_int
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn captype(word: *const c_char, end: *const c_char) -> c_int {
    unsafe {
        let mut p = word;
        while !spell_iswordp_nmw(p, curwin) {
            if if end.is_null() { *p == NUL } else { p >= end } {
                return 0;
            }
            mb_cptr_adv(&mut p);
        }
        let mut c = mb_ptr2char_adv(&raw mut p);
        let mut allcap = spell_isupper(c);
        let firstcap = allcap;
        let mut past_second = false;
        while if end.is_null() { *p != NUL } else { p < end } {
            if spell_iswordp_nmw(p, curwin) {
                c = utf_ptr2char(p);
                if !spell_isupper(c) {
                    if past_second && allcap {
                        return WF_KEEPCAP;
                    }
                    allcap = false;
                } else if !allcap {
                    return WF_KEEPCAP;
                }
                past_second = true;
            }
            mb_cptr_adv(&mut p);
        }
        if allcap {
            return WF_ALLCAP;
        }
        if firstcap {
            return WF_ONECAP;
        }
        0
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_delete_wordlist() {
    unsafe {
        if int_wordlist.is_null() {
            return;
        }
        let mut fname: [c_char; MAXPATHL] = [0; MAXPATHL];
        let _ = os_remove(int_wordlist);
        int_wordlist_spl((&raw mut fname).cast());
        let _ = os_remove((&raw const fname).cast());
        xfree(int_wordlist.cast());
        int_wordlist = ptr::null_mut();
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_free_all() {
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            ga_clear(&raw mut (*buf).b_s.b_langp);
            buf = (*buf).b_next;
        }
        while !first_lang.is_null() {
            let slang = first_lang;
            first_lang = (*slang).sl_next;
            slang_free(slang);
        }
        spell_delete_wordlist();
        xfree(repl_to.cast());
        repl_to = ptr::null_mut();
        xfree(repl_from.cast());
        repl_from = ptr::null_mut();
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_reload() {
    unsafe {
        init_spell_chartab();
        spell_free_all();
        let mut wp = firstwin;
        while !wp.is_null() {
            let w_s = (*wp).w_s.cast::<synblock_T>();
            if *(*w_s).b_p_spl != NUL && (*wp).w_onebuf_opt.wo_spell != 0 {
                let _ = parse_spelllang(wp);
                break;
            }
            wp = (*wp).w_next;
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn open_spellbuf() -> *mut buf_T {
    unsafe {
        let buf = xcalloc(1, size_of::<buf_T>()).cast::<buf_T>();
        (*buf).b_spell = true;
        (*buf).b_p_swf = 1;
        if ml_open(buf) == FAIL {
            let _ = logmsg!(
                3,
                ptr::null(),
                c"open_spellbuf".as_ptr(),
                1878,
                true,
                c"Error opening a new memline".as_ptr(),
            );
        }
        ml_open_file(buf);
        buf
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn close_spellbuf(buf: *mut buf_T) {
    unsafe {
        if buf.is_null() {
            return;
        }
        ml_close(buf, 1);
        xfree(buf.cast());
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn clear_spell_chartab(sp: *mut spelltab_T) {
    unsafe {
        (*sp).st_isw = [false; 256];
        (*sp).st_isu = [false; 256];
        for i in 0..256 {
            (*sp).st_fold[i] = i as u8;
            (*sp).st_upper[i] = i as u8;
        }
        for i in (b'0' as usize)..=(b'9' as usize) {
            (*sp).st_isw[i] = true;
        }
        for i in (b'A' as usize)..=(b'Z' as usize) {
            (*sp).st_isw[i] = true;
            (*sp).st_isu[i] = true;
            (*sp).st_fold[i] = (i + 0x20) as u8;
        }
        for i in (b'a' as usize)..=(b'z' as usize) {
            (*sp).st_isw[i] = true;
            (*sp).st_upper[i] = (i - 0x20) as u8;
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn init_spell_chartab() {
    unsafe {
        did_set_spelltab = false;
        clear_spell_chartab(&raw mut spelltab);
        for i in 128..256 {
            let f = utf_fold(i as c_int);
            let u = mb_toupper(i as c_int);
            spelltab.st_isu[i] = mb_isupper(i as c_int);
            spelltab.st_isw[i] = spelltab.st_isu[i] || mb_islower(i as c_int);
            spelltab.st_fold[i] = if f < 256 { f as u8 } else { i as u8 };
            spelltab.st_upper[i] = if u < 256 { u as u8 } else { i as u8 };
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_iswordp(p: *const c_char, wp: *const win_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        let l = utfc_ptr2len(p);
        let mut s = p;
        if l == 1 {
            if (*w_s).b_spell_ismw[*p as u8 as usize] {
                s = p.add(1);
            }
        } else {
            let c = utf_ptr2char(p);
            if if (0..256).contains(&c) {
                (*w_s).b_spell_ismw[c as usize]
            } else {
                !(*w_s).b_spell_ismw_mb.is_null()
                    && !vim_strchr((*w_s).b_spell_ismw_mb, c).is_null()
            } {
                s = p.add(l as usize);
            }
        }
        let c = utf_ptr2char(s);
        if c > 255 {
            return spell_mb_isword_class(mb_get_class(s), wp);
        }
        c >= 0 && spelltab.st_isw[c as usize]
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_iswordp_nmw(p: *const c_char, wp: *mut win_T) -> bool {
    unsafe {
        let c = utf_ptr2char(p);
        if c > 255 {
            return spell_mb_isword_class(mb_get_class(p), wp);
        }
        c >= 0 && spelltab.st_isw[c as usize]
    }
}
unsafe fn spell_mb_isword_class(cl: c_int, wp: *const win_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        if (*w_s).b_cjk != 0 {
            return cl == 2 || cl == 0x2800;
        }
        cl >= 2 && cl != 0x2070 && cl != 0x2080 && cl != 3
    }
}
unsafe fn spell_iswordp_w(p: *const c_int, wp: *const win_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        let s = if if *p < 256 && *p >= 0 {
            (*w_s).b_spell_ismw[*p as usize]
        } else {
            !(*w_s).b_spell_ismw_mb.is_null() && !vim_strchr((*w_s).b_spell_ismw_mb, *p).is_null()
        } {
            p.add(1)
        } else {
            p
        };
        if *s > 255 {
            return spell_mb_isword_class(utf_class(*s), wp);
        }
        *s >= 0 && spelltab.st_isw[*s as usize]
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_casefold(
    wp: *const win_T,
    str_: *const c_char,
    len: c_int,
    buf: *mut c_char,
    buflen: c_int,
) -> c_int {
    unsafe {
        if len >= buflen {
            *buf = NUL;
            return FAIL;
        }
        let mut outi: c_int = 0;
        let mut p = str_;
        let end = str_.add(len as usize);
        while p < end {
            if outi + MB_MAXBYTES as c_int > buflen {
                *buf.add(outi as usize) = NUL;
                return FAIL;
            }
            let mut c = mb_cptr2char_adv(&raw mut p);
            if c == 0x03a3 || c == 0x03c2 {
                if p == end || !spell_iswordp(p, wp) {
                    c = 0x03c2;
                } else {
                    c = 0x03c3;
                }
            } else {
                c = spell_tofold(c);
            }
            outi += utf_char2bytes(c, buf.add(outi as usize));
        }
        *buf.add(outi as usize) = NUL;
        OK
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn check_need_cap(wp: *mut win_T, lnum: linenr_T, col: colnr_T) -> bool {
    unsafe {
        let w_s = (*wp).w_s.cast::<synblock_T>();
        if (*w_s).b_cap_prog.is_null() {
            return false;
        }
        let mut need_cap = false;
        let mut line: *mut c_char = if col != 0 {
            ml_get_buf((*wp).w_buffer, lnum)
        } else {
            ptr::null_mut()
        };
        let mut line_copy: *mut c_char = ptr::null_mut();
        let mut endcol: colnr_T = 0;
        if col == 0 || (getwhitecols(line) as colnr_T) >= col {
            if lnum == 1 {
                need_cap = true;
            } else {
                line = ml_get_buf((*wp).w_buffer, lnum - 1);
                if *skipwhite(line) == NUL {
                    need_cap = true;
                } else {
                    line_copy = concat_str(line, c" ".as_ptr());
                    line = line_copy;
                    endcol = strlen(line) as colnr_T;
                }
            }
        } else {
            endcol = col;
        }
        if endcol > 0 {
            let mut regmatch: regmatch_T = std::mem::MaybeUninit::zeroed().assume_init();
            regmatch.regprog = (*w_s).b_cap_prog.cast();
            regmatch.rm_ic = false;
            let mut p = line.add(endcol as usize);
            loop {
                mb_ptr_back(line, &mut p);
                if p == line || spell_iswordp_nmw(p, wp) {
                    break;
                }
                if vim_regexec(&raw mut regmatch, p, 0)
                    && regmatch.endp[0] == line.add(endcol as usize)
                {
                    need_cap = true;
                    break;
                }
            }
            (*w_s).b_cap_prog = regmatch.regprog.cast();
        }
        xfree(line_copy.cast());
        need_cap
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_spellrepall(_eap: *mut exarg_T) {
    unsafe {
        let pos = (*curwin).w_cursor;
        let save_ws = p_ws;
        let mut prev_lnum: linenr_T = 0;
        if repl_from.is_null() || repl_to.is_null() {
            emsg(gettext(c"E752: No previous spell replacement".as_ptr()));
            return;
        }
        let repl_from_len = strlen(repl_from);
        let repl_to_len = strlen(repl_to);
        let addlen = repl_to_len as i64 - repl_from_len as i64;
        let frompatsize = repl_from_len + 7;
        let frompat = xmalloc(frompatsize).cast::<c_char>();
        let frompatlen =
            vim_snprintf!(frompat, frompatsize, c"\\V\\<%s\\>".as_ptr(), repl_from) as usize;
        p_ws = 0;
        sub_nsubs = 0;
        sub_nlines = 0;
        (*curwin).w_cursor.lnum = 0;
        while !got_int {
            if do_search(
                ptr::null_mut(),
                b'/' as c_int,
                b'/' as c_int,
                frompat,
                frompatlen,
                1,
                SEARCH_KEEP,
                false,
                ptr::null_mut(),
            ) == 0
                || u_save_cursor() == FAIL
            {
                break;
            }
            let line = get_cursor_line_ptr();
            if addlen <= 0
                || strncmp(
                    line.add((*curwin).w_cursor.col as usize),
                    repl_to,
                    repl_to_len,
                ) != 0
            {
                let p =
                    xmalloc((get_cursor_line_len() as i64 + addlen + 1) as usize).cast::<c_char>();
                ptr::copy(line, p, (*curwin).w_cursor.col as usize);
                strcpy(p.add((*curwin).w_cursor.col as usize), repl_to);
                strcat(p, line.add((*curwin).w_cursor.col as usize + repl_from_len));
                let _ = ml_replace((*curwin).w_cursor.lnum, p, false);
                inserted_bytes(
                    (*curwin).w_cursor.lnum,
                    (*curwin).w_cursor.col,
                    repl_from_len as c_int,
                    repl_to_len as c_int,
                );
                if (*curwin).w_cursor.lnum != prev_lnum {
                    sub_nlines += 1;
                    prev_lnum = (*curwin).w_cursor.lnum;
                }
                sub_nsubs += 1;
            }
            (*curwin).w_cursor.col += repl_to_len as colnr_T;
        }
        p_ws = save_ws;
        (*curwin).w_cursor = pos;
        xfree(frompat.cast());
        if sub_nsubs == 0 {
            semsg!(gettext(c"E753: Not found: %s".as_ptr()), repl_from);
        } else {
            do_sub_msg(false);
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn onecap_copy(word: *const c_char, wcopy: *mut c_char, upper: bool) {
    unsafe {
        let mut p = word;
        let mut c = mb_cptr2char_adv(&raw mut p);
        c = if upper {
            spell_toupper(c)
        } else {
            spell_tofold(c)
        };
        let l = utf_char2bytes(c, wcopy) as usize;
        xstrlcpy(wcopy.add(l), p, MAXWLEN - l);
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn allcap_copy(word: *const c_char, wcopy: *mut c_char) {
    unsafe {
        let mut d = wcopy;
        let mut s = word;
        while *s != NUL {
            let mut c = mb_cptr2char_adv(&raw mut s);
            if c == 0xdf {
                c = b'S' as c_int;
                if d.offset_from(wcopy) as usize >= MAXWLEN - 1 {
                    break;
                }
                *d = c as c_char;
                d = d.add(1);
            } else {
                c = spell_toupper(c);
            }
            if d.offset_from(wcopy) as usize >= MAXWLEN - MB_MAXBYTES {
                break;
            }
            d = d.add(utf_char2bytes(c, d) as usize);
        }
        *d = NUL;
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nofold_len(fword: *mut c_char, flen: c_int, word: *mut c_char) -> c_int {
    unsafe {
        let mut p = fword;
        let mut i: c_int = 0;
        let f_end = fword.add(flen as usize);
        while p < f_end {
            i += 1;
            mb_ptr_adv(&mut p);
        }
        p = word;
        while i > 0 {
            i -= 1;
            mb_ptr_adv(&mut p);
        }
        p.offset_from(word) as c_int
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn make_case_word(fword: *mut c_char, cword: *mut c_char, flags: c_int) {
    unsafe {
        if (flags & WF_ALLCAP) != 0 {
            allcap_copy(fword, cword);
        } else if (flags & WF_ONECAP) != 0 {
            onecap_copy(fword, cword, true);
        } else {
            strcpy(cword, fword);
        }
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn eval_soundfold(word: *const c_char) -> *mut c_char {
    unsafe {
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        if (*curwin).w_onebuf_opt.wo_spell != 0 && *(*w_s).b_p_spl != NUL {
            for lpi in 0..((*w_s).b_langp.ga_len as usize) {
                let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
                if (*(*lp).lp_slang).sl_sal.ga_len > 0 {
                    let mut sound: [c_char; MAXWLEN] = [0; MAXWLEN];
                    spell_soundfold(
                        (*lp).lp_slang,
                        word.cast_mut(),
                        false,
                        (&raw mut sound).cast(),
                    );
                    return xstrdup((&raw const sound).cast());
                }
            }
        }
        xstrdup(word)
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_soundfold(
    slang: *mut slang_T,
    inword: *mut c_char,
    folded: bool,
    res: *mut c_char,
) {
    unsafe {
        if (*slang).sl_sofo {
            spell_soundfold_sofo(slang, inword, res);
        } else {
            let mut fword: [c_char; MAXWLEN] = [0; MAXWLEN];
            let word: *const c_char = if folded {
                inword
            } else {
                spell_casefold(
                    curwin,
                    inword,
                    strlen(inword) as c_int,
                    (&raw mut fword).cast(),
                    MAXWLEN as c_int,
                );
                (&raw const fword).cast()
            };
            spell_soundfold_wsal(slang, word, res);
        }
    }
}
unsafe fn spell_soundfold_sofo(slang: *mut slang_T, inword: *const c_char, res: *mut c_char) {
    unsafe {
        let mut ri: usize = 0;
        let mut prevc: c_int = 0;
        let mut s = inword;
        while *s != NUL {
            let mut c = mb_cptr2char_adv(&raw mut s);
            if utf_class(c) == 0 {
                c = b' ' as c_int;
            } else if (0..256).contains(&c) {
                c = (*slang).sl_sal_first[c as usize];
            } else {
                let mut ip =
                    *((*slang).sl_sal.ga_data.cast::<*mut c_int>()).add((c & 0xff) as usize);
                if ip.is_null() {
                    c = NUL as c_int;
                } else {
                    loop {
                        if *ip == 0 {
                            c = NUL as c_int;
                            break;
                        }
                        if *ip == c {
                            c = *ip.add(1);
                            break;
                        }
                        ip = ip.add(2);
                    }
                }
            }
            if c != NUL as c_int && c != prevc {
                ri += utf_char2bytes(c, res.add(ri)) as usize;
                if ri + MB_MAXBYTES > MAXWLEN {
                    break;
                }
                prevc = c;
            }
        }
        *res.add(ri) = NUL;
    }
}
unsafe fn spell_soundfold_wsal(slang: *mut slang_T, inword: *const c_char, res: *mut c_char) {
    unsafe {
        let mut word: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut did_white = false;
        let mut wordlen: usize = 0;
        let mut s = inword;
        while *s != NUL {
            let t = s;
            let mut c = mb_cptr2char_adv(&raw mut s);
            if (*slang).sl_rem_accents {
                if utf_class(c) == 0 {
                    if did_white {
                        continue;
                    }
                    c = b' ' as c_int;
                    did_white = true;
                } else {
                    did_white = false;
                    if !spell_iswordp_nmw(t, curwin) {
                        continue;
                    }
                }
            }
            if wordlen < MAXWLEN - 1 {
                word[wordlen] = c;
                wordlen += 1;
            }
        }
        word[wordlen] = 0;
        let smp = (*slang).sl_sal.ga_data.cast::<salitem_T>();
        let mut wres: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut k: usize = 0;
        let mut p0: c_int = -333;
        let mut i: usize = 0;
        let mut reslen: usize = 0;
        let mut z: c_int = 0;
        while word[i] != 0 {
            let mut c = word[i];
            let mut n = (*slang).sl_sal_first[(c & 0xff) as usize];
            let mut z0: c_int = 0;
            if n >= 0 {
                while n < (*slang).sl_sal.ga_len {
                    let cur_n = n as usize;
                    let mut ws = (*smp.add(cur_n)).sm_lead_w;
                    if (*ws & 0xff) != (c & 0xff) || *ws == 0 {
                        break;
                    }
                    n += 1;
                    if c != *ws {
                        continue;
                    }
                    k = (*smp.add(cur_n)).sm_leadlen as usize;
                    if k > 1 {
                        if word[i + 1] != *ws.add(1) {
                            continue;
                        }
                        if k > 2 {
                            let mut j: usize = 2;
                            while j < k {
                                if word[i + j] != *ws.add(j) {
                                    break;
                                }
                                j += 1;
                            }
                            if j < k {
                                continue;
                            }
                        }
                    }
                    let mut pf = (*smp.add(cur_n)).sm_oneof_w;
                    if !pf.is_null() {
                        while *pf != 0 && *pf != word[i + k] {
                            pf = pf.add(1);
                        }
                        if *pf == 0 {
                            continue;
                        }
                        k += 1;
                    }
                    let mut sr = (*smp.add(cur_n)).sm_rules;
                    let mut pri: c_int = 5;
                    p0 = *sr as u8 as c_int;
                    let mut k0 = k;
                    while *sr == b'-' as c_char && k > 1 {
                        k -= 1;
                        sr = sr.add(1);
                    }
                    if *sr == b'<' as c_char {
                        sr = sr.add(1);
                    }
                    if ascii_isdigit(*sr as u8 as c_int) {
                        pri = (*sr as u8 - b'0') as c_int;
                        sr = sr.add(1);
                    }
                    if *sr == b'^' as c_char && *sr.add(1) == b'^' as c_char {
                        sr = sr.add(1);
                    }
                    let word_ptr = (&raw const word).cast::<c_int>();
                    if *sr == NUL
                        || (*sr == b'^' as c_char
                            && (i == 0
                                || !(word[i - 1] == b' ' as c_int
                                    || spell_iswordp_w(word_ptr.add(i - 1), curwin)))
                            && (*sr.add(1) != b'$' as c_char
                                || !spell_iswordp_w(word_ptr.add(i + k0), curwin)))
                        || (*sr == b'$' as c_char
                            && i > 0
                            && spell_iswordp_w(word_ptr.add(i - 1), curwin)
                            && !spell_iswordp_w(word_ptr.add(i + k0), curwin))
                    {
                        let c0 = word[i + k - 1];
                        let mut n0 = (*slang).sl_sal_first[(c0 & 0xff) as usize];
                        if (*slang).sl_followup
                            && k > 1
                            && n0 >= 0
                            && p0 != b'-' as c_int
                            && word[i + k] != 0
                        {
                            while (*(*smp.add(n0 as usize)).sm_lead_w & 0xff) == (c0 & 0xff) {
                                let cur_n0 = n0 as usize;
                                ws = (*smp.add(cur_n0)).sm_lead_w;
                                if c0 != *ws {
                                    n0 += 1;
                                    continue;
                                }
                                k0 = (*smp.add(cur_n0)).sm_leadlen as usize;
                                if k0 > 1 {
                                    if word[i + k] != *ws.add(1) {
                                        n0 += 1;
                                        continue;
                                    }
                                    if k0 > 2 {
                                        let mut pf2 = word_ptr.add(i + k + 1);
                                        let mut j: usize = 2;
                                        while j < k0 {
                                            if *pf2 != *ws.add(j) {
                                                break;
                                            }
                                            pf2 = pf2.add(1);
                                            j += 1;
                                        }
                                        if j < k0 {
                                            n0 += 1;
                                            continue;
                                        }
                                    }
                                }
                                k0 += k - 1;
                                pf = (*smp.add(cur_n0)).sm_oneof_w;
                                if !pf.is_null() {
                                    while *pf != 0 && *pf != word[i + k0] {
                                        pf = pf.add(1);
                                    }
                                    if *pf == 0 {
                                        n0 += 1;
                                        continue;
                                    }
                                    k0 += 1;
                                }
                                p0 = 5;
                                sr = (*smp.add(cur_n0)).sm_rules;
                                while *sr == b'-' as c_char {
                                    sr = sr.add(1);
                                }
                                if *sr == b'<' as c_char {
                                    sr = sr.add(1);
                                }
                                if ascii_isdigit(*sr as u8 as c_int) {
                                    p0 = (*sr as u8 - b'0') as c_int;
                                    sr = sr.add(1);
                                }
                                if *sr == NUL
                                    || (*sr == b'$' as c_char
                                        && !spell_iswordp_w(word_ptr.add(i + k0), curwin))
                                {
                                    if k0 == k {
                                        n0 += 1;
                                        continue;
                                    }
                                    if p0 < pri {
                                        n0 += 1;
                                        continue;
                                    }
                                    break;
                                }
                                n0 += 1;
                            }
                            if p0 >= pri
                                && (*(*smp.add(n0 as usize)).sm_lead_w & 0xff) == (c0 & 0xff)
                            {
                                continue;
                            }
                        }
                        ws = (*smp.add(cur_n)).sm_to_w;
                        sr = (*smp.add(cur_n)).sm_rules;
                        p0 = c_int::from(!vim_strchr(sr, b'<' as c_int).is_null());
                        if p0 == 1 && z == 0 {
                            if reslen > 0
                                && !ws.is_null()
                                && *ws != 0
                                && (wres[reslen - 1] == c || wres[reslen - 1] == *ws)
                            {
                                reslen -= 1;
                            }
                            z0 = 1;
                            z = 1;
                            k0 = 0;
                            if !ws.is_null() {
                                while *ws != 0 && word[i + k0] != 0 {
                                    word[i + k0] = *ws;
                                    k0 += 1;
                                    ws = ws.add(1);
                                }
                            }
                            if k > k0 {
                                let w_mut = (&raw mut word).cast::<c_int>();
                                ptr::copy(
                                    w_mut.add(i + k),
                                    w_mut.add(i + k0),
                                    wordlen - (i + k) + 1,
                                );
                            }
                            c = word[i];
                        } else {
                            i += k - 1;
                            z = 0;
                            if !ws.is_null() {
                                while *ws != 0 && *ws.add(1) != 0 && reslen < MAXWLEN {
                                    if reslen == 0 || wres[reslen - 1] != *ws {
                                        wres[reslen] = *ws;
                                        reslen += 1;
                                    }
                                    ws = ws.add(1);
                                }
                            }
                            c = if ws.is_null() { 0 } else { *ws };
                            if !strstr(sr, c"^^".as_ptr()).is_null() {
                                if c != 0 && reslen < MAXWLEN {
                                    wres[reslen] = c;
                                    reslen += 1;
                                }
                                let w_mut = (&raw mut word).cast::<c_int>();
                                ptr::copy(w_mut.add(i + 1), w_mut, wordlen - (i + 1) + 1);
                                i = 0;
                                z0 = 1;
                            }
                        }
                        break;
                    }
                }
            } else if ascii_iswhite(c) {
                c = b' ' as c_int;
                k = 1;
            }
            if z0 == 0 {
                if k != 0
                    && p0 == 0
                    && reslen < MAXWLEN
                    && c != 0
                    && (!(*slang).sl_collapse || reslen == 0 || wres[reslen - 1] != c)
                {
                    wres[reslen] = c;
                    reslen += 1;
                }
                i += 1;
                z = 0;
                k = 0;
            }
        }
        let mut l: usize = 0;
        for &wc in wres.iter().take(reslen) {
            l += utf_char2bytes(wc, res.add(l)) as usize;
            if l + MB_MAXBYTES > MAXWLEN {
                break;
            }
        }
        *res.add(l) = NUL;
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_spellinfo(_eap: *mut exarg_T) {
    unsafe {
        if no_spell_checking(curwin) {
            return;
        }
        msg_ext_set_kind(c"list_cmd".as_ptr());
        msg_start();
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        let mut lpi: c_int = 0;
        while lpi < (*w_s).b_langp.ga_len && !got_int {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi as usize);
            msg_puts(c"file: ".as_ptr());
            msg_puts((*(*lp).lp_slang).sl_fname);
            let p = (*(*lp).lp_slang).sl_info;
            if lpi < (*w_s).b_langp.ga_len || !p.is_null() {
                msg_putchar(b'\n' as c_int);
            }
            if !p.is_null() {
                msg_puts(p);
                if lpi < (*w_s).b_langp.ga_len - 1 {
                    msg_putchar(b'\n' as c_int);
                }
            }
            lpi += 1;
        }
        msg_end();
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn ex_spelldump(eap: *mut exarg_T) {
    unsafe {
        if no_spell_checking(curwin) {
            return;
        }
        let spl = get_option_value(kOptSpelllang, OPT_LOCAL);
        do_cmdline_cmd(c"new".as_ptr());
        set_option_value_give_err(kOptSpell, boolean_obj(true), OPT_LOCAL);
        set_option_value_give_err(kOptSpelllang, spl, OPT_LOCAL);
        optval_free(spl);
        if !buf_is_empty(curbuf) {
            return;
        }
        spell_dump_compl(
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            if (*eap).forceit != 0 {
                DUMPFLAG_COUNT
            } else {
                0
            },
        );
        if (*curbuf).b_ml.ml_line_count > 1 {
            let _ = ml_delete((*curbuf).b_ml.ml_line_count);
        }
        redraw_later(curwin, UPD_NOT_VALID);
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_dump_compl(
    pat: *mut c_char,
    ic: c_int,
    dir: *mut Direction,
    dumpflags_arg: c_int,
) {
    unsafe {
        let mut arridx: [idx_T; MAXWLEN] = [0; MAXWLEN];
        let mut curi: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut word: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut lnum: linenr_T = 0;
        let mut region_names: *mut c_char = ptr::null_mut();
        let mut do_region = true;
        let mut dumpflags = dumpflags_arg;
        if !pat.is_null() {
            if ic != 0 {
                dumpflags |= DUMPFLAG_ICASE;
            } else {
                let n = captype(pat, ptr::null());
                if n == WF_ONECAP {
                    dumpflags |= DUMPFLAG_ONECAP;
                } else if n == WF_ALLCAP && (strlen(pat) as c_int) > utfc_ptr2len(pat) {
                    dumpflags |= DUMPFLAG_ALLCAP;
                }
            }
        }
        let w_s = (*curwin).w_s.cast::<synblock_T>();
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            let p = (&raw mut (*(*lp).lp_slang).sl_regions).cast::<c_char>();
            if *p != 0 {
                if region_names.is_null() {
                    region_names = p;
                } else if strcmp(region_names, p) != 0 {
                    do_region = false;
                    break;
                }
            }
        }
        if do_region && !region_names.is_null() && pat.is_null() {
            vim_snprintf!(
                (&raw mut IObuff).cast(),
                IOSIZE,
                c"/regions=%s".as_ptr(),
                region_names,
            );
            let _ = ml_append(lnum, (&raw mut IObuff).cast(), 0, false);
            lnum += 1;
        } else {
            do_region = false;
        }
        for lpi in 0..((*w_s).b_langp.ga_len as usize) {
            let lp = langp_entry(&raw const (*w_s).b_langp, lpi);
            let slang = (*lp).lp_slang;
            if (*slang).sl_fbyts.is_null() {
                continue;
            }
            if pat.is_null() {
                vim_snprintf!(
                    (&raw mut IObuff).cast(),
                    IOSIZE,
                    c"# file: %s".as_ptr(),
                    (*slang).sl_fname,
                );
                let _ = ml_append(lnum, (&raw mut IObuff).cast(), 0, false);
                lnum += 1;
            }
            let patlen: isize = if !pat.is_null() && (*slang).sl_pbyts.is_null() {
                strlen(pat) as isize
            } else {
                -1
            };
            for round in 1..=2 {
                let (byts, idxs) = if round == 1 {
                    dumpflags &= !DUMPFLAG_KEEPCASE;
                    ((*slang).sl_fbyts, (*slang).sl_fidxs)
                } else {
                    dumpflags |= DUMPFLAG_KEEPCASE;
                    ((*slang).sl_kbyts, (*slang).sl_kidxs)
                };
                if byts.is_null() {
                    continue;
                }
                let mut depth: isize = 0;
                arridx[0] = 0;
                curi[0] = 1;
                while depth >= 0 && !got_int && (pat.is_null() || !ins_compl_interrupted()) {
                    let udepth = depth as usize;
                    if curi[udepth] > *byts.add(arridx[udepth] as usize) as c_int {
                        depth -= 1;
                        line_breakcheck();
                        ins_compl_check_keys(50, false);
                    } else {
                        let n = arridx[udepth] + curi[udepth];
                        curi[udepth] += 1;
                        let mut c = *byts.add(n as usize) as c_int;
                        if c == 0 || udepth >= MAXWLEN - 1 {
                            let mut flags = *idxs.add(n as usize);
                            if (round == 2 || (flags & WF_KEEPCAP) == 0)
                                && (flags & WF_NEEDCOMP) == 0
                                && (do_region
                                    || (flags & WF_REGION) == 0
                                    || (((flags as u32) >> 16) & ((*lp).lp_region as u32)) != 0)
                            {
                                word[udepth] = NUL;
                                if !do_region {
                                    flags &= !WF_REGION;
                                }
                                c = ((flags as u32) >> 24) as c_int;
                                if c == 0 || curi[udepth] == 2 {
                                    dump_word(
                                        slang,
                                        (&raw mut word).cast(),
                                        pat,
                                        dir,
                                        dumpflags,
                                        flags,
                                        lnum,
                                    );
                                    if pat.is_null() {
                                        lnum += 1;
                                    }
                                }
                                if c != 0 {
                                    lnum = dump_prefixes(
                                        slang,
                                        (&raw mut word).cast(),
                                        pat,
                                        dir,
                                        dumpflags,
                                        flags,
                                        lnum,
                                    );
                                }
                            }
                        } else {
                            word[udepth] = c as c_char;
                            depth += 1;
                            arridx[depth as usize] = *idxs.add(n as usize);
                            curi[depth as usize] = 1;
                            if depth <= patlen
                                && mb_strnicmp((&raw const word).cast(), pat, depth as usize) != 0
                            {
                                depth -= 1;
                            }
                        }
                    }
                }
            }
        }
    }
}
unsafe fn dump_word(
    slang: *mut slang_T,
    word: *mut c_char,
    pat: *mut c_char,
    dir: *mut Direction,
    dumpflags: c_int,
    wordflags: c_int,
    lnum: linenr_T,
) {
    unsafe {
        let mut keepcap = false;
        let mut cword: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut badword: [c_char; MAXWLEN + 10] = [0; MAXWLEN + 10];
        let mut flags = wordflags;
        if (dumpflags & DUMPFLAG_ONECAP) != 0 {
            flags |= WF_ONECAP;
        }
        if (dumpflags & DUMPFLAG_ALLCAP) != 0 {
            flags |= WF_ALLCAP;
        }
        let mut p: *mut c_char =
            if (dumpflags & DUMPFLAG_KEEPCASE) == 0 && (flags & WF_CAPMASK) != 0 {
                make_case_word(word, (&raw mut cword).cast(), flags);
                (&raw mut cword).cast()
            } else {
                if (dumpflags & DUMPFLAG_KEEPCASE) != 0
                    && ((captype(word, ptr::null()) & WF_KEEPCAP) == 0 || (flags & WF_FIXCAP) != 0)
                {
                    keepcap = true;
                }
                word
            };
        let tw = p;
        if pat.is_null() {
            if (flags & (WF_BANNED | WF_RARE | WF_REGION)) != 0 || keepcap {
                let bw_ptr = (&raw mut badword).cast::<c_char>();
                strcpy(bw_ptr, p);
                strcat(bw_ptr, c"/".as_ptr());
                if keepcap {
                    strcat(bw_ptr, c"=".as_ptr());
                }
                if (flags & WF_BANNED) != 0 {
                    strcat(bw_ptr, c"!".as_ptr());
                } else if (flags & WF_RARE) != 0 {
                    strcat(bw_ptr, c"?".as_ptr());
                }
                if (flags & WF_REGION) != 0 {
                    for i in 0..7 {
                        if (flags & (0x10000 << i)) != 0 {
                            let badword_len = strlen(bw_ptr);
                            vim_snprintf!(
                                bw_ptr.add(badword_len),
                                badword.len() - badword_len,
                                c"%d".as_ptr(),
                                i + 1,
                            );
                        }
                    }
                }
                p = bw_ptr;
            }
            if (dumpflags & DUMPFLAG_COUNT) != 0 {
                let hi = hash_find(&raw mut (*slang).sl_wordcount, tw);
                if !hashitem_empty(&*hi) {
                    let wc = (*hi)
                        .hi_key
                        .sub(std::mem::offset_of!(wordcount_T, wc_word))
                        .cast::<wordcount_T>();
                    vim_snprintf!(
                        (&raw mut IObuff).cast(),
                        IOSIZE,
                        c"%s\t%d".as_ptr(),
                        tw,
                        (*wc).wc_count as c_int,
                    );
                    p = (&raw mut IObuff).cast();
                }
            }
            let _ = ml_append(lnum, p, 0, false);
        } else if (if (dumpflags & DUMPFLAG_ICASE) != 0 {
            mb_strnicmp(p, pat, strlen(pat)) == 0
        } else {
            strncmp(p, pat, strlen(pat)) == 0
        }) && ins_compl_add_infercase(
            p,
            strlen(p) as c_int,
            p_ic != 0,
            ptr::null_mut(),
            *dir,
            false,
            0,
        ) == OK
        {
            *dir = FORWARD;
        }
    }
}
unsafe fn dump_prefixes(
    slang: *mut slang_T,
    word: *mut c_char,
    pat: *mut c_char,
    dir: *mut Direction,
    dumpflags: c_int,
    flags: c_int,
    startlnum: linenr_T,
) -> linenr_T {
    unsafe {
        let mut arridx: [idx_T; MAXWLEN] = [0; MAXWLEN];
        let mut curi: [c_int; MAXWLEN] = [0; MAXWLEN];
        let mut prefix: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut word_up: [c_char; MAXWLEN] = [0; MAXWLEN];
        let mut has_word_up = false;
        let mut lnum = startlnum;
        let mut c = utf_ptr2char(word);
        if spell_toupper(c) != c {
            onecap_copy(word, (&raw mut word_up).cast(), true);
            has_word_up = true;
        }
        let byts = (*slang).sl_pbyts;
        let idxs = (*slang).sl_pidxs;
        if !byts.is_null() {
            let mut depth: isize = 0;
            arridx[0] = 0;
            curi[0] = 1;
            while depth >= 0 && !got_int {
                let udepth = depth as usize;
                let mut n = arridx[udepth];
                let len = *byts.add(n as usize) as c_int;
                if curi[udepth] > len {
                    depth -= 1;
                    line_breakcheck();
                } else {
                    n += curi[udepth];
                    curi[udepth] += 1;
                    c = *byts.add(n as usize) as c_int;
                    if c == 0 {
                        let mut i: c_int = 1;
                        while i < len {
                            if *byts.add((n + i) as usize) != 0 {
                                break;
                            }
                            i += 1;
                        }
                        curi[udepth] += i - 1;
                        c = valid_word_prefix(i, n, flags, word, slang, false);
                        if c != 0 {
                            xstrlcpy(
                                (&raw mut prefix).cast::<c_char>().add(udepth),
                                word,
                                MAXWLEN - udepth,
                            );
                            dump_word(
                                slang,
                                (&raw mut prefix).cast(),
                                pat,
                                dir,
                                dumpflags,
                                if (c & WF_RAREPFX) != 0 {
                                    flags | WF_RARE
                                } else {
                                    flags
                                },
                                lnum,
                            );
                            if lnum != 0 {
                                lnum += 1;
                            }
                        }
                        if has_word_up {
                            c = valid_word_prefix(
                                i,
                                n,
                                flags,
                                (&raw mut word_up).cast(),
                                slang,
                                true,
                            );
                            if c != 0 {
                                xstrlcpy(
                                    (&raw mut prefix).cast::<c_char>().add(udepth),
                                    (&raw const word_up).cast(),
                                    MAXWLEN - udepth,
                                );
                                dump_word(
                                    slang,
                                    (&raw mut prefix).cast(),
                                    pat,
                                    dir,
                                    dumpflags,
                                    if (c & WF_RAREPFX) != 0 {
                                        flags | WF_RARE
                                    } else {
                                        flags
                                    },
                                    lnum,
                                );
                                if lnum != 0 {
                                    lnum += 1;
                                }
                            }
                        }
                    } else if udepth < MAXWLEN - 1 {
                        prefix[udepth] = c as c_char;
                        depth += 1;
                        arridx[depth as usize] = *idxs.add(n as usize);
                        curi[depth as usize] = 1;
                    }
                }
            }
        }
        lnum
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_to_word_end(start: *mut c_char, win: *mut win_T) -> *mut c_char {
    unsafe {
        let mut p = start;
        while *p != NUL && spell_iswordp(p, win) {
            mb_ptr_adv(&mut p);
        }
        p
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_word_start(startcol: c_int) -> c_int {
    unsafe {
        if no_spell_checking(curwin) {
            return startcol;
        }
        let line = get_cursor_line_ptr();
        let mut p = line.add(startcol as usize);
        while p > line {
            mb_ptr_back(line, &mut p);
            if spell_iswordp_nmw(p, curwin) {
                break;
            }
        }
        let mut col: c_int = 0;
        while p > line {
            col = p.offset_from(line) as c_int;
            mb_ptr_back(line, &mut p);
            if !spell_iswordp(p, curwin) {
                break;
            }
            col = 0;
        }
        col
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn spell_expand_check_cap(col: colnr_T) {
    unsafe {
        spell_expand_need_cap = check_need_cap(curwin, (*curwin).w_cursor.lnum, col);
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn expand_spelling(
    _lnum: linenr_T,
    pat: *mut c_char,
    matchp: *mut *mut *mut c_char,
) -> c_int {
    unsafe {
        let mut ga = garray_T {
            ga_len: 0,
            ga_maxlen: 0,
            ga_itemsize: 0,
            ga_growsize: 0,
            ga_data: ptr::null_mut(),
        };
        spell_suggest_list(&raw mut ga, pat, 100, spell_expand_need_cap, true);
        *matchp = ga.ga_data.cast();
        ga.ga_len
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_spelllang(val: *const c_char) -> bool {
    unsafe { valid_name(val, c".-_,@".as_ptr()) }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn valid_spellfile(val: *const c_char) -> bool {
    unsafe {
        let mut spf_name: [c_char; MAXPATHL] = [0; MAXPATHL];
        let mut spf = val.cast_mut();
        while *spf != NUL {
            let l = copy_option_part(
                &raw mut spf,
                (&raw mut spf_name).cast(),
                MAXPATHL,
                c",".as_ptr(),
            );
            let spf_ptr = (&raw mut spf_name).cast::<c_char>();
            if !(4..(MAXPATHL - 4)).contains(&l)
                || strcmp(spf_ptr.add(l - 4), c".add".as_ptr()) != 0
            {
                return false;
            }
            #[cfg(windows)]
            {
                path_to_slash(spf_ptr);
            }
            let mut s = spf_ptr;
            while *s != NUL {
                if !vim_is_fname_char(*s as u8 as c_int) {
                    return false;
                }
                s = s.add(1);
            }
        }
        true
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn did_set_spell_option() -> *const c_char {
    unsafe {
        let mut errmsg: *const c_char = ptr::null();
        let mut wp = firstwin;
        while !wp.is_null() {
            if (*wp).w_buffer == curbuf && (*wp).w_onebuf_opt.wo_spell != 0 {
                errmsg = parse_spelllang(wp);
                break;
            }
            wp = (*wp).w_next;
        }
        errmsg
    }
}
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn compile_cap_prog(
    value: *const c_char,
    prog: *mut *mut regprog_T,
) -> *const c_char {
    unsafe {
        let mut rp: *mut regprog_T = ptr::null_mut();
        if !value.is_null() && *value != NUL {
            let re = concat_str(c"^".as_ptr(), value);
            rp = vim_regcomp(re, RE_MAGIC);
            xfree(re.cast());
            if rp.is_null() {
                return (&raw const e_invarg).cast();
            }
        }
        vim_regfree(*prog);
        *prog = rp;
        ptr::null()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_in_str_and_spell_valid_case_work() {
        unsafe {
            let mut s = *b"abc\0";
            assert!(byte_in_str(s.as_mut_ptr(), b'b' as c_int));
            assert!(!byte_in_str(s.as_mut_ptr(), b'z' as c_int));
            assert!(spell_valid_case(WF_ALLCAP, 0));
            assert!(!spell_valid_case(WF_ALLCAP, WF_KEEPCAP | WF_FIXCAP));
            assert!(spell_valid_case(WF_ONECAP, WF_ONECAP));
            assert!(!spell_valid_case(0, WF_ONECAP));
        }
    }
}
