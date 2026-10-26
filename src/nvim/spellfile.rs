//! Code for reading and writing spell files (`src/nvim/spellfile.c`).

use crate::api::private::helpers::cstr_as_obj;
use crate::arglist::get_arglist_exp;
use crate::buffer::{buf_T, buflist_findname_exp};
use crate::charset::{NUL, ascii_isdigit, getdigits_int, skipdigits, skipwhite};
use crate::cursor::linenr_T;
use crate::drawscreen::{UPD_SOME_VALID, redraw_all_later};
use crate::errors::*;
use crate::ex_docmd::exarg_T;
use crate::fileio::{
    EOF, FILE, SEEK_SET, buf_reload, fclose, feof, ferror, fgetc, fputc, fread, fseek, ftell,
    fwrite, get2c, get3c, get4c, get8ctime, put_bytes, put_time, read_string, vim_fgets,
    vim_tempname,
};
use crate::globals::*;
use crate::mbyte::{
    convert_setup, enc_canonize, mb_charlen, mb_cptr2char_adv, mb_ptr_adv, mb_ptr_back,
    mb_ptr2char_adv, string_convert, utf_char2bytes, utf_ptr2char, utf_valid_string,
    utfc_ptr2len, vimconv_T,
};
use crate::memline::{ml_append_buf, ml_get_buf, ml_get_buf_len};
use crate::memory::{strequal, xfree, xmalloc, xmemcpyz, xstrdup, xstrlcat, xstrlcpy};
use crate::message::{
    emsg, msg, msg_clr_eos, msg_outtrans_long, msg_puts, msg_start, semsg, smsg, verbose_enter,
    verbose_leave,
};
use crate::option::{OPT_LOCAL, copy_option_part, kOptSpellfile, set_option_value_give_err};
use crate::option_vars::*;
use crate::os::env::home_replace;
use crate::os::fs::{MAXPATHL, os_fopen, os_isdir, os_mkdir, os_mkdir_recurse, os_path_exists};
use crate::os::input::{fast_breakcheck, line_breakcheck, veryfast_breakcheck};
use crate::os::stdpaths::{XDGVarType, get_xdg_home};
use crate::os::time::{Timestamp, os_time, time_t};
use crate::path::{
    dir_of_file_exists, kPathCmpExpand, kPathCmpLiteral, path_equal, path_tail,
    path_tail_with_sep, vim_ispathsep,
};
use crate::regexp::{
    RE_MAGIC, RE_STRICT, RE_STRING, regprog_T, vim_regcomp, vim_regexec_prog, vim_regfree,
};
use crate::runtime::{ETYPE_SPELL, estack_pop, estack_push};
use crate::spell::{
    MAXWLEN, SP_FORMERROR, WF_BANNED, WF_COMPROOT, WF_FIXCAP, WF_HAS_AFF, WF_KEEPCAP, WF_NEEDCOMP,
    WF_NOCOMPAFT, WF_NOCOMPBEF, WF_RARE, WF_REGION, WFP_NC, byte_in_str, captype,
    clear_spell_chartab, close_spellbuf, count_common_word, did_set_spelltab, e_format, first_lang,
    init_spell_chartab, init_syl_tab, int_wordlist, onecap_copy, open_spellbuf, parse_spelllang,
    slang_alloc, slang_clear, slang_clear_sug, spell_casefold, spell_enc, spell_soundfold,
    spell_toupper, spelltab,
};
use crate::spellsuggest::{fromto_T, idx_T, salitem_T, slang_T, spelltab_T};
use crate::strings::{
    atoi, gettext, has_non_ascii, memchr, memcmp, strcat, strcmp, strcpy, strerror, strlen,
    strncmp, strrchr, strstr, vim_snprintf, vim_strchr,
};
use crate::syntax::win_s;
use crate::ui::ui_flush;
use crate::undo::bufIsChanged;
use std::ffi::{CStr, c_char, c_int, c_long, c_uint};
use std::mem::size_of;
use std::ptr;

pub type SpellAddType = c_int;
use crate::change::TAB;
use crate::globals::IOSIZE;
use crate::math::{FAIL, OK};
use crate::viml::parser::parser::CONV_NONE;

pub const SPELL_ADD_GOOD: SpellAddType = 0;
pub const SPELL_ADD_BAD: SpellAddType = 1;
pub const SPELL_ADD_RARE: SpellAddType = 2;

const CMD_spellrare: c_int = 431;
const CMD_spellundo: c_int = 432;
const CMD_spellwrong: c_int = 433;

pub(crate) const MAXREGIONS: usize = 8;

const SPL_FNAME_TMPL: *const c_char = c"%s.%s.spl".as_ptr();
const SPL_FNAME_ADD: *const c_char = c".add.".as_ptr();
const SPL_FNAME_ASCII: *const c_char = c".ascii.".as_ptr();

const VIMSUGMAGIC: &[u8; 6] = b"VIMsug";
const VIMSUGMAGICL: usize = 6;
const VIMSUGVERSION: c_int = 1;

const WF_AFX: c_int = 0x20;

pub(crate) const WF_NOSUGGEST: c_int = 0x0400;

const WFP_UP: c_int = 0x04;
const WFP_COMPPERMIT: c_int = 0x08;
const WFP_COMPFORBID: c_int = 0x10;

const COMP_CHECKDUP: c_int = 1;
const COMP_CHECKREP: c_int = 2;
const COMP_CHECKCASE: c_int = 4;
const COMP_CHECKTRIPLE: c_int = 8;

const SP_TRUNCERROR: c_int = -1;
const SP_OTHERERROR: c_int = -3;

const BY_NOFLAGS: c_int = 0;
const BY_INDEX: c_int = 1;
const BY_FLAGS: c_int = 2;
const BY_FLAGS2: c_int = 3;
const BY_SPECIAL: c_int = BY_FLAGS2;

const ZERO_FLAG: c_uint = 65009;

const SAL_F0LLOWUP: c_int = 1;
const SAL_COLLAPSE: c_int = 2;
const SAL_REM_ACCENTS: c_int = 4;

const VIMSPELLMAGIC: &[u8; 8] = b"VIMspell";
const VIMSPELLMAGICL: usize = 8;
const VIMSPELLVERSION: c_int = 50;

const SN_REGION: c_int = 0;
const SN_CHARFLAGS: c_int = 1;
const SN_MIDWORD: c_int = 2;
const SN_PREFCOND: c_int = 3;
const SN_REP: c_int = 4;
const SN_SAL: c_int = 5;
const SN_SOFO: c_int = 6;
const SN_MAP: c_int = 7;
const SN_COMPOUND: c_int = 8;
const SN_SYLLABLE: c_int = 9;
const SN_NOBREAK: c_int = 10;
const SN_SUGFILE: c_int = 11;
const SN_REPSAL: c_int = 12;
const SN_WORDS: c_int = 13;
const SN_NOSPLITSUGS: c_int = 14;
const SN_INFO: c_int = 15;
const SN_NOCOMPOUNDSUGS: c_int = 16;
const SN_END: c_int = 255;

const SNF_REQUIRED: c_int = 1;

const CF_WORD: u8 = 0x01;
const CF_UPPER: u8 = 0x02;

const COMPOUND_MAX_LEN: usize = 100000;

const MAXLINELEN: usize = 500;
const MAXITEMCNT: usize = 30;

const AFT_CHAR: c_int = 0;
const AFT_LONG: c_int = 1;
const AFT_CAPLONG: c_int = 2;
const AFT_NUM: c_int = 3;

const AH_KEY_LEN: usize = 17;
const SBLOCKSIZE: usize = 16000;
const WN_MASK: c_int = 0xffff;
const SHARED_MASK: idx_T = 0x0800_0000;

const PFX_FLAGS: c_int = -256;

const CONDIT_COMB: c_int = 1;
const CONDIT_CFIX: c_int = 2;
const CONDIT_SUF: c_int = 4;
const CONDIT_AFF: c_int = 8;

const e_spell_trunc: *const c_char = c"E758: Truncated spell file".as_ptr();
const e_error_while_reading_sug_file_str: *const c_char =
    c"E782: Error while reading .sug file: %s".as_ptr();
const e_duplicate_char_in_map_entry: *const c_char = c"E783: Duplicate char in MAP entry".as_ptr();
const e_illegal_character_in_word: *const c_char = c"E1280: Illegal character in word".as_ptr();
const e_afftrailing: *const c_char = c"Trailing text in %s line %d: %s".as_ptr();
const e_affname: *const c_char = c"Affix name too long in %s line %d: %s".as_ptr();
const msg_compressing: *const c_char = c"Compressing word tree...".as_ptr();

static mut compress_start: c_int = 30000;
static mut compress_inc: c_int = 100;
static mut compress_added: c_int = 500000;

#[repr(C)]
struct afffile_T {
    af_enc: *mut c_char,
    af_flagtype: c_int,
    af_rare: c_uint,
    af_keepcase: c_uint,
    af_bad: c_uint,
    af_needaffix: c_uint,
    af_circumfix: c_uint,
    af_needcomp: c_uint,
    af_comproot: c_uint,
    af_compforbid: c_uint,
    af_comppermit: c_uint,
    af_nosuggest: c_uint,
    af_pfxpostpone: c_int,
    af_ignoreextra: bool,
    af_pref: std::collections::BTreeMap<Vec<u8>, affheader_T>,
    af_suff: std::collections::BTreeMap<Vec<u8>, affheader_T>,
    af_comp: std::collections::BTreeMap<Vec<u8>, compitem_T>,
}

impl Drop for afffile_T {
    fn drop(&mut self) {
        unsafe {
            xfree(self.af_enc.cast());
        }
    }
}

#[repr(C)]
struct affentry_T {
    ae_chop: *mut c_char,
    ae_add: *mut c_char,
    ae_flags: *mut c_char,
    ae_cond: *mut c_char,
    ae_prog: *mut regprog_T,
    ae_compforbid: c_char,
    ae_comppermit: c_char,
}

impl Drop for affentry_T {
    fn drop(&mut self) {
        unsafe {
            vim_regfree(self.ae_prog);
        }
    }
}

#[repr(C)]
struct affheader_T {
    ah_key: [c_char; AH_KEY_LEN],
    ah_flag: c_uint,
    ah_newID: c_int,
    ah_combine: c_int,
    ah_follows: c_int,
    ah_entries: Vec<affentry_T>,
}

#[repr(C)]
struct compitem_T {
    ci_key: [c_char; AH_KEY_LEN],
    ci_flag: c_uint,
    ci_newID: c_int,
}

#[repr(C)]
struct sblock_T {
    sb_used: usize,
    sb_data: [c_char; SBLOCKSIZE + 1],
}

impl sblock_T {
    const fn new() -> Self {
        Self {
            sb_used: 0,
            sb_data: [0; SBLOCKSIZE + 1],
        }
    }
}

#[repr(C)]
struct wordnode_T {
    wn_index: c_int,
    wn_wnode: *mut wordnode_T,
    wn_child: *mut wordnode_T,
    wn_sibling: *mut wordnode_T,
    wn_refs: c_int,
    wn_byte: u8,
    wn_affixID: u8,
    wn_flags: u16,
    wn_region: i16,
}

impl wordnode_T {
    const fn new() -> Self {
        Self {
            wn_index: 0,
            wn_wnode: ptr::null_mut(),
            wn_child: ptr::null_mut(),
            wn_sibling: ptr::null_mut(),
            wn_refs: 0,
            wn_byte: 0,
            wn_affixID: 0,
            wn_flags: 0,
            wn_region: 0,
        }
    }
}

#[repr(C)]
struct spellinfo_T {
    si_foldroot: *mut wordnode_T,
    si_foldwcount: c_int,
    si_keeproot: *mut wordnode_T,
    si_keepwcount: c_int,
    si_prefroot: *mut wordnode_T,
    si_sugtree: c_int,
    si_blocks: Vec<Box<sblock_T>>,
    si_blocks_cnt: c_int,
    si_did_emsg: c_int,
    si_compress_cnt: c_int,
    si_first_free: *mut wordnode_T,
    si_free_count: c_int,
    si_spellbuf: Option<Box<buf_T>>,
    si_ascii: c_int,
    si_add: c_int,
    si_clear_chartab: c_int,
    si_region: c_int,
    si_conv: vimconv_T,
    si_memtot: c_int,
    si_verbose: c_int,
    si_msg_count: c_int,
    si_info: *mut c_char,
    si_region_count: c_int,
    si_region_name: [c_char; MAXREGIONS * 2 + 1],
    si_rep: Vec<fromto_T>,
    si_repsal: Vec<fromto_T>,
    si_sal: Vec<fromto_T>,
    si_sofofr: *mut c_char,
    si_sofoto: *mut c_char,
    si_nosugfile: c_int,
    si_nosplitsugs: c_int,
    si_nocompoundsugs: c_int,
    si_followup: c_int,
    si_collapse: c_int,
    si_commonwords: std::collections::BTreeSet<Vec<u8>>,
    si_sugtime: time_t,
    si_rem_accents: c_int,
    si_map: Vec<c_char>,
    si_midword: *mut c_char,
    si_compmax: c_int,
    si_compminlen: c_int,
    si_compsylmax: c_int,
    si_compoptions: c_int,
    si_comppat: Vec<*mut c_char>,
    si_compflags: *mut c_char,
    si_nobreak: c_char,
    si_syllable: *mut c_char,
    si_prefcond: Vec<*mut c_char>,
    si_newprefID: c_int,
    si_newcompID: c_int,
}

impl Default for spellinfo_T {
    fn default() -> Self {
        Self {
            si_foldroot: ptr::null_mut(),
            si_foldwcount: 0,
            si_keeproot: ptr::null_mut(),
            si_keepwcount: 0,
            si_prefroot: ptr::null_mut(),
            si_sugtree: 0,
            si_blocks: Vec::new(),
            si_blocks_cnt: 0,
            si_did_emsg: 0,
            si_compress_cnt: 0,
            si_first_free: ptr::null_mut(),
            si_free_count: 0,
            si_spellbuf: None,
            si_ascii: 0,
            si_add: 0,
            si_clear_chartab: 0,
            si_region: 0,
            si_conv: vimconv_T::new(),
            si_memtot: 0,
            si_verbose: 0,
            si_msg_count: 0,
            si_info: ptr::null_mut(),
            si_region_count: 0,
            si_region_name: [0; MAXREGIONS * 2 + 1],
            si_rep: Vec::new(),
            si_repsal: Vec::new(),
            si_sal: Vec::new(),
            si_sofofr: ptr::null_mut(),
            si_sofoto: ptr::null_mut(),
            si_nosugfile: 0,
            si_nosplitsugs: 0,
            si_nocompoundsugs: 0,
            si_followup: 0,
            si_collapse: 0,
            si_commonwords: std::collections::BTreeSet::new(),
            si_sugtime: 0,
            si_rem_accents: 0,
            si_map: Vec::new(),
            si_midword: ptr::null_mut(),
            si_compmax: 0,
            si_compminlen: 0,
            si_compsylmax: 0,
            si_compoptions: 0,
            si_comppat: Vec::new(),
            si_compflags: ptr::null_mut(),
            si_nobreak: 0,
            si_syllable: ptr::null_mut(),
            si_prefcond: Vec::new(),
            si_newprefID: 0,
            si_newcompID: 0,
        }
    }
}

use crate::fileio::ungetc;

#[inline]
unsafe fn spell_read_bytes(buf: *mut c_char, n: usize, fd: *mut FILE) -> c_int {
    unsafe {
        let read_bytes = fread(buf.cast(), 1, n, fd);
        if read_bytes != n {
            if feof(fd) != 0 {
                SP_TRUNCERROR
            } else {
                SP_OTHERERROR
            }
        } else {
            0
        }
    }
}

#[inline]
unsafe fn spell_read_nonnul_bytes(buf: *mut c_char, n: usize, fd: *mut FILE) -> c_int {
    unsafe {
        let ret = spell_read_bytes(buf, n, fd);
        if ret != 0 {
            return ret;
        }
        if !memchr(buf.cast(), 0, n).is_null() {
            return SP_FORMERROR;
        }
        0
    }
}

#[inline]
unsafe fn spell_check_magic_string(fd: *mut FILE) -> c_int {
    unsafe {
        let mut buf = [0 as c_char; VIMSPELLMAGICL];
        let ret = spell_read_bytes(buf.as_mut_ptr(), VIMSPELLMAGICL, fd);
        if ret != 0 {
            return ret;
        }
        if memcmp(
            buf.as_ptr().cast(),
            VIMSPELLMAGIC.as_ptr().cast(),
            VIMSPELLMAGICL,
        ) != 0
        {
            return SP_FORMERROR;
        }
        0
    }
}

/// Load one spell file and store the info into a slang_T.
pub unsafe fn spell_load_file(
    fname: *mut c_char,
    lang: *mut c_char,
    old_lp: *mut slang_T,
    silent: bool,
) -> *mut slang_T {
    unsafe {
        let fd = os_fopen(fname, c"r".as_ptr());
        if fd.is_null() {
            if !silent {
                semsg!(gettext((&raw const e_notopen).cast()), fname);
            } else if p_verbose > 2 {
                verbose_enter();
                smsg!(0, (&raw const e_notopen).cast(), fname);
                verbose_leave();
            }
            if !lang.is_null() {
                *lang = NUL;
            }
            return ptr::null_mut();
        }
        if p_verbose > 2 {
            verbose_enter();
            smsg!(0, gettext(c"Reading spell file \"%s\"".as_ptr()), fname);
            verbose_leave();
        }

        let mut owned_lp = if old_lp.is_null() {
            let mut new_lp = slang_alloc(lang);
            new_lp.sl_fname = xstrdup(fname);
            new_lp.sl_add = !strstr(path_tail(fname), SPL_FNAME_ADD).is_null();
            Some(new_lp)
        } else {
            None
        };
        let mut lp: *mut slang_T = owned_lp.as_deref_mut().map_or(old_lp, |b| &raw mut *b);

        estack_push(ETYPE_SPELL, fname, 0);
        let did_estack_push = true;

        let mut fail = false;
        let scms_ret = spell_check_magic_string(fd);
        match scms_ret {
            SP_FORMERROR | SP_TRUNCERROR => {
                semsg!(
                    c"%s".as_ptr(),
                    gettext(c"E757: This does not look like a spell file".as_ptr()),
                );
                fail = true;
            }
            SP_OTHERERROR => {
                semsg!(
                    gettext(c"E5042: Failed to read spell file %s: %s".as_ptr()),
                    fname,
                    strerror(ferror(fd)),
                );
                fail = true;
            }
            _ => {}
        }

        if !fail {
            let c = fgetc(fd);
            if c < VIMSPELLVERSION {
                emsg(gettext(
                    c"E771: Old spell file, needs to be updated".as_ptr(),
                ));
                fail = true;
            } else if c > VIMSPELLVERSION {
                emsg(gettext(
                    c"E772: Spell file is for newer version of Vim".as_ptr(),
                ));
                fail = true;
            }
        }

        let mut res: c_int = 0;
        let mut check_someerror = false;
        while !fail {
            let n = fgetc(fd);
            if n == SN_END {
                break;
            }
            let c = fgetc(fd);
            let mut len = get4c(fd);
            if len < 0 {
                res = SP_TRUNCERROR;
                check_someerror = true;
                break;
            }

            res = 0;
            match n {
                SN_INFO => {
                    xfree((*lp).sl_info.cast());
                    (*lp).sl_info = read_string(fd, len as usize);
                    if (*lp).sl_info.is_null() {
                        fail = true;
                        break;
                    }
                }
                SN_REGION => {
                    res = read_region_section(fd, lp, len);
                }
                SN_CHARFLAGS => {
                    res = read_charflags_section(fd);
                }
                SN_MIDWORD => {
                    xfree((*lp).sl_midword.cast());
                    (*lp).sl_midword = read_string(fd, len as usize);
                    if (*lp).sl_midword.is_null() {
                        fail = true;
                        break;
                    }
                }
                SN_PREFCOND => {
                    res = read_prefcond_section(fd, lp);
                }
                SN_REP => {
                    res = read_rep_section(
                        fd,
                        &mut (*lp).sl_rep,
                        (&raw mut (*lp).sl_rep_first).cast::<i16>(),
                    );
                }
                SN_REPSAL => {
                    res = read_rep_section(
                        fd,
                        &mut (*lp).sl_repsal,
                        (&raw mut (*lp).sl_repsal_first).cast::<i16>(),
                    );
                }
                SN_SAL => {
                    res = read_sal_section(fd, lp);
                }
                SN_SOFO => {
                    res = read_sofo_section(fd, lp);
                }
                SN_MAP => {
                    let p = read_string(fd, len as usize);
                    if p.is_null() {
                        fail = true;
                        break;
                    }
                    set_map_str(lp, p);
                    xfree(p.cast());
                }
                SN_WORDS => {
                    res = read_words_section(fd, lp, len);
                }
                SN_SUGFILE => {
                    (*lp).sl_sugtime = get8ctime(fd);
                }
                SN_NOSPLITSUGS => {
                    (*lp).sl_nosplitsugs = true;
                }
                SN_NOCOMPOUNDSUGS => {
                    (*lp).sl_nocompoundsugs = true;
                }
                SN_COMPOUND => {
                    res = read_compound(fd, lp, len);
                }
                SN_NOBREAK => {
                    (*lp).sl_nobreak = true;
                }
                SN_SYLLABLE => {
                    xfree((*lp).sl_syllable.cast());
                    (*lp).sl_syllable = read_string(fd, len as usize);
                    if (*lp).sl_syllable.is_null() {
                        fail = true;
                        break;
                    }
                    if init_syl_tab(lp) != OK {
                        fail = true;
                        break;
                    }
                }
                _ => {
                    if (c & SNF_REQUIRED) != 0 {
                        emsg(gettext(c"E770: Unsupported section in spell file".as_ptr()));
                        fail = true;
                        break;
                    }
                    while len > 0 {
                        len -= 1;
                        if fgetc(fd) < 0 {
                            res = SP_TRUNCERROR;
                            break;
                        }
                    }
                }
            }
            if res != 0 {
                check_someerror = true;
                break;
            }
        }

        if !fail && !check_someerror {
            res = spell_read_tree(
                fd,
                &mut (*lp).sl_fbyts,
                &mut (*lp).sl_fidxs,
                false,
                0,
            );
            if res != 0 {
                check_someerror = true;
            }
        }

        if !fail && !check_someerror {
            res = spell_read_tree(
                fd,
                &mut (*lp).sl_kbyts,
                &mut (*lp).sl_kidxs,
                false,
                0,
            );
            if res != 0 {
                check_someerror = true;
            }
        }

        if !fail && !check_someerror {
            res = spell_read_tree(
                fd,
                &mut (*lp).sl_pbyts,
                &mut (*lp).sl_pidxs,
                true,
                (*lp).sl_prefprog.len() as c_int,
            );
            if res != 0 {
                check_someerror = true;
            }
        }

        if check_someerror {
            if res == SP_FORMERROR {
                emsg(gettext(e_format));
                fail = true;
            } else if res == SP_TRUNCERROR {
                emsg(gettext(e_spell_trunc));
                fail = true;
            } else if res == SP_OTHERERROR {
                fail = true;
            }
        }

        if !fail {
            if let Some(boxed_lp) = owned_lp.take()
                && !lang.is_null()
            {
                let langs = &raw mut first_lang;
                (*langs).push(std::pin::Pin::new(boxed_lp));
                lp = &raw mut **(*langs).last_mut().unwrap();
            }
        } else {
            if !lang.is_null() {
                *lang = NUL;
            }
            drop(owned_lp);
            lp = ptr::null_mut();
        }

        if !fd.is_null() {
            fclose(fd);
        }
        if did_estack_push {
            estack_pop();
        }

        lp
    }
}

unsafe fn tree_count_words(byts: *const u8, byts_len: c_int, idxs: *mut idx_T) {
    unsafe {
        let mut arridx = [0 as idx_T; MAXWLEN];
        let mut curi = [0 as c_int; MAXWLEN];
        let mut wordcount = [0 as c_int; MAXWLEN];

        arridx[0] = 0;
        curi[0] = 1;
        wordcount[0] = 0;
        let mut depth: isize = 0;
        while depth >= 0 && !got_int {
            let d = depth as usize;
            if curi[d] > *byts.offset(arridx[d] as isize) as c_int {
                *idxs.offset(arridx[d] as isize) = wordcount[d];
                if depth > 0 {
                    wordcount[d - 1] += wordcount[d];
                }
                depth -= 1;
                fast_breakcheck();
            } else {
                let mut n = arridx[d] + curi[d];
                curi[d] += 1;

                let c = *byts.offset(n as isize) as c_int;
                if c == 0 {
                    wordcount[d] += 1;
                    while n + 1 < byts_len && *byts.offset((n + 1) as isize) == 0 {
                        n += 1;
                        curi[d] += 1;
                    }
                } else if d < MAXWLEN - 1 {
                    depth += 1;
                    let nd = depth as usize;
                    arridx[nd] = *idxs.offset(n as isize);
                    curi[nd] = 1;
                    wordcount[nd] = 0;
                }
            }
        }
    }
}

/// Load the .sug files for languages that have one and weren't loaded yet.
pub unsafe fn suggest_load_files() {
    unsafe {
        let mut buf = [0 as c_char; MAXWLEN];
        let mut ga: Vec<u8> = Vec::new();

        let mut lpi: usize = 0;
        while lpi < (*win_s(curwin)).b_langp.len() {
            let lp = &(&(*win_s(curwin)).b_langp)[lpi];
            let slang = lp.lp_slang;
            if (*slang).sl_sugtime != 0 && !(*slang).sl_sugloaded {
                (*slang).sl_sugloaded = true;

                let dotp = strrchr((*slang).sl_fname, b'.' as c_int);
                if dotp.is_null() || !path_equal(dotp, c".spl".as_ptr(), kPathCmpLiteral) {
                    lpi += 1;
                    continue;
                }
                strcpy(dotp, c".sug".as_ptr());
                let fd = os_fopen((*slang).sl_fname, c"r".as_ptr());
                if !fd.is_null() {
                    let mut ok = true;
                    for slot in buf.iter_mut().take(VIMSUGMAGICL) {
                        *slot = fgetc(fd) as c_char;
                    }
                    if strncmp(buf.as_ptr(), VIMSUGMAGIC.as_ptr().cast(), VIMSUGMAGICL) != 0 {
                        semsg!(
                            gettext(c"E778: This does not look like a .sug file: %s".as_ptr()),
                            (*slang).sl_fname,
                        );
                        ok = false;
                    }
                    if ok {
                        let c = fgetc(fd);
                        if c < VIMSUGVERSION {
                            semsg!(
                                gettext(c"E779: Old .sug file, needs to be updated: %s".as_ptr()),
                                (*slang).sl_fname,
                            );
                            ok = false;
                        } else if c > VIMSUGVERSION {
                            semsg!(
                                gettext(
                                    c"E780: .sug file is for newer version of Vim: %s".as_ptr(),
                                ),
                                (*slang).sl_fname,
                            );
                            ok = false;
                        }
                    }

                    if ok {
                        let timestamp = get8ctime(fd);
                        if timestamp != (*slang).sl_sugtime {
                            semsg!(
                                gettext(c"E781: .sug file doesn't match .spl file: %s".as_ptr()),
                                (*slang).sl_fname,
                            );
                            ok = false;
                        }
                    }

                    if ok {
                        let mut someerror = false;
                        if spell_read_tree(
                            fd,
                            &mut (*slang).sl_sbyts,
                            &mut (*slang).sl_sidxs,
                            false,
                            0,
                        ) != 0
                        {
                            someerror = true;
                        }

                        if !someerror {
                            let mut sugbuf = open_spellbuf();
                            let sugbuf_ptr: *mut buf_T = &raw mut *sugbuf;
                            (*slang).sl_sugbuf = Some(sugbuf);
                            let wcount = get4c(fd);
                            if wcount < 0 {
                                someerror = true;
                            } else {
                                ga.reserve(100);
                                for wordnr in 0..wcount {
                                    ga.clear();
                                    loop {
                                        let c = fgetc(fd);
                                        if c < 0 {
                                            someerror = true;
                                            break;
                                        }
                                        ga.push(c as u8);
                                        if c == NUL as c_int {
                                            break;
                                        }
                                    }
                                    if someerror {
                                        break;
                                    }
                                    if ml_append_buf(
                                        sugbuf_ptr,
                                        wordnr as linenr_T,
                                        ga.as_mut_ptr().cast(),
                                        ga.len() as c_int,
                                        true,
                                    ) == FAIL
                                    {
                                        someerror = true;
                                        break;
                                    }
                                }
                                ga.clear();
                            }
                        }

                        if someerror {
                            semsg!(
                                gettext(e_error_while_reading_sug_file_str),
                                (*slang).sl_fname,
                            );
                            slang_clear_sug(slang);
                        } else {
                            tree_count_words(
                                (*slang).sl_fbyts.as_mut_ptr(),
                                (*slang).sl_fbyts.len() as c_int,
                                (*slang).sl_fidxs.as_mut_ptr(),
                            );
                            tree_count_words(
                                (*slang).sl_sbyts.as_mut_ptr(),
                                (*slang).sl_sbyts.len() as c_int,
                                (*slang).sl_sidxs.as_mut_ptr(),
                            );
                        }
                    }
                    fclose(fd);
                }
                strcpy(dotp, c".spl".as_ptr());
            }
            lpi += 1;
        }
    }
}

unsafe fn read_cnt_string(fd: *mut FILE, cnt_bytes: c_int, cntp: *mut c_int) -> *mut c_char {
    unsafe {
        let mut cnt: c_int = 0;
        for _ in 0..cnt_bytes {
            let c = fgetc(fd);
            if c == EOF {
                *cntp = SP_TRUNCERROR;
                return ptr::null_mut();
            }
            cnt = ((cnt as c_uint) << 8).wrapping_add(c as c_uint) as c_int;
        }
        *cntp = cnt;
        if cnt == 0 {
            return ptr::null_mut();
        }
        let str_ = read_string(fd, cnt as usize);
        if str_.is_null() {
            *cntp = SP_OTHERERROR;
        }
        str_
    }
}

unsafe fn read_region_section(fd: *mut FILE, lp: *mut slang_T, len: c_int) -> c_int {
    unsafe {
        if len > (MAXREGIONS * 2) as c_int {
            return SP_FORMERROR;
        }
        let regions_ptr = (&raw mut (*lp).sl_regions).cast::<c_char>();
        let ret = spell_read_nonnul_bytes(regions_ptr, len as usize, fd);
        if ret != 0 {
            return ret;
        }
        *regions_ptr.offset(len as isize) = NUL;
        0
    }
}

unsafe fn read_charflags_section(fd: *mut FILE) -> c_int {
    unsafe {
        let mut flagslen: c_int = 0;
        let mut follen: c_int = 0;

        let flags = read_cnt_string(fd, 1, &raw mut flagslen);
        if flagslen < 0 {
            return flagslen;
        }

        let fol = read_cnt_string(fd, 2, &raw mut follen);
        if follen < 0 {
            xfree(flags.cast());
            return follen;
        }

        if !flags.is_null() && !fol.is_null() {
            set_spell_charflags(flags, flagslen, fol);
        }

        xfree(flags.cast());
        xfree(fol.cast());

        if flags.is_null() != fol.is_null() {
            return SP_FORMERROR;
        }
        0
    }
}

unsafe fn read_prefcond_section(fd: *mut FILE, lp: *mut slang_T) -> c_int {
    unsafe {
        let cnt = get2c(fd);
        if cnt <= 0 {
            return SP_FORMERROR;
        }

        (*lp).sl_prefprog.resize(cnt as usize, ptr::null_mut());

        for i in 0..cnt {
            let n = fgetc(fd);
            if n < 0 || n >= MAXWLEN as c_int {
                return SP_FORMERROR;
            }

            if n > 0 {
                let mut buf = [0 as c_char; MAXWLEN + 1];
                buf[0] = b'^' as c_char;
                let ret = spell_read_nonnul_bytes(buf.as_mut_ptr().add(1), n as usize, fd);
                if ret != 0 {
                    return ret;
                }
                buf[(n + 1) as usize] = NUL;
                (*lp).sl_prefprog.as_mut_slice()[i as usize] =
                    vim_regcomp(buf.as_ptr(), RE_MAGIC | RE_STRING);
            }
        }
        0
    }
}

unsafe fn read_rep_section(fd: *mut FILE, gap: &mut Vec<fromto_T>, first: *mut i16) -> c_int {
    unsafe {
        let cnt = get2c(fd);
        if cnt < 0 {
            return SP_TRUNCERROR;
        }

        gap.reserve(cnt as usize);

        while (gap.len() as c_int) < cnt {
            let mut c: c_int = 0;
            let ft_from = read_cnt_string(fd, 1, &raw mut c);
            if c < 0 {
                return c;
            }
            if c == 0 {
                return SP_FORMERROR;
            }
            let ft_to = read_cnt_string(fd, 1, &raw mut c);
            if c <= 0 {
                xfree(ft_from.cast());
                if c < 0 {
                    return c;
                }
                return SP_FORMERROR;
            }
            gap.push(fromto_T { ft_from, ft_to });
        }

        for i in 0..256 {
            *first.add(i) = -1;
        }
        for (i, ftp) in gap.iter().enumerate() {
            let b = *ftp.ft_from as u8 as usize;
            if *first.add(b) == -1 {
                *first.add(b) = i as i16;
            }
        }
        0
    }
}

unsafe fn read_sal_section(fd: *mut FILE, slang: *mut slang_T) -> c_int {
    unsafe {
        (*slang).sl_sofo = false;

        let flags = fgetc(fd);
        if (flags & SAL_F0LLOWUP) != 0 {
            (*slang).sl_followup = true;
        }
        if (flags & SAL_COLLAPSE) != 0 {
            (*slang).sl_collapse = true;
        }
        if (flags & SAL_REM_ACCENTS) != 0 {
            (*slang).sl_rem_accents = true;
        }

        let cnt = get2c(fd);
        if cnt < 0 {
            return SP_TRUNCERROR;
        }

        let gap = &mut (*slang).sl_sal;
        gap.clear();
        gap.reserve((cnt + 1) as usize);

        while (gap.len() as c_int) < cnt {
            let mut c: c_int = NUL as c_int;
            let mut smp = salitem_T {
                sm_lead: ptr::null_mut(),
                sm_leadlen: 0,
                sm_oneof: ptr::null_mut(),
                sm_rules: ptr::null_mut(),
                sm_to: ptr::null_mut(),
                sm_lead_w: Vec::new(),
                sm_oneof_w: Vec::new(),
                sm_to_w: Vec::new(),
            };
            let mut ccnt = fgetc(fd);
            if ccnt < 0 {
                return SP_TRUNCERROR;
            }
            let mut p = xmalloc(ccnt as usize + 2).cast::<c_char>();
            smp.sm_lead = p;

            let mut i: c_int = 0;
            while i < ccnt {
                c = fgetc(fd);
                if !vim_strchr(c"0123456789(-<^$".as_ptr(), c).is_null() {
                    break;
                }
                *p = c as u8 as c_char;
                p = p.add(1);
                i += 1;
            }
            smp.sm_leadlen = p.offset_from(smp.sm_lead) as c_int;
            *p = NUL;
            p = p.add(1);

            if c == b'(' as c_int {
                smp.sm_oneof = p;
                i += 1;
                while i < ccnt {
                    c = fgetc(fd);
                    if c == b')' as c_int {
                        break;
                    }
                    *p = c as u8 as c_char;
                    p = p.add(1);
                    i += 1;
                }
                *p = NUL;
                p = p.add(1);
                i += 1;
                if i < ccnt {
                    c = fgetc(fd);
                }
            } else {
                smp.sm_oneof = ptr::null_mut();
            }

            smp.sm_rules = p;
            if i < ccnt {
                *p = c as u8 as c_char;
                p = p.add(1);
            }
            i += 1;
            if i < ccnt {
                let rem = (ccnt - i) as usize;
                let ret = spell_read_nonnul_bytes(p, rem, fd);
                if ret != 0 {
                    return ret;
                }
                p = p.add(rem);
            }
            *p = NUL;

            smp.sm_to = read_cnt_string(fd, 1, &raw mut ccnt);
            if ccnt < 0 {
                return ccnt;
            }

            smp.sm_lead_w = mb_str2wide(smp.sm_lead);
            smp.sm_leadlen = mb_charlen(smp.sm_lead);
            if !smp.sm_oneof.is_null() {
                smp.sm_oneof_w = mb_str2wide(smp.sm_oneof);
            }
            if !smp.sm_to.is_null() {
                smp.sm_to_w = mb_str2wide(smp.sm_to);
            }
            gap.push(smp);
        }

        if !gap.is_empty() {
            let p = xmalloc(1).cast::<c_char>();
            *p = NUL;
            gap.push(salitem_T {
                sm_lead: p,
                sm_lead_w: mb_str2wide(p),
                sm_leadlen: 0,
                sm_oneof: ptr::null_mut(),
                sm_oneof_w: Vec::new(),
                sm_rules: p,
                sm_to: ptr::null_mut(),
                sm_to_w: Vec::new(),
            });
        }

        set_sal_first(slang);
        0
    }
}

unsafe fn read_words_section(fd: *mut FILE, lp: *mut slang_T, len: c_int) -> c_int {
    unsafe {
        let mut done: c_int = 0;
        let mut word = [0u8; MAXWLEN];

        while done < len {
            let mut i: usize = 0;
            loop {
                let c = fgetc(fd);
                if c == EOF {
                    return SP_TRUNCERROR;
                }
                word[i] = c as u8;
                if word[i] == 0 {
                    break;
                }
                if i == MAXWLEN - 1 {
                    return SP_FORMERROR;
                }
                i += 1;
            }

            count_common_word(lp, word.as_mut_ptr().cast(), -1, 10);
            done += (i + 1) as c_int;
        }
        0
    }
}

unsafe fn read_sofo_section(fd: *mut FILE, slang: *mut slang_T) -> c_int {
    unsafe {
        let mut cnt: c_int = 0;
        let from = read_cnt_string(fd, 2, &raw mut cnt);
        if cnt < 0 {
            return cnt;
        }

        let to = read_cnt_string(fd, 2, &raw mut cnt);
        if cnt < 0 {
            xfree(from.cast());
            return cnt;
        }

        let res = if !from.is_null() && !to.is_null() {
            set_sofo(slang, from, to)
        } else if !from.is_null() || !to.is_null() {
            SP_FORMERROR
        } else {
            0
        };

        xfree(from.cast());
        xfree(to.cast());
        res
    }
}

unsafe fn read_compound(fd: *mut FILE, slang: *mut slang_T, len: c_int) -> c_int {
    unsafe {
        let mut todo = len;
        let mut cnt: c_int = 0;

        if todo < 2 {
            return SP_FORMERROR;
        }
        todo -= 1;
        let mut c = fgetc(fd);
        if c < 2 {
            c = MAXWLEN as c_int;
        }
        (*slang).sl_compmax = c;

        todo -= 1;
        c = fgetc(fd);
        if c < 1 {
            c = 0;
        }
        (*slang).sl_compminlen = c;

        todo -= 1;
        c = fgetc(fd);
        if c < 1 {
            c = MAXWLEN as c_int;
        }
        (*slang).sl_compsylmax = c;

        c = fgetc(fd);
        if c != 0 {
            ungetc(c, fd);
        } else {
            todo -= 1;
            c = fgetc(fd);
            todo -= 1;
            (*slang).sl_compoptions = c;

            let gap = &mut (*slang).sl_comppat;
            c = get2c(fd);
            if c < 0 {
                return SP_TRUNCERROR;
            }
            todo -= 2;
            gap.clear();
            gap.reserve(c as usize);
            while c > 0 {
                c -= 1;
                gap.push(read_cnt_string(fd, 1, &raw mut cnt));
                if cnt < 0 {
                    return cnt;
                }
                todo -= cnt + 1;
            }
        }
        if todo < 0 {
            return SP_FORMERROR;
        }

        if (todo as usize) > COMPOUND_MAX_LEN {
            return SP_FORMERROR;
        }
        let patsize = (todo as usize) * 4 + 7;
        let flagsize = (todo as usize) + 1;
        let mut pat_vec = vec![0 as c_char; patsize];
        let pat = pat_vec.as_mut_ptr();

        let mut cp = xmalloc(flagsize).cast::<u8>();
        (*slang).sl_compstartflags = cp;
        *cp = 0;

        let mut ap = xmalloc(flagsize).cast::<u8>();
        (*slang).sl_compallflags = ap;
        *ap = 0;

        let mut crp = xmalloc(flagsize).cast::<u8>();
        (*slang).sl_comprules = crp;

        let mut pp = pat;
        *pp = b'^' as c_char;
        pp = pp.add(1);
        *pp = b'\\' as c_char;
        pp = pp.add(1);
        *pp = b'(' as c_char;
        pp = pp.add(1);

        let mut atstart: c_int = 1;
        while todo > 0 {
            todo -= 1;
            c = fgetc(fd);
            if c == EOF {
                return SP_TRUNCERROR;
            }

            if vim_strchr(c"?*+[]/".as_ptr(), c).is_null()
                && !byte_in_str((*slang).sl_compallflags, c)
            {
                *ap = c as u8;
                ap = ap.add(1);
                *ap = 0;
            }

            if atstart != 0 {
                if c == b'[' as c_int {
                    atstart = 2;
                } else if c == b']' as c_int {
                    atstart = 0;
                } else {
                    if !byte_in_str((*slang).sl_compstartflags, c) {
                        *cp = c as u8;
                        cp = cp.add(1);
                        *cp = 0;
                    }
                    if atstart == 1 {
                        atstart = 0;
                    }
                }
            }

            if !crp.is_null() {
                if c == b'?' as c_int || c == b'+' as c_int || c == b'*' as c_int {
                    xfree((*slang).sl_comprules.cast());
                    (*slang).sl_comprules = ptr::null_mut();
                    crp = ptr::null_mut();
                } else {
                    *crp = c as u8;
                    crp = crp.add(1);
                }
            }

            if c == b'/' as c_int {
                *pp = b'\\' as c_char;
                pp = pp.add(1);
                *pp = b'|' as c_char;
                pp = pp.add(1);
                atstart = 1;
            } else {
                if c == b'?' as c_int || c == b'+' as c_int || c == b'~' as c_int {
                    *pp = b'\\' as c_char;
                    pp = pp.add(1);
                }
                pp = pp.add(utf_char2bytes(c, pp) as usize);
            }
        }

        *pp = b'\\' as c_char;
        pp = pp.add(1);
        *pp = b')' as c_char;
        pp = pp.add(1);
        *pp = b'$' as c_char;
        pp = pp.add(1);
        *pp = NUL;

        if !crp.is_null() {
            *crp = 0;
        }

        (*slang).sl_compprog = vim_regcomp(pat, RE_MAGIC + RE_STRING + RE_STRICT);
        drop(pat_vec);
        if (*slang).sl_compprog.is_null() {
            return SP_FORMERROR;
        }

        0
    }
}

unsafe fn set_sofo(lp: *mut slang_T, from: *const c_char, to: *const c_char) -> c_int {
    unsafe {
        (*lp).sl_sal = Vec::new();
        (*lp).sl_sofo_sal = Vec::new();

        let gap = &mut (*lp).sl_sofo_sal;
        gap.resize_with(256, Vec::new);
        (*lp).sl_sofo = true;
        (*lp).sl_sal_first = [0; 256];

        let mut p = from;
        let mut s = to;
        while *p != NUL && *s != NUL {
            let c = mb_cptr2char_adv(&raw mut p);
            let i = mb_cptr2char_adv(&raw mut s);
            if c >= 256 {
                gap[(c & 0xff) as usize].push((c, i));
            } else {
                (*lp).sl_sal_first[c as usize] = i;
            }
        }
        if *p != NUL || *s != NUL {
            return SP_FORMERROR;
        }

        0
    }
}

unsafe fn set_sal_first(lp: *mut slang_T) {
    unsafe {
        let gap = &mut (*lp).sl_sal;
        (*lp).sl_sal_first.fill(-1);
        let len = gap.len() as c_int;
        let mut i: c_int = 0;
        while i < len {
            let c = (gap[i as usize].sm_lead_w[0] & 0xff) as usize;
            if (*lp).sl_sal_first[c] == -1 {
                (*lp).sl_sal_first[c] = i;

                while i + 1 < len && (gap[(i + 1) as usize].sm_lead_w[0] & 0xff) as usize == c {
                    i += 1;
                }

                let mut n: c_int = 1;
                while i + n < len {
                    if (gap[(i + n) as usize].sm_lead_w[0] & 0xff) as usize == c {
                        i += 1;
                        n -= 1;
                        gap[i as usize..=(i + n) as usize].rotate_right(1);
                    }
                    n += 1;
                }
            }
            i += 1;
        }
    }
}

unsafe fn mb_str2wide(s: *const c_char) -> Vec<c_int> {
    unsafe {
        let mut res = Vec::with_capacity(mb_charlen(s) as usize + 1);
        let mut p = s;
        while *p != NUL {
            res.push(mb_ptr2char_adv(&raw mut p));
        }
        res.push(0);
        res
    }
}

unsafe fn spell_read_tree(
    fd: *mut FILE,
    bytsp: &mut Vec<u8>,
    idxsp: &mut Vec<idx_T>,
    prefixtree: bool,
    prefixcnt: c_int,
) -> c_int {
    unsafe {
        let len = get4c(fd);
        if len < 0 {
            return SP_TRUNCERROR;
        }
        if (len as usize) > usize::MAX / size_of::<c_int>() {
            return SP_FORMERROR;
        }
        if len <= 0 {
            return 0;
        }

        bytsp.clear();
        bytsp.resize(len as usize, 0);
        idxsp.clear();
        idxsp.resize(len as usize, 0);

        let idx = read_tree_node(
            fd,
            bytsp.as_mut_ptr(),
            idxsp.as_mut_ptr(),
            len,
            0,
            prefixtree,
            prefixcnt,
            0,
        );
        if idx < 0 {
            return idx;
        }
        if idx != len {
            return SP_FORMERROR;
        }
        0
    }
}

unsafe fn read_tree_node(
    fd: *mut FILE,
    byts: *mut u8,
    idxs: *mut idx_T,
    maxidx: c_int,
    startidx: idx_T,
    prefixtree: bool,
    maxprefcondnr: c_int,
    depth: c_int,
) -> idx_T {
    unsafe {
        let mut idx = startidx;

        if depth > MAXWLEN as c_int {
            return SP_FORMERROR;
        }

        let len = fgetc(fd);
        if len <= 0 {
            return SP_TRUNCERROR;
        }

        if len >= maxidx - startidx {
            return SP_FORMERROR;
        }
        *byts.offset(idx as isize) = len as u8;
        idx += 1;

        for _ in 1..=len {
            let mut c = fgetc(fd);
            if c < 0 {
                return SP_TRUNCERROR;
            }
            if c <= BY_SPECIAL {
                if c == BY_NOFLAGS && !prefixtree {
                    *idxs.offset(idx as isize) = 0;
                } else if c != BY_INDEX {
                    if prefixtree {
                        if c == BY_FLAGS {
                            let n = fgetc(fd);
                            if n < 0 {
                                return SP_TRUNCERROR;
                            }
                            c = n << 24;
                        } else {
                            c = 0;
                        }

                        let mut n = fgetc(fd);
                        if n < 0 {
                            return SP_TRUNCERROR;
                        }
                        c |= n;

                        n = get2c(fd);
                        if n < 0 || n >= maxprefcondnr {
                            return SP_FORMERROR;
                        }
                        c |= n << 8;
                    } else {
                        let c2 = c;
                        c = fgetc(fd);
                        if c < 0 {
                            return SP_TRUNCERROR;
                        }
                        if c2 == BY_FLAGS2 {
                            let n = fgetc(fd);
                            if n < 0 {
                                return SP_TRUNCERROR;
                            }
                            c += n << 8;
                        }
                        if (c & WF_REGION) != 0 {
                            let n = fgetc(fd);
                            if n < 0 {
                                return SP_TRUNCERROR;
                            }
                            c += n << 16;
                        }
                        if (c & WF_AFX) != 0 {
                            let n = fgetc(fd);
                            if n < 0 {
                                return SP_TRUNCERROR;
                            }
                            c = (((n as c_uint) << 24) as c_int).wrapping_add(c);
                        }
                    }

                    *idxs.offset(idx as isize) = c;
                    c = 0;
                } else {
                    let n = get3c(fd);
                    if n < 0 || n >= maxidx {
                        return SP_FORMERROR;
                    }
                    *idxs.offset(idx as isize) = n + SHARED_MASK;
                    c = fgetc(fd);
                }
            }
            *byts.offset(idx as isize) = c as u8;
            idx += 1;
        }

        for i in 1..=len {
            let pos = (startidx + i) as isize;
            if *byts.offset(pos) != 0 {
                if (*idxs.offset(pos) & SHARED_MASK) != 0 {
                    *idxs.offset(pos) &= !SHARED_MASK;
                } else {
                    *idxs.offset(pos) = idx;
                    idx = read_tree_node(
                        fd,
                        byts,
                        idxs,
                        maxidx,
                        idx,
                        prefixtree,
                        maxprefcondnr,
                        depth + 1,
                    );
                    if idx < 0 {
                        break;
                    }
                }
            }
        }

        idx
    }
}

unsafe fn spell_reload_one(fname: *mut c_char, added_word: bool) {
    unsafe {
        let mut didit = false;

        let langs = &raw mut first_lang;
        let len = (*langs).len();
        for i in (0..len).rev() {
            let slang: *mut slang_T = &raw mut *(*langs).as_mut_slice()[i];
            if path_equal(fname, (*slang).sl_fname, kPathCmpExpand) {
                slang_clear(slang);
                if spell_load_file(fname, ptr::null_mut(), slang, false).is_null() {
                    slang_clear(slang);
                }
                redraw_all_later(UPD_SOME_VALID);
                didit = true;
            }
        }

        if added_word && !didit {
            parse_spelllang(curwin);
        }
    }
}

/// Check the 'mkspellmem' option. Return FAIL if it's wrong.
pub unsafe fn spell_check_msm(value: *mut c_char, apply: bool) -> c_int {
    unsafe {
        let mut p = value;

        if !ascii_isdigit(*p as c_int) {
            return FAIL;
        }
        let start = (getdigits_int(&raw mut p, true, 0) * 10) / ((SBLOCKSIZE / 102) as c_int);
        if *p != b',' as c_char {
            return FAIL;
        }
        p = p.add(1);
        if !ascii_isdigit(*p as c_int) {
            return FAIL;
        }
        let incr = (getdigits_int(&raw mut p, true, 0) * 102) / ((SBLOCKSIZE / 10) as c_int);
        if *p != b',' as c_char {
            return FAIL;
        }
        p = p.add(1);
        if !ascii_isdigit(*p as c_int) {
            return FAIL;
        }
        let added = getdigits_int(&raw mut p, true, 0) * 1024;
        if *p != NUL {
            return FAIL;
        }

        if start == 0 || incr == 0 || added == 0 || incr > start {
            return FAIL;
        }

        if apply {
            compress_start = start;
            compress_inc = incr;
            compress_added = added;
        }
        OK
    }
}

unsafe fn spell_read_aff(spin: *mut spellinfo_T, fname: *mut c_char) -> Option<afffile_T> {
    unsafe {
        let mut rline = [0 as c_char; MAXLINELEN];
        let mut pc: *mut c_char = ptr::null_mut();
        let mut items = [ptr::null_mut::<c_char>(); MAXITEMCNT];
        let mut lnum: c_int = 0;
        let mut cur_aff: *mut affheader_T = ptr::null_mut();
        let mut did_postpone_prefix = false;
        let mut aff_todo: c_int = 0;
        let mut low: *mut c_char = ptr::null_mut();
        let mut fol: *mut c_char = ptr::null_mut();
        let mut upp: *mut c_char = ptr::null_mut();
        let mut found_map = false;
        let mut compminlen: c_int = 0;
        let mut compsylmax: c_int = 0;
        let mut compoptions: c_int = 0;
        let mut compmax: c_int = 0;
        let mut compflags: *mut c_char = ptr::null_mut();
        let mut midword: *mut c_char = ptr::null_mut();
        let mut syllable: *mut c_char = ptr::null_mut();
        let mut sofofrom: *mut c_char = ptr::null_mut();
        let mut sofoto: *mut c_char = ptr::null_mut();

        let fd = os_fopen(fname, c"r".as_ptr());
        if fd.is_null() {
            semsg!(gettext((&raw const e_notopen).cast()), fname);
            return None;
        }

        vim_snprintf!(
            (&raw mut IObuff).cast(),
            IOSIZE,
            gettext(c"Reading affix file %s...".as_ptr()),
            fname,
        );
        spell_message(spin, (&raw mut IObuff).cast());

        let do_rep = (*spin).si_rep.is_empty();
        let do_repsal = (*spin).si_repsal.is_empty();
        let do_sal = (*spin).si_sal.is_empty();
        let do_mapline = (*spin).si_map.is_empty();

        let mut aff = afffile_T {
            af_enc: ptr::null_mut(),
            af_flagtype: 0,
            af_rare: 0,
            af_keepcase: 0,
            af_bad: 0,
            af_needaffix: 0,
            af_circumfix: 0,
            af_needcomp: 0,
            af_comproot: 0,
            af_compforbid: 0,
            af_comppermit: 0,
            af_nosuggest: 0,
            af_pfxpostpone: 0,
            af_ignoreextra: false,
            af_pref: std::collections::BTreeMap::new(),
            af_suff: std::collections::BTreeMap::new(),
            af_comp: std::collections::BTreeMap::new(),
        };

        while !vim_fgets(rline.as_mut_ptr(), MAXLINELEN as c_int, fd) && !got_int {
            line_breakcheck();
            lnum += 1;

            if rline[0] == b'#' as c_char {
                continue;
            }

            xfree(pc.cast());
            let line = if (*spin).si_conv.vc_type != CONV_NONE {
                pc = string_convert(
                    &(*spin).si_conv,
                    rline.as_mut_ptr(),
                    ptr::null_mut(),
                );
                if pc.is_null() {
                    smsg!(
                        0,
                        gettext(c"Conversion failure for word in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        rline.as_ptr(),
                    );
                    continue;
                }
                pc
            } else {
                pc = ptr::null_mut();
                rline.as_mut_ptr()
            };

            let mut itemcnt: usize = 0;
            let mut p = line;
            loop {
                while *p != NUL && (*p as u8) <= b' ' {
                    p = p.add(1);
                }
                if *p == NUL || itemcnt == MAXITEMCNT {
                    break;
                }
                items[itemcnt] = p;
                itemcnt += 1;
                if itemcnt == 2 && spell_info_item(items[0]) {
                    while (*p as u8) >= b' ' || *p == TAB {
                        p = p.add(1);
                    }
                } else {
                    while (*p as u8) > b' ' {
                        p = p.add(1);
                    }
                }
                if *p == NUL {
                    break;
                }
                *p = NUL;
                p = p.add(1);
            }

            if itemcnt > 0 {
                let itemcnt_i = itemcnt as c_int;
                if is_aff_rule(&items, itemcnt_i, c"SET".as_ptr(), 2) && aff.af_enc.is_null() {
                    aff.af_enc = enc_canonize(items[1]);
                    if (*spin).si_ascii == 0
                        && convert_setup(&mut (*spin).si_conv, aff.af_enc, p_enc) == FAIL
                    {
                        smsg!(
                            0,
                            gettext(c"Conversion in %s not supported: from %s to %s".as_ptr()),
                            fname,
                            aff.af_enc,
                            p_enc,
                        );
                    }
                    (*spin).si_conv.vc_fail = true;
                } else if is_aff_rule(&items, itemcnt_i, c"FLAG".as_ptr(), 2)
                    && aff.af_flagtype == AFT_CHAR
                {
                    if strcmp(items[1], c"long".as_ptr()) == 0 {
                        aff.af_flagtype = AFT_LONG;
                    } else if strcmp(items[1], c"num".as_ptr()) == 0 {
                        aff.af_flagtype = AFT_NUM;
                    } else if strcmp(items[1], c"caplong".as_ptr()) == 0 {
                        aff.af_flagtype = AFT_CAPLONG;
                    } else {
                        smsg!(
                            0,
                            gettext(c"Invalid value for FLAG in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                    if aff.af_rare != 0
                        || aff.af_keepcase != 0
                        || aff.af_bad != 0
                        || aff.af_needaffix != 0
                        || aff.af_circumfix != 0
                        || aff.af_needcomp != 0
                        || aff.af_comproot != 0
                        || aff.af_nosuggest != 0
                        || !compflags.is_null()
                        || !aff.af_suff.is_empty()
                        || !aff.af_pref.is_empty()
                    {
                        smsg!(
                            0,
                            gettext(c"FLAG after using flags in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if spell_info_item(items[0]) && itemcnt > 1 {
                    let prev_len = if (*spin).si_info.is_null() {
                        0
                    } else {
                        strlen((*spin).si_info)
                    };
                    let p_info = getroom(
                        spin,
                        prev_len + strlen(items[0]) + strlen(items[1]) + 3,
                        false,
                    )
                    .cast::<c_char>();
                    if !(*spin).si_info.is_null() {
                        strcpy(p_info, (*spin).si_info);
                        strcat(p_info, c"\n".as_ptr());
                    }
                    strcat(p_info, items[0]);
                    strcat(p_info, c" ".as_ptr());
                    strcat(p_info, items[1]);
                    (*spin).si_info = p_info;
                } else if is_aff_rule(&items, itemcnt_i, c"MIDWORD".as_ptr(), 2)
                    && midword.is_null()
                {
                    midword = getroom_save(spin, items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"TRY".as_ptr(), 2) {
                    // ignored
                } else if (is_aff_rule(&items, itemcnt_i, c"RAR".as_ptr(), 2)
                    || is_aff_rule(&items, itemcnt_i, c"RARE".as_ptr(), 2))
                    && aff.af_rare == 0
                {
                    aff.af_rare = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if (is_aff_rule(&items, itemcnt_i, c"KEP".as_ptr(), 2)
                    || is_aff_rule(&items, itemcnt_i, c"KEEPCASE".as_ptr(), 2))
                    && aff.af_keepcase == 0
                {
                    aff.af_keepcase = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if (is_aff_rule(&items, itemcnt_i, c"BAD".as_ptr(), 2)
                    || is_aff_rule(&items, itemcnt_i, c"FORBIDDENWORD".as_ptr(), 2))
                    && aff.af_bad == 0
                {
                    aff.af_bad = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if is_aff_rule(&items, itemcnt_i, c"NEEDAFFIX".as_ptr(), 2)
                    && aff.af_needaffix == 0
                {
                    aff.af_needaffix = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if is_aff_rule(&items, itemcnt_i, c"CIRCUMFIX".as_ptr(), 2)
                    && aff.af_circumfix == 0
                {
                    aff.af_circumfix = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if is_aff_rule(&items, itemcnt_i, c"NOSUGGEST".as_ptr(), 2)
                    && aff.af_nosuggest == 0
                {
                    aff.af_nosuggest = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if (is_aff_rule(&items, itemcnt_i, c"NEEDCOMPOUND".as_ptr(), 2)
                    || is_aff_rule(&items, itemcnt_i, c"ONLYINCOMPOUND".as_ptr(), 2))
                    && aff.af_needcomp == 0
                {
                    aff.af_needcomp = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDROOT".as_ptr(), 2)
                    && aff.af_comproot == 0
                {
                    aff.af_comproot = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDFORBIDFLAG".as_ptr(), 2)
                    && aff.af_compforbid == 0
                {
                    aff.af_compforbid = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                    if !aff.af_pref.is_empty() {
                        smsg!(
                            0,
                            gettext(
                                c"Defining COMPOUNDFORBIDFLAG after PFX item may give wrong results in %s line %d"
                                    .as_ptr(),
                            ),
                            fname,
                            lnum,
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDPERMITFLAG".as_ptr(), 2)
                    && aff.af_comppermit == 0
                {
                    aff.af_comppermit = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                    if !aff.af_pref.is_empty() {
                        smsg!(
                            0,
                            gettext(
                                c"Defining COMPOUNDPERMITFLAG after PFX item may give wrong results in %s line %d"
                                    .as_ptr(),
                            ),
                            fname,
                            lnum,
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDFLAG".as_ptr(), 2)
                    && compflags.is_null()
                {
                    let p_cf = getroom(spin, strlen(items[1]) + 2, false).cast::<c_char>();
                    strcpy(p_cf, items[1]);
                    strcat(p_cf, c"+".as_ptr());
                    compflags = p_cf;
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDRULES".as_ptr(), 2) {
                    if atoi(items[1]) == 0 {
                        smsg!(
                            0,
                            gettext(c"Wrong COMPOUNDRULES value in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDRULE".as_ptr(), 2) {
                    if !compflags.is_null() || *skipdigits(items[1]) != NUL {
                        let mut l = strlen(items[1]) + 1;
                        if !compflags.is_null() {
                            l += strlen(compflags) + 1;
                        }
                        let p_cf = getroom(spin, l, false).cast::<c_char>();
                        if !compflags.is_null() {
                            strcpy(p_cf, compflags);
                            strcat(p_cf, c"/".as_ptr());
                        }
                        strcat(p_cf, items[1]);
                        compflags = p_cf;
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDWORDMAX".as_ptr(), 2)
                    && compmax == 0
                {
                    compmax = atoi(items[1]);
                    if compmax == 0 {
                        smsg!(
                            0,
                            gettext(c"Wrong COMPOUNDWORDMAX value in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDMIN".as_ptr(), 2)
                    && compminlen == 0
                {
                    compminlen = atoi(items[1]);
                    if compminlen == 0 {
                        smsg!(
                            0,
                            gettext(c"Wrong COMPOUNDMIN value in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"COMPOUNDSYLMAX".as_ptr(), 2)
                    && compsylmax == 0
                {
                    compsylmax = atoi(items[1]);
                    if compsylmax == 0 {
                        smsg!(
                            0,
                            gettext(c"Wrong COMPOUNDSYLMAX value in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDDUP".as_ptr(), 1) {
                    compoptions |= COMP_CHECKDUP;
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDREP".as_ptr(), 1) {
                    compoptions |= COMP_CHECKREP;
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDCASE".as_ptr(), 1) {
                    compoptions |= COMP_CHECKCASE;
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDTRIPLE".as_ptr(), 1) {
                    compoptions |= COMP_CHECKTRIPLE;
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDPATTERN".as_ptr(), 2) {
                    if atoi(items[1]) == 0 {
                        smsg!(
                            0,
                            gettext(c"Wrong CHECKCOMPOUNDPATTERN value in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[1],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"CHECKCOMPOUNDPATTERN".as_ptr(), 3) {
                    let gap = &mut (*spin).si_comppat;
                    let mut i: usize = 0;
                    while i + 1 < gap.len() {
                        if strcmp(gap[i], items[1]) == 0 && strcmp(gap[i + 1], items[2]) == 0 {
                            break;
                        }
                        i += 2;
                    }
                    if i + 1 >= gap.len() {
                        gap.push(getroom_save(spin, items[1]));
                        gap.push(getroom_save(spin, items[2]));
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"SYLLABLE".as_ptr(), 2)
                    && syllable.is_null()
                {
                    syllable = getroom_save(spin, items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"NOBREAK".as_ptr(), 1) {
                    (*spin).si_nobreak = 1;
                } else if is_aff_rule(&items, itemcnt_i, c"NOSPLITSUGS".as_ptr(), 1) {
                    (*spin).si_nosplitsugs = 1;
                } else if is_aff_rule(&items, itemcnt_i, c"NOCOMPOUNDSUGS".as_ptr(), 1) {
                    (*spin).si_nocompoundsugs = 1;
                } else if is_aff_rule(&items, itemcnt_i, c"NOSUGFILE".as_ptr(), 1) {
                    (*spin).si_nosugfile = 1;
                } else if is_aff_rule(&items, itemcnt_i, c"PFXPOSTPONE".as_ptr(), 1) {
                    aff.af_pfxpostpone = 1;
                } else if is_aff_rule(&items, itemcnt_i, c"IGNOREEXTRA".as_ptr(), 1) {
                    aff.af_ignoreextra = true;
                } else if (strcmp(items[0], c"PFX".as_ptr()) == 0
                    || strcmp(items[0], c"SFX".as_ptr()) == 0)
                    && aff_todo == 0
                    && itemcnt >= 4
                {
                    let mut lasti: usize = 4;
                    let mut key = [0 as c_char; AH_KEY_LEN];

                    let tp = if *items[0] == b'P' as c_char {
                        &mut aff.af_pref
                    } else {
                        &mut aff.af_suff
                    };

                    xstrlcpy(key.as_mut_ptr(), items[1], AH_KEY_LEN);
                    let key_bytes = std::ffi::CStr::from_ptr(key.as_ptr()).to_bytes();
                    if let Some(existing_aff) = tp.get_mut(key_bytes) {
                        cur_aff = &raw mut *existing_aff;
                        if (*cur_aff).ah_combine != c_int::from(*items[2] == b'Y' as c_char) {
                            smsg!(
                                0,
                                gettext(
                                    c"Different combining flag in continued affix block in %s line %d: %s"
                                        .as_ptr(),
                                ),
                                fname,
                                lnum,
                                items[1],
                            );
                        }
                        if (*cur_aff).ah_follows == 0 {
                            smsg!(
                                0,
                                gettext(c"Duplicate affix in %s line %d: %s".as_ptr()),
                                fname,
                                lnum,
                                items[1],
                            );
                        }
                    } else {
                        let ah_flag = affitem2flag(aff.af_flagtype, items[1], fname, lnum);
                        if ah_flag == 0 || strlen(items[1]) >= AH_KEY_LEN {
                            break;
                        }
                        if ah_flag == aff.af_bad
                            || ah_flag == aff.af_rare
                            || ah_flag == aff.af_keepcase
                            || ah_flag == aff.af_needaffix
                            || ah_flag == aff.af_circumfix
                            || ah_flag == aff.af_nosuggest
                            || ah_flag == aff.af_needcomp
                            || ah_flag == aff.af_comproot
                        {
                            smsg!(
                                0,
                                gettext(
                                    c"Affix also used for BAD/RARE/KEEPCASE/NEEDAFFIX/NEEDCOMPOUND/NOSUGGEST in %s line %d: %s"
                                        .as_ptr(),
                                ),
                                fname,
                                lnum,
                                items[1],
                            );
                        }
                        let mut new_ah = affheader_T {
                            ah_key: [0; AH_KEY_LEN],
                            ah_flag,
                            ah_newID: 0,
                            ah_combine: c_int::from(*items[2] == b'Y' as c_char),
                            ah_follows: 0,
                            ah_entries: Vec::new(),
                        };
                        strcpy(new_ah.ah_key.as_mut_ptr(), items[1]);
                        cur_aff = &raw mut *tp.entry(key_bytes.to_vec()).or_insert(new_ah);
                    }

                    if itemcnt > lasti && strcmp(items[lasti], c"S".as_ptr()) == 0 {
                        lasti += 1;
                        (*cur_aff).ah_follows = 1;
                    } else {
                        (*cur_aff).ah_follows = 0;
                    }

                    if itemcnt > lasti && !aff.af_ignoreextra && *items[lasti] != b'#' as c_char {
                        smsg!(0, gettext(e_afftrailing), fname, lnum, items[lasti]);
                    }

                    if strcmp(items[2], c"Y".as_ptr()) != 0 && strcmp(items[2], c"N".as_ptr()) != 0
                    {
                        smsg!(
                            0,
                            gettext(c"Expected Y or N in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            items[2],
                        );
                    }

                    if *items[0] == b'P' as c_char && aff.af_pfxpostpone != 0 {
                        if (*cur_aff).ah_newID == 0 {
                            check_renumber(spin);
                            (*spin).si_newprefID += 1;
                            (*cur_aff).ah_newID = (*spin).si_newprefID;
                            did_postpone_prefix = false;
                        } else {
                            did_postpone_prefix = true;
                        }
                    }

                    aff_todo = atoi(items[3]);
                } else if (strcmp(items[0], c"PFX".as_ptr()) == 0
                    || strcmp(items[0], c"SFX".as_ptr()) == 0)
                    && aff_todo > 0
                    && strcmp((&raw const (*cur_aff).ah_key).cast(), items[1]) == 0
                    && itemcnt >= 5
                {
                    let lasti: usize = 5;
                    if itemcnt > lasti
                        && *items[lasti] != b'#' as c_char
                        && (strcmp(items[lasti], c"-".as_ptr()) != 0 || itemcnt != lasti + 1)
                    {
                        smsg!(0, gettext(e_afftrailing), fname, lnum, items[lasti]);
                    }

                    aff_todo -= 1;
                    let mut aff_entry_val = affentry_T {
                        ae_chop: ptr::null_mut(),
                        ae_add: ptr::null_mut(),
                        ae_flags: ptr::null_mut(),
                        ae_cond: ptr::null_mut(),
                        ae_prog: ptr::null_mut(),
                        ae_compforbid: 0,
                        ae_comppermit: 0,
                    };
                    let aff_entry: *mut affentry_T = &raw mut aff_entry_val;

                    if strcmp(items[2], c"0".as_ptr()) != 0 {
                        (*aff_entry).ae_chop = getroom_save(spin, items[2]);
                    }
                    if strcmp(items[3], c"0".as_ptr()) != 0 {
                        (*aff_entry).ae_add = getroom_save(spin, items[3]);
                        (*aff_entry).ae_flags = vim_strchr((*aff_entry).ae_add, b'/' as c_int);
                        if !(*aff_entry).ae_flags.is_null() {
                            *(*aff_entry).ae_flags = NUL;
                            (*aff_entry).ae_flags = (*aff_entry).ae_flags.add(1);
                            aff_process_flags(&aff, &mut *aff_entry);
                        }
                    }

                    if (*spin).si_ascii == 0
                        || !(has_non_ascii((*aff_entry).ae_chop)
                            || has_non_ascii((*aff_entry).ae_add))
                    {
                        (*cur_aff).ah_entries.insert(0, aff_entry_val);
                        let aff_entry: *mut affentry_T = (*cur_aff).ah_entries.as_mut_ptr();

                        if strcmp(items[4], c".".as_ptr()) != 0 {
                            let mut buf = [0 as c_char; MAXLINELEN];
                            (*aff_entry).ae_cond = getroom_save(spin, items[4]);
                            vim_snprintf!(
                                buf.as_mut_ptr(),
                                MAXLINELEN,
                                if *items[0] == b'P' as c_char {
                                    c"^%s".as_ptr()
                                } else {
                                    c"%s$".as_ptr()
                                },
                                items[4],
                            );
                            (*aff_entry).ae_prog =
                                vim_regcomp(buf.as_ptr(), RE_MAGIC + RE_STRING + RE_STRICT);
                            if (*aff_entry).ae_prog.is_null() {
                                smsg!(
                                    0,
                                    gettext(c"Broken condition in %s line %d: %s".as_ptr()),
                                    fname,
                                    lnum,
                                    items[4],
                                );
                            }
                        }

                        if *items[0] == b'P' as c_char
                            && aff.af_pfxpostpone != 0
                            && (*aff_entry).ae_flags.is_null()
                        {
                            let mut upper = false;
                            if !(*aff_entry).ae_chop.is_null()
                                && !(*aff_entry).ae_add.is_null()
                                && *(*aff_entry)
                                    .ae_chop
                                    .add(utfc_ptr2len((*aff_entry).ae_chop) as usize)
                                    == NUL
                            {
                                let c = utf_ptr2char((*aff_entry).ae_chop);
                                let c_up = spell_toupper(c);
                                if c_up != c
                                    && ((*aff_entry).ae_cond.is_null()
                                        || utf_ptr2char((*aff_entry).ae_cond) == c)
                                {
                                    let mut p_end =
                                        (*aff_entry).ae_add.add(strlen((*aff_entry).ae_add));
                                    mb_ptr_back((*aff_entry).ae_add, &mut p_end);
                                    if utf_ptr2char(p_end) == c_up {
                                        upper = true;
                                        (*aff_entry).ae_chop = ptr::null_mut();
                                        *p_end = NUL;

                                        if !(*aff_entry).ae_cond.is_null() {
                                            let mut buf = [0 as c_char; MAXLINELEN];
                                            onecap_copy(items[4], buf.as_mut_ptr(), true);
                                            (*aff_entry).ae_cond =
                                                getroom_save(spin, buf.as_mut_ptr());
                                            if !(*aff_entry).ae_cond.is_null() {
                                                vim_snprintf!(
                                                    buf.as_mut_ptr(),
                                                    MAXLINELEN,
                                                    c"^%s".as_ptr(),
                                                    (*aff_entry).ae_cond,
                                                );
                                                vim_regfree((*aff_entry).ae_prog);
                                                (*aff_entry).ae_prog =
                                                    vim_regcomp(buf.as_ptr(), RE_MAGIC + RE_STRING);
                                            }
                                        }
                                    }
                                }
                            }

                            if (*aff_entry).ae_chop.is_null() {
                                let prefcond = &mut (*spin).si_prefcond;
                                let mut idx = prefcond.len() as c_int - 1;
                                while idx >= 0 {
                                    let p_cond = prefcond[idx as usize];
                                    if strequal(p_cond, (*aff_entry).ae_cond) {
                                        break;
                                    }
                                    idx -= 1;
                                }
                                if idx < 0 {
                                    idx = prefcond.len() as c_int;
                                    let cond_str = if (*aff_entry).ae_cond.is_null() {
                                        ptr::null_mut()
                                    } else {
                                        getroom_save(spin, (*aff_entry).ae_cond)
                                    };
                                    prefcond.push(cond_str);
                                }

                                let p_add = if (*aff_entry).ae_add.is_null() {
                                    c"".as_ptr()
                                } else {
                                    (*aff_entry).ae_add
                                };

                                let mut n = PFX_FLAGS;
                                if (*cur_aff).ah_combine == 0 {
                                    n |= WFP_NC;
                                }
                                if upper {
                                    n |= WFP_UP;
                                }
                                if (*aff_entry).ae_comppermit != 0 {
                                    n |= WFP_COMPPERMIT;
                                }
                                if (*aff_entry).ae_compforbid != 0 {
                                    n |= WFP_COMPFORBID;
                                }
                                tree_add_word(
                                    spin,
                                    p_add,
                                    (*spin).si_prefroot,
                                    n,
                                    idx,
                                    (*cur_aff).ah_newID,
                                );
                                did_postpone_prefix = true;
                            }

                            if aff_todo == 0 && !did_postpone_prefix {
                                (*spin).si_newprefID -= 1;
                                (*cur_aff).ah_newID = 0;
                            }
                        }
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"FOL".as_ptr(), 2) && fol.is_null() {
                    fol = xstrdup(items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"LOW".as_ptr(), 2) && low.is_null() {
                    low = xstrdup(items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"UPP".as_ptr(), 2) && upp.is_null() {
                    upp = xstrdup(items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"REP".as_ptr(), 2)
                    || is_aff_rule(&items, itemcnt_i, c"REPSAL".as_ptr(), 2)
                {
                    if !(*items[1] as u8).is_ascii_digit() {
                        smsg!(
                            0,
                            gettext(c"Expected REP(SAL) count in %s line %d".as_ptr()),
                            fname,
                            lnum,
                        );
                    }
                } else if (strcmp(items[0], c"REP".as_ptr()) == 0
                    || strcmp(items[0], c"REPSAL".as_ptr()) == 0)
                    && itemcnt >= 3
                {
                    if itemcnt > 3 && *items[3] != b'#' as c_char {
                        smsg!(0, gettext(e_afftrailing), fname, lnum, items[3]);
                    }
                    let is_repsal = *items[0].add(3) == b'S' as c_char;
                    if if is_repsal { do_repsal } else { do_rep } {
                        let mut p1 = items[1];
                        while *p1 != NUL {
                            if *p1 == b'_' as c_char {
                                *p1 = b' ' as c_char;
                            }
                            mb_ptr_adv(&mut p1);
                        }
                        let mut p2 = items[2];
                        while *p2 != NUL {
                            if *p2 == b'_' as c_char {
                                *p2 = b' ' as c_char;
                            }
                            mb_ptr_adv(&mut p2);
                        }
                        add_fromto(
                            spin,
                            if is_repsal {
                                &mut (*spin).si_repsal
                            } else {
                                &mut (*spin).si_rep
                            },
                            items[1],
                            items[2],
                        );
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"MAP".as_ptr(), 2) {
                    if !found_map {
                        found_map = true;
                        if !(*items[1] as u8).is_ascii_digit() {
                            smsg!(
                                0,
                                gettext(c"Expected MAP count in %s line %d".as_ptr()),
                                fname,
                                lnum,
                            );
                        }
                    } else if do_mapline {
                        let si_map = &mut (*spin).si_map;
                        let mut p_m: *const c_char = items[1];
                        while *p_m != NUL {
                            let c = mb_ptr2char_adv(&raw mut p_m);
                            let in_map = if si_map.is_empty() {
                                false
                            } else {
                                si_map.push(NUL);
                                let found = !vim_strchr(si_map.as_ptr(), c).is_null();
                                si_map.pop();
                                found
                            };
                            if in_map || !vim_strchr(p_m, c).is_null() {
                                smsg!(
                                    0,
                                    gettext(c"Duplicate character in MAP in %s line %d".as_ptr()),
                                    fname,
                                    lnum,
                                );
                            }
                        }
                        if !items[1].is_null() {
                            let s_len = strlen(items[1]);
                            si_map.extend_from_slice(std::slice::from_raw_parts(items[1], s_len));
                        }
                        si_map.push(b'/' as c_char);
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"SAL".as_ptr(), 3) {
                    if do_sal {
                        if strcmp(items[1], c"followup".as_ptr()) == 0 {
                            (*spin).si_followup = c_int::from(sal_to_bool(items[2]));
                        } else if strcmp(items[1], c"collapse_result".as_ptr()) == 0 {
                            (*spin).si_collapse = c_int::from(sal_to_bool(items[2]));
                        } else if strcmp(items[1], c"remove_accents".as_ptr()) == 0 {
                            (*spin).si_rem_accents = c_int::from(sal_to_bool(items[2]));
                        } else {
                            let to_s = if strcmp(items[2], c"_".as_ptr()) == 0 {
                                c"".as_ptr().cast_mut()
                            } else {
                                items[2]
                            };
                            add_fromto(spin, &mut (*spin).si_sal, items[1], to_s);
                        }
                    }
                } else if is_aff_rule(&items, itemcnt_i, c"SOFOFROM".as_ptr(), 2)
                    && sofofrom.is_null()
                {
                    sofofrom = getroom_save(spin, items[1]);
                } else if is_aff_rule(&items, itemcnt_i, c"SOFOTO".as_ptr(), 2)
                    && sofoto.is_null()
                {
                    sofoto = getroom_save(spin, items[1]);
                } else if strcmp(items[0], c"COMMON".as_ptr()) == 0 {
                    for &item in items.iter().take(itemcnt).skip(1) {
                        (*spin)
                            .si_commonwords
                            .insert(std::ffi::CStr::from_ptr(item).to_bytes().to_vec());
                    }
                } else {
                    smsg!(
                        0,
                        gettext(c"Unrecognized or duplicate item in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        items[0],
                    );
                }
            }
        }

        if !fol.is_null() || !low.is_null() || !upp.is_null() {
            if (*spin).si_clear_chartab != 0 {
                init_spell_chartab();
                (*spin).si_clear_chartab = 0;
            }
            xfree(fol.cast());
            xfree(low.cast());
            xfree(upp.cast());
        }

        if compmax != 0 {
            aff_check_number((*spin).si_compmax, compmax, c"COMPOUNDWORDMAX".as_ptr());
            (*spin).si_compmax = compmax;
        }

        if compminlen != 0 {
            aff_check_number((*spin).si_compminlen, compminlen, c"COMPOUNDMIN".as_ptr());
            (*spin).si_compminlen = compminlen;
        }

        if compsylmax != 0 {
            if syllable.is_null() {
                smsg!(
                    0,
                    c"%s".as_ptr(),
                    gettext(c"COMPOUNDSYLMAX used without SYLLABLE".as_ptr()),
                );
            }
            aff_check_number(
                (*spin).si_compsylmax,
                compsylmax,
                c"COMPOUNDSYLMAX".as_ptr(),
            );
            (*spin).si_compsylmax = compsylmax;
        }

        if compoptions != 0 {
            aff_check_number(
                (*spin).si_compoptions,
                compoptions,
                c"COMPOUND options".as_ptr(),
            );
            (*spin).si_compoptions |= compoptions;
        }

        if !compflags.is_null() {
            process_compflags(spin, &mut aff, compflags);
        }

        if (*spin).si_newcompID < (*spin).si_newprefID {
            if (*spin).si_newcompID == 127 || (*spin).si_newcompID == 255 {
                msg(gettext(c"Too many postponed prefixes".as_ptr()), 0);
            } else if (*spin).si_newprefID == 0 || (*spin).si_newprefID == 127 {
                msg(gettext(c"Too many compound flags".as_ptr()), 0);
            } else {
                msg(
                    gettext(c"Too many postponed prefixes and/or compound flags".as_ptr()),
                    0,
                );
            }
        }

        if !syllable.is_null() {
            aff_check_string((*spin).si_syllable, syllable, c"SYLLABLE".as_ptr());
            (*spin).si_syllable = syllable;
        }

        if !sofofrom.is_null() || !sofoto.is_null() {
            if sofofrom.is_null() || sofoto.is_null() {
                smsg!(
                    0,
                    gettext(c"Missing SOFO%s line in %s".as_ptr()),
                    if sofofrom.is_null() {
                        c"FROM".as_ptr()
                    } else {
                        c"TO".as_ptr()
                    },
                    fname,
                );
            } else if !(*spin).si_sal.is_empty() {
                smsg!(0, gettext(c"Both SAL and SOFO lines in %s".as_ptr()), fname);
            } else {
                aff_check_string((*spin).si_sofofr, sofofrom, c"SOFOFROM".as_ptr());
                aff_check_string((*spin).si_sofoto, sofoto, c"SOFOTO".as_ptr());
                (*spin).si_sofofr = sofofrom;
                (*spin).si_sofoto = sofoto;
            }
        }

        if !midword.is_null() {
            aff_check_string((*spin).si_midword, midword, c"MIDWORD".as_ptr());
            (*spin).si_midword = midword;
        }

        xfree(pc.cast());
        fclose(fd);
        Some(aff)
    }
}

unsafe fn is_aff_rule(
    items: &[*mut c_char],
    itemcnt: c_int,
    rulename: *const c_char,
    mincount: c_int,
) -> bool {
    unsafe {
        strcmp(items[0], rulename) == 0
            && (itemcnt == mincount
                || (itemcnt > mincount && *items[mincount as usize] == b'#' as c_char))
    }
}

unsafe fn aff_process_flags(affile: &afffile_T, entry: &mut affentry_T) {
    unsafe {
        if !entry.ae_flags.is_null()
            && (affile.af_compforbid != 0 || affile.af_comppermit != 0)
        {
            let mut p = entry.ae_flags;
            while *p != NUL {
                let prevp = p;
                let flag = get_affitem(affile.af_flagtype, &raw mut p);
                if flag == affile.af_comppermit || flag == affile.af_compforbid {
                    ptr::copy(p, prevp, strlen(p) + 1);
                    p = prevp;
                    if flag == affile.af_comppermit {
                        entry.ae_comppermit = 1;
                    } else {
                        entry.ae_compforbid = 1;
                    }
                }
                if affile.af_flagtype == AFT_NUM && *p == b',' as c_char {
                    p = p.add(1);
                }
            }
            if *entry.ae_flags == NUL {
                entry.ae_flags = ptr::null_mut();
            }
        }
    }
}

unsafe fn spell_info_item(s: *const c_char) -> bool {
    unsafe {
        strcmp(s, c"NAME".as_ptr()) == 0
            || strcmp(s, c"HOME".as_ptr()) == 0
            || strcmp(s, c"VERSION".as_ptr()) == 0
            || strcmp(s, c"AUTHOR".as_ptr()) == 0
            || strcmp(s, c"EMAIL".as_ptr()) == 0
            || strcmp(s, c"COPYRIGHT".as_ptr()) == 0
    }
}

unsafe fn affitem2flag(
    flagtype: c_int,
    item: *mut c_char,
    fname: *mut c_char,
    lnum: c_int,
) -> c_uint {
    unsafe {
        let mut p = item;
        let res = get_affitem(flagtype, &raw mut p);
        if res == 0 {
            if flagtype == AFT_NUM {
                smsg!(
                    0,
                    gettext(c"Flag is not a number in %s line %d: %s".as_ptr()),
                    fname,
                    lnum,
                    item,
                );
            } else {
                smsg!(
                    0,
                    gettext(c"Illegal flag in %s line %d: %s".as_ptr()),
                    fname,
                    lnum,
                    item,
                );
            }
        }
        if *p != NUL {
            smsg!(0, gettext(e_affname), fname, lnum, item);
            return 0;
        }
        res
    }
}

unsafe fn get_affitem(flagtype: c_int, pp: *mut *mut c_char) -> c_uint {
    unsafe {
        let mut res: c_int;
        if flagtype == AFT_NUM {
            if !ascii_isdigit(**pp as c_int) {
                *pp = (*pp).add(1);
                return 0;
            }
            res = getdigits_int(pp, true, 0);
            if res == 0 {
                res = ZERO_FLAG as c_int;
            }
        } else {
            let mut cp = (*pp).cast_const();
            res = mb_ptr2char_adv(&raw mut cp);
            *pp = cp.cast_mut();
            if flagtype == AFT_LONG
                || (flagtype == AFT_CAPLONG && res >= b'A' as c_int && res <= b'Z' as c_int)
            {
                if **pp == NUL {
                    return 0;
                }
                let mut cp2 = (*pp).cast_const();
                res = mb_ptr2char_adv(&raw mut cp2) + (res << 16);
                *pp = cp2.cast_mut();
            }
        }
        res as c_uint
    }
}

unsafe fn process_compflags(spin: *mut spellinfo_T, aff: &mut afffile_T, compflags: *mut c_char) {
    unsafe {
        let mut key = [0 as c_char; AH_KEY_LEN];

        let mut len = strlen(compflags) + 1;
        if !(*spin).si_compflags.is_null() {
            len += strlen((*spin).si_compflags) + 1;
        }
        let p = getroom(spin, len, false).cast::<c_char>();
        if !(*spin).si_compflags.is_null() {
            strcpy(p, (*spin).si_compflags);
            strcat(p, c"/".as_ptr());
        }
        (*spin).si_compflags = p;
        let mut tp = p.cast::<u8>().add(strlen(p));

        let mut cur = compflags;
        while *cur != NUL {
            if !vim_strchr(c"/?*+[]".as_ptr(), *cur as u8 as c_int).is_null() {
                *tp = *cur as u8;
                tp = tp.add(1);
                cur = cur.add(1);
            } else {
                let prevp = cur;
                let flag = get_affitem(aff.af_flagtype, &raw mut cur);
                if flag != 0 {
                    xmemcpyz(
                        key.as_mut_ptr().cast(),
                        prevp.cast(),
                        cur.offset_from(prevp) as usize,
                    );
                    let key_bytes = std::ffi::CStr::from_ptr(key.as_ptr()).to_bytes();
                    let id = if let Some(ci) = aff.af_comp.get(key_bytes) {
                        ci.ci_newID
                    } else {
                        let mut ci = compitem_T {
                            ci_key: [0; AH_KEY_LEN],
                            ci_flag: flag,
                            ci_newID: 0,
                        };
                        strcpy(ci.ci_key.as_mut_ptr(), key.as_ptr());
                        let mut new_id: c_int;
                        loop {
                            check_renumber(spin);
                            new_id = (*spin).si_newcompID;
                            (*spin).si_newcompID -= 1;
                            if vim_strchr(c"/?*+[]\\-^".as_ptr(), new_id).is_null() {
                                break;
                            }
                        }
                        ci.ci_newID = new_id;
                        aff.af_comp.insert(key_bytes.to_vec(), ci);
                        new_id
                    };
                    *tp = id as u8;
                    tp = tp.add(1);
                }
                if aff.af_flagtype == AFT_NUM && *cur == b',' as c_char {
                    cur = cur.add(1);
                }
            }
        }

        *tp = 0;
    }
}

unsafe fn check_renumber(spin: *mut spellinfo_T) {
    unsafe {
        if (*spin).si_newprefID == (*spin).si_newcompID && (*spin).si_newcompID < 128 {
            (*spin).si_newprefID = 127;
            (*spin).si_newcompID = 255;
        }
    }
}

unsafe fn store_afflist_add(store_afflist: *mut c_char, cntp: *mut c_int, id: c_int) -> c_int {
    unsafe {
        if *cntp >= (MAXWLEN - 1) as c_int {
            emsg(gettext(
                (&raw const e_too_many_postponed_prefixes_spell).cast(),
            ));
            return FAIL;
        }
        *store_afflist.offset(*cntp as isize) = id as u8 as c_char;
        *cntp += 1;
        *store_afflist.offset(*cntp as isize) = NUL;
        OK
    }
}

unsafe fn flag_in_afflist(flagtype: c_int, afflist: *mut c_char, flag: c_uint) -> bool {
    unsafe {
        match flagtype {
            AFT_CHAR => !vim_strchr(afflist, flag as c_int).is_null(),
            AFT_CAPLONG | AFT_LONG => {
                let mut p: *const c_char = afflist;
                while *p != NUL {
                    let mut n = mb_ptr2char_adv(&raw mut p) as c_uint;
                    if (flagtype == AFT_LONG || (n >= b'A' as c_uint && n <= b'Z' as c_uint))
                        && *p != NUL
                    {
                        n = (mb_ptr2char_adv(&raw mut p) as c_uint) + (n << 16);
                    }
                    if n == flag {
                        return true;
                    }
                }
                false
            }
            AFT_NUM => {
                let mut p = afflist;
                while *p != NUL {
                    let digits = getdigits_int(&raw mut p, true, 0);
                    debug_assert!(digits >= 0);
                    let mut n = digits as c_uint;
                    if n == 0 {
                        n = ZERO_FLAG;
                    }
                    if n == flag {
                        return true;
                    }
                    if *p != NUL {
                        p = p.add(1);
                    }
                }
                false
            }
            _ => false,
        }
    }
}

unsafe fn aff_check_number(spinval: c_int, affval: c_int, name: *const c_char) {
    unsafe {
        if spinval != 0 && spinval != affval {
            smsg!(
                0,
                gettext(c"%s value differs from what is used in another .aff file".as_ptr()),
                name,
            );
        }
    }
}

unsafe fn aff_check_string(spinval: *const c_char, affval: *const c_char, name: *const c_char) {
    unsafe {
        if !spinval.is_null() && strcmp(spinval, affval) != 0 {
            smsg!(
                0,
                gettext(c"%s value differs from what is used in another .aff file".as_ptr()),
                name,
            );
        }
    }
}

unsafe fn add_fromto(
    _spin: *mut spellinfo_T,
    gap: &mut Vec<fromto_T>,
    from: *mut c_char,
    to: *mut c_char,
) {
    unsafe {
        let mut word = [0 as c_char; MAXWLEN];
        spell_casefold(
            curwin,
            from,
            strlen(from) as c_int,
            word.as_mut_ptr(),
            MAXWLEN as c_int,
        );
        let ft_from = xstrdup(word.as_ptr());
        spell_casefold(
            curwin,
            to,
            strlen(to) as c_int,
            word.as_mut_ptr(),
            MAXWLEN as c_int,
        );
        let ft_to = xstrdup(word.as_ptr());
        gap.push(fromto_T { ft_from, ft_to });
    }
}

unsafe fn sal_to_bool(s: *const c_char) -> bool {
    unsafe { strcmp(s, c"1".as_ptr()) == 0 || strcmp(s, c"true".as_ptr()) == 0 }
}

unsafe fn spell_read_dic(
    spin: *mut spellinfo_T,
    fname: *mut c_char,
    affile: &afffile_T,
) -> c_int {
    unsafe {
        let mut ht: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        let mut line = [0 as c_char; MAXLINELEN];
        let mut store_afflist = [0 as c_char; MAXWLEN];
        let mut lnum: c_int = 1;
        let mut non_ascii: c_int = 0;
        let mut retval: c_int = OK;
        let mut message = [0 as c_char; MAXLINELEN + MAXWLEN];
        let mut duplicate: c_int = 0;
        let mut last_msg_time: Timestamp = 0;

        let fd = os_fopen(fname, c"r".as_ptr());
        if fd.is_null() {
            semsg!(gettext((&raw const e_notopen).cast()), fname);
            return FAIL;
        }

        vim_snprintf!(
            (&raw mut IObuff).cast(),
            IOSIZE,
            gettext(c"Reading dictionary file %s...".as_ptr()),
            fname,
        );
        spell_message(spin, (&raw mut IObuff).cast());

        (*spin).si_msg_count = 999999;

        if vim_fgets(line.as_mut_ptr(), MAXLINELEN as c_int, fd)
            || !ascii_isdigit(*skipwhite(line.as_ptr()) as c_int)
        {
            semsg!(gettext(c"E760: No word count in %s".as_ptr()), fname);
        }

        while !vim_fgets(line.as_mut_ptr(), MAXLINELEN as c_int, fd) && !got_int {
            line_breakcheck();
            lnum += 1;
            if line[0] == b'#' as c_char || line[0] == b'/' as c_char {
                continue;
            }
            let mut l = strlen(line.as_ptr());
            while l > 0 && (line[l - 1] as u8) <= b' ' {
                l -= 1;
            }
            if l == 0 {
                continue;
            }
            line[l] = NUL;

            let pc: *mut c_char;
            let w = if (*spin).si_conv.vc_type != CONV_NONE {
                pc = string_convert(
                    &(*spin).si_conv,
                    line.as_mut_ptr(),
                    ptr::null_mut(),
                );
                if pc.is_null() {
                    smsg!(
                        0,
                        gettext(c"Conversion failure for word in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        line.as_ptr(),
                    );
                    continue;
                }
                pc
            } else {
                pc = ptr::null_mut();
                line.as_mut_ptr()
            };

            let mut afflist: *mut c_char = ptr::null_mut();
            let mut p = w;
            while *p != NUL {
                if *p == b'\\' as c_char
                    && (*p.add(1) == b'\\' as c_char || *p.add(1) == b'/' as c_char)
                {
                    ptr::copy(p.add(1), p, strlen(p.add(1)) + 1);
                } else if *p == b'/' as c_char {
                    *p = NUL;
                    afflist = p.add(1);
                    break;
                }
                mb_ptr_adv(&mut p);
            }

            if (*spin).si_ascii != 0 && has_non_ascii(w) {
                non_ascii += 1;
                xfree(pc.cast());
                continue;
            }

            if (*spin).si_verbose != 0 && (*spin).si_msg_count > 10000 {
                (*spin).si_msg_count = 0;
                if os_time() > last_msg_time {
                    last_msg_time = os_time();
                    vim_snprintf!(
                        message.as_mut_ptr(),
                        message.len(),
                        gettext(c"line %6d, word %6d - %s".as_ptr()),
                        lnum,
                        (*spin).si_foldwcount + (*spin).si_keepwcount,
                        w,
                    );
                    msg_start();
                    msg_outtrans_long(message.as_ptr(), 0);
                    msg_clr_eos();
                    msg_didout = false;
                    msg_col = 0;
                    ui_flush();
                }
            }

            let dw = getroom_save(spin, w);
            if dw.is_null() {
                retval = FAIL;
                xfree(pc.cast());
                break;
            }

            let dw_bytes = std::ffi::CStr::from_ptr(dw).to_bytes().to_vec();
            if !ht.insert(dw_bytes) {
                if p_verbose > 0 {
                    smsg!(
                        0,
                        gettext(c"Duplicate word in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        dw,
                    );
                } else if duplicate == 0 {
                    smsg!(
                        0,
                        gettext(c"First duplicate word in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        dw,
                    );
                }
                duplicate += 1;
            }

            let mut flags: c_int = 0;
            store_afflist[0] = NUL;
            let mut pfxlen: c_int = 0;
            let mut totlen: c_int = 0;
            let mut need_affix = false;
            if !afflist.is_null() {
                flags |= get_affix_flags(affile, afflist);

                if affile.af_needaffix != 0
                    && flag_in_afflist(affile.af_flagtype, afflist, affile.af_needaffix)
                {
                    need_affix = true;
                }

                if affile.af_pfxpostpone != 0 {
                    if get_pfxlist(affile, afflist, store_afflist.as_mut_ptr(), &raw mut totlen)
                        == FAIL
                    {
                        retval = FAIL;
                        xfree(pc.cast());
                        break;
                    }
                    pfxlen = totlen;
                }

                if !(*spin).si_compflags.is_null()
                    && get_compflags(affile, afflist, store_afflist.as_mut_ptr(), &raw mut totlen)
                        == FAIL
                {
                    retval = FAIL;
                    xfree(pc.cast());
                    break;
                }
            }

            if store_word(
                spin,
                dw,
                flags,
                (*spin).si_region,
                store_afflist.as_ptr(),
                need_affix,
            ) == FAIL
            {
                retval = FAIL;
            }

            if !afflist.is_null() {
                if store_aff_word(
                    spin,
                    dw,
                    afflist,
                    affile,
                    &affile.af_suff,
                    Some(&affile.af_pref),
                    CONDIT_SUF,
                    flags,
                    store_afflist.as_mut_ptr(),
                    pfxlen,
                ) == FAIL
                {
                    retval = FAIL;
                }

                if store_aff_word(
                    spin,
                    dw,
                    afflist,
                    affile,
                    &affile.af_pref,
                    None,
                    CONDIT_SUF,
                    flags,
                    store_afflist.as_mut_ptr(),
                    pfxlen,
                ) == FAIL
                {
                    retval = FAIL;
                }
            }

            xfree(pc.cast());
        }

        if duplicate > 0 {
            smsg!(
                0,
                gettext(c"%d duplicate word(s) in %s".as_ptr()),
                duplicate,
                fname,
            );
        }
        if (*spin).si_ascii != 0 && non_ascii > 0 {
            smsg!(
                0,
                gettext(c"Ignored %d word(s) with non-ASCII characters in %s".as_ptr()),
                non_ascii,
                fname,
            );
        }

        fclose(fd);
        retval
    }
}

unsafe fn get_affix_flags(affile: &afffile_T, afflist: *mut c_char) -> c_int {
    unsafe {
        let mut flags: c_int = 0;

        if affile.af_keepcase != 0
            && flag_in_afflist(affile.af_flagtype, afflist, affile.af_keepcase)
        {
            flags |= WF_KEEPCAP | WF_FIXCAP;
        }
        if affile.af_rare != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_rare) {
            flags |= WF_RARE;
        }
        if affile.af_bad != 0 && flag_in_afflist(affile.af_flagtype, afflist, affile.af_bad) {
            flags |= WF_BANNED;
        }
        if affile.af_needcomp != 0
            && flag_in_afflist(affile.af_flagtype, afflist, affile.af_needcomp)
        {
            flags |= WF_NEEDCOMP;
        }
        if affile.af_comproot != 0
            && flag_in_afflist(affile.af_flagtype, afflist, affile.af_comproot)
        {
            flags |= WF_COMPROOT;
        }
        if affile.af_nosuggest != 0
            && flag_in_afflist(affile.af_flagtype, afflist, affile.af_nosuggest)
        {
            flags |= WF_NOSUGGEST;
        }
        flags
    }
}

unsafe fn get_pfxlist(
    affile: &afffile_T,
    afflist: *mut c_char,
    store_afflist: *mut c_char,
    cntp: *mut c_int,
) -> c_int {
    unsafe {
        let mut key = [0 as c_char; AH_KEY_LEN];

        let mut p = afflist;
        while *p != NUL {
            let prevp = p;
            if get_affitem(affile.af_flagtype, &raw mut p) != 0 {
                xmemcpyz(
                    key.as_mut_ptr().cast(),
                    prevp.cast(),
                    p.offset_from(prevp) as usize,
                );
                let key_bytes = std::ffi::CStr::from_ptr(key.as_ptr()).to_bytes();
                if let Some(ah) = affile.af_pref.get(key_bytes) {
                    let id = ah.ah_newID;
                    if id != 0 && store_afflist_add(store_afflist, cntp, id) == FAIL {
                        return FAIL;
                    }
                }
            }
            if affile.af_flagtype == AFT_NUM && *p == b',' as c_char {
                p = p.add(1);
            }
        }
        OK
    }
}

unsafe fn get_compflags(
    affile: &afffile_T,
    afflist: *mut c_char,
    store_afflist: *mut c_char,
    cntp: *mut c_int,
) -> c_int {
    unsafe {
        let mut key = [0 as c_char; AH_KEY_LEN];

        let mut p = afflist;
        while *p != NUL {
            let prevp = p;
            if get_affitem(affile.af_flagtype, &raw mut p) != 0 {
                xmemcpyz(
                    key.as_mut_ptr().cast(),
                    prevp.cast(),
                    p.offset_from(prevp) as usize,
                );
                let key_bytes = std::ffi::CStr::from_ptr(key.as_ptr()).to_bytes();
                if let Some(ci) = affile.af_comp.get(key_bytes)
                    && store_afflist_add(store_afflist, cntp, ci.ci_newID) == FAIL
                {
                    return FAIL;
                }
            }
            if affile.af_flagtype == AFT_NUM && *p == b',' as c_char {
                p = p.add(1);
            }
        }
        OK
    }
}

unsafe fn store_aff_word(
    spin: *mut spellinfo_T,
    word: *mut c_char,
    afflist: *mut c_char,
    affile: &afffile_T,
    ht: &std::collections::BTreeMap<Vec<u8>, affheader_T>,
    xht: Option<&std::collections::BTreeMap<Vec<u8>, affheader_T>>,
    condit: c_int,
    flags: c_int,
    pfxlist: *mut c_char,
    pfxlen: c_int,
) -> c_int {
    unsafe {
        let mut newword = [0 as c_char; MAXWLEN];
        let mut retval = OK;
        let mut store_afflist = [0 as c_char; MAXWLEN];
        let mut pfx_pfxlist = [0 as c_char; MAXWLEN];
        let wordlen = strlen(word);

        for ah in ht.values() {
            if retval != OK {
                break;
            }
            {
                if ((condit & CONDIT_COMB) == 0 || ah.ah_combine != 0)
                    && flag_in_afflist(affile.af_flagtype, afflist, ah.ah_flag)
                {
                    for ae_ref in &ah.ah_entries {
                        let ae: *mut affentry_T = (&raw const *ae_ref).cast_mut();
                        if (xht.is_some()
                            || affile.af_pfxpostpone == 0
                            || !(*ae).ae_chop.is_null()
                            || !(*ae).ae_flags.is_null())
                            && ((*ae).ae_chop.is_null() || strlen((*ae).ae_chop) < wordlen)
                            && ((*ae).ae_prog.is_null()
                                || vim_regexec_prog(&raw mut (*ae).ae_prog, false, word, 0))
                            && (((condit & CONDIT_CFIX) == 0)
                                == ((condit & CONDIT_AFF) == 0
                                    || (*ae).ae_flags.is_null()
                                    || !flag_in_afflist(
                                        affile.af_flagtype,
                                        (*ae).ae_flags,
                                        affile.af_circumfix,
                                    )))
                        {
                            if xht.is_none() {
                                if (*ae).ae_add.is_null() {
                                    newword[0] = NUL;
                                } else {
                                    xstrlcpy(newword.as_mut_ptr(), (*ae).ae_add, MAXWLEN);
                                }
                                let mut p = word;
                                if !(*ae).ae_chop.is_null() {
                                    let mut i = mb_charlen((*ae).ae_chop);
                                    while i > 0 {
                                        mb_ptr_adv(&mut p);
                                        i -= 1;
                                    }
                                }
                                xstrlcat(newword.as_mut_ptr(), p, MAXWLEN);
                            } else {
                                xstrlcpy(newword.as_mut_ptr(), word, MAXWLEN);
                                if !(*ae).ae_chop.is_null() {
                                    let mut p = newword.as_mut_ptr().add(strlen(newword.as_ptr()));
                                    let mut i = mb_charlen((*ae).ae_chop);
                                    while i > 0 {
                                        mb_ptr_back(newword.as_ptr(), &mut p);
                                        i -= 1;
                                    }
                                    *p = NUL;
                                }
                                if !(*ae).ae_add.is_null() {
                                    xstrlcat(newword.as_mut_ptr(), (*ae).ae_add, MAXWLEN);
                                }
                            }

                            let mut use_flags = flags;
                            let mut use_pfxlist = pfxlist;
                            let mut use_pfxlen = pfxlen;
                            let mut need_affix = false;
                            let mut use_condit = condit | CONDIT_COMB | CONDIT_AFF;
                            if !(*ae).ae_flags.is_null() {
                                use_flags |= get_affix_flags(affile, (*ae).ae_flags);

                                if affile.af_needaffix != 0
                                    && flag_in_afflist(
                                        affile.af_flagtype,
                                        (*ae).ae_flags,
                                        affile.af_needaffix,
                                    )
                                {
                                    need_affix = true;
                                }

                                if affile.af_circumfix != 0
                                    && flag_in_afflist(
                                        affile.af_flagtype,
                                        (*ae).ae_flags,
                                        affile.af_circumfix,
                                    )
                                {
                                    use_condit |= CONDIT_CFIX;
                                    if (condit & CONDIT_CFIX) == 0 {
                                        need_affix = true;
                                    }
                                }

                                if affile.af_pfxpostpone != 0 || !(*spin).si_compflags.is_null() {
                                    let mut listlen: c_int = 0;

                                    if affile.af_pfxpostpone != 0 {
                                        if get_pfxlist(
                                            affile,
                                            (*ae).ae_flags,
                                            store_afflist.as_mut_ptr(),
                                            &raw mut listlen,
                                        ) == FAIL
                                        {
                                            retval = FAIL;
                                            break;
                                        }
                                        use_pfxlen = listlen;
                                    } else {
                                        use_pfxlen = 0;
                                    }
                                    use_pfxlist = store_afflist.as_mut_ptr();

                                    for i in 0..pfxlen {
                                        let mut j: c_int = 0;
                                        while j < use_pfxlen {
                                            if *pfxlist.offset(i as isize)
                                                == *use_pfxlist.offset(j as isize)
                                            {
                                                break;
                                            }
                                            j += 1;
                                        }
                                        if j == use_pfxlen
                                            && store_afflist_add(
                                                use_pfxlist,
                                                &raw mut listlen,
                                                *pfxlist.offset(i as isize) as u8 as c_int,
                                            ) == FAIL
                                        {
                                            retval = FAIL;
                                            break;
                                        }
                                        use_pfxlen = listlen;
                                    }
                                    if retval == FAIL {
                                        break;
                                    }

                                    if !(*spin).si_compflags.is_null() {
                                        if get_compflags(
                                            affile,
                                            (*ae).ae_flags,
                                            use_pfxlist,
                                            &raw mut listlen,
                                        ) == FAIL
                                        {
                                            retval = FAIL;
                                            break;
                                        }
                                    } else {
                                        *use_pfxlist.offset(use_pfxlen as isize) = NUL;
                                    }
                                    if retval == FAIL {
                                        break;
                                    }

                                    let mut i = pfxlen;
                                    while *pfxlist.offset(i as isize) != NUL {
                                        let mut j = use_pfxlen;
                                        while *use_pfxlist.offset(j as isize) != NUL {
                                            if *pfxlist.offset(i as isize)
                                                == *use_pfxlist.offset(j as isize)
                                            {
                                                break;
                                            }
                                            j += 1;
                                        }
                                        if *use_pfxlist.offset(j as isize) == NUL
                                            && store_afflist_add(
                                                use_pfxlist,
                                                &raw mut listlen,
                                                *pfxlist.offset(i as isize) as u8 as c_int,
                                            ) == FAIL
                                        {
                                            retval = FAIL;
                                            break;
                                        }
                                        i += 1;
                                    }
                                    if retval == FAIL {
                                        break;
                                    }
                                }
                            }

                            if !use_pfxlist.is_null() && (*ae).ae_compforbid != 0 {
                                xmemcpyz(
                                    pfx_pfxlist.as_mut_ptr().cast(),
                                    use_pfxlist.cast(),
                                    use_pfxlen as usize,
                                );
                                use_pfxlist = pfx_pfxlist.as_mut_ptr();
                            }

                            if !(*spin).si_prefroot.is_null()
                                && !(*(*spin).si_prefroot).wn_sibling.is_null()
                            {
                                use_flags |= WF_HAS_AFF;
                                if ah.ah_combine == 0 && !use_pfxlist.is_null() {
                                    use_pfxlist = use_pfxlist.offset(use_pfxlen as isize);
                                }
                            }

                            if !(*spin).si_compflags.is_null() && (*ae).ae_comppermit == 0 {
                                if xht.is_some() {
                                    use_flags |= WF_NOCOMPAFT;
                                } else {
                                    use_flags |= WF_NOCOMPBEF;
                                }
                            }

                            if store_word(
                                spin,
                                newword.as_mut_ptr(),
                                use_flags,
                                (*spin).si_region,
                                use_pfxlist,
                                need_affix,
                            ) == FAIL
                            {
                                retval = FAIL;
                            }

                            if (condit & CONDIT_SUF) != 0 && !(*ae).ae_flags.is_null() {
                                let next_condit =
                                    use_condit & if xht.is_none() { !0 } else { !CONDIT_SUF };
                                if store_aff_word(
                                    spin,
                                    newword.as_mut_ptr(),
                                    (*ae).ae_flags,
                                    affile,
                                    &affile.af_suff,
                                    xht,
                                    next_condit,
                                    use_flags,
                                    use_pfxlist,
                                    pfxlen,
                                ) == FAIL
                                {
                                    retval = FAIL;
                                }
                            }

                            if let Some(xht_ref) = xht
                                && ah.ah_combine != 0
                                && (store_aff_word(
                                    spin,
                                    newword.as_mut_ptr(),
                                    afflist,
                                    affile,
                                    xht_ref,
                                    None,
                                    use_condit,
                                    use_flags,
                                    use_pfxlist,
                                    pfxlen,
                                ) == FAIL
                                    || (!(*ae).ae_flags.is_null()
                                        && store_aff_word(
                                            spin,
                                            newword.as_mut_ptr(),
                                            (*ae).ae_flags,
                                            affile,
                                            xht_ref,
                                            None,
                                            use_condit,
                                            use_flags,
                                            use_pfxlist,
                                            pfxlen,
                                        ) == FAIL))
                            {
                                retval = FAIL;
                            }
                        }
                    }
                }
            }
        }

        retval
    }
}

unsafe fn spell_read_wordfile(spin: *mut spellinfo_T, fname: *mut c_char) -> c_int {
    unsafe {
        let mut lnum: linenr_T = 0;
        let mut rline = [0 as c_char; MAXLINELEN];
        let mut pc: *mut c_char = ptr::null_mut();
        let mut retval = OK;
        let mut did_word = false;
        let mut non_ascii: c_int = 0;

        let fd = os_fopen(fname, c"r".as_ptr());
        if fd.is_null() {
            semsg!(gettext((&raw const e_notopen).cast()), fname);
            return FAIL;
        }

        vim_snprintf!(
            (&raw mut IObuff).cast(),
            IOSIZE,
            gettext(c"Reading word file %s...".as_ptr()),
            fname,
        );
        spell_message(spin, (&raw mut IObuff).cast());

        while !vim_fgets(rline.as_mut_ptr(), MAXLINELEN as c_int, fd) && !got_int {
            line_breakcheck();
            lnum += 1;

            if rline[0] == b'#' as c_char {
                continue;
            }

            let mut l = strlen(rline.as_ptr());
            while l > 0 && (rline[l - 1] as u8) <= b' ' {
                l -= 1;
            }
            if l == 0 {
                continue;
            }
            rline[l] = NUL;

            xfree(pc.cast());
            let mut line = if (*spin).si_conv.vc_type != CONV_NONE {
                pc = string_convert(
                    &(*spin).si_conv,
                    rline.as_mut_ptr(),
                    ptr::null_mut(),
                );
                if pc.is_null() {
                    smsg!(
                        0,
                        gettext(c"Conversion failure for word in %s line %d: %s".as_ptr()),
                        fname,
                        lnum,
                        rline.as_ptr(),
                    );
                    continue;
                }
                pc
            } else {
                pc = ptr::null_mut();
                rline.as_mut_ptr()
            };

            if *line == b'/' as c_char {
                line = line.add(1);
                if strncmp(line, c"encoding=".as_ptr(), 9) == 0 {
                    if (*spin).si_conv.vc_type != CONV_NONE {
                        smsg!(
                            0,
                            gettext(
                                c"Duplicate /encoding= line ignored in %s line %d: %s".as_ptr(),
                            ),
                            fname,
                            lnum,
                            line.sub(1),
                        );
                    } else if did_word {
                        smsg!(
                            0,
                            gettext(
                                c"/encoding= line after word ignored in %s line %d: %s".as_ptr(),
                            ),
                            fname,
                            lnum,
                            line.sub(1),
                        );
                    } else {
                        line = line.add(9);
                        let enc = enc_canonize(line);
                        if (*spin).si_ascii == 0
                            && convert_setup(&mut (*spin).si_conv, enc, p_enc) == FAIL
                        {
                            smsg!(
                                0,
                                gettext(c"Conversion in %s not supported: from %s to %s".as_ptr()),
                                fname,
                                line,
                                p_enc,
                            );
                        }
                        xfree(enc.cast());
                        (*spin).si_conv.vc_fail = true;
                    }
                    continue;
                }

                if strncmp(line, c"regions=".as_ptr(), 8) == 0 {
                    if (*spin).si_region_count > 1 {
                        smsg!(
                            0,
                            gettext(c"Duplicate /regions= line ignored in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            line,
                        );
                    } else {
                        line = line.add(8);
                        if strlen(line) > MAXREGIONS * 2 {
                            smsg!(
                                0,
                                gettext(c"Too many regions in %s line %d: %s".as_ptr()),
                                fname,
                                lnum,
                                line,
                            );
                        } else {
                            (*spin).si_region_count = (strlen(line) / 2) as c_int;
                            strcpy((&raw mut (*spin).si_region_name).cast(), line);
                            (*spin).si_region = (1 << (*spin).si_region_count) - 1;
                        }
                    }
                    continue;
                }

                smsg!(
                    0,
                    gettext(c"/ line ignored in %s line %d: %s".as_ptr()),
                    fname,
                    lnum,
                    line.sub(1),
                );
                continue;
            }

            let mut flags: c_int = 0;
            let mut regionmask = (*spin).si_region;

            let mut p = vim_strchr(line, b'/' as c_int);
            if !p.is_null() {
                *p = NUL;
                p = p.add(1);
                while *p != NUL {
                    if *p == b'=' as c_char {
                        flags |= WF_KEEPCAP | WF_FIXCAP;
                    } else if *p == b'!' as c_char {
                        flags |= WF_BANNED;
                    } else if *p == b'?' as c_char {
                        flags |= WF_RARE;
                    } else if ascii_isdigit(*p as u8 as c_int) {
                        if (flags & WF_REGION) == 0 {
                            regionmask = 0;
                        }
                        flags |= WF_REGION;

                        let r = (*p as u8 - b'0') as c_int;
                        if r == 0 || r > (*spin).si_region_count {
                            smsg!(
                                0,
                                gettext(c"Invalid region nr in %s line %d: %s".as_ptr()),
                                fname,
                                lnum,
                                p,
                            );
                            break;
                        }
                        regionmask |= 1 << (r - 1);
                    } else {
                        smsg!(
                            0,
                            gettext(c"Unrecognized flags in %s line %d: %s".as_ptr()),
                            fname,
                            lnum,
                            p,
                        );
                        break;
                    }
                    p = p.add(1);
                }
            }

            if (*spin).si_ascii != 0 && has_non_ascii(line) {
                non_ascii += 1;
                continue;
            }

            if store_word(spin, line, flags, regionmask, ptr::null(), false) == FAIL {
                retval = FAIL;
                break;
            }
            did_word = true;
        }

        xfree(pc.cast());
        fclose(fd);

        if (*spin).si_ascii != 0 && non_ascii > 0 {
            vim_snprintf!(
                (&raw mut IObuff).cast(),
                IOSIZE,
                gettext(c"Ignored %d words with non-ASCII characters".as_ptr()),
                non_ascii,
            );
            spell_message(spin, (&raw mut IObuff).cast());
        }

        retval
    }
}

unsafe fn getroom(spin: *mut spellinfo_T, len: usize, align: bool) -> *mut u8 {
    unsafe {
        debug_assert!(len <= SBLOCKSIZE);

        if align && let Some(bl) = (*spin).si_blocks.last_mut() {
            let align_mask = size_of::<*mut c_char>() - 1;
            bl.sb_used = (bl.sb_used + align_mask) & !align_mask;
        }

        if (*spin)
            .si_blocks
            .last()
            .is_none_or(|bl| bl.sb_used + len > SBLOCKSIZE)
        {
            (*spin).si_blocks.push(Box::new(sblock_T::new()));
            (*spin).si_blocks_cnt += 1;
        }

        let bl = (*spin).si_blocks.last_mut().unwrap();
        let p = bl.sb_data.as_mut_ptr().add(bl.sb_used);
        bl.sb_used += len;
        p.cast()
    }
}

unsafe fn getroom_save(spin: *mut spellinfo_T, s: *const c_char) -> *mut c_char {
    unsafe {
        let s_size = strlen(s) + 1;
        let dst = getroom(spin, s_size, false).cast::<c_char>();
        ptr::copy_nonoverlapping(s, dst, s_size);
        dst
    }
}

unsafe fn wordtree_alloc(spin: *mut spellinfo_T) -> *mut wordnode_T {
    unsafe { getroom(spin, size_of::<wordnode_T>(), true).cast() }
}

unsafe fn valid_spell_word(word: *const c_char, end: *const c_char) -> bool {
    unsafe {
        if !utf_valid_string(word, end) {
            return false;
        }
        let mut p = word;
        while *p != NUL && p < end {
            if (*p as u8) < b' ' || (*p == b'/' as c_char && *p.add(1) == NUL) {
                return false;
            }
            p = p.add(utfc_ptr2len(p) as usize);
        }
        true
    }
}

unsafe fn store_word(
    spin: *mut spellinfo_T,
    word: *mut c_char,
    flags: c_int,
    region: c_int,
    pfxlist: *const c_char,
    need_affix: bool,
) -> c_int {
    unsafe {
        let len = strlen(word) as c_int;
        let ct = captype(word, word.offset(len as isize));
        let mut foldword = [0 as c_char; MAXWLEN];
        let mut res = OK;

        if !valid_spell_word(word, word.offset(len as isize)) {
            return FAIL;
        }

        spell_casefold(curwin, word, len, foldword.as_mut_ptr(), MAXWLEN as c_int);
        let mut p = pfxlist;
        while res == OK {
            if !need_affix || (!p.is_null() && *p != NUL) {
                res = tree_add_word(
                    spin,
                    foldword.as_ptr(),
                    (*spin).si_foldroot,
                    ct | flags,
                    region,
                    if p.is_null() { 0 } else { *p as u8 as c_int },
                );
            }
            if p.is_null() || *p == NUL {
                break;
            }
            p = p.add(1);
        }
        (*spin).si_foldwcount += 1;

        if res == OK && (ct == WF_KEEPCAP || (flags & WF_KEEPCAP) != 0) {
            let mut p2 = pfxlist;
            while res == OK {
                if !need_affix || (!p2.is_null() && *p2 != NUL) {
                    res = tree_add_word(
                        spin,
                        word,
                        (*spin).si_keeproot,
                        flags,
                        region,
                        if p2.is_null() { 0 } else { *p2 as u8 as c_int },
                    );
                }
                if p2.is_null() || *p2 == NUL {
                    break;
                }
                p2 = p2.add(1);
            }
            (*spin).si_keepwcount += 1;
        }
        res
    }
}

unsafe fn tree_add_word(
    spin: *mut spellinfo_T,
    word: *const c_char,
    root: *mut wordnode_T,
    flags: c_int,
    region: c_int,
    affixID: c_int,
) -> c_int {
    unsafe {
        let mut node = root;
        let mut prev: *mut *mut wordnode_T = ptr::null_mut();

        let mut i: usize = 0;
        loop {
            if !node.is_null() && (*node).wn_refs > 1 {
                (*node).wn_refs -= 1;
                let mut copyprev = prev;
                let mut copyp = node;
                while !copyp.is_null() {
                    let np = get_wordnode(spin);
                    if np.is_null() {
                        return FAIL;
                    }
                    (*np).wn_child = (*copyp).wn_child;
                    if !(*np).wn_child.is_null() {
                        (*(*np).wn_child).wn_refs += 1;
                    }
                    (*np).wn_byte = (*copyp).wn_byte;
                    if (*np).wn_byte == 0 {
                        (*np).wn_flags = (*copyp).wn_flags;
                        (*np).wn_region = (*copyp).wn_region;
                        (*np).wn_affixID = (*copyp).wn_affixID;
                    }

                    (*np).wn_refs = 1;
                    if !copyprev.is_null() {
                        *copyprev = np;
                    }
                    copyprev = &raw mut (*np).wn_sibling;

                    if copyp == node {
                        node = np;
                    }
                    copyp = (*copyp).wn_sibling;
                }
            }

            let wb = *word.add(i) as u8;
            while !node.is_null()
                && ((*node).wn_byte < wb
                    || ((*node).wn_byte == 0
                        && if flags < 0 {
                            ((*node).wn_affixID as c_uint) < (affixID as c_uint)
                        } else {
                            ((*node).wn_flags as c_uint) < ((flags & WN_MASK) as c_uint)
                                || ((*node).wn_flags as c_int == (flags & WN_MASK)
                                    && if (*spin).si_sugtree != 0 {
                                        ((*node).wn_region as c_int & 0xffff) < region
                                    } else {
                                        ((*node).wn_affixID as c_uint) < (affixID as c_uint)
                                    })
                        }))
            {
                prev = &raw mut (*node).wn_sibling;
                node = *prev;
            }

            if node.is_null()
                || (*node).wn_byte != wb
                || (wb == 0
                    && (flags < 0
                        || (*spin).si_sugtree != 0
                        || (*node).wn_flags as c_int != (flags & WN_MASK)
                        || (*node).wn_affixID as c_int != affixID))
            {
                let np = get_wordnode(spin);
                if np.is_null() {
                    return FAIL;
                }
                (*np).wn_byte = wb;

                if node.is_null() {
                    (*np).wn_refs = 1;
                } else {
                    (*np).wn_refs = (*node).wn_refs;
                    (*node).wn_refs = 1;
                }
                if !prev.is_null() {
                    *prev = np;
                }
                (*np).wn_sibling = node;
                node = np;
            }

            if wb == 0 {
                (*node).wn_flags = flags as u16;
                (*node).wn_region |= region as i16;
                (*node).wn_affixID = affixID as u8;
                break;
            }
            prev = &raw mut (*node).wn_child;
            node = *prev;
            i += 1;
        }

        (*spin).si_msg_count += 1;

        if (*spin).si_compress_cnt > 1 {
            (*spin).si_compress_cnt -= 1;
            if (*spin).si_compress_cnt == 1 {
                (*spin).si_blocks_cnt += compress_inc;
            }
        }

        let do_compress = if (*spin).si_compress_cnt == 1 {
            (*spin).si_free_count < MAXWLEN as c_int
        } else {
            (*spin).si_blocks_cnt >= compress_start
        };
        if do_compress {
            (*spin).si_blocks_cnt -= compress_inc;
            (*spin).si_compress_cnt = compress_added;

            if (*spin).si_verbose != 0 {
                msg_start();
                msg_puts(gettext(msg_compressing));
                msg_clr_eos();
                msg_didout = false;
                msg_col = 0;
                ui_flush();
            }

            wordtree_compress(spin, (*spin).si_foldroot, c"case-folded".as_ptr());
            if affixID >= 0 {
                wordtree_compress(spin, (*spin).si_keeproot, c"keep-case".as_ptr());
            }
        }

        OK
    }
}

unsafe fn get_wordnode(spin: *mut spellinfo_T) -> *mut wordnode_T {
    unsafe {
        if (*spin).si_first_free.is_null() {
            getroom(spin, size_of::<wordnode_T>(), true).cast()
        } else {
            let n = (*spin).si_first_free;
            (*spin).si_first_free = (*n).wn_child;
            *n = wordnode_T::new();
            (*spin).si_free_count -= 1;
            n
        }
    }
}

unsafe fn deref_wordnode(spin: *mut spellinfo_T, node: *mut wordnode_T) -> c_int {
    unsafe {
        let mut cnt: c_int = 0;

        (*node).wn_refs -= 1;
        if (*node).wn_refs == 0 {
            let mut np = node;
            while !np.is_null() {
                let next = (*np).wn_sibling;
                if !(*np).wn_child.is_null() {
                    cnt += deref_wordnode(spin, (*np).wn_child);
                }
                free_wordnode(spin, np);
                cnt += 1;
                np = next;
            }
            cnt += 1;
        }
        cnt
    }
}

unsafe fn free_wordnode(spin: *mut spellinfo_T, n: *mut wordnode_T) {
    unsafe {
        (*n).wn_child = (*spin).si_first_free;
        (*spin).si_first_free = n;
        (*spin).si_free_count += 1;
    }
}

unsafe fn wordtree_compress(spin: *mut spellinfo_T, root: *mut wordnode_T, name: *const c_char) {
    unsafe {
        let mut ht: std::collections::HashMap<[u8; 6], *mut wordnode_T> =
            std::collections::HashMap::new();
        let mut tot: c_int = 0;

        if (*root).wn_sibling.is_null() {
            return;
        }

        let (n, _) = node_compress(spin, (*root).wn_sibling, &mut ht, &raw mut tot);

        if (*spin).si_verbose != 0 || p_verbose > 2 {
            let perc: c_long = if tot > 1_000_000 {
                ((tot - n) / (tot / 100)) as c_long
            } else if tot == 0 {
                0
            } else {
                ((tot - n) * 100 / tot) as c_long
            };
            vim_snprintf!(
                (&raw mut IObuff).cast(),
                IOSIZE,
                gettext(c"Compressed %s: %d of %d nodes; %d (%ld%%) remaining".as_ptr()),
                name,
                n,
                tot,
                tot - n,
                perc,
            );
            spell_message(spin, (&raw mut IObuff).cast());
        }
    }
}

unsafe fn node_compress(
    spin: *mut spellinfo_T,
    node: *mut wordnode_T,
    ht: &mut std::collections::HashMap<[u8; 6], *mut wordnode_T>,
    tot: *mut c_int,
) -> (c_int, [u8; 6]) {
    unsafe {
        let mut len: c_int = 0;
        let mut compressed: c_int = 0;

        let mut np = node;
        while !np.is_null() && !got_int {
            len += 1;
            let child = (*np).wn_child;
            if !child.is_null() {
                let (child_compressed, hk) = node_compress(spin, child, ht, tot);
                compressed += child_compressed;

                if let Some(&first_tp) = ht.get(&hk) {
                    let mut tp = first_tp;
                    while !tp.is_null() {
                        if node_equal(child, tp) {
                            (*tp).wn_refs += 1;
                            compressed += deref_wordnode(spin, child);
                            (*np).wn_child = tp;
                            break;
                        }
                        tp = (*tp).wn_wnode;
                    }
                    if tp.is_null() {
                        tp = first_tp;
                        (*child).wn_wnode = (*tp).wn_wnode;
                        (*tp).wn_wnode = child;
                    }
                } else {
                    ht.insert(hk, child);
                }
            }
            np = (*np).wn_sibling;
        }
        *tot += len + 1;

        let mut hashkey = [0u8; 6];
        hashkey[0] = len as u8;
        let mut nr: c_uint = 0;
        np = node;
        while !np.is_null() {
            let n: c_uint = if (*np).wn_byte == 0 {
                ((*np).wn_flags as c_int
                    + (((*np).wn_region as c_int) << 8)
                    + (((*np).wn_affixID as c_int) << 16)) as c_uint
            } else {
                ((*np).wn_byte as usize).wrapping_add(((*np).wn_child as usize) << 8) as c_uint
            };
            nr = nr.wrapping_mul(101).wrapping_add(n);
            np = (*np).wn_sibling;
        }

        let mut n = nr & 0xff;
        hashkey[1] = if n == 0 { 1 } else { n as u8 };
        n = (nr >> 8) & 0xff;
        hashkey[2] = if n == 0 { 1 } else { n as u8 };
        n = (nr >> 16) & 0xff;
        hashkey[3] = if n == 0 { 1 } else { n as u8 };
        n = (nr >> 24) & 0xff;
        hashkey[4] = if n == 0 { 1 } else { n as u8 };
        hashkey[5] = 0;

        veryfast_breakcheck();

        (compressed, hashkey)
    }
}

unsafe extern "C-unwind" fn node_equal(n1: *mut wordnode_T, n2: *mut wordnode_T) -> bool {
    unsafe {
        let mut p1 = n1;
        let mut p2 = n2;
        while !p1.is_null() && !p2.is_null() {
            if (*p1).wn_byte != (*p2).wn_byte
                || if (*p1).wn_byte == 0 {
                    (*p1).wn_flags != (*p2).wn_flags
                        || (*p1).wn_region != (*p2).wn_region
                        || (*p1).wn_affixID != (*p2).wn_affixID
                } else {
                    (*p1).wn_child != (*p2).wn_child
                }
            {
                break;
            }
            p1 = (*p1).wn_sibling;
            p2 = (*p2).wn_sibling;
        }
        p1.is_null() && p2.is_null()
    }
}

unsafe fn write_vim_spell(spin: *mut spellinfo_T, fname: *mut c_char) -> c_int {
    unsafe {
        let mut retval = OK;

        let fd = os_fopen(fname, c"w".as_ptr());
        if fd.is_null() {
            semsg!(gettext((&raw const e_notopen).cast()), fname);
            return FAIL;
        }

        let mut fwv = fwrite(VIMSPELLMAGIC.as_ptr().cast(), VIMSPELLMAGICL, 1, fd);
        if fwv != 1 {
            if fclose(fd) == EOF {
                // error
            }
            emsg(gettext((&raw const e_write).cast()));
            return FAIL;
        }

        fputc(VIMSPELLVERSION, fd);

        if !(*spin).si_info.is_null() {
            fputc(SN_INFO, fd);
            fputc(0, fd);
            let i = strlen((*spin).si_info);
            put_bytes(fd, i as u64, 4);
            fwv &= fwrite((*spin).si_info.cast(), i, 1, fd);
        }

        let regionmask: c_int = if (*spin).si_region_count > 1 {
            fputc(SN_REGION, fd);
            fputc(SNF_REQUIRED, fd);
            let l = ((*spin).si_region_count as usize) * 2;
            put_bytes(fd, l as u64, 4);
            fwv &= fwrite((&raw const (*spin).si_region_name).cast(), l, 1, fd);
            (1 << (*spin).si_region_count) - 1
        } else {
            0
        };

        if (*spin).si_ascii == 0 && (*spin).si_add == 0 {
            let mut folchars = [0 as c_char; 128 * 8];

            fputc(SN_CHARFLAGS, fd);
            fputc(SNF_REQUIRED, fd);

            let mut l: usize = 0;
            for i in 128..256 {
                l += utf_char2bytes(spelltab.st_fold[i] as c_int, folchars.as_mut_ptr().add(l))
                    as usize;
            }
            put_bytes(fd, (1 + 128 + 2 + l) as u64, 4);

            fputc(128, fd);
            for i in 128..256 {
                let mut flags: c_int = 0;
                if spelltab.st_isw[i] {
                    flags |= CF_WORD as c_int;
                }
                if spelltab.st_isu[i] {
                    flags |= CF_UPPER as c_int;
                }
                fputc(flags, fd);
            }

            put_bytes(fd, l as u64, 2);
            fwv &= fwrite(folchars.as_ptr().cast(), l, 1, fd);
        }

        if !(*spin).si_midword.is_null() {
            fputc(SN_MIDWORD, fd);
            fputc(SNF_REQUIRED, fd);

            let i = strlen((*spin).si_midword);
            put_bytes(fd, i as u64, 4);
            fwv &= fwrite((*spin).si_midword.cast(), i, 1, fd);
        }

        if !(*spin).si_prefcond.is_empty() {
            fputc(SN_PREFCOND, fd);
            fputc(SNF_REQUIRED, fd);

            let l = write_spell_prefcond(ptr::null_mut(), &(*spin).si_prefcond, &raw mut fwv)
                as usize;
            put_bytes(fd, l as u64, 4);

            write_spell_prefcond(fd, &(*spin).si_prefcond, &raw mut fwv);
        }

        for round in 1..=3 {
            let gap = if round == 1 {
                &mut (*spin).si_rep
            } else if round == 2 {
                if !(*spin).si_sofofr.is_null() && !(*spin).si_sofoto.is_null() {
                    continue;
                }
                &mut (*spin).si_sal
            } else {
                &mut (*spin).si_repsal
            };

            if gap.is_empty() {
                continue;
            }

            if round != 2 {
                gap.sort_unstable_by(|p1, p2| {
                    CStr::from_ptr(p1.ft_from)
                        .to_bytes()
                        .cmp(CStr::from_ptr(p2.ft_from).to_bytes())
                });
            }

            let sect_id = if round == 1 {
                SN_REP
            } else if round == 2 {
                SN_SAL
            } else {
                SN_REPSAL
            };
            fputc(sect_id, fd);
            fputc(0, fd);

            let mut l: usize = 2;
            for ftp in gap.iter() {
                l += 1 + strlen(ftp.ft_from);
                l += 1 + strlen(ftp.ft_to);
            }
            if round == 2 {
                l += 1;
            }
            put_bytes(fd, l as u64, 4);

            if round == 2 {
                let mut i: c_int = 0;
                if (*spin).si_followup != 0 {
                    i |= SAL_F0LLOWUP;
                }
                if (*spin).si_collapse != 0 {
                    i |= SAL_COLLAPSE;
                }
                if (*spin).si_rem_accents != 0 {
                    i |= SAL_REM_ACCENTS;
                }
                fputc(i, fd);
            }

            put_bytes(fd, gap.len() as u64, 2);
            for ftp in gap.iter() {
                for rr in 1..=2 {
                    let p = if rr == 1 { ftp.ft_from } else { ftp.ft_to };
                    let slen = strlen(p);
                    fputc(slen as c_int, fd);
                    if slen > 0 {
                        fwv &= fwrite(p.cast(), slen, 1, fd);
                    }
                }
            }
        }

        if !(*spin).si_sofofr.is_null() && !(*spin).si_sofoto.is_null() {
            fputc(SN_SOFO, fd);
            fputc(0, fd);

            let l1 = strlen((*spin).si_sofofr);
            let l2 = strlen((*spin).si_sofoto);
            put_bytes(fd, (l1 + l2 + 4) as u64, 4);

            put_bytes(fd, l1 as u64, 2);
            fwv &= fwrite((*spin).si_sofofr.cast(), l1, 1, fd);

            put_bytes(fd, l2 as u64, 2);
            fwv &= fwrite((*spin).si_sofoto.cast(), l2, 1, fd);
        }

        if !(*spin).si_commonwords.is_empty() {
            fputc(SN_WORDS, fd);
            fputc(0, fd);

            for round in 1..=2 {
                let mut len: usize = 0;
                for word in &(*spin).si_commonwords {
                    let l = word.len() + 1;
                    len += l;
                    if round == 2 {
                        fwv &= fwrite(word.as_ptr().cast(), word.len(), 1, fd);
                        fwv &= fwrite(c"".as_ptr().cast(), 1, 1, fd);
                    }
                }
                if round == 1 {
                    put_bytes(fd, len as u64, 4);
                }
            }
        }

        if !(*spin).si_map.is_empty() {
            fputc(SN_MAP, fd);
            fputc(0, fd);
            let l = (*spin).si_map.len();
            put_bytes(fd, l as u64, 4);
            fwv &= fwrite((*spin).si_map.as_ptr().cast(), l, 1, fd);
        }

        if (*spin).si_nosugfile == 0
            && (!(*spin).si_sal.is_empty()
                || (!(*spin).si_sofofr.is_null() && !(*spin).si_sofoto.is_null()))
        {
            fputc(SN_SUGFILE, fd);
            fputc(0, fd);
            put_bytes(fd, 8, 4);

            (*spin).si_sugtime = os_time() as time_t;
            put_time(fd, (*spin).si_sugtime);
        }

        if (*spin).si_nosplitsugs != 0 {
            fputc(SN_NOSPLITSUGS, fd);
            fputc(0, fd);
            put_bytes(fd, 0, 4);
        }

        if (*spin).si_nocompoundsugs != 0 {
            fputc(SN_NOCOMPOUNDSUGS, fd);
            fputc(0, fd);
            put_bytes(fd, 0, 4);
        }

        if !(*spin).si_compflags.is_null() {
            fputc(SN_COMPOUND, fd);
            fputc(0, fd);

            let mut l = strlen((*spin).si_compflags);
            for &p in &(*spin).si_comppat {
                l += strlen(p) + 1;
            }
            put_bytes(fd, (l + 7) as u64, 4);

            fputc((*spin).si_compmax, fd);
            fputc((*spin).si_compminlen, fd);
            fputc((*spin).si_compsylmax, fd);
            fputc(0, fd);
            fputc((*spin).si_compoptions, fd);
            put_bytes(fd, (*spin).si_comppat.len() as u64, 2);
            for &p in &(*spin).si_comppat {
                let plen = strlen(p);
                fputc(plen as c_int, fd);
                fwv &= fwrite(p.cast(), plen, 1, fd);
            }
            let cflen = strlen((*spin).si_compflags);
            fwv &= fwrite((*spin).si_compflags.cast(), cflen, 1, fd);
        }

        if (*spin).si_nobreak != 0 {
            fputc(SN_NOBREAK, fd);
            fputc(0, fd);
            put_bytes(fd, 0, 4);
        }

        if !(*spin).si_syllable.is_null() {
            fputc(SN_SYLLABLE, fd);
            fputc(0, fd);

            let l = strlen((*spin).si_syllable);
            put_bytes(fd, l as u64, 4);
            fwv &= fwrite((*spin).si_syllable.cast(), l, 1, fd);
        }

        fputc(SN_END, fd);

        (*spin).si_memtot = 0;
        for round in 1..=3 {
            let tree = if round == 1 {
                (*(*spin).si_foldroot).wn_sibling
            } else if round == 2 {
                (*(*spin).si_keeproot).wn_sibling
            } else {
                (*(*spin).si_prefroot).wn_sibling
            };

            clear_node(tree);

            let nodecount = put_node(ptr::null_mut(), tree, 0, regionmask, round == 3) as usize;
            put_bytes(fd, nodecount as u64, 4);
            (*spin).si_memtot += (nodecount + nodecount * size_of::<c_int>()) as c_int;

            put_node(fd, tree, 0, regionmask, round == 3);
        }

        if fputc(0, fd) == EOF {
            retval = FAIL;
        }
        if fclose(fd) == EOF {
            retval = FAIL;
        }

        if fwv != 1 {
            retval = FAIL;
        }
        if retval == FAIL {
            emsg(gettext((&raw const e_write).cast()));
        }

        retval
    }
}

unsafe fn clear_node(node: *mut wordnode_T) {
    unsafe {
        if !node.is_null() {
            let mut np = node;
            while !np.is_null() {
                (*np).wn_index = 0;
                (*np).wn_wnode = ptr::null_mut();

                if (*np).wn_byte != 0 {
                    clear_node((*np).wn_child);
                }
                np = (*np).wn_sibling;
            }
        }
    }
}

unsafe fn put_node(
    fd: *mut FILE,
    node: *mut wordnode_T,
    idx: c_int,
    regionmask: c_int,
    prefixtree: bool,
) -> c_int {
    unsafe {
        if node.is_null() {
            return 0;
        }

        (*node).wn_index = idx;

        let mut siblingcount: c_int = 0;
        let mut np = node;
        while !np.is_null() {
            siblingcount += 1;
            np = (*np).wn_sibling;
        }

        if !fd.is_null() {
            fputc(siblingcount, fd);
        }

        np = node;
        while !np.is_null() {
            if (*np).wn_byte == 0 {
                if !fd.is_null() {
                    if prefixtree {
                        if (*np).wn_flags == PFX_FLAGS as u16 {
                            fputc(BY_NOFLAGS, fd);
                        } else {
                            fputc(BY_FLAGS, fd);
                            fputc((*np).wn_flags as c_int, fd);
                        }
                        fputc((*np).wn_affixID as c_int, fd);
                        put_bytes(fd, (*np).wn_region as u16 as u64, 2);
                    } else {
                        let mut flags = (*np).wn_flags as c_int;
                        if regionmask != 0 && ((*np).wn_region as c_int) != regionmask {
                            flags |= WF_REGION;
                        }
                        if (*np).wn_affixID != 0 {
                            flags |= WF_AFX;
                        }
                        if flags == 0 {
                            fputc(BY_NOFLAGS, fd);
                        } else {
                            if (*np).wn_flags >= 0x100 {
                                fputc(BY_FLAGS2, fd);
                                fputc(flags, fd);
                                fputc(((flags as c_uint) >> 8) as c_int, fd);
                            } else {
                                fputc(BY_FLAGS, fd);
                                fputc(flags, fd);
                            }
                            if (flags & WF_REGION) != 0 {
                                fputc((*np).wn_region as c_int, fd);
                            }
                            if (flags & WF_AFX) != 0 {
                                fputc((*np).wn_affixID as c_int, fd);
                            }
                        }
                    }
                }
            } else {
                let child = (*np).wn_child;
                if (*child).wn_index != 0 && (*child).wn_wnode != node {
                    if !fd.is_null() {
                        fputc(BY_INDEX, fd);
                        put_bytes(fd, (*child).wn_index as u64, 3);
                    }
                } else if (*child).wn_wnode.is_null() {
                    (*child).wn_wnode = node;
                }

                if !fd.is_null() && fputc((*np).wn_byte as c_int, fd) == EOF {
                    emsg(gettext((&raw const e_write).cast()));
                    return 0;
                }
            }
            np = (*np).wn_sibling;
        }

        let mut newindex = idx + siblingcount + 1;

        np = node;
        while !np.is_null() {
            if (*np).wn_byte != 0 && (*(*np).wn_child).wn_wnode == node {
                newindex = put_node(fd, (*np).wn_child, newindex, regionmask, prefixtree);
            }
            np = (*np).wn_sibling;
        }

        newindex
    }
}

/// ":mkspell [-ascii] outfile  infile ..."
/// ":mkspell [-ascii] addfile"
pub unsafe fn ex_mkspell(eap: *mut exarg_T) {
    unsafe {
        let mut fnames: Vec<*mut c_char> = Vec::new();
        let mut arg = (*eap).arg;
        let mut ascii = false;

        if strncmp(arg, c"-ascii".as_ptr(), 6) == 0 {
            ascii = true;
            arg = skipwhite(arg.add(6));
        }

        if get_arglist_exp(arg, &mut fnames, false) != OK {
            return;
        }

        mkspell(&fnames, ascii, (*eap).forceit != 0, false);
        crate::path::clear_cstrings(&mut fnames);
    }
}

unsafe fn spell_make_sugfile(spin: *mut spellinfo_T, wfname: *mut c_char) {
    unsafe {
        let langs = &raw mut first_lang;
        let mut slang = (*langs)
            .iter_mut()
            .rev()
            .find(|s| path_equal(wfname, s.sl_fname, kPathCmpExpand))
            .map_or(ptr::null_mut(), |s| &raw mut **s);
        let mut temp_slang: Option<Box<slang_T>> = None;
        if slang.is_null() {
            spell_message(spin, gettext(c"Reading back spell file...".as_ptr()));
            let mut new_lp = slang_alloc(ptr::null_mut());
            new_lp.sl_fname = xstrdup(wfname);
            new_lp.sl_add = !strstr(path_tail(wfname), SPL_FNAME_ADD).is_null();
            slang = spell_load_file(wfname, ptr::null_mut(), &raw mut *new_lp, false);
            if slang.is_null() {
                return;
            }
            temp_slang = Some(new_lp);
        }

        (*spin).si_blocks.clear();
        (*spin).si_blocks_cnt = 0;
        (*spin).si_compress_cnt = 0;
        (*spin).si_free_count = 0;
        (*spin).si_first_free = ptr::null_mut();
        (*spin).si_foldwcount = 0;

        spell_message(spin, gettext(c"Performing soundfolding...".as_ptr()));
        if sug_filltree(spin, slang) != FAIL && sug_maketable(spin) != FAIL {
            smsg!(
                0,
                gettext(c"Number of words after soundfolding: %lld".as_ptr()),
                (*spin).si_spellbuf.as_ref().unwrap().b_ml.ml_line_count as i64,
            );

            spell_message(spin, gettext(msg_compressing));
            wordtree_compress(spin, (*spin).si_foldroot, c"case-folded".as_ptr());

            let mut fname_buf = [0 as c_char; MAXPATHL];
            let fname = fname_buf.as_mut_ptr();
            xstrlcpy(fname, wfname, MAXPATHL);
            let len = strlen(fname);
            *fname.add(len - 2) = b'u' as c_char;
            *fname.add(len - 1) = b'g' as c_char;
            sug_write(spin, fname);
        }

        drop(temp_slang);
        (*spin).si_blocks.clear();
        close_spellbuf(&mut (*spin).si_spellbuf);
    }
}

unsafe fn sug_filltree(spin: *mut spellinfo_T, slang: *mut slang_T) -> c_int {
    unsafe {
        let mut arridx = [0 as idx_T; MAXWLEN];
        let mut curi = [0 as c_int; MAXWLEN];
        let mut tword = [0 as c_char; MAXWLEN];
        let mut tsalword = [0 as c_char; MAXWLEN];
        let mut words_done: c_uint = 0;
        let mut wordcount = [0 as c_int; MAXWLEN];

        (*spin).si_foldroot = wordtree_alloc(spin);
        (*spin).si_sugtree = 1;

        if (*slang).sl_fbyts.is_empty() || (*slang).sl_fidxs.is_empty() {
            return FAIL;
        }
        let fbyts_len = (*slang).sl_fbyts.len() as c_int;
        let byts = (*slang).sl_fbyts.as_mut_ptr();
        let idxs = (*slang).sl_fidxs.as_mut_ptr();

        arridx[0] = 0;
        curi[0] = 1;
        wordcount[0] = 0;

        let mut depth: isize = 0;
        while depth >= 0 && !got_int {
            let d = depth as usize;
            if curi[d] > *byts.offset(arridx[d] as isize) as c_int {
                *idxs.offset(arridx[d] as isize) = wordcount[d];
                if depth > 0 {
                    wordcount[d - 1] += wordcount[d];
                }
                depth -= 1;
                line_breakcheck();
            } else {
                let mut n = arridx[d] + curi[d];
                curi[d] += 1;

                let c = *byts.offset(n as isize) as c_int;
                if c == 0 {
                    tword[d] = NUL;
                    spell_soundfold(slang, tword.as_mut_ptr(), true, tsalword.as_mut_ptr());

                    if tree_add_word(
                        spin,
                        tsalword.as_ptr(),
                        (*spin).si_foldroot,
                        (words_done >> 16) as c_int,
                        (words_done & 0xffff) as c_int,
                        0,
                    ) == FAIL
                    {
                        return FAIL;
                    }

                    words_done += 1;
                    wordcount[d] += 1;
                    (*spin).si_blocks_cnt = 0;

                    while n + 1 < fbyts_len && *byts.offset((n + 1) as isize) == 0 {
                        n += 1;
                        curi[d] += 1;
                    }
                } else if d < MAXWLEN - 1 {
                    tword[d] = c as u8 as c_char;
                    depth += 1;
                    let nd = depth as usize;
                    arridx[nd] = *idxs.offset(n as isize);
                    curi[nd] = 1;
                    wordcount[nd] = 0;
                }
            }
        }

        smsg!(
            0,
            gettext(c"Total number of words: %d".as_ptr()),
            words_done,
        );
        OK
    }
}

unsafe fn sug_maketable(spin: *mut spellinfo_T) -> c_int {
    unsafe {
        let mut ga: Vec<c_char> = Vec::with_capacity(100);
        let mut res = OK;

        (*spin).si_spellbuf = Some(open_spellbuf());

        if sug_filltable(spin, (*(*spin).si_foldroot).wn_sibling, 0, &mut ga) == -1 {
            res = FAIL;
        }

        res
    }
}

unsafe fn sug_filltable(
    spin: *mut spellinfo_T,
    node: *mut wordnode_T,
    startwordnr: c_int,
    gap: &mut Vec<c_char>,
) -> c_int {
    unsafe {
        let mut wordnr = startwordnr;

        let mut p = node;
        while !p.is_null() {
            if (*p).wn_byte == 0 {
                gap.clear();
                let mut prev_nr: c_int = 0;
                let mut np = p;
                while !np.is_null() && (*np).wn_byte == 0 {
                    let mut nr =
                        (((*np).wn_flags as c_int) << 16) + ((*np).wn_region as c_int & 0xffff);
                    nr -= prev_nr;
                    prev_nr += nr;
                    let mut obuf = [0 as c_char; 8];
                    let olen = offset2bytes(nr, obuf.as_mut_ptr()) as usize;
                    gap.extend_from_slice(&obuf[..olen]);
                    np = (*np).wn_sibling;
                }

                gap.push(NUL);

                let spellbuf = (*spin).si_spellbuf.as_deref_mut().unwrap();
                if ml_append_buf(
                    spellbuf,
                    wordnr as linenr_T,
                    gap.as_mut_ptr(),
                    gap.len() as c_int,
                    true,
                ) == FAIL
                {
                    return -1;
                }
                wordnr += 1;

                while !(*p).wn_sibling.is_null() && (*(*p).wn_sibling).wn_byte == 0 {
                    (*p).wn_sibling = (*(*p).wn_sibling).wn_sibling;
                }

                (*p).wn_flags = 0;
                (*p).wn_region = 0;
            } else {
                wordnr = sug_filltable(spin, (*p).wn_child, wordnr, gap);
                if wordnr == -1 {
                    return -1;
                }
            }
            p = (*p).wn_sibling;
        }
        wordnr
    }
}

unsafe fn offset2bytes(nr: c_int, buf_in: *mut c_char) -> c_int {
    unsafe {
        let buf = buf_in.cast::<u8>();

        let b1 = nr % 255 + 1;
        let mut rem = nr / 255;
        let b2 = rem % 255 + 1;
        rem /= 255;
        let b3 = rem % 255 + 1;
        let b4 = rem / 255 + 1;

        if b4 > 1 || b3 > 0x1f {
            *buf.add(0) = (0xe0 + b4) as u8;
            *buf.add(1) = b3 as u8;
            *buf.add(2) = b2 as u8;
            *buf.add(3) = b1 as u8;
            return 4;
        }
        if b3 > 1 || b2 > 0x3f {
            *buf.add(0) = (0xc0 + b3) as u8;
            *buf.add(1) = b2 as u8;
            *buf.add(2) = b1 as u8;
            return 3;
        }
        if b2 > 1 || b1 > 0x7f {
            *buf.add(0) = (0x80 + b2) as u8;
            *buf.add(1) = b1 as u8;
            return 2;
        }
        *buf.add(0) = b1 as u8;
        1
    }
}

unsafe fn sug_write(spin: *mut spellinfo_T, fname: *mut c_char) {
    unsafe {
        let fd = os_fopen(fname, c"w".as_ptr());
        if fd.is_null() {
            semsg!(gettext((&raw const e_notopen).cast()), fname);
            return;
        }

        vim_snprintf!(
            (&raw mut IObuff).cast(),
            IOSIZE,
            gettext(c"Writing suggestion file %s...".as_ptr()),
            fname,
        );
        spell_message(spin, (&raw mut IObuff).cast());

        if fwrite(VIMSUGMAGIC.as_ptr().cast(), VIMSUGMAGICL, 1, fd) != 1 {
            emsg(gettext((&raw const e_write).cast()));
            fclose(fd);
            return;
        }
        fputc(VIMSUGVERSION, fd);
        put_time(fd, (*spin).si_sugtime);

        (*spin).si_memtot = 0;
        let tree = (*(*spin).si_foldroot).wn_sibling;
        clear_node(tree);

        let nodecount = put_node(ptr::null_mut(), tree, 0, 0, false) as usize;
        put_bytes(fd, nodecount as u64, 4);
        (*spin).si_memtot += (nodecount + nodecount * size_of::<c_int>()) as c_int;

        put_node(fd, tree, 0, 0, false);

        let spellbuf: *mut buf_T = (*spin)
            .si_spellbuf
            .as_deref_mut()
            .map_or(ptr::null_mut(), |b| &raw mut *b);
        let wcount = (*spellbuf).b_ml.ml_line_count;
        put_bytes(fd, wcount as u64, 4);

        let mut write_err = false;
        for lnum in 1..=wcount {
            let line = ml_get_buf(spellbuf, lnum);
            let len = ml_get_buf_len(spellbuf, lnum) + 1;
            if fwrite(line.cast(), len as usize, 1, fd) == 0 {
                emsg(gettext((&raw const e_write).cast()));
                write_err = true;
                break;
            }
            (*spin).si_memtot += len;
        }

        if !write_err {
            if fputc(0, fd) == EOF {
                emsg(gettext((&raw const e_write).cast()));
            }

            vim_snprintf!(
                (&raw mut IObuff).cast(),
                IOSIZE,
                gettext(c"Estimated runtime memory use: %d bytes".as_ptr()),
                (*spin).si_memtot,
            );
            spell_message(spin, (&raw mut IObuff).cast());
        }

        fclose(fd);
    }
}

unsafe fn mkspell(fnames: &[*mut c_char], ascii: bool, over_write: bool, added_word: bool) {
    unsafe {
        let mut fname_buf = [0 as c_char; MAXPATHL];
        let mut wfname_buf = [0 as c_char; MAXPATHL];
        let mut afile: [Option<afffile_T>; MAXREGIONS] = [const { None }; MAXREGIONS];
        let mut error = false;
        let mut spin = spellinfo_T::default();

        spell_check_msm(p_msm, true);

        spin.si_verbose = c_int::from(!added_word);
        spin.si_ascii = c_int::from(ascii);
        spin.si_followup = 1;
        spin.si_rem_accents = 1;
        spin.si_newcompID = 127;

        let fcount = fnames.len() as c_int;
        let innames = if fcount == 1 { fnames } else { &fnames[1..] };
        let mut incount = fcount - 1;

        let wfname = wfname_buf.as_mut_ptr();

        if let Some(&f0) = fnames.first() {
            let len = strlen(f0);
            if fcount == 1 && len > 4 && strcmp(f0.add(len - 4), c".add".as_ptr()) == 0 {
                incount = 1;
                vim_snprintf!(wfname, MAXPATHL, c"%s.spl".as_ptr(), f0);
            } else if fcount == 1 {
                incount = 1;
                vim_snprintf!(
                    wfname,
                    MAXPATHL,
                    SPL_FNAME_TMPL,
                    f0,
                    if spin.si_ascii != 0 {
                        c"ascii".as_ptr()
                    } else {
                        spell_enc()
                    },
                );
            } else if len > 4 && strcmp(f0.add(len - 4), c".spl".as_ptr()) == 0 {
                xstrlcpy(wfname, f0, MAXPATHL);
            } else {
                vim_snprintf!(
                    wfname,
                    MAXPATHL,
                    SPL_FNAME_TMPL,
                    f0,
                    if spin.si_ascii != 0 {
                        c"ascii".as_ptr()
                    } else {
                        spell_enc()
                    },
                );
            }

            if !strstr(path_tail(wfname), SPL_FNAME_ASCII).is_null() {
                spin.si_ascii = 1;
            }
            if !strstr(path_tail(wfname), SPL_FNAME_ADD).is_null() {
                spin.si_add = 1;
            }
        }

        if incount <= 0 {
            emsg(gettext((&raw const e_invarg).cast()));
        } else if !vim_strchr(path_tail(wfname), b'_' as c_int).is_null() {
            emsg(gettext(
                c"E751: Output file name must not have region name".as_ptr(),
            ));
        } else if incount > MAXREGIONS as c_int {
            semsg!(
                gettext(c"E754: Only up to %d regions supported".as_ptr()),
                MAXREGIONS as c_int,
            );
        } else if !over_write && os_path_exists(wfname) {
            emsg(gettext((&raw const e_exists).cast()));
        } else if os_isdir(wfname) {
            semsg!(gettext((&raw const e_isadir2).cast()), wfname);
        } else {
            let fname = fname_buf.as_mut_ptr();

            let mut region_ok = true;
            for (i, slot) in afile.iter_mut().enumerate().take(incount as usize) {
                *slot = None;

                if incount > 1 {
                    let in_i = innames[i];
                    let len = strlen(in_i);
                    if strlen(path_tail(in_i)) < 5 || *in_i.add(len - 3) != b'_' as c_char {
                        semsg!(gettext(c"E755: Invalid region in %s".as_ptr()), in_i);
                        region_ok = false;
                        break;
                    }
                    spin.si_region_name[i * 2] =
                        (*in_i.add(len - 2) as u8).to_ascii_lowercase() as c_char;
                    spin.si_region_name[i * 2 + 1] =
                        (*in_i.add(len - 1) as u8).to_ascii_lowercase() as c_char;
                }
            }

            if region_ok {
                spin.si_region_count = incount;
                spin.si_foldroot = wordtree_alloc(&raw mut spin);
                spin.si_keeproot = wordtree_alloc(&raw mut spin);
                spin.si_prefroot = wordtree_alloc(&raw mut spin);

                if spin.si_add == 0 {
                    spin.si_clear_chartab = 1;
                }

                let mut i: usize = 0;
                while (i as c_int) < incount && !error {
                    let in_i = innames[i];
                    spin.si_conv.vc_type = CONV_NONE;
                    spin.si_region = 1 << i;

                    vim_snprintf!(fname, MAXPATHL, c"%s.aff".as_ptr(), in_i);
                    if os_path_exists(fname) {
                        afile[i] = spell_read_aff(&raw mut spin, fname);
                        if let Some(ref aff) = afile[i] {
                            vim_snprintf!(fname, MAXPATHL, c"%s.dic".as_ptr(), in_i);
                            if spell_read_dic(&raw mut spin, fname, aff) == FAIL {
                                error = true;
                            }
                        } else {
                            error = true;
                        }
                    } else if spell_read_wordfile(&raw mut spin, in_i) == FAIL {
                        error = true;
                    }

                    convert_setup(&mut spin.si_conv, ptr::null_mut(), ptr::null_mut());
                    i += 1;
                }

                if !spin.si_compflags.is_null() && spin.si_nobreak != 0 {
                    msg(
                        gettext(c"Warning: both compounding and NOBREAK specified".as_ptr()),
                        0,
                    );
                }

                if !error && !got_int {
                    spell_message(&raw const spin, gettext(msg_compressing));
                    wordtree_compress(&raw mut spin, spin.si_foldroot, c"case-folded".as_ptr());
                    wordtree_compress(&raw mut spin, spin.si_keeproot, c"keep-case".as_ptr());
                    wordtree_compress(&raw mut spin, spin.si_prefroot, c"prefixes".as_ptr());
                }

                if !error && !got_int {
                    vim_snprintf!(
                        (&raw mut IObuff).cast(),
                        IOSIZE,
                        gettext(c"Writing spell file %s...".as_ptr()),
                        wfname,
                    );
                    spell_message(&raw const spin, (&raw mut IObuff).cast());

                    error = write_vim_spell(&raw mut spin, wfname) == FAIL;

                    spell_message(&raw const spin, gettext(c"Done!".as_ptr()));
                    vim_snprintf!(
                        (&raw mut IObuff).cast(),
                        IOSIZE,
                        gettext(c"Estimated runtime memory use: %d bytes".as_ptr()),
                        spin.si_memtot,
                    );
                    spell_message(&raw const spin, (&raw mut IObuff).cast());

                    if !error {
                        spell_reload_one(wfname, added_word);
                    }
                }

                spin.si_commonwords.clear();

                for slot in afile.iter_mut().take(incount as usize) {
                    *slot = None;
                }

                spin.si_blocks.clear();

                if spin.si_sugtime != 0 && !error && !got_int {
                    spell_make_sugfile(&raw mut spin, wfname);
                }
            }
        }
    }
}

unsafe fn spell_message(spin: *const spellinfo_T, str_: *const c_char) {
    unsafe {
        if (*spin).si_verbose != 0 || p_verbose > 2 {
            if (*spin).si_verbose == 0 {
                verbose_enter();
            }
            msg(str_, 0);
            ui_flush();
            if (*spin).si_verbose == 0 {
                verbose_leave();
            }
        }
    }
}

/// ":[count]spellgood  {word}"
/// ":[count]spellwrong {word}"
/// ":[count]spellundo  {word}"
/// ":[count]spellrare  {word}"
pub unsafe fn ex_spell(eap: *mut exarg_T) {
    unsafe {
        let what = if (*eap).cmdidx == CMD_spellwrong {
            SPELL_ADD_BAD
        } else if (*eap).cmdidx == CMD_spellrare {
            SPELL_ADD_RARE
        } else {
            SPELL_ADD_GOOD
        };
        spell_add_word(
            (*eap).arg,
            strlen((*eap).arg) as c_int,
            what,
            if (*eap).forceit != 0 {
                0
            } else {
                (*eap).line2 as c_int
            },
            (*eap).cmdidx == CMD_spellundo,
        );
    }
}

/// Add "word[len]" to 'spellfile' as a good or bad word.
pub unsafe fn spell_add_word(
    word: *mut c_char,
    len: c_int,
    what: SpellAddType,
    idx: c_int,
    undo: bool,
) {
    unsafe {
        let mut fd: *mut FILE = ptr::null_mut();
        let mut buf: *mut buf_T = ptr::null_mut();
        let mut new_spf = false;
        let mut fnamebuf_arr = [0 as c_char; MAXPATHL];
        let mut line = [0 as c_char; MAXWLEN * 2];

        if !valid_spell_word(word, word.offset(len as isize)) {
            emsg(gettext(e_illegal_character_in_word));
            return;
        }

        let fname: *mut c_char = if idx == 0 {
            if int_wordlist.is_null() {
                int_wordlist = vim_tempname();
                if int_wordlist.is_null() {
                    return;
                }
            }
            int_wordlist
        } else {
            if *(*win_s(curwin)).b_p_spf == NUL {
                init_spellfile();
                new_spf = true;
            }

            if *(*win_s(curwin)).b_p_spf == NUL {
                semsg!(gettext((&raw const e_notset).cast()), c"spellfile".as_ptr());
                return;
            }
            let fnamebuf = fnamebuf_arr.as_mut_ptr();

            let mut spf = (*win_s(curwin)).b_p_spf;
            let mut i: c_int = 1;
            while *spf != NUL {
                copy_option_part(&raw mut spf, fnamebuf, MAXPATHL, c",".as_ptr());
                if i == idx {
                    break;
                }
                if *spf == NUL {
                    semsg!(
                        gettext(c"E765: 'spellfile' does not have %d entries".as_ptr()),
                        idx,
                    );
                    return;
                }
                i += 1;
            }

            buf = buflist_findname_exp(fnamebuf);
            if !buf.is_null() && (*buf).b_ml.ml_mfp.is_null() {
                buf = ptr::null_mut();
            }
            if !buf.is_null() && bufIsChanged(buf) {
                emsg(gettext((&raw const e_bufloaded).cast()));
                return;
            }

            fnamebuf
        };

        if what == SPELL_ADD_BAD || undo {
            let mut fpos_next: c_long = 0;
            fd = os_fopen(fname, c"r".as_ptr());
            if !fd.is_null() {
                while !vim_fgets(line.as_mut_ptr(), (MAXWLEN * 2) as c_int, fd) {
                    let fpos = fpos_next;
                    fpos_next = ftell(fd);
                    if fpos_next < 0 {
                        break;
                    }
                    let linelen = strlen(line.as_ptr());
                    let ulen = len as usize;
                    if linelen >= ulen
                        && strncmp(word, line.as_ptr(), ulen) == 0
                        && (line[ulen] == b'/' as c_char || (line[ulen] as u8) < b' ')
                    {
                        fclose(fd);
                        fd = os_fopen(fname, c"r+".as_ptr());
                        if fd.is_null() {
                            break;
                        }
                        if fseek(fd, fpos, SEEK_SET) == 0 {
                            fputc(b'#' as c_int, fd);
                            if undo {
                                home_replace(
                                    ptr::null_mut(),
                                    fname,
                                    (&raw mut NameBuff).cast(),
                                    MAXPATHL,
                                    true,
                                );
                                smsg!(
                                    0,
                                    gettext(c"Word '%.*s' removed from %s".as_ptr()),
                                    len,
                                    word,
                                    (&raw const NameBuff).cast::<c_char>(),
                                );
                            }
                        }
                        if fseek(fd, fpos_next, SEEK_SET) != 0 {
                            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                            semsg!(
                                c"%s: %s".as_ptr(),
                                gettext(c"Seek error in spellfile".as_ptr()),
                                strerror(err),
                            );
                            break;
                        }
                    }
                }
                if !fd.is_null() {
                    fclose(fd);
                }
            }
        }

        if !undo {
            fd = os_fopen(fname, c"a".as_ptr());
            if fd.is_null() && new_spf && !dir_of_file_exists(fname) {
                let p = path_tail_with_sep(fname);
                if p != fname {
                    let c = *p;
                    *p = NUL;
                    os_mkdir(fname, 0o755);
                    *p = c;
                    fd = os_fopen(fname, c"a".as_ptr());
                }
            }

            if fd.is_null() {
                semsg!(gettext((&raw const e_notopen).cast()), fname);
            } else {
                let wlen = (0..(len as usize))
                    .take_while(|&i| *word.add(i) != NUL)
                    .count();
                fwrite(word.cast(), 1, wlen, fd);
                if what == SPELL_ADD_BAD {
                    fwrite(c"/!\n".as_ptr().cast(), 1, 3, fd);
                } else if what == SPELL_ADD_RARE {
                    fwrite(c"/?\n".as_ptr().cast(), 1, 3, fd);
                } else {
                    fputc(b'\n' as c_int, fd);
                }
                fclose(fd);

                home_replace(
                    ptr::null_mut(),
                    fname,
                    (&raw mut NameBuff).cast(),
                    MAXPATHL,
                    true,
                );
                smsg!(
                    0,
                    gettext(c"Word '%.*s' added to %s".as_ptr()),
                    len,
                    word,
                    (&raw const NameBuff).cast::<c_char>(),
                );
            }
        }

        if !fd.is_null() {
            mkspell(&[fname], false, true, true);

            if !buf.is_null() {
                buf_reload(buf, (*buf).b_orig_mode, false);
            }

            redraw_all_later(UPD_SOME_VALID);
        }
    }
}

unsafe fn init_spellfile() {
    unsafe {
        let mut aspath = false;
        let mut lstart = (*curbuf).b_s.b_p_spl;

        if *(*win_s(curwin)).b_p_spl == NUL || (*win_s(curwin)).b_langp.is_empty() {
            return;
        }

        let mut lend = (*win_s(curwin)).b_p_spl;
        while *lend != NUL && vim_strchr(c",._".as_ptr(), *lend as u8 as c_int).is_null() {
            if vim_ispathsep(*lend as c_int) {
                aspath = true;
                lstart = lend.add(1);
            }
            lend = lend.add(1);
        }

        let mut buf_arr = [0 as c_char; MAXPATHL];
        let buf = buf_arr.as_mut_ptr();
        let buf_len = MAXPATHL;

        if !aspath {
            let xdg_path = get_xdg_home(XDGVarType::kXDGDataHome);
            xstrlcpy(buf, xdg_path, buf_len);
            xfree(xdg_path.cast());

            xstrlcat(buf, c"/site/spell".as_ptr(), buf_len);

            let mut failed_dir: *mut c_char = ptr::null_mut();
            if os_mkdir_recurse(buf, 0o755, &raw mut failed_dir, ptr::null_mut()) != 0 {
                xfree(failed_dir.cast());
                return;
            }
        } else {
            let plen = lend.offset_from((*curbuf).b_s.b_p_spl) as usize;
            if plen >= buf_len {
                return;
            }
            xmemcpyz(buf.cast(), (*curbuf).b_s.b_p_spl.cast(), plen);
        }

        let cur_len = strlen(buf);
        vim_snprintf!(
            buf.add(cur_len),
            buf_len - cur_len,
            c"/%.*s".as_ptr(),
            lend.offset_from(lstart) as c_int,
            lstart,
        );

        let fname = (*(&(*win_s(curwin)).b_langp)[0].lp_slang).sl_fname;
        let enc_suffix =
            if !fname.is_null() && !strstr(path_tail(fname), c".ascii.".as_ptr()).is_null() {
                c"ascii".as_ptr()
            } else {
                spell_enc()
            };
        let cur_len2 = strlen(buf);
        vim_snprintf!(
            buf.add(cur_len2),
            buf_len - cur_len2,
            c".%s.add".as_ptr(),
            enc_suffix,
        );

        set_option_value_give_err(kOptSpellfile, cstr_as_obj(buf), OPT_LOCAL);
    }
}

unsafe fn set_spell_charflags(flags_in: *const c_char, cnt: c_int, fol: *const c_char) {
    unsafe {
        let flags = flags_in.cast::<u8>();
        let mut new_st = spelltab_T::new();
        let mut p = fol;

        clear_spell_chartab(&raw mut new_st);

        for i in 0..128 {
            if (i as c_int) < cnt {
                new_st.st_isw[i + 128] = (*flags.add(i) & CF_WORD) != 0;
                new_st.st_isu[i + 128] = (*flags.add(i) & CF_UPPER) != 0;
            }

            if *p != NUL {
                let c = mb_ptr2char_adv(&raw mut p);
                new_st.st_fold[i + 128] = c as u8;
                if (i + 128) as c_int != c && new_st.st_isu[i + 128] && c < 256 {
                    new_st.st_upper[c as usize] = (i + 128) as u8;
                }
            }
        }

        set_spell_finish(&raw mut new_st);
    }
}

unsafe fn set_spell_finish(new_st: *mut spelltab_T) -> c_int {
    unsafe {
        if did_set_spelltab {
            for i in 0..256 {
                if spelltab.st_isw[i] != (*new_st).st_isw[i]
                    || spelltab.st_isu[i] != (*new_st).st_isu[i]
                    || spelltab.st_fold[i] != (*new_st).st_fold[i]
                    || spelltab.st_upper[i] != (*new_st).st_upper[i]
                {
                    emsg(gettext(
                        c"E763: Word characters differ between spell files".as_ptr(),
                    ));
                    return FAIL;
                }
            }
        } else {
            spelltab = *new_st;
            did_set_spelltab = true;
        }

        OK
    }
}

unsafe fn write_spell_prefcond(fd: *mut FILE, gap: &[*mut c_char], fwv: *mut usize) -> c_int {
    unsafe {
        if !fd.is_null() {
            put_bytes(fd, gap.len() as u64, 2);
        }
        let mut totlen = 2 + gap.len();
        for &p in gap {
            if !p.is_null() {
                let len = strlen(p);
                if !fd.is_null() {
                    fputc(len as c_int, fd);
                    *fwv &= fwrite(p.cast(), len, 1, fd);
                }
                totlen += len;
            } else if !fd.is_null() {
                fputc(0, fd);
            }
        }

        totlen as c_int
    }
}

unsafe fn set_map_str(lp: *mut slang_T, map: *const c_char) {
    unsafe {
        let mut headc: c_int = 0;

        if *map == NUL {
            (*lp).sl_has_map = false;
            return;
        }
        (*lp).sl_has_map = true;

        for i in 0..256 {
            (*lp).sl_map_array[i] = 0;
        }
        (*lp).sl_map_hash.clear();

        let mut p = map;
        while *p != NUL {
            let c = mb_cptr2char_adv(&raw mut p);
            if c == b'/' as c_int {
                headc = 0;
            } else {
                if headc == 0 {
                    headc = c;
                }

                if c >= 256 {
                    if (*lp).sl_map_hash.insert(c, headc).is_some() {
                        emsg(gettext(e_duplicate_char_in_map_entry));
                    }
                } else {
                    (*lp).sl_map_array[c as usize] = headc;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset2bytes_and_spell_check_msm_work() {
        unsafe {
            let mut buf = [0 as c_char; 8];
            assert_eq!(offset2bytes(0, buf.as_mut_ptr()), 1);
            assert_eq!(buf[0] as u8, 1);

            assert_eq!(offset2bytes(254, buf.as_mut_ptr()), 2);
            assert_eq!(buf[0] as u8, 0x81);
            assert_eq!(buf[1] as u8, 255);

            let mut msm_valid = *b"460000,2000,500\0";
            assert_eq!(spell_check_msm(msm_valid.as_mut_ptr().cast(), false), OK);

            let mut msm_invalid = *b"0,2000,500\0";
            assert_eq!(
                spell_check_msm(msm_invalid.as_mut_ptr().cast(), false),
                FAIL
            );
        }
    }
}
